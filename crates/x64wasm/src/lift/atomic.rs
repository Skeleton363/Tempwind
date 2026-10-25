//! Exchanges: `xchg`, `xadd`, `cmpxchg`. With one thread a `lock`ed
//! read-modify-write is the plain one (`abi.md`).

use iced_x86::OpKind;

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::ir::{BinOp, FlagOp, RAX, Value};

impl Lower<'_> {
    /// `xchg r/m, r`: both writes zero-extend 32-bit registers (`87 C0`
    /// zero-extends RAX; `90` is a nop and never gets here).
    pub(super) fn xchg(&mut self) -> Lowered<Flow> {
        if self.insn.op_count() != 2 {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let a = self.read(0, w)?;
        let a = self.snapshot(a);
        let b = self.read(1, w)?;
        let b = self.snapshot(b);
        self.write(0, w, b)?;
        self.write(1, w, a)?;
        Ok(Flow::Next)
    }

    /// `xadd r/m, r`: t = dest + src; src = dest; dest = t, in that order.
    pub(super) fn xadd(&mut self) -> Lowered<Flow> {
        if self.insn.op_count() != 2 {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let a = self.read(0, w)?;
        let a = self.snapshot(a);
        let b = self.read(1, w)?;
        let raw = self.bin(BinOp::Add, a, b);
        let t = self.mask(raw, w);
        self.flags(FlagOp::Add, w, a, b, t);
        if self.insn.op_kind(0) == OpKind::Memory {
            // The address may use the source register: store first.
            self.write(0, w, t)?;
            self.write(1, w, a)?;
        } else {
            self.write(1, w, a)?;
            self.write(0, w, t)?;
        }
        Ok(Flow::Next)
    }

    /// `cmpxchg r/m, r`: compares the accumulator with dest; equal: dest =
    /// src; else the accumulator = dest. The SDM's pseudocode writes dest
    /// back on a mismatch too, but the CPU leaves a 32-bit register
    /// destination's upper half then (the fixtures show it), so a register
    /// is only written on a match; memory gets its own value back.
    pub(super) fn cmpxchg(&mut self) -> Lowered<Flow> {
        if self.insn.op_count() != 2 || self.insn.op_kind(1) != OpKind::Register {
            return Err(Unsupported);
        }
        let w = self.width_of(0)?;
        let t = self.read(0, w)?;
        let t = self.snapshot(t);
        let src = self.read(1, w)?;
        let src = self.snapshot(src);
        let acc = self.read_reg(RAX, w);
        let acc = self.snapshot(acc);
        let raw = self.bin(BinOp::Sub, acc, t);
        let diff = self.mask(raw, w);
        self.flags(FlagOp::Sub, w, acc, t, diff);
        let eq = self.bin(BinOp::Eq, acc, t);
        if self.insn.op_kind(0) == OpKind::Register {
            let (g, merged) = self.merged(0, w, src)?;
            let d = self.select(eq, merged, Value::Reg(g));
            self.set_reg(g, d);
        } else {
            let d = self.select(eq, src, t);
            self.write(0, w, d)?;
        }
        // Only a mismatch writes the accumulator (RAX keeps its upper half
        // on success).
        let merged = self.merge(RAX, w, t);
        let rax = self.select(eq, Value::Reg(RAX), merged);
        self.set_reg(RAX, rax);
        Ok(Flow::Next)
    }
}
