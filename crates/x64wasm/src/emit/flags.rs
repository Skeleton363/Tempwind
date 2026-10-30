//! Lazy status flags: conditions from a statically known record kind,
//! from the dynamic record (through `flags`), the shared `flags` function
//! (function 2) that materializes a record, and the per-kind CF and OF
//! formulas both use (`docs/design/translator.md`, "Lazy flag record").

use wasm_encoder::{BlockType, Function, InstructionSink, ValType};

use super::{ChunkEmitter, F_FLAGS, L_CC_A, L_CC_B, L_CC_R, L_SCRATCH};
use crate::abi;
use crate::ir::{Cond, FlagOp, Width};

/// What is statically known about the current record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Known {
    Lazy(FlagOp, Width),
    /// Materialized in `cc_a` (`cc_op` = 0).
    Eager,
}

/// Where a record's operands and its width's sign bit are.
#[derive(Clone, Copy)]
struct Rec {
    a: u32,
    b: u32,
    r: u32,
    sign: Sign,
}

#[derive(Clone, Copy)]
enum Sign {
    Const(u64),
    Local(u32),
}

impl Rec {
    fn sign(&self, f: &mut InstructionSink<'_>) {
        match self.sign {
            Sign::Const(s) => f.i64_const(s as i64),
            Sign::Local(l) => f.local_get(l),
        };
    }

    /// Replaces the i64 on the stack by its sign bit (i32 0 or 1).
    fn msb(&self, f: &mut InstructionSink<'_>) {
        self.sign(f);
        f.i64_and();
        f.i64_const(0);
        f.i64_ne();
    }

    /// Pushes `b - 1` as the i64 shift count.
    fn count_minus_one(&self, f: &mut InstructionSink<'_>) {
        f.local_get(self.b);
        f.i64_const(1);
        f.i64_sub();
    }

    /// Pushes CF of a record of kind `op` (i32 0 or 1).
    fn cf(&self, f: &mut InstructionSink<'_>, op: FlagOp) {
        let (a, b, r) = (self.a, self.b, self.r);
        match op {
            FlagOp::Add => {
                f.local_get(r);
                f.local_get(a);
                f.i64_lt_u();
            }
            FlagOp::Sub => {
                f.local_get(a);
                f.local_get(b);
                f.i64_lt_u();
            }
            FlagOp::Logic => {
                f.i32_const(0);
            }
            // msb((a & b) | ((a ^ b) & !r))
            FlagOp::Adc => {
                f.local_get(a);
                f.local_get(b);
                f.i64_and();
                f.local_get(a);
                f.local_get(b);
                f.i64_xor();
                f.local_get(r);
                f.i64_const(-1);
                f.i64_xor();
                f.i64_and();
                f.i64_or();
                self.msb(f);
            }
            // msb((!a & b) | (!(a ^ b) & r))
            FlagOp::Sbb => {
                f.local_get(a);
                f.i64_const(-1);
                f.i64_xor();
                f.local_get(b);
                f.i64_and();
                f.local_get(a);
                f.local_get(b);
                f.i64_xor();
                f.i64_const(-1);
                f.i64_xor();
                f.local_get(r);
                f.i64_and();
                f.i64_or();
                self.msb(f);
            }
            FlagOp::Inc | FlagOp::Dec | FlagOp::Mul => {
                let l = if op == FlagOp::Mul { a } else { b };
                f.local_get(l);
                f.i64_eqz();
                f.i32_eqz();
            }
            FlagOp::Shl => {
                f.local_get(a);
                self.count_minus_one(f);
                f.i64_shl();
                self.msb(f);
            }
            FlagOp::Shr | FlagOp::Sar => {
                f.local_get(a);
                if op == FlagOp::Sar {
                    // Sign-extend from the width: (a ^ sign) - sign.
                    self.sign(f);
                    f.i64_xor();
                    self.sign(f);
                    f.i64_sub();
                }
                self.count_minus_one(f);
                if op == FlagOp::Sar {
                    f.i64_shr_s();
                } else {
                    f.i64_shr_u();
                }
                f.i32_wrap_i64();
                f.i32_const(1);
                f.i32_and();
            }
        }
    }

    /// Pushes OF of a record of kind `op` (i32 0 or 1).
    fn of(&self, f: &mut InstructionSink<'_>, op: FlagOp) {
        let (a, b, r) = (self.a, self.b, self.r);
        match op {
            // (a ^ r) & (b ^ r)
            FlagOp::Add | FlagOp::Adc => {
                f.local_get(a);
                f.local_get(r);
                f.i64_xor();
                f.local_get(b);
                f.local_get(r);
                f.i64_xor();
                f.i64_and();
                self.msb(f);
            }
            // (a ^ b) & (a ^ r)
            FlagOp::Sub | FlagOp::Sbb => {
                f.local_get(a);
                f.local_get(b);
                f.i64_xor();
                f.local_get(a);
                f.local_get(r);
                f.i64_xor();
                f.i64_and();
                self.msb(f);
            }
            FlagOp::Logic => {
                f.i32_const(0);
            }
            FlagOp::Inc | FlagOp::Dec => {
                f.local_get(if op == FlagOp::Inc { r } else { a });
                self.sign(f);
                f.i64_eq();
            }
            FlagOp::Shl | FlagOp::Shr | FlagOp::Sar => {
                f.local_get(a);
                f.local_get(r);
                f.i64_xor();
                self.msb(f);
            }
            FlagOp::Mul => {
                f.local_get(a);
                f.i64_eqz();
                f.i32_eqz();
            }
        }
    }
}

