//! String instructions: `movs`, `stos`, `lods`, `cmps`, `scas`, with or
//! without `rep`/`repe`/`repne`.
//! The 67h forms (ESI, EDI, ECX) stay unsupported.

use iced_x86::{Mnemonic, OpKind, Register};

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::abi;
use crate::ir::{BinOp, Gpr, Op, RCX, RDI, RSI, Rep, StrOp, Value, Width};

impl Lower<'_> {
    pub(super) fn string(&mut self, m: Mnemonic) -> Lowered<Flow> {
        let insn = self.insn;
        let op = match m {
            Mnemonic::Movsb | Mnemonic::Movsw | Mnemonic::Movsd | Mnemonic::Movsq => StrOp::Movs,
            Mnemonic::Stosb | Mnemonic::Stosw | Mnemonic::Stosd | Mnemonic::Stosq => StrOp::Stos,
            Mnemonic::Lodsb | Mnemonic::Lodsw | Mnemonic::Lodsd | Mnemonic::Lodsq => StrOp::Lods,
            Mnemonic::Cmpsb | Mnemonic::Cmpsw | Mnemonic::Cmpsd | Mnemonic::Cmpsq => StrOp::Cmps,
            Mnemonic::Scasb | Mnemonic::Scasw | Mnemonic::Scasd | Mnemonic::Scasq => StrOp::Scas,
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
        // F2 on movs, stos and lods repeats as F3 does.
        let rep = match (
            insn.has_rep_prefix(),
            insn.has_repne_prefix(),
            op.compares(),
        ) {
            (false, false, _) => Rep::Once,
            (_, _, false) => Rep::Rep,
            (true, _, true) => Rep::Repe,
            (false, true, true) => Rep::Repne,
        };
        self.string_checks(op, size, rep, delta);
        self.push_op(Op::RepString {
            op,
            width,
            rep,
            delta,
        });
        Ok(Flow::Next)
    }

    /// The memory checks of a string instruction: the first element at RSI
    /// and RDI, and for `rep` (when RCX != 0) the last one too, at
    /// `ptr + (RCX - 1) * delta`. `repe`/`repne` may stop early, so they
    /// check only the first.
    fn string_checks(&mut self, op: StrOp, size: u64, rep: Rep, delta: Value) {
        let ptrs: &[(Gpr, bool)] = match op {
            StrOp::Movs => &[(RSI, false), (RDI, true)],
            StrOp::Stos => &[(RDI, true)],
            StrOp::Lods => &[(RSI, false)],
            StrOp::Cmps => &[(RSI, false), (RDI, false)],
            StrOp::Scas => &[(RDI, false)],
        };
        let wanted = |w: bool| self.checks.null || (w && self.checks.store);
        if !ptrs.iter().any(|&(_, w)| wanted(w)) {
            return;
        }
        let size = size as u32;
        let when = (rep != Rep::Once).then_some(Value::Reg(RCX));
        let span = (rep == Rep::Rep).then(|| {
            let n = self.bin(BinOp::Sub, Value::Reg(RCX), Value::Imm(1));
            self.bin(BinOp::Mul, n, delta)
        });
        for &(reg, write) in ptrs {
            let p = Value::Reg(reg);
            self.mem_check_when(p, size, write, when);
            if let Some(span) = span {
                let last = self.bin(BinOp::Add, p, span);
                self.mem_check_when(last, size, write, when);
            }
        }
    }
}
