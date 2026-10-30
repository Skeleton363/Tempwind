//! Shifts and rotates: `shl`/`sal`, `shr`, `sar`, `rol`, `ror`, `rcl`,
//! `rcr`, `shld`, `shrd` (`docs/design/translator.md`, "Integer results").

use iced_x86::{Mnemonic, OpKind};

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::abi::flags::{CF, OF};
use crate::ir::{BinOp, Cond, FlagOp, Value, Width};

/// A masked shift count.
#[derive(Clone, Copy)]
enum Count {
    /// An immediate: known at translation time.
    Static(u64),
    /// From CL.
    Dynamic(Value),
}

impl Count {
    fn value(self) -> Value {
        match self {
            Count::Static(c) => Value::Imm(c),
            Count::Dynamic(v) => v,
        }
    }

    /// The condition of a record that only applies for a nonzero count.
    fn when(self) -> Option<Value> {
        match self {
            Count::Static(_) => None,
            Count::Dynamic(v) => Some(v),
        }
    }
}

impl Lower<'_> {
    /// The count operand, masked to 5 bits (6 for 64-bit operands).
    fn count(&mut self, operand: u32, w: Width) -> Lowered<Count> {
        let m = if w == Width::W64 { 63 } else { 31 };
        match self.insn.op_kind(operand) {
            OpKind::Immediate8 => Ok(Count::Static(self.insn.immediate8() as u64 & m)),
            OpKind::Register => {
                let cl = self.read(operand, Width::W8)?;
                Ok(Count::Dynamic(self.bin(BinOp::And, cl, Value::Imm(m))))
            }
            _ => Err(Unsupported),
        }
    }

    /// A count of 0 changes nothing, but a register destination is still
    /// written back (a 32-bit one is zero-extended).
    fn write_back(&mut self, w: Width, a: Value) -> Lowered<Flow> {
        if self.insn.op_kind(0) == OpKind::Register {
            self.write(0, w, a)?;
        }
        Ok(Flow::Next)
    }

    /// The bit of `v` at `bit` (0 or 1).
    fn bit_at(&mut self, v: Value, bit: u32) -> Value {
        let s = self.bin(BinOp::ShrU, v, Value::Imm(bit as u64));
        self.bin(BinOp::And, s, Value::Imm(1))
    }

    /// An eager record with CF and OF replaced (rotates).
    fn set_cf_of(&mut self, cf: Value, of: Value, when: Option<Value>) {
        let of = self.bin(BinOp::Shl, of, Value::Imm(11));
        let set = self.bin(BinOp::Or, cf, of);
        self.flags_update(CF | OF, set, 0, when);
    }

    /// `shl`/`sal`, `shr`, `sar`.
    pub(super) fn shift(&mut self, m: Mnemonic) -> Lowered<Flow> {
        if self.insn.op_count() != 2 {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let a = self.read(0, w)?;
        let count = self.count(1, w)?;
        if let Count::Static(0) = count {
            return self.write_back(w, a);
        }
        let c = count.value();
        let (r, fop) = match m {
            Mnemonic::Shl | Mnemonic::Sal => {
                let r = self.bin(BinOp::Shl, a, c);
                (self.mask(r, w), FlagOp::Shl)
            }
            Mnemonic::Shr => (self.bin(BinOp::ShrU, a, c), FlagOp::Shr),
            Mnemonic::Sar => {
                let s = self.sext(a, w);
                let r = self.bin(BinOp::ShrS, s, c);
                (self.mask(r, w), FlagOp::Sar)
            }
            _ => return Err(Unsupported),
        };
        self.flags_when(fop, w, a, c, r, count.when());
        self.write(0, w, r)?;
        Ok(Flow::Next)
    }

    /// `rol`, `ror`.
    pub(super) fn rotate(&mut self, m: Mnemonic) -> Lowered<Flow> {
        if self.insn.op_count() != 2 {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let bits = w.bits() as u64;
        let a = self.read(0, w)?;
        let count = self.count(1, w)?;
        if let Count::Static(0) = count {
            return self.write_back(w, a);
        }
        let c = count.value();
        let left = m == Mnemonic::Rol;
        let r = if w == Width::W64 {
            self.bin(if left { BinOp::Rotl } else { BinOp::Rotr }, a, c)
        } else {
            // k = c mod width; a shift by width - 0 gives 0 for w <= 32.
            let k = self.bin(BinOp::And, c, Value::Imm(bits - 1));
            let back = self.bin(BinOp::Sub, Value::Imm(bits), k);
            let (s1, s2) = if left {
                (BinOp::Shl, BinOp::ShrU)
            } else {
                (BinOp::ShrU, BinOp::Shl)
            };
            let x = self.bin(s1, a, k);
            let y = self.bin(s2, a, back);
            let r = self.bin(BinOp::Or, x, y);
            self.mask(r, w)
        };
        let msb = self.bit_at(r, w.bits() - 1);
        let (cf, of) = if left {
            let cf = self.bin(BinOp::And, r, Value::Imm(1));
            (cf, self.bin(BinOp::Xor, msb, cf))
        } else {
            let next = self.bit_at(r, w.bits() - 2);
            (msb, self.bin(BinOp::Xor, msb, next))
        };
        self.set_cf_of(cf, of, count.when());
        self.write(0, w, r)?;
        Ok(Flow::Next)
    }

    /// `rcl`, `rcr`: rotates through CF.
    pub(super) fn rotate_carry(&mut self, m: Mnemonic) -> Lowered<Flow> {
        if self.insn.op_count() != 2 {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let bits = w.bits() as u64;
        let a = self.read(0, w)?;
        let count = self.count(1, w)?;
        // t = c mod (width + 1) for 8 and 16 bits.
        let count = match (count, w) {
            (Count::Static(c), Width::W8 | Width::W16) => Count::Static(c % (bits + 1)),
            (Count::Dynamic(c), Width::W8 | Width::W16) => {
                Count::Dynamic(self.bin(BinOp::RemU, c, Value::Imm(bits + 1)))
            }
            (c, _) => c,
        };
        if let Count::Static(0) = count {
            return self.write_back(w, a);
        }
        let t = count.value();
        let left = m == Mnemonic::Rcl;
        let carry = self.cond(Cond::B);
        let (r, cf) = if w == Width::W64 {
            let back = self.bin(BinOp::Sub, Value::Imm(64), t);
            let t1 = self.bin(BinOp::Sub, t, Value::Imm(1));
            let (r, cf) = if left {
                // (a << t) | (carry << (t - 1)) | ((a >> 1) >> (64 - t))
                let x = self.bin(BinOp::Shl, a, t);
                let y = self.bin(BinOp::Shl, carry, t1);
                let z = self.bin(BinOp::ShrU, a, Value::Imm(1));
                let z = self.bin(BinOp::ShrU, z, back);
                let r = self.bin(BinOp::Or, x, y);
                let r = self.bin(BinOp::Or, r, z);
                let cf = self.bin(BinOp::ShrU, a, back);
                (r, self.bin(BinOp::And, cf, Value::Imm(1)))
            } else {
                // (a >> t) | (carry << (64 - t)) | ((a << 1) << (64 - t))
                let x = self.bin(BinOp::ShrU, a, t);
                let y = self.bin(BinOp::Shl, carry, back);
                let z = self.bin(BinOp::Shl, a, Value::Imm(1));
                let z = self.bin(BinOp::Shl, z, back);
                let r = self.bin(BinOp::Or, x, y);
                let r = self.bin(BinOp::Or, r, z);
                let cf = self.bin(BinOp::ShrU, a, t1);
                (r, self.bin(BinOp::And, cf, Value::Imm(1)))
            };
            // A dynamic t = 0 keeps the value.
            (self.select(t, r, a), cf)
        } else {
            // Rotate the (w + 1)-bit value carry:a.
            let wide = Width::W64.mask() >> (63 - bits);
            let hi = self.bin(BinOp::Shl, carry, Value::Imm(bits));
            let v = self.bin(BinOp::Or, hi, a);
            let back = self.bin(BinOp::Sub, Value::Imm(bits + 1), t);
            let (s1, s2) = if left {
                (BinOp::Shl, BinOp::ShrU)
            } else {
                (BinOp::ShrU, BinOp::Shl)
            };
            let x = self.bin(s1, v, t);
            let y = self.bin(s2, v, back);
            let full = self.bin(BinOp::Or, x, y);
            let full = self.bin(BinOp::And, full, Value::Imm(wide));
            let cf = self.bin(BinOp::ShrU, full, Value::Imm(bits));
            (self.mask(full, w), cf)
        };
        let msb = self.bit_at(r, w.bits() - 1);
        let of = if left {
            self.bin(BinOp::Xor, msb, cf)
        } else {
            let next = self.bit_at(r, w.bits() - 2);
            self.bin(BinOp::Xor, msb, next)
        };
        self.set_cf_of(cf, of, count.when());
        self.write(0, w, r)?;
        Ok(Flow::Next)
    }

    /// `shld`, `shrd`: shifts in bits from a second register.
    pub(super) fn shift_double(&mut self, m: Mnemonic) -> Lowered<Flow> {
        if self.insn.op_count() != 3 {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let bits = w.bits() as u64;
        let dst = self.read(0, w)?;
        let src = self.read(1, w)?;
        let count = self.count(2, w)?;
        if let Count::Static(0) = count {
            return self.write_back(w, dst);
        }
        let c = count.value();
        let back = self.bin(BinOp::Sub, Value::Imm(bits), c);
        let left = m == Mnemonic::Shld;
        let (s1, s2, fop) = if left {
            (BinOp::Shl, BinOp::ShrU, FlagOp::Shl)
        } else {
            (BinOp::ShrU, BinOp::Shl, FlagOp::Shr)
        };
        let x = self.bin(s1, dst, c);
        let y = self.bin(s2, src, back);
        let r = self.bin(BinOp::Or, x, y);
        let mut r = self.mask(r, w);
        if w == Width::W64
            && let Count::Dynamic(c) = count
        {
            // 64 - 0 shifts by 0 (mod 64): a count of 0 keeps the value.
            r = self.select(c, r, dst);
        }
        self.flags_when(fop, w, dst, c, r, count.when());
        self.write(0, w, r)?;
        Ok(Flow::Next)
    }
}
