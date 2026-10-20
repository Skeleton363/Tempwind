//! Differential instruction tests: every snippet runs natively in the
//! fixed-address sandbox (`testkit::spike::native`) and through the
//! translator in wasmtime, from the same registers, flags and data page. All
//! GPRs, the status flags, the exit RIP and the data page must match; RFLAGS
//! bits other than the status flags must be preserved from entry.

#![cfg(all(target_arch = "x86_64", unix))]

use std::collections::HashMap;

use iced_x86::code_asm::*;
use iced_x86::{Code, Decoder, DecoderOptions, Formatter, Instruction, IntelFormatter, Register};
use testkit::host::{Host, Machine};
use testkit::spike::native::{
    SANDBOX_DATA, SANDBOX_DATA_SIZE, SANDBOX_MEMORY_BYTES, SANDBOX_SNIPPET, with_sandbox,
};
use wasmtime::Module;
use x64wasm::abi::{self, CpuState, flags};
use x64wasm::{CodeRegion, Options};

const STATE_ADDR: u64 = 0x1000;
/// Guest RSP for snippets: the middle of the data page.
const STACK: u64 = SANDBOX_DATA + 0x8000;
/// Data base register value for memory forms.
const DATA: u64 = SANDBOX_DATA + 0x100;

/// Deterministic xorshift64* generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

/// Operand values covering carry/overflow/sign/parity/AF boundaries at every
/// width, with junk in the upper bits for the narrow forms.
const EDGES: [u64; 24] = [
    0,
    1,
    2,
    0xf,
    0x10,
    0x7f,
    0x80,
    0x81,
    0xff,
    0x100,
    0x7fff,
    0x8000,
    0xffff,
    0x7fff_ffff,
    0x8000_0000,
    0xffff_ffff,
    0x1_0000_0000,
    0x7fff_ffff_ffff_ffff,
    0x8000_0000_0000_0000,
    0xffff_ffff_ffff_ffff,
    0xdead_beef_8000_7f80,
    0x1234_5678_ffff_ff7f,
    0xffff_ffff_0000_0001,
    0x0f0f_0f0f_f0f0_f0f0,
];

struct Harness {
    host: Host,
    machine: Machine,
    rng: Rng,
    runs: usize,
    /// Modules by (code, entries).
    modules: HashMap<(Vec<u8>, Vec<u64>), Module>,
}

impl Harness {
    fn new(seed: u64) -> Harness {
        let host = Host::new().unwrap();
        let machine = host.machine(SANDBOX_MEMORY_BYTES).unwrap();
        Harness {
            host,
            machine,
            rng: Rng(seed | 1),
            runs: 0,
            modules: HashMap::new(),
        }
    }

    fn module(&mut self, code: &[u8], entries: &[u64]) -> Module {
        let key = (code.to_vec(), entries.to_vec());
        if let Some(m) = self.modules.get(&key) {
            return m.clone();
        }
        let t = x64wasm::translate(
            CodeRegion {
                base: SANDBOX_SNIPPET,
                bytes: code,
            },
            entries,
            &Options::default(),
        )
        .unwrap();
        // Compiling validates the module.
        let m = self.host.compile(&t.wasm).unwrap();
        self.modules.insert(key, m.clone());
        m
    }

    /// Runs the translated snippet like a host that translates more code
    /// when execution leaves the translated blocks: on a LEFT_CODE exit to
    /// an untranslated address inside the snippet, add it as an entry,
    /// retranslate, and resume from the exit state.
    fn run_wasm(&mut self, code: &[u8], input: &CpuState) -> CpuState {
        let end = SANDBOX_SNIPPET + code.len() as u64;
        let mut entries = vec![SANDBOX_SNIPPET];
        let mut state = *input;
        for _ in 0..16 {
            let module = self.module(code, &entries);
            state = self
                .machine
                .run_state(&module, STATE_ADDR, &state)
                .unwrap();
            let rip = state.rip;
            let inside = (SANDBOX_SNIPPET..end).contains(&rip);
            if state.exit == abi::EXIT_LEFT_CODE && inside && !entries.contains(&rip) {
                entries.push(rip);
                state.exit = 0;
                continue;
            }
            break;
        }
        state
    }

    /// A random register file with RSP on the sandbox stack, `rbx` = DATA,
    /// random status flags.
    fn random_state(&mut self) -> CpuState {
        let mut s = CpuState::default();
        for r in s.gpr.iter_mut() {
            *r = self.rng.next();
        }
        s.gpr[4] = STACK;
        s.gpr[3] = DATA;
        s.rip = SANDBOX_SNIPPET;
        s.rflags = 0x202 | (self.rng.next() & flags::STATUS);
        s
    }

