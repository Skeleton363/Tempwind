//! Exact x86 soft float for translated code (`docs/design/abi.md`,
//! "Floating-point helper module (`x64wasm_fp.wasm`, v4)").
//!
//! One function per export of `x64wasm_fp.wasm`. Each returns its result
//! together with its status word: for SSE the MXCSR after the operation (with
//! [`FP_UNMASKED`] when a raised exception is unmasked), for x87 the FSW-style
//! status (exception flags, ES, the condition codes the operation defines
//! and, for 80-bit results, `se` in bits 16-31).
//!
//! Integer arithmetic only: no `f32`/`f64` arithmetic anywhere (enforced by
//! `clippy::float_arithmetic`), so results are the same on every host.
//! Values are passed as bits: f32 as `u32`, f64 as `u64`, an 80-bit value as
//! `m: u64` (significand with the J bit) and `se: u32` (sign in bit 15,
//! biased exponent in bits 0-14).
//!
//! On wasm32 the crate exports the C ABI functions of the contract
//! (`wasm` module); native builds export no unmangled symbols.

#![no_std]
#![deny(clippy::float_arithmetic, clippy::float_cmp)]

// Native builds (tests, the native cdylib) link std for its panic handler.
#[cfg(not(target_arch = "wasm32"))]
extern crate std;

mod exact;
mod sse;
#[cfg(target_arch = "wasm32")]
mod wasm;
mod x87;

pub use crate::exact::{DE, IE, OE, PE, UE, ZE};
pub use sse::FP_UNMASKED;

use sse::{D, Op, S};

/// Kinds of `kind` and `bkind` (x87 operands).
pub const KIND_F80: u32 = 0;
pub const KIND_F32: u32 = 1;
pub const KIND_F64: u32 = 2;
pub const KIND_I16: u32 = 3;
pub const KIND_I32: u32 = 4;
pub const KIND_I64: u32 = 5;

// ---- SSE ----

pub fn f32_add(a: u32, b: u32, mxcsr: u32) -> (u32, u32) {
    let (r, s) = sse::arith(&S, a as u64, b as u64, mxcsr, Op::Add);
    (r as u32, s)
}
pub fn f32_sub(a: u32, b: u32, mxcsr: u32) -> (u32, u32) {
    let (r, s) = sse::arith(&S, a as u64, b as u64, mxcsr, Op::Sub);
    (r as u32, s)
}
pub fn f32_mul(a: u32, b: u32, mxcsr: u32) -> (u32, u32) {
    let (r, s) = sse::arith(&S, a as u64, b as u64, mxcsr, Op::Mul);
    (r as u32, s)
}
pub fn f32_div(a: u32, b: u32, mxcsr: u32) -> (u32, u32) {
    let (r, s) = sse::arith(&S, a as u64, b as u64, mxcsr, Op::Div);
    (r as u32, s)
}
pub fn f32_min(a: u32, b: u32, mxcsr: u32) -> (u32, u32) {
    let (r, s) = sse::minmax(&S, a as u64, b as u64, mxcsr, false);
    (r as u32, s)
}
pub fn f32_max(a: u32, b: u32, mxcsr: u32) -> (u32, u32) {
    let (r, s) = sse::minmax(&S, a as u64, b as u64, mxcsr, true);
    (r as u32, s)
}
pub fn f32_sqrt(a: u32, mxcsr: u32) -> (u32, u32) {
    let (r, s) = sse::sqrt_(&S, a as u64, mxcsr);
    (r as u32, s)
}
pub fn f64_add(a: u64, b: u64, mxcsr: u32) -> (u64, u32) {
    sse::arith(&D, a, b, mxcsr, Op::Add)
}
pub fn f64_sub(a: u64, b: u64, mxcsr: u32) -> (u64, u32) {
    sse::arith(&D, a, b, mxcsr, Op::Sub)
}
pub fn f64_mul(a: u64, b: u64, mxcsr: u32) -> (u64, u32) {
    sse::arith(&D, a, b, mxcsr, Op::Mul)
}
pub fn f64_div(a: u64, b: u64, mxcsr: u32) -> (u64, u32) {
    sse::arith(&D, a, b, mxcsr, Op::Div)
}
pub fn f64_min(a: u64, b: u64, mxcsr: u32) -> (u64, u32) {
    sse::minmax(&D, a, b, mxcsr, false)
}
pub fn f64_max(a: u64, b: u64, mxcsr: u32) -> (u64, u32) {
    sse::minmax(&D, a, b, mxcsr, true)
}
pub fn f64_sqrt(a: u64, mxcsr: u32) -> (u64, u32) {
    sse::sqrt_(&D, a, mxcsr)
}
/// cmpss: 1 if predicate `pred` (imm8 bits 2:0) holds.
pub fn f32_cmp(a: u32, b: u32, pred: u32, mxcsr: u32) -> (u32, u32) {
    sse::cmp_pred(&S, a as u64, b as u64, pred, mxcsr)
}
pub fn f64_cmp(a: u64, b: u64, pred: u32, mxcsr: u32) -> (u32, u32) {
    sse::cmp_pred(&D, a, b, pred, mxcsr)
}
/// comiss (`signaling` 1) and ucomiss (0): ZF 0x40, PF 0x04, CF 0x01.
pub fn f32_comi(a: u32, b: u32, signaling: u32, mxcsr: u32) -> (u32, u32) {
    sse::comi(&S, a as u64, b as u64, signaling != 0, mxcsr)
}
pub fn f64_comi(a: u64, b: u64, signaling: u32, mxcsr: u32) -> (u32, u32) {
    sse::comi(&D, a, b, signaling != 0, mxcsr)
}
pub fn f32_to_f64(a: u32, mxcsr: u32) -> (u64, u32) {
    sse::convert(&S, &D, a as u64, mxcsr)
}
pub fn f64_to_f32(a: u64, mxcsr: u32) -> (u32, u32) {
    let (r, s) = sse::convert(&D, &S, a, mxcsr);
    (r as u32, s)
}
pub fn i32_to_f32(a: i32, mxcsr: u32) -> (u32, u32) {
    let (r, s) = sse::from_int(&S, a as i64, mxcsr);
    (r as u32, s)
}
pub fn i64_to_f32(a: i64, mxcsr: u32) -> (u32, u32) {
    let (r, s) = sse::from_int(&S, a, mxcsr);
    (r as u32, s)
}
pub fn i64_to_f64(a: i64, mxcsr: u32) -> (u64, u32) {
    sse::from_int(&D, a, mxcsr)
}
/// cvtss2si r32 (`trunc` 0) and cvttss2si r32 (1).
pub fn f32_to_i32(a: u32, mxcsr: u32, trunc: u32) -> (i32, u32) {
    let (r, s) = sse::to_int_(&S, a as u64, mxcsr, trunc != 0, 32);
    (r as i32, s)
}
pub fn f64_to_i32(a: u64, mxcsr: u32, trunc: u32) -> (i32, u32) {
    let (r, s) = sse::to_int_(&D, a, mxcsr, trunc != 0, 32);
    (r as i32, s)
}
pub fn f32_to_i64(a: u32, mxcsr: u32, trunc: u32) -> (i64, u32) {
    sse::to_int_(&S, a as u64, mxcsr, trunc != 0, 64)
}
pub fn f64_to_i64(a: u64, mxcsr: u32, trunc: u32) -> (i64, u32) {
    sse::to_int_(&D, a, mxcsr, trunc != 0, 64)
}

