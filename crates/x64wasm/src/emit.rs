//! IR -> WebAssembly (memory64) code generation, one wasm function per
//! chunk (ABI v3, `docs/design/abi.md`; design in `docs/design/translator.md`).
//!
//! Module layout:
//! - imports, in order: `env.memory` (memory64), `env.table` (the
//!   process-wide funcref table), the globals `env.table_base` (i32),
//!   `env.lookup` (i64, the lookup header's address) and `env.depth`
//!   (mutable i32, nested guest calls); exports `run(state: i64) -> i32`;
//! - function 0 `run`: the trampoline. Looks up `state.rip`, calls the chunk
//!   through the table, and loops on `STATUS_UNWIND`;
//! - function 1 `lookup(rip: i64) -> (i32, i32)`: guest address -> (table
//!   index, entry), a probe of the hash table in guest memory; (-1, 0) if
//!   there is no block;
//! - function 2 `flags(op, a, b, r) -> i64`: materializes the status flags of
//!   a lazy flag record;
//! - functions 3.. : one per chunk, `(state: i64, entry: i32, ret: i64) -> i32`,
//!   at table index `table_base` + function index - 3 (an active element
//!   segment at `global.get table_base`);
//! - the custom section `x64wasm.blocks` lists every block for the host.
//!
//! Transfers to addresses outside the module, and all indirect ones, probe
//! the lookup table and use `call_indirect` / `return_call_indirect`; a miss
//! unwinds to the trampoline.
//!
//! Control flow inside a chunk:
//!
//! ```text
//! <load registers and the lazy flag record from the state block>
//! block $leave
//!   loop $dispatch
//!     block $bN-1 ... block $b0 block $miss
//!       br_table [$b0 .. $bN-1] default=$miss   (on the `entry` parameter)
//!     end  unreachable
//!     end  <code of block 0>
//!     ...
//!     end  <code of block N-1>
//!   end
//!   unreachable
//! end
//! <store registers and flags; RIP unless RETURNED; return status>
//! ```
//!
//! Every label `$bj` with `j > k` encloses the code of block `k`, so forward
//! branches inside the chunk are plain `br $bj` (or a fall-through when
//! `j = k + 1`). Backward branches set `entry` and go through `$dispatch`; a
//! block that branches to itself is wrapped in its own `loop`. Returns,
//! unwinds and exits set `status` (and `rip`) and leave through `$leave`.
//! Calls and tail transfers store the state inline and use `call` /
//! `call_indirect` and `return_call` / `return_call_indirect`.

use std::borrow::Cow;
use std::collections::HashMap;

use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, CustomSection, ElementSection, Elements, EntityType,
    ExportKind, ExportSection, Function, FunctionSection, GlobalType, ImportSection,
    IndirectNameMap, InstructionSink, MemArg, MemoryType, Module, NameMap, NameSection, RefType,
    TableType, TypeSection, ValType,
};

use crate::Options;
use crate::abi;
use crate::func::Partition;
use crate::ir::{
    BinOp, Block, CC_EAGER, CallTarget, Op, Program, Temp, Term, UnOp, VTemp, Value, Width,
    reg_name,
};
use crate::link::{BlockEntry, encode_blocks};

mod flags;
mod helpers;
mod simd;
mod string;

use flags::{Known, emit_flags};

// Parameters and locals of a chunk function.
const P_STATE: u32 = 0;
/// The block to start at; reused as the dispatch index.
const P_ENTRY: u32 = 1;
/// The return address the wasm caller expects.
const P_RET: u32 = 2;
const L_GPR0: u32 = 3;
const L_RIP: u32 = 19;
const L_CC_A: u32 = 20;
const L_CC_B: u32 = 21;
const L_CC_R: u32 = 22;
const L_SCRATCH: u32 = 23;
const L_TEMP0: u32 = 24;

const F_RUN: u32 = 0;
const F_LOOKUP: u32 = 1;
const F_FLAGS: u32 = 2;
/// Function index of chunk 0.
pub const F_CHUNK0: u32 = 3;

const T_RUN: u32 = 0;
const T_FLAGS: u32 = 1;
const T_CHUNK: u32 = 2;
const T_LOOKUP: u32 = 3;
/// Only present when a helper uses it.
const T_DIV128: u32 = 4;

const TABLE: u32 = 0;
const G_TABLE_BASE: u32 = 0;
const G_LOOKUP: u32 = 1;
const G_DEPTH: u32 = 2;

