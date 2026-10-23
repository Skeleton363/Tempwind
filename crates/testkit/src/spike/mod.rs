//! Spike programs: small x86-64 functions (assembled with iced-x86's
//! `CodeAssembler`) translated into wasm, plus the manifest the JS hosts run
//! (see `docs/design/abi.md`).
//!
//! Each case is a function using the Win64 argument registers (RCX, RDX)
//! that returns with `ret` to [`abi::SENTINEL_RET`], which the host pushed.
//! The expected results are computed in Rust; on x86-64 Unix hosts the tests
//! also check them by running each case natively ([`native::run_case`]).

pub mod bench;
#[cfg(all(target_arch = "x86_64", unix))]
pub mod native;

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use iced_x86::BlockEncoderOptions;
use iced_x86::code_asm::*;
use serde_json::{Map, Value, json};
use x64wasm::abi::{self, CpuState};
use x64wasm::{CodeRegion, Options, Translation};

use crate::host::{Host, Machine};

/// Register index by name (`abi::GPR_NAMES` order).
pub fn reg(name: &str) -> usize {
    abi::GPR_NAMES
        .iter()
        .position(|&n| n == name)
        .unwrap_or_else(|| panic!("unknown register {name}"))
}

/// One spike program plus its initial state and expected results.
#[derive(Clone, Debug)]
pub struct Case {
    pub name: &'static str,
    /// Guest address of `code`.
    pub code_base: u64,
    pub code: Vec<u8>,
    pub entry: u64,
    /// Further code addresses to translate from (address-taken functions
    /// and continuations that direct control flow doesn't reach).
    pub extra_entries: Vec<u64>,
    /// Guest memory the host must provide.
    pub memory_bytes: u64,
    /// Guest address of the CPU state block.
    pub state_addr: u64,
    /// Initial registers (RIP = entry); unlisted registers are 0.
    pub init_regs: Vec<(usize, u64)>,
    pub init_rflags: u64,
    /// Initial memory besides the code (which is always mapped at `code_base`).
    pub init_mem: Vec<(u64, Vec<u8>)>,
    pub expect_exit: u32,
    pub expect_rip: u64,
    pub expect_regs: Vec<(usize, u64)>,
    pub expect_mem: Vec<(u64, Vec<u8>)>,
    /// Expected RFLAGS under a mask, if checked.
    pub expect_rflags: Option<(u64, u64)>,
}

impl Case {
    /// The translation entry points: the entry, then the extra entries.
    pub fn entries(&self) -> Vec<u64> {
        let mut entries = vec![self.entry];
        entries.extend(&self.extra_entries);
        entries
    }

    pub fn translate(&self, opts: &Options) -> Result<Translation> {
        x64wasm::translate(
            CodeRegion {
                base: self.code_base,
                bytes: &self.code,
            },
            &self.entries(),
            opts,
        )
        .map_err(|e| anyhow::anyhow!("translating {}: {e}", self.name))
    }

    /// All memory written before the run: the code, then `init_mem`.
    pub fn all_init_mem(&self) -> Vec<(u64, Vec<u8>)> {
        let mut mem = vec![(self.code_base, self.code.clone())];
        mem.extend(self.init_mem.iter().cloned());
        mem
    }

    pub fn init_state(&self) -> CpuState {
        let mut state = CpuState {
            rip: self.entry,
            rflags: self.init_rflags,
            ..CpuState::default()
        };
        for &(r, v) in &self.init_regs {
            state.gpr[r] = v;
        }
        state
    }

    /// Runs the case in wasmtime and returns the final state and machine.
    pub fn run(&self, host: &Host, wasm: &[u8]) -> Result<(CpuState, Machine)> {
        let module = host.compile(wasm)?;
        let mut machine = host.machine(self.memory_bytes)?;
        for (addr, bytes) in self.all_init_mem() {
            machine.write(addr, &bytes)?;
        }
        let out = machine.run_state(&module, self.state_addr, &self.init_state())?;
        Ok((out, machine))
    }

    /// Checks a final state against the expectations.
    pub fn check(&self, out: &CpuState, machine: &Machine) -> Result<()> {
        self.check_with(out, |addr, len| machine.read(addr, len))
    }

    /// Like [`Case::check`], reading memory through `read(addr, len)`.
    pub fn check_with(
        &self,
        out: &CpuState,
        read: impl Fn(u64, usize) -> Result<Vec<u8>>,
    ) -> Result<()> {
        ensure!(
            out.exit == self.expect_exit,
            "{}: exit {} (expected {}), rip {:#x}",
            self.name,
            out.exit,
            self.expect_exit,
            out.rip
        );
        ensure!(
            out.rip == self.expect_rip,
            "{}: rip {:#x}, expected {:#x}",
            self.name,
            out.rip,
            self.expect_rip
        );
        for &(r, v) in &self.expect_regs {
            ensure!(
                out.gpr[r] == v,
                "{}: {} = {:#x}, expected {v:#x}",
                self.name,
                abi::GPR_NAMES[r],
                out.gpr[r]
            );
        }
        if let Some((value, mask)) = self.expect_rflags {
            ensure!(
                out.rflags & mask == value & mask,
                "{}: rflags {:#x}, expected {value:#x} under mask {mask:#x}",
                self.name,
                out.rflags
            );
        }
        for (addr, bytes) in &self.expect_mem {
            let got = read(*addr, bytes.len())?;
            ensure!(
                &got == bytes,
                "{}: memory at {addr:#x} is {}, expected {}",
                self.name,
                hex(&got),
                hex(bytes)
            );
        }
        Ok(())
    }

