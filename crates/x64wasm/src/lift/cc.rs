//! Conditional moves and sets: `setcc`, `cmovcc`.

use super::branch::cond;
use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::ir::Width;

impl Lower<'_> {
    /// `setcc r/m8`: 1 if the condition holds, else 0.
    pub(super) fn setcc(&mut self) -> Lowered<Flow> {
        if self.insn.op_count() != 1 || self.width_of(0)? != Width::W8 {
            return Err(Unsupported);
        }
        let c = cond(self.insn.condition_code())?;
        let v = self.cond(c);
        self.write(0, Width::W8, v)?;
        Ok(Flow::Next)
    }

    /// `cmovcc r, r/m`: the source is always read, and the destination is
    /// always written (a 32-bit one is zero-extended even when the
    /// condition is false).
    pub(super) fn cmovcc(&mut self) -> Lowered<Flow> {
        if self.insn.op_count() != 2 {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let src = self.read(1, w)?;
        let dst = self.read(0, w)?;
        let c = cond(self.insn.condition_code())?;
        let c = self.cond(c);
        let v = self.select(c, src, dst);
        self.write(0, w, v)?;
        Ok(Flow::Next)
    }
}
