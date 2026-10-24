//! Stack: `push`, `pop`.

use iced_x86::Code;

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::ir::{BinOp, RSP, Value, Width};

impl Lower<'_> {
    pub(super) fn push(&mut self) -> Lowered<Flow> {
        let v = match self.insn.code() {
            Code::Push_r64 | Code::Pushq_imm8 | Code::Pushq_imm32 | Code::Push_rm64 => {
                self.read(0, Width::W64)?
            }
            _ => return Err(Unsupported),
        };
        self.push64(v);
        Ok(Flow::Next)
    }

    pub(super) fn pop(&mut self) -> Lowered<Flow> {
        if !matches!(self.insn.code(), Code::Pop_r64 | Code::Pop_rm64) {
            return Err(Unsupported);
        }
        let v = self.load(Width::W64, Value::Reg(RSP));
        let sp = self.bin(BinOp::Add, Value::Reg(RSP), Value::Imm(8));
        self.set_reg(RSP, sp);
        // A memory destination's address uses RSP after the increment.
        self.write(0, Width::W64, v)?;
        Ok(Flow::Next)
    }
}
