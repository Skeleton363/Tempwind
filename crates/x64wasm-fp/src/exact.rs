//! Format-independent exact arithmetic: unpacked operands, exact results and
//! rounding to a precision and exponent range. Integer arithmetic only.

use core::cmp::Ordering;

/// Exception flags, in the same bits in MXCSR and the x87 FSW.
pub const IE: u32 = 0x01;
pub const DE: u32 = 0x02;
pub const ZE: u32 = 0x04;
pub const OE: u32 = 0x08;
pub const UE: u32 = 0x10;
pub const PE: u32 = 0x20;

/// Rounding control (MXCSR.RC and FCW.RC use the same encoding).
pub const RC_NEAR: u32 = 0;
pub const RC_DOWN: u32 = 1;
pub const RC_UP: u32 = 2;
pub const RC_ZERO: u32 = 3;

/// The quiet bit of a NaN significand in the 80-bit layout.
pub const QUIET: u64 = 1 << 62;
pub const J: u64 = 1 << 63;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Zero,
    Finite,
    Inf,
    QNan,
    SNan,
    /// An x87 encoding the FPU doesn't support (unnormal, pseudo-NaN,
    /// pseudo-infinity): an invalid operand.
    Bad,
}

/// An unpacked operand. Finite: value = sig * 2^exp (sig nonzero, not
/// necessarily normalized). NaN: sig is the significand in the 80-bit
/// layout (J bit set, quiet bit 62, payload below).
#[derive(Clone, Copy, Debug)]
pub struct Val {
    pub sign: bool,
    pub class: Class,
    pub exp: i32,
    pub sig: u64,
    /// The operand was a denormal (or an x87 pseudo-denormal): DE.
    pub denormal: bool,
}

impl Val {
    pub const fn zero(sign: bool) -> Val {
        Val { sign, class: Class::Zero, exp: 0, sig: 0, denormal: false }
    }
    pub const fn is_nan(&self) -> bool {
        matches!(self.class, Class::QNan | Class::SNan)
    }
    pub fn from_int(i: i64) -> Val {
        if i == 0 {
            Val::zero(false)
        } else {
            Val { sign: i < 0, class: Class::Finite, exp: 0, sig: i.unsigned_abs(), denormal: false }
        }
    }
}

/// An exact (or sticky) result before rounding.
#[derive(Clone, Copy, Debug)]
pub enum Pre {
    Zero(bool),
    Inf(bool),
    /// (-1)^sign * sig * 2^exp, sig nonzero; bit 0 may be a sticky bit
    /// (callers keep at least two bits below any rounding point).
    Num(bool, i32, u128),
    /// An invalid operation: IE and the default NaN.
    Invalid,
    /// Finite nonzero / 0: ZE and an infinity of this sign.
    DivZero(bool),
}

/// A target precision and exponent range.
#[derive(Clone, Copy, Debug)]
pub struct Fmt {
    /// Significand bits, the leading bit included.
    pub p: u32,
    /// Exponent of the smallest normal number.
    pub emin: i32,
    /// Exponent of the largest finite number.
    pub emax: i32,
    /// Exponent of the least significant bit of a denormal.
    pub lsb_min: i32,
}

pub const F32: Fmt = Fmt { p: 24, emin: -126, emax: 127, lsb_min: -149 };
pub const F64: Fmt = Fmt { p: 53, emin: -1022, emax: 1023, lsb_min: -1074 };
pub const fn f80(p: u32) -> Fmt {
    Fmt { p, emin: -16382, emax: 16383, lsb_min: -16445 }
}

/// A rounded result: value = sig * 2^exp (sig < 2^p; 0 means zero), or an
/// infinity.
#[derive(Clone, Copy, Debug)]
pub struct Rounded {
    pub sign: bool,
    pub sig: u64,
    pub exp: i32,
    pub inf: bool,
    /// OE, UE and PE as the rounding raised them.
    pub flags: u32,
    /// The magnitude was rounded up (x87 C1).
    pub up: bool,
    /// Tiny (after rounding, unbounded exponent).
    pub tiny: bool,
}