fn mem(offset: u64) -> MemArg {
    MemArg {
        offset,
        align: 0,
        memory_index: 0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Label {
    Leave,
    Dispatch,
    Block(usize),
    Miss,
    SelfLoop,
    If,
    /// Labels of a string instruction's loop.
    StrDone,
    StrLoop,
}

struct ChunkEmitter<'p> {
    prog: &'p Program,
    part: &'p Partition,
    index: &'p HashMap<u64, usize>,
    /// This chunk's index (its table index).
    chunk: u32,
    /// Program block indices of this chunk, in entry order.
    blocks: &'p [usize],
    depth_limit: Option<u32>,
    labels: Vec<Label>,
    /// The lazy flag record kind known statically at this point, if any.
    known_cc: Option<Known>,
    /// The current op's result stays on the stack for the next op.
    keep: Option<Temp>,
    /// A temporary that is on top of the wasm stack instead of in its local.
    on_stack: Option<Temp>,
    l_cc_op: u32,
    l_status: u32,
    l_func: u32,
    /// The first v128 local (vector temporaries).
    l_v0: u32,
    /// Function index of the `div128` helper, if the module has it.
    f_div128: Option<u32>,
    /// Address of the instruction being emitted.
    cur_addr: u64,
    /// Something branches to `$leave`.
    leave_used: bool,
}

impl ChunkEmitter<'_> {
    fn depth(&self, label: Label) -> u32 {
        // The outer labels sit at fixed stack positions (Leave, Dispatch,
        // Block(N-1) .. Block(0), Miss); only the few labels inside a block
        // are searched.
        let n = self.blocks.len();
        let pos = match label {
            Label::Leave => 0,
            Label::Dispatch => 1,
            Label::Block(j) => 2 + (n - 1 - j),
            _ => self
                .labels
                .iter()
                .rposition(|&l| l == label)
                .expect("label in scope"),
        };
        debug_assert_eq!(self.labels.get(pos), Some(&label), "label in scope");
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
        let mut uses = [0u8; 32];
        let mut count = |v: Value| {
            if let Value::Temp(t) = v
                && let Some(u) = uses.get_mut(t.0 as usize)
            {
                *u = u.saturating_add(1);
            }
        };
        for op in ops {
            for v in operands(op).into_iter().flatten() {
                count(v);
            }
        }
        // The value a terminator pushes first.
        let term_value = match term {
            Some(Term::Indirect(v) | Term::Ret(v)) => Some(*v),
            Some(Term::BranchIf { value, .. }) => Some(*value),
            Some(Term::Call {
                target: CallTarget::Indirect(v),
                ..
            }) => Some(*v),
            _ => None,
        };
        if let Some(v) = term_value {
            count(v);
        }
        for (i, op) in ops.iter().enumerate() {
            // Dead flag records emit nothing, so look past them.
            let next = ops[i + 1..]
                .iter()
                .find(|o| {
                    !matches!(
                        o,
                        Op::Flags { live: false, .. } | Op::FlagsUpdate { live: false, .. }
                    )
                });
            let next_first = match next {
                Some(next) => first_operand(next),
                None => term_value,
            };
            self.keep = match (defined(op), next_first) {
                (Some(t), Some(Value::Temp(u))) if t == u && uses.get(t.0 as usize) == Some(&1) => {
                    Some(t)
                }
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
                    BinOp::ShrS => f.i64_shr_s(),
                    BinOp::Rotl => f.i64_rotl(),
                    BinOp::Rotr => f.i64_rotr(),
                    BinOp::Mul => f.i64_mul(),
                    BinOp::DivU => f.i64_div_u(),
                    BinOp::DivS => f.i64_div_s(),
                    BinOp::RemU => f.i64_rem_u(),
                    BinOp::RemS => f.i64_rem_s(),
                    BinOp::Eq => f.i64_eq().i64_extend_i32_u(),
                    BinOp::Ne => f.i64_ne().i64_extend_i32_u(),
                    BinOp::LtU => f.i64_lt_u().i64_extend_i32_u(),
                    BinOp::LtS => f.i64_lt_s().i64_extend_i32_u(),
                };
                self.set_temp(f, dst);
            }
            Op::Un { dst, op, a } => {
                self.value(f, a);
                match op {
                    UnOp::Clz => f.i64_clz(),
                    UnOp::Ctz => f.i64_ctz(),
                    UnOp::Sext8 => f.i64_extend8_s(),
                    UnOp::Sext16 => f.i64_extend16_s(),
                    UnOp::Sext32 => f.i64_extend32_s(),
                };
                self.set_temp(f, dst);
            }
            Op::Select { dst, cond, a, b } => {
                self.value(f, a);
                self.value(f, b);
                self.value(f, cond);
                f.i64_const(0);
                f.i64_ne();
                f.select();
                self.set_temp(f, dst);
            }
            Op::Load { dst, width, addr } => {
                self.value(f, addr);
                load(f, width, mem(0));
                self.set_temp(f, dst);
            }
            Op::Store { width, addr, value } => {
                self.value(f, addr);
                self.value(f, value);
                store(f, width, mem(0));
            }
            Op::SetReg { reg, value } => {
                self.value(f, value);
                f.local_set(L_GPR0 + reg as u32);
            }
            Op::LoadState { dst, width, offset } => {
                f.local_get(P_STATE);
                load(f, width, mem(offset as u64));
                self.set_temp(f, dst);
            }
            Op::StoreState {
                width,
                offset,
                value,
            } => {
                f.local_get(P_STATE);
                self.value(f, value);
                store(f, width, mem(offset as u64));
            }
            Op::Cond { dst, cond } => {
                self.cond(f, cond);
                f.i64_extend_i32_u();
                self.set_temp(f, dst);
            }
            Op::Flags {
                op,
                width,
                a,
                b,
                r,
                when,
                live,
            } => {
                if !live {
                    // Nothing can read these flags; the cc locals keep the
                    // previous record, so stop treating it as known.
                    self.known_cc = None;
                    return;
                }
                self.begin_when(f, when);
                if op.is_partial() {
                    // Inc/Dec keep CF: it goes to `cc_b`.
                    self.current_cf(f);
                    f.i64_extend_i32_u();
                    f.local_set(L_CC_B);
                }
                f.i32_const(op.cc_op(width) as i32);
                f.local_set(self.l_cc_op);
                self.value(f, a);
                f.local_set(L_CC_A);
                if !op.is_partial() {
                    self.value(f, b);
                    f.local_set(L_CC_B);
                }
                self.value(f, r);
                f.local_set(L_CC_R);
                self.end_when(f, when, Known::Lazy(op, width));
            }
            Op::FlagsUpdate {
                clear,
                set,
                flip,
                when,
                live,
            } => {
                if !live {
                    self.known_cc = None;
                    return;
                }
                self.begin_when(f, when);
                self.current_flags(f);
                if clear != 0 {
                    f.i64_const(!clear as i64);
                    f.i64_and();
                }
                if set != Value::Imm(0) {
                    self.value(f, set);
                    f.i64_or();
                }
                if flip != 0 {
                    f.i64_const(flip as i64);
                    f.i64_xor();
                }
                f.local_set(L_CC_A);
                f.i32_const(CC_EAGER as i32);
                f.local_set(self.l_cc_op);
                self.end_when(f, when, Known::Eager);
            }
            Op::FaultIf { cond, kind } => {
                self.value(f, cond);
                f.i64_const(0);
                f.i64_ne();
                f.if_(BlockType::Empty);
                self.labels.push(Label::If);
                self.fault(f, kind, self.cur_addr);
                f.end();
                self.labels.pop();
            }
            Op::DivWide {
                q,
                r,
                fault,
                hi,
                lo,
                d,
                signed,
            } => {
                self.value(f, hi);
                self.value(f, lo);
                self.value(f, d);
                f.i32_const(signed as i32);
                f.call(self.f_div128.expect("div128 helper emitted"));
                f.i64_extend_i32_u();
                f.local_set(self.temp(fault));
                f.local_set(self.temp(r));
                f.local_set(self.temp(q));
            }
            Op::RepString {
                op,
                width,
                rep,
                delta,
            } => self.rep_string(f, op, width, rep, delta),
            Op::VLoad { .. }
            | Op::VStore { .. }
            | Op::VBin { .. }
            | Op::VShuffle { .. }
            | Op::VShift { .. }
            | Op::VConst { .. }
            | Op::VFromGpr { .. }
            | Op::VExtract { .. } => self.vector_op(f, op),
        }
    }

    /// Opens `if when != 0` for a conditional record.
    fn begin_when(&mut self, f: &mut InstructionSink<'_>, when: Option<Value>) {
        if let Some(w) = when {
            self.value(f, w);
            f.i64_const(0);
            f.i64_ne();
            f.if_(BlockType::Empty);
            self.labels.push(Label::If);
        }
    }

    /// Closes [`Self::begin_when`]; after a conditional record the kind is
    /// unknown.
    fn end_when(&mut self, f: &mut InstructionSink<'_>, when: Option<Value>, known: Known) {
        if when.is_some() {
            f.end();
            self.labels.pop();
            self.known_cc = None;
        } else {
            self.known_cc = Some(known);
        }
    }

    fn vtemp(&self, v: VTemp) -> u32 {
        self.l_v0 + v.0
    }

    /// (chunk, entry) of the block at `addr`.
    fn locate(&self, addr: u64) -> Option<(u32, u32)> {
        self.index.get(&addr).map(|&b| self.part.block_pos[b])
    }

    /// Loads the registers and the lazy flag record from the state block.
    fn load_state(&self, f: &mut InstructionSink<'_>) {
        for i in 0..16u32 {
            f.local_get(P_STATE);
            f.i64_load(mem(abi::gpr_offset(i)));
            f.local_set(L_GPR0 + i);
        }
        f.local_get(P_STATE);
        f.i32_load(mem(abi::STATE_CC_OP));
        f.local_set(self.l_cc_op);
        for (local, offset) in [
            (L_CC_A, abi::STATE_CC_A),
            (L_CC_B, abi::STATE_CC_B),
            (L_CC_R, abi::STATE_CC_R),
        ] {
            f.local_get(P_STATE);
            f.i64_load(mem(offset));
            f.local_set(local);
        }
    }

    /// Stores the registers and the lazy flag record to the state block.
    fn store_state(&self, f: &mut InstructionSink<'_>) {
        for i in 0..16u32 {
            f.local_get(P_STATE);
            f.local_get(L_GPR0 + i);
            f.i64_store(mem(abi::gpr_offset(i)));
        }
        f.local_get(P_STATE);
        f.local_get(self.l_cc_op);
        f.i32_store(mem(abi::STATE_CC_OP));
        for (local, offset) in [
            (L_CC_A, abi::STATE_CC_A),
            (L_CC_B, abi::STATE_CC_B),
            (L_CC_R, abi::STATE_CC_R),
        ] {
            f.local_get(P_STATE);
            f.local_get(local);
            f.i64_store(mem(offset));
        }
    }

    /// Leaves through `$leave` with `status`; RIP must already be in its local.
    fn leave(&mut self, f: &mut InstructionSink<'_>, status: i32) {
        f.i32_const(status);
        f.local_set(self.l_status);
        self.br_leave(f);
    }

    fn br_leave(&mut self, f: &mut InstructionSink<'_>) {
        self.leave_used = true;
        f.br(self.depth(Label::Leave));
    }

    fn exit(&mut self, f: &mut InstructionSink<'_>, reason: u32, rip: u64) {
        f.local_get(P_STATE);
        f.i32_const(reason as i32);
        f.i32_store(mem(abi::STATE_EXIT));
        f.i64_const(rip as i64);
        f.local_set(L_RIP);
        self.leave(f, abi::STATUS_EXIT);
    }

    /// Leaves with `EXIT_FAULT` at the instruction at `rip` (fault address 0).
    fn fault(&mut self, f: &mut InstructionSink<'_>, kind: u32, rip: u64) {
        f.local_get(P_STATE);
        f.i32_const(kind as i32);
        f.i32_store(mem(abi::STATE_FAULT_KIND));
        f.local_get(P_STATE);
        f.i64_const(0);
        f.i64_store(mem(abi::STATE_FAULT_ADDR));
        self.exit(f, abi::EXIT_FAULT, rip);
    }

    /// Transfers control to the block at `target`: inside the chunk, as a
    /// tail call to another chunk, or through the trampoline if there is no
    /// block. `fall` is the index of the block whose code follows, if control
    /// can fall through to it from here.
    fn goto(&mut self, f: &mut InstructionSink<'_>, target: u64, k: usize, fall: Option<usize>) {
        match self.locate(target) {
            Some((c, j)) if c == self.chunk => {
                let j = j as usize;
                if Some(j) == fall {
                } else if j == k
                    && self
                        .labels
                        .iter()
                        .rev()
                        .take(2)
                        .any(|&l| l == Label::SelfLoop)
                {
                    f.br(self.depth(Label::SelfLoop));
                } else if j > k {
                    f.br(self.depth(Label::Block(j)));
                } else {
                    f.i32_const(j as i32);
                    f.local_set(P_ENTRY);
                    f.br(self.depth(Label::Dispatch));
                }
            }
            Some((c, j)) => {
                self.store_state(f);
                f.local_get(P_STATE);
                f.i32_const(j as i32);
                f.local_get(P_RET);
                f.return_call(F_CHUNK0 + c);
            }
            None => {
                // Outside the module: through the lookup table.
                f.i64_const(target as i64);
                f.local_set(L_RIP);
                self.probe(f);
                self.tail_call_indirect(f);
            }
        }
    }

    /// Probes the lookup table for the address in the RIP local: on a hit,
    /// `entry` and `func` hold the slot; on a miss, unwinds to it.
    fn probe(&mut self, f: &mut InstructionSink<'_>) {
        f.local_get(L_RIP);
        f.call(F_LOOKUP);
        f.local_set(P_ENTRY);
        f.local_tee(self.l_func);
        f.i32_const(0);
        f.i32_lt_s();
        self.unwind_if(f);
    }

    /// Stores the state and tail-calls the probed `(func, entry)`.
    fn tail_call_indirect(&mut self, f: &mut InstructionSink<'_>) {
        self.store_state(f);
        f.local_get(P_STATE);
        f.local_get(P_ENTRY);
        f.local_get(P_RET);
        f.local_get(self.l_func);
        f.return_call_indirect(TABLE, T_CHUNK);
    }

    /// Calls the probed `(func, entry)` (after the depth guard), expecting
    /// `ret` back.
    fn call_probed(&mut self, f: &mut InstructionSink<'_>, ret: u64) {
        self.depth_guard(f);
        self.store_state(f);
        Self::add_depth(f, 1);
        f.local_get(P_STATE);
        f.local_get(P_ENTRY);
        f.i64_const(ret as i64);
        f.local_get(self.l_func);
        f.call_indirect(TABLE, T_CHUNK);
    }

    /// Unwinds instead of calling when `depth` has reached the limit. RIP
    /// must already hold the call target.
    fn depth_guard(&mut self, f: &mut InstructionSink<'_>) {
        if let Some(limit) = self.depth_limit {
            f.global_get(G_DEPTH);
            f.i32_const(limit as i32);
            f.i32_ge_u();
            self.unwind_if(f);
        }
    }

    /// Unwinds (RIP already set) if the i32 on the stack is nonzero.
    fn unwind_if(&mut self, f: &mut InstructionSink<'_>) {
        f.if_(BlockType::Empty);
        self.labels.push(Label::If);
        self.leave(f, abi::STATUS_UNWIND);
        f.end();
        self.labels.pop();
    }

    fn add_depth(f: &mut InstructionSink<'_>, delta: i32) {
        f.global_get(G_DEPTH);
        f.i32_const(delta);
        f.i32_add();
        f.global_set(G_DEPTH);
    }

    /// After a call: pass a status other than RETURNED up, else reload.
    fn after_call(&mut self, f: &mut InstructionSink<'_>) {
        Self::add_depth(f, -1);
        f.local_tee(self.l_status);
        f.if_(BlockType::Empty);
        f.local_get(self.l_status);
        f.return_();
        f.end();
        self.load_state(f);
        self.known_cc = None;
    }

    fn call(&mut self, f: &mut InstructionSink<'_>, target: CallTarget, ret: u64) {
        match target {
            CallTarget::Direct(t) => {
                let Some((c, j)) = self.locate(t) else {
                    // Outside the module: through the lookup table.
                    f.i64_const(t as i64);
                    f.local_set(L_RIP);
                    self.probe(f);
                    self.call_probed(f, ret);
                    self.after_call(f);
                    return;
                };
                if self.depth_limit.is_some() {
                    f.i64_const(t as i64);
                    f.local_set(L_RIP);
                }
                self.depth_guard(f);
                self.store_state(f);
                Self::add_depth(f, 1);
                f.local_get(P_STATE);
                f.i32_const(j as i32);
                f.i64_const(ret as i64);
                f.call(F_CHUNK0 + c);
            }
            CallTarget::Indirect(v) => {
                self.value(f, v);
                f.local_set(L_RIP);
                self.probe(f);
                self.call_probed(f, ret);
            }
        }
        self.after_call(f);
    }

    fn block(&mut self, f: &mut InstructionSink<'_>, k: usize, block: &Block) {
        self.known_cc = None;
        let addr = block.addr;
        let self_loop = match block.term {
            Term::Jump(t) => t == addr,
            Term::Branch {
                taken, fallthrough, ..
            }
            | Term::BranchIf {
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
            self.cur_addr = insn.addr;
            self.insn(f, &insn.ops, last.then_some(&block.term));
        }
        let next = (k + 1 < self.blocks.len()).then_some(k + 1);
        match block.term {
            Term::Jump(t) => self.goto(f, t, k, next),
            Term::Branch {
                cond,
                taken,
                fallthrough,
            } => {
                // Prefer falling through to the next block on the else path.
                let next_addr = next.map(|j| self.prog.blocks[self.blocks[j]].addr);
                let (cond, taken, fallthrough) = if next_addr.is_some() && next_addr == Some(taken)
                {
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
            Term::BranchIf {
                value,
                taken,
                fallthrough,
            } => {
                let next_addr = next.map(|j| self.prog.blocks[self.blocks[j]].addr);
                self.value(f, value);
                let (taken, fallthrough) = if next_addr.is_some() && next_addr == Some(taken) {
                    f.i64_eqz();
                    (fallthrough, taken)
                } else {
                    f.i64_const(0);
                    f.i64_ne();
                    (taken, fallthrough)
                };
                f.if_(BlockType::Empty);
                self.labels.push(Label::If);
                self.goto(f, taken, k, None);
                f.end();
                self.labels.pop();
                self.goto(f, fallthrough, k, next);
            }
            Term::Call { target, ret } => {
                self.call(f, target, ret);
                self.goto(f, ret, k, next);
            }
            Term::Ret(v) => {
                // RETURNED (0) if the popped address is the expected one,
                // else UNWIND (1) to it.
                self.value(f, v);
                f.local_tee(L_RIP);
                f.local_get(P_RET);
                f.i64_ne();
                f.local_set(self.l_status);
                self.br_leave(f);
            }
            Term::Indirect(v) => {
                self.value(f, v);
                f.local_tee(L_RIP);
                f.call(F_LOOKUP);
                f.local_set(P_ENTRY);
                // A hit in this chunk is a local dispatch.
                f.local_tee(self.l_func);
                f.global_get(G_TABLE_BASE);
                f.i32_const(self.chunk as i32);
                f.i32_add();
                f.i32_eq();
                f.br_if(self.depth(Label::Dispatch));
                f.local_get(self.l_func);
                f.i32_const(0);
                f.i32_lt_s();
                self.unwind_if(f);
                self.tail_call_indirect(f);
            }
            Term::Exit { reason, rip } => self.exit(f, reason, rip),
            Term::Fault { kind, rip } => self.fault(f, kind, rip),
        }
        if self_loop {
            f.end();
            self.labels.pop();
        }
    }

    fn body(&mut self, f: &mut InstructionSink<'_>) {
        let n = self.blocks.len();
        self.load_state(f);

        f.block(BlockType::Empty);
        self.labels.push(Label::Leave);
        f.loop_(BlockType::Empty);
        self.labels.push(Label::Dispatch);
        for k in (0..n).rev() {
            f.block(BlockType::Empty);
            self.labels.push(Label::Block(k));
        }
        f.block(BlockType::Empty);
        self.labels.push(Label::Miss);
        f.local_get(P_ENTRY);
        let targets: Vec<u32> = (0..n).map(|k| self.depth(Label::Block(k))).collect();
        f.br_table(targets, self.depth(Label::Miss));
        f.end();
        self.labels.pop();
        // Callers only pass entries of this chunk.
        f.unreachable();
        let (prog, blocks) = (self.prog, self.blocks);
        for (k, &b) in blocks.iter().enumerate() {
            f.end();
            assert_eq!(self.labels.pop(), Some(Label::Block(k)));
            self.block(f, k, &prog.blocks[b]);
        }
        f.end(); // loop
        self.labels.pop();
        f.unreachable();
        f.end(); // leave block
        self.labels.pop();
        if !self.leave_used {
            f.unreachable();
            f.end();
            return;
        }

        // Leave: RETURNED, UNWIND or EXIT.
        self.store_state(f);
        f.local_get(self.l_status);
        f.if_(BlockType::Empty);
        f.local_get(P_STATE);
        f.local_get(L_RIP);
        f.i64_store(mem(abi::STATE_RIP));
        f.end();
        f.local_get(self.l_status);
        f.end();
    }
}

/// The temporary an op defines (as its last action, so it may stay on the
/// stack for the next op).
fn defined(op: &Op) -> Option<Temp> {
    match *op {
        Op::Mov { dst, .. }
        | Op::Bin { dst, .. }
        | Op::Un { dst, .. }
        | Op::Select { dst, .. }
        | Op::Load { dst, .. }
        | Op::LoadState { dst, .. }
        | Op::Cond { dst, .. }
        | Op::VExtract { dst, .. } => Some(dst),
        _ => None,
    }
}

/// The operand an op consumes first from the stack, with nothing pushed
/// above it before (only stack-neutral code may come first).
fn first_operand(op: &Op) -> Option<Value> {
    match *op {
        Op::Mov { src, .. } => Some(src),
        Op::Bin { a, .. } | Op::Un { a, .. } | Op::Select { a, .. } => Some(a),
        Op::Load { addr, .. } | Op::Store { addr, .. } => Some(addr),
        Op::SetReg { value, .. } => Some(value),
        Op::FaultIf { cond, .. } => Some(cond),
        Op::DivWide { hi, .. } => Some(hi),
        Op::Flags {
            when: Some(w),
            live: true,
            ..
        }
        | Op::FlagsUpdate {
            when: Some(w),
            live: true,
            ..
        } => Some(w),
        Op::Flags {
            a,
            when: None,
            live: true,
            ..
        } => Some(a),
        _ => None,
    }
}

/// Every operand an op reads.
fn operands(op: &Op) -> [Option<Value>; 4] {
    match *op {
        Op::Mov { src, .. } => [Some(src), None, None, None],
        Op::Bin { a, b, .. } => [Some(a), Some(b), None, None],
        Op::Un { a, .. } => [Some(a), None, None, None],
        Op::Select { cond, a, b, .. } => [Some(cond), Some(a), Some(b), None],
        Op::Load { addr, .. } => [Some(addr), None, None, None],
        Op::Store { addr, value, .. } => [Some(addr), Some(value), None, None],
        Op::SetReg { value, .. } | Op::StoreState { value, .. } => [Some(value), None, None, None],
        Op::Flags {
            a,
            b,
            r,
            when,
            live: true,
            ..
        } => [Some(a), Some(b), Some(r), when],
        Op::FlagsUpdate {
            set,
            when,
            live: true,
            ..
        } => [Some(set), when, None, None],
        Op::Flags { live: false, .. } | Op::FlagsUpdate { live: false, .. } => [None; 4],
        Op::FaultIf { cond, .. } => [Some(cond), None, None, None],
        Op::DivWide { hi, lo, d, .. } => [Some(hi), Some(lo), Some(d), None],
        Op::RepString { delta, .. } => [Some(delta), None, None, None],
        Op::VLoad {
            loc: crate::ir::VLoc::Mem(a),
            ..
        }
        | Op::VStore {
            loc: crate::ir::VLoc::Mem(a),
            ..
        } => [Some(a), None, None, None],
        Op::VShift { count, .. } => [Some(count), None, None, None],
        Op::VFromGpr { value, .. } => [Some(value), None, None, None],
        Op::LoadState { .. }
        | Op::Cond { .. }
        | Op::VLoad { .. }
        | Op::VStore { .. }
        | Op::VBin { .. }
        | Op::VShuffle { .. }
        | Op::VConst { .. }
        | Op::VExtract { .. } => [None; 4],
    }
}

/// A zero-extending i64 load of `width`.
fn load(f: &mut InstructionSink<'_>, width: Width, m: MemArg) {
    match width {
        Width::W8 => f.i64_load8_u(m),
        Width::W16 => f.i64_load16_u(m),
        Width::W32 => f.i64_load32_u(m),
        Width::W64 => f.i64_load(m),
    };
}

/// An i64 store of the low `width` bits.
fn store(f: &mut InstructionSink<'_>, width: Width, m: MemArg) {
    match width {
        Width::W8 => f.i64_store8(m),
        Width::W16 => f.i64_store16(m),
        Width::W32 => f.i64_store32(m),
        Width::W64 => f.i64_store(m),
    };
}

/// `lookup(rip: i64) -> (func: i32, entry: i32)`: probes the process-wide
/// lookup table (`abi.md`, "Lookup table"), reading its header every time;
/// `(-1, 0)` if there is no block at `rip`.
fn emit_lookup() -> Function {
    const RIP: u32 = 0;
    const SLOTS: u32 = 1;
    const MASK: u32 = 2;
    const I: u32 = 3;
    const AT: u32 = 4;
    const KEY: u32 = 5;
    let mut func = Function::new([(5, ValType::I64)]);
    let mut f = func.instructions();
    f.global_get(G_LOOKUP);
    f.i64_load(mem(abi::LOOKUP_SLOTS));
    f.local_set(SLOTS);
    f.global_get(G_LOOKUP);
    f.i64_load(mem(abi::LOOKUP_MASK));
    f.local_tee(MASK);
    f.local_get(RIP);
    f.i64_const(abi::LOOKUP_HASH_MUL as i64);
    f.i64_mul();
    f.i64_const(abi::LOOKUP_HASH_SHIFT as i64);
    f.i64_shr_u();
    f.i64_and();
    f.local_set(I);
    f.loop_(BlockType::Empty);
    f.local_get(SLOTS);
    f.local_get(I);
    f.i64_const(abi::LOOKUP_SLOT_SIZE.trailing_zeros() as i64);
    f.i64_shl();
    f.i64_add();
    f.local_tee(AT);
    f.i64_load(mem(0));
    // Empty first: address 0 is never a key, so a probe for 0 misses.
    f.local_tee(KEY);
    f.i64_eqz();
    f.if_(BlockType::Empty);
    f.i32_const(-1);
    f.i32_const(0);
    f.return_();
    f.end();
    f.local_get(KEY);
    f.local_get(RIP);
    f.i64_eq();
    f.if_(BlockType::Empty);
    f.local_get(AT);
    f.i32_load(mem(8));
    f.local_get(AT);
    f.i32_load(mem(12));
    f.return_();
    f.end();
    f.local_get(I);
    f.i64_const(1);
    f.i64_add();
    f.local_get(MASK);
    f.i64_and();
    f.local_set(I);
    f.br(0);
    f.end();
    f.unreachable();
    f.end();
    func
}

/// `run(state: i64) -> i32`: the trampoline (`docs/design/abi.md`).
fn emit_run() -> Function {
    const STATE: u32 = 0;
    const FUNC: u32 = 1;
    const ENTRY: u32 = 2;
    const STATUS: u32 = 3;
    let mut func = Function::new([(3, ValType::I32)]);
    let mut f = func.instructions();
    // The lazy flag record starts materialized from RFLAGS.
    f.local_get(STATE);
    f.i32_const(CC_EAGER as i32);
    f.i32_store(mem(abi::STATE_CC_OP));
    f.local_get(STATE);
    f.local_get(STATE);
    f.i64_load(mem(abi::STATE_RFLAGS));
    f.i64_store(mem(abi::STATE_CC_A));
    f.i32_const(0);
    f.global_set(G_DEPTH);

    f.block(BlockType::Empty); // $done
    f.loop_(BlockType::Empty); // $next
    f.local_get(STATE);
    f.i64_load(mem(abi::STATE_RIP));
    f.call(F_LOOKUP);
    f.local_set(ENTRY);
    f.local_tee(FUNC);
    f.i32_const(0);
    f.i32_lt_s();
    f.if_(BlockType::Empty);
    f.i32_const(abi::EXIT_LEFT_CODE as i32);
    f.local_set(STATUS);
    f.br(2);
    f.end();
    f.local_get(STATE);
    f.local_get(ENTRY);
    f.i64_const(abi::NO_RET as i64);
    f.local_get(FUNC);
    f.call_indirect(TABLE, T_CHUNK);
    f.local_tee(STATUS);
    f.i32_const(abi::STATUS_EXIT);
    f.i32_eq();
    f.if_(BlockType::Empty);
    f.local_get(STATE);
    f.i32_load(mem(abi::STATE_EXIT));
    f.local_set(STATUS);
    f.br(2);
    f.end();
    // RETURNED: the guest returned to NO_RET itself.
    f.local_get(STATUS);
    f.i32_eqz();
    f.if_(BlockType::Empty);
    f.local_get(STATE);
    f.i64_const(abi::NO_RET as i64);
    f.i64_store(mem(abi::STATE_RIP));
    f.end();
    f.i32_const(0);
    f.global_set(G_DEPTH);
    f.br(0);
    f.end(); // loop
    f.end(); // $done

    // Materialize RFLAGS; STATUS holds the exit reason.
    f.local_get(STATE);
    f.local_get(STATE);
    f.i64_load(mem(abi::STATE_RFLAGS));
    f.i64_const(!abi::flags::STATUS as i64);
    f.i64_and();
    f.local_get(STATE);
    f.i32_load(mem(abi::STATE_CC_OP));
    for offset in [abi::STATE_CC_A, abi::STATE_CC_B, abi::STATE_CC_R] {
        f.local_get(STATE);
        f.i64_load(mem(offset));
    }
    f.call(F_FLAGS);
    f.i64_or();
    f.i64_store(mem(abi::STATE_RFLAGS));
    f.local_get(STATE);
    f.local_get(STATUS);
    f.i32_store(mem(abi::STATE_EXIT));
    f.local_get(STATUS);
    f.end();
    func
}

/// A module that exports the `flags` function and the `div128` helper
/// alone (no imports), for testing them against reference models.
pub fn helpers_test_module() -> Vec<u8> {
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types.ty().function(
        [ValType::I32, ValType::I64, ValType::I64, ValType::I64],
        [ValType::I64],
    );
    types.ty().function(
        [ValType::I64, ValType::I64, ValType::I64, ValType::I32],
        [ValType::I64, ValType::I64, ValType::I32],
    );
    module.section(&types);
    let mut funcs = FunctionSection::new();
    funcs.function(0);
    funcs.function(1);
    module.section(&funcs);
    let mut exports = ExportSection::new();
    exports.export("flags", ExportKind::Func, 0);
    exports.export("div128", ExportKind::Func, 1);
    module.section(&exports);
    let mut code = CodeSection::new();
    code.function(&emit_flags());
    code.function(&helpers::emit_div128());
    module.section(&code);
    module.finish()
}

/// Encodes `prog`, partitioned as `part`, as a module following the ABI.
pub fn emit(prog: &Program, part: &Partition, opts: &Options) -> Vec<u8> {
    let mut module = Module::new();
    let n_chunks = part.chunks.len() as u32;

    let mut types = TypeSection::new();
    types.ty().function([ValType::I64], [ValType::I32]); // T_RUN
    types.ty().function(
        [ValType::I32, ValType::I64, ValType::I64, ValType::I64],
        [ValType::I64],
    ); // T_FLAGS
    types
        .ty()
        .function([ValType::I64, ValType::I32, ValType::I64], [ValType::I32]); // T_CHUNK
    types
        .ty()
        .function([ValType::I64], [ValType::I32, ValType::I32]); // T_LOOKUP
    let uses_div128 = prog
        .blocks
        .iter()
        .flat_map(|b| b.insns.iter().flat_map(|i| i.ops.iter()))
        .any(|op| matches!(op, Op::DivWide { .. }));
    if uses_div128 {
        types.ty().function(
            [ValType::I64, ValType::I64, ValType::I64, ValType::I32],
            [ValType::I64, ValType::I64, ValType::I32],
        ); // T_DIV128
    }
    let f_div128 = uses_div128.then_some(F_CHUNK0 + n_chunks);
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
    imports.import(
        abi::MEMORY_MODULE,
        abi::TABLE_NAME,
        EntityType::Table(TableType {
            element_type: RefType::FUNCREF,
            table64: false,
            minimum: 0,
            maximum: None,
            shared: false,
        }),
    );
    for (name, val_type, mutable) in [
        (abi::TABLE_BASE_NAME, ValType::I32, false),
        (abi::LOOKUP_NAME, ValType::I64, false),
        (abi::DEPTH_NAME, ValType::I32, true),
    ] {
        imports.import(
            abi::MEMORY_MODULE,
            name,
            EntityType::Global(GlobalType {
                val_type,
                mutable,
                shared: false,
            }),
        );
    }
    module.section(&imports);

    let mut funcs = FunctionSection::new();
    funcs.function(T_RUN);
    funcs.function(T_LOOKUP);
    funcs.function(T_FLAGS);
    for _ in 0..n_chunks {
        funcs.function(T_CHUNK);
    }
    if uses_div128 {
        funcs.function(T_DIV128);
    }
    module.section(&funcs);

    let mut exports = ExportSection::new();
    exports.export(abi::RUN_EXPORT, ExportKind::Func, F_RUN);
    module.section(&exports);

    // Chunk i goes to table index table_base + i.
    let mut elements = ElementSection::new();
    let chunk_funcs: Vec<u32> = (0..n_chunks).map(|c| F_CHUNK0 + c).collect();
    elements.active(
        Some(TABLE),
        &ConstExpr::global_get(G_TABLE_BASE),
        Elements::Functions(Cow::Borrowed(&chunk_funcs)),
    );
    module.section(&elements);

    let index = prog.index_map();
    let mut code = CodeSection::new();
    code.function(&emit_run());
    code.function(&emit_lookup());
    code.function(&emit_flags());
    let mut temps = Vec::with_capacity(part.chunks.len());
    for (c, chunk) in part.chunks.iter().enumerate() {
        let t = chunk
            .blocks
            .iter()
            .flat_map(|&b| prog.blocks[b].insns.iter().map(|i| i.temps))
            .max()
            .unwrap_or(0);
        let vt = chunk
            .blocks
            .iter()
            .flat_map(|&b| prog.blocks[b].insns.iter().map(|i| i.vtemps))
            .max()
            .unwrap_or(0);
        temps.push((t, vt));
        let l_cc_op = L_TEMP0 + t;
        let mut locals = vec![(L_TEMP0 - L_GPR0 + t, ValType::I64), (3, ValType::I32)];
        if vt > 0 {
            locals.push((vt, ValType::V128));
        }
        let mut func = Function::new(locals);
        let mut emitter = ChunkEmitter {
            prog,
            part,
            index: &index,
            chunk: c as u32,
            blocks: &chunk.blocks,
            depth_limit: opts.call_depth_limit,
            labels: Vec::new(),
            known_cc: None,
            keep: None,
            on_stack: None,
            l_cc_op,
            l_status: l_cc_op + 1,
            l_func: l_cc_op + 2,
            l_v0: l_cc_op + 3,
            f_div128,
            cur_addr: 0,
            leave_used: false,
        };
        emitter.body(&mut func.instructions());
        code.function(&func);
    }
    if uses_div128 {
        code.function(&helpers::emit_div128());
    }
    module.section(&code);

    // Blocks are sorted by address, since program blocks are.
    let blocks: Vec<BlockEntry> = prog
        .blocks
        .iter()
        .zip(&part.block_pos)
        .map(|(b, &(chunk, entry))| BlockEntry {
            addr: b.addr,
            chunk,
            entry,
        })
        .collect();
    module.section(&CustomSection {
        name: Cow::Borrowed(abi::BLOCKS_SECTION),
        data: Cow::Owned(encode_blocks(n_chunks, &blocks)),
    });

    module.section(&names(prog, part, &temps, f_div128));
    module.finish()
}

fn names(
    prog: &Program,
    part: &Partition,
    temps: &[(u32, u32)],
    f_div128: Option<u32>,
) -> NameSection {
    let mut names = NameSection::new();
    let mut funcs = NameMap::new();
    funcs.append(F_RUN, "run");
    funcs.append(F_LOOKUP, "lookup");
    funcs.append(F_FLAGS, "flags");
    for c in 0..part.chunks.len() {
        funcs.append(F_CHUNK0 + c as u32, &part.chunk_name(prog, c));
    }
    if let Some(i) = f_div128 {
        funcs.append(i, "div128");
    }
    names.functions(&funcs);

    let mut locals = IndirectNameMap::new();
    let mut run = NameMap::new();
    for (i, n) in ["state", "func", "entry", "status"].iter().enumerate() {
        run.append(i as u32, n);
    }
    locals.append(F_RUN, &run);
    let mut lookup = NameMap::new();
    for (i, n) in ["rip", "slots", "mask", "i", "at", "key"]
        .iter()
        .enumerate()
    {
        lookup.append(i as u32, n);
    }
    locals.append(F_LOOKUP, &lookup);
    let mut flags = NameMap::new();
    for (i, n) in ["op", "a", "b", "r", "sign", "cf_of"].iter().enumerate() {
        flags.append(i as u32, n);
    }
    locals.append(F_FLAGS, &flags);
    for (c, &(t, vt)) in temps.iter().enumerate() {
        let mut chunk = NameMap::new();
        chunk.append(P_STATE, "state");
        chunk.append(P_ENTRY, "entry");
        chunk.append(P_RET, "ret");
        for i in 0..16u8 {
            chunk.append(L_GPR0 + i as u32, reg_name(i));
        }
        chunk.append(L_RIP, "rip");
        chunk.append(L_CC_A, "cc_a");
        chunk.append(L_CC_B, "cc_b");
        chunk.append(L_CC_R, "cc_r");
        chunk.append(L_SCRATCH, "scratch");
        for i in 0..t {
            chunk.append(L_TEMP0 + i, &format!("t{i}"));
        }
        chunk.append(L_TEMP0 + t, "cc_op");
        chunk.append(L_TEMP0 + t + 1, "status");
        chunk.append(L_TEMP0 + t + 2, "func");
        for i in 0..vt {
            chunk.append(L_TEMP0 + t + 3 + i, &format!("v{i}"));
        }
        locals.append(F_CHUNK0 + c as u32, &chunk);
    }
    if let Some(i) = f_div128 {
        let mut div = NameMap::new();
        for (j, n) in helpers::DIV128_LOCALS.iter().enumerate() {
            div.append(j as u32, n);
        }
        locals.append(i, &div);
    }
    names.locals(&locals);

    let mut tables = NameMap::new();
    tables.append(TABLE, "table");
    names.tables(&tables);
    let mut globals = NameMap::new();
    globals.append(G_TABLE_BASE, "table_base");
    globals.append(G_LOOKUP, "lookup");
    globals.append(G_DEPTH, "depth");
    names.globals(&globals);
    names
}
