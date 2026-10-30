//! The exports of `x64wasm_fp.wasm` (`abi.md`, "Floating-point helper
//! module"): C ABI wrappers over the Rust API that keep the last status for
//! `status()`. wasm32 only, so native builds export no unmangled symbols.

use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// The status of the last call. One instance per thread (no shared memory).
static STATUS: AtomicU32 = AtomicU32::new(0);

#[inline(always)]
fn keep<T>((r, s): (T, u32)) -> T {
    STATUS.store(s, Relaxed);
    r
}

#[unsafe(no_mangle)]
pub extern "C" fn status() -> i32 {
    STATUS.load(Relaxed) as i32
}

macro_rules! sse_bin {
    ($($name:ident: $t:ty, $u:ty;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(a: $t, b: $t, mxcsr: i32) -> $t {
            keep(crate::$name(a as $u, b as $u, mxcsr as u32)) as $t
        }
    )*};
}
sse_bin! {
    f32_add: i32, u32; f32_sub: i32, u32; f32_mul: i32, u32; f32_div: i32, u32; f32_min: i32, u32; f32_max: i32, u32;
    f64_add: i64, u64; f64_sub: i64, u64; f64_mul: i64, u64; f64_div: i64, u64; f64_min: i64, u64; f64_max: i64, u64;
}

#[unsafe(no_mangle)]
pub extern "C" fn f32_sqrt(a: i32, mxcsr: i32) -> i32 {
    keep(crate::f32_sqrt(a as u32, mxcsr as u32)) as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn f64_sqrt(a: i64, mxcsr: i32) -> i64 {
    keep(crate::f64_sqrt(a as u64, mxcsr as u32)) as i64
}
#[unsafe(no_mangle)]
pub extern "C" fn f32_cmp(a: i32, b: i32, pred: i32, mxcsr: i32) -> i32 {
    keep(crate::f32_cmp(
        a as u32,
        b as u32,
        pred as u32,
        mxcsr as u32,
    )) as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn f64_cmp(a: i64, b: i64, pred: i32, mxcsr: i32) -> i32 {
    keep(crate::f64_cmp(
        a as u64,
        b as u64,
        pred as u32,
        mxcsr as u32,
    )) as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn f32_comi(a: i32, b: i32, signaling: i32, mxcsr: i32) -> i32 {
    keep(crate::f32_comi(
        a as u32,
        b as u32,
        signaling as u32,
        mxcsr as u32,
    )) as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn f64_comi(a: i64, b: i64, signaling: i32, mxcsr: i32) -> i32 {
    keep(crate::f64_comi(
        a as u64,
        b as u64,
        signaling as u32,
        mxcsr as u32,
    )) as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn f32_to_f64(a: i32, mxcsr: i32) -> i64 {
    keep(crate::f32_to_f64(a as u32, mxcsr as u32)) as i64
}
#[unsafe(no_mangle)]
pub extern "C" fn f64_to_f32(a: i64, mxcsr: i32) -> i32 {
    keep(crate::f64_to_f32(a as u64, mxcsr as u32)) as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn i32_to_f32(a: i32, mxcsr: i32) -> i32 {
    keep(crate::i32_to_f32(a, mxcsr as u32)) as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn i64_to_f32(a: i64, mxcsr: i32) -> i32 {
    keep(crate::i64_to_f32(a, mxcsr as u32)) as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn i64_to_f64(a: i64, mxcsr: i32) -> i64 {
    keep(crate::i64_to_f64(a, mxcsr as u32)) as i64
}
#[unsafe(no_mangle)]
pub extern "C" fn f32_to_i32(a: i32, mxcsr: i32, trunc: i32) -> i32 {
    keep(crate::f32_to_i32(a as u32, mxcsr as u32, trunc as u32))
}
#[unsafe(no_mangle)]
pub extern "C" fn f64_to_i32(a: i64, mxcsr: i32, trunc: i32) -> i32 {
    keep(crate::f64_to_i32(a as u64, mxcsr as u32, trunc as u32))
}
#[unsafe(no_mangle)]
pub extern "C" fn f32_to_i64(a: i32, mxcsr: i32, trunc: i32) -> i64 {
    keep(crate::f32_to_i64(a as u32, mxcsr as u32, trunc as u32))
}
#[unsafe(no_mangle)]
pub extern "C" fn f64_to_i64(a: i64, mxcsr: i32, trunc: i32) -> i64 {
    keep(crate::f64_to_i64(a as u64, mxcsr as u32, trunc as u32))
}

macro_rules! x87_bin {
    ($($name:ident)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(am: i64, ase: i32, bm: i64, bse: i32, bkind: i32, fcw: i32) -> i64 {
            keep(crate::$name(am as u64, ase as u32, bm as u64, bse as u32, bkind as u32, fcw as u32)) as i64
        }
    )*};
}
x87_bin!(f80_add f80_sub f80_subr f80_mul f80_div f80_divr);

macro_rules! x87_st01 {
    ($($name:ident)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(am: i64, ase: i32, bm: i64, bse: i32, fcw: i32) -> i64 {
            keep(crate::$name(am as u64, ase as u32, bm as u64, bse as u32, fcw as u32)) as i64
        }
    )*};
}
x87_st01!(f80_scale f80_prem f80_prem1);

#[unsafe(no_mangle)]
pub extern "C" fn f80_sqrt(am: i64, ase: i32, fcw: i32) -> i64 {
    keep(crate::f80_sqrt(am as u64, ase as u32, fcw as u32)) as i64
}
#[unsafe(no_mangle)]
pub extern "C" fn f80_round(am: i64, ase: i32, fcw: i32) -> i64 {
    keep(crate::f80_round(am as u64, ase as u32, fcw as u32)) as i64
}
#[unsafe(no_mangle)]
pub extern "C" fn f80_load(bm: i64, bkind: i32, fcw: i32) -> i64 {
    keep(crate::f80_load(bm as u64, bkind as u32, fcw as u32)) as i64
}
#[unsafe(no_mangle)]
pub extern "C" fn f80_store(am: i64, ase: i32, kind: i32, fcw: i32) -> i64 {
    keep(crate::f80_store(
        am as u64,
        ase as u32,
        kind as u32,
        fcw as u32,
    )) as i64
}
#[unsafe(no_mangle)]
pub extern "C" fn f80_cmp(
    am: i64,
    ase: i32,
    bm: i64,
    bse: i32,
    bkind: i32,
    quiet: i32,
    fcw: i32,
) -> i32 {
    keep(crate::f80_cmp(
        am as u64,
        ase as u32,
        bm as u64,
        bse as u32,
        bkind as u32,
        quiet as u32,
        fcw as u32,
    )) as i32
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
