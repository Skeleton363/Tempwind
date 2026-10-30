//! x87 (`translator.md`, "x87"): the stack forms lower to one [`Op::X87`]
//! each, expanded in `emit/x87.rs`; the non-waiting control forms (fnstcw,
//! fnstsw, fnclex, fnstenv, fninit) are plain state and memory ops. The
//! waiting forms (fwait, fldcw, finit, fstcw, fstsw, fclex, fstenv) start
//! with [`X87::Wait`], which exits with `EXIT_UNSUPPORTED` (#MF) if an
//! unmasked exception is pending.

use iced_x86::{MemorySize, Mnemonic, OpKind, Register};

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::abi;
use crate::ir::{BinOp, Op, RAX, Value, Width, X87, X87Arith, X87Kind, X87Src, X87Unary};

/// FSW: ES and B.
const FSW_ES_B: u64 = 0x8080;
/// FSW bits fnclex clears: the exception flags, SF, ES and B.
const FSW_CLEX: u64 = 0x80ff;

/// The `kind` of a memory operand of `size`.
fn kind(size: MemorySize) -> Lowered<X87Kind> {
    Ok(match size {
        MemorySize::Float32 => X87Kind::F32,
        MemorySize::Float64 => X87Kind::F64,
        MemorySize::Float80 => X87Kind::F80,
        MemorySize::Int16 => X87Kind::I16,
        MemorySize::Int32 => X87Kind::I32,
        MemorySize::Int64 => X87Kind::I64,
        _ => return Err(Unsupported),
    })
}

/// i in ST(i).
fn st(r: Register) -> Lowered<u8> {
    if r.is_st() {
        Ok((r as u32 - Register::ST0 as u32) as u8)
    } else {
        Err(Unsupported)
    }
}

/// The mnemonics [`Lower::x87`] handles.
pub(super) fn is_x87(m: Mnemonic) -> bool {
    use Mnemonic as M;
    matches!(
        m,
        M::Wait
            | M::Fninit
            | M::Finit
            | M::Fnstcw
            | M::Fstcw
            | M::Fnstsw
            | M::Fstsw
            | M::Fnclex
            | M::Fclex
            | M::Fldcw
            | M::Fnstenv
            | M::Fstenv
            | M::Fldenv
            | M::Fld
            | M::Fild
            | M::Fldz
            | M::Fld1
            | M::Fst
            | M::Fstp
            | M::Fist
            | M::Fistp
            | M::Fxch
            | M::Fadd
            | M::Faddp
            | M::Fiadd
            | M::Fsub
            | M::Fsubp
            | M::Fisub
            | M::Fsubr
            | M::Fsubrp
            | M::Fisubr
            | M::Fmul
            | M::Fmulp
            | M::Fimul
            | M::Fdiv
            | M::Fdivp
            | M::Fidiv
            | M::Fdivr
            | M::Fdivrp
            | M::Fidivr
            | M::Fsqrt
            | M::Frndint
            | M::Fchs
            | M::Fabs
            | M::Fscale
            | M::Fcom
            | M::Fcomp
            | M::Ficom
            | M::Ficomp
            | M::Fucom
            | M::Fucomp
            | M::Fcompp
            | M::Fucompp
            | M::Fcomi
            | M::Fcomip
            | M::Fucomi
            | M::Fucomip
    )
}

