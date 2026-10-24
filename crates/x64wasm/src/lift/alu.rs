//! Integer arithmetic: `add`, `sub`, `cmp`.

use iced_x86::Mnemonic;

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::ir::{BinOp, FlagOp, Op};

impl Lower<'_> {
    /// `add`, `sub` and `cmp` (`sub` without the write).
    pub(super) fn add_sub(&mut self, m: Mnemonic) -> Lowered<Flow> {
        if self.insn.op_count() != 2 {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let a = self.read(0, w)?;
        let b = self.read(1, w)?;
        let (bin, fop) = if m == Mnemonic::Add {
            (BinOp::Add, FlagOp::Add)
        } else {
            (BinOp::Sub, FlagOp::Sub)
        };
        let raw = self.bin(bin, a, b);
        let r = self.mask(raw, w);
        self.push_op(Op::Flags {
            op: fop,
            width: w,
            a,
            b,
            r,
            live: true,
        });
        if m != Mnemonic::Cmp {
            self.write(0, w, r)?;
        }
        Ok(Flow::Next)
    }
}
