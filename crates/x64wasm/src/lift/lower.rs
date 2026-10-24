//! Lowering state for one instruction, the op builders every family uses,
//! and the dispatch by mnemonic to the family modules.

use iced_x86::{Instruction, Mnemonic};

use super::{Flow, Lowered, Unsupported};
use crate::ir::{BinOp, Gpr, Op, RSP, Temp, Value, Width};

/// Lowering state for one instruction.
pub(super) struct Lower<'i> {
    pub(super) insn: &'i Instruction,
    pub(super) bytes: &'i [u8],
    pub(super) ops: Vec<Op>,
    pub(super) temps: u32,
    /// The memory operand's effective address, once computed.
    pub(super) ea: Option<Value>,
}

impl Lower<'_> {
    pub(super) fn temp(&mut self) -> Temp {
        let t = Temp(self.temps);
        self.temps += 1;
        t
    }

    pub(super) fn push_op(&mut self, op: Op) {
        self.ops.push(op);
    }

    pub(super) fn bin(&mut self, op: BinOp, a: Value, b: Value) -> Value {
        if let (Value::Imm(x), Value::Imm(y)) = (a, b) {
            return Value::Imm(match op {
                BinOp::Add => x.wrapping_add(y),
                BinOp::Sub => x.wrapping_sub(y),
                BinOp::And => x & y,
                BinOp::Or => x | y,
                BinOp::Xor => x ^ y,
                BinOp::Shl => x.wrapping_shl(y as u32),
                BinOp::ShrU => x.wrapping_shr(y as u32),
            });
        }
        let dst = self.temp();
        self.push_op(Op::Bin { dst, op, a, b });
        Value::Temp(dst)
    }

    pub(super) fn mask(&mut self, v: Value, width: Width) -> Value {
        match width {
            Width::W64 => v,
            w => self.bin(BinOp::And, v, Value::Imm(w.mask())),
        }
    }

    pub(super) fn load(&mut self, width: Width, addr: Value) -> Value {
        let dst = self.temp();
        self.push_op(Op::Load { dst, width, addr });
        Value::Temp(dst)
    }

    pub(super) fn store(&mut self, width: Width, addr: Value, value: Value) {
        self.push_op(Op::Store { width, addr, value });
    }

    pub(super) fn set_reg(&mut self, reg: Gpr, value: Value) {
        self.push_op(Op::SetReg { reg, value });
    }

    /// Copies a value into a temporary so later register writes don't change it.
    pub(super) fn snapshot(&mut self, v: Value) -> Value {
        match v {
            Value::Reg(_) => {
                let dst = self.temp();
                self.push_op(Op::Mov { dst, src: v });
                Value::Temp(dst)
            }
            v => v,
        }
    }

    pub(super) fn push64(&mut self, value: Value) {
        let sp = self.bin(BinOp::Sub, Value::Reg(RSP), Value::Imm(8));
        self.store(Width::W64, sp, value);
        self.set_reg(RSP, sp);
    }

    pub(super) fn lower(&mut self) -> Lowered<Flow> {
        let insn = self.insn;
        if insn.has_lock_prefix() {
            return Err(Unsupported);
        }
        match insn.mnemonic() {
            Mnemonic::Nop => Ok(Flow::Next),
            Mnemonic::Mov => self.mov(),
            m @ (Mnemonic::Add | Mnemonic::Sub | Mnemonic::Cmp) => self.add_sub(m),
            Mnemonic::Lea => self.lea(),
            Mnemonic::Push => self.push(),
            Mnemonic::Pop => self.pop(),
            Mnemonic::Call => self.call(),
            Mnemonic::Ret => self.ret(),
            Mnemonic::Jmp => self.jmp(),
            _ if insn.is_jcc_short_or_near() => self.jcc(),
            _ => Err(Unsupported),
        }
    }
}
