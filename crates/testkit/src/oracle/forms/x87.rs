//! x87 forms (`instruction-scope.md`, "M4 additions to the scope"): loads,
//! stores, arithmetic, compares and the control forms, with the stack
//! states, control words and values of `docs/design/fixtures.md`,
//! "Floating-point cases (M4)".
//!
//! Per case:
//! - TOP = case index mod 8, so every form sees every TOP;
//! - blocks of 8 cases take turns through the stack scenarios: operands
//!   present, an empty operand (underflow), a full ST(7) under a push
//!   (overflow), a full and an empty stack;
//! - FCW: every exception masked, PC single, double or extended (never the
//!   reserved 01), RC random; the control forms also load and store other
//!   masks, which raise nothing by themselves;
//! - FSW: condition codes and sticky flags random, ES and B consistent with
//!   FCW (clear unless an unmasked flag is pending, and then only for the
//!   non-waiting forms, which don't take #MF);
//! - tags consistent with the contents (`fixture::full_tag_word`);
//! - values: edges first, then random; m80 operands include unnormals,
//!   pseudo-denormals, pseudo-NaNs and pseudo-infinities;
//! - the condition codes the SDM leaves undefined per instruction go into
//!   `undefined.fsw`; `fnstenv`'s reserved halves, pointers, opcode and
//!   selectors into `undefined.mem`.

use iced_x86::Mnemonic;

use super::special::Special;
use super::sse_fp::{self, Fmt};
use super::*;
use crate::oracle::fixture::full_tag_word;

/// Memory operand formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mf {
    F32,
    F64,
    F80,
    I16,
    I32,
    I64,
}

impl Mf {
    fn size(self) -> u64 {
        match self {
            Mf::F32 | Mf::I32 => 4,
            Mf::F64 | Mf::I64 => 8,
            Mf::F80 => 10,
            Mf::I16 => 2,
        }
    }
}

/// What a form does, as far as its inputs are concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Op {
    /// fld m32/m64/m80, fild m16/m32/m64: push from memory.
    Load(Mf),
    /// fld st(i).
    LoadSt,
    /// fldz.
    Ldz,
    /// fstp m32/m64/m80, fistp m16/m32/m64.
    Store(Mf),
    /// fstp st(i).
    StoreSt,
    /// fxch st(i).
    Xch,
    /// fabs, fchs, fsqrt, frndint (`Code` tells which).
    Unary,
    /// fscale.
    Scale,
    /// fadd m32, fmul m64.
    ArithMem(Mf),
    /// The register forms of fadd, fmul, fdiv, ... (st(0), st(i) or
    /// st(i), st(0), with or without a pop).
    Arith,
    /// fcomp m64, ficomp m16.
    ComMem(Mf),
    /// fcomi, fucomi, fucomip.
    Comi,
    Fnstcw,
    Fldcw,
    FnstswMem,
    FnstswAx,
    Fnstenv,
    Fldenv,
    Fnclex,
    Fwait,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Spec {
    pub code: Code,
    pub op: Op,
}

/// Cases per data form: 8 TOPs times 8 stack scenarios.
const COUNT: usize = 64;
/// Cases per control form.
const CONTROL_COUNT: usize = 48;