    fn random_data(&mut self) -> Vec<u8> {
        (0..SANDBOX_DATA_SIZE / 8)
            .flat_map(|_| self.rng.next().to_le_bytes())
            .collect()
    }

    /// Runs `code` natively and translated from `input`; panics on any difference.
    fn check(&mut self, code: &[u8], input: &CpuState, data: &[u8]) {
        self.runs += 1;
        let (native, native_data) =
            with_sandbox(|sb| sb.run(code, input, data)).expect("native run");
        self.machine.write(SANDBOX_DATA, data).unwrap();
        self.machine.write(SANDBOX_SNIPPET, code).unwrap();
        let wasm = self.run_wasm(code, input);
        let wasm_data = self.machine.read(SANDBOX_DATA, SANDBOX_DATA_SIZE).unwrap();

        let mut errors = Vec::new();
        if wasm.exit != abi::EXIT_LEFT_CODE || wasm.rip != native.rip {
            errors.push(format!(
                "exit {} rip {:#x}, native rip {:#x}",
                wasm.exit, wasm.rip, native.rip
            ));
        }
        for i in 0..16 {
            if wasm.gpr[i] != native.gpr[i] {
                errors.push(format!(
                    "{}: wasm {:#x} native {:#x}",
                    abi::GPR_NAMES[i],
                    wasm.gpr[i],
                    native.gpr[i]
                ));
            }
        }
        if wasm.rflags & flags::STATUS != native.rflags & flags::STATUS {
            errors.push(format!(
                "status flags: wasm {} native {}",
                flag_names(wasm.rflags),
                flag_names(native.rflags)
            ));
        }
        if wasm.rflags & !flags::STATUS != input.rflags & !flags::STATUS {
            errors.push(format!(
                "non-status rflags bits changed: {:#x} -> {:#x}",
                input.rflags, wasm.rflags
            ));
        }
        if wasm_data != native_data
            && let Some(at) = (0..SANDBOX_DATA_SIZE).find(|&i| wasm_data[i] != native_data[i])
        {
            let at = at & !7;
            errors.push(format!(
                "data differs at {:#x}: wasm {:02x?} native {:02x?}",
                SANDBOX_DATA + at as u64,
                &wasm_data[at..at + 8],
                &native_data[at..at + 8]
            ));
        }
        if !errors.is_empty() {
            panic!(
                "mismatch for\n{}\ninput regs {:x?} rflags {:#x}\n{}",
                disasm(code),
                input.gpr,
                input.rflags,
                errors.join("\n")
            );
        }
    }

    /// Runs `code` for every pair of edge values in registers `ra`, `rb`
    /// (other registers random), plus random rounds.
    fn pairs(&mut self, code: &[u8], ra: usize, rb: Option<usize>) {
        let data = self.random_data();
        for &a in &EDGES {
            for &b in &EDGES {
                if rb.is_none() && b != 0 {
                    continue;
                }
                let mut s = self.random_state();
                s.gpr[ra] = a;
                if let Some(rb) = rb {
                    s.gpr[rb] = b;
                }
                self.check(code, &s, &data);
            }
        }
        for _ in 0..64 {
            let s = self.random_state();
            let mut s2 = s;
            s2.gpr[ra] = s.gpr[ra] & 0xff;
            self.check(code, &s, &data);
            self.check(code, &s2, &data);
        }
    }

    /// Runs `code` from random registers with `setup` applied, several times.
    fn random(&mut self, code: &[u8], rounds: usize, setup: impl Fn(&mut CpuState, &mut Rng)) {
        for _ in 0..rounds {
            let mut s = self.random_state();
            setup(&mut s, &mut self.rng);
            let data = self.random_data();
            self.check(code, &s, &data);
        }
    }
}

fn flag_names(rflags: u64) -> String {
    let mut s = String::new();
    for (bit, name) in [
        (flags::CF, "CF"),
        (flags::PF, "PF"),
        (flags::AF, "AF"),
        (flags::ZF, "ZF"),
        (flags::SF, "SF"),
        (flags::OF, "OF"),
    ] {
        if rflags & bit != 0 {
            s.push_str(name);
            s.push(' ');
        }
    }
    s
}

fn disasm(code: &[u8]) -> String {
    let mut out = String::new();
    let mut d = Decoder::with_ip(64, code, SANDBOX_SNIPPET, DecoderOptions::NONE);
    let mut f = IntelFormatter::new();
    while d.can_decode() {
        let insn = d.decode();
        out.push_str(&format!("  {:#x}: ", insn.ip()));
        f.format(&insn, &mut out);
        out.push('\n');
    }
    out
}