/// `sig >> shift` rounded by `rc` for a value of sign `sign`: (result,
/// inexact, incremented).
pub fn shr_round(sig: u128, shift: i32, sign: bool, rc: u32) -> (u128, bool, bool) {
    if shift <= 0 {
        return (sig << (-shift) as u32, false, false);
    }
    let (k, half, sticky) = if shift >= 128 {
        if shift == 128 { (0, sig >> 127 != 0, sig << 1 != 0) } else { (0, false, sig != 0) }
    } else {
        let s = shift as u32;
        let rem = sig & ((1u128 << s) - 1);
        let h = 1u128 << (s - 1);
        (sig >> s, rem & h != 0, rem & (h - 1) != 0)
    };
    let inexact = half || sticky;
    let inc = match rc {
        RC_NEAR => half && (sticky || k & 1 != 0),
        RC_DOWN => inexact && sign,
        RC_UP => inexact && !sign,
        _ => false,
    };
    (k + inc as u128, inexact, inc)
}

/// Rounds (-1)^sign * sig * 2^exp (sig nonzero) to `f` by `rc`. Tininess is
/// detected after rounding (x86). `ue_unmasked`: UE is raised on tininess
/// alone, as with an unmasked underflow exception.
pub fn round(sign: bool, exp: i32, sig: u128, f: &Fmt, rc: u32, ue_unmasked: bool) -> Rounded {
    let p = f.p as i32;
    let e = exp + 127 - sig.leading_zeros() as i32;
    let tiny = e < f.emin && {
        let (k, _, _) = shr_round(sig, e - (p - 1) - exp, sign, rc);
        !(k >> p != 0 && e + 1 >= f.emin)
    };
    let mut lsb = (e - (p - 1)).max(f.lsb_min);
    let (mut k, inexact, up) = shr_round(sig, lsb - exp, sign, rc);
    if k >> p != 0 {
        k >>= 1;
        lsb += 1;
    }
    let mut flags = 0;
    if inexact {
        flags |= PE;
    }
    if tiny && (inexact || ue_unmasked) {
        flags |= UE;
    }
    let mut r = Rounded { sign, sig: k as u64, exp: lsb, inf: false, flags, up, tiny };
    if k != 0 && lsb + 127 - k.leading_zeros() as i32 > f.emax {
        r.flags |= OE | PE;
        let to_inf = match rc {
            RC_NEAR => true,
            RC_DOWN => sign,
            RC_UP => !sign,
            _ => false,
        };
        if to_inf {
            r.inf = true;
            r.up = true;
        } else {
            r.sig = (1u64 << f.p) - 1;
            r.exp = f.emax - (p - 1);
            r.up = false;
        }
    }
    r
}

/// The significand shifted so its top bit is bit 63, and the matching exponent.
pub fn norm(v: &Val) -> (i32, u64) {
    let lz = v.sig.leading_zeros();
    (v.exp - lz as i32, v.sig << lz)
}

/// a + b with the given signs (b's sign flipped for a subtraction). Operands
/// are Zero, Finite or Inf.
pub fn add(sa: bool, a: &Val, sb: bool, b: &Val, rc: u32) -> Pre {
    use Class::*;
    match (a.class, b.class) {
        (Inf, Inf) => {
            if sa == sb {
                Pre::Inf(sa)
            } else {
                Pre::Invalid
            }
        }
        (Inf, _) => Pre::Inf(sa),
        (_, Inf) => Pre::Inf(sb),
        (Zero, Zero) => Pre::Zero(if sa == sb { sa } else { rc == RC_DOWN }),
        (Zero, _) => Pre::Num(sb, b.exp, b.sig as u128),
        (_, Zero) => Pre::Num(sa, a.exp, a.sig as u128),
        _ => {
            let (ea, ma) = norm(a);
            let (eb, mb) = norm(b);
            let ((sx, ex, mx), (sy, ey, my)) =
                if (ea, ma) >= (eb, mb) { ((sa, ea, ma), (sb, eb, mb)) } else { ((sb, eb, mb), (sa, ea, ma)) };
            let d = (ex - ey) as u32;
            let x = (mx as u128) << 62;
            let y0 = (my as u128) << 62;
            let y = if d >= 126 {
                1
            } else {
                (y0 >> d) | ((y0 & ((1u128 << d) - 1) != 0) as u128)
            };
            let e = ex - 62;
            if sx == sy {
                Pre::Num(sx, e, x + y)
            } else if x == y {
                Pre::Zero(rc == RC_DOWN)
            } else {
                Pre::Num(sx, e, x - y)
            }
        }
    }
}

