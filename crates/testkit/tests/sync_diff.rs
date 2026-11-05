//! Differential test of the register and flag sync (`translator.md`,
//! "Registers and flags"): seeded random programs of calls, loops, branches
//! and register and flag use, translated with `Options::sync_all` (the
//! reference) and with the analysis under several option sets, run in
//! wasmtime. The final state blocks and memory must match.

use iced_x86::code_asm::*;
use testkit::host::Host;
use testkit::spike::Case;
use x64wasm::Options;
use x64wasm::abi::{self, CpuState};

const CODE: u64 = 0x10_0000;
const DATA: u64 = 0x20_0000;
const DATA_BYTES: usize = 1024;
const STACK: u64 = 0x2f_f000;
const STACK_CHECKED: u64 = 0x2000;
const STATE: u64 = 0x30_0000;
const MEMORY: u64 = 0x40_0000;

const R64: [AsmRegister64; 16] = [
    rax, rcx, rdx, rbx, rsp, rbp, rsi, rdi, r8, r9, r10, r11, r12, r13, r14, r15,
];
const R8: [AsmRegister8; 16] = [
    al, cl, dl, bl, spl, bpl, sil, dil, r8b, r9b, r10b, r11b, r12b, r13b, r14b, r15b,
];
/// Registers the generated code writes freely: all but RSP, R14 (the data
/// pointer) and R15 (loop counters, saved around each loop).
const POOL: [usize; 13] = [0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13];
const DATA_REG: AsmRegister64 = r14;
const LOOP_REG: AsmRegister64 = r15;

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    fn reg(&mut self) -> usize {
        POOL[self.below(POOL.len() as u64) as usize]
    }
}

struct Gen<'a> {
    a: &'a mut CodeAssembler,
    rng: Rng,
    funcs: Vec<CodeLabel>,
    /// The function being generated.
    current: usize,
}

type R = Result<(), IcedError>;

