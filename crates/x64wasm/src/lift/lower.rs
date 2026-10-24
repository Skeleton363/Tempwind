//! Lowering state for one instruction, the op builders every family uses,
//! and the dispatch by mnemonic to the family modules.

use iced_x86::{Instruction, Mnemonic};

use super::{Flow, Lowered, Unsupported};
use crate::ir::{BinOp, Cond, FlagOp, Gpr, Op, RSP, Temp, UnOp, VTemp, Value, Width};

/// Lowering state for one instruction.
pub(super) struct Lower<'i> {
    pub(super) insn: &'i Instruction,
    pub(super) bytes: &'i [u8],
    pub(super) ops: Vec<Op>,
    pub(super) temps: u32,
    pub(super) vtemps: u32,
    /// The memory operand's effective address, once computed.
    pub(super) ea: Option<Value>,
}

impl Lower<'_> {
    pub(super) fn temp(&mut self) -> Temp {
        let t = Temp(self.temps);
        self.temps += 1;
        t
    }

    pub(super) fn vtemp(&mut self) -> VTemp {
        let t = VTemp(self.vtemps);
        self.vtemps += 1;
        t
    }

    pub(super) fn push_op(&mut self, op: Op) {
        self.ops.push(op);
    }

    pub(super) fn bin(&mut self, op: BinOp, a: Value, b: Value) -> Value {
        if let (Value::Imm(x), Value::Imm(y)) = (a, b)
            && let Some(v) = op.eval(x, y)
        {
            return Value::Imm(v);
        }
        let dst = self.temp();
        self.push_op(Op::Bin { dst, op, a, b });
        Value::Temp(dst)
    }

    pub(super) fn un(&mut self, op: UnOp, a: Value) -> Value {
        if let Value::Imm(x) = a {
            return Value::Imm(op.eval(x));
        }
        let dst = self.temp();
        self.push_op(Op::Un { dst, op, a });
        Value::Temp(dst)
    }

    /// `cond != 0 ? a : b`
    pub(super) fn select(&mut self, cond: Value, a: Value, b: Value) -> Value {
        if let Value::Imm(c) = cond {
            return if c != 0 { a } else { b };
        }
        let dst = self.temp();
        self.push_op(Op::Select { dst, cond, a, b });
        Value::Temp(dst)
    }

    /// Sign-extends the low `width` bits of `v` to 64 bits.
    pub(super) fn sext(&mut self, v: Value, width: Width) -> Value {
        match UnOp::sext(width) {
            Some(op) => self.un(op, v),
            None => v,
        }
    }

    /// The condition (0 or 1) of the current flag record.
    pub(super) fn cond(&mut self, cond: Cond) -> Value {
        let dst = self.temp();
        self.push_op(Op::Cond { dst, cond });
        Value::Temp(dst)
    }

    pub(super) fn load_state(&mut self, width: Width, offset: u64) -> Value {
        let dst = self.temp();
        self.push_op(Op::LoadState {
            dst,
            width,
            offset: offset as u32,
        });
        Value::Temp(dst)
    }

    pub(super) fn store_state(&mut self, width: Width, offset: u64, value: Value) {
        self.push_op(Op::StoreState {
            width,
            offset: offset as u32,
            value,
        });
    }

    /// A full (or, with `when`, conditional) lazy flag record.
    pub(super) fn flags_when(
        &mut self,
        op: FlagOp,
        width: Width,
        a: Value,
        b: Value,
        r: Value,
        when: Option<Value>,
    ) {
        self.push_op(Op::Flags {
            op,
            width,
            a,
            b,
            r,
            when,
            live: true,
        });
    }

    pub(super) fn flags(&mut self, op: FlagOp, width: Width, a: Value, b: Value, r: Value) {
        self.flags_when(op, width, a, b, r, None);
    }

    /// An eager record from the previous one: `((F & !clear) | set) ^ flip`.
    pub(super) fn flags_update(&mut self, clear: u64, set: Value, flip: u64, when: Option<Value>) {
        self.push_op(Op::FlagsUpdate {
            clear,
            set,
            flip,
            when,
            live: true,
        });
    }

    /// Faults with `kind` if `cond != 0`. Must come before the
    /// instruction's first side effect.
    pub(super) fn fault_if(&mut self, cond: Value, kind: u32) {
        debug_assert!(
            !self.ops.iter().any(Op::has_side_effect),
            "fault check after a side effect"
        );
        if cond == Value::Imm(0) {
            return;
        }
        self.push_op(Op::FaultIf { cond, kind });
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
