//! Lazy status flags: conditions from a statically known record kind,
//! from the dynamic record (through `flags`), and the shared `flags`
//! function (function 2) that materializes a record.

use wasm_encoder::{BlockType, Function, InstructionSink, ValType};

use super::{ChunkEmitter, F_FLAGS, L_CC_A, L_CC_B, L_CC_R, L_SCRATCH};
use crate::abi;
use crate::ir::{Cond, FlagOp, Width};

impl ChunkEmitter<'_> {
    /// Pushes the materialized status flags (i64) of the current record.
    pub(super) fn materialize_flags(&self, f: &mut InstructionSink<'_>) {
        f.local_get(self.l_cc_op);
        f.local_get(L_CC_A);
        f.local_get(L_CC_B);
        f.local_get(L_CC_R);
        f.call(F_FLAGS);
    }

    /// Pushes `cond` as an i32 (0 or 1).
    pub(super) fn cond(&self, f: &mut InstructionSink<'_>, cond: Cond) {
        let base = Cond::from_index(cond as u32 & !1);
        match self.known_cc {
            Some((op, w)) => self.static_cond(f, op, w, base),
            None => {
                self.materialize_flags(f);
                f.local_set(L_SCRATCH);
                dynamic_cond(f, base);
            }
        }
        if cond.is_negated() {
            f.i32_eqz();
        }
    }

    pub(super) fn zf(&self, f: &mut InstructionSink<'_>) {
        f.local_get(L_CC_R);
        f.i64_eqz();
    }

    pub(super) fn sf(&self, f: &mut InstructionSink<'_>, w: Width) {
        f.local_get(L_CC_R);
        f.i64_const(w.sign_bit() as i64);
        f.i64_ge_u();
    }

    pub(super) fn cf(&self, f: &mut InstructionSink<'_>, op: FlagOp) {
        match op {
            FlagOp::Add => {
                f.local_get(L_CC_R);
                f.local_get(L_CC_A);
            }
            FlagOp::Sub => {
                f.local_get(L_CC_A);
                f.local_get(L_CC_B);
            }
        }
        f.i64_lt_u();
    }

    pub(super) fn of(&self, f: &mut InstructionSink<'_>, op: FlagOp, w: Width) {
        match op {
            // (a ^ r) & (b ^ r)
            FlagOp::Add => {
                f.local_get(L_CC_A);
                f.local_get(L_CC_R);
                f.i64_xor();
                f.local_get(L_CC_B);
                f.local_get(L_CC_R);
                f.i64_xor();
            }
            // (a ^ b) & (a ^ r)
            FlagOp::Sub => {
                f.local_get(L_CC_A);
                f.local_get(L_CC_B);
                f.i64_xor();
                f.local_get(L_CC_A);
                f.local_get(L_CC_R);
                f.i64_xor();
            }
        }
        f.i64_and();
        f.i64_const(w.sign_bit() as i64);
        f.i64_and();
        f.i64_const(0);
        f.i64_ne();
    }

    pub(super) fn pf(&self, f: &mut InstructionSink<'_>) {
        f.local_get(L_CC_R);
        f.i64_const(0xff);
        f.i64_and();
        f.i64_popcnt();
        f.i64_const(1);
        f.i64_and();
        f.i64_eqz();
    }

    pub(super) fn sext(f: &mut InstructionSink<'_>, local: u32, w: Width) {
        f.local_get(local);
        match w {
            Width::W8 => f.i64_extend8_s(),
            Width::W16 => f.i64_extend16_s(),
            Width::W32 => f.i64_extend32_s(),
            Width::W64 => f,
        };
    }

    /// A non-negated condition for a statically known record kind.
    pub(super) fn static_cond(
        &self,
        f: &mut InstructionSink<'_>,
        op: FlagOp,
        w: Width,
        base: Cond,
    ) {
        match (op, base) {
            (_, Cond::O) => self.of(f, op, w),
            (_, Cond::B) => self.cf(f, op),
            (_, Cond::E) => self.zf(f),
            (FlagOp::Sub, Cond::Be) => {
                f.local_get(L_CC_A);
                f.local_get(L_CC_B);
                f.i64_le_u();
            }
            (_, Cond::Be) => {
                self.cf(f, op);
                self.zf(f);
                f.i32_or();
            }
            (_, Cond::S) => self.sf(f, w),
            (_, Cond::P) => self.pf(f),
            (FlagOp::Sub, Cond::L) => {
                Self::sext(f, L_CC_A, w);
                Self::sext(f, L_CC_B, w);
                f.i64_lt_s();
            }
            (FlagOp::Sub, Cond::Le) => {
                Self::sext(f, L_CC_A, w);
                Self::sext(f, L_CC_B, w);
                f.i64_le_s();
            }
            (_, Cond::L) => {
                self.sf(f, w);
                self.of(f, op, w);
                f.i32_ne();
            }
            (_, Cond::Le) => {
                self.zf(f);
                self.sf(f, w);
                self.of(f, op, w);
                f.i32_ne();
                f.i32_or();
            }
            _ => unreachable!("negated condition"),
        }
    }
}

