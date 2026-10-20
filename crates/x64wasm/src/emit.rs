//! IR -> WebAssembly (memory64) code generation.
//!
//! Module layout (see `docs/design/abi.md`):
//! - imports `env.memory` (memory64), exports `run(state: i64) -> i32`;
//! - function 0 `run`: loads the guest registers from the state block into
//!   locals, dispatches to blocks, stores everything back on exit;
//! - function 1 `lookup(rip: i64) -> i32`: guest address -> block index
//!   (binary search compiled to code; -1 if there is no block);
//! - function 2 `flags(op, a, b, r) -> i64`: materializes the status flags of
//!   a lazy flag record.
//!
//! Control flow inside `run`:
//!
//! ```text
//! block $exit
//!   loop $dispatch
//!     block $bN-1 ... block $b0 block $miss
//!       br_table [$b0 .. $bN-1] default=$miss   (on local idx)
//!     end  <miss: reason = LEFT_CODE, br $exit>
//!     end  <code of block 0>
//!     ...
//!     end  <code of block N-1>
//!   end
//! end
//! <epilogue: store state, return reason>
//! ```
//!
//! Every label `$bj` with `j > k` encloses the code of block `k`, so forward
//! direct branches are plain `br $bj` (or a fall-through when `j = k + 1`).
//! Backward branches set `idx` and go through `$dispatch`; a block that
//! branches to itself is wrapped in its own `loop`. Indirect branches call
//! `lookup` and dispatch.

use std::collections::HashMap;

use wasm_encoder::{
    BlockType, CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection,
    ImportSection, IndirectNameMap, InstructionSink, MemArg, MemoryType, Module, NameMap,
    NameSection, TypeSection, ValType,
};

use crate::Options;
use crate::abi;
use crate::ir::{
    BinOp, Block, CC_EAGER, Cond, FlagOp, Op, Program, Temp, Term, Value, Width, reg_name,
};

// Locals of `run`.
const L_STATE: u32 = 0;
const L_GPR0: u32 = 1;
const L_RIP: u32 = 17;
const L_RFLAGS_IN: u32 = 18;
const L_CC_A: u32 = 19;
const L_CC_B: u32 = 20;
const L_CC_R: u32 = 21;
const L_SCRATCH: u32 = 22;
const L_TEMP0: u32 = 23;

const F_RUN: u32 = 0;
const F_LOOKUP: u32 = 1;
const F_FLAGS: u32 = 2;

