//! Control transfers: `call`, `ret`, `jmp`, `jcc`, `jrcxz`, `loop`,
//! `loope`, `loopne`.

use iced_x86::{Code, ConditionCode, OpKind};

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::ir::{BinOp, CallTarget, Cond, RCX, RSP, Term, Value, Width};

pub(super) fn cond(cc: ConditionCode) -> Lowered<Cond> {
    Ok(match cc {
        ConditionCode::o => Cond::O,
        ConditionCode::no => Cond::No,
        ConditionCode::b => Cond::B,
        ConditionCode::ae => Cond::Ae,
        ConditionCode::e => Cond::E,
        ConditionCode::ne => Cond::Ne,
        ConditionCode::be => Cond::Be,
        ConditionCode::a => Cond::A,
        ConditionCode::s => Cond::S,
        ConditionCode::ns => Cond::Ns,
        ConditionCode::p => Cond::P,
        ConditionCode::np => Cond::Np,
        ConditionCode::l => Cond::L,
        ConditionCode::ge => Cond::Ge,
        ConditionCode::le => Cond::Le,
        ConditionCode::g => Cond::G,
        _ => return Err(Unsupported),
    })
}

impl Lower<'_> {
    pub(super) fn call(&mut self) -> Lowered<Flow> {
        let insn = self.insn;
        let next = insn.next_ip();
        match insn.code() {
            Code::Call_rel32_64 => {
                self.push64(Value::Imm(next));
                Ok(Flow::End(Term::Call {
                    target: CallTarget::Direct(insn.near_branch_target()),
                    ret: next,
                }))
            }
            Code::Call_rm64 => {
                let target = self.read(0, Width::W64)?;
                let target = self.snapshot(target);
                self.push64(Value::Imm(next));
                Ok(Flow::End(Term::Call {
                    target: CallTarget::Indirect(target),
                    ret: next,
                }))
            }
            _ => Err(Unsupported),
        }
    }

    pub(super) fn ret(&mut self) -> Lowered<Flow> {
        let insn = self.insn;
        let extra = match insn.code() {
            Code::Retnq => 0,
            Code::Retnq_imm16 => insn.immediate16() as u64,
            _ => return Err(Unsupported),
        };
        let target = self.load(Width::W64, Value::Reg(RSP));
        let sp = self.bin(BinOp::Add, Value::Reg(RSP), Value::Imm(8 + extra));
        self.set_reg(RSP, sp);
        Ok(Flow::End(Term::Ret(target)))
    }

    pub(super) fn jmp(&mut self) -> Lowered<Flow> {
        let insn = self.insn;
        match insn.code() {
            Code::Jmp_rel8_64 | Code::Jmp_rel32_64 => {
                Ok(Flow::End(Term::Jump(insn.near_branch_target())))
            }
            Code::Jmp_rm64 => {
                let target = self.read(0, Width::W64)?;
                Ok(Flow::End(Term::Indirect(target)))
            }
            _ => Err(Unsupported),
        }
    }

    pub(super) fn jcc(&mut self) -> Lowered<Flow> {
        let insn = self.insn;
        if insn.op_kind(0) != OpKind::NearBranch64 {
            return Err(Unsupported);
        }
        Ok(Flow::End(Term::Branch {
            cond: cond(insn.condition_code())?,
            taken: insn.near_branch_target(),
            fallthrough: insn.next_ip(),
        }))
    }

    /// `jrcxz`, `loop`, `loope`, `loopne` (RCX forms only; the 67h forms
    /// with ECX stay unsupported). `loop` decrements RCX without flags.
    pub(super) fn loop_rcx(&mut self) -> Lowered<Flow> {
        let insn = self.insn;
        let value = match insn.code() {
            Code::Jrcxz_rel8_64 => self.bin(BinOp::Eq, Value::Reg(RCX), Value::Imm(0)),
            code
            @ (Code::Loop_rel8_64_RCX | Code::Loope_rel8_64_RCX | Code::Loopne_rel8_64_RCX) => {
                let rcx = self.bin(BinOp::Sub, Value::Reg(RCX), Value::Imm(1));
                self.set_reg(RCX, rcx);
                let nz = self.bin(BinOp::Ne, rcx, Value::Imm(0));
                match code {
                    Code::Loope_rel8_64_RCX => {
                        let z = self.cond(Cond::E);
                        self.bin(BinOp::And, nz, z)
                    }
                    Code::Loopne_rel8_64_RCX => {
                        let z = self.cond(Cond::Ne);
                        self.bin(BinOp::And, nz, z)
                    }
                    _ => nz,
                }
            }
            _ => return Err(Unsupported),
        };
        Ok(Flow::End(Term::BranchIf {
            value,
            taken: insn.near_branch_target(),
            fallthrough: insn.next_ip(),
        }))
    }
}
