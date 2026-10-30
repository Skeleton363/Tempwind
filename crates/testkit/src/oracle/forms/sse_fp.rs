//! Operand values and MXCSR of the SSE/SSE2 floating-point forms
//! (`docs/design/fixtures.md`, "Floating-point cases (M4)"). The generic
//! generator (`generic.rs`) supplies registers, memory operands and
//! encodings; this module decides what goes into them.
//!
//! - Every exception stays masked (MXCSR bits 7-12 set); RC, FTZ, DAZ and the
//!   sticky flags vary (PE clear in about half the cases, set in the rest).
//! - Edge values first, per lane format: pairs for the two-operand forms,
//!   single values for the others. Packed forms take a different edge in
//!   each lane, so classes mix across lanes; scalar forms see each edge
//!   under two rounding modes.
//! - Then seeded random values, with a bias towards equal and nearly equal
//!   operands (compares, cancellation).

use iced_x86::{Code, Mnemonic};

use super::Rng;

/// Lane format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fmt {
    /// binary32
    S,
    /// binary64
    D,
}

impl Fmt {
    fn bits(self) -> u32 {
        match self {
            Fmt::S => 32,
            Fmt::D => 64,
        }
    }

    fn mant(self) -> u32 {
        match self {
            Fmt::S => 23,
            Fmt::D => 52,
        }
    }

    fn emax(self) -> u64 {
        match self {
            Fmt::S => 0xff,
            Fmt::D => 0x7ff,
        }
    }

    fn bias(self) -> u64 {
        self.emax() / 2
    }

    fn frac_mask(self) -> u64 {
        (1 << self.mant()) - 1
    }

    fn make(self, neg: bool, exp: u64, frac: u64) -> u64 {
        (u64::from(neg) << (self.bits() - 1)) | (exp << self.mant()) | (frac & self.frac_mask())
    }

    /// `x` in this format (rounded to nearest for binary32).
    fn from_f64(self, x: f64) -> u64 {
        match self {
            Fmt::S => u64::from((x as f32).to_bits()),
            Fmt::D => x.to_bits(),
        }
    }
}

/// Named edge values.
#[derive(Clone, Copy, Debug)]
enum V {
    Z,
    NZ,
    DMin,
    DMax,
    NDMax,
    NMin,
    NMin1,
    NNMin,
    One,
    NOne,
    /// 1 + ulp
    OneUlp,
    /// The largest value below 1.
    OneM,
    /// Half an ulp of 1: 1 + HalfUlp is a tie.
    HalfUlp,
    Half,
    OneHalf,
    Two,
    Three,
    NThree,
    Ten,
    Max,
    NMax,
    Inf,
    NInf,
    /// The default QNaN (positive).
    QNan,
    /// The QNaN indefinite (negative).
    NQNan,
    /// A QNaN with a payload.
    QNanP,
    /// An SNaN with payload 1.
    SNan,
    /// A negative SNaN with another payload.
    NSNanP,
}

const ALL_V: [V; 28] = [
    V::Z,
    V::NZ,
    V::DMin,
    V::DMax,
    V::NDMax,
    V::NMin,
    V::NMin1,
    V::NNMin,
    V::One,
    V::NOne,
    V::OneUlp,
    V::OneM,
    V::HalfUlp,
    V::Half,
    V::OneHalf,
    V::Two,
    V::Three,
    V::NThree,
    V::Ten,
    V::Max,
    V::NMax,
    V::Inf,
    V::NInf,
    V::QNan,
    V::NQNan,
    V::QNanP,
    V::SNan,
    V::NSNanP,
];