fn mem(offset: u64) -> MemArg {
    MemArg {
        offset,
        align: 0,
        memory_index: 0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Label {
    Exit,
    Dispatch,
    Block(usize),
    Miss,
    SelfLoop,
    If,
}

struct RunEmitter<'p> {
    prog: &'p Program,
    index: HashMap<u64, usize>,
    labels: Vec<Label>,
    /// The lazy flag record kind known statically at this point, if any.
    known_cc: Option<(FlagOp, Width)>,
    /// The current op's result stays on the stack for the next op.
    keep: Option<Temp>,
    /// A temporary that is on top of the wasm stack instead of in its local.
    on_stack: Option<Temp>,
    temps: u32,
    l_cc_op: u32,
    l_idx: u32,
    l_reason: u32,
}

impl RunEmitter<'_> {
    fn depth(&self, label: Label) -> u32 {
        let pos = self
            .labels
            .iter()
            .rposition(|&l| l == label)
            .expect("label in scope");
        (self.labels.len() - 1 - pos) as u32
    }

    fn temp(&self, t: Temp) -> u32 {
        L_TEMP0 + t.0
    }

    fn value(&mut self, f: &mut InstructionSink<'_>, v: Value) {
        if let Value::Temp(t) = v
            && self.on_stack == Some(t)
        {
            // Left on the stack by the previous op.
            self.on_stack = None;
            return;
        }
        match v {
            Value::Reg(r) => f.local_get(L_GPR0 + r as u32),
            Value::Temp(t) => f.local_get(self.temp(t)),
            Value::Imm(x) => f.i64_const(x as i64),
        };
    }

    /// Stores an op's result, unless it is marked to stay on the stack.
    fn set_temp(&mut self, f: &mut InstructionSink<'_>, t: Temp) {
        if self.keep == Some(t) {
            self.keep = None;
            self.on_stack = Some(t);
        } else {
            f.local_set(self.temp(t));
        }
    }

    /// Emits one instruction's ops (and, for the last instruction of a
    /// block, the terminator's operand), keeping single-use temporaries on
    /// the wasm stack when the next op consumes them first.
    fn insn(&mut self, f: &mut InstructionSink<'_>, ops: &[Op], term: Option<&Term>) {
        let mut uses: HashMap<u32, u32> = HashMap::new();
        let mut count = |v: Value| {
            if let Value::Temp(t) = v {
                *uses.entry(t.0).or_default() += 1;
            }
        };
        for op in ops {
            for v in operands(op) {
                count(v);
            }
        }
        let term_value = match term {
            Some(Term::Indirect(v)) => Some(*v),
            _ => None,
        };
        if let Some(v) = term_value {
            count(v);
        }
        for (i, op) in ops.iter().enumerate() {
            // Dead flag records emit nothing, so look past them.
            let next = ops[i + 1..]
                .iter()
                .find(|o| !matches!(o, Op::Flags { live: false, .. }));
            let next_first = match next {
                Some(next) => first_operand(next),
                None => term_value,
            };
            self.keep = match (defined(op), next_first) {
                (Some(t), Some(Value::Temp(u))) if t == u && uses.get(&t.0) == Some(&1) => Some(t),
                _ => None,
            };
            self.op(f, op);
            debug_assert!(self.keep.is_none());
        }
    }

    fn op(&mut self, f: &mut InstructionSink<'_>, op: &Op) {
        match *op {
            Op::Mov { dst, src } => {
                self.value(f, src);
                self.set_temp(f, dst);
            }
            Op::Bin { dst, op, a, b } => {
                self.value(f, a);
                self.value(f, b);
                match op {
                    BinOp::Add => f.i64_add(),
                    BinOp::Sub => f.i64_sub(),
                    BinOp::And => f.i64_and(),
                    BinOp::Or => f.i64_or(),
                    BinOp::Xor => f.i64_xor(),
                    BinOp::Shl => f.i64_shl(),
                    BinOp::ShrU => f.i64_shr_u(),
                };
                self.set_temp(f, dst);
            }
            Op::Load { dst, width, addr } => {
                self.value(f, addr);
                match width {
                    Width::W8 => f.i64_load8_u(mem(0)),
                    Width::W16 => f.i64_load16_u(mem(0)),
                    Width::W32 => f.i64_load32_u(mem(0)),
                    Width::W64 => f.i64_load(mem(0)),
                };
                self.set_temp(f, dst);
            }
            Op::Store { width, addr, value } => {
                self.value(f, addr);
                self.value(f, value);
                match width {
                    Width::W8 => f.i64_store8(mem(0)),
                    Width::W16 => f.i64_store16(mem(0)),
                    Width::W32 => f.i64_store32(mem(0)),
                    Width::W64 => f.i64_store(mem(0)),
                };
            }
            Op::SetReg { reg, value } => {
                self.value(f, value);
                f.local_set(L_GPR0 + reg as u32);
            }
            Op::Flags {
                op,
                width,
                a,
                b,
                r,
                live,
            } => {
                if !live {
                    return;
                }
                f.i32_const(op.cc_op(width) as i32);
                f.local_set(self.l_cc_op);
                self.value(f, a);
                f.local_set(L_CC_A);
                self.value(f, b);
                f.local_set(L_CC_B);
                self.value(f, r);
                f.local_set(L_CC_R);
                self.known_cc = Some((op, width));
            }
        }
    }

    /// Pushes the materialized status flags (i64) of the current record.
    fn materialize_flags(&self, f: &mut InstructionSink<'_>) {
        f.local_get(self.l_cc_op);
        f.local_get(L_CC_A);
        f.local_get(L_CC_B);
        f.local_get(L_CC_R);
        f.call(F_FLAGS);
    }

    /// Pushes `cond` as an i32 (0 or 1).
    fn cond(&self, f: &mut InstructionSink<'_>, cond: Cond) {
        let base = Cond::from_index(cond as u32 & !1);
        match self.known_cc {
            Some((op, w)) => self.static_cond(f, op, w, base),
            None => {
                self.materialize_flags(f);
                f.local_set(L_SCRATCH);
                dynamic_cond(f, base);
            }
        }
        if cond.is_negated() {
            f.i32_eqz();
        }
    }

    fn zf(&self, f: &mut InstructionSink<'_>) {
        f.local_get(L_CC_R);
        f.i64_eqz();
    }

    fn sf(&self, f: &mut InstructionSink<'_>, w: Width) {
        f.local_get(L_CC_R);
        f.i64_const(w.sign_bit() as i64);
        f.i64_ge_u();
    }

    fn cf(&self, f: &mut InstructionSink<'_>, op: FlagOp) {
        match op {
            FlagOp::Add => {
                f.local_get(L_CC_R);
                f.local_get(L_CC_A);
            }
            FlagOp::Sub => {
                f.local_get(L_CC_A);
                f.local_get(L_CC_B);
            }
        }
        f.i64_lt_u();
    }

    fn of(&self, f: &mut InstructionSink<'_>, op: FlagOp, w: Width) {
        match op {
            // (a ^ r) & (b ^ r)
            FlagOp::Add => {
                f.local_get(L_CC_A);
                f.local_get(L_CC_R);
                f.i64_xor();
                f.local_get(L_CC_B);
                f.local_get(L_CC_R);
                f.i64_xor();
            }
            // (a ^ b) & (a ^ r)
            FlagOp::Sub => {
                f.local_get(L_CC_A);
                f.local_get(L_CC_B);
                f.i64_xor();
                f.local_get(L_CC_A);
                f.local_get(L_CC_R);
                f.i64_xor();
            }
        }
        f.i64_and();
        f.i64_const(w.sign_bit() as i64);
        f.i64_and();
        f.i64_const(0);
        f.i64_ne();
    }

    fn pf(&self, f: &mut InstructionSink<'_>) {
        f.local_get(L_CC_R);
        f.i64_const(0xff);
        f.i64_and();
        f.i64_popcnt();
        f.i64_const(1);
        f.i64_and();
        f.i64_eqz();
    }

    fn sext(f: &mut InstructionSink<'_>, local: u32, w: Width) {
        f.local_get(local);
        match w {
            Width::W8 => f.i64_extend8_s(),
            Width::W16 => f.i64_extend16_s(),
            Width::W32 => f.i64_extend32_s(),
            Width::W64 => f,
        };
    }

    /// A non-negated condition for a statically known record kind.
    fn static_cond(&self, f: &mut InstructionSink<'_>, op: FlagOp, w: Width, base: Cond) {
        match (op, base) {
            (_, Cond::O) => self.of(f, op, w),
            (_, Cond::B) => self.cf(f, op),
            (_, Cond::E) => self.zf(f),
            (FlagOp::Sub, Cond::Be) => {
                f.local_get(L_CC_A);
                f.local_get(L_CC_B);
                f.i64_le_u();
            }
            (_, Cond::Be) => {
                self.cf(f, op);
                self.zf(f);
                f.i32_or();
            }
            (_, Cond::S) => self.sf(f, w),
            (_, Cond::P) => self.pf(f),
            (FlagOp::Sub, Cond::L) => {
                Self::sext(f, L_CC_A, w);
                Self::sext(f, L_CC_B, w);
                f.i64_lt_s();
            }
            (FlagOp::Sub, Cond::Le) => {
                Self::sext(f, L_CC_A, w);
                Self::sext(f, L_CC_B, w);
                f.i64_le_s();
            }
            (_, Cond::L) => {
                self.sf(f, w);
                self.of(f, op, w);
                f.i32_ne();
            }
            (_, Cond::Le) => {
                self.zf(f);
                self.sf(f, w);
                self.of(f, op, w);
                f.i32_ne();
                f.i32_or();
            }
            _ => unreachable!("negated condition"),
        }
    }

    /// Transfers control to the block at `target`, or exits if there is none.
    /// `fall` is the index of the block whose code follows, if control can
    /// fall through to it from here.
    fn goto(&mut self, f: &mut InstructionSink<'_>, target: u64, k: usize, fall: Option<usize>) {
        match self.index.get(&target).copied() {
            Some(j) if Some(j) == fall => {}
            Some(j) if j == k && self.labels.contains(&Label::SelfLoop) => {
                f.br(self.depth(Label::SelfLoop));
            }
            Some(j) if j > k => {
                f.br(self.depth(Label::Block(j)));
            }
            Some(j) => {
                f.i32_const(j as i32);
                f.local_set(self.l_idx);
                f.br(self.depth(Label::Dispatch));
            }
            None => self.exit(f, abi::EXIT_LEFT_CODE, target),
        }
    }

    fn exit(&self, f: &mut InstructionSink<'_>, reason: u32, rip: u64) {
        f.i64_const(rip as i64);
        f.local_set(L_RIP);
        f.i32_const(reason as i32);
        f.local_set(self.l_reason);
        f.br(self.depth(Label::Exit));
    }

    fn block(&mut self, f: &mut InstructionSink<'_>, k: usize, block: &Block) {
        self.known_cc = None;
        let addr = block.addr;
        let self_loop = match block.term {
            Term::Jump(t) => t == addr,
            Term::Branch {
                taken, fallthrough, ..
            } => taken == addr || fallthrough == addr,
            _ => false,
        };
        if self_loop {
            f.loop_(BlockType::Empty);
            self.labels.push(Label::SelfLoop);
        }
        for (i, insn) in block.insns.iter().enumerate() {
            let last = i + 1 == block.insns.len();
            self.insn(f, &insn.ops, last.then_some(&block.term));
        }
        let next = (k + 1 < self.prog.blocks.len()).then_some(k + 1);
        match block.term {
            Term::Jump(t) => self.goto(f, t, k, next),
            Term::Branch {
                cond,
                taken,
                fallthrough,
            } => {
                // Prefer falling through to the next block on the else path.
                let (cond, taken, fallthrough) =
                    if next.is_some() && self.index.get(&taken).copied() == next {
                        (cond.negate(), fallthrough, taken)
                    } else {
                        (cond, taken, fallthrough)
                    };
                self.cond(f, cond);
                f.if_(BlockType::Empty);
                self.labels.push(Label::If);
                self.goto(f, taken, k, None);
                f.end();
                self.labels.pop();
                self.goto(f, fallthrough, k, next);
            }
            Term::Indirect(v) => {
                self.value(f, v);
                f.local_tee(L_RIP);
                f.call(F_LOOKUP);
                f.local_set(self.l_idx);
                f.br(self.depth(Label::Dispatch));
            }
            Term::Exit { reason, rip } => self.exit(f, reason, rip),
        }
        if self_loop {
            f.end();
            self.labels.pop();
        }
    }

    fn run(&mut self, f: &mut InstructionSink<'_>) {
        let n = self.prog.blocks.len();
        // Prologue.
        for i in 0..16u32 {
            f.local_get(L_STATE);
            f.i64_load(mem(abi::gpr_offset(i)));
            f.local_set(L_GPR0 + i);
        }
        f.local_get(L_STATE);
        f.i64_load(mem(abi::STATE_RFLAGS));
        f.local_tee(L_RFLAGS_IN);
        f.local_set(L_CC_A);
        f.i32_const(CC_EAGER as i32);
        f.local_set(self.l_cc_op);
        f.local_get(L_STATE);
        f.i64_load(mem(abi::STATE_RIP));
        f.local_tee(L_RIP);
        f.call(F_LOOKUP);
        f.local_set(self.l_idx);

        f.block(BlockType::Empty);
        self.labels.push(Label::Exit);
        f.loop_(BlockType::Empty);
        self.labels.push(Label::Dispatch);
        for k in (0..n).rev() {
            f.block(BlockType::Empty);
            self.labels.push(Label::Block(k));
        }
        f.block(BlockType::Empty);
        self.labels.push(Label::Miss);
        f.local_get(self.l_idx);
        let targets: Vec<u32> = (0..n).map(|k| self.depth(Label::Block(k))).collect();
        f.br_table(targets, self.depth(Label::Miss));
        f.end();
        self.labels.pop();
        // Miss: RIP holds an address without a block.
        f.i32_const(abi::EXIT_LEFT_CODE as i32);
        f.local_set(self.l_reason);
        f.br(self.depth(Label::Exit));
        let prog = self.prog;
        for (k, block) in prog.blocks.iter().enumerate() {
            f.end();
            assert_eq!(self.labels.pop(), Some(Label::Block(k)));
            self.block(f, k, block);
        }
        f.end(); // loop
        self.labels.pop();
        f.unreachable();
        f.end(); // exit block
        self.labels.pop();

        // Epilogue.
        for i in 0..16u32 {
            f.local_get(L_STATE);
            f.local_get(L_GPR0 + i);
            f.i64_store(mem(abi::gpr_offset(i)));
        }
        f.local_get(L_STATE);
        f.local_get(L_RIP);
        f.i64_store(mem(abi::STATE_RIP));
        f.local_get(L_STATE);
        f.local_get(L_RFLAGS_IN);
        f.i64_const(!abi::flags::STATUS as i64);
        f.i64_and();
        self.materialize_flags(f);
        f.i64_or();
        f.i64_store(mem(abi::STATE_RFLAGS));
        f.local_get(L_STATE);
        f.local_get(self.l_reason);
        f.i32_store(mem(abi::STATE_EXIT));
        f.local_get(self.l_reason);
        f.end();
    }
}

