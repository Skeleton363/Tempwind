//! Data movement: `mov`, `lea`.

use iced_x86::OpKind;

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::ir::Width;

impl Lower<'_> {
    pub(super) fn mov(&mut self) -> Lowered<Flow> {
        if self.insn.op_count() != 2 {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let v = self.read(1, w)?;
        self.write(0, w, v)?;
        Ok(Flow::Next)
    }

    pub(super) fn lea(&mut self) -> Lowered<Flow> {
        let insn = self.insn;
        if insn.op_kind(0) != OpKind::Register || insn.op_kind(1) != OpKind::Memory {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        if w == Width::W8 {
            return Err(Unsupported);
        }
        let addr = self.ea_offset()?;
        let v = self.mask(addr, w);
        self.write(0, w, v)?;
        Ok(Flow::Next)
    }
}
