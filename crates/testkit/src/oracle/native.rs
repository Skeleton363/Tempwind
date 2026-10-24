//! Native execution of single instructions under ptrace (x86-64 Linux).
//!
//! A forked child calls `PTRACE_TRACEME`, maps the code page at `CODE_ADDR`
//! and the data window at `DATA_ADDR` (`MAP_FIXED_NOREPLACE`) and stops
//! itself with `SIGSTOP`. For each case the parent writes the code page and
//! the whole data window through `/proc/<pid>/mem`, sets every GPR, RIP,
//! RFLAGS and the FS and GS bases with `PTRACE_SETREGS` and the x87/SSE state
//! with `PTRACE_SETFPREGS` (the FXSAVE image), then single-steps until RIP
//! leaves the instruction (one step, or one per iteration of a `rep` string
//! instruction) or the instruction faults, and reads everything back.
//! Faults map to [`FaultKind`]s as `docs/design/fixtures.md` specifies; any
//! other stop is a recording bug.

use super::fixture::{Fault, FaultKind, FpState};

/// Most single steps one case may take (`rep` iterations).
pub const MAX_STEPS: u32 = 65_536;

/// Highest FS or GS base the kernel accepts (`TASK_SIZE_MAX`, exclusive).
pub const MAX_SEGMENT_BASE: u64 = 0x7fff_ffff_f000;

/// Result of running one instruction natively.
#[derive(Clone, Debug)]
pub struct StepResult {
    pub gprs: [u64; 16],
    pub rip: u64,
    pub rflags: u64,
    pub fp: FpState,
    /// The exception that stopped the instruction, if any.
    pub fault: Option<Fault>,
    /// Single steps taken (1 except for `rep` string instructions).
    pub steps: u32,
    /// The whole data window after the instruction.
    pub data: Vec<u8>,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use imp::Tracee;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::os::unix::fs::FileExt;

    use anyhow::{Context, Result, bail, ensure};
    use x64wasm::abi::flags;

    use super::{Fault, FaultKind, FpState, MAX_SEGMENT_BASE, MAX_STEPS, StepResult};
    use crate::oracle::fixture::{Input, abridged_tag, full_tag_word};
    use crate::oracle::{CODE_ADDR, CODE_SIZE, DATA_ADDR, DATA_SIZE};

    const EXIT_TRACEME: i32 = 101;
    const EXIT_MAP_CODE: i32 = 102;
    const EXIT_MAP_DATA: i32 = 103;
    const EXIT_STOP: i32 = 104;

    // si_code values (Linux uapi asm-generic/siginfo.h).
    const SI_KERNEL: i32 = 0x80;
    const FPE_INTDIV: i32 = 1;
    const ILL_ILLOPN: i32 = 2;
    const SEGV_MAPERR: i32 = 1;
    const SEGV_ACCERR: i32 = 2;

    /// A stopped child process that executes cases one instruction at a time.
    pub struct Tracee {
        pid: libc::pid_t,
        mem: File,
        template: libc::user_regs_struct,
        fp_template: libc::user_fpregs_struct,
        code_page: Vec<u8>,
    }

    /// Runs in the forked child: only async-signal-safe calls, no allocation.
    unsafe fn child() -> ! {
        // SAFETY: plain system calls on fixed arguments; this process only
        // exists to be traced and never returns into Rust code.
        unsafe {
            if libc::ptrace(libc::PTRACE_TRACEME, 0, 0, 0) != 0 {
                libc::_exit(EXIT_TRACEME);
            }
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            let map = |addr: u64, len: u64, prot: i32| {
                let p = libc::mmap(
                    addr as *mut libc::c_void,
                    len as usize,
                    prot,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_FIXED_NOREPLACE,
                    -1,
                    0,
                );
                p as u64 == addr
            };
            let rwx = libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC;
            if !map(CODE_ADDR, CODE_SIZE, rwx) {
                libc::_exit(EXIT_MAP_CODE);
            }
            if !map(DATA_ADDR, DATA_SIZE, libc::PROT_READ | libc::PROT_WRITE) {
                libc::_exit(EXIT_MAP_DATA);
            }
            libc::raise(libc::SIGSTOP);
            // The parent never resumes this code; it redirects RIP.
            libc::_exit(EXIT_STOP);
        }
    }