fn assemble(build: impl FnOnce(&mut CodeAssembler) -> Result<(), IcedError>) -> Vec<u8> {
    let mut a = CodeAssembler::new(64).unwrap();
    build(&mut a).unwrap();
    a.assemble(SANDBOX_SNIPPET).unwrap()
}

/// One instruction with an exact encoding.
fn exact(insn: Result<Instruction, IcedError>) -> Vec<u8> {
    let insn = insn.unwrap();
    let mut e = iced_x86::Encoder::new(64);
    e.encode(&insn, SANDBOX_SNIPPET).unwrap();
    e.take_buffer()
}

fn idx(r: Register) -> usize {
    (r.full_register() as usize) - (Register::RAX as usize)
}

/// The same two-operand ALU form for add, sub and cmp.
fn alu3(build: impl Fn(Code) -> Result<Instruction, IcedError>, add: Code, sub: Code, cmp: Code) -> Vec<Vec<u8>> {
    [add, sub, cmp].into_iter().map(|c| exact(build(c))).collect()
}

#[test]
fn alu_reg_reg_all_widths() {
    let mut h = Harness::new(1);
    use Register::*;
    let forms: &[(Code, Code, Code, Register, Register)] = &[
        (Code::Add_rm64_r64, Code::Sub_rm64_r64, Code::Cmp_rm64_r64, RAX, RCX),
        (Code::Add_r64_rm64, Code::Sub_r64_rm64, Code::Cmp_r64_rm64, R9, R14),
        (Code::Add_rm32_r32, Code::Sub_rm32_r32, Code::Cmp_rm32_r32, EDX, ESI),
        (Code::Add_r32_rm32, Code::Sub_r32_rm32, Code::Cmp_r32_rm32, R10D, EAX),
        (Code::Add_rm16_r16, Code::Sub_rm16_r16, Code::Cmp_rm16_r16, CX, DI),
        (Code::Add_r16_rm16, Code::Sub_r16_rm16, Code::Cmp_r16_rm16, R12W, DX),
        (Code::Add_rm8_r8, Code::Sub_rm8_r8, Code::Cmp_rm8_r8, CL, DL),
        (Code::Add_r8_rm8, Code::Sub_r8_rm8, Code::Cmp_r8_rm8, AL, BL),
        (Code::Add_rm8_r8, Code::Sub_rm8_r8, Code::Cmp_rm8_r8, AH, CL),
        (Code::Add_rm8_r8, Code::Sub_rm8_r8, Code::Cmp_rm8_r8, DL, BH),
        (Code::Add_r8_rm8, Code::Sub_r8_rm8, Code::Cmp_r8_rm8, CH, DH),
        (Code::Add_rm8_r8, Code::Sub_rm8_r8, Code::Cmp_rm8_r8, SIL, DIL),
        (Code::Add_r8_rm8, Code::Sub_r8_rm8, Code::Cmp_r8_rm8, R8L, R15L),
        (Code::Add_rm8_r8, Code::Sub_rm8_r8, Code::Cmp_rm8_r8, BPL, R11L),
        // Same register on both sides.
        (Code::Add_rm64_r64, Code::Sub_rm64_r64, Code::Cmp_rm64_r64, RSI, RSI),
        (Code::Add_rm8_r8, Code::Sub_rm8_r8, Code::Cmp_rm8_r8, AH, AH),
    ];
    for &(add, sub, cmp, d, s) in forms {
        for code in alu3(|c| Instruction::with2(c, d, s), add, sub, cmp) {
            let rb = (idx(d) != idx(s)).then_some(idx(s));
            h.pairs(&code, idx(d), rb);
        }
    }
    eprintln!("{} runs", h.runs);
}

