//! SSE/SSE2 scalar floating point (`abi.md`, "Floating-point helper
//! module", SSE). Every function returns (result, MXCSR after).

use crate::exact::*;
use core::cmp::Ordering;

/// MXCSR bit 31 in a status: a raised exception is unmasked.
pub const FP_UNMASKED: u32 = 0x8000_0000;
const DAZ: u32 = 0x40;
const FTZ: u32 = 0x8000;
const UM: u32 = 0x800;

/// An IEEE binary interchange format.
pub(crate) struct Ieee {
    frac: u32,
    ebits: u32,
    fmt: Fmt,
}

pub(crate) const S: Ieee = Ieee { frac: 23, ebits: 8, fmt: F32 };
pub(crate) const D: Ieee = Ieee { frac: 52, ebits: 11, fmt: F64 };

impl Ieee {
    const fn bias(&self) -> i32 {
        (1 << (self.ebits - 1)) - 1
    }
    const fn emask(&self) -> u64 {
        (1 << self.ebits) - 1
    }
    const fn sign_bit(&self) -> u64 {
        1 << (self.frac + self.ebits)
    }
    pub(crate) fn unpack(&self, bits: u64, daz: bool) -> Val {
        let sign = bits & self.sign_bit() != 0;
        let e = (bits >> self.frac) & self.emask();
        let f = bits & ((1 << self.frac) - 1);
        if e == self.emask() {
            if f == 0 {
                return Val { sign, class: Class::Inf, exp: 0, sig: 0, denormal: false };
            }
            let class = if f >> (self.frac - 1) != 0 { Class::QNan } else { Class::SNan };
            return Val { sign, class, exp: 0, sig: J | (f << (63 - self.frac)), denormal: false };
        }
        if e == 0 {
            if f == 0 || daz {
                return Val::zero(sign);
            }
            let exp = 1 - self.bias() - self.frac as i32;
            return Val { sign, class: Class::Finite, exp, sig: f, denormal: true };
        }
        let exp = e as i32 - self.bias() - self.frac as i32;
        Val { sign, class: Class::Finite, exp, sig: f | (1 << self.frac), denormal: false }
    }
    fn signed(&self, sign: bool, mag: u64) -> u64 {
        if sign { mag | self.sign_bit() } else { mag }
    }
    pub(crate) fn inf(&self, sign: bool) -> u64 {
        self.signed(sign, self.emask() << self.frac)
    }
    pub(crate) fn zero(&self, sign: bool) -> u64 {
        self.signed(sign, 0)
    }
    /// A NaN from a significand in the 80-bit layout (payload truncated).
    pub(crate) fn nan(&self, sign: bool, sig: u64) -> u64 {
        let f = (sig >> (63 - self.frac)) & ((1 << self.frac) - 1);
        self.signed(sign, (self.emask() << self.frac) | f)
    }
    /// The default NaN (QNaN floating-point indefinite).
    pub(crate) fn default_nan(&self) -> u64 {
        self.nan(true, J | QUIET)
    }
    pub(crate) fn pack(&self, r: &Rounded) -> u64 {
        if r.inf {
            return self.inf(r.sign);
        }
        if r.sig == 0 {
            return self.zero(r.sign);
        }
        let m = 63 - r.sig.leading_zeros() as i32;
        let e = r.exp + m;
        let mag = if e >= self.fmt.emin {
            let f = (r.sig << (self.frac as i32 - m)) & ((1 << self.frac) - 1);
            (((e + self.bias()) as u64) << self.frac) | f
        } else {
            r.sig << (r.exp - self.fmt.lsb_min)
        };
        self.signed(r.sign, mag)
    }
}

fn unmasked(fl: u32, mxcsr: u32) -> bool {
    fl & !(mxcsr >> 7) & 0x3f != 0
}

fn status(mxcsr: u32, fl: u32) -> u32 {
    let s = (mxcsr & !FP_UNMASKED) | fl;
    if unmasked(fl, mxcsr) { s | FP_UNMASKED } else { s }
}

