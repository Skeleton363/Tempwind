//! Flag and no-op instructions: `ud2`, `cmc`, `clc`, `stc`, `std`, `cld`;
//! the nop forms, `pause`, `prefetch*` (no access, never fault).

use iced_x86::Mnemonic;

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::abi::{self, flags::CF, flags::DF};
use crate::ir::{BinOp, Term, Value, Width};

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
}
