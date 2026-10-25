//! Vector ops on the XMM registers in the state block, with wasm SIMD
//! (`docs/design/translator.md`, "SSE and SSE2 without floating-point
//! arithmetic").

use wasm_encoder::{InstructionSink, MemArg, ValType};

use super::{ChunkEmitter, L_SCRATCH, P_STATE, mem};
use crate::ir::{Lanes, Op, VBinOp, VExtractOp, VLoc, VShiftOp, VWidth, Value};

/// Bytes 2-3 of every 32-bit lane of two vectors: the high halves of
/// 16x16->32-bit products.
const HIGH_HALVES: [u8; 16] = [2, 3, 6, 7, 10, 11, 14, 15, 18, 19, 22, 23, 26, 27, 30, 31];
/// 32-bit lanes 0 and 2, twice.
const EVEN_DWORDS: [u8; 16] = [0, 1, 2, 3, 8, 9, 10, 11, 0, 1, 2, 3, 8, 9, 10, 11];

impl ChunkEmitter<'_> {
    /// Pushes the address of `loc` and returns the memarg to use with it.
    fn vloc(&mut self, f: &mut InstructionSink<'_>, loc: VLoc) -> MemArg {
        match loc {
            VLoc::State(o) => {
                f.local_get(P_STATE);
                mem(o as u64)
            }
            VLoc::Mem(a) => {
                self.value(f, a);
                mem(0)
            }
        }
    }

    pub(super) fn vector_op(&mut self, f: &mut InstructionSink<'_>, op: &Op) {
        match *op {
            Op::VLoad { dst, width, loc } => {
                let m = self.vloc(f, loc);
                match width {
                    VWidth::W32 => f.v128_load32_zero(m),
                    VWidth::W64 => f.v128_load64_zero(m),
                    VWidth::W128 => f.v128_load(m),
                };
                f.local_set(self.vtemp(dst));
            }
            Op::VStore { loc, width, src } => {
                let m = self.vloc(f, loc);
                f.local_get(self.vtemp(src));
                match width {
                    VWidth::W32 => f.v128_store32_lane(m, 0),
                    VWidth::W64 => f.v128_store64_lane(m, 0),
                    VWidth::W128 => f.v128_store(m),
                };
            }
            Op::VBin { dst, op, a, b } => {
                let (a, b) = (self.vtemp(a), self.vtemp(b));
                vbin(f, op, a, b);
                f.local_set(self.vtemp(dst));
            }
            Op::VShuffle { dst, a, b, lanes } => {
                f.local_get(self.vtemp(a));
                f.local_get(self.vtemp(b));
                f.i8x16_shuffle(lanes);
                f.local_set(self.vtemp(dst));
            }
            Op::VShift { dst, op, a, count } => {
                let a = self.vtemp(a);
                self.vshift(f, op, a, count);
                f.local_set(self.vtemp(dst));
            }
            Op::VConst { dst, value } => {
                f.v128_const(value as i128);
                f.local_set(self.vtemp(dst));
            }
            Op::VFromGpr { dst, value } => {
                f.v128_const(0);
                self.value(f, value);
                f.i64x2_replace_lane(0);
                f.local_set(self.vtemp(dst));
            }
            Op::VExtract { dst, op, src } => {
                f.local_get(self.vtemp(src));
                match op {
                    VExtractOp::Lane64(l) => {
                        f.i64x2_extract_lane(l & 1);
                    }
                    VExtractOp::Bitmask(lanes) => {
                        match lanes {
                            Lanes::B => f.i8x16_bitmask(),
                            Lanes::W => f.i16x8_bitmask(),
                            Lanes::D => f.i32x4_bitmask(),
                            Lanes::Q => f.i64x2_bitmask(),
                        };
                        f.i64_extend_i32_u();
                    }
                }
                self.set_temp(f, dst);
            }
            _ => unreachable!("not a vector op"),
        }
    }

    /// Pushes `a` shifted with x86 semantics: an unsigned 64-bit count;
    /// logical shifts by the lane width or more give 0, arithmetic ones use
    /// width - 1.
    fn vshift(&mut self, f: &mut InstructionSink<'_>, op: VShiftOp, a: u32, count: Value) {
        let lanes = match op {
            VShiftOp::Shl(l) | VShiftOp::ShrU(l) | VShiftOp::ShrS(l) => l,
        };
        let bits = lanes.bits() as u64;
        let shift = |f: &mut InstructionSink<'_>| {
            match (op, lanes) {
                (VShiftOp::Shl(_), Lanes::B) => f.i8x16_shl(),
                (VShiftOp::Shl(_), Lanes::W) => f.i16x8_shl(),
                (VShiftOp::Shl(_), Lanes::D) => f.i32x4_shl(),
                (VShiftOp::Shl(_), Lanes::Q) => f.i64x2_shl(),
                (VShiftOp::ShrU(_), Lanes::B) => f.i8x16_shr_u(),
                (VShiftOp::ShrU(_), Lanes::W) => f.i16x8_shr_u(),
                (VShiftOp::ShrU(_), Lanes::D) => f.i32x4_shr_u(),
                (VShiftOp::ShrU(_), Lanes::Q) => f.i64x2_shr_u(),
                (VShiftOp::ShrS(_), Lanes::B) => f.i8x16_shr_s(),
                (VShiftOp::ShrS(_), Lanes::W) => f.i16x8_shr_s(),
                (VShiftOp::ShrS(_), Lanes::D) => f.i32x4_shr_s(),
                (VShiftOp::ShrS(_), Lanes::Q) => f.i64x2_shr_s(),
            };
        };
        let arith = matches!(op, VShiftOp::ShrS(_));
        if let Value::Imm(c) = count {
            if arith {
                f.local_get(a);
                f.i32_const(c.min(bits - 1) as i32);
                shift(f);
            } else if c >= bits {
                f.v128_const(0);
            } else {
                f.local_get(a);
                f.i32_const(c as i32);
                shift(f);
            }
            return;
        }
        self.value(f, count);
        f.local_set(L_SCRATCH);
        if arith {
            f.local_get(a);
            // min_u(count, bits - 1)
            f.local_get(L_SCRATCH);
            f.i64_const(bits as i64 - 1);
            f.local_get(L_SCRATCH);
            f.i64_const(bits as i64 - 1);
            f.i64_lt_u();
            f.select();
            f.i32_wrap_i64();
            shift(f);
        } else {
            f.local_get(a);
            f.local_get(L_SCRATCH);
            f.i32_wrap_i64();
            shift(f);
            f.v128_const(0);
            f.local_get(L_SCRATCH);
            f.i64_const(bits as i64);
            f.i64_lt_u();
            f.typed_select(ValType::V128);
        }
    }
}

