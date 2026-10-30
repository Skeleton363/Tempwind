//! SSE and SSE2 floating point (`docs/design/translator.md`, "SSE: fast path
//! and helper"). Each op takes the inline fast path, the wasm instruction
//! itself, when its result is bit-exact under the current MXCSR and the
//! operation can't change MXCSR; otherwise it calls the `fp` helper (once
//! per lane, lane 0 first, threading MXCSR), checks `FP_UNMASKED` and stores
//! the new MXCSR.
//!
//! Scalar and packed forms share the code: the fast path computes on the
//! whole vector and the test looks at the lanes the instruction uses (lane 0
//! for scalar forms) through the sign bits of a lane mask.

use wasm_encoder::{BlockType, InstructionSink};

use super::{ChunkEmitter, Label, P_STATE, mem};
use crate::abi::{self, FpFunc};
use crate::ir::{FpArith, FpCvt, FpFmt, Op, SseFp, Value, Width};

/// MXCSR bits of the arithmetic fast-path test: the exception masks (bits
/// 7-12), RC (13-14) and PE (5).
const MXCSR_ARITH_MASK: i32 = 0x7FA0;
/// Every exception masked, RC = nearest, PE already set.
const MXCSR_ARITH: i32 = 0x1FA0;
/// PM and PE: an inexact result changes nothing.
const MXCSR_PM_PE: i32 = 0x1020;
/// The rounding-control field.
const MXCSR_RC: i32 = 0x6000;

const ZF: i64 = abi::flags::ZF as i64;
const CF: i64 = abi::flags::CF as i64;

/// Vector operations by format.
#[derive(Clone, Copy)]
enum V {
    Abs,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Sqrt,
    Trunc,
    Nearest,
}

fn vop(f: &mut InstructionSink<'_>, fmt: FpFmt, op: V) {
    match (fmt, op) {
        (FpFmt::S, V::Abs) => f.f32x4_abs(),
        (FpFmt::S, V::Eq) => f.f32x4_eq(),
        (FpFmt::S, V::Ne) => f.f32x4_ne(),
        (FpFmt::S, V::Lt) => f.f32x4_lt(),
        (FpFmt::S, V::Le) => f.f32x4_le(),
        (FpFmt::S, V::Gt) => f.f32x4_gt(),
        (FpFmt::S, V::Ge) => f.f32x4_ge(),
        (FpFmt::S, V::Add) => f.f32x4_add(),
        (FpFmt::S, V::Sub) => f.f32x4_sub(),
        (FpFmt::S, V::Mul) => f.f32x4_mul(),
        (FpFmt::S, V::Div) => f.f32x4_div(),
        (FpFmt::S, V::Sqrt) => f.f32x4_sqrt(),
        (FpFmt::S, V::Trunc) => f.f32x4_trunc(),
        (FpFmt::S, V::Nearest) => f.f32x4_nearest(),
        (FpFmt::D, V::Abs) => f.f64x2_abs(),
        (FpFmt::D, V::Eq) => f.f64x2_eq(),
        (FpFmt::D, V::Ne) => f.f64x2_ne(),
        (FpFmt::D, V::Lt) => f.f64x2_lt(),
        (FpFmt::D, V::Le) => f.f64x2_le(),
        (FpFmt::D, V::Gt) => f.f64x2_gt(),
        (FpFmt::D, V::Ge) => f.f64x2_ge(),
        (FpFmt::D, V::Add) => f.f64x2_add(),
        (FpFmt::D, V::Sub) => f.f64x2_sub(),
        (FpFmt::D, V::Mul) => f.f64x2_mul(),
        (FpFmt::D, V::Div) => f.f64x2_div(),
        (FpFmt::D, V::Sqrt) => f.f64x2_sqrt(),
        (FpFmt::D, V::Trunc) => f.f64x2_trunc(),
        (FpFmt::D, V::Nearest) => f.f64x2_nearest(),
    };
}

/// Lanes of a format in a vector.
const fn lanes(fmt: FpFmt) -> u8 {
    match fmt {
        FpFmt::S => 4,
        FpFmt::D => 2,
    }
}

