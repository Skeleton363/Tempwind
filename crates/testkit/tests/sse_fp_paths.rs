//! SSE floating point beyond the L1 fixtures (`translator.md`, "SSE: fast
//! path and helper"):
//! - the inline fast path gives what the helper gives: each case runs once
//!   with MXCSR = 0x1FA0 (PE set, so the arithmetic test can pass) and once
//!   with 0x1F80 (PE clear, so an inexact operation takes the helper, which
//!   the fixtures check against the CPU); results must match and MXCSR may
//!   differ only in PE. Random ordinary operands make most cases fast;
//! - an unmasked exception exits with `EXIT_UNSUPPORTED` before any change:
//!   RIP at the instruction, registers, MXCSR and flags as before.

use testkit::host::{Host, Machine, Module};
use x64wasm::abi::{self, CpuState};
use x64wasm::{CodeRegion, Options};

const CODE: u64 = 0x1000;
const STATE: u64 = 0x2000;
const MEMORY: u64 = 0x1_0000;
const XMM0: u64 = STATE + abi::STATE_XMM;
const XMM1: u64 = XMM0 + 16;
const MXCSR: u64 = STATE + abi::STATE_MXCSR;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A normal f64 with a modest exponent (bits).
    fn f64(&mut self) -> u64 {
        let r = self.next();
        let exp = 1023 - 40 + (r >> 52) % 81;
        (r & (1 << 63)) | exp << 52 | (self.next() & ((1 << 52) - 1))
    }

    /// A normal f32 with a modest exponent (bits).
    fn f32(&mut self) -> u32 {
        let r = self.next();
        let exp = 127 - 20 + (r >> 40) % 41;
        ((r as u32) & (1 << 31)) | (exp as u32) << 23 | (self.next() as u32 & ((1 << 23) - 1))
    }

    fn vec_f64(&mut self) -> u128 {
        self.f64() as u128 | (self.f64() as u128) << 64
    }

    fn vec_f32(&mut self) -> u128 {
        (0..4).fold(0, |v, i| v | (self.f32() as u128) << (32 * i))
    }

    /// Floats that convert to i32 in range: a mix of integral values and
    /// fractions, every lane.
    fn vec_int_like(&mut self, f64_lanes: bool) -> u128 {
        if f64_lanes {
            let mut lane = || {
                let v = (self.next() as i32 >> 2) as f64;
                let v = if self.next() & 1 == 0 { v } else { v / 8.0 };
                v.to_bits() as u128
            };
            lane() | lane() << 64
        } else {
            (0..4).fold(0, |acc, i| {
                let v = (self.next() as i32 >> 8) as f32;
                let v = if self.next() & 1 == 0 { v } else { v / 4.0 };
                acc | (v.to_bits() as u128) << (32 * i)
            })
        }
    }
}

struct Runner {
    host: Host,
}

#[derive(Debug, PartialEq, Eq)]
struct Out {
    exit: u32,
    rip: u64,
    gpr: [u64; 16],
    rflags: u64,
    xmm0: u128,
    xmm1: u128,
    mxcsr: u32,
}

impl Runner {
    fn module(&self, code: &[u8]) -> Module {
        let t = x64wasm::translate(
            CodeRegion {
                base: CODE,
                bytes: code,
            },
            &[CODE],
            &Options::default(),
        )
        .unwrap();
        assert!(t.unsupported.is_empty(), "{code:02x?} is unsupported");
        self.host.compile(&t.wasm).unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        m: &mut Machine,
        module: &Module,
        xmm0: u128,
        xmm1: u128,
        rax: u64,
        rflags: u64,
        mxcsr: u32,
    ) -> Out {
        let mut state = CpuState {
            rip: CODE,
            rflags,
            ..CpuState::default()
        };
        state.gpr[0] = rax;
        state.gpr[3] = 0x5555_5555_5555_5555;
        m.write(STATE, &state.to_bytes()).unwrap();
        m.write(XMM0, &xmm0.to_le_bytes()).unwrap();
        m.write(XMM1, &xmm1.to_le_bytes()).unwrap();
        m.write(MXCSR, &mxcsr.to_le_bytes()).unwrap();
        let exit = m.run(module, STATE).unwrap();
        let out = m.read_state(STATE).unwrap();
        let read128 =
            |m: &Machine, at| u128::from_le_bytes(m.read(at, 16).unwrap().try_into().unwrap());
        Out {
            exit,
            rip: out.rip,
            gpr: out.gpr,
            rflags: out.rflags,
            xmm0: read128(m, XMM0),
            xmm1: read128(m, XMM1),
            mxcsr: u32::from_le_bytes(m.read(MXCSR, 4).unwrap().try_into().unwrap()),
        }
    }
}

