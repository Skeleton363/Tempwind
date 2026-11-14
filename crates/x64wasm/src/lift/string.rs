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
        self.string_checks(op, size, rep, delta, df);
        self.push_op(Op::RepString {
            op,
            width,
            rep,
            delta,
        });
        Ok(Flow::Next)
    }

    /// The memory checks of a string instruction: the first element at RSI
    /// and RDI. A `rep` range (when RCX != 0) checks NULL at its last
    /// element too, at `ptr + (RCX - 1) * delta`, and the store check tests
    /// every page from its lowest byte to its highest (`translator.md`,
    /// "Memory checks"). `repe`/`repne` may stop early, so they check only
    /// the first element.
    fn string_checks(&mut self, op: StrOp, size: u64, rep: Rep, delta: Value, df: Value) {
        let ptrs: &[(Gpr, bool)] = match op {
            StrOp::Movs => &[(RSI, false), (RDI, true)],
            StrOp::Stos => &[(RDI, true)],
            StrOp::Lods => &[(RSI, false)],
            StrOp::Cmps => &[(RSI, false), (RDI, false)],
            StrOp::Scas => &[(RDI, false)],
        };
        let (null, store) = (self.checks.null, self.checks.store);
        if !ptrs.iter().any(|&(_, w)| null || (w && store)) {
            return;
        }
        let when = (rep != Rep::Once).then_some(Value::Reg(RCX));
        if rep != Rep::Rep {
            for &(reg, write) in ptrs {
                self.mem_check_when(Value::Reg(reg), size as u32, write, when);
            }
            return;
        }
        let n = self.bin(BinOp::Sub, Value::Reg(RCX), Value::Imm(1));
        let span = self.bin(BinOp::Mul, n, delta);
        for &(reg, write) in ptrs {
            let p = Value::Reg(reg);
            let last = self.bin(BinOp::Add, p, span);
            for at in [p, last] {
                self.mem_check_kinds(at, size as u32, write, when, null, false);
            }
            if write && store {
                // DF = 1 runs from `p` down to `last`.
                let lo = self.select(df, last, p);
                let top = self.select(df, p, last);
                let hi = match size {
                    1 => top,
                    _ => self.bin(BinOp::Add, top, Value::Imm(size - 1)),
                };
                self.page_range_check(lo, hi, when);
            }
        }
    }
}