fn v(f: Fmt, x: V) -> u64 {
    let (b, e, m) = (f.bias(), f.emax(), f.mant());
    let all = f.frac_mask();
    match x {
        V::Z => 0,
        V::NZ => f.make(true, 0, 0),
        V::DMin => 1,
        V::DMax => all,
        V::NDMax => f.make(true, 0, all),
        V::NMin => f.make(false, 1, 0),
        V::NMin1 => f.make(false, 1, 1),
        V::NNMin => f.make(true, 1, 0),
        V::One => f.make(false, b, 0),
        V::NOne => f.make(true, b, 0),
        V::OneUlp => f.make(false, b, 1),
        V::OneM => f.make(false, b - 1, all),
        V::HalfUlp => f.make(false, b - u64::from(m) - 1, 0),
        V::Half => f.make(false, b - 1, 0),
        V::OneHalf => f.make(false, b, 1 << (m - 1)),
        V::Two => f.make(false, b + 1, 0),
        V::Three => f.make(false, b + 1, 1 << (m - 1)),
        V::NThree => f.make(true, b + 1, 1 << (m - 1)),
        V::Ten => f.make(false, b + 3, 1 << (m - 2)),
        V::Max => f.make(false, e - 1, all),
        V::NMax => f.make(true, e - 1, all),
        V::Inf => f.make(false, e, 0),
        V::NInf => f.make(true, e, 0),
        V::QNan => f.make(false, e, 1 << (m - 1)),
        V::NQNan => f.make(true, e, 1 << (m - 1)),
        V::QNanP => f.make(false, e, (1 << (m - 1)) | 0x1234_5678_9abc_def0 & all),
        V::SNan => f.make(false, e, 1),
        V::NSNanP => f.make(true, e, (1 << (m - 2)) | 0x55),
    }
}

/// (destination, source) edge pairs of the two-operand forms: signed zeros,
/// rounding ties, overflow, underflow (exact, tiny and to zero), denormal
/// operands, invalid operations, division by zero and NaN propagation.
const PAIRS: [(V, V); 40] = [
    (V::One, V::One),
    (V::One, V::NOne),
    (V::Z, V::NZ),
    (V::NZ, V::NZ),
    (V::NZ, V::Z),
    // add: 1 + half an ulp ties to even (down); 1 + ulp + half an ulp up.
    (V::One, V::HalfUlp),
    (V::OneUlp, V::HalfUlp),
    // mul: (1 + ulp) * 1.5 = 1.5 + 1.5 ulp, a tie.
    (V::OneUlp, V::OneHalf),
    (V::Max, V::Max),
    (V::Max, V::Two),
    (V::NMax, V::Max),
    (V::NMax, V::Two),
    (V::NMin, V::Half),
    (V::NMin1, V::OneM),
    (V::DMin, V::Half),
    (V::DMax, V::DMin),
    (V::NMin1, V::NMin),
    (V::DMin, V::DMax),
    (V::NDMax, V::DMin),
    (V::NMin, V::Max),
    (V::One, V::DMin),
    (V::DMax, V::One),
    (V::Inf, V::Inf),
    (V::Inf, V::NInf),
    (V::Inf, V::Z),
    (V::Z, V::Z),
    (V::One, V::Z),
    (V::NOne, V::NZ),
    (V::One, V::Three),
    (V::Ten, V::NThree),
    (V::QNan, V::One),
    (V::One, V::QNanP),
    (V::SNan, V::One),
    (V::One, V::NSNanP),
    (V::QNanP, V::SNan),
    (V::SNan, V::QNanP),
    (V::NQNan, V::QNanP),
    (V::NSNanP, V::SNan),
    (V::DMin, V::DMin),
    (V::NInf, V::NMax),
];

/// A random value: any finite magnitude, near 1, denormals, extremes,
/// small integers, NaNs with random payloads, and the edges.
fn rand_fp(rng: &mut Rng, f: Fmt) -> u64 {
    let neg = rng.chance(1, 2);
    let frac = rng.next_u64();
    let e = f.emax();
    match rng.below(12) {
        0 | 1 => v(f, rng.pick(&ALL_V)),
        2 => f.make(neg, 0, frac),
        3 | 4 => f.make(neg, f.bias() - 4 + rng.below(9), frac),
        5 => f.make(neg, 1 + rng.below(4), frac),
        6 => f.make(neg, e - 1 - rng.below(4), frac),
        7 => {
            let n = rng.range_i64(-1000, 1000) as f64;
            f.from_f64(n)
        }
        8 => f.make(neg, e, frac | 1),
        _ => f.make(neg, 1 + rng.below(e - 1), frac),
    }
}