/// The x87 forms, family by family.
pub(super) fn forms(out: &mut Vec<Form>) {
    use Code as C;
    use Mf::*;
    let families: [(&'static str, &[(Code, Op)]); 5] = [
        (
            "x87_load",
            &[
                (C::Fld_m64fp, Op::Load(F64)),
                (C::Fld_m80fp, Op::Load(F80)),
                (C::Fild_m32int, Op::Load(I32)),
                (C::Fild_m64int, Op::Load(I64)),
                (C::Fld_sti, Op::LoadSt),
                (C::Fldz, Op::Ldz),
                (C::Fxch_st0_sti, Op::Xch),
            ],
        ),
        (
            "x87_store",
            &[
                (C::Fstp_m32fp, Op::Store(F32)),
                (C::Fstp_m64fp, Op::Store(F64)),
                (C::Fstp_m80fp, Op::Store(F80)),
                (C::Fistp_m16int, Op::Store(I16)),
                (C::Fistp_m32int, Op::Store(I32)),
                (C::Fistp_m64int, Op::Store(I64)),
                (C::Fstp_sti, Op::StoreSt),
            ],
        ),
        (
            "x87_arith",
            &[
                (C::Fadd_m32fp, Op::ArithMem(F32)),
                (C::Fmul_m64fp, Op::ArithMem(F64)),
                (C::Fadd_st0_sti, Op::Arith),
                (C::Faddp_sti_st0, Op::Arith),
                (C::Fsubp_sti_st0, Op::Arith),
                (C::Fmul_sti_st0, Op::Arith),
                (C::Fmulp_sti_st0, Op::Arith),
                (C::Fdiv_st0_sti, Op::Arith),
                (C::Fdiv_sti_st0, Op::Arith),
                (C::Fdivp_sti_st0, Op::Arith),
                (C::Fdivr_st0_sti, Op::Arith),
                (C::Fdivrp_sti_st0, Op::Arith),
                (C::Fabs, Op::Unary),
                (C::Fchs, Op::Unary),
                (C::Fsqrt, Op::Unary),
                (C::Frndint, Op::Unary),
                (C::Fscale, Op::Scale),
            ],
        ),
        (
            "x87_cmp",
            &[
                (C::Fcomp_m64fp, Op::ComMem(F64)),
                (C::Ficomp_m16int, Op::ComMem(I16)),
                (C::Fcomi_st0_sti, Op::Comi),
                (C::Fucomi_st0_sti, Op::Comi),
                (C::Fucomip_st0_sti, Op::Comi),
            ],
        ),
        (
            "x87_control",
            &[
                (C::Fnstcw_m2byte, Op::Fnstcw),
                (C::Fldcw_m2byte, Op::Fldcw),
                (C::Fnstsw_m2byte, Op::FnstswMem),
                (C::Fnstsw_AX, Op::FnstswAx),
                (C::Fnstenv_m28byte, Op::Fnstenv),
                (C::Fldenv_m28byte, Op::Fldenv),
                (C::Fnclex, Op::Fnclex),
                (C::Wait, Op::Fwait),
            ],
        ),
    ];
    for (family, codes) in families {
        for &(code, op) in codes {
            let mem = mem_format(op).is_some();
            let count = if is_control(op) { CONTROL_COUNT } else { COUNT };
            out.push(Form {
                id: format!("{code:?}/{}", if mem { "mem" } else { "reg" }),
                family,
                count,
                kind: Kind::Special(Special::X87(Spec { code, op })),
            });
        }
    }
}

fn is_control(op: Op) -> bool {
    matches!(
        op,
        Op::Fnstcw
            | Op::Fldcw
            | Op::FnstswMem
            | Op::FnstswAx
            | Op::Fnstenv
            | Op::Fldenv
            | Op::Fnclex
            | Op::Fwait
    )
}

/// The memory operand's format, if the form has one (the control forms:
/// `I16` for m2byte; m28 is `F80` here, its size is in `mem_size`).
fn mem_format(op: Op) -> Option<Mf> {
    match op {
        Op::Load(m) | Op::Store(m) | Op::ArithMem(m) | Op::ComMem(m) => Some(m),
        Op::Fnstcw | Op::Fldcw | Op::FnstswMem => Some(Mf::I16),
        Op::Fnstenv | Op::Fldenv => Some(Mf::F80),
        _ => None,
    }
}

fn mem_size(op: Op) -> u64 {
    match op {
        Op::Fnstenv | Op::Fldenv => 28,
        _ => mem_format(op).map_or(0, Mf::size),
    }
}

// ---------------------------------------------------------------------------
// 80-bit values

const SIGN: u128 = 1 << 79;
const J: u64 = 1 << 63;

fn f80(neg: bool, exp: u64, sig: u64) -> u128 {
    (if neg { SIGN } else { 0 }) | u128::from(exp & 0x7fff) << 64 | u128::from(sig)
}

/// 2^e times the significand `sig` (J bit at bit 63).
fn f80e(neg: bool, e: i64, sig: u64) -> u128 {
    f80(neg, (16383 + e) as u64, sig)
}

/// An f64 value as an 80-bit register value (exact).
fn from_f64(x: f64) -> u128 {
    let b = x.to_bits();
    let neg = b >> 63 == 1;
    let e = (b >> 52) & 0x7ff;
    let frac = b & ((1 << 52) - 1);
    match e {
        0 if frac == 0 => f80(neg, 0, 0),
        0 => {
            // Denormal: normalize.
            let lz = frac.leading_zeros() as i64; // >= 12
            let sig = frac << lz;
            f80e(neg, -1022 - (lz - 11), sig)
        }
        0x7ff => f80(neg, 0x7fff, J | frac << 11),
        _ => f80e(neg, e as i64 - 1023, J | frac << 11),
    }
}

/// n / 2 as an 80-bit value (n up to 2^66; low bits beyond 64 are cut).
fn half_int(neg: bool, n: u128) -> u128 {
    if n == 0 {
        return f80(neg, 0, 0);
    }
    let len = 128 - n.leading_zeros() as i64;
    let sig = if len > 64 {
        (n >> (len - 64)) as u64
    } else {
        (n << (64 - len)) as u64
    };
    f80e(neg, len - 2, sig)
}

/// Edge register values: zeros, ±1 and neighbours, ties under PC single and
/// double, the extremes, denormals and pseudo-denormals, unnormals,
/// infinities, NaNs (quiet, signaling, indefinite) and the unsupported
/// pseudo-NaN and pseudo-infinity.
fn edges80() -> Vec<u128> {
    vec![
        f80(false, 0, 0),
        f80(true, 0, 0),
        f80e(false, 0, J),
        f80e(true, 0, J),
        f80e(false, 0, J | 1),
        f80e(false, 0, J | 1 << 39),  // 1 + 2^-24: a tie under PC single
        f80e(false, 0, J | 1 << 10),  // 1 + 2^-53: a tie under PC double
        f80e(false, 0, J | 3 << 39),  // 1 + 3 * 2^-24
        f80e(false, -1, u64::MAX),    // largest below 1
        f80e(false, 0, J | 1 << 62),  // 1.5
        f80e(true, 1, J | 1 << 62),   // -3
        f80e(false, 3, J | 1 << 61),  // 10
        f80e(false, -1, J),           // 0.5
        f80(false, 0x7ffe, u64::MAX), // max
        f80(true, 0x7ffe, u64::MAX),  // -max
        f80(false, 1, J),             // min normal
        f80(false, 0, 1),             // min denormal
        f80(true, 0, u64::MAX >> 1),  // -max denormal
        f80(false, 0, J | 5),         // pseudo-denormal
        f80(false, 0x3fff, 1 << 62),  // unnormal
        f80(false, 0x7fff, J),        // inf
        f80(true, 0x7fff, J),         // -inf
        f80(false, 0x7fff, J | 1 << 62), // QNaN
        f80(true, 0x7fff, J | 1 << 62), // indefinite
        f80(false, 0x7fff, J | 1 << 62 | 0x1234_5678),
        f80(false, 0x7fff, J | 1),               // SNaN
        f80(true, 0x7fff, J | 0x0abc_def0_0000), // SNaN, payload
        f80(false, 0x7fff, 1 << 62),             // pseudo-NaN
        f80(false, 0x7fff, 0),                   // pseudo-infinity
        from_f64(f64::MAX),
        from_f64(f64::MIN_POSITIVE),
        from_f64(f32::MAX as f64),
    ]
}

/// A random register value.
fn rand80(rng: &mut Rng) -> u128 {
    let neg = rng.chance(1, 2);
    let sig = rng.next_u64();
    match rng.below(16) {
        0..=2 => {
            let e = edges80();
            rng.pick(&e)
        }
        3 => f80(neg, 0, sig >> 1),
        4 => f80(neg, 0x7fff, sig | J),
        5 => f80(neg, 1 + rng.below(0x7ffe), sig & !J), // unnormal
        6 | 7 => f80e(neg, rng.range_i64(-8, 8), sig | J),
        8 => f80e(neg, rng.range_i64(-1100, 1100), sig | J),
        9 => from_f64(rng.range_i64(-100_000, 100_000) as f64),
        10 => from_f64(rng.range_i64(-400, 400) as f64 / 4.0),
        11 => f80(neg, 1 + rng.below(8), sig | J),
        12 => f80(neg, 0x7ffe - rng.below(8), sig | J),
        _ => f80(neg, 1 + rng.below(0x7ffe), sig | J),
    }
}

/// A value related to `a`: equal, negated, a few ulps away, or the same
/// exponent.
fn related80(rng: &mut Rng, a: u128) -> u128 {
    match rng.below(4) {
        0 => a,
        1 => a ^ SIGN,
        2 => a ^ u128::from(rng.below(8) + 1),
        _ => (a & !u128::from(u64::MAX >> 1)) | u128::from(rng.next_u64() >> 1),
    }
}

/// Values at the limits of an `n`-bit integer store, and halves around 0.
fn int_limits80(n: u32) -> Vec<u128> {
    let lim = 1u128 << (n - 1); // 2^(n-1)
    let h = |neg, twice: u128| half_int(neg, twice);
    vec![
        h(false, 2 * lim),     // 2^(n-1): out of range
        h(true, 2 * lim),      // -2^(n-1): in range
        h(false, 2 * lim - 1), // 2^(n-1) - 0.5
        h(false, 2 * lim - 2), // 2^(n-1) - 1
        h(true, 2 * lim + 1),  // -2^(n-1) - 0.5
        h(true, 2 * lim + 2),  // -2^(n-1) - 1
        h(false, 2 * lim + 1), // 2^(n-1) + 0.5
        h(false, 1),
        h(false, 3),
        h(false, 5),
        h(true, 1),
        h(true, 3),
        h(false, 0),
        h(true, 0),
        f80e(false, -2, J | 1),    // just above 0.25
        f80e(false, -1, u64::MAX), // just below 1
        f80(false, 0, 1),
        f80(false, 0x7fff, J),
        f80(true, 0x7fff, J | 1 << 62),
        f80(false, 0x7fff, J | 1),
        f80(false, 0x3fff, 1 << 62), // unnormal
        h(false, 2 * 12345 + 1),
    ]
}

/// Values around the range and precision of a binary32 or binary64 store.
fn narrow_limits80(f: Fmt) -> Vec<u128> {
    let (emax, emin, p) = match f {
        Fmt::S => (127i64, -126i64, 24u32),
        Fmt::D => (1023, -1022, 53),
    };
    let ones = u64::MAX << (64 - p); // p significant bits
    let half = 1u64 << (63 - p); // half an ulp
    vec![
        f80e(false, emax, ones),             // max
        f80e(false, emax, ones | half),      // max + half an ulp: overflow tie
        f80e(true, emax, ones | (half - 1)), // just below the tie
        f80e(false, emax + 1, J),            // overflow
        f80e(false, emin, J),                // min normal
        f80e(false, emin - 1, J | half),     // tiny
        f80e(false, emin - p as i64 + 1, J), // min denormal
        f80e(false, emin - p as i64, J),     // half the min denormal: tie to 0
        f80e(true, emin - p as i64, J | 1),  // just above
        f80e(false, emin - 200, J),          // underflow to 0
        f80e(false, 0, J | half),            // 1 + half an ulp: tie
        f80e(false, 0, J | (3 * half)),      // 1 + 1.5 ulps: tie, up
        f80e(true, 0, J | half | 1),
        f80(false, 0x7fff, J | 1 << 62 | 0x7ff), // NaN payload cut
        f80(false, 0x7fff, J | 1 << 40),         // SNaN
        f80(false, 0x3fff, 1 << 62),             // unnormal
        f80(false, 0, J | 5),                    // pseudo-denormal
    ]
}

/// The value of an m80/m64/m32/integer memory operand, as bytes.
fn mem_value(rng: &mut Rng, mf: Mf, idx: usize) -> Vec<u8> {
    let pick =
        |rng: &mut Rng, e: &[u64]| -> u64 { if idx < e.len() { e[idx] } else { rng.pick(e) } };
    let le = |v: u128, n: u64| v.to_le_bytes()[..n as usize].to_vec();
    match mf {
        Mf::F80 => {
            let e = edges80();
            let v = if idx < e.len() { e[idx] } else { rand80(rng) };
            le(v, 10)
        }
        Mf::F32 | Mf::F64 => {
            let f = if mf == Mf::F32 { Fmt::S } else { Fmt::D };
            let e: Vec<u64> = sse_fp::ALL_V.iter().map(|&x| sse_fp::v(f, x)).collect();
            let v = if idx < e.len() {
                e[idx]
            } else {
                sse_fp::rand_fp(rng, f)
            };
            le(u128::from(v), mf.size())
        }
        Mf::I16 | Mf::I32 | Mf::I64 => {
            let bits = mf.size() as u32 * 8;
            let mut e = edges(bits);
            e.extend([(1 << 53) + 1, (1 << 63) - 1, 12345, 2, 3, 0xfff6]);
            let v = if idx < e.len() || rng.chance(1, 3) {
                pick(rng, &e)
            } else {
                rand_val(rng, bits)
            };
            le(u128::from(v), mf.size())
        }
    }
}

/// An 80-bit value from the first bytes of an integer or float memory
/// operand, for compares against a register (related operands).
fn as_reg_value(mf: Mf, bytes: &[u8]) -> Option<u128> {
    let mut b = [0u8; 16];
    b[..bytes.len()].copy_from_slice(bytes);
    let raw = u128::from_le_bytes(b);
    Some(match mf {
        Mf::F64 => from_f64(f64::from_bits(raw as u64)),
        Mf::F32 => from_f64(f64::from(f32::from_bits(raw as u32))),
        Mf::I16 => {
            let v = raw as u16 as i16 as i64;
            half_int(v < 0, u128::from(v.unsigned_abs()) * 2)
        }
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Control and status words

/// FCW with every exception masked: PC single, double or extended, RC any.
fn masked_fcw(rng: &mut Rng) -> u16 {
    let pc = rng.pick(&[0u16, 2, 3]);
    let rc = rng.below(4) as u16;
    0x7f | pc << 8 | rc << 10
}

/// FCW for the control forms: any masks, PC and RC.
fn any_fcw(rng: &mut Rng) -> u16 {
    let masks = if rng.chance(1, 3) {
        0x3f
    } else {
        rng.below(64) as u16
    };
    (masked_fcw(rng) & !0x3f) | masks
}

/// FSW bits: condition codes and sticky flags (with SF), TOP given; ES and B
/// set exactly when a flag is pending under `fcw`.
fn fsw(rng: &mut Rng, top: u16, fcw: u16) -> u16 {
    let cc = rng.next_u64() as u16 & 0x4700;
    let flags = match rng.below(3) {
        0 => 0,
        1 => rng.below(128) as u16,
        _ => rng.pick(&[0x20u16, 0x01, 0x41, 0x10, 0x08, 0x04, 0x02, 0x30]),
    };
    with_es(cc | flags | top << 11, fcw)
}

/// Sets ES and B from the pending flags (flags & !masks).
fn with_es(fsw: u16, fcw: u16) -> u16 {
    let pending = fsw & !fcw & 0x3f != 0;
    (fsw & !0x8080) | if pending { 0x8080 } else { 0 }
}

// ---------------------------------------------------------------------------
// Cases

/// The stack scenarios, one per block of 8 cases (one per TOP).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scen {
    Normal,
    /// An operand is empty (readers), or ST(7) is full (pushes).
    Fault,
    Full,
    Empty,
}

const SCENARIOS: [Scen; 8] = [
    Scen::Normal,
    Scen::Normal,
    Scen::Fault,
    Scen::Normal,
    Scen::Full,
    Scen::Normal,
    Scen::Empty,
    Scen::Normal,
];

fn st_reg(i: usize) -> Register {
    [
        Register::ST0,
        Register::ST1,
        Register::ST2,
        Register::ST3,
        Register::ST4,
        Register::ST5,
        Register::ST6,
        Register::ST7,
    ][i]
}

pub(super) fn case(
    rng: &mut Rng,
    form: &Form,
    idx: usize,
    defaults: &[u64; 16],
    spec: Spec,
) -> Result<Option<GenCase>> {
    let Spec { code, op } = spec;
    let mut b = Builder::new(rng, defaults);
    let top = if idx < 64 {
        (idx % 8) as u16
    } else {
        rng.below(8) as u16
    };
    let scen = SCENARIOS[(idx / 8) % 8];
    let m = code.mnemonic();

    // Control word and status word.
    let waiting = !matches!(
        op,
        Op::Fnstcw | Op::FnstswMem | Op::FnstswAx | Op::Fnstenv | Op::Fnclex
    );
    let fcw = if is_control(op) && op != Op::Fldenv {
        any_fcw(rng)
    } else {
        masked_fcw(rng)
    };
    let mut status = fsw(rng, top, fcw);
    if waiting && status & 0x80 != 0 {
        // A waiting form would take #MF: no pending exception.
        status &= !(0x3f & !fcw);
        status = with_es(status, fcw);
    }
    b.fp.fcw = fcw;
    b.fp.fsw = status;

    // ST(i) operand index of the register forms.
    let sti = rng.below(8) as usize;
    let reads: Vec<usize> = match op {
        Op::LoadSt => vec![sti],
        Op::Store(_) | Op::StoreSt | Op::Unary | Op::ArithMem(_) | Op::ComMem(_) => vec![0],
        Op::Xch | Op::Arith | Op::Comi => vec![0, sti],
        Op::Scale => vec![0, 1],
        _ => vec![],
    };
    let push = matches!(op, Op::Load(_) | Op::LoadSt | Op::Ldz);

    // Occupancy of ST(0)-ST(7).
    let mut full: [bool; 8] = std::array::from_fn(|_| rng.chance(1, 2));
    match scen {
        Scen::Full => full = [true; 8],
        Scen::Empty => full = [false; 8],
        Scen::Normal | Scen::Fault => {
            for &i in &reads {
                full[i] = true;
            }
            if push {
                full[7] = false;
            }
            if scen == Scen::Fault {
                if push {
                    full[7] = true;
                } else if !reads.is_empty() {
                    full[reads[rng.below(reads.len() as u64) as usize]] = false;
                }
            }
        }
    }

    // Contents: operands from the edges first, the rest random.
    let mut st: [u128; 8] = std::array::from_fn(|_| rand80(rng));
    let e80 = edges80();
    let dom: Vec<u128> = match (op, m) {
        (Op::Store(Mf::I16), _) => int_limits80(16),
        (Op::Store(Mf::I32), _) => int_limits80(32),
        (Op::Store(Mf::I64), _) | (_, Mnemonic::Frndint) => int_limits80(64),
        (Op::Store(Mf::F32), _) => narrow_limits80(Fmt::S),
        (Op::Store(Mf::F64), _) => narrow_limits80(Fmt::D),
        _ => e80.clone(),
    };
    let a = if idx < dom.len() {
        dom[idx]
    } else if rng.chance(1, 4) {
        rng.pick(&dom)
    } else {
        rand80(rng)
    };
    let second = if idx < e80.len() {
        e80[(idx * 7 + 3) % e80.len()]
    } else if rng.chance(2, 5) {
        related80(rng, a)
    } else {
        rand80(rng)
    };
    match op {
        Op::LoadSt => st[sti] = a,
        Op::Scale => {
            st[0] = a;
            // Scale factors: small integers and fractions, huge, specials.
            st[1] = match rng.below(6) {
                0 => from_f64(rng.range_i64(-70, 70) as f64),
                1 => from_f64(rng.range_i64(-400, 400) as f64 / 8.0),
                2 => from_f64(rng.pick(&[1e5, -1e5, 16384.0, -16446.0, 0.5, -0.5])),
                _ => second,
            };
        }
        _ if !reads.is_empty() => {
            st[0] = a;
            if reads.len() > 1 && sti != 0 {
                st[sti] = second;
            }
        }
        _ => {}
    }
    b.fp.st = st;
    let abridged = (0..8)
        .filter(|&i| full[i])
        .fold(0u8, |acc, i| acc | 1 << ((usize::from(top) + i) % 8));
    b.fp.ftw = full_tag_word(abridged, &b.fp);

    // Memory operand.
    let size = mem_size(op);
    let mut ea = None;
    let mut undefined = Undefined::NONE;
    let mut mem_op = None;
    // fadd m32, fmul m64: the memory operand as a NaN (sign, significand).
    let mut mem_operand: Option<Option<(bool, u64)>> = None;
    if size > 0 {
        let lo = DATA_ADDR + OPERAND_LO + 32;
        let hi = DATA_ADDR + OPERAND_HI - size - 32;
        let mut at = lo + rng.below(hi - lo);
        if rng.chance(3, 4) {
            at &= !(size.next_power_of_two().min(16) - 1);
        }
        let mode = ALL_MODES[idx % ALL_MODES.len()];
        mem_op = Some(solve_addr(rng, &mut b, mode, at, true));
        b.define(rng, at - 8, size + 16)?;
        let bytes: Vec<u8> = match op {
            Op::Load(mf) | Op::ArithMem(mf) | Op::ComMem(mf) => {
                let mut v = mem_value(rng, mf, idx);
                // Compares: equal operands in some cases.
                if matches!(op, Op::ComMem(_))
                    && idx >= 32
                    && rng.chance(1, 3)
                    && let Some(r) = as_reg_value(mf, &v)
                {
                    b.fp.st[0] = r;
                    b.fp.ftw = full_tag_word(abridged, &b.fp);
                }
                if mf == Mf::F80 && idx >= edges80().len() && rng.chance(1, 4) {
                    v = (a.to_le_bytes()[..10]).to_vec();
                }
                if let Op::ArithMem(_) = op {
                    mem_operand = Some(nan_of_mem(mf, &v));
                }
                v
            }
            Op::Fldcw => {
                let (word, reserved) = loaded_fcw(rng, idx);
                if reserved {
                    undefined.fp.fcw = 0xf040;
                }
                // Unmasking a pending flag: ES and B follow the CPU model.
                if b.fp.fsw & !word & 0x3f != 0 {
                    undefined.fp.fsw |= 0x8080;
                }
                word.to_le_bytes().to_vec()
            }
            Op::Fldenv => {
                let (word, reserved) = loaded_fcw(rng, idx);
                if reserved {
                    undefined.fp.fcw = 0xf040;
                }
                let new_top = rng.below(8) as u16;
                let sw = fsw(rng, new_top, word);
                // Any tags: the CPU keeps only empty / not empty.
                let tw = rng.next_u64() as u16;
                let mut env = Vec::new();
                for (k, v) in [word, sw, tw].into_iter().enumerate() {
                    env.extend(v.to_le_bytes());
                    env.extend(if k == 0 && rng.chance(1, 2) {
                        [0xff, 0xff]
                    } else {
                        (rng.next_u64() as u16).to_le_bytes()
                    });
                }
                for _ in 0..4 {
                    env.extend((rng.next_u64() as u32).to_le_bytes());
                }
                env
            }
            // Destinations: random bytes, already defined.
            _ => {
                let mut v = vec![0u8; size as usize];
                for x in &mut v {
                    *x = rng.next_u64() as u8;
                }
                v
            }
        };
        b.put(at, &bytes)?;
        if op == Op::Fnstenv {
            // Reserved upper halves of the first three dwords, then the
            // instruction and data pointers, opcode and selectors.
            let ff = |n| vec![0xffu8; n];
            undefined.mem = vec![
                MemBlock {
                    addr: at + 2,
                    bytes: ff(2),
                },
                MemBlock {
                    addr: at + 6,
                    bytes: ff(2),
                },
                MemBlock {
                    addr: at + 10,
                    bytes: ff(18),
                },
            ];
        }
        ea = Some(at);
    }

    // The instruction.
    let instr = match op {
        Op::LoadSt | Op::StoreSt => Instruction::with1(code, st_reg(sti))?,
        Op::Xch | Op::Comi => Instruction::with2(code, Register::ST0, st_reg(sti))?,
        Op::Arith => {
            if format!("{code:?}").ends_with("_st0_sti") {
                Instruction::with2(code, Register::ST0, st_reg(sti))?
            } else {
                Instruction::with2(code, st_reg(sti), Register::ST0)?
            }
        }
        Op::FnstswAx => Instruction::with1(code, Register::AX)?,
        _ => match mem_op {
            Some(mo) => Instruction::with1(code, mo)?,
            None => Instruction::with(code),
        },
    };

    // Two NaNs of the same kind with equal significands and opposite
    // signs: the SDM's rule (the larger significand wins) leaves the sign
    // of the result to the CPU model, so it is undefined.
    let st = &b.fp.st;
    let pop = format!("{code:?}").contains("p_");
    let (x, y, dst) = match op {
        Op::ArithMem(_) => (nan_of(st[0]), mem_operand.flatten(), Some(0)),
        Op::Scale => (nan_of(st[0]), nan_of(st[1]), Some(0)),
        Op::Arith if format!("{code:?}").ends_with("_st0_sti") => {
            (nan_of(st[0]), nan_of(st[sti]), Some(0))
        }
        Op::Arith => (
            nan_of(st[0]),
            nan_of(st[sti]),
            Some(if pop { (sti + 7) % 8 } else { sti }),
        ),
        _ => (None, None, None),
    };
    if let (Some((sx, gx)), Some((sy, gy)), Some(d)) = (x, y, dst)
        && sx != sy
        && gx == gy
    {
        undefined.fp.st[d] |= SIGN;
    }

    // Condition codes the SDM leaves undefined ("FPU Flags Affected").
    undefined.fp.fsw |= match op {
        Op::ComMem(_) | Op::Comi | Op::Fldenv => 0,
        o if is_control(o) => 0x4700,
        _ => 0x4500,
    };
    let Some(mut case) = finish(Finish {
        form,
        b,
        instr,
        target: None,
        verify_ea: ea,
    })?
    else {
        return Ok(None);
    };
    ensure!(
        crate::census::form_name(
            &Decoder::with_ip(64, &case.bytes, CODE_ADDR, DecoderOptions::NONE).decode()
        ) == form.id,
        "{}: encodes as another form",
        form.id
    );
    case.undefined = undefined;
    Ok(Some(case))
}

/// (sign, significand) of an 80-bit NaN (J bit set, fraction not 0).
fn nan_of(v: u128) -> Option<(bool, u64)> {
    let sig = v as u64;
    ((v >> 64) & 0x7fff == 0x7fff && sig & J != 0 && sig << 1 != 0).then_some((v & SIGN != 0, sig))
}

/// (sign, significand as an 80-bit register would hold it) of an m32 or
/// m64 NaN, converted bit-exactly (no quieting).
fn nan_of_mem(mf: Mf, bytes: &[u8]) -> Option<(bool, u64)> {
    let mut b = [0u8; 8];
    b[..bytes.len()].copy_from_slice(bytes);
    let raw = u64::from_le_bytes(b);
    let (neg, exp, frac, emax, shift) = match mf {
        Mf::F32 => (
            raw >> 31 & 1 == 1,
            raw >> 23 & 0xff,
            raw & 0x7f_ffff,
            0xff,
            40,
        ),
        Mf::F64 => (
            raw >> 63 == 1,
            raw >> 52 & 0x7ff,
            raw & ((1 << 52) - 1),
            0x7ff,
            11,
        ),
        _ => return None,
    };
    (exp == emax && frac != 0).then_some((neg, J | frac << shift))
}

/// A control word for fldcw and fldenv: any masks, PC and RC; in every
/// fourth case also the reserved bits 6 and 12-15 at random, whose value
/// after the load is left undefined (CPU models differ).
fn loaded_fcw(rng: &mut Rng, idx: usize) -> (u16, bool) {
    let w = any_fcw(rng);
    if idx % 4 == 3 {
        (w & !0xf040 | rng.next_u64() as u16 & 0xf040, true)
    } else {
        (w, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(from_f64(1.0), 0x3fff_8000_0000_0000_0000);
        assert_eq!(from_f64(-2.0), 0xc000_8000_0000_0000_0000);
        assert_eq!(from_f64(0.0), 0);
        assert_eq!(from_f64(f64::from_bits(1)), f80e(false, -1074, J));
        assert_eq!(half_int(false, 2), from_f64(1.0));
        assert_eq!(half_int(true, 3), from_f64(-1.5));
        assert_eq!(half_int(false, 1 << 64), from_f64(2f64.powi(63)));
        assert_eq!(narrow_limits80(Fmt::S)[0], from_f64(f32::MAX as f64));
        assert_eq!(narrow_limits80(Fmt::D)[0], from_f64(f64::MAX));
        assert_eq!(narrow_limits80(Fmt::D)[6], from_f64(f64::from_bits(1)));
        // QNaN default (binary32) and the indefinite: a sign tie.
        let m = nan_of_mem(Mf::F32, &0x7fc0_0000u32.to_le_bytes()).unwrap();
        let r = nan_of(f80(true, 0x7fff, J | 1 << 62)).unwrap();
        assert_eq!((m.1, !m.0), (r.1, r.0));
        assert_eq!(nan_of(f80(false, 0x7fff, J)), None);
        assert_eq!(nan_of(f80(false, 0x7fff, 1 << 62)), None);
    }
}
