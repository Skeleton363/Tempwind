//! Lowering state for one instruction, the op builders every family uses,
//! and the dispatch by mnemonic to the family modules.

use iced_x86::{Instruction, Mnemonic, OpKind};

use super::{Checks, Flow, Lowered, Unsupported};
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
    /// The memory checks to insert (`translator.md`, "Memory checks").
    pub(super) checks: Checks,
    /// Temporaries allocated before the first side effect (once there is
    /// one): the values a check placed before it may use.
    pub(super) effect_temps: Option<u32>,
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
        if self.effect_temps.is_none() && op.has_side_effect() {
            self.effect_temps = Some(self.temps);
        }
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

    /// The memory checks of an access of `len` bytes at `addr` (only if
    /// `when != 0`, with `when`). They go before the instruction's first
    /// side effect: an access after one (the store of a read-modify-write
    /// after its flag record) is checked there, and merges into the check of
    /// the same bytes when there is one, which then also tests the page
    /// byte. Its address must be computed before that side effect; an
    /// instruction whose later accesses use addresses computed after it
    /// checks them first ([`Lower::mem_check`]) and uses the unchecked
    /// builders.
    pub(super) fn mem_check_when(
        &mut self,
        addr: Value,
        len: u32,
        write: bool,
        when: Option<Value>,
    ) {
        let null = self.checks.null;
        let code = write && self.checks.store;
        if !null && !code {
            return;
        }
        let at = self
            .ops
            .iter()
            .position(Op::has_side_effect)
            .unwrap_or(self.ops.len());
        let same = self.ops[..at].iter_mut().find_map(|op| match op {
            Op::MemCheck {
                addr: a,
                len: l,
                when: w,
                code,
                ..
            } if (*a, *l, *w) == (addr, len, when) => Some(code),
            _ => None,
        });
        if let Some(c) = same {
            *c |= code;
            return;
        }
        let check = Op::MemCheck {
            addr,
            len,
            write,
            null,
            code,
            when,
        };
        if at < self.ops.len() {
            // The values must be the same before the side effect.
            let written: crate::sync::Set = self.ops[at..]
                .iter()
                .map(|op| crate::sync::op_effect(op).writes)
                .fold(0, |a, b| a | b);
            let before = |v: Value| match v {
                Value::Reg(r) => written & (1 << r) == 0,
                Value::Temp(t) => self.effect_temps.is_some_and(|n| t.0 < n),
                Value::Imm(_) => true,
            };
            assert!(
                before(addr) && when.is_none_or(before),
                "memory check of an address computed after a side effect"
            );
        }
        self.ops.insert(at, check);
    }

    pub(super) fn mem_check(&mut self, addr: Value, len: u32, write: bool) {
        self.mem_check_when(addr, len, write, None);
    }

    pub(super) fn load(&mut self, width: Width, addr: Value) -> Value {
        self.mem_check(addr, width.bits() / 8, false);
        self.load_unchecked(width, addr)
    }

    /// A load whose memory checks are already in place.
    pub(super) fn load_unchecked(&mut self, width: Width, addr: Value) -> Value {
        let dst = self.temp();
        self.push_op(Op::Load { dst, width, addr });
        Value::Temp(dst)
    }

    pub(super) fn store(&mut self, width: Width, addr: Value, value: Value) {
        self.mem_check(addr, width.bits() / 8, true);
        self.store_unchecked(width, addr, value);
    }

    /// A store whose memory checks are already in place.
    pub(super) fn store_unchecked(&mut self, width: Width, addr: Value, value: Value) {
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
            | M::Cmpsb
            | M::Cmpsw
            | M::Cmpsd
            | M::Cmpsq
            | M::Scasb
            | M::Scasw
            | M::Scasd
            | M::Scasq
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
            _ if super::x87::is_x87(m) => self.x87(m),
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
            _ if super::sse::is_sse(m) => self.sse(m),
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