impl Gen<'_> {
    fn jcc(&mut self, label: CodeLabel) -> R {
        match self.rng.below(8) {
            0 => self.a.je(label),
            1 => self.a.jne(label),
            2 => self.a.jb(label),
            3 => self.a.jae(label),
            4 => self.a.jl(label),
            5 => self.a.jg(label),
            6 => self.a.js(label),
            _ => self.a.jp(label),
        }
    }

    fn setcc(&mut self, r: usize) -> R {
        let d = R8[r];
        match self.rng.below(6) {
            0 => self.a.sete(d),
            1 => self.a.setb(d),
            2 => self.a.setl(d),
            3 => self.a.seta(d),
            4 => self.a.sets(d),
            _ => self.a.seto(d),
        }
    }

    fn cmov(&mut self, d: usize, s: usize) -> R {
        let (d, s) = (R64[d], R64[s]);
        match self.rng.below(4) {
            0 => self.a.cmove(d, s),
            1 => self.a.cmovb(d, s),
            2 => self.a.cmovg(d, s),
            _ => self.a.cmovns(d, s),
        }
    }

    /// A flag-reading op that keeps the result in `d`.
    fn read_flags(&mut self) -> R {
        let (d, s) = (self.rng.reg(), self.rng.reg());
        match self.rng.below(4) {
            0 => self.setcc(d),
            1 => self.cmov(d, s),
            2 => self.a.adc(R64[d], R64[s]),
            _ => self.a.sbb(R64[d], R64[s]),
        }
    }

    fn data_slot(&mut self) -> AsmMemoryOperand {
        qword_ptr(DATA_REG + (self.rng.below(64) * 8) as i32)
    }

    fn simple(&mut self) -> R {
        let (d, s) = (self.rng.reg(), self.rng.reg());
        let (rd, rs) = (R64[d], R64[s]);
        match self.rng.below(22) {
            0 => self.a.mov(rd, self.rng.next()),
            1 => self.a.mov(R64[d].into_r32(), self.rng.below(1000) as u32),
            2 => self.a.add(rd, rs),
            3 => self.a.sub(rd, rs),
            4 => self.a.xor(rd, rs),
            5 => self.a.and(rd, rs),
            6 => self.a.or(rd, rs),
            7 => self.a.imul_2(rd, rs),
            8 => self.a.inc(rd),
            9 => self.a.dec(rd),
            10 => self.a.shl(rd, 1 + self.rng.below(63) as u32),
            11 => self.a.sar(rd, 1 + self.rng.below(63) as u32),
            12 => self.a.rol(rd, 1 + self.rng.below(63) as u32),
            13 => self
                .a
                .lea(rd, ptr(rs + rs * 4 + self.rng.below(100) as i32)),
            14 => {
                let m = self.data_slot();
                self.a.mov(rd, m)
            }
            15 => {
                let m = self.data_slot();
                self.a.mov(m, rs)
            }
            16 => self.a.cmp(rd, rs),
            17 => self.a.test(rd, rs),
            18 | 19 => self.read_flags(),
            20 => {
                // push one register, pop another.
                self.a.push(rs)?;
                self.a.pop(rd)
            }
            _ => self.rep_string(),
        }
    }

    /// `rep movsb` or `rep stosb` inside the upper half of the data area.
    fn rep_string(&mut self) -> R {
        let half = (DATA_BYTES / 2) as u64;
        let len = self.rng.below(64);
        let dst = half + self.rng.below(half - 64);
        let src = half + self.rng.below(half - 64);
        self.a.lea(rdi, ptr(DATA_REG + dst as i32))?;
        self.a.mov(ecx, len as u32)?;
        if self.rng.chance(50) {
            self.a.lea(rsi, ptr(DATA_REG + src as i32))?;
            self.a.rep().movsb()
        } else {
            self.a.rep().stosb()
        }
    }

    /// A call to a later function: direct, or through a register.
    fn call(&mut self) -> R {
        let n = self.funcs.len();
        if self.current + 1 >= n {
            return self.simple();
        }
        let target =
            self.funcs[self.current + 1 + self.rng.below((n - self.current - 1) as u64) as usize];
        let flags_across = self.rng.chance(40);
        if flags_across {
            let (x, y) = (self.rng.reg(), self.rng.reg());
            self.a.cmp(R64[x], R64[y])?;
        }
        if self.rng.chance(25) {
            self.a.lea(rax, ptr(target))?;
            self.a.call(rax)?;
        } else {
            self.a.call(target)?;
        }
        if flags_across {
            self.read_flags()?;
        }
        Ok(())
    }

    fn stmts(&mut self, depth: u32, count: u64) -> R {
        for _ in 0..count {
            let k = self.rng.below(100);
            if depth < 2 && k < 10 {
                // if/else
                let mut other = self.a.create_label();
                let mut end = self.a.create_label();
                let (x, y) = (self.rng.reg(), self.rng.reg());
                self.a.cmp(R64[x], R64[y])?;
                self.jcc(other)?;
                let count = 1 + self.rng.below(4);
                self.stmts(depth + 1, count)?;
                self.a.jmp(end)?;
                self.a.set_label(&mut other)?;
                let count = 1 + self.rng.below(3);
                self.stmts(depth + 1, count)?;
                self.a.set_label(&mut end)?;
                self.a.nop()?;
            } else if depth < 2 && k < 18 {
                // A counted loop on R15, saved around it.
                let mut top = self.a.create_label();
                self.a.push(LOOP_REG)?;
                self.a
                    .mov(LOOP_REG.into_r32(), 1 + self.rng.below(3) as u32)?;
                self.a.set_label(&mut top)?;
                let count = 1 + self.rng.below(5);
                self.stmts(depth + 1, count)?;
                self.a.dec(LOOP_REG)?;
                self.a.jnz(top)?;
                self.a.pop(LOOP_REG)?;
            } else if depth < 3 && k < 24 {
                // A register saved and restored around a body.
                let r = R64[self.rng.reg()];
                self.a.push(r)?;
                let count = 1 + self.rng.below(4);
                self.stmts(depth + 1, count)?;
                self.a.pop(r)?;
            } else if depth < 2 && k < 28 {
                self.local_indirect(depth)?;
            } else if k == 28 && self.rng.chance(15) {
                // A fault (division by zero) after writes in the same block:
                // the run ends here, and the leave stores the dirty set.
                let (x, z) = (self.rng.reg(), self.rng.reg());
                self.a.mov(R64[x], self.rng.next())?;
                self.a.mov(R64[z].into_r32(), 0u32)?;
                self.a.div(R64[z])?;
            } else if k < 40 {
                self.call()?;
            } else {
                self.simple()?;
            }
        }
        Ok(())
    }

    /// A jump through a register to a block of the same function (which a
    /// direct branch also targets, so it is a block): a local dispatch.
    fn local_indirect(&mut self, depth: u32) -> R {
        let mut target = self.a.create_label();
        let (x, y) = (self.rng.reg(), self.rng.reg());
        self.a.cmp(R64[x], R64[y])?;
        self.jcc(target)?;
        let count = 1 + self.rng.below(3);
        self.stmts(depth + 1, count)?;
        let r = R64[self.rng.reg()];
        self.a.lea(r, ptr(target))?;
        self.a.jmp(r)?;
        self.a.set_label(&mut target)?;
        self.a.nop()
    }

    fn function(&mut self, i: usize) -> R {
        self.current = i;
        let mut label = self.funcs[i];
        self.a.set_label(&mut label)?;
        self.funcs[i] = label;
        let count = 3 + self.rng.below(12);
        self.stmts(0, count)?;
        let n = self.funcs.len();
        if i + 1 < n && self.rng.chance(20) {
            // A tail transfer.
            let t = self.funcs[i + 1 + self.rng.below((n - i - 1) as u64) as usize];
            self.a.jmp(t)
        } else {
            self.a.ret()
        }
    }
}