/// A vector with every lane of `fmt` set to `bits`.
fn splat(fmt: FpFmt, bits: u64) -> i128 {
    let v = match fmt {
        FpFmt::S => {
            let b = (bits as u32) as u128;
            b | b << 32 | b << 64 | b << 96
        }
        FpFmt::D => bits as u128 | (bits as u128) << 64,
    };
    v as i128
}

/// The smallest normal number.
fn min_normal(fmt: FpFmt) -> u64 {
    match fmt {
        FpFmt::S => f32::MIN_POSITIVE.to_bits() as u64,
        FpFmt::D => f64::MIN_POSITIVE.to_bits(),
    }
}

fn infinity(fmt: FpFmt) -> u64 {
    match fmt {
        FpFmt::S => f32::INFINITY.to_bits() as u64,
        FpFmt::D => f64::INFINITY.to_bits(),
    }
}

/// -2^(bits - 1) and 2^(bits - 1) in `fmt`: an integral value v fits a
/// signed `bits`-bit integer iff lo <= v < hi.
fn int_range(fmt: FpFmt, width: Width) -> (u64, u64) {
    match (fmt, width) {
        (FpFmt::S, Width::W64) => (
            (-9_223_372_036_854_775_808.0f32).to_bits() as u64,
            9_223_372_036_854_775_808.0f32.to_bits() as u64,
        ),
        (FpFmt::S, _) => (
            (-2_147_483_648.0f32).to_bits() as u64,
            2_147_483_648.0f32.to_bits() as u64,
        ),
        (FpFmt::D, Width::W64) => (
            (-9_223_372_036_854_775_808.0f64).to_bits(),
            9_223_372_036_854_775_808.0f64.to_bits(),
        ),
        (FpFmt::D, _) => (
            (-2_147_483_648.0f64).to_bits(),
            2_147_483_648.0f64.to_bits(),
        ),
    }
}

/// Bytes of a vector with lane 0 of `fmt` from the second operand and the
/// rest from the first.
fn lane0_from_b(fmt: FpFmt) -> [u8; 16] {
    let n = match fmt {
        FpFmt::S => 4,
        FpFmt::D => 8,
    };
    core::array::from_fn(|i| if i < n { 16 + i as u8 } else { i as u8 })
}

/// Pushes 1 if the lane-mask vector on the stack is all ones in lanes
/// 0..n of `fmt`, else 0.
fn all_lanes(f: &mut InstructionSink<'_>, fmt: FpFmt, n: u8) {
    if n == lanes(fmt) {
        match fmt {
            FpFmt::S => f.i32x4_all_true(),
            FpFmt::D => f.i64x2_all_true(),
        };
    } else {
        match fmt {
            FpFmt::S => f.i32x4_bitmask(),
            FpFmt::D => f.i64x2_bitmask(),
        };
        let m = (1i32 << n) - 1;
        f.i32_const(m);
        f.i32_and();
        f.i32_const(m);
        f.i32_eq();
    }
}

/// Pushes the lanes of `local` that are neither NaN nor denormal:
/// `|x| >= MIN_NORMAL or x == 0`.
fn ordinary(f: &mut InstructionSink<'_>, fmt: FpFmt, local: u32) {
    f.local_get(local);
    vop(f, fmt, V::Abs);
    f.v128_const(splat(fmt, min_normal(fmt)));
    vop(f, fmt, V::Ge);
    f.local_get(local);
    f.v128_const(0);
    vop(f, fmt, V::Eq);
    f.v128_or();
}

/// Pushes the lanes of `local` with `MIN_NORMAL < |x| < infinity`.
fn safe_result(f: &mut InstructionSink<'_>, fmt: FpFmt, local: u32) {
    f.local_get(local);
    vop(f, fmt, V::Abs);
    f.v128_const(splat(fmt, min_normal(fmt)));
    vop(f, fmt, V::Gt);
    f.local_get(local);
    vop(f, fmt, V::Abs);
    f.v128_const(splat(fmt, infinity(fmt)));
    vop(f, fmt, V::Lt);
    f.v128_and();
}