/// Rounds a `Pre` to `t` under `mxcsr` (RC, FTZ, UM): (bits, flags).
fn finish(t: &Ieee, pre: Pre, fl: u32, mxcsr: u32) -> (u64, u32) {
    match pre {
        // The CPU reports no DE when the operation is invalid or divides by 0.
        Pre::Invalid => (t.default_nan(), (fl & !DE) | IE),
        Pre::DivZero(s) => (t.inf(s), (fl & !DE) | ZE),
        Pre::Zero(s) => (t.zero(s), fl),
        Pre::Inf(s) => (t.inf(s), fl),
        Pre::Num(s, e, m) => {
            if unmasked(fl, mxcsr) {
                return (0, fl);
            }
            let ue_unmasked = mxcsr & UM == 0;
            let r = round(s, e, m, &t.fmt, (mxcsr >> 13) & 3, ue_unmasked);
            if r.tiny && mxcsr & FTZ != 0 && !ue_unmasked {
                (t.zero(s), fl | UE | PE)
            } else {
                (t.pack(&r), fl | r.flags)
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

pub(crate) fn arith(t: &Ieee, a: u64, b: u64, mxcsr: u32, op: Op) -> (u64, u32) {
    let daz = mxcsr & DAZ != 0;
    let (va, vb) = (t.unpack(a, daz), t.unpack(b, daz));
    if va.is_nan() || vb.is_nan() {
        let fl = if va.class == Class::SNan || vb.class == Class::SNan { IE } else { 0 };
        let n = if va.is_nan() { va } else { vb };
        return (t.nan(n.sign, n.sig | QUIET), status(mxcsr, fl));
    }
    let fl = if va.denormal || vb.denormal { DE } else { 0 };
    let rc = (mxcsr >> 13) & 3;
    let pre = match op {
        Op::Add => add(va.sign, &va, vb.sign, &vb, rc),
        Op::Sub => add(va.sign, &va, !vb.sign, &vb, rc),
        Op::Mul => mul(&va, &vb),
        Op::Div => div(&va, &vb),
    };
    let (r, fl) = finish(t, pre, fl, mxcsr);
    (r, status(mxcsr, fl))
}

pub(crate) fn sqrt_(t: &Ieee, a: u64, mxcsr: u32) -> (u64, u32) {
    let va = t.unpack(a, mxcsr & DAZ != 0);
    if va.is_nan() {
        let fl = if va.class == Class::SNan { IE } else { 0 };
        return (t.nan(va.sign, va.sig | QUIET), status(mxcsr, fl));
    }
    let fl = if va.denormal { DE } else { 0 };
    let (r, fl) = finish(t, sqrt(&va), fl, mxcsr);
    (r, status(mxcsr, fl))
}

pub(crate) fn minmax(t: &Ieee, a: u64, b: u64, mxcsr: u32, max: bool) -> (u64, u32) {
    let daz = mxcsr & DAZ != 0;
    let (va, vb) = (t.unpack(a, daz), t.unpack(b, daz));
    // The operand returned is the one DAZ read: a denormal becomes zero.
    let pick = |v: &Val, bits: u64| if v.class == Class::Zero { t.zero(v.sign) } else { bits };
    if va.is_nan() || vb.is_nan() {
        return (pick(&vb, b), status(mxcsr, IE));
    }
    let fl = if va.denormal || vb.denormal { DE } else { 0 };
    let want = if max { Ordering::Greater } else { Ordering::Less };
    let r = if cmp(&va, &vb) == want { pick(&va, a) } else { pick(&vb, b) };
    (r, status(mxcsr, fl))
}

/// Order of a and b with the compare flags: None = unordered.
fn compare(t: &Ieee, a: u64, b: u64, mxcsr: u32, signaling: bool) -> (Option<Ordering>, u32) {
    let daz = mxcsr & DAZ != 0;
    let (va, vb) = (t.unpack(a, daz), t.unpack(b, daz));
    if va.is_nan() || vb.is_nan() {
        let snan = va.class == Class::SNan || vb.class == Class::SNan;
        return (None, if snan || signaling { IE } else { 0 });
    }
    let fl = if va.denormal || vb.denormal { DE } else { 0 };
    (Some(cmp(&va, &vb)), fl)
}

pub(crate) fn cmp_pred(t: &Ieee, a: u64, b: u64, pred: u32, mxcsr: u32) -> (u32, u32) {
    let pred = pred & 7;
    let (o, fl) = compare(t, a, b, mxcsr, matches!(pred, 1 | 2 | 5 | 6));
    let lt = o == Some(Ordering::Less);
    let eq = o == Some(Ordering::Equal);
    let r = match pred {
        0 => eq,
        1 => lt,
        2 => lt || eq,
        3 => o.is_none(),
        4 => !eq,
        5 => !lt,
        6 => !(lt || eq),
        _ => o.is_some(),
    };
    (r as u32, status(mxcsr, fl))
}

pub(crate) fn comi(t: &Ieee, a: u64, b: u64, signaling: bool, mxcsr: u32) -> (u32, u32) {
    let (o, fl) = compare(t, a, b, mxcsr, signaling);
    let r = match o {
        None => 0x45,
        Some(Ordering::Less) => 0x01,
        Some(Ordering::Equal) => 0x40,
        Some(Ordering::Greater) => 0,
    };
    (r, status(mxcsr, fl))
}

/// Float to float (cvtss2sd, cvtsd2ss).
pub(crate) fn convert(from: &Ieee, to: &Ieee, a: u64, mxcsr: u32) -> (u64, u32) {
    let va = from.unpack(a, mxcsr & DAZ != 0);
    let (r, fl) = match va.class {
        Class::QNan | Class::SNan => {
            let fl = if va.class == Class::SNan { IE } else { 0 };
            (to.nan(va.sign, va.sig | QUIET), fl)
        }
        Class::Zero => (to.zero(va.sign), 0),
        Class::Inf => (to.inf(va.sign), 0),
        _ => {
            let fl = if va.denormal { DE } else { 0 };
            finish(to, Pre::Num(va.sign, va.exp, va.sig as u128), fl, mxcsr)
        }
    };
    (r, status(mxcsr, fl))
}

pub(crate) fn from_int(t: &Ieee, i: i64, mxcsr: u32) -> (u64, u32) {
    let pre = if i == 0 { Pre::Zero(false) } else { Pre::Num(i < 0, 0, i.unsigned_abs() as u128) };
    let (r, fl) = finish(t, pre, 0, mxcsr);
    (r, status(mxcsr, fl))
}

/// Float to a signed integer of `bits` bits (32 or 64): the value
/// sign-extended to i64.
pub(crate) fn to_int_(t: &Ieee, a: u64, mxcsr: u32, trunc: bool, bits: u32) -> (i64, u32) {
    let indef = -1i64 << (bits - 1);
    let va = t.unpack(a, mxcsr & DAZ != 0);
    let rc = if trunc { RC_ZERO } else { (mxcsr >> 13) & 3 };
    let (r, fl) = match va.class {
        Class::Zero => (0, 0),
        Class::Finite => match to_int(&va, rc) {
            Some((k, inexact, _)) if k <= (1u64 << (bits - 1)) - (!va.sign) as u64 => {
                let v = if va.sign { (k as i64).wrapping_neg() } else { k as i64 };
                (v, if inexact { PE } else { 0 })
            }
            _ => (indef, IE),
        },
        _ => (indef, IE),
    };
    (r, status(mxcsr, fl))
}
