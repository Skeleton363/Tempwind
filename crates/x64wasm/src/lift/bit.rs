//! Bit tests and scans: `bt`, `bts`, `btr`, `btc`, `bsf`, `bsr`.

use iced_x86::{Mnemonic, OpKind};

use super::lower::Lower;
use super::operand::gpr;
use super::{Flow, Lowered, Unsupported};
use crate::abi::flags::CF;
use crate::ir::{BinOp, FlagOp, UnOp, Value};

impl Lower<'_> {
    /// `bt`, `bts`, `btr`, `btc`: CF = the selected bit; the others write
    /// the operand back with the bit set, cleared or flipped.
    pub(super) fn bit_test(&mut self, m: Mnemonic) -> Lowered<Flow> {
        if self.insn.op_count() != 2 {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let bits = w.bits() as u64;
        let off = self.read(1, w)?;
        let dynamic_mem =
            self.insn.op_kind(0) == OpKind::Memory && self.insn.op_kind(1) == OpKind::Register;
        let v = if dynamic_mem {
            // A signed bit offset selects a whole operand relative to the
            // address: ea + (sext(offset) >>s log2(w)) * (w / 8).
            let ea = self.ea()?;
            let s = self.sext(off, w);
            let unit = self.bin(BinOp::ShrS, s, Value::Imm(bits.trailing_zeros() as u64));
            let bytes = self.bin(
                BinOp::Shl,
                unit,
                Value::Imm((bits / 8).trailing_zeros() as u64),
            );
            let addr = self.bin(BinOp::Add, ea, bytes);
            self.seg_ea = Some(addr);
            self.load(w, addr)
        } else {
            self.read(0, w)?
        };
        let bit = self.bin(BinOp::And, off, Value::Imm(bits - 1));
        let shifted = self.bin(BinOp::ShrU, v, bit);
        let cf = self.bin(BinOp::And, shifted, Value::Imm(1));
        self.flags_update(CF, cf, 0, None);
        let one = self.bin(BinOp::Shl, Value::Imm(1), bit);
        let r = match m {
            Mnemonic::Bt => return Ok(Flow::Next),
            Mnemonic::Bts => self.bin(BinOp::Or, v, one),
            Mnemonic::Btr => {
                let not = self.bin(BinOp::Xor, one, Value::Imm(w.mask()));
                self.bin(BinOp::And, v, not)
            }
            Mnemonic::Btc => self.bin(BinOp::Xor, v, one),
            _ => return Err(Unsupported),
        };
        self.write(0, w, r)?;
        Ok(Flow::Next)
    }

    /// `bsf`, `bsr`: ZF = (source == 0); a zero source leaves the whole
    /// destination register as it was.
    pub(super) fn bit_scan(&mut self, m: Mnemonic) -> Lowered<Flow> {
        if self.insn.op_count() != 2 || self.insn.op_kind(0) != OpKind::Register {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let (g, _) = gpr(self.insn.op0_register())?;
        let src = self.read(1, w)?;
        let src = self.snapshot(src);
        let index = if m == Mnemonic::Bsf {
            self.un(UnOp::Ctz, src)
        } else {
            let lz = self.un(UnOp::Clz, src);
            self.bin(BinOp::Sub, Value::Imm(63), lz)
        };
        self.flags(FlagOp::Logic, w, src, Value::Imm(0), src);
        let merged = self.merge(g, w, index);
        let v = self.select(src, merged, Value::Reg(g));
        self.set_reg(g, v);
        Ok(Flow::Next)
    }
}
