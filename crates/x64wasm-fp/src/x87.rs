//! x87 arithmetic on 80-bit values (`abi.md`, "Floating-point helper
//! module", x87). Every function returns (result, status): exception flags
//! in bits 0-5, ES in bit 7, the condition codes it defines in bits 8, 9, 10
//! and 14, and `se` of an 80-bit result in bits 16-31.

use crate::exact::*;
use crate::sse::{D, Ieee, S};
use core::cmp::Ordering;

pub(crate) const C0: u32 = 0x100;
pub(crate) const C1: u32 = 0x200;
pub(crate) const C2: u32 = 0x400;
pub(crate) const C3: u32 = 0x4000;

/// The QNaN floating-point indefinite.
const INDEF: (u64, u32) = (0xC000_0000_0000_0000, 0xFFFF);
const UM: u32 = 0x10;

const fn sgn(sign: bool) -> u32 {
    if sign { 0x8000 } else { 0 }
}

pub(crate) fn unpack80(m: u64, se: u32) -> Val {
    let sign = se & 0x8000 != 0;
    let e = (se & 0x7FFF) as i32;
    let bad = Val {
        sign,
        class: Class::Bad,
        exp: 0,
        sig: m,
        denormal: false,
    };
    if e == 0x7FFF {
        return match (m & J != 0, m << 1 == 0, m & QUIET != 0) {
            (false, _, _) => bad,
            (true, true, _) => Val {
                sign,
                class: Class::Inf,
                exp: 0,
                sig: m,
                denormal: false,
            },
            (true, false, true) => Val {
                sign,
                class: Class::QNan,
                exp: 0,
                sig: m,
                denormal: false,
            },
            (true, false, false) => Val {
                sign,
                class: Class::SNan,
                exp: 0,
                sig: m,
                denormal: false,
            },
        };
    }
    if e == 0 {
        if m == 0 {
            return Val::zero(sign);
        }
        // Denormals and pseudo-denormals (J set): both have the value
        // m * 2^(1 - 16383 - 63).
        return Val {
            sign,
            class: Class::Finite,
            exp: -16445,
            sig: m,
            denormal: true,
        };
    }
    if m & J == 0 {
        return bad; // unnormal
    }
    Val {
        sign,
        class: Class::Finite,
        exp: e - 16383 - 63,
        sig: m,
        denormal: false,
    }
}

/// An operand of kind `kind` (`KIND_*`), unconverted.
pub(crate) fn unpack_kind(bm: u64, bse: u32, kind: u32) -> Val {
    match kind {
        0 => unpack80(bm, bse),
        1 => S.unpack(bm & 0xFFFF_FFFF, false),
        2 => D.unpack(bm, false),
        _ => Val::from_int(bm as i64),
    }
}

fn pack80(r: &Rounded) -> (u64, u32) {
    if r.inf {
        return (J, sgn(r.sign) | 0x7FFF);
    }
    if r.sig == 0 {
        return (0, sgn(r.sign));
    }
    let top = 63 - r.sig.leading_zeros() as i32;
    let e = r.exp + top;
    if e >= -16382 {
        (r.sig << (63 - top), sgn(r.sign) | (e + 16383) as u32)
    } else {
        (r.sig << (r.exp + 16445), sgn(r.sign))
    }
}

fn unmasked(fl: u32, fcw: u32) -> bool {
    fl & !fcw & 0x3F != 0
}

fn status(fl: u32, fcw: u32, cc: u32, se: u32) -> u32 {
    let es = if unmasked(fl, fcw) { 0x80 } else { 0 };
    fl | es | cc | (se << 16)
}

fn rc(fcw: u32) -> u32 {
    (fcw >> 10) & 3
}

fn pc_fmt(fcw: u32) -> Fmt {
    f80(match (fcw >> 8) & 3 {
        0 => 24,
        2 => 53,
        _ => 64,
    })
}

fn quiet(v: &Val) -> (u64, u32) {
    (v.sig | QUIET, sgn(v.sign) | 0x7FFF)
}

