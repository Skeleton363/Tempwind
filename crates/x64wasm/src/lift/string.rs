//! String instructions: `movs`, `stos`, `lods`, with or without `rep`.
//! The 67h forms (ESI, EDI, ECX) stay unsupported.

use iced_x86::{Mnemonic, OpKind, Register};

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::abi;
use crate::ir::{BinOp, Op, StrOp, Value, Width};

impl Lower<'_> {
    pub(super) fn string(&mut self, m: Mnemonic) -> Lowered<Flow> {
        let insn = self.insn;
        let op = match m {
            Mnemonic::Movsb | Mnemonic::Movsw | Mnemonic::Movsd | Mnemonic::Movsq => StrOp::Movs,
            Mnemonic::Stosb | Mnemonic::Stosw | Mnemonic::Stosd | Mnemonic::Stosq => StrOp::Stos,
            Mnemonic::Lodsb | Mnemonic::Lodsw | Mnemonic::Lodsd | Mnemonic::Lodsq => StrOp::Lods,
            _ => return Err(Unsupported),
        };
        for i in 0..insn.op_count() {
            match insn.op_kind(i) {
                OpKind::Register | OpKind::MemoryESRDI => {}
                OpKind::MemorySegRSI => {
                    if matches!(insn.memory_segment(), Register::FS | Register::GS) {
                        return Err(Unsupported);
                    }
                }
                _ => return Err(Unsupported),
            }
        }
        let width = Width::from_bytes(insn.memory_size().size()).ok_or(Unsupported)?;
        let size = (width.bits() / 8) as u64;
        // delta = DF ? -size : +size, from the state's RFLAGS.
        let rflags = self.load_state(Width::W64, abi::STATE_RFLAGS);
        let df = self.bin(BinOp::And, rflags, Value::Imm(abi::flags::DF));
        let delta = self.select(df, Value::Imm(size.wrapping_neg()), Value::Imm(size));
        self.push_op(Op::RepString {
            op,
            width,
            rep: insn.has_rep_prefix() || insn.has_repne_prefix(),
            delta,
        });
        Ok(Flow::Next)
    }
}
