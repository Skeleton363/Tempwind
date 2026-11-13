//! Stack: `push`, `pop`, `pushfq`, `popfq`, `enter`.

use iced_x86::Code;

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::abi::{self, flags};
use crate::ir::{self, BinOp, RSP, Value, Width};

/// RBP (encoding 5).
const RBP: u8 = 5;
/// RFLAGS.RF and RFLAGS.VM, which pushfq stores as 0.
const RF: u64 = 1 << 16;
const VM: u64 = 1 << 17;
/// RFLAGS.NT and RFLAGS.ID: plain bits in 64-bit user mode, kept by popfq.
const NT: u64 = 1 << 14;
const ID: u64 = 1 << 21;
/// RFLAGS.AC: with TF, a bit popfq stops at (single-step traps and
/// alignment checks aren't modelled).
const AC: u64 = 1 << 18;

impl Lower<'_> {
    pub(super) fn push(&mut self) -> Lowered<Flow> {
        let v = match self.insn.code() {
            Code::Push_r64 | Code::Pushq_imm8 | Code::Pushq_imm32 | Code::Push_rm64 => {
                self.read(0, Width::W64)?
            }
            _ => return Err(Unsupported),
        };
        self.push64(v);
        Ok(Flow::Next)
    }

    pub(super) fn pop(&mut self) -> Lowered<Flow> {
        if !matches!(self.insn.code(), Code::Pop_r64 | Code::Pop_rm64) {
            return Err(Unsupported);
        }
        let v = self.load(Width::W64, Value::Reg(RSP));
        let sp = self.bin(BinOp::Add, Value::Reg(RSP), Value::Imm(8));
        self.set_reg(RSP, sp);
        // A memory destination's address uses RSP after the increment.
        self.write(0, Width::W64, v)?;
        Ok(Flow::Next)
    }
}

impl Lower<'_> {
    /// `pushfq`: RFLAGS with the status flags of the flag record; RF and VM
    /// are 0 in the image (`translator.md`, "RFLAGS images").
    pub(super) fn pushfq(&mut self) -> Lowered<Flow> {
        let state = self.load_state(Width::W64, abi::STATE_RFLAGS);
        let kept = self.bin(BinOp::And, state, Value::Imm(!(flags::STATUS | RF | VM)));
        let status = self.read_flags();
        let image = self.bin(BinOp::Or, kept, status);
        self.push64(image);
        Ok(Flow::Next)
    }

    /// `popfq`: the status flags, DF, NT and ID from the stack; every other
    /// bit of RFLAGS stays. An image with TF or AC set exits with
    /// `EXIT_UNSUPPORTED` before any change (`translator.md`, "RFLAGS
    /// images").
    pub(super) fn popfq(&mut self) -> Lowered<Flow> {
        let v = self.load(Width::W64, Value::Reg(RSP));
        let stop = self.bin(BinOp::And, v, Value::Imm(flags::TF | AC));
        self.fault_if(stop, ir::UNSUPPORTED);
        let sp = self.bin(BinOp::Add, Value::Reg(RSP), Value::Imm(8));
        self.set_reg(RSP, sp);
        let status = self.bin(BinOp::And, v, Value::Imm(flags::STATUS));
        self.flags_update(flags::STATUS, status, 0, None);
        let loaded = flags::DF | NT | ID;
        let state = self.load_state(Width::W64, abi::STATE_RFLAGS);
        let kept = self.bin(BinOp::And, state, Value::Imm(!loaded));
        let bits = self.bin(BinOp::And, v, Value::Imm(loaded));
        let rflags = self.bin(BinOp::Or, kept, bits);
        self.store_state(Width::W64, abi::STATE_RFLAGS, rflags);
        Ok(Flow::Next)
    }

    /// `enter imm16, imm8` (64-bit): push RBP; copy level − 1 frame
    /// pointers from below the old RBP; push the new frame pointer if the
    /// level is not 0; RBP = the frame; RSP −= size. The level is imm8 mod
    /// 32.
    pub(super) fn enter(&mut self) -> Lowered<Flow> {
        if self.insn.code() != Code::Enterq_imm16_imm8 {
            return Err(Unsupported);
        }
        let size = u64::from(self.insn.immediate16());
        let level = u64::from(self.insn.immediate8_2nd() % 32);
        let rbp = self.snapshot(Value::Reg(RBP));
        let pushes = if level == 0 { 1 } else { level + 1 };
        // Every address first: the memory checks go before the first store.
        let slots: Vec<Value> = (1..=pushes)
            .map(|k| self.bin(BinOp::Sub, Value::Reg(RSP), Value::Imm(8 * k)))
            .collect();
        let copies: Vec<Value> = (1..level)
            .map(|i| self.bin(BinOp::Sub, rbp, Value::Imm(8 * i)))
            .collect();
        for &a in &copies {
            self.mem_check(a, 8, false);
        }
        for &a in &slots {
            self.mem_check(a, 8, true);
        }
        let frame = slots[0];
        self.store_unchecked(Width::W64, frame, rbp);
        for (i, &from) in copies.iter().enumerate() {
            let v = self.load_unchecked(Width::W64, from);
            self.store_unchecked(Width::W64, slots[i + 1], v);
        }
        if level > 0 {
            self.store_unchecked(Width::W64, slots[level as usize], frame);
        }
        self.set_reg(RBP, frame);
        let sp = self.bin(BinOp::Sub, slots[pushes as usize - 1], Value::Imm(size));
        self.set_reg(RSP, sp);
        Ok(Flow::Next)
    }
}