pub fn mul(a: &Val, b: &Val) -> Pre {
    use Class::*;
    let s = a.sign ^ b.sign;
    match (a.class, b.class) {
        (Inf, Zero) | (Zero, Inf) => Pre::Invalid,
        (Inf, _) | (_, Inf) => Pre::Inf(s),
        (Zero, _) | (_, Zero) => Pre::Zero(s),
        _ => Pre::Num(s, a.exp + b.exp, a.sig as u128 * b.sig as u128),
    }
}

pub fn div(a: &Val, b: &Val) -> Pre {
    use Class::*;
    let s = a.sign ^ b.sign;
    match (a.class, b.class) {
        (Inf, Inf) | (Zero, Zero) => Pre::Invalid,
        (Inf, _) => Pre::Inf(s),
        (_, Inf) | (Zero, _) => Pre::Zero(s),
        (_, Zero) => Pre::DivZero(s),
        _ => {
            let (ea, ma) = norm(a);
            let (eb, mb) = norm(b);
            let mb = mb as u128;
            let n = (ma as u128) << 64;
            let (q1, r1) = (n / mb, n % mb);
            let n2 = r1 << 64;
            let (q2, r2) = (n2 / mb, n2 % mb);
            let q = (q1 << 62) | (q2 >> 2) | ((q2 & 3 != 0 || r2 != 0) as u128);
            Pre::Num(s, ea - eb - 126, q)
        }
    }
}

pub fn sqrt(a: &Val) -> Pre {
    match a.class {
        Class::Zero => Pre::Zero(a.sign),
        _ if a.sign => Pre::Invalid,
        Class::Inf => Pre::Inf(false),
        _ => {
            let (e, m) = norm(a);
            let (e, m) = if e & 1 != 0 { (e - 1, (m as u128) << 1) } else { (e, m as u128) };
            let n = m << 62;
            let mut r: u128 = 0;
            let mut rem: u128 = 0;
            for i in 0..68u32 {
                let pair = if i < 64 { (n >> (126 - 2 * i)) & 3 } else { 0 };
                rem = (rem << 2) | pair;
                let t = (r << 2) | 1;
                if rem >= t {
                    rem -= t;
                    r = (r << 1) | 1;
                } else {
                    r <<= 1;
                }
            }
            Pre::Num(false, (e - 62) / 2 - 5, (r << 1) | (rem != 0) as u128)
        }
    }
}

/// Rounds a finite value to an integer by `rc`: (magnitude, inexact,
/// rounded up), or `None` if the magnitude is 2^64 or more.
pub fn to_int(v: &Val, rc: u32) -> Option<(u64, bool, bool)> {
    if v.class == Class::Zero {
        return Some((0, false, false));
    }
    if v.exp >= 0 {
        if v.exp + 64 - v.sig.leading_zeros() as i32 > 64 {
            return None;
        }
        return Some((v.sig << v.exp, false, false));
    }
    let (k, inexact, up) = shr_round(v.sig as u128, -v.exp, v.sign, rc);
    if k >> 64 != 0 {
        return None;
    }
    Some((k as u64, inexact, up))
}

/// Magnitude order of Zero, Finite and Inf operands.
fn cmp_mag(a: &Val, b: &Val) -> Ordering {
    let key = |v: &Val| match v.class {
        Class::Zero => (0, 0, 0),
        Class::Inf => (2, 0, 0),
        _ => {
            let (e, m) = norm(v);
            (1, e, m)
        }
    };
    key(a).cmp(&key(b))
}

/// Numeric order of two non-NaN operands (±0 equal).
pub fn cmp(a: &Val, b: &Val) -> Ordering {
    if a.class == Class::Zero && b.class == Class::Zero {
        return Ordering::Equal;
    }
    match (a.sign, b.sign) {
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        (false, false) => cmp_mag(a, b),
        (true, true) => cmp_mag(b, a),
    }
}