/// Pushes a non-negated condition from materialized flags in `L_SCRATCH`.
fn dynamic_cond(f: &mut InstructionSink<'_>, base: Cond) {
    use abi::flags::*;
    let bit = |f: &mut InstructionSink<'_>, mask: u64| {
        f.local_get(L_SCRATCH);
        f.i64_const(mask as i64);
        f.i64_and();
        f.i64_const(0);
        f.i64_ne();
    };
    match base {
        Cond::O => bit(f, OF),
        Cond::B => bit(f, CF),
        Cond::E => bit(f, ZF),
        Cond::Be => bit(f, CF | ZF),
        Cond::S => bit(f, SF),
        Cond::P => bit(f, PF),
        Cond::L => {
            bit(f, SF);
            bit(f, OF);
            f.i32_ne();
        }
        Cond::Le => {
            bit(f, ZF);
            bit(f, SF);
            bit(f, OF);
            f.i32_ne();
            f.i32_or();
        }
        _ => unreachable!("negated condition"),
    }
}

/// `flags(op: i32, a: i64, b: i64, r: i64) -> i64`: the status flags of a
/// lazy record (operands zero-extended to the record's width).
pub(super) fn emit_flags() -> Function {
    use abi::flags::*;
    const OP: u32 = 0;
    const A: u32 = 1;
    const B: u32 = 2;
    const R: u32 = 3;
    const SIGN: u32 = 4;
    const CF_OF: u32 = 5;
    let mut func = Function::new([(2, ValType::I64)]);
    let mut f = func.instructions();
    // Eager: the flags themselves are in `a`.
    f.local_get(OP);
    f.i32_eqz();
    f.if_(BlockType::Empty);
    f.local_get(A);
    f.i64_const(STATUS as i64);
    f.i64_and();
    f.return_();
    f.end();
    // sign = 1 << (8 << width) - 1, width = (op - 1) & 3
    f.i64_const(1);
    f.i64_const(8);
    f.local_get(OP);
    f.i32_const(1);
    f.i32_sub();
    f.i32_const(3);
    f.i32_and();
    f.i64_extend_i32_u();
    f.i64_shl();
    f.i64_const(1);
    f.i64_sub();
    f.i64_shl();
    f.local_set(SIGN);
    // kind = (op - 1) >> 2: 0 = add, 1 = sub. CF | OF:
    f.local_get(OP);
    f.i32_const(1);
    f.i32_sub();
    f.i32_const(2);
    f.i32_shr_u();
    f.if_(BlockType::Result(ValType::I64));
    {
        // sub: CF = a < b, OF = (a ^ b) & (a ^ r)
        f.local_get(A);
        f.local_get(B);
        f.i64_lt_u();
        f.i64_extend_i32_u();
        f.local_get(A);
        f.local_get(B);
        f.i64_xor();
        f.local_get(A);
        f.local_get(R);
        f.i64_xor();
        f.i64_and();
        f.local_get(SIGN);
        f.i64_and();
        f.i64_eqz();
        f.i32_eqz();
        f.i64_extend_i32_u();
        f.i64_const(11);
        f.i64_shl();
        f.i64_or();
    }
    f.else_();
    {
        // add: CF = r < a, OF = (a ^ r) & (b ^ r)
        f.local_get(R);
        f.local_get(A);
        f.i64_lt_u();
        f.i64_extend_i32_u();
        f.local_get(A);
        f.local_get(R);
        f.i64_xor();
        f.local_get(B);
        f.local_get(R);
        f.i64_xor();
        f.i64_and();
        f.local_get(SIGN);
        f.i64_and();
        f.i64_eqz();
        f.i32_eqz();
        f.i64_extend_i32_u();
        f.i64_const(11);
        f.i64_shl();
        f.i64_or();
    }
    f.end();
    f.local_set(CF_OF);
    // PF
    f.local_get(R);
    f.i64_const(0xff);
    f.i64_and();
    f.i64_popcnt();
    f.i64_const(1);
    f.i64_and();
    f.i64_const(1);
    f.i64_xor();
    f.i64_const(2);
    f.i64_shl();
    // AF
    f.local_get(A);
    f.local_get(B);
    f.i64_xor();
    f.local_get(R);
    f.i64_xor();
    f.i64_const(AF as i64);
    f.i64_and();
    f.i64_or();
    // ZF
    f.local_get(R);
    f.i64_eqz();
    f.i64_extend_i32_u();
    f.i64_const(6);
    f.i64_shl();
    f.i64_or();
    // SF
    f.local_get(R);
    f.local_get(SIGN);
    f.i64_and();
    f.i64_eqz();
    f.i32_eqz();
    f.i64_extend_i32_u();
    f.i64_const(7);
    f.i64_shl();
    f.i64_or();
    f.local_get(CF_OF);
    f.i64_or();
    f.end();
    func
}
