//! Spike programs: small x86-64 functions (assembled with iced-x86's
//! `CodeAssembler`) translated into wasm, plus the manifest the JS hosts run
//! (see `docs/design/abi.md`).
//!
//! Each case is a function using the Win64 argument registers (RCX, RDX)
//! that returns with `ret` to [`abi::SENTINEL_RET`], which the host pushed.

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
    pub fn translate(&self, opts: &Options) -> Result<Translation> {
        x64wasm::translate(
            CodeRegion {
                base: self.code_base,
                bytes: &self.code,
            },
            &[self.entry],
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
            let got = machine.read(*addr, bytes.len())?;
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
    let result = a.assemble_options(HIGH_CODE, BlockEncoderOptions::RETURN_NEW_INSTRUCTION_OFFSETS)?;
    let ret_addr = result.label_ip(&ret_site)?;
    let code = result.inner.code_buffer;

    let r11_value = HIGH_DATA + 5 * 8 + 0x100;
    let data = u64s(&[HIGH_STACK - 16, ret_addr, 0x89ab_ce00]);
    Ok(Case {
        name: "high_address",
        code_base: HIGH_CODE,
        code,
        entry: HIGH_CODE,
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

/// The cases written by `testkit spike`.
pub fn cases() -> Result<Vec<Case>> {
    Ok(vec![
        sum_loop(100)?,
        fib_recursive(20)?,
        array(64)?,
        high_address()?,
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
        println!(
            "{}: {} guest instructions, {} blocks, {} wasm bytes",
            case.name,
            translation.guest_instructions,
            translation.blocks.len(),
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