/// The temporary an op defines.
fn defined(op: &Op) -> Option<Temp> {
    match *op {
        Op::Mov { dst, .. } | Op::Bin { dst, .. } | Op::Load { dst, .. } => Some(dst),
        _ => None,
    }
}

/// The operand an op pushes first (nothing before it stays on the stack).
fn first_operand(op: &Op) -> Option<Value> {
    match *op {
        Op::Mov { src, .. } => Some(src),
        Op::Bin { a, .. } => Some(a),
        Op::Load { addr, .. } | Op::Store { addr, .. } => Some(addr),
        Op::SetReg { value, .. } => Some(value),
        Op::Flags { a, live: true, .. } => Some(a),
        Op::Flags { live: false, .. } => None,
    }
}

/// Every operand an op reads.
fn operands(op: &Op) -> Vec<Value> {
    match *op {
        Op::Mov { src, .. } => vec![src],
        Op::Bin { a, b, .. } => vec![a, b],
        Op::Load { addr, .. } => vec![addr],
        Op::Store { addr, value, .. } => vec![addr, value],
        Op::SetReg { value, .. } => vec![value],
        Op::Flags { a, b, r, live, .. } => {
            if live {
                vec![a, b, r]
            } else {
                Vec::new()
            }
        }
    }
}

/// Pushes a non-negated condition from materialized flags in `L_SCRATCH`.
fn dynamic_cond(f: &mut InstructionSink<'_>, base: Cond) {
    use abi::flags::*;
    let bit = |f: &mut InstructionSink<'_>, mask: u64| {
        f.local_get(L_SCRATCH);
        f.i64_const(mask as i64);
        f.i64_and();
        f.i64_const(0);
        f.i64_ne();
    };
    match base {
        Cond::O => bit(f, OF),
        Cond::B => bit(f, CF),
        Cond::E => bit(f, ZF),
        Cond::Be => bit(f, CF | ZF),
        Cond::S => bit(f, SF),
        Cond::P => bit(f, PF),
        Cond::L => {
            bit(f, SF);
            bit(f, OF);
            f.i32_ne();
        }
        Cond::Le => {
            bit(f, ZF);
            bit(f, SF);
            bit(f, OF);
            f.i32_ne();
            f.i32_or();
        }
        _ => unreachable!("negated condition"),
    }
}