/// Pushes `a op b` (x86 operand order: `a` is the destination).
fn vbin(f: &mut InstructionSink<'_>, op: VBinOp, a: u32, b: u32) {
    use Lanes::{B, D, Q, W};
    let ab = |f: &mut InstructionSink<'_>| {
        f.local_get(a);
        f.local_get(b);
    };
    match op {
        VBinOp::AndNot => {
            // !a & b = andnot(b, a)
            f.local_get(b);
            f.local_get(a);
            f.v128_andnot();
            return;
        }
        VBinOp::MulHiS16 | VBinOp::MulHiU16 => {
            ab(f);
            if op == VBinOp::MulHiS16 {
                f.i32x4_extmul_low_i16x8_s();
            } else {
                f.i32x4_extmul_low_i16x8_u();
            }
            ab(f);
            if op == VBinOp::MulHiS16 {
                f.i32x4_extmul_high_i16x8_s();
            } else {
                f.i32x4_extmul_high_i16x8_u();
            }
            f.i8x16_shuffle(HIGH_HALVES);
            return;
        }
        VBinOp::MulU32 => {
            f.local_get(a);
            f.local_get(a);
            f.i8x16_shuffle(EVEN_DWORDS);
            f.local_get(b);
            f.local_get(b);
            f.i8x16_shuffle(EVEN_DWORDS);
            f.i64x2_extmul_low_i32x4_u();
            return;
        }
        VBinOp::SadBw => {
            ab(f);
            f.i8x16_max_u();
            ab(f);
            f.i8x16_min_u();
            f.i8x16_sub();
            f.i16x8_extadd_pairwise_i8x16_u();
            f.i32x4_extadd_pairwise_i16x8_u();
            // Lane p0 + p1 * 2^32 times 2^32 + 1, shifted down by 32, is
            // p0 + p1 (both below 2^16).
            f.v128_const(0x0000_0001_0000_0001_0000_0001_0000_0001);
            f.i64x2_mul();
            f.i32_const(32);
            f.i64x2_shr_u();
            return;
        }
        _ => {}
    }
    ab(f);
    match op {
        VBinOp::And => f.v128_and(),
        VBinOp::Or => f.v128_or(),
        VBinOp::Xor => f.v128_xor(),
        VBinOp::Add(B) => f.i8x16_add(),
        VBinOp::Add(W) => f.i16x8_add(),
        VBinOp::Add(D) => f.i32x4_add(),
        VBinOp::Add(Q) => f.i64x2_add(),
        VBinOp::Sub(B) => f.i8x16_sub(),
        VBinOp::Sub(W) => f.i16x8_sub(),
        VBinOp::Sub(D) => f.i32x4_sub(),
        VBinOp::Sub(Q) => f.i64x2_sub(),
        VBinOp::AddSatS(B) => f.i8x16_add_sat_s(),
        VBinOp::AddSatS(_) => f.i16x8_add_sat_s(),
        VBinOp::AddSatU(B) => f.i8x16_add_sat_u(),
        VBinOp::AddSatU(_) => f.i16x8_add_sat_u(),
        VBinOp::SubSatS(B) => f.i8x16_sub_sat_s(),
        VBinOp::SubSatS(_) => f.i16x8_sub_sat_s(),
        VBinOp::SubSatU(B) => f.i8x16_sub_sat_u(),
        VBinOp::SubSatU(_) => f.i16x8_sub_sat_u(),
        VBinOp::Eq(B) => f.i8x16_eq(),
        VBinOp::Eq(W) => f.i16x8_eq(),
        VBinOp::Eq(D) => f.i32x4_eq(),
        VBinOp::Eq(Q) => f.i64x2_eq(),
        VBinOp::GtS(B) => f.i8x16_gt_s(),
        VBinOp::GtS(W) => f.i16x8_gt_s(),
        VBinOp::GtS(D) => f.i32x4_gt_s(),
        VBinOp::GtS(Q) => f.i64x2_gt_s(),
        VBinOp::MinU(B) => f.i8x16_min_u(),
        VBinOp::MinU(W) => f.i16x8_min_u(),
        VBinOp::MinU(_) => f.i32x4_min_u(),
        VBinOp::MaxU(B) => f.i8x16_max_u(),
        VBinOp::MaxU(W) => f.i16x8_max_u(),
        VBinOp::MaxU(_) => f.i32x4_max_u(),
        VBinOp::MinS(B) => f.i8x16_min_s(),
        VBinOp::MinS(W) => f.i16x8_min_s(),
        VBinOp::MinS(_) => f.i32x4_min_s(),
        VBinOp::MaxS(B) => f.i8x16_max_s(),
        VBinOp::MaxS(W) => f.i16x8_max_s(),
        VBinOp::MaxS(_) => f.i32x4_max_s(),
        VBinOp::AvgU(B) => f.i8x16_avgr_u(),
        VBinOp::AvgU(_) => f.i16x8_avgr_u(),
        VBinOp::MulLo16 => f.i16x8_mul(),
        VBinOp::MaddWd => f.i32x4_dot_i16x8_s(),
        VBinOp::PackSsWb => f.i8x16_narrow_i16x8_s(),
        VBinOp::PackUsWb => f.i8x16_narrow_i16x8_u(),
        VBinOp::PackSsDw => f.i16x8_narrow_i32x4_s(),
        VBinOp::AndNot | VBinOp::MulHiS16 | VBinOp::MulHiU16 | VBinOp::MulU32 | VBinOp::SadBw => {
            unreachable!()
        }
    };
}