/// The NaN result of two operands, at least one a NaN (SDM Vol. 1, table
/// 4-7, x87): a QNaN over an SNaN, else the larger significand.
fn nan2<'a>(a: &'a Val, b: &'a Val) -> &'a Val {
    match (a.is_nan(), b.is_nan()) {
        (true, false) => a,
        (false, true) => b,
        _ => match (a.class, b.class) {
            (Class::QNan, Class::SNan) => a,
            (Class::SNan, Class::QNan) => b,
            _ => match (a.sig << 1).cmp(&(b.sig << 1)) {
                Ordering::Less => b,
                Ordering::Greater => a,
                Ordering::Equal => {
                    if a.sign {
                        b
                    } else {
                        a
                    }
                }
            },
        },
    }
}

/// An 80-bit result: rounds `pre` to `fmt` and adds `cc`.
fn finish(pre: Pre, fl: u32, fcw: u32, fmt: &Fmt, cc: u32) -> (u64, u32) {
    let (m, se, fl, cc) = match pre {
        Pre::Invalid => (INDEF.0, INDEF.1, (fl & !DE) | IE, cc),
        Pre::DivZero(s) => (J, sgn(s) | 0x7FFF, (fl & !DE) | ZE, cc),
        Pre::Zero(s) => (0, sgn(s), fl, cc),
        Pre::Inf(s) => (J, sgn(s) | 0x7FFF, fl, cc),
        Pre::Num(s, e, m) => {
            if unmasked(fl, fcw) {
                return (0, status(fl, fcw, 0, 0));
            }
            let r = round(s, e, m, fmt, rc(fcw), fcw & UM == 0);
            let (m, se) = pack80(&r);
            (m, se, fl | r.flags, cc | if r.up { C1 } else { 0 })
        }
    };
    (m, status(fl, fcw, cc, se))
}

/// Checks shared by the two-operand functions: an unsupported operand or a
/// NaN decides the result alone.
fn special2(a: &Val, b: &Val, fcw: u32) -> Option<(u64, u32)> {
    if a.class == Class::Bad || b.class == Class::Bad {
        return Some(finish(Pre::Invalid, 0, fcw, &f80(64), 0));
    }
    if a.is_nan() || b.is_nan() {
        let fl = if a.class == Class::SNan || b.class == Class::SNan {
            IE
        } else {
            0
        };
        let (m, se) = quiet(nan2(a, b));
        return Some((m, status(fl, fcw, 0, se)));
    }
    None
}

#[derive(Clone, Copy)]
pub(crate) enum Op {
    Add,
    Sub,
    SubR,
    Mul,
    Div,
    DivR,
}

pub(crate) fn arith(
    am: u64,
    ase: u32,
    bm: u64,
    bse: u32,
    bkind: u32,
    fcw: u32,
    op: Op,
) -> (u64, u32) {
    let a = unpack80(am, ase);
    let b = unpack_kind(bm, bse, bkind);
    if let Some(r) = special2(&a, &b, fcw) {
        return r;
    }
    let fl = if a.denormal || b.denormal { DE } else { 0 };
    let rc = rc(fcw);
    let pre = match op {
        Op::Add => add(a.sign, &a, b.sign, &b, rc),
        Op::Sub => add(a.sign, &a, !b.sign, &b, rc),
        Op::SubR => add(b.sign, &b, !a.sign, &a, rc),
        Op::Mul => mul(&a, &b),
        Op::Div => div(&a, &b),
        Op::DivR => div(&b, &a),
    };
    finish(pre, fl, fcw, &pc_fmt(fcw), 0)
}

/// One-operand checks: an unsupported operand or a NaN.
fn special1(a: &Val, fcw: u32) -> Option<(u64, u32)> {
    if a.class == Class::Bad {
        return Some(finish(Pre::Invalid, 0, fcw, &f80(64), 0));
    }
    if a.is_nan() {
        let fl = if a.class == Class::SNan { IE } else { 0 };
        let (m, se) = quiet(a);
        return Some((m, status(fl, fcw, 0, se)));
    }
    None
}

pub(crate) fn sqrt_(am: u64, ase: u32, fcw: u32) -> (u64, u32) {
    let a = unpack80(am, ase);
    if let Some(r) = special1(&a, fcw) {
        return r;
    }
    let fl = if a.denormal { DE } else { 0 };
    finish(sqrt(&a), fl, fcw, &pc_fmt(fcw), 0)
}