trait Low32 {
    fn into_r32(self) -> AsmRegister32;
}

impl Low32 for AsmRegister64 {
    fn into_r32(self) -> AsmRegister32 {
        const R32: [AsmRegister32; 16] = [
            eax, ecx, edx, ebx, esp, ebp, esi, edi, r8d, r9d, r10d, r11d, r12d, r13d, r14d, r15d,
        ];
        R32[R64.iter().position(|&r| r == self).unwrap()]
    }
}

fn program(seed: u64) -> Case {
    let mut a = CodeAssembler::new(64).unwrap();
    let mut rng = Rng(seed);
    let n = 2 + rng.below(5) as usize;
    let funcs = (0..n).map(|_| a.create_label()).collect();
    let mut init_regs = vec![(4, STACK), (14, DATA), (15, 0)];
    for &r in &POOL {
        init_regs.push((r, rng.next() >> rng.below(64)));
    }
    let data: Vec<u8> = (0..DATA_BYTES).map(|_| rng.next() as u8).collect();
    let mut g = Gen {
        a: &mut a,
        rng,
        funcs,
        current: 0,
    };
    for i in 0..n {
        g.function(i).unwrap();
    }
    let funcs = g.funcs.clone();
    let result = a
        .assemble_options(
            CODE,
            iced_x86::BlockEncoderOptions::RETURN_NEW_INSTRUCTION_OFFSETS,
        )
        .unwrap();
    // Every function is an entry: the indirect calls take their addresses.
    let entries: Vec<u64> = funcs.iter().map(|l| result.label_ip(l).unwrap()).collect();
    Case {
        name: "sync_diff",
        code_base: CODE,
        code: result.inner.code_buffer,
        entry: entries[0],
        extra_entries: entries[1..].to_vec(),
        memory_bytes: MEMORY,
        state_addr: STATE,
        init_regs,
        init_rflags: 0x202,
        init_mem: vec![
            (STACK, abi::SENTINEL_RET.to_le_bytes().to_vec()),
            (DATA, data),
        ],
        expect_exit: abi::EXIT_LEFT_CODE,
        expect_rip: abi::SENTINEL_RET,
        expect_regs: vec![],
        expect_mem: vec![],
        expect_rflags: None,
    }
}

/// The final state, the data area and the checked stack.
fn run(host: &Host, case: &Case, opts: &Options) -> (CpuState, Vec<u8>, Vec<u8>) {
    let t = case.translate(opts).unwrap();
    let (out, machine) = case.run(host, &t.wasm).unwrap();
    let data = machine.read(DATA, DATA_BYTES).unwrap();
    let stack = machine
        .read(STACK - STACK_CHECKED, STACK_CHECKED as usize + 8)
        .unwrap();
    (out, data, stack)
}

#[test]
fn random_programs_match_sync_all() {
    let host = Host::fast_compile().unwrap();
    let o = |max_function_insns, max_chunk_insns, call_depth_limit| Options {
        max_function_insns,
        max_chunk_insns,
        call_depth_limit,
        ..Options::default()
    };
    let variants = [
        Options::default(),
        o(1, 1, Some(0)),
        o(8, 1000, Some(2)),
        o(20, 40, None),
    ];
    let reference = Options {
        sync_all: true,
        ..Options::default()
    };
    // Runs that end at the sentinel and at a fault.
    let mut ends = [0; 2];
    for seed in 0..150u64 {
        let case = program(seed);
        let want = run(&host, &case, &reference);
        assert!(
            [abi::EXIT_LEFT_CODE, abi::EXIT_FAULT].contains(&want.0.exit),
            "seed {seed}: the reference run ends at the sentinel or a fault, not {}",
            want.0.exit
        );
        ends[(want.0.exit == abi::EXIT_FAULT) as usize] += 1;
        for opts in &variants {
            let got = run(&host, &case, opts);
            assert_eq!(got.0, want.0, "seed {seed}: state with {opts:?}");
            assert!(got.1 == want.1, "seed {seed}: data area with {opts:?}");
            assert!(got.2 == want.2, "seed {seed}: stack with {opts:?}");
        }
    }
    assert!(
        ends[0] >= 50 && ends[1] >= 5,
        "sentinel and fault ends: {ends:?}"
    );
}