/// A source value related to `a`: equal, negated, or a few ulps away.
fn related(rng: &mut Rng, f: Fmt, a: u64) -> u64 {
    let sign = 1u64 << (f.bits() - 1);
    match rng.below(4) {
        0 => a,
        1 => a ^ sign,
        2 => a ^ (rng.below(8) + 1),
        _ => (a & !f.frac_mask()) | (rng.next_u64() & f.frac_mask()),
    }
}

/// What a form's source operand holds.
#[derive(Clone, Copy, Debug)]
enum Dom {
    /// Floating point in this format.
    Float(Fmt),
    /// Floating point near the limits of an `n`-bit integer conversion.
    IntLim(Fmt, u32),
    /// binary64 values around the binary32 range (narrowing).
    Narrow,
    /// `n`-bit integers.
    Int(u32),
}

impl Dom {
    fn lane_bits(self) -> u32 {
        match self {
            Dom::Float(f) | Dom::IntLim(f, _) => f.bits(),
            Dom::Narrow => 64,
            Dom::Int(n) => n,
        }
    }
}

/// How a form uses its operands.
#[derive(Clone, Copy, Debug)]
struct Info {
    /// The destination is also a floating-point source (add, cmp, comis).
    binary: bool,
    src: Dom,
    /// Significant source lanes.
    lanes: u32,
    /// Format of the destination's lanes (for the values it starts with).
    dst: Option<Fmt>,
    /// cmpps/cmpss/cmppd/cmpsd: the predicate immediate.
    predicate: bool,
}

/// The SSE floating-point mnemonics and how they use their operands, or
/// `None` for any other form.
fn info(code: Code) -> Option<Info> {
    use Fmt::{D, S};
    use Mnemonic as M;
    // The GPR operand's width: `Cvtsi2sd_xmm_rm64`, `Cvtsd2si_r64_xmmm64`.
    let name = format!("{code:?}");
    let gpr64 = name.contains("_r64") || name.contains("_rm64");
    let ib = if gpr64 { 64 } else { 32 };
    let bin = |f, lanes| Info {
        binary: true,
        src: Dom::Float(f),
        lanes,
        dst: Some(f),
        predicate: false,
    };
    let un = |src, lanes, dst| Info {
        binary: false,
        src,
        lanes,
        dst,
        predicate: false,
    };
    Some(match code.mnemonic() {
        M::Addps | M::Subps | M::Mulps | M::Divps | M::Minps | M::Maxps => bin(S, 4),
        M::Addss | M::Subss | M::Mulss | M::Divss | M::Minss | M::Maxss => bin(S, 1),
        M::Addpd | M::Subpd | M::Mulpd | M::Divpd | M::Minpd | M::Maxpd => bin(D, 2),
        M::Addsd | M::Subsd | M::Mulsd | M::Divsd | M::Minsd | M::Maxsd => bin(D, 1),
        M::Comiss | M::Ucomiss => bin(S, 1),
        M::Comisd | M::Ucomisd => bin(D, 1),
        M::Cmpps => Info {
            predicate: true,
            ..bin(S, 4)
        },
        M::Cmpss => Info {
            predicate: true,
            ..bin(S, 1)
        },
        M::Cmppd => Info {
            predicate: true,
            ..bin(D, 2)
        },
        M::Cmpsd => Info {
            predicate: true,
            ..bin(D, 1)
        },
        M::Sqrtps => un(Dom::Float(S), 4, Some(S)),
        M::Sqrtss => un(Dom::Float(S), 1, Some(S)),
        M::Sqrtpd => un(Dom::Float(D), 2, Some(D)),
        M::Sqrtsd => un(Dom::Float(D), 1, Some(D)),
        M::Cvtsi2ss => un(Dom::Int(ib), 1, Some(S)),
        M::Cvtsi2sd => un(Dom::Int(ib), 1, Some(D)),
        M::Cvtss2si | M::Cvttss2si => un(Dom::IntLim(S, ib), 1, None),
        M::Cvtsd2si | M::Cvttsd2si => un(Dom::IntLim(D, ib), 1, None),
        M::Cvtss2sd => un(Dom::Float(S), 1, Some(D)),
        M::Cvtsd2ss => un(Dom::Narrow, 1, Some(S)),
        M::Cvtps2pd => un(Dom::Float(S), 2, None),
        M::Cvtpd2ps => un(Dom::Narrow, 2, None),
        M::Cvtdq2ps => un(Dom::Int(32), 4, None),
        M::Cvtps2dq | M::Cvttps2dq => un(Dom::IntLim(S, 32), 4, None),
        M::Cvtdq2pd => un(Dom::Int(32), 2, None),
        M::Cvtpd2dq | M::Cvttpd2dq => un(Dom::IntLim(D, 32), 2, None),
        _ => return None,
    })
}