    fn manifest_entry(&self, wasm_file: &str) -> Value {
        let regs = |list: &[(usize, u64)]| {
            let mut m = Map::new();
            for &(r, v) in list {
                m.insert(abi::GPR_NAMES[r].to_string(), json!(h64(v)));
            }
            Value::Object(m)
        };
        let mem = |list: &[(u64, Vec<u8>)]| {
            list.iter()
                .map(|(a, b)| json!({ "addr": h64(*a), "hex": hex(b) }))
                .collect::<Vec<_>>()
        };
        let mut expect = json!({
            "exit": self.expect_exit,
            "rip": h64(self.expect_rip),
            "regs": regs(&self.expect_regs),
            "mem": mem(&self.expect_mem),
        });
        if let Some((value, mask)) = self.expect_rflags {
            expect["rflags"] = json!(h64(value));
            expect["rflagsMask"] = json!(h64(mask));
        }
        json!({
            "name": self.name,
            "wasm": wasm_file,
            "memoryBytes": h64(self.memory_bytes),
            "state": h64(self.state_addr),
            "init": {
                "rip": h64(self.entry),
                "rflags": h64(self.init_rflags),
                "regs": regs(&self.init_regs),
                "mem": mem(&self.all_init_mem()),
            },
            "expect": expect,
        })
    }
}

fn h64(v: u64) -> String {
    format!("{v:#x}")
}

