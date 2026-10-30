//! x87 control: `fninit` (`finit` is `wait` + `fninit`; `wait` is a no-op
//! until M4 models pending x87 exceptions).

use iced_x86::Mnemonic;

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::abi;
use crate::ir::{Value, Width};

impl Lower<'_> {
    pub(super) fn x87(&mut self, m: Mnemonic) -> Lowered<Flow> {
        match m {
            Mnemonic::Wait => {}
            Mnemonic::Fninit | Mnemonic::Finit => {
                // FCW = 0x037F, FSW = 0 (TOP 0, no exceptions), every
                // register empty (the full tag word). R0-R7 keep their
                // contents.
                self.store_state(Width::W16, abi::STATE_FCW, Value::Imm(0x037f));
                self.store_state(Width::W16, abi::STATE_FSW, Value::Imm(0));
                self.store_state(Width::W16, abi::STATE_FTW, Value::Imm(0xffff));
            }
            _ => return Err(Unsupported),
        }
        Ok(Flow::Next)
    }
}
