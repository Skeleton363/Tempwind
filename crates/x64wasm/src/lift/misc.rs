//! Flag and no-op instructions: `ud2`, `cmc`, `clc`, `stc`, `std`, `cld`,
//! `lahf`, `sahf`; the nop forms, `pause`, `prefetch*` (no access, never
//! fault); `cpuid`; `mov r/m16, Sreg` (`segment_selector`).

use iced_x86::{Mnemonic, OpKind, Register};

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::abi::{
    self,
    flags::{AF, CF, DF, PF, RESERVED1, SF, ZF},
};
use crate::ir::{BinOp, RAX, RCX, RDX, Term, Value, Width};

/// RBX (encoding 3).
const RBX: u8 = 3;

impl Lower<'_> {
    pub(super) fn misc(&mut self, m: Mnemonic) -> Lowered<Flow> {
        match m {
            Mnemonic::Nop
            | Mnemonic::Pause
            | Mnemonic::Endbr64
            | Mnemonic::Prefetchnta
            | Mnemonic::Prefetcht0
            | Mnemonic::Prefetcht1
            | Mnemonic::Prefetcht2
            | Mnemonic::Prefetchw
            | Mnemonic::Prefetch => {}
            Mnemonic::Ud2 => {
                return Ok(Flow::End(Term::Fault {
                    kind: abi::FAULT_INVALID_OPCODE,
                    rip: self.insn.ip(),
                }));
            }
            Mnemonic::Cmc => self.flags_update(0, Value::Imm(0), CF, None),
            Mnemonic::Clc => self.flags_update(CF, Value::Imm(0), 0, None),
            Mnemonic::Stc => self.flags_update(0, Value::Imm(CF), 0, None),
            Mnemonic::Cpuid => self.cpuid(),
            // AH = SF:ZF:0:AF:0:PF:1:CF.
            Mnemonic::Lahf => {
                let f = self.read_flags();
                let ah = self.bin(BinOp::And, f, Value::Imm(SF | ZF | AF | PF | CF));
                let ah = self.bin(BinOp::Or, ah, Value::Imm(RESERVED1));
                let ah = self.bin(BinOp::Shl, ah, Value::Imm(8));
                let kept = self.bin(BinOp::And, Value::Reg(RAX), Value::Imm(!0xff00));
                let rax = self.bin(BinOp::Or, kept, ah);
                self.set_reg(RAX, rax);
            }
            // SF, ZF, AF, PF and CF from AH; OF unchanged.
            Mnemonic::Sahf => {
                let ah = self.bin(BinOp::ShrU, Value::Reg(RAX), Value::Imm(8));
                let mask = SF | ZF | AF | PF | CF;
                let set = self.bin(BinOp::And, ah, Value::Imm(mask));
                self.flags_update(mask, set, 0, None);
            }
            Mnemonic::Std | Mnemonic::Cld => {
                // DF lives in the state's RFLAGS while code runs.
                let rflags = self.load_state(Width::W64, abi::STATE_RFLAGS);
                let v = if m == Mnemonic::Std {
                    self.bin(BinOp::Or, rflags, Value::Imm(DF))
                } else {
                    self.bin(BinOp::And, rflags, Value::Imm(!DF))
                };
                self.store_state(Width::W64, abi::STATE_RFLAGS, v);
            }
            _ => return Err(Unsupported),
        }
        Ok(Flow::Next)
    }

    /// `mov r/m16, Sreg`: the selector (`segment_selector`) to memory (16
    /// bits) or, with 66h, to the low 16 bits of a register. A 32- or 64-bit
    /// register destination stays unsupported: the SDM leaves its upper bits
    /// to the CPU model.
    pub(super) fn mov_from_sreg(&mut self) -> Lowered<Flow> {
        let sel = segment_selector(self.insn.op_register(1)).ok_or(Unsupported)?;
        let w = match self.insn.op_kind(0) {
            OpKind::Memory => Width::W16,
            _ if self.width_of(0)? == Width::W16 => Width::W16,
            _ => return Err(Unsupported),
        };
        self.write(0, w, Value::Imm(u64::from(sel)))?;
        Ok(Flow::Next)
    }

    /// `cpuid`: a lookup of EAX in [`abi::cpuid_leaves`] at run time. Writes EAX,
    /// EBX, ECX and EDX as 32-bit registers; ECX (the subleaf) is ignored
    /// and the flags are unchanged.
    fn cpuid(&mut self) {
        let leaf = self.read_reg(RAX, Width::W32);
        let hits: Vec<(Value, [u32; 4])> = abi::cpuid_leaves()
            .into_iter()
            .map(|(l, regs)| (self.bin(BinOp::Eq, leaf, Value::Imm(l as u64)), regs))
            .collect();
        let values: Vec<Value> = (0..4)
            .map(|r| {
                hits.iter().rev().fold(Value::Imm(0), |rest, &(hit, regs)| {
                    let v = Value::Imm(regs[r] as u64);
                    if v == rest {
                        rest
                    } else {
                        self.select(hit, v, rest)
                    }
                })
            })
            .collect();
        for (reg, v) in [RAX, RBX, RCX, RDX].into_iter().zip(values) {
            self.set_reg(reg, v);
        }
    }
}

/// The selector a segment register reads as (`translator.md`, "Segment
/// selectors"): the values of a 64-bit Linux process, which native Wine's
/// PE code sees too.
pub(crate) fn segment_selector(r: Register) -> Option<u16> {
    Some(match r {
        Register::CS => 0x33,
        Register::SS => 0x2b,
        Register::DS | Register::ES | Register::FS | Register::GS => 0,
        _ => return None,
    })
}