/// Lowercase hex, a space after every 4 bytes.
pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2 + bytes.len() / 4);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            s.push(' ');
        }
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn u64s(values: &[u64]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn asm(a: &mut CodeAssembler, base: u64) -> Result<Vec<u8>> {
    Ok(a.assemble(base)?)
}

// Low-memory layout shared by the small cases.
const LOW_MEMORY: u64 = 0x40_0000;
const LOW_CODE: u64 = 0x10_0000;
const LOW_DATA: u64 = 0x20_0000;
const LOW_STACK: u64 = 0x2f_f000;
const LOW_STATE: u64 = 0x30_0000;
const INIT_RFLAGS: u64 = 0x202;

fn sentinel_at(stack: u64) -> (u64, Vec<u8>) {
    (stack, abi::SENTINEL_RET.to_le_bytes().to_vec())
}

/// Machine code of `sum_loop`: `rax = 1 + 2 + ... + rcx`.
pub fn sum_loop_code(base: u64) -> Result<Vec<u8>> {
    let mut a = CodeAssembler::new(64)?;
    let mut top = a.create_label();
    let mut done = a.create_label();
    a.mov(eax, 0)?;
    a.cmp(rcx, 0)?;
    a.je(done)?;
    a.set_label(&mut top)?;
    a.add(rax, rcx)?;
    a.sub(rcx, 1)?;
    a.jne(top)?;
    a.set_label(&mut done)?;
    a.ret()?;
    asm(&mut a, base)
}

/// (a) sum_loop: `rax = sum 1..=n` with add/sub/cmp/jcc.
pub fn sum_loop(n: u64) -> Result<Case> {
    let sum = (n as u128 * (n as u128 + 1) / 2) as u64;
    // The last flag writer is `sub rcx, 1` reaching 0 (ZF, PF set), or
    // `cmp rcx, 0` with rcx = 0 (same flags).
    let flags = INIT_RFLAGS | abi::flags::ZF | abi::flags::PF;
    Ok(Case {
        name: "sum_loop",
        code_base: LOW_CODE,
        code: sum_loop_code(LOW_CODE)?,
        entry: LOW_CODE,
        extra_entries: vec![],
        memory_bytes: LOW_MEMORY,
        state_addr: LOW_STATE,
        init_regs: vec![(reg("rcx"), n), (reg("rsp"), LOW_STACK)],
        init_rflags: INIT_RFLAGS,
        init_mem: vec![sentinel_at(LOW_STACK)],
        expect_exit: abi::EXIT_LEFT_CODE,
        expect_rip: abi::SENTINEL_RET,
        expect_regs: vec![
            (reg("rax"), sum),
            (reg("rcx"), 0),
            (reg("rsp"), LOW_STACK + 8),
        ],
        expect_mem: vec![],
        expect_rflags: Some((flags, u64::MAX)),
    })
}

/// Reference fib.
pub fn fib(n: u64) -> u64 {
    let (mut a, mut b) = (0u64, 1u64);
    for _ in 0..n {
        (a, b) = (b, a.wrapping_add(b));
    }
    a
}

/// (b) fib_recursive: `rax = fib(rcx)` with call/ret/push/pop; RBX and RCX
/// are preserved.
pub fn fib_recursive(n: u64) -> Result<Case> {
    let mut a = CodeAssembler::new(64)?;
    let mut fib_l = a.create_label();
    let mut base_case = a.create_label();
    a.set_label(&mut fib_l)?;
    a.cmp(rcx, 2)?;
    a.jb(base_case)?;
    a.push(rbx)?;
    a.push(rcx)?;
    a.sub(rcx, 1)?;
    a.call(fib_l)?;
    a.mov(rbx, rax)?;
    a.pop(rcx)?;
    a.push(rcx)?;
    a.sub(rcx, 2)?;
    a.call(fib_l)?;
    a.add(rax, rbx)?;
    a.pop(rcx)?;
    a.pop(rbx)?;
    a.ret()?;
    a.set_label(&mut base_case)?;
    a.mov(rax, rcx)?;
    a.ret()?;
    let rbx_marker = 0x0123_4567_89ab_cdef;
    Ok(Case {
        name: "fib_recursive",
        code_base: LOW_CODE,
        code: asm(&mut a, LOW_CODE)?,
        entry: LOW_CODE,
        extra_entries: vec![],
        memory_bytes: LOW_MEMORY,
        state_addr: LOW_STATE,
        init_regs: vec![
            (reg("rcx"), n),
            (reg("rbx"), rbx_marker),
            (reg("rsp"), LOW_STACK),
        ],
        init_rflags: INIT_RFLAGS,
        init_mem: vec![sentinel_at(LOW_STACK)],
        expect_exit: abi::EXIT_LEFT_CODE,
        expect_rip: abi::SENTINEL_RET,
        expect_regs: vec![
            (reg("rax"), fib(n)),
            (reg("rbx"), rbx_marker),
            (reg("rcx"), n),
            (reg("rsp"), LOW_STACK + 8),
        ],
        expect_mem: vec![],
        expect_rflags: None,
    })
}

/// (c) array: fills `a[i] = 3i + 7` for `i < rcx` at `rdx` with
/// `[base + index*8]` stores, sums it walking a pointer with lea, and stores
/// the low byte of the sum after the array.
pub fn array(n: u64) -> Result<Case> {
    let mut a = CodeAssembler::new(64)?;
    let mut fill = a.create_label();
    let mut fill_done = a.create_label();
    let mut sum = a.create_label();
    let mut done = a.create_label();
    a.mov(r8d, 0)?;
    a.set_label(&mut fill)?;
    a.cmp(r8, rcx)?;
    a.jae(fill_done)?;
    a.lea(r9, ptr(r8 + r8 * 2 + 7))?;
    a.mov(qword_ptr(rdx + r8 * 8), r9)?;
    a.add(r8, 1)?;
    a.jmp(fill)?;
    a.set_label(&mut fill_done)?;
    a.mov(eax, 0)?;
    a.lea(r10, ptr(rdx + rcx * 8))?;
    a.mov(r11, rdx)?;
    a.set_label(&mut sum)?;
    a.cmp(r11, r10)?;
    a.jae(done)?;
    a.add(rax, qword_ptr(r11))?;
    a.lea(r11, ptr(r11 + 8))?;
    a.jmp(sum)?;
    a.set_label(&mut done)?;
    a.mov(byte_ptr(r10), al)?;
    a.ret()?;

    let values: Vec<u64> = (0..n).map(|i| 3 * i + 7).collect();
    let total: u64 = values.iter().sum();
    let mut expect = u64s(&values);
    expect.push(total as u8);
    Ok(Case {
        name: "array",
        code_base: LOW_CODE,
        code: asm(&mut a, LOW_CODE)?,
        entry: LOW_CODE,
        extra_entries: vec![],
        memory_bytes: LOW_MEMORY,
        state_addr: LOW_STATE,
        init_regs: vec![
            (reg("rcx"), n),
            (reg("rdx"), LOW_DATA),
            (reg("rsp"), LOW_STACK),
        ],
        init_rflags: INIT_RFLAGS,
        init_mem: vec![sentinel_at(LOW_STACK)],
        expect_exit: abi::EXIT_LEFT_CODE,
        expect_rip: abi::SENTINEL_RET,
        expect_regs: vec![
            (reg("rax"), total),
            (reg("r8"), n),
            (reg("r10"), LOW_DATA + 8 * n),
            (reg("r11"), LOW_DATA + 8 * n),
            (reg("rsp"), LOW_STACK + 8),
        ],
        expect_mem: vec![(LOW_DATA, expect)],
        expect_rflags: None,
    })
}

/// Code base of the high-address case (a typical x64 EXE image base + 0x1000).
pub const HIGH_CODE: u64 = 0x1_4000_1000;
const HIGH_STACK: u64 = 0x1_4007_f000;
const HIGH_DATA: u64 = 0x1_4009_0000;
const HIGH_MEMORY: u64 = 0x1_400a_0000;
const HIGH_STATE: u64 = 0x1_0000;

/// (d) high_address: code at 0x1_4000_1000, stack and data above 4 GiB,
/// RIP-relative lea and load, call/ret and stores at 64-bit addresses.
pub fn high_address() -> Result<Case> {
    let mut a = CodeAssembler::new(64)?;
    let mut entry = a.create_label();
    let mut helper = a.create_label();
    let mut ret_site = a.create_label();
    let mut data_ptr = a.create_label();
    a.set_label(&mut entry)?;
    a.lea(rax, ptr(entry))?;
    a.mov(rdx, qword_ptr(data_ptr))?;
    a.mov(rcx, 5u64)?;
    a.push(rax)?;
    a.call(helper)?;
    a.set_label(&mut ret_site)?;
    a.pop(r8)?;
    a.ret()?;
    a.set_label(&mut helper)?;
    a.mov(r9, rsp)?;
    a.mov(qword_ptr(rdx), r9)?;
    a.mov(r10, qword_ptr(rsp))?;
    a.mov(qword_ptr(rdx + 8), r10)?;
    a.mov(dword_ptr(rdx + 16), 0x89ab_cdefu32)?;
    a.add(qword_ptr(rdx + 16), 0x11)?;
    a.lea(r11, ptr(rdx + rcx * 8 + 0x100))?;
    a.mov(qword_ptr(r11), r11)?;
    a.ret()?;
    a.set_label(&mut data_ptr)?;
    a.dq(&[HIGH_DATA])?;
    let result = a.assemble_options(
        HIGH_CODE,
        BlockEncoderOptions::RETURN_NEW_INSTRUCTION_OFFSETS,
    )?;
    let ret_addr = result.label_ip(&ret_site)?;
    let code = result.inner.code_buffer;

    let r11_value = HIGH_DATA + 5 * 8 + 0x100;
    let data = u64s(&[HIGH_STACK - 16, ret_addr, 0x89ab_ce00]);
    Ok(Case {
        name: "high_address",
        code_base: HIGH_CODE,
        code,
        entry: HIGH_CODE,
        extra_entries: vec![],
        memory_bytes: HIGH_MEMORY,
        state_addr: HIGH_STATE,
        init_regs: vec![(reg("rsp"), HIGH_STACK)],
        init_rflags: INIT_RFLAGS,
        init_mem: vec![sentinel_at(HIGH_STACK)],
        expect_exit: abi::EXIT_LEFT_CODE,
        expect_rip: abi::SENTINEL_RET,
        expect_regs: vec![
            (reg("rax"), HIGH_CODE),
            (reg("rcx"), 5),
            (reg("rdx"), HIGH_DATA),
            (reg("rsp"), HIGH_STACK + 8),
            (reg("r8"), HIGH_CODE),
            (reg("r9"), HIGH_STACK - 16),
            (reg("r10"), ret_addr),
            (reg("r11"), r11_value),
        ],
        expect_mem: vec![
            (HIGH_DATA, data),
            (r11_value, r11_value.to_le_bytes().to_vec()),
            (HIGH_STACK - 8, HIGH_CODE.to_le_bytes().to_vec()),
        ],
        expect_rflags: None,
    })
}

/// A case in the low-memory layout: code at `LOW_CODE` entered at its first
/// byte, RSP = `LOW_STACK` holding the sentinel. `init_regs` and
/// `expect_regs` come after RSP (initial `LOW_STACK`, expected
/// `LOW_STACK + 8`).
fn low_case(
    name: &'static str,
    code: Vec<u8>,
    init_regs: &[(&str, u64)],
    expect_regs: &[(&str, u64)],
) -> Case {
    let regs = |list: &[(&str, u64)]| list.iter().map(|&(r, v)| (reg(r), v)).collect::<Vec<_>>();
    let mut init = vec![(reg("rsp"), LOW_STACK)];
    init.extend(regs(init_regs));
    let mut expect = vec![(reg("rsp"), LOW_STACK + 8)];
    expect.extend(regs(expect_regs));
    Case {
        name,
        code_base: LOW_CODE,
        code,
        entry: LOW_CODE,
        extra_entries: vec![],
        memory_bytes: LOW_MEMORY,
        state_addr: LOW_STATE,
        init_regs: init,
        init_rflags: INIT_RFLAGS,
        init_mem: vec![sentinel_at(LOW_STACK)],
        expect_exit: abi::EXIT_LEFT_CODE,
        expect_rip: abi::SENTINEL_RET,
        expect_regs: expect,
        expect_mem: vec![],
        expect_rflags: None,
    }
}

/// Assembles at `base` and returns the code and the addresses of `labels`.
fn asm_with_labels(
    a: &mut CodeAssembler,
    base: u64,
    labels: &[CodeLabel],
) -> Result<(Vec<u8>, Vec<u64>)> {
    asm_with_labels_opts(a, base, labels, BlockEncoderOptions::NONE)
}

fn asm_with_labels_opts(
    a: &mut CodeAssembler,
    base: u64,
    labels: &[CodeLabel],
    options: u32,
) -> Result<(Vec<u8>, Vec<u64>)> {
    let result = a.assemble_options(
        base,
        options | BlockEncoderOptions::RETURN_NEW_INSTRUCTION_OFFSETS,
    )?;
    let addrs = labels
        .iter()
        .map(|l| result.label_ip(l))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((result.inner.code_buffer, addrs))
}

/// Value preserved in callee-saved registers by the cases below.
const MARKER: u64 = 0x0123_4567_89ab_cdef;

/// fib_rec: `rax = fib(rcx)` compiled the way a C compiler does it for
/// Win64: RBP frame, 32-byte shadow space, the argument spilled to its home
/// slot, RBX callee-saved.
pub fn fib_rec(n: u64) -> Result<Case> {
    let mut a = CodeAssembler::new(64)?;
    let mut fib_l = a.create_label();
    let mut small = a.create_label();
    let mut out = a.create_label();
    a.set_label(&mut fib_l)?;
    a.push(rbp)?;
    a.mov(rbp, rsp)?;
    a.push(rbx)?;
    a.sub(rsp, 40)?;
    a.mov(qword_ptr(rbp + 16), rcx)?;
    a.cmp(rcx, 2)?;
    a.jb(small)?;
    a.sub(rcx, 1)?;
    a.call(fib_l)?;
    a.mov(rbx, rax)?;
    a.mov(rcx, qword_ptr(rbp + 16))?;
    a.sub(rcx, 2)?;
    a.call(fib_l)?;
    a.add(rax, rbx)?;
    a.jmp(out)?;
    a.set_label(&mut small)?;
    a.mov(rax, rcx)?;
    a.set_label(&mut out)?;
    a.lea(rsp, ptr(rbp - 8))?;
    a.pop(rbx)?;
    a.pop(rbp)?;
    a.ret()?;
    let rbp_marker = 0x7777_0000_1111_2222;
    Ok(low_case(
        "fib_rec",
        asm(&mut a, LOW_CODE)?,
        &[("rcx", n), ("rbx", MARKER), ("rbp", rbp_marker)],
        &[("rax", fib(n)), ("rbx", MARKER), ("rbp", rbp_marker)],
    ))
}

// Layout of deep_rec: a 31 MiB stack.
const DEEP_MEMORY: u64 = 0x200_0000;
const DEEP_STACK: u64 = 0x1ff_f000;

/// deep_rec: `rax = 1 + 2 + ... + rcx` by plain recursion (one call per
/// level), far deeper than the call-depth limit.
pub fn deep_rec(n: u64) -> Result<Case> {
    let mut a = CodeAssembler::new(64)?;
    let mut sum = a.create_label();
    let mut zero = a.create_label();
    a.set_label(&mut sum)?;
    a.cmp(rcx, 0)?;
    a.je(zero)?;
    a.push(rcx)?;
    a.sub(rcx, 1)?;
    a.call(sum)?;
    a.pop(rcx)?;
    a.add(rax, rcx)?;
    a.ret()?;
    a.set_label(&mut zero)?;
    a.mov(eax, 0)?;
    a.ret()?;
    let total = (n as u128 * (n as u128 + 1) / 2) as u64;
    Ok(Case {
        memory_bytes: DEEP_MEMORY,
        init_regs: vec![(reg("rsp"), DEEP_STACK), (reg("rcx"), n)],
        init_mem: vec![sentinel_at(DEEP_STACK)],
        expect_regs: vec![
            (reg("rsp"), DEEP_STACK + 8),
            (reg("rax"), total),
            (reg("rcx"), n),
        ],
        ..low_case("deep_rec", asm(&mut a, LOW_CODE)?, &[], &[])
    })
}

/// Reference for tail_rec: alternate ping (`rax += rcx`) and pong
/// (`rax += 3`), decrementing `rcx` to 0.
fn ping_pong(n: u64, start_ping: bool) -> u64 {
    let (mut acc, mut ping) = (0u64, start_ping);
    for k in (1..=n).rev() {
        acc = acc.wrapping_add(if ping { k } else { 3 });
        ping = !ping;
    }
    acc
}

/// tail_rec: two functions (ping, pong) that call each other through tail
/// jumps (`jmp`, not `call`) `rcx` times, from each of them in turn.
pub fn tail_rec(n: u64) -> Result<Case> {
    let mut a = CodeAssembler::new(64)?;
    let mut ping = a.create_label();
    let mut pong = a.create_label();
    let mut ping_done = a.create_label();
    let mut pong_done = a.create_label();
    a.mov(eax, 0)?;
    a.mov(rdx, rcx)?;
    a.call(ping)?;
    a.mov(r8, rax)?;
    a.mov(eax, 0)?;
    a.mov(rcx, rdx)?;
    a.call(pong)?;
    a.ret()?;
    a.set_label(&mut ping)?;
    a.cmp(rcx, 0)?;
    a.je(ping_done)?;
    a.add(rax, rcx)?;
    a.sub(rcx, 1)?;
    a.jmp(pong)?;
    a.set_label(&mut ping_done)?;
    a.ret()?;
    a.set_label(&mut pong)?;
    a.cmp(rcx, 0)?;
    a.je(pong_done)?;
    a.add(rax, 3)?;
    a.sub(rcx, 1)?;
    a.jmp(ping)?;
    a.set_label(&mut pong_done)?;
    a.ret()?;
    Ok(low_case(
        "tail_rec",
        asm(&mut a, LOW_CODE)?,
        &[("rcx", n)],
        &[
            ("rax", ping_pong(n, false)),
            ("r8", ping_pong(n, true)),
            ("rcx", 0),
            ("rdx", n),
        ],
    ))
}

/// ret_tricks: a callee that adds 6 to its return address to skip a 6-byte
/// `je` after the call; a `pop rax; jmp rax` return; the `call next; next:
/// pop rax` get-PC idiom.
pub fn ret_tricks() -> Result<Case> {
    let mut a = CodeAssembler::new(64)?;
    let mut skip6 = a.create_label();
    let mut junk = a.create_label();
    let mut after = a.create_label();
    let mut popret = a.create_label();
    let mut next = a.create_label();
    let mut bad = a.create_label();
    a.push(rbx)?;
    a.mov(ebx, 0)?;
    a.call(skip6)?;
    a.set_label(&mut junk)?;
    // Skipped: a near je (0f 84 rel32), whose fall-through is a block.
    a.set_prefer_short_branch(false);
    a.je(bad)?;
    a.set_prefer_short_branch(true);
    a.set_label(&mut after)?;
    a.add(rbx, 1)?;
    a.call(popret)?;
    a.add(rbx, 10)?;
    a.call(next)?;
    a.set_label(&mut next)?;
    a.pop(rax)?;
    a.add(rbx, 100)?;
    a.mov(rdx, rbx)?;
    a.pop(rbx)?;
    a.ret()?;
    a.set_label(&mut skip6)?;
    a.add(qword_ptr(rsp), 6)?;
    a.ret()?;
    a.set_label(&mut popret)?;
    a.mov(r9d, 7)?;
    a.pop(rax)?;
    a.jmp(rax)?;
    a.set_label(&mut bad)?;
    a.mov(edx, 0xbad)?;
    a.ret()?;
    // Keep every branch in the form it was created in (the je near).
    let (code, addrs) = asm_with_labels_opts(
        &mut a,
        LOW_CODE,
        &[junk, after, next],
        BlockEncoderOptions::DONT_FIX_BRANCHES,
    )?;
    ensure!(addrs[1] - addrs[0] == 6, "the skipped je is not 6 bytes");
    Ok(low_case(
        "ret_tricks",
        code,
        &[("rbx", MARKER)],
        &[("rax", addrs[2]), ("rbx", MARKER), ("rdx", 111), ("r9", 7)],
    ))
}

/// call_table: indirect calls through a table of function pointers in
/// guest memory (`call [rbx + rcx*8]`) and through a register. The targets
/// are only reached through the table (translated as extra entries), except
/// `g`, which is also called directly; one entry points at a block in the
/// middle of `g`.
pub fn call_table() -> Result<Case> {
    let mut a = CodeAssembler::new(64)?;
    let mut top = a.create_label();
    let mut f_one = a.create_label();
    let mut f_add10 = a.create_label();
    let mut f_triple = a.create_label();
    let mut g = a.create_label();
    let mut g_mid = a.create_label();
    a.push(rbx)?;
    a.push(rsi)?;
    a.mov(rbx, LOW_DATA)?;
    a.mov(esi, 0)?;
    a.mov(ecx, 0)?;
    a.set_label(&mut top)?;
    a.mov(eax, 0)?;
    a.call(qword_ptr(rbx + rcx * 8))?;
    a.add(rsi, rax)?;
    a.add(rcx, 1)?;
    a.cmp(rcx, 5)?;
    a.jb(top)?;
    a.call(g)?;
    a.add(rsi, rax)?;
    a.mov(rdx, qword_ptr(rbx + 8))?;
    a.mov(eax, 5)?;
    a.call(rdx)?;
    a.add(rax, rsi)?;
    a.pop(rsi)?;
    a.pop(rbx)?;
    a.ret()?;
    a.set_label(&mut f_one)?;
    a.mov(eax, 1)?;
    a.ret()?;
    a.set_label(&mut f_add10)?;
    a.add(rax, 10)?;
    a.ret()?;
    a.set_label(&mut g)?;
    a.mov(eax, 100)?;
    a.cmp(rcx, 2)?;
    a.jne(g_mid)?;
    a.add(rax, 5000)?;
    a.set_label(&mut g_mid)?;
    a.add(rax, 1000)?;
    a.ret()?;
    a.set_label(&mut f_triple)?;
    a.lea(rax, ptr(rcx + rcx * 2))?;
    a.ret()?;
    let (code, addrs) = asm_with_labels(&mut a, LOW_CODE, &[f_one, f_add10, g_mid, f_triple, g])?;
    // Table calls: 1, 0 + 10, 0 + 1000, 3 * 3, g(4) = 1100; then g(5) =
    // 1100 directly and 5 + 10 through rdx.
    let total = 1 + 10 + 1000 + 9 + 1100 + 1100 + 15;
    Ok(Case {
        extra_entries: vec![addrs[0], addrs[1], addrs[3]],
        init_mem: vec![sentinel_at(LOW_STACK), (LOW_DATA, u64s(&addrs))],
        ..low_case(
            "call_table",
            code,
            &[("rbx", MARKER), ("rsi", !MARKER)],
            &[
                ("rax", total),
                ("rbx", MARKER),
                ("rsi", !MARKER),
                ("rcx", 5),
                ("rdx", addrs[1]),
            ],
        )
    })
}

/// shared_tail: two functions that end by jumping into one shared epilogue
/// block (the first function owns it; the second reaches it by a tail
/// transfer).
pub fn shared_tail() -> Result<Case> {
    let mut a = CodeAssembler::new(64)?;
    let mut fa = a.create_label();
    let mut fb = a.create_label();
    let mut epi = a.create_label();
    a.push(rbx)?;
    a.mov(ecx, 5)?;
    a.call(fa)?;
    a.mov(r10, rax)?;
    a.mov(ecx, 7)?;
    a.call(fb)?;
    a.add(rax, r10)?;
    a.pop(rbx)?;
    a.ret()?;
    a.set_label(&mut fa)?;
    a.push(rbx)?;
    a.push(rsi)?;
    a.mov(rbx, rcx)?;
    a.add(rbx, rbx)?;
    a.lea(rsi, ptr(rbx + 1))?;
    a.mov(rax, rsi)?;
    a.jmp(epi)?;
    a.set_label(&mut fb)?;
    a.push(rbx)?;
    a.push(rsi)?;
    a.lea(rax, ptr(rcx + rcx * 2))?;
    a.mov(rbx, rax)?;
    a.mov(rsi, rax)?;
    a.jmp(epi)?;
    a.set_label(&mut epi)?;
    a.pop(rsi)?;
    a.pop(rbx)?;
    a.ret()?;
    Ok(low_case(
        "shared_tail",
        asm(&mut a, LOW_CODE)?,
        &[("rbx", MARKER), ("rsi", !MARKER)],
        &[
            ("rax", 11 + 21),
            ("r10", 11),
            ("rbx", MARKER),
            ("rsi", !MARKER),
        ],
    ))
}

/// longjmp_like: `setjmp` saves its return address and RSP in a buffer;
/// `level` recurses 6 levels deep; the deepest calls `longjmp`, which
/// restores RSP and jumps to the saved address, skipping every return.
pub fn longjmp_like() -> Result<Case> {
    let mut a = CodeAssembler::new(64)?;
    let mut setjmp = a.create_label();
    let mut longjmp = a.create_label();
    let mut level = a.create_label();
    let mut deepest = a.create_label();
    let mut resumed = a.create_label();
    let mut out = a.create_label();
    let mut setjmp_ret = a.create_label();
    a.push(rbx)?;
    a.push(rbp)?;
    a.mov(rbx, LOW_DATA)?;
    a.mov(rcx, rbx)?;
    a.call(setjmp)?;
    a.set_label(&mut setjmp_ret)?;
    a.cmp(rax, 0)?;
    a.jne(resumed)?;
    a.mov(ecx, 5)?;
    a.mov(ebp, 0)?;
    a.call(level)?;
    a.mov(eax, 0xbad)?;
    a.jmp(out)?;
    a.set_label(&mut resumed)?;
    a.mov(rdx, rax)?;
    a.mov(rax, rbp)?;
    a.set_label(&mut out)?;
    a.pop(rbp)?;
    a.pop(rbx)?;
    a.ret()?;
    a.set_label(&mut setjmp)?;
    a.mov(rax, qword_ptr(rsp))?;
    a.mov(qword_ptr(rcx), rax)?;
    a.lea(rax, ptr(rsp + 8))?;
    a.mov(qword_ptr(rcx + 8), rax)?;
    a.mov(eax, 0)?;
    a.ret()?;
    a.set_label(&mut level)?;
    a.add(rbp, 1)?;
    a.cmp(rcx, 0)?;
    a.je(deepest)?;
    a.push(rcx)?;
    a.sub(rcx, 1)?;
    a.call(level)?;
    a.pop(rcx)?;
    a.ret()?;
    a.set_label(&mut deepest)?;
    a.mov(rcx, rbx)?;
    a.mov(edx, 42)?;
    a.call(longjmp)?;
    a.mov(eax, 0xbad2)?;
    a.ret()?;
    a.set_label(&mut longjmp)?;
    a.mov(rax, rdx)?;
    a.mov(rsp, qword_ptr(rcx + 8))?;
    a.jmp(qword_ptr(rcx))?;
    let (code, addrs) = asm_with_labels(&mut a, LOW_CODE, &[setjmp_ret])?;
    let rbp_marker = 0x5555_aaaa_0000_ffff;
    Ok(Case {
        expect_mem: vec![(LOW_DATA, u64s(&[addrs[0], LOW_STACK - 16]))],
        ..low_case(
            "longjmp_like",
            code,
            &[("rbx", MARKER), ("rbp", rbp_marker)],
            &[
                ("rax", 6),
                ("rdx", 42),
                ("rbx", MARKER),
                ("rbp", rbp_marker),
            ],
        )
    })
}

/// big_function: one generated function of about 2800 instructions (more
/// than the default `max_function_insns`): `segments` small loops of three
/// blocks each, each followed by straight-line code, and a taken forward
/// branch from a quarter of the way in to three quarters, which crosses
/// chunks.
pub fn big_function(segments: u64) -> Result<Case> {
    let mut a = CodeAssembler::new(64)?;
    let mut labels: Vec<CodeLabel> = (0..segments).map(|_| a.create_label()).collect();
    let skip_from = segments / 4;
    let skip_to = 3 * segments / 4;
    // Reference model of the registers the code computes.
    let mut m = [0u64, 1, 0, 0, 0]; // rax, rdx, r8, r9, r10
    a.mov(eax, 0)?;
    a.mov(edx, 1)?;
    a.mov(r8d, 0)?;
    for s in 0..segments {
        if s == skip_from {
            a.cmp(r11, 1)?;
            a.je(labels[skip_to as usize])?;
        }
        a.set_label(&mut labels[s as usize])?;
        let live = !(skip_from..skip_to).contains(&s);
        let iterations = 2 + s % 5;
        let k = (s & 0x7f) as i32;
        let mut top = a.create_label();
        let mut mid = a.create_label();
        a.mov(ecx, iterations as u32)?;
        a.set_label(&mut top)?;
        a.add(rax, rdx)?;
        a.cmp(rcx, 2)?;
        a.jne(mid)?;
        a.add(rax, 1)?;
        a.set_label(&mut mid)?;
        a.add(rdx, k)?;
        a.sub(rcx, 1)?;
        a.jne(top)?;
        if live {
            for c in (1..=iterations).rev() {
                m[0] = m[0].wrapping_add(m[1]);
                if c == 2 {
                    m[0] = m[0].wrapping_add(1);
                }
                m[1] = m[1].wrapping_add(k as u64);
            }
        }
        for _ in 0..4 {
            a.add(r8, rax)?;
            a.lea(r9, ptr(r8 + rdx * 2 + 0x10))?;
            a.sub(r8, r9)?;
            a.mov(r10, r9)?;
            a.add(r10, r8)?;
            if live {
                m[2] = m[2].wrapping_add(m[0]);
                m[3] = m[2].wrapping_add(m[1].wrapping_mul(2)).wrapping_add(0x10);
                m[2] = m[2].wrapping_sub(m[3]);
                m[4] = m[3].wrapping_add(m[2]);
            }
        }
    }
    a.ret()?;
    Ok(low_case(
        "big_function",
        asm(&mut a, LOW_CODE)?,
        &[("r11", 1)],
        &[
            ("rax", m[0]),
            ("rcx", 0),
            ("rdx", m[1]),
            ("r8", m[2]),
            ("r9", m[3]),
            ("r10", m[4]),
        ],
    ))
}

/// The cases written by `testkit spike`.
pub fn cases() -> Result<Vec<Case>> {
    Ok(vec![
        sum_loop(100)?,
        fib_recursive(20)?,
        array(64)?,
        high_address()?,
        fib_rec(20)?,
        deep_rec(100_000)?,
        tail_rec(100_000)?,
        ret_tricks()?,
        call_table()?,
        shared_tail()?,
        longjmp_like()?,
        big_function(100)?,
    ])
}

/// `testkit spike --out DIR`: translates every case, checks it in wasmtime,
/// and writes `DIR/<name>.wasm` plus `DIR/manifest.json`.
pub fn write(out: &Path) -> Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let host = Host::new()?;
    let mut entries = Vec::new();
    for case in cases()? {
        let translation = case.translate(&Options::default())?;
        let (state, machine) = case.run(&host, &translation.wasm)?;
        case.check(&state, &machine)?;
        let file = format!("{}.wasm", case.name);
        std::fs::write(out.join(&file), &translation.wasm)?;
        entries.push(case.manifest_entry(&file));
        let chunks: usize = translation.functions.iter().map(|f| f.chunks.len()).sum();
        println!(
            "{}: {} guest instructions, {} blocks, {} functions ({} chunks), {} wasm bytes",
            case.name,
            translation.guest_instructions,
            translation.blocks.len(),
            translation.functions.len(),
            chunks,
            translation.wasm.len()
        );
    }
    let manifest = json!({
        "abi": {
            "stateSize": abi::STATE_SIZE,
            "gpr": abi::STATE_GPR,
            "rip": abi::STATE_RIP,
            "rflags": abi::STATE_RFLAGS,
            "exit": abi::STATE_EXIT,
        },
        "cases": entries,
    });
    let path = out.join("manifest.json");
    std::fs::write(&path, serde_json::to_string_pretty(&manifest)? + "\n")?;
    println!("wrote {}", path.display());
    Ok(())
}
