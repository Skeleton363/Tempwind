//! Extensions and byte swaps: `movzx`, `movsx`, `movsxd`, `cbw`, `cwde`,
//! `cdqe`, `cwd`, `cdq`, `cqo`, `bswap`.

use iced_x86::{Code, Mnemonic};

use super::lower::Lower;
use super::operand::gpr;
use super::{Flow, Lowered, Unsupported};
use crate::ir::{BinOp, RAX, RDX, Value, Width};

impl Lower<'_> {
    /// `movzx`, `movsx`, `movsxd`.
    pub(super) fn extend(&mut self, m: Mnemonic) -> Lowered<Flow> {
        if self.insn.op_count() != 2 {
            return Err(Unsupported);
        }
        let wd = self.width_of(0)?;
        let ws = self.width_of(1)?;
        let v = self.read(1, ws)?;
        let v = if m == Mnemonic::Movzx {
            v
        } else {
            let s = self.sext(v, ws);
            self.mask(s, wd)
        };
        self.write(0, wd, v)?;
        Ok(Flow::Next)
    }

    /// `cbw`/`cwde`/`cdqe`: the accumulator's low half sign-extended into it.
    pub(super) fn widen_acc(&mut self) -> Lowered<Flow> {
        let (from, to) = match self.insn.code() {
            Code::Cbw => (Width::W8, Width::W16),
            Code::Cwde => (Width::W16, Width::W32),
            Code::Cdqe => (Width::W32, Width::W64),
            _ => return Err(Unsupported),
        };
        let v = self.read_reg(RAX, from);
        let s = self.sext(v, from);
        let s = self.mask(s, to);
        self.write_reg(RAX, to, s);
        Ok(Flow::Next)
    }

    /// `cwd`/`cdq`/`cqo`: DX, EDX or RDX = the accumulator's sign.
    pub(super) fn sign_acc(&mut self) -> Lowered<Flow> {
        let w = match self.insn.code() {
            Code::Cwd => Width::W16,
            Code::Cdq => Width::W32,
            Code::Cqo => Width::W64,
            _ => return Err(Unsupported),
        };
        let v = self.read_reg(RAX, w);
        let s = self.sext(v, w);
        let fill = self.bin(BinOp::ShrS, s, Value::Imm(63));
        let fill = self.mask(fill, w);
        self.write_reg(RDX, w, fill);
        Ok(Flow::Next)
    }

    /// `bswap r32/r64`; `bswap r16` (undefined in the SDM) clears the low
    /// 16 bits, as CPUs do.
    pub(super) fn bswap(&mut self) -> Lowered<Flow> {
        let (g, _) = gpr(self.insn.op0_register())?;
        let w = self.width_of(0)?;
        let x = self.read_reg(g, w);
        let swap = |l: &mut Self, x: Value, shift: u64, m: u64| {
            let hi = l.bin(BinOp::ShrU, x, Value::Imm(shift));
            let hi = l.bin(BinOp::And, hi, Value::Imm(m));
            let lo = l.bin(BinOp::And, x, Value::Imm(m));
            let lo = l.bin(BinOp::Shl, lo, Value::Imm(shift));
            l.bin(BinOp::Or, hi, lo)
        };
        let r = match w {
            Width::W64 => {
                let x = swap(self, x, 8, 0x00ff_00ff_00ff_00ff);
                let x = swap(self, x, 16, 0x0000_ffff_0000_ffff);
                self.bin(BinOp::Rotl, x, Value::Imm(32))
            }
            Width::W32 => {
                let x = swap(self, x, 8, 0x00ff_00ff);
                swap(self, x, 16, 0x0000_ffff)
            }
            Width::W16 => Value::Imm(0),
            Width::W8 => return Err(Unsupported),
        };
        self.write_reg(g, w, r);
        Ok(Flow::Next)
    }
}