#[test]
fn alu_reg_imm_all_widths() {
    let mut h = Harness::new(2);
    use Register::*;
    type Form = (Code, Code, Code, Register, i64);
    let forms: &[Form] = &[
        // imm8 sign-extended
        (Code::Add_rm64_imm8, Code::Sub_rm64_imm8, Code::Cmp_rm64_imm8, RCX, -1),
        (Code::Add_rm64_imm8, Code::Sub_rm64_imm8, Code::Cmp_rm64_imm8, R13, 0x7f),
        (Code::Add_rm64_imm8, Code::Sub_rm64_imm8, Code::Cmp_rm64_imm8, RDX, -128),
        (Code::Add_rm32_imm8, Code::Sub_rm32_imm8, Code::Cmp_rm32_imm8, ESI, -1),
        (Code::Add_rm32_imm8, Code::Sub_rm32_imm8, Code::Cmp_rm32_imm8, R11D, 1),
        (Code::Add_rm16_imm8, Code::Sub_rm16_imm8, Code::Cmp_rm16_imm8, DI, -128),
        (Code::Add_rm16_imm8, Code::Sub_rm16_imm8, Code::Cmp_rm16_imm8, R9W, 0x10),
        (Code::Add_rm8_imm8, Code::Sub_rm8_imm8, Code::Cmp_rm8_imm8, CL, 0x80),
        (Code::Add_rm8_imm8, Code::Sub_rm8_imm8, Code::Cmp_rm8_imm8, BH, 0x7f),
        (Code::Add_rm8_imm8, Code::Sub_rm8_imm8, Code::Cmp_rm8_imm8, SPL, 0x01),
        (Code::Add_rm8_imm8, Code::Sub_rm8_imm8, Code::Cmp_rm8_imm8, R10L, 0xff),
        // imm32 (sign-extended at 64), imm16
        (Code::Add_rm64_imm32, Code::Sub_rm64_imm32, Code::Cmp_rm64_imm32, RCX, 0x1234_5678),
        (Code::Add_rm64_imm32, Code::Sub_rm64_imm32, Code::Cmp_rm64_imm32, R8, -0x8000_0000),
        (Code::Add_rm64_imm32, Code::Sub_rm64_imm32, Code::Cmp_rm64_imm32, RDI, 0x7fff_ffff),
        (Code::Add_rm32_imm32, Code::Sub_rm32_imm32, Code::Cmp_rm32_imm32, EDX, 0x89ab_cdef),
        (Code::Add_rm32_imm32, Code::Sub_rm32_imm32, Code::Cmp_rm32_imm32, R15D, 0x8000_0000),
        (Code::Add_rm16_imm16, Code::Sub_rm16_imm16, Code::Cmp_rm16_imm16, BP, 0x8001),
        // Accumulator short forms
        (Code::Add_AL_imm8, Code::Sub_AL_imm8, Code::Cmp_AL_imm8, AL, 0x80),
        (Code::Add_AL_imm8, Code::Sub_AL_imm8, Code::Cmp_AL_imm8, AL, 0x01),
        (Code::Add_AX_imm16, Code::Sub_AX_imm16, Code::Cmp_AX_imm16, AX, 0x7fff),
        (Code::Add_EAX_imm32, Code::Sub_EAX_imm32, Code::Cmp_EAX_imm32, EAX, 0xffff_ffff),
        (Code::Add_RAX_imm32, Code::Sub_RAX_imm32, Code::Cmp_RAX_imm32, RAX, -2),
        (Code::Add_RAX_imm32, Code::Sub_RAX_imm32, Code::Cmp_RAX_imm32, RAX, 0x4000_0000),
    ];
    for &(add, sub, cmp, d, imm) in forms {
        for code in alu3(|c| Instruction::with2(c, d, imm as i32), add, sub, cmp) {
            h.pairs(&code, idx(d), Option::None);
        }
    }
    eprintln!("{} runs", h.runs);
}