/// Operands of a form: (xmm0, xmm1, rax).
type Gen = fn(&mut Rng) -> (u128, u128, u64);

fn f64_pair(r: &mut Rng) -> (u128, u128, u64) {
    (r.vec_f64(), r.vec_f64(), 0)
}

fn f32_pair(r: &mut Rng) -> (u128, u128, u64) {
    (r.vec_f32(), r.vec_f32(), 0)
}

/// Positive operands (sqrt's fast path).
fn f64_positive(r: &mut Rng) -> (u128, u128, u64) {
    let sign = 1u128 << 63 | 1 << 127;
    (r.vec_f64(), r.vec_f64() & !sign, 0)
}

fn f32_positive(r: &mut Rng) -> (u128, u128, u64) {
    let sign = (0..4).fold(0u128, |v, i| v | 1 << (32 * i + 31));
    (r.vec_f32(), r.vec_f32() & !sign, 0)
}

fn f64_ints(r: &mut Rng) -> (u128, u128, u64) {
    (r.vec_f64(), r.vec_int_like(true), 0)
}

fn f32_ints(r: &mut Rng) -> (u128, u128, u64) {
    (r.vec_f32(), r.vec_int_like(false), 0)
}

fn dwords(r: &mut Rng) -> (u128, u128, u64) {
    let mut lane = || {
        let v = r.next() as i32;
        (if r.next() & 1 == 0 { v >> 6 } else { v }) as u32 as u128
    };
    let b = (0..4).fold(0, |acc, i| acc | lane() << (32 * i));
    (0, b, 0)
}

fn int64(r: &mut Rng) -> (u128, u128, u64) {
    let v = r.next() as i64;
    let v = match r.next() % 3 {
        0 => v >> 40,
        1 => v >> 8,
        _ => v,
    };
    (r.vec_f64(), 0, v as u64)
}

#[test]
fn fast_path_matches_the_helper() {
    let runner = Runner {
        host: Host::new().unwrap(),
    };
    // (name, code, operands)
    let forms: &[(&str, &[u8], Gen)] = &[
        ("addss", &[0xf3, 0x0f, 0x58, 0xc1], f32_pair),
        ("addsd", &[0xf2, 0x0f, 0x58, 0xc1], f64_pair),
        ("addps", &[0x0f, 0x58, 0xc1], f32_pair),
        ("addpd", &[0x66, 0x0f, 0x58, 0xc1], f64_pair),
        ("subsd", &[0xf2, 0x0f, 0x5c, 0xc1], f64_pair),
        ("subps", &[0x0f, 0x5c, 0xc1], f32_pair),
        ("mulss", &[0xf3, 0x0f, 0x59, 0xc1], f32_pair),
        ("mulpd", &[0x66, 0x0f, 0x59, 0xc1], f64_pair),
        ("divsd", &[0xf2, 0x0f, 0x5e, 0xc1], f64_pair),
        ("divps", &[0x0f, 0x5e, 0xc1], f32_pair),
        ("sqrtsd", &[0xf2, 0x0f, 0x51, 0xc1], f64_positive),
        ("sqrtps", &[0x0f, 0x51, 0xc1], f32_positive),
        ("sqrtss", &[0xf3, 0x0f, 0x51, 0xc1], f32_positive),
        ("sqrtpd", &[0x66, 0x0f, 0x51, 0xc1], f64_positive),
        ("cvtsd2ss", &[0xf2, 0x0f, 0x5a, 0xc1], f64_pair),
        ("cvtpd2ps", &[0x66, 0x0f, 0x5a, 0xc1], f64_pair),
        ("cvtdq2ps", &[0x0f, 0x5b, 0xc1], dwords),
        ("cvtps2dq", &[0x66, 0x0f, 0x5b, 0xc1], f32_ints),
        ("cvttps2dq", &[0xf3, 0x0f, 0x5b, 0xc1], f32_ints),
        ("cvtpd2dq", &[0xf2, 0x0f, 0xe6, 0xc1], f64_ints),
        ("cvttpd2dq", &[0x66, 0x0f, 0xe6, 0xc1], f64_ints),
        ("cvtsi2ss xmm0, rax", &[0xf3, 0x48, 0x0f, 0x2a, 0xc0], int64),
        ("cvtsi2sd xmm0, rax", &[0xf2, 0x48, 0x0f, 0x2a, 0xc0], int64),
        ("cvtsi2ss xmm0, eax", &[0xf3, 0x0f, 0x2a, 0xc0], int64),
        (
            "cvtsd2si rax, xmm1",
            &[0xf2, 0x48, 0x0f, 0x2d, 0xc1],
            f64_ints,
        ),
        ("cvttsd2si eax, xmm1", &[0xf2, 0x0f, 0x2c, 0xc1], f64_ints),
        (
            "cvtss2si rax, xmm1",
            &[0xf3, 0x48, 0x0f, 0x2d, 0xc1],
            f32_ints,
        ),
        ("cvttss2si eax, xmm1", &[0xf3, 0x0f, 0x2c, 0xc1], f32_ints),
    ];
    const CASES: u64 = 300;
    let mut rng = Rng(0x5445_4d50_5749_4e44);
    for &(name, code, generate) in forms {
        let module = runner.module(code);
        let mut fast = runner.host.machine(MEMORY).unwrap();
        let mut slow = runner.host.machine(MEMORY).unwrap();
        for _ in 0..CASES {
            let (a, b, rax) = generate(&mut rng);
            let f = runner.run(&mut fast, &module, a, b, rax, 0x202, 0x1fa0);
            let s = runner.run(&mut slow, &module, a, b, rax, 0x202, 0x1f80);
            let ctx = format!("{name}: xmm0 {a:#034x} xmm1 {b:#034x} rax {rax:#x}");
            assert_eq!(f.exit, abi::EXIT_LEFT_CODE, "{ctx}");
            assert_eq!(
                (f.xmm0, f.gpr, f.rflags),
                (s.xmm0, s.gpr, s.rflags),
                "{ctx}"
            );
            assert_eq!(f.mxcsr, s.mxcsr | 0x20, "{ctx}: MXCSR");
        }
        // Most cases are fast at 0x1FA0, and the inexact ones at 0x1F80 are
        // slow, so both paths were compared.
        assert!(
            fast.fp_calls() * 10 < CASES,
            "{name}: {} helper calls in {CASES} cases with PE set",
            fast.fp_calls()
        );
        assert!(
            slow.fp_calls() * 4 > CASES,
            "{name}: only {} helper calls in {CASES} cases with PE clear",
            slow.fp_calls()
        );
    }
}