/// frndint: rounds to an integer by RC (PC doesn't apply).
pub(crate) fn round_int(am: u64, ase: u32, fcw: u32) -> (u64, u32) {
    let a = unpack80(am, ase);
    if let Some(r) = special1(&a, fcw) {
        return r;
    }
    match a.class {
        Class::Zero | Class::Inf => (am, status(0, fcw, 0, ase)),
        _ => {
            let fl = if a.denormal { DE } else { 0 };
            if unmasked(fl, fcw) || a.exp >= 0 {
                return (am, status(fl, fcw, 0, ase));
            }
            // |a| < 2^63 here, so the integer fits.
            let (k, inexact, up) = to_int(&a, rc(fcw)).unwrap_or((0, false, false));
            let pre = if k == 0 {
                Pre::Zero(a.sign)
            } else {
                Pre::Num(a.sign, 0, k as u128)
            };
            let fl = fl | if inexact { PE } else { 0 };
            finish(pre, fl, fcw, &f80(64), if up { C1 } else { 0 })
        }
    }
}

/// trunc(b) for fscale, clamped far beyond any finite result's range.
fn scale_count(b: &Val) -> i32 {
    const LIMIT: u64 = 1 << 20;
    if b.class == Class::Zero {
        return 0;
    }
    let mag = if b.exp >= 0 {
        LIMIT
    } else if b.exp <= -64 {
        0
    } else {
        (b.sig >> -b.exp).min(LIMIT)
    };
    if b.sign { -(mag as i32) } else { mag as i32 }
}

pub(crate) fn scale(am: u64, ase: u32, bm: u64, bse: u32, fcw: u32) -> (u64, u32) {
    let a = unpack80(am, ase);
    let b = unpack80(bm, bse);
    if let Some(r) = special2(&a, &b, fcw) {
        return r;
    }
    let fl = if a.denormal || b.denormal { DE } else { 0 };
    let pre = match (a.class, b.class) {
        (Class::Zero, Class::Inf) if !b.sign => Pre::Invalid,
        (Class::Inf, Class::Inf) if b.sign => Pre::Invalid,
        (Class::Zero, _) => Pre::Zero(a.sign),
        (Class::Inf, _) => Pre::Inf(a.sign),
        (_, Class::Inf) => {
            if b.sign {
                Pre::Zero(a.sign)
            } else {
                Pre::Inf(a.sign)
            }
        }
        _ => Pre::Num(a.sign, a.exp + scale_count(&b), a.sig as u128),
    };
    finish(pre, fl, fcw, &f80(64), 0)
}

/// The exponent difference beyond which fprem and fprem1 reduce partially,
/// and the reduction they make then (`N` in the SDM's pseudocode).
fn partial_n(d: i32) -> u32 {
    (32 + (d - 32) % 32) as u32
}

/// fprem (`nearest` false) and fprem1 (true): a = ST(0), b = ST(1).
pub(crate) fn prem(am: u64, ase: u32, bm: u64, bse: u32, fcw: u32, nearest: bool) -> (u64, u32) {
    let a = unpack80(am, ase);
    let b = unpack80(bm, bse);
    if let Some(r) = special2(&a, &b, fcw) {
        return r;
    }
    let fl = if a.denormal || b.denormal { DE } else { 0 };
    let (pre, cc) = match (a.class, b.class) {
        (Class::Inf, _) | (_, Class::Zero) => (Pre::Invalid, 0),
        (Class::Zero, _) => (Pre::Zero(a.sign), 0),
        (_, Class::Inf) => (Pre::Num(a.sign, a.exp, a.sig as u128), 0),
        _ => {
            let (ea, ma) = norm(&a);
            let (eb, mb) = norm(&b);
            let d = ea - eb;
            let mut sign = a.sign;
            let (r, base, cc) = if d >= 64 {
                let n = partial_n(d);
                let r = ((ma as u128) << n) % mb as u128;
                (r, eb + d - n as i32, C2)
            } else if d < -1 || (d == -1 && !nearest) {
                (ma as u128, ea, 0)
            } else {
                let (x, y, base) = if d >= 0 {
                    ((ma as u128) << d, mb as u128, eb)
                } else {
                    (ma as u128, (mb as u128) << 1, ea)
                };
                let (mut q, mut r) = (x / y, x % y);
                if nearest && (2 * r > y || (2 * r == y && q & 1 != 0)) {
                    q += 1;
                    r = y - r;
                    sign = !sign;
                }
                let cc = (if q & 4 != 0 { C0 } else { 0 })
                    | (if q & 2 != 0 { C3 } else { 0 })
                    | (if q & 1 != 0 { C1 } else { 0 });
                (r, base, cc)
            };
            (
                if r == 0 {
                    Pre::Zero(sign)
                } else {
                    Pre::Num(sign, base, r)
                },
                cc,
            )
        }
    };
    finish(pre, fl, fcw, &f80(64), cc)
}