    fn wait(pid: libc::pid_t) -> Result<i32> {
        let mut status = 0;
        loop {
            // SAFETY: `status` is a valid out pointer.
            let r = unsafe { libc::waitpid(pid, &mut status, libc::__WALL) };
            if r == pid {
                return Ok(status);
            }
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::Interrupted {
                return Err(err).context("waitpid");
            }
        }
    }

    fn describe(status: i32) -> String {
        if libc::WIFEXITED(status) {
            let code = libc::WEXITSTATUS(status);
            let why = match code {
                EXIT_TRACEME => " (PTRACE_TRACEME failed)",
                EXIT_MAP_CODE => " (could not map the code page at 0x100000)",
                EXIT_MAP_DATA => " (could not map the data window at 0x200000)",
                EXIT_STOP => " (resumed unexpectedly after SIGSTOP)",
                _ => "",
            };
            format!("exited with code {code}{why}")
        } else if libc::WIFSIGNALED(status) {
            format!("killed by signal {}", libc::WTERMSIG(status))
        } else if libc::WIFSTOPPED(status) {
            format!("stopped by signal {}", libc::WSTOPSIG(status))
        } else {
            format!("status {status:#x}")
        }
    }

    fn signal_name(sig: i32) -> &'static str {
        match sig {
            libc::SIGSEGV => "SIGSEGV",
            libc::SIGBUS => "SIGBUS",
            libc::SIGILL => "SIGILL",
            libc::SIGFPE => "SIGFPE",
            libc::SIGTRAP => "SIGTRAP",
            libc::SIGSTOP => "SIGSTOP",
            _ => "signal",
        }
    }

    /// Reads a 16-byte register slot of an FXSAVE image.
    fn slot(words: &[u32], i: usize) -> u128 {
        (0..4).fold(0, |acc, k| acc | u128::from(words[4 * i + k]) << (32 * k))
    }

    fn set_slot(words: &mut [u32], i: usize, v: u128) {
        for k in 0..4 {
            words[4 * i + k] = (v >> (32 * k)) as u32;
        }
    }

    /// The fixture's FP state as an FXSAVE image on top of `template`.
    fn to_fxsave(
        template: &libc::user_fpregs_struct,
        fp: &FpState,
    ) -> Result<libc::user_fpregs_struct> {
        let abridged = abridged_tag(fp.ftw);
        let derived = full_tag_word(abridged, fp);
        ensure!(
            derived == fp.ftw,
            "input tag word {:#06x} does not match the registers' contents (a CPU would report {derived:#06x}); generator bug",
            fp.ftw
        );
        ensure!(
            fp.mxcsr & !template.mxcr_mask == 0,
            "input MXCSR {:#x} sets bits outside MXCSR_MASK {:#x}",
            fp.mxcsr,
            template.mxcr_mask
        );
        let mut x = *template;
        x.cwd = fp.fcw;
        x.swd = fp.fsw;
        x.ftw = u16::from(abridged);
        x.fop = 0;
        x.rip = 0;
        x.rdp = 0;
        x.mxcsr = fp.mxcsr;
        for (i, &v) in fp.st.iter().enumerate() {
            set_slot(&mut x.st_space, i, v);
        }
        for (i, &v) in fp.xmm.iter().enumerate() {
            set_slot(&mut x.xmm_space, i, v);
        }
        Ok(x)
    }

    fn from_fxsave(x: &libc::user_fpregs_struct) -> FpState {
        let mut fp = FpState {
            xmm: std::array::from_fn(|i| slot(&x.xmm_space, i)),
            mxcsr: x.mxcsr,
            fcw: x.cwd,
            fsw: x.swd,
            ftw: 0,
            st: std::array::from_fn(|i| slot(&x.st_space, i) & ((1u128 << 80) - 1)),
        };
        fp.ftw = full_tag_word(x.ftw as u8, &fp);
        fp
    }

    impl Tracee {
        pub fn spawn() -> Result<Self> {
            // SAFETY: the child only makes async-signal-safe calls (see `child`).
            let pid = unsafe { libc::fork() };
            if pid < 0 {
                return Err(std::io::Error::last_os_error()).context("fork");
            }
            if pid == 0 {
                // SAFETY: we are the freshly forked child.
                unsafe { child() }
            }
            let status = wait(pid)?;
            if !(libc::WIFSTOPPED(status) && libc::WSTOPSIG(status) == libc::SIGSTOP) {
                bail!(
                    "recorder child {}; ptrace may be blocked in this environment",
                    describe(status)
                );
            }
            let mut tracee = Tracee {
                pid,
                mem: OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(format!("/proc/{pid}/mem"))
                    .context("opening the child's /proc/<pid>/mem")?,
                // SAFETY: user_regs_struct and user_fpregs_struct are plain old data.
                template: unsafe { std::mem::zeroed() },
                fp_template: unsafe { std::mem::zeroed() },
                code_page: vec![0; CODE_SIZE as usize],
            };
            tracee.template = tracee.get_regs()?;
            tracee.fp_template = tracee.get_fpregs()?;
            Ok(tracee)
        }

        fn get_regs(&self) -> Result<libc::user_regs_struct> {
            // SAFETY: plain old data, filled by the kernel.
            let mut regs: libc::user_regs_struct = unsafe { std::mem::zeroed() };
            // SAFETY: valid pid of a stopped tracee and a valid out pointer.
            let r = unsafe {
                libc::ptrace(
                    libc::PTRACE_GETREGS,
                    self.pid,
                    0,
                    &mut regs as *mut libc::user_regs_struct,
                )
            };
            ensure!(
                r == 0,
                "PTRACE_GETREGS: {}",
                std::io::Error::last_os_error()
            );
            Ok(regs)
        }

        fn get_fpregs(&self) -> Result<libc::user_fpregs_struct> {
            // SAFETY: plain old data, filled by the kernel.
            let mut fp: libc::user_fpregs_struct = unsafe { std::mem::zeroed() };
            // SAFETY: valid pid of a stopped tracee and a valid out pointer.
            let r = unsafe {
                libc::ptrace(
                    libc::PTRACE_GETFPREGS,
                    self.pid,
                    0,
                    &mut fp as *mut libc::user_fpregs_struct,
                )
            };
            ensure!(
                r == 0,
                "PTRACE_GETFPREGS: {}",
                std::io::Error::last_os_error()
            );
            Ok(fp)
        }

        fn siginfo(&self) -> Result<libc::siginfo_t> {
            // SAFETY: plain old data, filled by the kernel.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            // SAFETY: valid pid of a stopped tracee and a valid out pointer.
            let r = unsafe {
                libc::ptrace(
                    libc::PTRACE_GETSIGINFO,
                    self.pid,
                    0,
                    &mut info as *mut libc::siginfo_t,
                )
            };
            ensure!(
                r == 0,
                "PTRACE_GETSIGINFO: {}",
                std::io::Error::last_os_error()
            );
            Ok(info)
        }

        /// The fault a signal stop stands for (`docs/design/fixtures.md`).
        fn classify(&self, sig: i32, rip: u64) -> Result<Fault> {
            let info = self.siginfo()?;
            let code = info.si_code;
            // SAFETY: si_addr is valid for the fault signals handled here.
            let addr = unsafe { info.si_addr() as u64 };
            let kind = match (sig, code) {
                (libc::SIGFPE, FPE_INTDIV) => FaultKind::De,
                (libc::SIGILL, ILL_ILLOPN) => FaultKind::Ud,
                (libc::SIGSEGV, SI_KERNEL) => FaultKind::Gp,
                (libc::SIGSEGV, SEGV_MAPERR | SEGV_ACCERR) => FaultKind::Pf,
                _ => bail!(
                    "{} ({sig}) with si_code {code:#x} at rip {rip:#x}, address {addr:#x}: not a fault the fixtures record",
                    signal_name(sig)
                ),
            };
            Ok(Fault {
                kind,
                addr: if kind == FaultKind::Pf { addr } else { 0 },
            })
        }

        /// Runs the instruction `code` from `CODE_ADDR` with the given data
        /// window and the registers, bases and FP state of `input`
        /// (`input.mem` is ignored: `data` holds it).
        pub fn step(&mut self, code: &[u8], data: &[u8], input: &Input) -> Result<StepResult> {
            ensure!(code.len() <= 15 && data.len() == DATA_SIZE as usize);
            for (name, base) in [("fs_base", input.fs_base), ("gs_base", input.gs_base)] {
                ensure!(
                    base < MAX_SEGMENT_BASE,
                    "{name} {base:#x} is not below {MAX_SEGMENT_BASE:#x}; the kernel rejects it"
                );
            }
            let fx = to_fxsave(&self.fp_template, &input.fp)?;
            self.code_page.fill(0);
            self.code_page[..code.len()].copy_from_slice(code);
            self.mem
                .write_all_at(&self.code_page, CODE_ADDR)
                .context("writing the code page")?;
            self.mem
                .write_all_at(data, DATA_ADDR)
                .context("writing the data window")?;

            let mut regs = self.template;
            let g = &input.gprs;
            (regs.rax, regs.rcx, regs.rdx, regs.rbx) = (g[0], g[1], g[2], g[3]);
            (regs.rsp, regs.rbp, regs.rsi, regs.rdi) = (g[4], g[5], g[6], g[7]);
            (regs.r8, regs.r9, regs.r10, regs.r11) = (g[8], g[9], g[10], g[11]);
            (regs.r12, regs.r13, regs.r14, regs.r15) = (g[12], g[13], g[14], g[15]);
            regs.rip = CODE_ADDR;
            regs.eflags = input.rflags;
            regs.fs_base = input.fs_base;
            regs.gs_base = input.gs_base;
            // No system call to restart.
            regs.orig_rax = u64::MAX;
            // SAFETY: valid pid of a stopped tracee and a valid in pointer.
            let r = unsafe {
                libc::ptrace(
                    libc::PTRACE_SETREGS,
                    self.pid,
                    0,
                    &regs as *const libc::user_regs_struct,
                )
            };
            ensure!(
                r == 0,
                "PTRACE_SETREGS: {}",
                std::io::Error::last_os_error()
            );
            // SAFETY: valid pid of a stopped tracee and a valid in pointer.
            let r = unsafe {
                libc::ptrace(
                    libc::PTRACE_SETFPREGS,
                    self.pid,
                    0,
                    &fx as *const libc::user_fpregs_struct,
                )
            };
            ensure!(
                r == 0,
                "PTRACE_SETFPREGS: {}",
                std::io::Error::last_os_error()
            );
            let mut steps = 0;
            let (out, fault) = loop {
                steps += 1;
                ensure!(
                    steps <= MAX_STEPS,
                    "still at the instruction after {MAX_STEPS} single steps"
                );
                // SAFETY: valid pid of a stopped tracee.
                let r = unsafe { libc::ptrace(libc::PTRACE_SINGLESTEP, self.pid, 0, 0) };
                ensure!(
                    r == 0,
                    "PTRACE_SINGLESTEP: {}",
                    std::io::Error::last_os_error()
                );
                let status = wait(self.pid)?;
                if !libc::WIFSTOPPED(status) {
                    bail!("recorder child {}", describe(status));
                }
                let sig = libc::WSTOPSIG(status);
                let out = self.get_regs()?;
                if sig != libc::SIGTRAP {
                    // The next PTRACE_SINGLESTEP passes signal 0, which
                    // discards this one.
                    break (out, Some(self.classify(sig, out.rip)?));
                }
                if out.rip != CODE_ADDR {
                    break (out, None);
                }
            };
            ensure!(
                out.cs == self.template.cs && out.ss == self.template.ss,
                "segment registers changed"
            );
            // The kernel hides the TF it sets for PTRACE_SINGLESTEP; make sure.
            ensure!(
                out.eflags & flags::TF == 0,
                "TF leaked into the recorded rflags {:#x}",
                out.eflags
            );
            let fp = from_fxsave(&self.get_fpregs()?);
            let mut data_out = vec![0; DATA_SIZE as usize];
            self.mem
                .read_exact_at(&mut data_out, DATA_ADDR)
                .context("reading the data window")?;
            Ok(StepResult {
                gprs: [
                    out.rax, out.rcx, out.rdx, out.rbx, out.rsp, out.rbp, out.rsi, out.rdi, out.r8,
                    out.r9, out.r10, out.r11, out.r12, out.r13, out.r14, out.r15,
                ],
                rip: out.rip,
                rflags: out.eflags,
                fp,
                fault,
                steps,
                data: data_out,
            })
        }
    }

    impl Drop for Tracee {
        fn drop(&mut self) {
            // SAFETY: we own this child process.
            unsafe {
                libc::kill(self.pid, libc::SIGKILL);
                let mut status = 0;
                libc::waitpid(self.pid, &mut status, libc::__WALL);
            }
        }
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::*;
    use crate::oracle::fixture::Input;
    use crate::oracle::{CODE_ADDR, DATA_ADDR, DATA_SIZE};

    const RAX: usize = 0;
    const RCX: usize = 1;
    const RBX: usize = 3;
    const RDI: usize = 7;

    fn input(gprs: &[(usize, u64)]) -> Input {
        let mut g = [0x5a5a_0000_0000_0000u64; 16];
        for &(i, v) in gprs {
            g[i] = v;
        }
        Input::new(g, 0x202, Vec::new())
    }

    fn run(code: &[u8], data: &[u8], input: &Input) -> StepResult {
        let mut t = Tracee::spawn().expect("spawning the recorder child");
        t.step(code, data, input).expect("stepping")
    }

    fn window() -> Vec<u8> {
        (0..DATA_SIZE).map(|i| (i * 7 + 3) as u8).collect()
    }

    /// A faulting instruction leaves every register and memory unchanged.
    fn assert_unchanged(r: &StepResult, i: &Input, data: &[u8]) {
        assert_eq!(r.rip, CODE_ADDR);
        assert_eq!(r.gprs, i.gprs);
        assert_eq!(r.fp, i.fp);
        assert_eq!(r.steps, 1);
        assert!(r.data == data, "memory changed");
    }

    #[test]
    fn divide_by_zero_is_de() {
        let i = input(&[(RCX, 0)]);
        let data = window();
        let r = run(&[0x48, 0xf7, 0xf1], &data, &i); // div rcx
        assert_eq!(
            r.fault,
            Some(Fault {
                kind: FaultKind::De,
                addr: 0
            })
        );
        assert_unchanged(&r, &i, &data);
    }

    #[test]
    fn ud2_is_ud() {
        let i = input(&[]);
        let data = window();
        let r = run(&[0x0f, 0x0b], &data, &i);
        assert_eq!(r.fault.map(|f| f.kind), Some(FaultKind::Ud));
        assert_unchanged(&r, &i, &data);
    }

    #[test]
    fn misaligned_movaps_is_gp() {
        let i = input(&[(RAX, DATA_ADDR + 8)]);
        let data = window();
        let r = run(&[0x0f, 0x28, 0x00], &data, &i); // movaps xmm0, [rax]
        assert_eq!(
            r.fault,
            Some(Fault {
                kind: FaultKind::Gp,
                addr: 0
            })
        );
        assert_unchanged(&r, &i, &data);
    }

    #[test]
    fn unmapped_load_is_pf_with_address() {
        let i = input(&[(RBX, 0x1234)]);
        let data = window();
        let r = run(&[0x48, 0x8b, 0x03], &data, &i); // mov rax, [rbx]
        assert_eq!(
            r.fault,
            Some(Fault {
                kind: FaultKind::Pf,
                addr: 0x1234
            })
        );
        assert_unchanged(&r, &i, &data);
    }

    #[test]
    fn fs_and_gs_bases_apply() {
        let data = window();
        let at = |off: usize| u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        // mov rax, fs:[0x10] / mov rax, gs:[0x10]
        for (prefix, off) in [(0x64u8, 0x100usize), (0x65, 0x2000)] {
            let mut i = input(&[]);
            i.fs_base = DATA_ADDR + 0x100;
            i.gs_base = DATA_ADDR + 0x2000;
            let code = [prefix, 0x48, 0x8b, 0x04, 0x25, 0x10, 0, 0, 0];
            let r = run(&code, &data, &i);
            assert_eq!(r.fault, None);
            assert_eq!(r.gprs[RAX], at(off + 0x10), "prefix {prefix:#x}");
        }
    }

    #[test]
    fn rep_stosb_takes_one_step_per_iteration() {
        let data = window();
        let mut i = input(&[(RCX, 5), (RDI, DATA_ADDR + 0x100), (RAX, 0xab)]);
        let r = run(&[0xf3, 0xaa], &data, &i);
        assert_eq!(r.fault, None);
        assert_eq!(r.rip, CODE_ADDR + 2);
        assert_eq!(r.gprs[RCX], 0);
        assert_eq!(r.gprs[RDI], DATA_ADDR + 0x105);
        assert_eq!(r.steps, 5, "one step per iteration");
        assert_eq!(&r.data[0x100..0x105], &[0xab; 5]);
        assert_eq!(r.data[0x105], data[0x105]);
        // Count 0: one step, nothing written.
        i.gprs[RCX] = 0;
        let r = run(&[0xf3, 0xaa], &data, &i);
        assert_eq!(
            (r.steps, r.rip, r.gprs[RDI]),
            (1, CODE_ADDR + 2, DATA_ADDR + 0x100)
        );
        assert!(r.data == data);
    }

    #[test]
    fn xmm_and_mxcsr_round_trip() {
        let data = window();
        let mut i = input(&[(RAX, DATA_ADDR + 0x40)]);
        i.fp.xmm[1] = 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210;
        i.fp.xmm[2] = 0xffff_0000_ffff_0000_1111_2222_3333_4444;
        i.fp.xmm[9] = 0x9999;
        i.fp.mxcsr = 0x9fc0;
        let r = run(&[0x66, 0x0f, 0xef, 0xca], &data, &i); // pxor xmm1, xmm2
        assert_eq!(r.fault, None);
        assert_eq!(r.fp.xmm[1], i.fp.xmm[1] ^ i.fp.xmm[2]);
        assert_eq!(r.fp.xmm[9], 0x9999);
        assert_eq!(r.fp.mxcsr, 0x9fc0);
        let r = run(&[0x0f, 0xae, 0x18], &data, &i); // stmxcsr [rax]
        assert_eq!(&r.data[0x40..0x44], &0x9fc0u32.to_le_bytes());
    }

    #[test]
    fn fninit_resets_control_status_and_tags() {
        let data = window();
        let mut i = input(&[]);
        i.fp.fcw = 0x027f;
        i.fp.fsw = (7 << 11) | 0x0001; // TOP 7, IE
        i.fp.st[0] = 0x3fff_8000_0000_0000_0000; // 1.0 in R7
        i.fp.st[1] = 0x4000_c000_0000_0000_0000; // 3.0 in R0, tagged empty
        i.fp.ftw = 0x3fff;
        let r = run(&[0xdb, 0xe3], &data, &i);
        assert_eq!(r.fault, None);
        assert_eq!((r.fp.fcw, r.fp.fsw, r.fp.ftw), (0x037f, 0, 0xffff));
        // The registers keep their contents; ST(i) is relative to TOP.
        assert_eq!(r.fp.physical(), i.fp.physical());
        assert_eq!(r.fp.st[7], i.fp.st[0]);
        assert_eq!(r.fp.st[0], i.fp.st[1]);
    }

    #[test]
    fn rejects_an_inconsistent_tag_word() {
        let mut i = input(&[]);
        i.fp.ftw = 0xfffc; // R0 "valid" but it holds +0.0
        let mut t = Tracee::spawn().unwrap();
        let err = t.step(&[0x90], &window(), &i).unwrap_err();
        assert!(format!("{err:#}").contains("tag word"), "{err:#}");
    }
}