#[test]
fn alu_memory_forms() {
    let mut h = Harness::new(3);
    let mut snippets: Vec<Vec<u8>> = Vec::new();
    macro_rules! three {
        ($dst:expr, $src:expr) => {
            snippets.push(assemble(|a| a.add($dst, $src)));
            snippets.push(assemble(|a| a.sub($dst, $src)));
            snippets.push(assemble(|a| a.cmp($dst, $src)));
        };
    }
    // r/mem
    three!(rax, qword_ptr(rbx + 8));
    three!(ecx, dword_ptr(rbx + rsi * 4 + 0x40));
    three!(dx, word_ptr(rbx - 0x10));
    three!(al, byte_ptr(rbx + 3));
    three!(ah, byte_ptr(rbx + 5));
    three!(r9b, byte_ptr(rbx + 7));
    // mem/r
    three!(qword_ptr(rbx), rcx);
    three!(dword_ptr(rbx + 0x7c), r8d);
    three!(word_ptr(rbx + 1), si);
    three!(byte_ptr(rbx + 2), dh);
    three!(byte_ptr(rbx + 9), dil);
    // mem/imm
    three!(qword_ptr(rbx + 0x20), -3);
    three!(qword_ptr(rbx + 0x28), 0x7654_3210);
    three!(qword_ptr(rbx + 0x30), -0x7000_0000);
    three!(dword_ptr(rbx + 0x34), 0x80);
    three!(dword_ptr(rbx + 0x38), 0x8000_0001u32 as i32);
    three!(word_ptr(rbx + 0x3a), 0x7fff);
    three!(word_ptr(rbx + 0x3c), -1);
    three!(byte_ptr(rbx + 0x3e), 0x80);
    three!(byte_ptr(rbx + 0x3f), 1);
    for code in &snippets {
        h.random(code, 48, |s, r| {
            s.gpr[6] = r.next() & 0x3f; // rsi: small index
        });
    }
    // Edge operands through memory: the data page starts with the edge values.
    let code = assemble(|a| {
        a.add(rax, qword_ptr(rbx + rsi * 8))?;
        a.sub(ecx, dword_ptr(rbx + rdi * 8))?;
        a.cmp(byte_ptr(rbx + rdi * 8), dl)
    });
    let mut data = h.random_data();
    for (i, v) in EDGES.iter().enumerate() {
        let at = 0x100 + 8 * i;
        data[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    for (i, &a) in EDGES.iter().enumerate() {
        for (j, &b) in EDGES.iter().enumerate() {
            let mut s = h.random_state();
            s.gpr[0] = a;
            s.gpr[1] = b;
            s.gpr[2] = a;
            s.gpr[6] = j as u64;
            s.gpr[7] = i as u64;
            h.check(&code, &s, &data);
        }
    }
    eprintln!("{} runs", h.runs);
}

#[test]
fn mov_forms() {
    let mut h = Harness::new(4);
    use Register::*;
    let mut snippets: Vec<Vec<u8>> = vec![
        exact(Instruction::with2(Code::Mov_rm64_r64, RAX, R11)),
        exact(Instruction::with2(Code::Mov_r64_rm64, R12, RCX)),
        exact(Instruction::with2(Code::Mov_rm32_r32, EDX, ESI)),
        exact(Instruction::with2(Code::Mov_r32_rm32, R8D, EAX)),
        exact(Instruction::with2(Code::Mov_rm16_r16, CX, R13W)),
        exact(Instruction::with2(Code::Mov_r16_rm16, BP, SP)),
        exact(Instruction::with2(Code::Mov_rm8_r8, AL, CL)),
        exact(Instruction::with2(Code::Mov_r8_rm8, AH, DL)),
        exact(Instruction::with2(Code::Mov_rm8_r8, BL, CH)),
        exact(Instruction::with2(Code::Mov_rm8_r8, DH, BH)),
        exact(Instruction::with2(Code::Mov_rm8_r8, SIL, R9L)),
        exact(Instruction::with2(Code::Mov_r8_rm8, DIL, BPL)),
        exact(Instruction::with2(Code::Mov_rm8_r8, R15L, SPL)),
        exact(Instruction::with2(Code::Mov_rm64_imm32, RCX, -5i32)),
        exact(Instruction::with2(Code::Mov_rm64_imm32, R10, 0x7fff_ffffi32)),
        exact(Instruction::with2(Code::Mov_r64_imm64, RDX, 0x8123_4567_89ab_cdefu64)),
        exact(Instruction::with2(Code::Mov_r32_imm32, ESI, 0xfedc_ba98u32)),
        exact(Instruction::with2(Code::Mov_rm32_imm32, R9D, 0x8000_0000u32)),
        exact(Instruction::with2(Code::Mov_r16_imm16, DI, 0xbeefu32)),
        exact(Instruction::with2(Code::Mov_rm16_imm16, R14W, 0x1234u32)),
        exact(Instruction::with2(Code::Mov_r8_imm8, AH, 0xa5u32)),
        exact(Instruction::with2(Code::Mov_r8_imm8, R8L, 0x5au32)),
        exact(Instruction::with2(Code::Mov_rm8_imm8, SIL, 0x77u32)),
        exact(Instruction::with2(Code::Mov_r8_imm8, CL, 0x80u32)),
    ];
    // reg <- mem, mem <- reg, mem <- imm at every width; moffs forms.
    snippets.push(assemble(|a| {
        a.mov(rax, qword_ptr(rbx + 0x18))?;
        a.mov(ecx, dword_ptr(rbx + rdi * 2 + 0x20))?;
        a.mov(dx, word_ptr(rbx - 4))?;
        a.mov(sil, byte_ptr(rbx + 1))?;
        a.mov(bh, byte_ptr(rbx + 2))?;
        a.mov(r13b, byte_ptr(rbx + 3))
    }));
    snippets.push(assemble(|a| {
        a.mov(qword_ptr(rbx + 0x40), rcx)?;
        a.mov(dword_ptr(rbx + 0x48), edx)?;
        a.mov(word_ptr(rbx + 0x4c), r8w)?;
        a.mov(byte_ptr(rbx + 0x4e), ah)?;
        a.mov(byte_ptr(rbx + 0x4f), r12b)?;
        a.mov(qword_ptr(rsp + 8), rsp)
    }));
    snippets.push(assemble(|a| {
        a.mov(qword_ptr(rbx + 0x50), -2)?;
        a.mov(qword_ptr(rbx + 0x58), 0x7fff_0000)?;
        a.mov(dword_ptr(rbx + 0x60), 0x8765_4321u32)?;
        a.mov(word_ptr(rbx + 0x64), 0xfffe)?;
        a.mov(byte_ptr(rbx + 0x66), 0x99)?;
        a.mov(byte_ptr(rbx + rdi * 1 + 0x67), 0x11)
    }));
    let moffs = SANDBOX_DATA + 0x200;
    snippets.push(exact(Instruction::with2(
        Code::Mov_AL_moffs8,
        AL,
        iced_x86::MemoryOperand::with_displ(moffs, 8),
    )));
    snippets.push(exact(Instruction::with2(
        Code::Mov_moffs64_RAX,
        iced_x86::MemoryOperand::with_displ(moffs + 8, 8),
        RAX,
    )));
    snippets.push(exact(Instruction::with2(
        Code::Mov_EAX_moffs32,
        EAX,
        iced_x86::MemoryOperand::with_displ(moffs + 0x10, 8),
    )));
    for code in &snippets {
        h.random(code, 32, |s, r| {
            s.gpr[7] = r.next() & 0xff; // rdi: small index
        });
    }
    eprintln!("{} runs", h.runs);
}

#[test]
fn lea_forms() {
    let mut h = Harness::new(5);
    let snippets = [
        assemble(|a| a.lea(rax, ptr(rbx + rcx * 8 + 0x12345))),
        assemble(|a| a.lea(rdx, ptr(rsi + rdi))),
        assemble(|a| a.lea(r8, ptr(rcx * 4 - 8))),
        assemble(|a| a.lea(r9, ptr(r12 + r13 * 2))),
        assemble(|a| a.lea(r10, ptr(r13 - 0x80))),
        assemble(|a| a.lea(r11, ptr(rsp + 0x10))),
        assemble(|a| a.lea(rbp, ptr(rbp + rbp * 4))),
        assemble(|a| a.lea(rcx, ptr(0x2000_1000u64))),
        assemble(|a| a.lea(eax, ptr(rbx + rcx * 2 + 0x7fff_ffff))),
        assemble(|a| a.lea(r15d, ptr(rsi - 1))),
        assemble(|a| a.lea(si, ptr(rdi + rdx * 8 + 3))),
        // RIP-relative
        assemble(|a| {
            let mut l = a.create_label();
            a.lea(rdi, ptr(l))?;
            a.set_label(&mut l)?;
            a.nop()
        }),
        exact(Instruction::with2(
            Code::Lea_r64_m,
            Register::RDI,
            iced_x86::MemoryOperand::with_base_displ(Register::RIP, (SANDBOX_SNIPPET - 0x1234) as i64),
        )),
        exact(Instruction::with2(
            Code::Lea_r32_m,
            Register::R8D,
            iced_x86::MemoryOperand::with_base_displ(Register::RIP, (SANDBOX_SNIPPET + 0x10_0000) as i64),
        )),
    ];
    for code in &snippets {
        h.random(code, 64, |_, _| {});
    }
    eprintln!("{} runs", h.runs);
}

#[test]
fn push_pop_forms() {
    let mut h = Harness::new(6);
    let snippets = [
        assemble(|a| {
            a.push(rcx)?;
            a.push(r12)?;
            a.pop(rdx)?;
            a.pop(r13)
        }),
        assemble(|a| a.push(rsp)),
        assemble(|a| a.pop(rsp)),
        assemble(|a| {
            a.push(rsp)?;
            a.pop(rsp)
        }),
        assemble(|a| {
            a.push(-1)?;
            a.push(0x7f)?;
            a.push(-0x8000_0000)?;
            a.push(0x1234_5678)
        }),
        assemble(|a| {
            a.push(qword_ptr(rbx + 8))?;
            a.push(qword_ptr(rsp))?;
            a.push(qword_ptr(rsp + 16))?;
            a.pop(qword_ptr(rbx + 0x40))
        }),
        assemble(|a| a.pop(qword_ptr(rsp))),
        assemble(|a| a.pop(qword_ptr(rsp + 8))),
        assemble(|a| a.pop(qword_ptr(rsp - 8))),
        assemble(|a| a.pop(qword_ptr(rsp + rcx * 8))),
        exact(Instruction::with1(Code::Push_rm64, Register::R9)),
        exact(Instruction::with1(Code::Pop_rm64, Register::R9)),
        exact(Instruction::with1(Code::Pushq_imm8, -2i32)),
        exact(Instruction::with1(Code::Pushq_imm32, 0x8000_0000u32 as i32)),
    ];
    for code in &snippets {
        h.random(code, 32, |s, r| {
            s.gpr[1] = r.next() & 7;
            // `pop rsp` needs a stack slot that points back into the stack.
            s.gpr[4] = STACK;
        });
    }
    // pop rsp with a valid new stack value in [rsp].
    let code = assemble(|a| {
        a.pop(rsp)?;
        a.pop(rax)
    });
    for i in 0..16u64 {
        let s = h.random_state();
        let mut data = h.random_data();
        let at = (STACK - SANDBOX_DATA) as usize;
        let target = STACK + 0x100 + 8 * i;
        data[at..at + 8].copy_from_slice(&target.to_le_bytes());
        h.check(&code, &s, &data);
    }
    eprintln!("{} runs", h.runs);
}

#[test]
fn jcc_all_conditions() {
    let mut h = Harness::new(7);
    type Jcc = fn(&mut CodeAssembler, CodeLabel) -> Result<(), IcedError>;
    let jccs: [Jcc; 16] = [
        |a, l| a.jo(l),
        |a, l| a.jno(l),
        |a, l| a.jb(l),
        |a, l| a.jae(l),
        |a, l| a.je(l),
        |a, l| a.jne(l),
        |a, l| a.jbe(l),
        |a, l| a.ja(l),
        |a, l| a.js(l),
        |a, l| a.jns(l),
        |a, l| a.jp(l),
        |a, l| a.jnp(l),
        |a, l| a.jl(l),
        |a, l| a.jge(l),
        |a, l| a.jle(l),
        |a, l| a.jg(l),
    ];
    // Flag producers. They don't change rcx/rdx, so every condition in a
    // snippet sees the flags of the same operands.
    type Setup = fn(&mut CodeAssembler) -> Result<(), IcedError>;
    let setups: [Setup; 9] = [
        |_| Ok(()), // flags straight from the entry RFLAGS
        |a| a.cmp(rcx, rdx),
        |a| a.cmp(ecx, edx),
        |a| a.cmp(cx, dx),
        |a| a.cmp(cl, dl),
        |a| {
            a.mov(r8, rcx)?;
            a.sub(r8d, edx)
        },
        |a| {
            a.mov(r8, rcx)?;
            a.add(r8, rdx)
        },
        |a| {
            a.mov(r8, rcx)?;
            a.add(r8b, dl)
        },
        |a| {
            a.mov(r8, rcx)?;
            a.add(r8w, 0x7ff0)
        },
    ];
    for (setup_index, setup) in setups.iter().enumerate() {
        for short in [true, false] {
            // For each condition i:
            //   setup; [jmp next; next:] jcc taken; mov byte [rbx+i], 1; jmp done;
            //   taken: mov byte [rbx+i], 2; done:
            // With the extra jmp the jcc starts its own block, so its
            // condition comes from the lazy flag record at run time instead
            // of the statically known producer.
            for split in [false, true] {
                let code = assemble(|a| {
                    a.set_prefer_short_branch(short);
                    for (i, jcc) in jccs.iter().enumerate() {
                        let mut taken = a.create_label();
                        let mut done = a.create_label();
                        let mut next = a.create_label();
                        setup(a)?;
                        if split {
                            a.jmp(next)?;
                            a.set_label(&mut next)?;
                        }
                        jcc(a, taken)?;
                        a.mov(byte_ptr(rbx + i as i32), 1)?;
                        a.jmp(done)?;
                        a.set_label(&mut taken)?;
                        a.mov(byte_ptr(rbx + i as i32), 2)?;
                        a.set_label(&mut done)?;
                    }
                    a.nop()
                });
                let data = vec![0u8; 0x200];
                if setup_index == 0 {
                    // Every combination of the six status flags.
                    for bits in 0..64u64 {
                        let mut s = h.random_state();
                        s.rflags = 0x202;
                        for (k, f) in [flags::CF, flags::PF, flags::AF, flags::ZF, flags::SF, flags::OF]
                            .into_iter()
                            .enumerate()
                        {
                            if bits >> k & 1 != 0 {
                                s.rflags |= f;
                            }
                        }
                        h.check(&code, &s, &data);
                    }
                } else {
                    for &a in &EDGES {
                        for &b in EDGES.iter().step_by(2) {
                            let mut s = h.random_state();
                            s.gpr[1] = a;
                            s.gpr[2] = b;
                            h.check(&code, &s, &data);
                        }
                    }
                }
            }
        }
    }
    eprintln!("{} runs", h.runs);
}

#[test]
fn call_ret_jmp_forms() {
    let mut h = Harness::new(8);
    let snippets = [
        // call rel32 / ret
        assemble(|a| {
            let mut f = a.create_label();
            let mut end = a.create_label();
            a.call(f)?;
            a.mov(ebx, 3)?;
            a.jmp(end)?;
            a.set_label(&mut f)?;
            a.mov(ecx, 7)?;
            a.mov(rdx, qword_ptr(rsp))?;
            a.ret()?;
            a.set_label(&mut end)?;
            a.nop()
        }),
        // ret imm16
        assemble(|a| {
            let mut f = a.create_label();
            let mut end = a.create_label();
            a.push(5)?;
            a.push(6)?;
            a.call(f)?;
            a.mov(r8, rsp)?;
            a.jmp(end)?;
            a.set_label(&mut f)?;
            a.ret_1(16)?;
            a.set_label(&mut end)?;
            a.nop()
        }),
        // call r64, call m64, jmp r64, jmp m64
        assemble(|a| {
            let mut f = a.create_label();
            let mut g = a.create_label();
            let mut end = a.create_label();
            a.lea(rax, ptr(f))?;
            a.call(rax)?;
            a.lea(rcx, ptr(g))?;
            a.mov(qword_ptr(rbx), rcx)?;
            a.call(qword_ptr(rbx))?;
            a.lea(rdx, ptr(end))?;
            a.mov(qword_ptr(rbx + 8), rdx)?;
            a.jmp(qword_ptr(rbx + 8))?;
            a.set_label(&mut f)?;
            a.add(r9, 1)?;
            a.ret()?;
            a.set_label(&mut g)?;
            a.add(r10, 1)?;
            a.mov(r11, rsp)?;
            a.ret()?;
            a.set_label(&mut end)?;
            // Jump to the end of the snippet (no block there: LEFT_CODE).
            a.lea(rsi, ptr(end))?;
            a.add(rsi, 7 + 4 + 2)?;
            a.jmp(rsi)
        }),
        // call [rsp]: the target is read before the push
        assemble(|a| {
            let mut f = a.create_label();
            let mut end = a.create_label();
            a.lea(rax, ptr(f))?;
            a.push(rax)?;
            a.call(qword_ptr(rsp))?;
            a.jmp(end)?;
            a.set_label(&mut f)?;
            a.mov(rdi, qword_ptr(rsp + 8))?;
            a.ret()?;
            a.set_label(&mut end)?;
            a.nop()
        }),
        // nested calls and a backward loop with a multi-block body
        assemble(|a| {
            let mut f = a.create_label();
            let mut g = a.create_label();
            let mut top = a.create_label();
            let mut skip = a.create_label();
            let mut end = a.create_label();
            a.mov(ecx, 10)?;
            a.set_label(&mut top)?;
            a.call(f)?;
            a.cmp(rax, 100)?;
            a.jb(skip)?;
            a.add(rdx, 1)?;
            a.set_label(&mut skip)?;
            a.sub(ecx, 1)?;
            a.jne(top)?;
            a.jmp(end)?;
            a.set_label(&mut f)?;
            a.call(g)?;
            a.add(rax, rcx)?;
            a.ret()?;
            a.set_label(&mut g)?;
            a.add(rax, 7)?;
            a.ret()?;
            a.set_label(&mut end)?;
            a.nop()
        }),
    ];
    for code in &snippets {
        h.random(code, 24, |s, r| s.gpr[0] = r.next() & 0xff);
    }
    eprintln!("{} runs", h.runs);
}

#[test]
fn nop_and_long_nops() {
    let mut h = Harness::new(9);
    let code = [
        0x90, // nop
        0x66, 0x90, // xchg ax, ax
        0x0f, 0x1f, 0x00, // nop dword [rax]
        0x0f, 0x1f, 0x44, 0x00, 0x00, // nop dword [rax+rax+0]
        0x66, 0x0f, 0x1f, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00, // nop word [rax+rax+0]
    ];
    h.random(&code, 8, |_, _| {});
}