#[test]
fn unmasked_exceptions_exit_before_any_change() {
    let runner = Runner {
        host: Host::new().unwrap(),
    };
    let one = 1.0f64.to_bits() as u128;
    let zero = 0u128;
    let qnan = 0x7ff8_0000_0000_0000u128;
    let pair = |lo: u128, hi: u128| lo | hi << 64;
    // (name, code, xmm0, xmm1, MXCSR with the exception unmasked)
    let cases: &[(&str, &[u8], u128, u128, u32)] = &[
        // ZE unmasked: 1 / 0.
        ("divsd", &[0xf2, 0x0f, 0x5e, 0xc1], one, zero, 0x1d80),
        // Lane 0 is fine, lane 1 divides by 0: nothing is written.
        (
            "divpd",
            &[0x66, 0x0f, 0x5e, 0xc1],
            pair(one, one),
            pair(one, zero),
            0x1d80,
        ),
        // PE unmasked: 1 / 3 is inexact, even with PE already set.
        (
            "divsd",
            &[0xf2, 0x0f, 0x5e, 0xc1],
            one,
            pair(3.0f64.to_bits() as u128, 0),
            0x0fa0,
        ),
        // IE unmasked: a QNaN to an integer.
        (
            "cvttsd2si rax",
            &[0xf2, 0x48, 0x0f, 0x2c, 0xc1],
            one,
            qnan,
            0x1f00,
        ),
        // IE unmasked: comisd with a QNaN; the flags stay.
        ("comisd", &[0x66, 0x0f, 0x2f, 0xc1], one, qnan, 0x1f00),
        // IE unmasked: sqrt of -1.
        (
            "sqrtsd",
            &[0xf2, 0x0f, 0x51, 0xc1],
            one,
            (-1.0f64).to_bits() as u128,
            0x1f00,
        ),
    ];
    for &(name, code, a, b, mxcsr) in cases {
        let module = runner.module(code);
        let mut m = runner.host.machine(MEMORY).unwrap();
        let rax = 0x1234_5678_9abc_def0;
        let rflags = 0x202 | abi::flags::ZF | abi::flags::SF;
        let out = runner.run(&mut m, &module, a, b, rax, rflags, mxcsr);
        assert_eq!(out.exit, abi::EXIT_UNSUPPORTED, "{name}");
        assert_eq!(out.rip, CODE, "{name}");
        assert_eq!((out.xmm0, out.xmm1), (a, b), "{name}: XMM changed");
        assert_eq!(out.mxcsr, mxcsr, "{name}: MXCSR changed");
        assert_eq!(out.gpr[0], rax, "{name}: RAX changed");
        assert_eq!(out.rflags, rflags, "{name}: RFLAGS changed");
        // The same operands with the exception masked complete.
        let out = runner.run(&mut m, &module, a, b, rax, rflags, 0x1f80);
        assert_eq!(out.exit, abi::EXIT_LEFT_CODE, "{name} (masked)");
    }
}
