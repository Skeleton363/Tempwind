//! Native execution of single instructions under ptrace (x86-64 Linux).
//!
//! A forked child calls `PTRACE_TRACEME`, maps the code page at `CODE_ADDR`
//! and the data window at `DATA_ADDR` (`MAP_FIXED_NOREPLACE`) and stops
//! itself with `SIGSTOP`. For each case the parent writes the code page and
//! the whole data window through `/proc/<pid>/mem`, sets every GPR, RIP and
//! RFLAGS with `PTRACE_SETREGS`, executes exactly one instruction with
//! `PTRACE_SINGLESTEP`, and reads registers and the data window back. Any
//! stop other than the single-step `SIGTRAP` is a recording bug.

/// Result of one native single step.
#[derive(Clone, Debug)]
pub struct StepResult {
    pub gprs: [u64; 16],
    pub rip: u64,
    pub rflags: u64,
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

    use super::StepResult;
    use crate::oracle::{CODE_ADDR, CODE_SIZE, DATA_ADDR, DATA_SIZE};

    const EXIT_TRACEME: i32 = 101;
    const EXIT_MAP_CODE: i32 = 102;
    const EXIT_MAP_DATA: i32 = 103;
    const EXIT_STOP: i32 = 104;

    /// A stopped child process that executes cases one instruction at a time.
    pub struct Tracee {
        pid: libc::pid_t,
        mem: File,
        template: libc::user_regs_struct,
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
                // SAFETY: user_regs_struct is plain old data.
                template: unsafe { std::mem::zeroed() },
                code_page: vec![0; CODE_SIZE as usize],
            };
            tracee.template = tracee.get_regs()?;
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
            ensure!(r == 0, "PTRACE_GETREGS: {}", std::io::Error::last_os_error());
            Ok(regs)
        }

        fn fault_addr(&self) -> u64 {
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
            if r != 0 {
                return 0;
            }
            // SAFETY: si_addr is valid for the fault signals we report.
            unsafe { info.si_addr() as u64 }
        }

        /// Executes the instruction `code` once from `CODE_ADDR` with the
        /// given registers and data window.
        pub fn step(
            &mut self,
            code: &[u8],
            data: &[u8],
            gprs: &[u64; 16],
            rflags: u64,
        ) -> Result<StepResult> {
            ensure!(code.len() <= 15 && data.len() == DATA_SIZE as usize);
            self.code_page.fill(0);
            self.code_page[..code.len()].copy_from_slice(code);
            self.mem
                .write_all_at(&self.code_page, CODE_ADDR)
                .context("writing the code page")?;
            self.mem
                .write_all_at(data, DATA_ADDR)
                .context("writing the data window")?;

            let mut regs = self.template;
            let g = gprs;
            (regs.rax, regs.rcx, regs.rdx, regs.rbx) = (g[0], g[1], g[2], g[3]);
            (regs.rsp, regs.rbp, regs.rsi, regs.rdi) = (g[4], g[5], g[6], g[7]);
            (regs.r8, regs.r9, regs.r10, regs.r11) = (g[8], g[9], g[10], g[11]);
            (regs.r12, regs.r13, regs.r14, regs.r15) = (g[12], g[13], g[14], g[15]);
            regs.rip = CODE_ADDR;
            regs.eflags = rflags;
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
            ensure!(r == 0, "PTRACE_SETREGS: {}", std::io::Error::last_os_error());
            // SAFETY: valid pid of a stopped tracee.
            let r = unsafe { libc::ptrace(libc::PTRACE_SINGLESTEP, self.pid, 0, 0) };
            ensure!(r == 0, "PTRACE_SINGLESTEP: {}", std::io::Error::last_os_error());
            let status = wait(self.pid)?;
            if !libc::WIFSTOPPED(status) {
                bail!("recorder child {}", describe(status));
            }
            let sig = libc::WSTOPSIG(status);
            let out = self.get_regs()?;
            if sig != libc::SIGTRAP {
                bail!(
                    "{} ({sig}) at rip {:#x}, fault address {:#x}: the case touches memory outside the mapped pages",
                    signal_name(sig),
                    out.rip,
                    self.fault_addr()
                );
            }
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