/// fld m32/m64 and fild: exact, IE for an SNaN, DE for a denormal.
pub(crate) fn load(bm: u64, kind: u32, fcw: u32) -> (u64, u32) {
    let v = unpack_kind(bm, 0, kind);
    if kind == 0 {
        return (bm, 0);
    }
    if let Some(r) = special1(&v, fcw) {
        return r;
    }
    let pre = match v.class {
        Class::Zero => Pre::Zero(v.sign),
        Class::Inf => Pre::Inf(v.sign),
        _ => Pre::Num(v.sign, v.exp, v.sig as u128),
    };
    finish(pre, if v.denormal { DE } else { 0 }, fcw, &f80(64), 0)
}

fn store_float(t: &Ieee, a: &Val, fcw: u32) -> (u64, u32, u32) {
    match a.class {
        Class::Bad => (t.default_nan(), IE, 0),
        Class::QNan => (t.nan(a.sign, a.sig), 0, 0),
        Class::SNan => (t.nan(a.sign, a.sig | QUIET), IE, 0),
        Class::Zero => (t.zero(a.sign), 0, 0),
        Class::Inf => (t.inf(a.sign), 0, 0),
        Class::Finite => {
            // No DE for a denormal source (the CPU decides).
            let r = round(
                a.sign,
                a.exp,
                a.sig as u128,
                t.fmt(),
                rc(fcw),
                fcw & UM == 0,
            );
            (t.pack(&r), r.flags, if r.up { C1 } else { 0 })
        }
    }
}

/// fst/fstp m32/m64 (kinds 1, 2) and fist/fistp (kinds 3-5).
pub(crate) fn store(am: u64, ase: u32, kind: u32, fcw: u32) -> (u64, u32) {
    let a = unpack80(am, ase);
    let (r, fl, cc) = match kind {
        1 => store_float(&S, &a, fcw),
        2 => store_float(&D, &a, fcw),
        _ => {
            let bits = match kind {
                3 => 16,
                4 => 32,
                _ => 64,
            };
            let indef = (-1i64 << (bits - 1)) as u64;
            match a.class {
                Class::Zero => (0, 0, 0),
                Class::Finite => {
                    // fist raises no DE for a denormal (the CPU decides).
                    let fl = 0;
                    match to_int(&a, rc(fcw)) {
                        Some((k, inexact, up)) if k <= (1u64 << (bits - 1)) - (!a.sign) as u64 => {
                            let v = if a.sign {
                                (k as i64).wrapping_neg()
                            } else {
                                k as i64
                            };
                            (
                                v as u64,
                                fl | if inexact { PE } else { 0 },
                                if up { C1 } else { 0 },
                            )
                        }
                        _ => (indef, fl | IE, 0),
                    }
                }
                _ => (indef, IE, 0),
            }
        }
    };
    (r, status(fl, fcw, cc, 0))
}

/// fcom, fucom (`quiet`), ficom, ftst, fcomi, fucomi: C3, C2, C0.
pub(crate) fn compare(
    am: u64,
    ase: u32,
    bm: u64,
    bse: u32,
    bkind: u32,
    quiet: bool,
    fcw: u32,
) -> (u32, u32) {
    let a = unpack80(am, ase);
    let b = unpack_kind(bm, bse, bkind);
    let unordered = C3 | C2 | C0;
    let (cc, fl) = if a.class == Class::Bad || b.class == Class::Bad {
        (unordered, IE)
    } else if a.is_nan() || b.is_nan() {
        let snan = a.class == Class::SNan || b.class == Class::SNan;
        (unordered, if snan || !quiet { IE } else { 0 })
    } else {
        let fl = if a.denormal || b.denormal { DE } else { 0 };
        let cc = match cmp(&a, &b) {
            Ordering::Less => C0,
            Ordering::Equal => C3,
            Ordering::Greater => 0,
        };
        (cc, fl)
    };
    (cc, status(fl, fcw, cc, 0))
}
