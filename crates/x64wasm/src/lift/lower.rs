//! Lowering state for one instruction, the op builders every family uses,
//! and the dispatch by mnemonic to the family modules.

use iced_x86::{Instruction, Mnemonic, OpKind};

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
    /// The memory operand's guest address (with the FS/GS base), once computed.
    pub(super) seg_ea: Option<Value>,
}

impl Lower<'_> {
    pub(super) fn temp(&mut self) -> Temp {
        let t = Temp(self.temps);
        self.temps += 1;
        t
    }

    #[allow(dead_code)] // SSE (M3), next
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
        use Mnemonic as M;
        let insn = self.insn;
        let m = insn.mnemonic();
        if insn.has_lock_prefix() {
            // iced rejects `lock` where it would #UD; this is the SDM's list.
            let lockable = matches!(
                m,
                M::Add
                    | M::Adc
                    | M::And
                    | M::Btc
                    | M::Btr
                    | M::Bts
                    | M::Cmpxchg
                    | M::Dec
                    | M::Inc
                    | M::Neg
                    | M::Not
                    | M::Or
                    | M::Sbb
                    | M::Sub
                    | M::Xadd
                    | M::Xchg
                    | M::Xor
            );
            if !lockable || insn.op_kind(0) != OpKind::Memory {
                return Err(Unsupported);
            }
        }
        match m {
            M::Mov => self.mov(),
            M::Add | M::Sub | M::Cmp | M::Adc | M::Sbb => self.add_sub(m),
            M::And | M::Or | M::Xor | M::Test => self.logic(m),
            M::Inc | M::Dec | M::Neg | M::Not => self.unary(m),
            M::Movzx | M::Movsx | M::Movsxd => self.extend(m),
            M::Cbw | M::Cwde | M::Cdqe => self.widen_acc(),
            M::Cwd | M::Cdq | M::Cqo => self.sign_acc(),
            M::Bswap => self.bswap(),
            M::Shl | M::Sal | M::Shr | M::Sar => self.shift(m),
            M::Rol | M::Ror => self.rotate(m),
            M::Rcl | M::Rcr => self.rotate_carry(m),
            M::Shld | M::Shrd => self.shift_double(m),
            M::Mul => self.mul_acc(false),
            M::Imul => self.imul(),
            M::Div | M::Idiv => self.div(m),
            M::Bt | M::Bts | M::Btr | M::Btc => self.bit_test(m),
            M::Bsf | M::Bsr => self.bit_scan(m),
            M::Xchg => self.xchg(),
            M::Xadd => self.xadd(),
            M::Cmpxchg => self.cmpxchg(),
            M::Movsb
            | M::Movsw
            | M::Movsd
            | M::Movsq
            | M::Stosb
            | M::Stosw
            | M::Stosd
            | M::Stosq
            | M::Lodsb
            | M::Lodsw
            | M::Lodsd
            | M::Lodsq
                if is_string(insn) =>
            {
                self.string(m)
            }
            M::Lea => self.lea(),
            M::Push => self.push(),
            M::Pop => self.pop(),
            M::Call => self.call(),
            M::Ret => self.ret(),
            M::Jmp => self.jmp(),
            M::Jrcxz | M::Jecxz | M::Loop | M::Loope | M::Loopne => self.loop_rcx(),
            _ if insn.is_jcc_short_or_near() => self.jcc(),
            M::Seto
            | M::Setno
            | M::Setb
            | M::Setae
            | M::Sete
            | M::Setne
            | M::Setbe
            | M::Seta
            | M::Sets
            | M::Setns
            | M::Setp
            | M::Setnp
            | M::Setl
            | M::Setge
            | M::Setle
            | M::Setg => self.setcc(),
            M::Cmovo
            | M::Cmovno
            | M::Cmovb
            | M::Cmovae
            | M::Cmove
            | M::Cmovne
            | M::Cmovbe
            | M::Cmova
            | M::Cmovs
            | M::Cmovns
            | M::Cmovp
            | M::Cmovnp
            | M::Cmovl
            | M::Cmovge
            | M::Cmovle
            | M::Cmovg => self.cmovcc(),
            _ => self.misc(m),
        }
    }
}

/// A string instruction (as opposed to SSE `movsd`, which shares the
/// mnemonic): it has an implicit RSI/RDI (or ESI/EDI) memory operand.
fn is_string(insn: &Instruction) -> bool {
    (0..insn.op_count()).any(|i| {
        matches!(
            insn.op_kind(i),
            OpKind::MemorySegRSI | OpKind::MemorySegESI | OpKind::MemoryESRDI | OpKind::MemoryESEDI
        )
    })
}
