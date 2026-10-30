//! Integer arithmetic and logic: `add`, `sub`, `cmp`, `adc`, `sbb`, `and`,
//! `or`, `xor`, `test`, `inc`, `dec`, `neg`, `not`.

use iced_x86::Mnemonic;

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::ir::{BinOp, Cond, FlagOp, Value};

impl Lower<'_> {
    /// `add`, `sub` and `cmp` (`sub` without the write), `adc` and `sbb`.
    pub(super) fn add_sub(&mut self, m: Mnemonic) -> Lowered<Flow> {
        if self.insn.op_count() != 2 {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let a = self.read(0, w)?;
        let b = self.read(1, w)?;
        let (bin, fop) = match m {
            Mnemonic::Add => (BinOp::Add, FlagOp::Add),
            Mnemonic::Sub | Mnemonic::Cmp => (BinOp::Sub, FlagOp::Sub),
            Mnemonic::Adc => (BinOp::Add, FlagOp::Adc),
            Mnemonic::Sbb => (BinOp::Sub, FlagOp::Sbb),
            _ => return Err(Unsupported),
        };
        let mut raw = self.bin(bin, a, b);
        if matches!(fop, FlagOp::Adc | FlagOp::Sbb) {
            // The carry-in is read before the new record.
            let carry = self.cond(Cond::B);
            raw = self.bin(bin, raw, carry);
        }
        let r = self.mask(raw, w);
        self.flags(fop, w, a, b, r);
        if m != Mnemonic::Cmp {
            self.write(0, w, r)?;
        }
        Ok(Flow::Next)
    }

    /// `and`, `or`, `xor` and `test` (`and` without the write).
    pub(super) fn logic(&mut self, m: Mnemonic) -> Lowered<Flow> {
        if self.insn.op_count() != 2 {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let a = self.read(0, w)?;
        let b = self.read(1, w)?;
        let bin = match m {
            Mnemonic::And | Mnemonic::Test => BinOp::And,
            Mnemonic::Or => BinOp::Or,
            Mnemonic::Xor => BinOp::Xor,
            _ => return Err(Unsupported),
        };
        let r = self.bin(bin, a, b);
        self.flags(FlagOp::Logic, w, r, Value::Imm(0), r);
        if m != Mnemonic::Test {
            self.write(0, w, r)?;
        }
        Ok(Flow::Next)
    }

    /// `inc`, `dec`, `neg` and `not`.
    pub(super) fn unary(&mut self, m: Mnemonic) -> Lowered<Flow> {
        if self.insn.op_count() != 1 {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let a = self.read(0, w)?;
        let r = match m {
            Mnemonic::Inc | Mnemonic::Dec => {
                let (bin, fop) = if m == Mnemonic::Inc {
                    (BinOp::Add, FlagOp::Inc)
                } else {
                    (BinOp::Sub, FlagOp::Dec)
                };
                let raw = self.bin(bin, a, Value::Imm(1));
                let r = self.mask(raw, w);
                // `b` is a placeholder: the emitter keeps the previous CF.
                self.flags(fop, w, a, Value::Imm(0), r);
                r
            }
            Mnemonic::Neg => {
                let raw = self.bin(BinOp::Sub, Value::Imm(0), a);
                let r = self.mask(raw, w);
                self.flags(FlagOp::Sub, w, Value::Imm(0), a, r);
                r
            }
            Mnemonic::Not => self.bin(BinOp::Xor, a, Value::Imm(w.mask())),
            _ => return Err(Unsupported),
        };
        self.write(0, w, r)?;
        Ok(Flow::Next)
    }
}