/// Pushes lane `i` of `local` as integer bits.
fn lane_bits(f: &mut InstructionSink<'_>, fmt: FpFmt, local: u32, i: u8) {
    f.local_get(local);
    match fmt {
        FpFmt::S => f.i32x4_extract_lane(i),
        FpFmt::D => f.i64x2_extract_lane(i),
    };
}

fn replace_lane(f: &mut InstructionSink<'_>, fmt: FpFmt, i: u8) {
    match fmt {
        FpFmt::S => f.i32x4_replace_lane(i),
        FpFmt::D => f.i64x2_replace_lane(i),
    };
}

/// How the slow path calls the helper for one lane.
struct Call<'a> {
    func: FpFunc,
    /// Pushes the arguments before MXCSR for lane `i`.
    args: &'a dyn Fn(&mut InstructionSink<'_>, u8),
    /// A constant argument after MXCSR (`trunc`).
    after: Option<i32>,
    /// Turns the helper's result into the lane value (`cmp`'s mask).
    post: &'a dyn Fn(&mut InstructionSink<'_>),
    /// The result lane format.
    out: FpFmt,
}

impl ChunkEmitter<'_> {
    fn fp_locals(&self) -> (u32, u32) {
        self.l_fp.expect("chunk has floating-point locals")
    }

    /// MXCSR from the state block into the `mxcsr` local.
    fn load_mxcsr(&self, f: &mut InstructionSink<'_>) {
        let (mx, _) = self.fp_locals();
        f.local_get(P_STATE);
        f.i32_load(mem(abi::STATE_MXCSR));
        f.local_set(mx);
    }

    /// Pushes `(mxcsr & mask) == value` (from the `mxcsr` local).
    fn mxcsr_is(&self, f: &mut InstructionSink<'_>, mask: i32, value: i32) {
        let (mx, _) = self.fp_locals();
        f.local_get(mx);
        f.i32_const(mask);
        f.i32_and();
        f.i32_const(value);
        f.i32_eq();
    }

    /// After a helper: `mxcsr = status()`; exits with `EXIT_UNSUPPORTED`
    /// if `FP_UNMASKED` (bit 31) is set.
    fn check_status(&mut self, f: &mut InstructionSink<'_>) {
        let (mx, _) = self.fp_locals();
        f.call(self.fp.index(FpFunc::Status));
        f.local_tee(mx);
        f.i32_const(0);
        f.i32_lt_s();
        f.if_(BlockType::Empty);
        self.labels.push(Label::If);
        self.exit(f, abi::EXIT_UNSUPPORTED, self.cur_addr);
        f.end();
        self.labels.pop();
    }

    /// Stores the `mxcsr` local to the state block.
    fn store_mxcsr(&self, f: &mut InstructionSink<'_>) {
        let (mx, _) = self.fp_locals();
        f.local_get(P_STATE);
        f.local_get(mx);
        f.i32_store(mem(abi::STATE_MXCSR));
    }

    /// The helper for lanes 0..n, lane 0 first, each replacing its lane of
    /// `acc` and checking the status. MXCSR must be in its local.
    fn helper_lanes(&mut self, f: &mut InstructionSink<'_>, acc: u32, n: u8, call: &Call<'_>) {
        let (mx, _) = self.fp_locals();
        for i in 0..n {
            f.local_get(acc);
            (call.args)(f, i);
            f.local_get(mx);
            if let Some(c) = call.after {
                f.i32_const(c);
            }
            f.call(self.fp.index(call.func));
            (call.post)(f);
            replace_lane(f, call.out, i);
            f.local_set(acc);
            self.check_status(f);
        }
    }

    /// `if <test on the stack> { fast } else { slow }`, the branches
    /// emitted by the closures.
    fn fast_or_slow(
        &mut self,
        f: &mut InstructionSink<'_>,
        fast: impl FnOnce(&mut Self, &mut InstructionSink<'_>),
        slow: impl FnOnce(&mut Self, &mut InstructionSink<'_>),
    ) {
        f.if_(BlockType::Empty);
        self.labels.push(Label::If);
        fast(self, f);
        f.else_();
        slow(self, f);
        f.end();
        self.labels.pop();
    }

    pub(super) fn fp_op(&mut self, f: &mut InstructionSink<'_>, op: &Op) {
        match *op {
            Op::SseFp { dst, op, a, b } => {
                let (dst, a, b) = (self.vtemp(dst), self.vtemp(a), self.vtemp(b));
                match op {
                    SseFp::Arith { op, fmt, packed } => self.arith(f, op, fmt, packed, dst, a, b),
                    SseFp::Cmp { pred, fmt, packed } => self.cmp(f, pred, fmt, packed, dst, a, b),
                    SseFp::Cvt(cvt) => self.cvt(f, cvt, dst, a, b),
                }
            }
            Op::SseFromInt {
                dst,
                fmt,
                width,
                a,
                src,
            } => {
                let (dst, a) = (self.vtemp(dst), self.vtemp(a));
                self.cvt_from_int(f, fmt, width, dst, a, src);
            }
            Op::SseToInt {
                dst,
                fmt,
                width,
                trunc,
                b,
            } => {
                let (dst, b) = (self.temp(dst), self.vtemp(b));
                self.cvt_to_int(f, fmt, width, trunc, dst, b);
            }
            Op::SseComi {
                dst,
                fmt,
                signaling,
                a,
                b,
            } => {
                let (dst, a, b) = (self.temp(dst), self.vtemp(a), self.vtemp(b));
                self.comi(f, fmt, signaling, dst, a, b);
            }
            _ => unreachable!("not a floating-point op"),
        }
    }

    /// Sets `dst` from the fast result in the `fpv` local: every lane, or
    /// lane 0 merged into `a`.
    fn set_result(&self, f: &mut InstructionSink<'_>, fmt: FpFmt, packed: bool, dst: u32, a: u32) {
        let (_, v) = self.fp_locals();
        if packed {
            f.local_get(v);
        } else {
            f.local_get(a);
            f.local_get(v);
            f.i8x16_shuffle(lane0_from_b(fmt));
        }
        f.local_set(dst);
    }

    #[allow(clippy::too_many_arguments)]
    fn arith(
        &mut self,
        f: &mut InstructionSink<'_>,
        op: FpArith,
        fmt: FpFmt,
        packed: bool,
        dst: u32,
        a: u32,
        b: u32,
    ) {
        let (_, v) = self.fp_locals();
        let n = if packed { lanes(fmt) } else { 1 };
        match op {
            FpArith::Add | FpArith::Sub | FpArith::Mul | FpArith::Div => {
                f.local_get(a);
                f.local_get(b);
                vop(
                    f,
                    fmt,
                    match op {
                        FpArith::Add => V::Add,
                        FpArith::Sub => V::Sub,
                        FpArith::Mul => V::Mul,
                        _ => V::Div,
                    },
                );
                f.local_set(v);
                // MXCSR test, no operand denormal, MIN_NORMAL < |r| < inf.
                self.load_mxcsr(f);
                self.mxcsr_is(f, MXCSR_ARITH_MASK, MXCSR_ARITH);
                ordinary(f, fmt, a);
                ordinary(f, fmt, b);
                f.v128_and();
                safe_result(f, fmt, v);
                f.v128_and();
                all_lanes(f, fmt, n);
                f.i32_and();
            }
            FpArith::Sqrt => {
                f.local_get(b);
                vop(f, fmt, V::Sqrt);
                f.local_set(v);
                // MXCSR test and a positive normal operand.
                self.load_mxcsr(f);
                self.mxcsr_is(f, MXCSR_ARITH_MASK, MXCSR_ARITH);
                safe_operand(f, fmt, b);
                all_lanes(f, fmt, n);
                f.i32_and();
            }
            FpArith::Min | FpArith::Max => {
                // x86 returns the second operand unless a < b (a > b).
                f.local_get(a);
                f.local_get(b);
                f.local_get(a);
                f.local_get(b);
                vop(f, fmt, if op == FpArith::Min { V::Lt } else { V::Gt });
                f.v128_bitselect();
                f.local_set(v);
                // No operand NaN or denormal; MXCSR isn't read.
                ordinary(f, fmt, a);
                ordinary(f, fmt, b);
                f.v128_and();
                all_lanes(f, fmt, n);
            }
        }
        let func = arith_func(op, fmt);
        let sqrt = op == FpArith::Sqrt;
        let read_mxcsr = matches!(op, FpArith::Min | FpArith::Max);
        self.fast_or_slow(
            f,
            |e, f| e.set_result(f, fmt, packed, dst, a),
            |e, f| {
                if read_mxcsr {
                    e.load_mxcsr(f);
                }
                f.local_get(a);
                f.local_set(dst);
                let args = move |f: &mut InstructionSink<'_>, i: u8| {
                    if !sqrt {
                        lane_bits(f, fmt, a, i);
                    }
                    lane_bits(f, fmt, b, i);
                };
                e.helper_lanes(
                    f,
                    dst,
                    n,
                    &Call {
                        func,
                        args: &args,
                        after: None,
                        post: &|_| {},
                        out: fmt,
                    },
                );
                e.store_mxcsr(f);
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn cmp(
        &mut self,
        f: &mut InstructionSink<'_>,
        pred: u8,
        fmt: FpFmt,
        packed: bool,
        dst: u32,
        a: u32,
        b: u32,
    ) {
        let (_, v) = self.fp_locals();
        let n = if packed { lanes(fmt) } else { 1 };
        let two = |f: &mut InstructionSink<'_>, x: u32, y: u32, op: V| {
            f.local_get(x);
            f.local_get(y);
            vop(f, fmt, op);
        };
        match pred & 7 {
            0 => two(f, a, b, V::Eq),
            1 => two(f, a, b, V::Lt),
            2 => two(f, a, b, V::Le),
            3 => {
                two(f, a, a, V::Ne);
                two(f, b, b, V::Ne);
                f.v128_or();
            }
            4 => two(f, a, b, V::Ne),
            5 => {
                two(f, a, b, V::Lt);
                f.v128_not();
            }
            6 => {
                two(f, a, b, V::Le);
                f.v128_not();
            }
            _ => {
                two(f, a, a, V::Eq);
                two(f, b, b, V::Eq);
                f.v128_and();
            }
        }
        f.local_set(v);
        // No operand NaN or denormal; MXCSR isn't read.
        ordinary(f, fmt, a);
        ordinary(f, fmt, b);
        f.v128_and();
        all_lanes(f, fmt, n);
        let func = match fmt {
            FpFmt::S => FpFunc::F32Cmp,
            FpFmt::D => FpFunc::F64Cmp,
        };
        self.fast_or_slow(
            f,
            |e, f| e.set_result(f, fmt, packed, dst, a),
            |e, f| {
                e.load_mxcsr(f);
                f.local_get(a);
                f.local_set(dst);
                // The mask is 0 - result.
                let args = move |f: &mut InstructionSink<'_>, i: u8| {
                    match fmt {
                        FpFmt::S => f.i32_const(0),
                        FpFmt::D => f.i64_const(0),
                    };
                    lane_bits(f, fmt, a, i);
                    lane_bits(f, fmt, b, i);
                    f.i32_const((pred & 7) as i32);
                };
                let post = move |f: &mut InstructionSink<'_>| {
                    match fmt {
                        FpFmt::S => f.i32_sub(),
                        FpFmt::D => f.i64_extend_i32_u().i64_sub(),
                    };
                };
                e.helper_lanes(
                    f,
                    dst,
                    n,
                    &Call {
                        func,
                        args: &args,
                        after: None,
                        post: &post,
                        out: fmt,
                    },
                );
                e.store_mxcsr(f);
            },
        );
    }

    fn cvt(&mut self, f: &mut InstructionSink<'_>, cvt: FpCvt, dst: u32, a: u32, b: u32) {
        use FpFmt::{D, S};
        let (_, v) = self.fp_locals();
        // (source format, result lane format, lanes, helper, `acc` starts
        // as `a` (scalar forms) or zero (the upper half is zeroed)).
        let (from, out, n, func, merge, zero) = match cvt {
            FpCvt::Dq2Pd => {
                // Always exact.
                f.local_get(b);
                f.f64x2_convert_low_i32x4_s();
                f.local_set(dst);
                return;
            }
            FpCvt::Ss2Sd => (S, D, 1, FpFunc::F32ToF64, true, false),
            FpCvt::Ps2Pd => (S, D, 2, FpFunc::F32ToF64, false, false),
            FpCvt::Sd2Ss => (D, S, 1, FpFunc::F64ToF32, true, false),
            FpCvt::Pd2Ps => (D, S, 2, FpFunc::F64ToF32, false, true),
            FpCvt::Dq2Ps => (S, S, 4, FpFunc::I32ToF32, false, false),
            FpCvt::Ps2Dq { .. } => (S, S, 4, FpFunc::F32ToI32, false, false),
            FpCvt::Pd2Dq { .. } => (D, S, 2, FpFunc::F64ToI32, false, true),
        };
        let mut trunc_arg = None;
        match cvt {
            FpCvt::Ss2Sd | FpCvt::Ps2Pd => {
                // No NaN or denormal operand: the widening is exact.
                f.local_get(b);
                f.f64x2_promote_low_f32x4();
                f.local_set(v);
                ordinary(f, S, b);
                all_lanes(f, S, n);
            }
            FpCvt::Sd2Ss | FpCvt::Pd2Ps => {
                // The arithmetic test on the narrowed result (a denormal
                // operand narrows to 0 and fails it).
                f.local_get(b);
                f.f32x4_demote_f64x2_zero();
                f.local_set(v);
                self.load_mxcsr(f);
                self.mxcsr_is(f, MXCSR_ARITH_MASK, MXCSR_ARITH);
                safe_result(f, S, v);
                all_lanes(f, S, n);
                f.i32_and();
            }
            FpCvt::Dq2Ps => {
                // Exact when every |x| <= 2^24, else the arithmetic test.
                f.local_get(b);
                f.f32x4_convert_i32x4_s();
                f.local_set(v);
                f.local_get(b);
                f.v128_const(splat(S, 1 << 24));
                f.i32x4_add();
                f.v128_const(splat(S, 1 << 25));
                f.i32x4_le_u();
                f.i32x4_all_true();
                self.load_mxcsr(f);
                self.mxcsr_is(f, MXCSR_ARITH_MASK, MXCSR_ARITH);
                f.i32_or();
            }
            FpCvt::Ps2Dq { trunc } | FpCvt::Pd2Dq { trunc } => {
                trunc_arg = Some(trunc as i32);
                f.local_get(b);
                vop(f, from, if trunc { V::Trunc } else { V::Nearest });
                f.local_set(v);
                self.load_mxcsr(f);
                self.int_test(f, from, Width::W32, trunc, b, n);
            }
            FpCvt::Dq2Pd => unreachable!(),
        }
        let widen = matches!(cvt, FpCvt::Ss2Sd | FpCvt::Ps2Pd);
        self.fast_or_slow(
            f,
            |e, f| {
                // The fast result is in `fpv`, except for the integer
                // conversions, whose rounded lanes are.
                match cvt {
                    FpCvt::Ps2Dq { .. } => {
                        f.local_get(v);
                        f.i32x4_trunc_sat_f32x4_s();
                        f.local_set(dst);
                    }
                    FpCvt::Pd2Dq { .. } => {
                        f.local_get(v);
                        f.i32x4_trunc_sat_f64x2_s_zero();
                        f.local_set(dst);
                    }
                    _ => e.set_result(f, out, !merge, dst, a),
                }
            },
            |e, f| {
                if widen {
                    e.load_mxcsr(f);
                }
                if zero {
                    f.v128_const(0);
                } else {
                    f.local_get(if merge { a } else { b });
                }
                f.local_set(dst);
                let args = move |f: &mut InstructionSink<'_>, i: u8| lane_bits(f, from, b, i);
                e.helper_lanes(
                    f,
                    dst,
                    n,
                    &Call {
                        func,
                        args: &args,
                        after: trunc_arg,
                        post: &|_| {},
                        out,
                    },
                );
                e.store_mxcsr(f);
            },
        );
    }

    /// Pushes the fast-path test of a float -> int conversion whose
    /// truncated or rounded lanes are in the `fpv` local and MXCSR in its
    /// local: lanes 0..n in range, and either integral already or PE set
    /// with PM masked; rounding also needs RC = nearest.
    fn int_test(
        &self,
        f: &mut InstructionSink<'_>,
        fmt: FpFmt,
        width: Width,
        trunc: bool,
        x: u32,
        n: u8,
    ) {
        let (_, v) = self.fp_locals();
        let (lo, hi) = int_range(fmt, width);
        f.local_get(v);
        f.v128_const(splat(fmt, lo));
        vop(f, fmt, V::Ge);
        f.local_get(v);
        f.v128_const(splat(fmt, hi));
        vop(f, fmt, V::Lt);
        f.v128_and();
        all_lanes(f, fmt, n);
        f.local_get(v);
        f.local_get(x);
        vop(f, fmt, V::Eq);
        all_lanes(f, fmt, n);
        self.mxcsr_is(f, MXCSR_PM_PE, MXCSR_PM_PE);
        f.i32_or();
        f.i32_and();
        if !trunc {
            self.mxcsr_is(f, MXCSR_RC, 0);
            f.i32_and();
        }
    }

    fn cvt_from_int(
        &mut self,
        f: &mut InstructionSink<'_>,
        fmt: FpFmt,
        width: Width,
        dst: u32,
        a: u32,
        src: Value,
    ) {
        let convert = |e: &mut Self, f: &mut InstructionSink<'_>| {
            f.local_get(a);
            e.value(f, src);
            match fmt {
                FpFmt::S => f.f32_convert_i64_s().f32x4_replace_lane(0),
                FpFmt::D => f.f64_convert_i64_s().f64x2_replace_lane(0),
            };
            f.local_set(dst);
        };
        if fmt == FpFmt::D && width != Width::W64 {
            // cvtsi2sd r/m32 is always exact.
            convert(self, f);
            return;
        }
        // Exact when |x| <= 2^24 (f32) or 2^53 (f64), else the arithmetic
        // test.
        let limit: i64 = match fmt {
            FpFmt::S => 1 << 24,
            FpFmt::D => 1 << 53,
        };
        self.value(f, src);
        f.i64_const(limit);
        f.i64_add();
        f.i64_const(2 * limit);
        f.i64_le_u();
        self.load_mxcsr(f);
        self.mxcsr_is(f, MXCSR_ARITH_MASK, MXCSR_ARITH);
        f.i32_or();
        let func = match (fmt, width) {
            (FpFmt::S, Width::W64) => FpFunc::I64ToF32,
            (FpFmt::S, _) => FpFunc::I32ToF32,
            _ => FpFunc::I64ToF64,
        };
        self.fast_or_slow(f, convert, |e, f| {
            let (mx, _) = e.fp_locals();
            f.local_get(a);
            e.value(f, src);
            if width != Width::W64 {
                f.i32_wrap_i64();
            }
            f.local_get(mx);
            f.call(e.fp.index(func));
            replace_lane(f, fmt, 0);
            f.local_set(dst);
            e.check_status(f);
            e.store_mxcsr(f);
        });
    }

    fn cvt_to_int(
        &mut self,
        f: &mut InstructionSink<'_>,
        fmt: FpFmt,
        width: Width,
        trunc: bool,
        dst: u32,
        b: u32,
    ) {
        let (mx, v) = self.fp_locals();
        f.local_get(b);
        vop(f, fmt, if trunc { V::Trunc } else { V::Nearest });
        f.local_set(v);
        self.load_mxcsr(f);
        self.int_test(f, fmt, width, trunc, b, 1);
        let wide = width == Width::W64;
        let func = match (fmt, wide) {
            (FpFmt::S, false) => FpFunc::F32ToI32,
            (FpFmt::S, true) => FpFunc::F32ToI64,
            (FpFmt::D, false) => FpFunc::F64ToI32,
            (FpFmt::D, true) => FpFunc::F64ToI64,
        };
        self.fast_or_slow(
            f,
            |_, f| {
                f.local_get(v);
                match (fmt, wide) {
                    (FpFmt::S, false) => f.f32x4_extract_lane(0).i32_trunc_f32_s(),
                    (FpFmt::S, true) => f.f32x4_extract_lane(0).i64_trunc_f32_s(),
                    (FpFmt::D, false) => f.f64x2_extract_lane(0).i32_trunc_f64_s(),
                    (FpFmt::D, true) => f.f64x2_extract_lane(0).i64_trunc_f64_s(),
                };
                if !wide {
                    f.i64_extend_i32_u();
                }
                f.local_set(dst);
            },
            |e, f| {
                lane_bits(f, fmt, b, 0);
                f.local_get(mx);
                f.i32_const(trunc as i32);
                f.call(e.fp.index(func));
                if !wide {
                    f.i64_extend_i32_u();
                }
                f.local_set(dst);
                e.check_status(f);
                e.store_mxcsr(f);
            },
        );
    }

    fn comi(
        &mut self,
        f: &mut InstructionSink<'_>,
        fmt: FpFmt,
        signaling: bool,
        dst: u32,
        a: u32,
        b: u32,
    ) {
        // No operand NaN or denormal; MXCSR isn't read.
        ordinary(f, fmt, a);
        ordinary(f, fmt, b);
        f.v128_and();
        all_lanes(f, fmt, 1);
        let func = match fmt {
            FpFmt::S => FpFunc::F32Comi,
            FpFmt::D => FpFunc::F64Comi,
        };
        let scalar = move |f: &mut InstructionSink<'_>, x: u32| {
            f.local_get(x);
            match fmt {
                FpFmt::S => f.f32x4_extract_lane(0),
                FpFmt::D => f.f64x2_extract_lane(0),
            };
        };
        self.fast_or_slow(
            f,
            |_, f| {
                // less: CF; equal: ZF; greater: none.
                f.i64_const(CF);
                f.i64_const(ZF);
                f.i64_const(0);
                scalar(f, a);
                scalar(f, b);
                match fmt {
                    FpFmt::S => f.f32_eq(),
                    FpFmt::D => f.f64_eq(),
                };
                f.select();
                scalar(f, a);
                scalar(f, b);
                match fmt {
                    FpFmt::S => f.f32_lt(),
                    FpFmt::D => f.f64_lt(),
                };
                f.select();
                f.local_set(dst);
            },
            |e, f| {
                let (mx, _) = e.fp_locals();
                e.load_mxcsr(f);
                lane_bits(f, fmt, a, 0);
                lane_bits(f, fmt, b, 0);
                f.i32_const(signaling as i32);
                f.local_get(mx);
                f.call(e.fp.index(func));
                f.i64_extend_i32_u();
                f.local_set(dst);
                e.check_status(f);
                e.store_mxcsr(f);
            },
        );
    }
}