/// `lookup(rip: i64) -> i32`: binary search over the sorted block addresses.
fn emit_lookup(addrs: &[u64]) -> Function {
    fn search(f: &mut InstructionSink<'_>, addrs: &[u64], lo: usize, hi: usize) {
        if hi - lo <= 4 {
            for (i, &addr) in addrs.iter().enumerate().take(hi).skip(lo) {
                f.local_get(0);
                f.i64_const(addr as i64);
                f.i64_eq();
                f.if_(BlockType::Empty);
                f.i32_const(i as i32);
                f.return_();
                f.end();
            }
        } else {
            let mid = (lo + hi) / 2;
            f.local_get(0);
            f.i64_const(addrs[mid] as i64);
            f.i64_lt_u();
            f.if_(BlockType::Empty);
            search(f, addrs, lo, mid);
            f.else_();
            search(f, addrs, mid, hi);
            f.end();
        }
    }
    let mut func = Function::new([]);
    let mut f = func.instructions();
    search(&mut f, addrs, 0, addrs.len());
    f.i32_const(-1);
    f.end();
    func
}

/// `flags(op: i32, a: i64, b: i64, r: i64) -> i64`: the status flags of a
/// lazy record (operands zero-extended to the record's width).
fn emit_flags() -> Function {
    use abi::flags::*;
    const OP: u32 = 0;
    const A: u32 = 1;
    const B: u32 = 2;
    const R: u32 = 3;
    const SIGN: u32 = 4;
    const CF_OF: u32 = 5;
    let mut func = Function::new([(2, ValType::I64)]);
    let mut f = func.instructions();
    // Eager: the flags themselves are in `a`.
    f.local_get(OP);
    f.i32_eqz();
    f.if_(BlockType::Empty);
    f.local_get(A);
    f.i64_const(STATUS as i64);
    f.i64_and();
    f.return_();
    f.end();
    // sign = 1 << (8 << width) - 1, width = (op - 1) & 3
    f.i64_const(1);
    f.i64_const(8);
    f.local_get(OP);
    f.i32_const(1);
    f.i32_sub();
    f.i32_const(3);
    f.i32_and();
    f.i64_extend_i32_u();
    f.i64_shl();
    f.i64_const(1);
    f.i64_sub();
    f.i64_shl();
    f.local_set(SIGN);
    // kind = (op - 1) >> 2: 0 = add, 1 = sub. CF | OF:
    f.local_get(OP);
    f.i32_const(1);
    f.i32_sub();
    f.i32_const(2);
    f.i32_shr_u();
    f.if_(BlockType::Result(ValType::I64));
    {
        // sub: CF = a < b, OF = (a ^ b) & (a ^ r)
        f.local_get(A);
        f.local_get(B);
        f.i64_lt_u();
        f.i64_extend_i32_u();
        f.local_get(A);
        f.local_get(B);
        f.i64_xor();
        f.local_get(A);
        f.local_get(R);
        f.i64_xor();
        f.i64_and();
        f.local_get(SIGN);
        f.i64_and();
        f.i64_eqz();
        f.i32_eqz();
        f.i64_extend_i32_u();
        f.i64_const(11);
        f.i64_shl();
        f.i64_or();
    }
    f.else_();
    {
        // add: CF = r < a, OF = (a ^ r) & (b ^ r)
        f.local_get(R);
        f.local_get(A);
        f.i64_lt_u();
        f.i64_extend_i32_u();
        f.local_get(A);
        f.local_get(R);
        f.i64_xor();
        f.local_get(B);
        f.local_get(R);
        f.i64_xor();
        f.i64_and();
        f.local_get(SIGN);
        f.i64_and();
        f.i64_eqz();
        f.i32_eqz();
        f.i64_extend_i32_u();
        f.i64_const(11);
        f.i64_shl();
        f.i64_or();
    }
    f.end();
    f.local_set(CF_OF);
    // PF
    f.local_get(R);
    f.i64_const(0xff);
    f.i64_and();
    f.i64_popcnt();
    f.i64_const(1);
    f.i64_and();
    f.i64_const(1);
    f.i64_xor();
    f.i64_const(2);
    f.i64_shl();
    // AF
    f.local_get(A);
    f.local_get(B);
    f.i64_xor();
    f.local_get(R);
    f.i64_xor();
    f.i64_const(AF as i64);
    f.i64_and();
    f.i64_or();
    // ZF
    f.local_get(R);
    f.i64_eqz();
    f.i64_extend_i32_u();
    f.i64_const(6);
    f.i64_shl();
    f.i64_or();
    // SF
    f.local_get(R);
    f.local_get(SIGN);
    f.i64_and();
    f.i64_eqz();
    f.i32_eqz();
    f.i64_extend_i32_u();
    f.i64_const(7);
    f.i64_shl();
    f.i64_or();
    f.local_get(CF_OF);
    f.i64_or();
    f.end();
    func
}