impl Lower<'_> {
    fn x87_op(&mut self, op: X87, addr: Option<Value>) {
        self.push_op(Op::X87 { op, addr });
    }

    /// Operand `i` as an x87 source: ST(j) or the memory operand.
    fn x87_src(&mut self, i: u32) -> Lowered<(X87Src, Option<Value>)> {
        match self.insn.op_kind(i) {
            OpKind::Register => Ok((X87Src::St(st(self.insn.op_register(i))?), None)),
            OpKind::Memory => {
                let k = kind(self.insn.memory_size())?;
                let a = self.ea()?;
                Ok((X87Src::Mem(k), Some(a)))
            }
            _ => Err(Unsupported),
        }
    }

    /// The last operand (the source of fld, fcom, fxch …).
    fn x87_last(&mut self) -> Lowered<(X87Src, Option<Value>)> {
        match self.insn.op_count() {
            0 => Err(Unsupported),
            n => self.x87_src(n - 1),
        }
    }

    /// The memory operand's address, if the instruction has one.
    fn x87_mem(&mut self) -> Lowered<Value> {
        if (0..self.insn.op_count()).any(|i| self.insn.op_kind(i) == OpKind::Memory) {
            self.ea()
        } else {
            Err(Unsupported)
        }
    }

    pub(super) fn x87(&mut self, m: Mnemonic) -> Lowered<Flow> {
        use Mnemonic as M;
        match m {
            M::Wait => self.x87_op(X87::Wait, None),
            M::Fninit | M::Finit => {
                if m == M::Finit {
                    self.x87_op(X87::Wait, None);
                }
                // FCW = 0x037F, FSW = 0 (TOP 0, no exceptions), every
                // register empty (the full tag word). R0-R7 keep their
                // contents.
                self.store_state(Width::W16, abi::STATE_FCW, Value::Imm(0x037f));
                self.store_state(Width::W16, abi::STATE_FSW, Value::Imm(0));
                self.store_state(Width::W16, abi::STATE_FTW, Value::Imm(0xffff));
            }
            M::Fnstcw | M::Fstcw => {
                let a = self.x87_mem()?;
                if m == M::Fstcw {
                    self.x87_op(X87::Wait, None);
                }
                let v = self.load_state(Width::W16, abi::STATE_FCW);
                self.store(Width::W16, a, v);
            }
            M::Fnstsw | M::Fstsw => {
                let dst = match self.insn.op_kind(0) {
                    OpKind::Register if self.insn.op_register(0) == Register::AX => None,
                    OpKind::Memory => Some(self.ea()?),
                    _ => return Err(Unsupported),
                };
                if m == M::Fstsw {
                    self.x87_op(X87::Wait, None);
                }
                let v = self.load_state(Width::W16, abi::STATE_FSW);
                match dst {
                    Some(a) => self.store(Width::W16, a, v),
                    None => self.write_reg(RAX, Width::W16, v),
                }
            }
            M::Fnclex | M::Fclex => {
                if m == M::Fclex {
                    self.x87_op(X87::Wait, None);
                }
                let fsw = self.load_state(Width::W16, abi::STATE_FSW);
                let v = self.bin(BinOp::And, fsw, Value::Imm(!FSW_CLEX));
                self.store_state(Width::W16, abi::STATE_FSW, v);
            }
            M::Fldcw => self.fldcw()?,
            M::Fnstenv | M::Fstenv => self.fnstenv(m == M::Fstenv)?,
            M::Fldenv => {
                let a = self.x87_mem()?;
                self.x87_op(X87::LoadEnv, Some(a));
            }
            M::Fld | M::Fild => match self.x87_last()? {
                (X87Src::St(i), _) => self.x87_op(X87::LoadSt(i), None),
                (X87Src::Mem(k), a) => self.x87_op(X87::Load(k), a),
            },
            M::Fldz => self.x87_op(X87::LoadConst(0, 0), None),
            M::Fld1 => self.x87_op(X87::LoadConst(1 << 63, 0x3fff), None),
            M::Fst | M::Fstp | M::Fist | M::Fistp => {
                let pop = matches!(m, M::Fstp | M::Fistp);
                match self.x87_src(0)? {
                    (X87Src::St(i), _) => self.x87_op(X87::StoreSt { i, pop }, None),
                    (X87Src::Mem(kind), a) => self.x87_op(X87::Store { kind, pop }, a),
                }
            }
            M::Fxch => match self.x87_last()? {
                (X87Src::St(i), _) => self.x87_op(X87::Xch(i), None),
                _ => return Err(Unsupported),
            },
            M::Fadd
            | M::Faddp
            | M::Fiadd
            | M::Fsub
            | M::Fsubp
            | M::Fisub
            | M::Fsubr
            | M::Fsubrp
            | M::Fisubr
            | M::Fmul
            | M::Fmulp
            | M::Fimul
            | M::Fdiv
            | M::Fdivp
            | M::Fidiv
            | M::Fdivr
            | M::Fdivrp
            | M::Fidivr => {
                let op = match m {
                    M::Fadd | M::Faddp | M::Fiadd => X87Arith::Add,
                    M::Fsub | M::Fsubp | M::Fisub => X87Arith::Sub,
                    M::Fsubr | M::Fsubrp | M::Fisubr => X87Arith::Subr,
                    M::Fmul | M::Fmulp | M::Fimul => X87Arith::Mul,
                    M::Fdiv | M::Fdivp | M::Fidiv => X87Arith::Div,
                    _ => X87Arith::Divr,
                };
                let pop = matches!(
                    m,
                    M::Faddp | M::Fsubp | M::Fsubrp | M::Fmulp | M::Fdivp | M::Fdivrp
                );
                // Two registers (ST(0), ST(i) either way round), or a
                // memory operand alone with ST(0) as the destination.
                let (dst, (src, a)) = match self.insn.op_count() {
                    1 if self.insn.op_kind(0) == OpKind::Memory => (0, self.x87_src(0)?),
                    2 if self.insn.op_kind(0) == OpKind::Register => {
                        (st(self.insn.op_register(0))?, self.x87_src(1)?)
                    }
                    _ => return Err(Unsupported),
                };
                self.x87_op(X87::Arith { op, dst, src, pop }, a);
            }
            M::Fsqrt => self.x87_op(X87::Unary(X87Unary::Sqrt), None),
            M::Frndint => self.x87_op(X87::Unary(X87Unary::Round), None),
            M::Fchs => self.x87_op(X87::Unary(X87Unary::Chs), None),
            M::Fabs => self.x87_op(X87::Unary(X87Unary::Abs), None),
            M::Fscale => self.x87_op(X87::Scale, None),
            M::Fcom | M::Fcomp | M::Ficom | M::Ficomp | M::Fucom | M::Fucomp => {
                let pops = matches!(m, M::Fcomp | M::Ficomp | M::Fucomp) as u8;
                let quiet = matches!(m, M::Fucom | M::Fucomp);
                let (src, a) = match self.insn.op_count() {
                    0 => (X87Src::St(1), None),
                    _ => self.x87_last()?,
                };
                self.x87_op(X87::Com { src, quiet, pops }, a);
            }
            M::Fcompp | M::Fucompp => {
                let quiet = m == M::Fucompp;
                self.x87_op(
                    X87::Com {
                        src: X87Src::St(1),
                        quiet,
                        pops: 2,
                    },
                    None,
                );
            }
            M::Fcomi | M::Fcomip | M::Fucomi | M::Fucomip => {
                let i = match self.x87_last()? {
                    (X87Src::St(i), _) => i,
                    _ => return Err(Unsupported),
                };
                let dst = self.temp();
                self.x87_op(
                    X87::Comi {
                        dst,
                        i,
                        quiet: matches!(m, M::Fucomi | M::Fucomip),
                        pop: matches!(m, M::Fcomip | M::Fucomip),
                    },
                    None,
                );
                // C3 -> ZF, C2 -> PF, C0 -> CF; OF, SF and AF 0.
                self.flags_update(abi::flags::STATUS, Value::Temp(dst), 0, None);
            }
            _ => return Err(Unsupported),
        }
        Ok(Flow::Next)
    }

    /// fldcw m16: FCW from memory; ES and B follow the flags it leaves
    /// unmasked (an exception that becomes pending is raised by the next
    /// waiting instruction).
    fn fldcw(&mut self) -> Lowered<()> {
        let a = self.x87_mem()?;
        self.x87_op(X87::Wait, None);
        let cw = self.load(Width::W16, a);
        let fsw = self.load_state(Width::W16, abi::STATE_FSW);
        let masks = self.bin(BinOp::Xor, cw, Value::Imm(0x3f));
        let pending = self.bin(BinOp::And, fsw, masks);
        let pending = self.bin(BinOp::And, pending, Value::Imm(0x3f));
        let pending = self.bin(BinOp::Ne, pending, Value::Imm(0));
        let es = self.bin(BinOp::Mul, pending, Value::Imm(FSW_ES_B));
        let kept = self.bin(BinOp::And, fsw, Value::Imm(!FSW_ES_B));
        let fsw = self.bin(BinOp::Or, kept, es);
        self.store_state(Width::W16, abi::STATE_FCW, cw);
        self.store_state(Width::W16, abi::STATE_FSW, fsw);
        Ok(())
    }

    /// fnstenv m28: the 32-bit protected-mode environment (FCW, FSW and the
    /// tag word in the low halves of the first three dwords; the pointers,
    /// opcode and selectors 0), then every exception masked, which clears ES
    /// and B.
    fn fnstenv(&mut self, wait: bool) -> Lowered<()> {
        let a = self.x87_mem()?;
        if wait {
            self.x87_op(X87::Wait, None);
        }
        let fcw = self.load_state(Width::W16, abi::STATE_FCW);
        let fsw = self.load_state(Width::W16, abi::STATE_FSW);
        let ftw = self.load_state(Width::W16, abi::STATE_FTW);
        let hi = self.bin(BinOp::Shl, fsw, Value::Imm(32));
        let w0 = self.bin(BinOp::Or, fcw, hi);
        self.store(Width::W64, a, w0);
        let a8 = self.bin(BinOp::Add, a, Value::Imm(8));
        self.store(Width::W64, a8, ftw);
        let a16 = self.bin(BinOp::Add, a, Value::Imm(16));
        self.store(Width::W64, a16, Value::Imm(0));
        let a24 = self.bin(BinOp::Add, a, Value::Imm(24));
        self.store(Width::W32, a24, Value::Imm(0));
        let fcw = self.bin(BinOp::Or, fcw, Value::Imm(0x3f));
        self.store_state(Width::W16, abi::STATE_FCW, fcw);
        let fsw = self.bin(BinOp::And, fsw, Value::Imm(!FSW_ES_B));
        self.store_state(Width::W16, abi::STATE_FSW, fsw);
        Ok(())
    }
}