/// Whether `code` is one of the SSE floating-point forms of this module.
pub(super) fn is_fp(code: Code) -> bool {
    info(code).is_some()
}

/// Edge values of a single-operand domain.
fn dom_edges(d: Dom) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    let mut push = |x: u64| {
        if !out.contains(&x) {
            out.push(x);
        }
    };
    match d {
        Dom::Float(f) => ALL_V.iter().for_each(|&x| push(v(f, x))),
        Dom::IntLim(f, n) => {
            let lim = 2f64.powi(n as i32 - 1);
            for x in [
                lim,
                -lim,
                lim - 0.5,
                -lim - 0.5,
                -lim - 1.0,
                lim - 1.0,
                lim + 1.0,
                lim * 2.0,
                // The largest values below the limit in each format.
                f64::from_bits(lim.to_bits() - 1),
                f64::from(f32::from_bits((lim as f32).to_bits() - 1)),
                -f64::from(f32::from_bits((lim as f32).to_bits() - 1)),
                0.5,
                1.5,
                2.5,
                -0.5,
                -1.5,
                -2.5,
                0.49999997,
                0.0,
                -0.0,
                1.0,
                -1.0,
                1e10,
                -1e10,
                123456.75,
            ] {
                push(f.from_f64(x));
            }
            for x in [
                V::DMin,
                V::NDMax,
                V::Max,
                V::NMax,
                V::Inf,
                V::NInf,
                V::QNan,
                V::SNan,
                V::NSNanP,
                V::OneM,
            ] {
                push(v(f, x));
            }
        }
        Dom::Narrow => {
            let fmax = f64::from(f32::MAX);
            let fulp = 2f64.powi(104); // half an ulp of f32::MAX
            let fmin = f64::from(f32::MIN_POSITIVE);
            let dmin = 2f64.powi(-149);
            for x in [
                fmax,
                -fmax,
                fmax + fulp,
                fmax + fulp / 2.0,
                fmax + fulp * 1.5,
                -(fmax + fulp),
                fmin,
                fmin * (1.0 - 2f64.powi(-24)),
                dmin,
                dmin / 2.0,
                dmin * 0.75,
                -dmin / 2.0,
                dmin * 1.5,
                1.0 + 2f64.powi(-24),
                1.0 + 2f64.powi(-23) + 2f64.powi(-24),
                1.0 + 2f64.powi(-24) + 2f64.powi(-52),
                -(1.0 + 2f64.powi(-24)),
                1.0,
                -0.0,
                0.0,
                1e-50,
                -1e50,
                1.0 / 3.0,
            ] {
                push(x.to_bits());
            }
            for x in ALL_V {
                push(v(Fmt::D, x));
            }
            // NaNs whose payload binary32 truncates.
            push(Fmt::D.make(false, 0x7ff, 0x0000_0000_1fff_ffff));
            push(Fmt::D.make(true, 0x7ff, 0x0008_0000_0000_0001));
        }
        Dom::Int(n) => {
            let m = if n == 64 { u64::MAX } else { (1 << n) - 1 };
            for x in [
                0u64,
                1,
                u64::MAX,
                1 << (n - 1),
                (1 << (n - 1)) - 1,
                (1 << 24) + 1,
                (1 << 24) + 3,
                ((1 << 24) + 1u64).wrapping_neg(),
                (1 << 25) + 2,
                (1 << 53) + 1,
                (1 << 53) + 3,
                ((1 << 53) + 1u64).wrapping_neg(),
                (1 << 54) + 2,
                0x7fff_ffbf,
                0x7fff_ffc0,
                0x8000_0040,
                0xffff_ffff,
                0x1_0000_0000,
                0x7fff_ffff_ffff_fdff,
                0x7fff_ffff_ffff_fe00,
                0x8000_0000_0000_0400,
                12345,
                (12345u64).wrapping_neg(),
            ] {
                push(x & m);
            }
        }
    }
    out
}