/// Encodes `prog` as a module following the ABI.
pub fn emit(prog: &Program, opts: &Options) -> Vec<u8> {
    let mut module = Module::new();

    let mut types = TypeSection::new();
    types.ty().function([ValType::I64], [ValType::I32]); // 0: run, lookup
    types.ty().function(
        [ValType::I32, ValType::I64, ValType::I64, ValType::I64],
        [ValType::I64],
    ); // 1: flags
    module.section(&types);

    let mut imports = ImportSection::new();
    imports.import(
        abi::MEMORY_MODULE,
        abi::MEMORY_NAME,
        EntityType::Memory(MemoryType {
            minimum: 1,
            maximum: opts.shared_memory.then_some(abi::MAX_MEMORY_PAGES),
            memory64: true,
            shared: opts.shared_memory,
            page_size_log2: None,
        }),
    );
    module.section(&imports);

    let mut funcs = FunctionSection::new();
    funcs.function(0);
    funcs.function(0);
    funcs.function(1);
    module.section(&funcs);

    let mut exports = ExportSection::new();
    exports.export(abi::RUN_EXPORT, ExportKind::Func, F_RUN);
    module.section(&exports);

    let temps = prog
        .blocks
        .iter()
        .flat_map(|b| b.insns.iter().map(|i| i.temps))
        .max()
        .unwrap_or(0);
    let l_cc_op = L_TEMP0 + temps;
    let mut run = Function::new([
        (L_TEMP0 - 1 + temps, ValType::I64),
        (3, ValType::I32),
    ]);
    let mut emitter = RunEmitter {
        prog,
        index: prog.index_map(),
        labels: Vec::new(),
        known_cc: None,
        keep: None,
        on_stack: None,
        temps,
        l_cc_op,
        l_idx: l_cc_op + 1,
        l_reason: l_cc_op + 2,
    };
    emitter.run(&mut run.instructions());

    let addrs: Vec<u64> = prog.blocks.iter().map(|b| b.addr).collect();
    let mut code = CodeSection::new();
    code.function(&run);
    code.function(&emit_lookup(&addrs));
    code.function(&emit_flags());
    module.section(&code);

    module.section(&names(&emitter));
    module.finish()
}

