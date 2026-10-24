//! String instructions (`movs`, `stos`, `lods`) with their `rep` loops
//! (`docs/design/translator.md`, "Integer results").

use wasm_encoder::{BlockType, InstructionSink};

use super::{ChunkEmitter, L_GPR0, L_SCRATCH, Label, load, mem, store};
use crate::ir::{StrOp, Value, Width};

const RAX: u32 = L_GPR0;
const RCX: u32 = L_GPR0 + 1;
const RSI: u32 = L_GPR0 + 6;
const RDI: u32 = L_GPR0 + 7;

/// Above this count the fast paths' byte length could wrap; such a count
/// faults anyway, in the element loop.
const FAST_MAX: i64 = 1 << 40;

fn advance(f: &mut InstructionSink<'_>, reg: u32) {
    f.local_get(reg);
    f.local_get(L_SCRATCH);
    f.i64_add();
    f.local_set(reg);
}

impl ChunkEmitter<'_> {
    pub(super) fn rep_string(
        &mut self,
        f: &mut InstructionSink<'_>,
        op: StrOp,
        width: Width,
        rep: bool,
        delta: Value,
    ) {
        self.value(f, delta);
        f.local_set(L_SCRATCH);
        if !rep {
            Self::iteration(f, op, width);
            return;
        }
        let size = (width.bits() / 8) as i64;
        f.block(BlockType::Empty);
        self.labels.push(Label::StrDone);
        let fast = match op {
            StrOp::Movs => true,
            StrOp::Stos => width == Width::W8,
            StrOp::Lods => false,
        };
        if fast {
            // DF = 0 and a count whose byte length can't wrap.
            f.local_get(L_SCRATCH);
            f.i64_const(0);
            f.i64_gt_s();
            f.local_get(RCX);
            f.i64_const(FAST_MAX);
            f.i64_lt_u();
            f.i32_and();
            if op == StrOp::Movs {
                // No overlap ahead of the source: a forward copy is a memmove.
                f.local_get(RDI);
                f.local_get(RSI);
                f.i64_sub();
                f.local_get(RCX);
                f.i64_const(size);
                f.i64_mul();
                f.i64_ge_u();
                f.i32_and();
            }
            f.if_(BlockType::Empty);
            self.labels.push(Label::If);
            // n = rcx * size, in the scratch local (delta is no longer needed).
            f.local_get(RCX);
            f.i64_const(size);
            f.i64_mul();
            f.local_set(L_SCRATCH);
            f.local_get(RDI);
            if op == StrOp::Movs {
                f.local_get(RSI);
                f.local_get(L_SCRATCH);
                f.memory_copy(0, 0);
                advance(f, RSI);
            } else {
                f.local_get(RAX);
                f.i32_wrap_i64();
                f.local_get(L_SCRATCH);
                f.memory_fill(0);
            }
            advance(f, RDI);
            f.i64_const(0);
            f.local_set(RCX);
            f.br(self.depth(Label::StrDone));
            f.end();
            self.labels.pop();
        }
        f.loop_(BlockType::Empty);
        self.labels.push(Label::StrLoop);
        f.local_get(RCX);
        f.i64_eqz();
        f.br_if(self.depth(Label::StrDone));
        Self::iteration(f, op, width);
        f.local_get(RCX);
        f.i64_const(1);
        f.i64_sub();
        f.local_set(RCX);
        f.br(self.depth(Label::StrLoop));
        f.end();
        self.labels.pop();
        f.end();
        self.labels.pop();
    }

    /// One iteration; the scratch local holds delta.
    fn iteration(f: &mut InstructionSink<'_>, op: StrOp, width: Width) {
        match op {
            StrOp::Movs => {
                f.local_get(RDI);
                f.local_get(RSI);
                load(f, width, mem(0));
                store(f, width, mem(0));
                advance(f, RSI);
                advance(f, RDI);
            }
            StrOp::Stos => {
                f.local_get(RDI);
                f.local_get(RAX);
                store(f, width, mem(0));
                advance(f, RDI);
            }
            StrOp::Lods => {
                match width {
                    Width::W64 | Width::W32 => {
                        f.local_get(RSI);
                        load(f, width, mem(0));
                    }
                    w => {
                        f.local_get(RAX);
                        f.i64_const(!w.mask() as i64);
                        f.i64_and();
                        f.local_get(RSI);
                        load(f, w, mem(0));
                        f.i64_or();
                    }
                }
                f.local_set(RAX);
                advance(f, RSI);
            }
        }
    }
}
