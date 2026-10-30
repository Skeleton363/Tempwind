//! The unmasked-exception indicator, which can't run natively without
//! trapping: unit tests from the SDM's rules (Vol. 1, 11.5 for SSE: IE, DE
//! and ZE are detected before the computation, which an unmasked one stops;
//! OE, UE and PE after it; an unmasked UE fires on tininess alone).

use x64wasm_fp::*;

const MASKED: u32 = 0x1F80;
const IM: u32 = 0x80;
const DM: u32 = 0x100;
const ZM: u32 = 0x200;
const OM: u32 = 0x400;
const UM: u32 = 0x800;
const PM: u32 = 0x1000;
const FTZ: u32 = 0x8000;

const ONE: u32 = 0x3F80_0000;
const TWO: u32 = 0x4000_0000;
const MIN_NORMAL: u32 = 0x0080_0000;

#[test]
fn sse_masked_never_sets_the_indicator() {
    let (_, s) = f32_div(ONE, 0, MASKED);
    assert_eq!(s, MASKED | ZE);
    let (_, s) = f32_add(0x7F80_0001, ONE, MASKED);
    assert_eq!(s, MASKED | IE);
}

#[test]
fn sse_precomputation_exceptions_stop_the_operation() {
    // Unmasked DE: the add isn't performed, so no PE although 1 + denormal
    // is inexact.
    let (_, s) = f32_add(ONE, 1, MASKED & !DM);
    assert_eq!(s, (MASKED & !DM) | DE | FP_UNMASKED);
    // Masked DE, unmasked PE: the operation proceeds and PE is unmasked.
    let (_, s) = f32_add(ONE, 1, MASKED & !PM);
    assert_eq!(s, (MASKED & !PM) | DE | PE | FP_UNMASKED);
    let (_, s) = f32_add(0x7F80_0001, ONE, MASKED & !IM);
    assert_eq!(s, (MASKED & !IM) | IE | FP_UNMASKED);
    let (_, s) = f64_div(0x3FF0_0000_0000_0000, 0, MASKED & !ZM);
    assert_eq!(s, (MASKED & !ZM) | ZE | FP_UNMASKED);
    let (_, s) = f32_sqrt(0xBF80_0000, MASKED & !IM);
    assert_eq!(s, (MASKED & !IM) | IE | FP_UNMASKED);
    // A QNaN compare: IE only for the signaling predicates and comiss.
    let (_, s) = f32_cmp(0x7FC0_0000, ONE, 0, MASKED & !IM);
    assert_eq!(s, MASKED & !IM);
    let (_, s) = f32_cmp(0x7FC0_0000, ONE, 1, MASKED & !IM);
    assert_eq!(s, (MASKED & !IM) | IE | FP_UNMASKED);
    let (_, s) = f32_comi(0x7FC0_0000, ONE, 1, MASKED & !IM);
    assert_eq!(s, (MASKED & !IM) | IE | FP_UNMASKED);
    let (_, s) = f32_comi(0x7FC0_0000, ONE, 0, MASKED & !IM);
    assert_eq!(s, MASKED & !IM);
    let (_, s) = f32_to_i32(0x7FC0_0000, MASKED & !IM, 0);
    assert_eq!(s, (MASKED & !IM) | IE | FP_UNMASKED);
}

#[test]
fn sse_postcomputation_exceptions() {
    let (_, s) = f32_mul(0x7F7F_FFFF, TWO, MASKED & !OM);
    assert_eq!(s & FP_UNMASKED, FP_UNMASKED);
    assert_eq!(s & OE, OE);
    let (_, s) = f32_add(ONE, 0x3380_0000, MASKED & !PM);
    assert_eq!(s, (MASKED & !PM) | PE | FP_UNMASKED);
    // Exact.
    let (_, s) = f32_add(ONE, ONE, 0x1F80 & !0x1F80);
    assert_eq!(s, 0);
}

#[test]
fn sse_unmasked_underflow_fires_on_tininess_alone() {
    // MIN_NORMAL / 2 is an exact denormal: masked, no flag at all; unmasked,
    // UE.
    let (r, s) = f32_div(MIN_NORMAL, TWO, MASKED);
    assert_eq!((r, s), (0x0040_0000, MASKED));
    let (_, s) = f32_div(MIN_NORMAL, TWO, MASKED & !UM);
    assert_eq!(s, (MASKED & !UM) | UE | FP_UNMASKED);
    // FTZ applies only while UE is masked.
    let (r, s) = f32_div(MIN_NORMAL, TWO, MASKED | FTZ);
    assert_eq!((r, s), (0, MASKED | FTZ | UE | PE));
    let (_, s) = f32_div(MIN_NORMAL, TWO, (MASKED & !UM) | FTZ);
    assert_eq!(s, ((MASKED & !UM) | FTZ) | UE | FP_UNMASKED);
    // A tiny result that rounds to MIN_NORMAL is not tiny after rounding.
    let (r, s) = f32_mul(0x3F7F_FFFF, 0x0080_0001, MASKED & !UM);
    assert_eq!(r, MIN_NORMAL);
    assert_eq!(s, (MASKED & !UM) | PE);
}

#[test]
fn sse_sticky_flags_alone_dont_set_the_indicator() {
    // ZE already set and unmasked, but this operation raises nothing.
    let m = (MASKED & !ZM) | ZE;
    let (_, s) = f32_add(ONE, ONE, m);
    assert_eq!(s, m);
    // An input bit 31 is ignored.
    let (_, s) = f32_add(ONE, ONE, MASKED | FP_UNMASKED);
    assert_eq!(s, MASKED);
}