// ---- x87 ----
//
// `a` is DEST and `b` SRC as the SDM names them. `bkind` operands pass their
// value unconverted in `bm` (integers sign-extended; `bse` ignored). 80-bit
// results return `m`, with `se` in status bits 16-31.

macro_rules! x87_arith {
    ($($(#[$doc:meta])* $name:ident => $op:ident;)*) => {$(
        $(#[$doc])*
        pub fn $name(am: u64, ase: u32, bm: u64, bse: u32, bkind: u32, fcw: u32) -> (u64, u32) {
            x87::arith(am, ase, bm, bse, bkind, fcw, x87::Op::$op)
        }
    )*};
}
x87_arith! {
    /// fadd, faddp, fiadd: a + b, rounded to PC by RC; C1 = rounded up.
    f80_add => Add;
    /// fsub, fsubp, fisub: a - b.
    f80_sub => Sub;
    /// fsubr, fsubrp, fisubr: b - a.
    f80_subr => SubR;
    f80_mul => Mul;
    /// fdiv, fdivp, fidiv: a / b.
    f80_div => Div;
    /// fdivr, fdivrp, fidivr: b / a.
    f80_divr => DivR;
}
/// fsqrt (PC and RC).
pub fn f80_sqrt(am: u64, ase: u32, fcw: u32) -> (u64, u32) {
    x87::sqrt_(am, ase, fcw)
}
/// frndint (RC).
pub fn f80_round(am: u64, ase: u32, fcw: u32) -> (u64, u32) {
    x87::round_int(am, ase, fcw)
}
/// fscale: a = ST(0), b = ST(1).
pub fn f80_scale(am: u64, ase: u32, bm: u64, bse: u32, fcw: u32) -> (u64, u32) {
    x87::scale(am, ase, bm, bse, fcw)
}
/// fprem: C2 = incomplete, C0, C3, C1 = quotient bits 2, 1, 0.
pub fn f80_prem(am: u64, ase: u32, bm: u64, bse: u32, fcw: u32) -> (u64, u32) {
    x87::prem(am, ase, bm, bse, fcw, false)
}
/// fprem1 (the IEEE remainder).
pub fn f80_prem1(am: u64, ase: u32, bm: u64, bse: u32, fcw: u32) -> (u64, u32) {
    x87::prem(am, ase, bm, bse, fcw, true)
}
/// fld m32/m64 (kinds 1, 2) and fild (kinds 3-5).
pub fn f80_load(bm: u64, bkind: u32, fcw: u32) -> (u64, u32) {
    x87::load(bm, bkind, fcw)
}
/// fst m32/m64 (kinds 1, 2: f32/f64 bits) and fist (kinds 3-5: the integer
/// sign-extended, or the integer indefinite).
pub fn f80_store(am: u64, ase: u32, kind: u32, fcw: u32) -> (u64, u32) {
    x87::store(am, ase, kind, fcw)
}
/// fcom family: C3, C2, C0 in their FSW positions (`quiet` for fucom).
pub fn f80_cmp(
    am: u64,
    ase: u32,
    bm: u64,
    bse: u32,
    bkind: u32,
    quiet: u32,
    fcw: u32,
) -> (u32, u32) {
    x87::compare(am, ase, bm, bse, bkind, quiet != 0, fcw)
}