fn names(e: &RunEmitter<'_>) -> NameSection {
    let mut names = NameSection::new();
    let mut funcs = NameMap::new();
    funcs.append(F_RUN, "run");
    funcs.append(F_LOOKUP, "lookup");
    funcs.append(F_FLAGS, "flags");
    names.functions(&funcs);

    let mut run = NameMap::new();
    run.append(L_STATE, "state");
    for i in 0..16u8 {
        run.append(L_GPR0 + i as u32, reg_name(i));
    }
    run.append(L_RIP, "rip");
    run.append(L_RFLAGS_IN, "rflags_in");
    run.append(L_CC_A, "cc_a");
    run.append(L_CC_B, "cc_b");
    run.append(L_CC_R, "cc_r");
    run.append(L_SCRATCH, "scratch");
    for t in 0..e.temps {
        run.append(L_TEMP0 + t, &format!("t{t}"));
    }
    run.append(e.l_cc_op, "cc_op");
    run.append(e.l_idx, "idx");
    run.append(e.l_reason, "reason");
    let mut lookup = NameMap::new();
    lookup.append(0, "rip");
    let mut flags = NameMap::new();
    for (i, n) in ["op", "a", "b", "r", "sign", "cf_of"].iter().enumerate() {
        flags.append(i as u32, n);
    }
    let mut locals = IndirectNameMap::new();
    locals.append(F_RUN, &run);
    locals.append(F_LOOKUP, &lookup);
    locals.append(F_FLAGS, &flags);
    names.locals(&locals);
    names
}