impl ChunkEmitter<'_> {
    fn rec(w: Width) -> Rec {
        Rec {
            a: L_CC_A,
            b: L_CC_B,
            r: L_CC_R,
            sign: Sign::Const(w.sign_bit()),
        }
    }

    /// Pushes the materialized status flags (i64) of the current record.
    pub(super) fn materialize_flags(&self, f: &mut InstructionSink<'_>) {
        f.local_get(self.l_cc_op);
        f.local_get(L_CC_A);
        f.local_get(L_CC_B);
        f.local_get(L_CC_R);
        f.call(F_FLAGS);
    }

    /// Pushes the current flags (i64; bits other than the status flags may
    /// be set when the record is eager).
    pub(super) fn current_flags(&self, f: &mut InstructionSink<'_>) {
        if self.known_cc == Some(Known::Eager) {
            f.local_get(L_CC_A);
        } else {
            self.materialize_flags(f);
        }
    }

    /// Pushes the current CF as an i32 (0 or 1).
    pub(super) fn current_cf(&self, f: &mut InstructionSink<'_>) {
        match self.known_cc {
            Some(Known::Lazy(op, w)) => Self::rec(w).cf(f, op),
            _ => {
                self.current_flags(f);
                f.i32_wrap_i64();
                f.i32_const(abi::flags::CF as i32);
                f.i32_and();
            }
        }
    }

    /// Pushes `cond` as an i32 (0 or 1).
    pub(super) fn cond(&self, f: &mut InstructionSink<'_>, cond: Cond) {
        let base = Cond::from_index(cond as u32 & !1);
        match self.known_cc {
            Some(Known::Lazy(op, w)) => Self::static_cond(f, op, w, base),
            Some(Known::Eager) => {
                f.local_get(L_CC_A);
                f.local_set(L_SCRATCH);
                dynamic_cond(f, base);
            }
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

    pub(super) fn zf(f: &mut InstructionSink<'_>) {
        f.local_get(L_CC_R);
        f.i64_eqz();
    }

    pub(super) fn sf(f: &mut InstructionSink<'_>, w: Width) {
        f.local_get(L_CC_R);
        f.i64_const(w.sign_bit() as i64);
        f.i64_ge_u();
    }

    pub(super) fn pf(f: &mut InstructionSink<'_>) {
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
    pub(super) fn static_cond(f: &mut InstructionSink<'_>, op: FlagOp, w: Width, base: Cond) {
        let rec = Self::rec(w);
        match (op, base) {
            (_, Cond::O) => rec.of(f, op),
            (_, Cond::B) => rec.cf(f, op),
            (_, Cond::E) => Self::zf(f),
            (FlagOp::Sub, Cond::Be) => {
                f.local_get(L_CC_A);
                f.local_get(L_CC_B);
                f.i64_le_u();
            }
            (FlagOp::Logic, Cond::Be) => Self::zf(f),
            (_, Cond::Be) => {
                rec.cf(f, op);
                Self::zf(f);
                f.i32_or();
            }
            (_, Cond::S) => Self::sf(f, w),
            (_, Cond::P) => Self::pf(f),
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
            (FlagOp::Logic, Cond::L) => Self::sf(f, w),
            (FlagOp::Logic, Cond::Le) => {
                Self::zf(f);
                Self::sf(f, w);
                f.i32_or();
            }
            (_, Cond::L) => {
                Self::sf(f, w);
                rec.of(f, op);
                f.i32_ne();
            }
            (_, Cond::Le) => {
                Self::zf(f);
                Self::sf(f, w);
                rec.of(f, op);
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
    // CF | OF by kind = (op - 1) >> 2.
    let rec = Rec {
        a: A,
        b: B,
        r: R,
        sign: Sign::Local(SIGN),
    };
    let n = FlagOp::ALL.len() as u32;
    f.block(BlockType::Empty); // $end
    for _ in 0..n {
        f.block(BlockType::Empty);
    }
    f.local_get(OP);
    f.i32_const(1);
    f.i32_sub();
    f.i32_const(2);
    f.i32_shr_u();
    let targets: Vec<u32> = (0..n).collect();
    f.br_table(targets, n - 1);
    for (i, op) in FlagOp::ALL.into_iter().enumerate() {
        debug_assert_eq!(op.kind(), i as u32);
        f.end();
        rec.cf(&mut f, op);
        f.i64_extend_i32_u();
        rec.of(&mut f, op);
        f.i64_extend_i32_u();
        f.i64_const(11);
        f.i64_shl();
        f.i64_or();
        f.local_set(CF_OF);
        // Inside the blocks of the later kinds and $end.
        f.br(n - 1 - i as u32);
    }
    f.end(); // $end
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

/// `(a, b, r) -> i32`: `cond` of a record of kind `op` and width `w`
/// through the static path (for tests).
pub(super) fn static_cond_function(op: FlagOp, w: Width, cond: Cond) -> Function {
    // The record's locals sit where a chunk has them.
    let mut func = Function::new([(L_SCRATCH + 1 - 3, ValType::I64)]);
    let mut f = func.instructions();
    for (param, local) in [(0, L_CC_A), (1, L_CC_B), (2, L_CC_R)] {
        f.local_get(param);
        f.local_set(local);
    }
    ChunkEmitter::static_cond(&mut f, op, w, Cond::from_index(cond as u32 & !1));
    if cond.is_negated() {
        f.i32_eqz();
    }
    f.end();
    func
}

/// `(flags) -> i32`: `cond` of materialized flags (for tests).
pub(super) fn dynamic_cond_function(cond: Cond) -> Function {
    let mut func = Function::new([(L_SCRATCH, ValType::I64)]);
    let mut f = func.instructions();
    f.local_get(0);
    f.local_set(L_SCRATCH);
    dynamic_cond(&mut f, Cond::from_index(cond as u32 & !1));
    if cond.is_negated() {
        f.i32_eqz();
    }
    f.end();
    func
}