/// Pushes the lanes of `local` that are positive normal numbers
/// (`MIN_NORMAL <= x < infinity`): sqrt's fast-path operands.
fn safe_operand(f: &mut InstructionSink<'_>, fmt: FpFmt, local: u32) {
    f.local_get(local);
    f.v128_const(splat(fmt, min_normal(fmt)));
    vop(f, fmt, V::Ge);
    f.local_get(local);
    f.v128_const(splat(fmt, infinity(fmt)));
    vop(f, fmt, V::Lt);
    f.v128_and();
}

fn arith_func(op: FpArith, fmt: FpFmt) -> FpFunc {
    use FpFunc as F;
    match (op, fmt) {
        (FpArith::Add, FpFmt::S) => F::F32Add,
        (FpArith::Sub, FpFmt::S) => F::F32Sub,
        (FpArith::Mul, FpFmt::S) => F::F32Mul,
        (FpArith::Div, FpFmt::S) => F::F32Div,
        (FpArith::Min, FpFmt::S) => F::F32Min,
        (FpArith::Max, FpFmt::S) => F::F32Max,
        (FpArith::Sqrt, FpFmt::S) => F::F32Sqrt,
        (FpArith::Add, FpFmt::D) => F::F64Add,
        (FpArith::Sub, FpFmt::D) => F::F64Sub,
        (FpArith::Mul, FpFmt::D) => F::F64Mul,
        (FpArith::Div, FpFmt::D) => F::F64Div,
        (FpArith::Min, FpFmt::D) => F::F64Min,
        (FpArith::Max, FpFmt::D) => F::F64Max,
        (FpArith::Sqrt, FpFmt::D) => F::F64Sqrt,
    }
}