// ---- x87: ES (status bit 7) when a raised exception is unmasked in FCW ----

const FCW: u32 = 0x037F; // all masked, PC 64, RC nearest
const ES: u32 = 0x80;
const C1: u32 = 0x200;
const X1: (u64, u32) = (1 << 63, 0x3FFF);
const X_MIN_NORMAL: (u64, u32) = (1 << 63, 1);
const X2: (u64, u32) = (1 << 63, 0x4000);

fn low(s: u32) -> u32 {
    s & 0xFFFF
}

#[test]
fn x87_masked_never_sets_es() {
    let (_, s) = f80_div(X1.0, X1.1, 0, 0, KIND_F80, FCW);
    assert_eq!(low(s), ZE);
    let (_, s) = f80_add(X1.0, X1.1, 0xA000_0000_0000_0000, 0x7FFF, KIND_F80, FCW);
    assert_eq!(low(s), IE);
}

#[test]
fn x87_precomputation_exceptions_stop_the_operation() {
    // Unmasked DE: no PE although 1 + denormal is inexact.
    let (_, s) = f80_add(X1.0, X1.1, 1, 0, KIND_F80, FCW & !0x02);
    assert_eq!(low(s), DE | ES);
    // A denormal m32 operand raises DE too.
    let (_, s) = f80_mul(X1.0, X1.1, 1, 0, KIND_F32, FCW & !0x02);
    assert_eq!(low(s), DE | ES);
    let (_, s) = f80_add(
        X1.0,
        X1.1,
        0xA000_0000_0000_0000,
        0x7FFF,
        KIND_F80,
        FCW & !0x01,
    );
    assert_eq!(low(s), IE | ES);
    // An unsupported operand (unnormal) is invalid.
    let (_, s) = f80_add(
        X1.0,
        X1.1,
        0x4000_0000_0000_0000,
        0x3FFF,
        KIND_F80,
        FCW & !0x01,
    );
    assert_eq!(low(s), IE | ES);
    let (_, s) = f80_div(X1.0, X1.1, 0, 0, KIND_F80, FCW & !0x04);
    assert_eq!(low(s), ZE | ES);
    // fcom raises IE for a QNaN, fucom doesn't.
    let (_, s) = f80_cmp(
        X1.0,
        X1.1,
        0xC000_0000_0000_0000,
        0x7FFF,
        KIND_F80,
        0,
        FCW & !0x01,
    );
    assert_eq!(low(s), IE | ES | 0x4500);
    let (_, s) = f80_cmp(
        X1.0,
        X1.1,
        0xC000_0000_0000_0000,
        0x7FFF,
        KIND_F80,
        1,
        FCW & !0x01,
    );
    assert_eq!(low(s), 0x4500);
    let (_, s) = f80_load(0x7FA0_0000, KIND_F32, FCW & !0x01);
    assert_eq!(low(s), IE | ES);
    let (_, s) = f80_store(X1.0, 0x3FFF + 40, KIND_I32, FCW & !0x01);
    assert_eq!(low(s), IE | ES);
}

#[test]
fn x87_postcomputation_exceptions() {
    // 1 + 2^-70 is inexact; rounded down, so C1 = 0.
    let (_, s) = f80_add(X1.0, X1.1, 1 << 63, 0x3FFF - 70, KIND_F80, FCW & !0x20);
    assert_eq!(low(s), PE | ES);
    let (_, s) = f80_mul(u64::MAX, 0x7FFE, X2.0, X2.1, KIND_F80, FCW & !0x08);
    assert_eq!(low(s) & (OE | ES), OE | ES);
    // fst m32 of a value beyond f32's range overflows there.
    let (_, s) = f80_store(X1.0, 0x3FFF + 200, KIND_F32, FCW & !0x08);
    assert_eq!(low(s) & (OE | ES), OE | ES);
}

#[test]
fn x87_unmasked_underflow_fires_on_tininess_alone() {
    // MIN_NORMAL / 2 is an exact denormal: masked, no flag; unmasked, UE.
    let (m, s) = f80_div(X_MIN_NORMAL.0, X_MIN_NORMAL.1, X2.0, X2.1, KIND_F80, FCW);
    assert_eq!((m, low(s), s >> 16), (1 << 62, 0, 0));
    let (_, s) = f80_div(
        X_MIN_NORMAL.0,
        X_MIN_NORMAL.1,
        X2.0,
        X2.1,
        KIND_F80,
        FCW & !0x10,
    );
    assert_eq!(low(s), UE | ES);
    // The same for a store to m32: 2^-127 is an exact f32 denormal.
    let (r, s) = f80_store(X1.0, 0x3FFF - 127, KIND_F32, FCW);
    assert_eq!((r, low(s)), (0x0040_0000, 0));
    let (_, s) = f80_store(X1.0, 0x3FFF - 127, KIND_F32, FCW & !0x10);
    assert_eq!(low(s), UE | ES);
    // Rounded up into the denormal range: C1.
    let (_, s) = f80_store(X1.0, 0x3FFF - 160, KIND_F32, (FCW & !0x10) | 0x800);
    assert_eq!(low(s), UE | PE | ES | C1);
}

#[test]
fn x87_status_carries_se() {
    let (m, s) = f80_add(X1.0, X1.1, X1.0, X1.1, KIND_F80, FCW);
    assert_eq!((m, s >> 16, low(s)), (1 << 63, 0x4000, 0));
    let (m, s) = f80_load(0xBF80_0000, KIND_F32, FCW);
    assert_eq!((m, s >> 16), (1 << 63, 0xBFFF));
}