/// A random value of a domain.
fn rand_dom(rng: &mut Rng, d: Dom) -> u64 {
    match d {
        Dom::Float(f) => rand_fp(rng, f),
        Dom::IntLim(f, n) => match rng.below(5) {
            0 => rand_fp(rng, f),
            1 => {
                let e = dom_edges(d);
                rng.pick(&e)
            }
            2 => {
                // Halves and quarters of small integers: ties and roundings.
                let x = rng.range_i64(-64, 64) as f64 / 4.0;
                f.from_f64(x)
            }
            _ => {
                // Any magnitude up to just past the limit.
                let k = rng.below(u64::from(n) + 2) as i32;
                let r = (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
                let x = 2f64.powi(k) * (1.0 + r);
                f.from_f64(if rng.chance(1, 2) { -x } else { x })
            }
        },
        Dom::Narrow => match rng.below(5) {
            0 => rand_fp(rng, Fmt::D),
            1 => {
                let e = dom_edges(d);
                rng.pick(&e)
            }
            _ => {
                // Exponents around the binary32 range, random significand.
                let neg = rng.chance(1, 2);
                let exp = 1023 - 160 + rng.below(300);
                Fmt::D.make(neg, exp, rng.next_u64())
            }
        },
        Dom::Int(n) => {
            let m = if n == 64 { u64::MAX } else { (1 << n) - 1 };
            let x = match rng.below(4) {
                0 => {
                    let e = dom_edges(d);
                    rng.pick(&e)
                }
                1 => {
                    let k = rng.below(u64::from(n));
                    let x = (1u64 << k) | (rng.next_u64() & ((1 << k) - 1));
                    if rng.chance(1, 2) {
                        x.wrapping_neg()
                    } else {
                        x
                    }
                }
                _ => rng.next_u64(),
            };
            x & m
        }
    }
}

/// Random XMM bits for lanes nothing reads.
fn rand_u128(rng: &mut Rng) -> u128 {
    u128::from(rng.next_u64()) | u128::from(rng.next_u64()) << 64
}

/// Random lanes of format `f`.
fn rand_lanes(rng: &mut Rng, f: Fmt) -> u128 {
    let bits = f.bits();
    let mut out = 0u128;
    for l in 0..128 / bits {
        out |= u128::from(rand_fp(rng, f)) << (l * bits);
    }
    out
}

/// The inputs of one case.
pub(super) struct Plan {
    /// The destination register's value (XMM or GPR).
    pub dst: u128,
    /// The source operand's value (XMM, GPR or memory, low bits).
    pub src: u128,
    /// The predicate of cmpps and friends.
    pub imm: Option<u8>,
    pub mxcsr: u32,
}

/// MXCSR with every exception masked: RC (`rc` in edge cases), FTZ, DAZ
/// and the sticky flags vary; PE is set in about half the cases.
fn mxcsr(rng: &mut Rng, rc: Option<u32>) -> u32 {
    let rc = rc.unwrap_or_else(|| rng.below(4) as u32);
    let pe = rng.chance(1, 2);
    let others = if rng.chance(1, 3) {
        rng.below(32) as u32
    } else {
        0
    };
    let ftz = rng.chance(1, 4);
    let daz = rng.chance(1, 4);
    0x1f80 | rc << 13 | u32::from(ftz) << 15 | u32::from(daz) << 6 | others | u32::from(pe) << 5
}

/// Case `idx` of SSE floating-point form `code`.
pub(super) fn plan(rng: &mut Rng, idx: usize, code: Code) -> Plan {
    let info = info(code).expect("an SSE floating-point form");
    let lane_bits = info.src.lane_bits();
    let lanes = info.lanes as usize;
    let fmt = match info.src {
        Dom::Float(f) => f,
        _ => Fmt::D,
    };
    // Edge phase: two passes over the edges, under different rounding modes.
    let n = if info.binary {
        PAIRS.len()
    } else {
        dom_edges(info.src).len()
    };
    let edge = idx < 2 * n;
    let rc = edge.then(|| ((idx % n + 2 * (idx / n)) % 4) as u32);
    // Lane l of an edge case takes edge (idx + l * stride) mod n.
    let stride = n / lanes + 1;
    let mut dst = match info.dst {
        Some(f) => rand_lanes(rng, f),
        None => rand_u128(rng),
    };
    let mut src = rand_u128(rng);
    let lane_mask = if lane_bits == 64 {
        u128::from(u64::MAX)
    } else {
        (1u128 << lane_bits) - 1
    };
    for l in 0..lanes {
        let (a, b) = if info.binary {
            if edge {
                let (x, y) = PAIRS[(idx + l * stride) % n];
                (v(fmt, x), v(fmt, y))
            } else {
                let a = rand_fp(rng, fmt);
                let b = if rng.chance(2, 5) {
                    related(rng, fmt, a)
                } else {
                    rand_fp(rng, fmt)
                };
                (a, b)
            }
        } else if edge {
            let e = dom_edges(info.src);
            (0, e[(idx + l * stride) % n])
        } else {
            (0, rand_dom(rng, info.src))
        };
        let sh = l as u32 * lane_bits;
        if info.binary {
            dst = (dst & !(lane_mask << sh)) | (u128::from(a) << sh);
        }
        src = (src & !(lane_mask << sh)) | (u128::from(b) << sh);
    }
    let imm = info.predicate.then_some((idx % 8) as u8);
    let mxcsr = mxcsr(rng, rc);
    Plan {
        dst,
        src,
        imm,
        mxcsr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_widths() {
        let ib = |c| match info(c).unwrap().src {
            Dom::Int(n) | Dom::IntLim(_, n) => n,
            _ => 0,
        };
        assert_eq!(ib(Code::Cvtsd2si_r32_xmmm64), 32);
        assert_eq!(ib(Code::Cvtsd2si_r64_xmmm64), 64);
        assert_eq!(ib(Code::Cvtsi2sd_xmm_rm32), 32);
        assert_eq!(ib(Code::Cvtsi2ss_xmm_rm64), 64);
        assert_eq!(ib(Code::Cvttss2si_r64_xmmm32), 64);
        assert_eq!(ib(Code::Cvtpd2dq_xmm_xmmm128), 32);
    }

    #[test]
    fn edge_values() {
        assert_eq!(v(Fmt::S, V::One), 0x3f80_0000);
        assert_eq!(v(Fmt::D, V::One), 0x3ff0_0000_0000_0000);
        assert_eq!(f32::from_bits(v(Fmt::S, V::HalfUlp) as u32), 2f32.powi(-24));
        assert_eq!(f64::from_bits(v(Fmt::D, V::HalfUlp)), 2f64.powi(-53));
        assert_eq!(f64::from_bits(v(Fmt::D, V::OneHalf)), 1.5);
        assert_eq!(f32::from_bits(v(Fmt::S, V::Three) as u32), 3.0);
        assert_eq!(f32::from_bits(v(Fmt::S, V::Ten) as u32), 10.0);
        assert_eq!(f64::from_bits(v(Fmt::D, V::Max)), f64::MAX);
        assert_eq!(f64::from_bits(v(Fmt::D, V::NMin)), f64::MIN_POSITIVE);
        assert_eq!(
            f32::from_bits(v(Fmt::S, V::OneM) as u32),
            1.0 - 2f32.powi(-24)
        );
        assert_eq!(v(Fmt::S, V::QNan), 0x7fc0_0000);
        assert_eq!(v(Fmt::S, V::NQNan), 0xffc0_0000);
        assert!(f32::from_bits(v(Fmt::S, V::SNan) as u32).is_nan());
        // Every edge value is a valid lane.
        for d in [
            Dom::Float(Fmt::S),
            Dom::IntLim(Fmt::S, 32),
            Dom::IntLim(Fmt::D, 64),
            Dom::Narrow,
            Dom::Int(32),
        ] {
            let bits = d.lane_bits();
            assert!(dom_edges(d).iter().all(|&x| bits == 64 || x >> bits == 0));
        }
    }
}
