//! Register and flag sync (`docs/design/translator.md`, "Registers and
//! flags"): which of the 17 values (the 16 GPRs and the lazy flag record as
//! one unit) a chunk loads at entry, stores when control leaves it, and
//! reloads after a call.
//!
//! - **Dirty:** a forward may-analysis over each chunk. A write makes a value
//!   dirty; a call stores the dirty set and reloads, so nothing is dirty
//!   after it. Nothing is dirty at entry.
//! - **Live:** backward liveness over each chunk. Every store (calls,
//!   returns, tail transfers, and the cold leaves inside blocks) uses the
//!   dirty set at its point, and a local indirect dispatch is an edge to
//!   every block of the chunk.
//! - **MOD:** per guest function, the values a call to it may change in the
//!   state block: a fixpoint over the module's direct calls and tail
//!   transfers. Indirect and probed targets change everything.
//!
//! The emitter replays the dirty transfer ([`op_effect`]) op by op, so its
//! stores are exactly the sets the liveness counted as uses.

use std::collections::HashMap;

use crate::abi;
use crate::func::Partition;
use crate::ir::{Block, CallTarget, Op, Program, Rep, StrOp, Term, VLoc, Value};

/// A set of values: bit `r` for GPR `r`, [`FLAGS`] for the flag record.
pub type Set = u32;
/// The lazy flag record (`cc_op`, `cc_a`, `cc_b`, `cc_r`).
pub const FLAGS: Set = 1 << 16;
/// Every value.
pub const ALL: Set = (1 << 17) - 1;

const RAX: Set = 1 << crate::ir::RAX;
const RCX: Set = 1 << crate::ir::RCX;
const RSI: Set = 1 << 6;
const RDI: Set = 1 << 7;

/// The number of locals a set moves: one per GPR, four for the flag record.
pub fn weight(s: Set) -> i64 {
    (s & 0xffff).count_ones() as i64 + if s & FLAGS != 0 { 4 } else { 0 }
}

/// What one op does to the values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effect {
    /// Read before anything is written.
    pub uses: Set,
    /// Always overwritten (liveness kills them).
    pub kills: Set,
    /// May be written (they become dirty).
    pub writes: Set,
    /// May leave the chunk before its writes, through the shared epilogue,
    /// storing the dirty set from before the op.
    pub may_leave: bool,
}

fn value(v: Value) -> Set {
    match v {
        Value::Reg(r) => 1 << r,
        Value::Temp(_) | Value::Imm(_) => 0,
    }
}

fn opt(v: Option<Value>) -> Set {
    v.map_or(0, value)
}

/// The effect of `op` on the values, as the emitter lowers it.
pub fn op_effect(op: &Op) -> Effect {
    let reads = |uses| Effect {
        uses,
        ..Effect::default()
    };
    match *op {
        Op::Mov { src, .. } => reads(value(src)),
        Op::Bin { a, b, .. } => reads(value(a) | value(b)),
        Op::Un { a, .. } => reads(value(a)),
        Op::Select { cond, a, b, .. } => reads(value(cond) | value(a) | value(b)),
        Op::Load { addr, .. } => reads(value(addr)),
        Op::Store { addr, value: v, .. } => reads(value(addr) | value(v)),
        Op::SetReg { reg, value: v } => Effect {
            uses: value(v),
            kills: 1 << reg,
            writes: 1 << reg,
            may_leave: false,
        },
        Op::LoadState { .. } => Effect::default(),
        Op::StoreState { value: v, .. } => reads(value(v)),
        Op::Cond { .. } => reads(FLAGS),
        // Dead records emit nothing.
        Op::Flags { live: false, .. } | Op::FlagsUpdate { live: false, .. } => Effect::default(),
        Op::Flags {
            op, a, b, r, when, ..
        } => {
            // Inc/Dec read CF of the previous record; a conditional record
            // may keep it.
            let partial = op.is_partial();
            Effect {
                uses: value(a) | value(b) | value(r) | opt(when) | if partial { FLAGS } else { 0 },
                kills: if partial || when.is_some() { 0 } else { FLAGS },
                writes: FLAGS,
                may_leave: false,
            }
        }
        Op::FlagsUpdate {
            clear, set, when, ..
        } => {
            let full = clear & abi::flags::STATUS == abi::flags::STATUS;
            Effect {
                uses: value(set) | opt(when) | if full { 0 } else { FLAGS },
                kills: if full && when.is_none() { FLAGS } else { 0 },
                writes: FLAGS,
                may_leave: false,
            }
        }
        Op::FaultIf { cond, .. } => Effect {
            uses: value(cond),
            may_leave: true,
            ..Effect::default()
        },
        Op::DivWide { hi, lo, d, .. } => reads(value(hi) | value(lo) | value(d)),
        Op::RepString { op, rep, delta, .. } => {
            // `emit/string.rs`: the implicit registers. They are read and
            // written in place (and kept when RCX = 0), so they are uses and
            // never kills.
            let regs = match op {
                StrOp::Movs | StrOp::Cmps => RSI | RDI,
                StrOp::Stos | StrOp::Scas => RAX | RDI,
                StrOp::Lods => RAX | RSI,
            };
            let count = if rep == Rep::Once { 0 } else { RCX };
            let writes = match op {
                StrOp::Movs | StrOp::Cmps => RSI | RDI,
                StrOp::Stos | StrOp::Scas => RDI,
                StrOp::Lods => RAX | RSI,
            } | count;
            let flags = op.compares();
            Effect {
                uses: regs | count | value(delta),
                kills: if flags && rep == Rep::Once { FLAGS } else { 0 },
                writes: writes | if flags { FLAGS } else { 0 },
                may_leave: false,
            }
        }
        Op::VLoad { loc, .. } | Op::VStore { loc, .. } => match loc {
            VLoc::Mem(a) => reads(value(a)),
            VLoc::State(_) => Effect::default(),
        },
        Op::VShift { count, .. } => reads(value(count)),
        Op::VFromGpr { value: v, .. } => reads(value(v)),
        Op::VBin { .. } | Op::VShuffle { .. } | Op::VConst { .. } | Op::VExtract { .. } => {
            Effect::default()
        }
        // The floating-point ops may exit on an unmasked exception, before
        // writing anything.
        Op::SseFromInt { src, .. } => Effect {
            uses: value(src),
            may_leave: true,
            ..Effect::default()
        },
        Op::SseFp { .. } | Op::SseToInt { .. } | Op::SseComi { .. } => Effect {
            may_leave: true,
            ..Effect::default()
        },
        Op::X87 { addr, .. } => Effect {
            uses: opt(addr),
            may_leave: true,
            ..Effect::default()
        },
    }
}

/// The values a terminator reads itself (not its stores).
fn term_reads(term: &Term) -> Set {
    match *term {
        Term::Branch { .. } => FLAGS,
        Term::BranchIf { value: v, .. } | Term::Indirect(v) | Term::Ret(v) => value(v),
        Term::Call {
            target: CallTarget::Indirect(v),
            ..
        } => value(v),
        _ => 0,
    }
}

/// The analysis results for one chunk, by entry index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChunkSync {
    /// Values that may be dirty when each block starts.
    pub dirty_in: Vec<Set>,
    /// Values live into each block.
    pub live_in: Vec<Set>,
    /// For a block that ends in a call: the values to reload after it
    /// returns (live after the call and in the callee's MOD).
    pub reload: Vec<Set>,
    /// The fast entry: the guest function's own entry block, if this chunk
    /// holds it, with what it loads.
    pub fast: Option<(usize, Set)>,
    /// What every other entry loads: the union of the live-in sets.
    pub generic: Set,
}

/// The analysis results for a module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sync {
    /// Per chunk.
    pub chunks: Vec<ChunkSync>,
    /// Per guest function: MOD, the values a call to it may change.
    pub modified: Vec<Set>,
}

/// Where a transfer to a guest address goes, seen from one chunk.
enum Dest {
    /// A block of the same chunk, by entry index.
    Local(usize),
    /// A block of another chunk, of this guest function.
    Other(usize),
    /// No block in the module: through the lookup table.
    Probe,
}

struct Ctx<'p> {
    part: &'p Partition,
    index: &'p HashMap<u64, usize>,
}

impl Ctx<'_> {
    fn dest(&self, chunk: usize, addr: u64) -> Dest {
        match self.index.get(&addr) {
            Some(&b) => {
                let (c, j) = self.part.block_pos[b];
                if c as usize == chunk {
                    Dest::Local(j as usize)
                } else {
                    Dest::Other(self.part.chunks[c as usize].function)
                }
            }
            None => Dest::Probe,
        }
    }
}

/// Runs the analyses. With `sync_all`, every set is [`ALL`]: the reference
/// mode loads, stores and reloads everything.
pub fn analyze(
    prog: &Program,
    part: &Partition,
    index: &HashMap<u64, usize>,
    sync_all: bool,
) -> Sync {
    let ctx = Ctx { part, index };
    if sync_all {
        let chunks = part
            .chunks
            .iter()
            .enumerate()
            .map(|(c, chunk)| {
                let n = chunk.blocks.len();
                ChunkSync {
                    dirty_in: vec![ALL; n],
                    live_in: vec![ALL; n],
                    reload: vec![ALL; n],
                    fast: own_entry(prog, part, c).map(|k| (k, ALL)),
                    generic: ALL,
                }
            })
            .collect();
        return Sync {
            chunks,
            modified: vec![ALL; part.functions.len()],
        };
    }
    let modified = modified(prog, &ctx);
    let chunks = (0..part.chunks.len())
        .map(|c| chunk_sync(prog, &ctx, &modified, c))
        .collect();
    Sync { chunks, modified }
}

/// The entry index of the guest function's entry block in chunk `c`.
fn own_entry(prog: &Program, part: &Partition, c: usize) -> Option<usize> {
    let chunk = &part.chunks[c];
    let entry = part.functions[chunk.function].entry;
    chunk
        .blocks
        .iter()
        .position(|&b| prog.blocks[b].addr == entry)
}

/// The addresses a terminator continues at in guest code (not call targets).
fn successors(term: &Term) -> Vec<u64> {
    match *term {
        Term::Jump(t) => vec![t],
        Term::Branch {
            taken, fallthrough, ..
        }
        | Term::BranchIf {
            taken, fallthrough, ..
        } => vec![taken, fallthrough],
        Term::Call { ret, .. } => vec![ret],
        Term::Indirect(_) | Term::Ret(_) | Term::Exit { .. } | Term::Fault { .. } => vec![],
    }
}

/// MOD per guest function.
fn modified(prog: &Program, ctx: &Ctx<'_>) -> Vec<Set> {
    let part = ctx.part;
    let n = part.functions.len();
    let mut own = vec![0; n];
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (c, chunk) in part.chunks.iter().enumerate() {
        let f = chunk.function;
        for &b in &chunk.blocks {
            let block = &prog.blocks[b];
            for op in block.insns.iter().flat_map(|i| i.ops.iter()) {
                own[f] |= op_effect(op).writes;
            }
            let mut to = |dest: Dest| match dest {
                Dest::Local(_) => {}
                Dest::Other(g) => succs[f].push(g),
                Dest::Probe => own[f] = ALL,
            };
            match block.term {
                Term::Call {
                    target: CallTarget::Direct(t),
                    ..
                } => match ctx.index.get(&t) {
                    Some(&tb) => to(Dest::Other(
                        part.chunks[part.block_pos[tb].0 as usize].function,
                    )),
                    None => to(Dest::Probe),
                },
                Term::Call {
                    target: CallTarget::Indirect(_),
                    ..
                }
                | Term::Indirect(_) => to(Dest::Probe),
                _ => {}
            }
            for t in successors(&block.term) {
                to(ctx.dest(c, t));
            }
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        for f in 0..n {
            let m = succs[f].iter().fold(own[f], |m, &g| m | own[g]);
            if m != own[f] {
                own[f] = m;
                changed = true;
            }
        }
    }
    own
}

/// MOD of a call target.
fn target_mod(ctx: &Ctx<'_>, modified: &[Set], target: CallTarget) -> Set {
    match target {
        CallTarget::Direct(t) => ctx.index.get(&t).map_or(ALL, |&b| {
            modified[ctx.part.chunks[ctx.part.block_pos[b].0 as usize].function]
        }),
        CallTarget::Indirect(_) => ALL,
    }
}

/// The dirty set at the end of a block that starts with `dirty`, and the
/// dirty set before each op.
fn scan(block: &Block, mut dirty: Set) -> (Set, Vec<Set>) {
    let mut before = Vec::new();
    for op in block.insns.iter().flat_map(|i| i.ops.iter()) {
        before.push(dirty);
        dirty |= op_effect(op).writes;
    }
    (dirty, before)
}

fn chunk_sync(prog: &Program, ctx: &Ctx<'_>, modified: &[Set], c: usize) -> ChunkSync {
    let blocks: Vec<&Block> = ctx.part.chunks[c]
        .blocks
        .iter()
        .map(|&b| &prog.blocks[b])
        .collect();
    let n = blocks.len();

    // Forward: may-dirty.
    let mut dirty_in = vec![0 as Set; n];
    let mut changed = true;
    while changed {
        changed = false;
        for k in 0..n {
            let (end, _) = scan(blocks[k], dirty_in[k]);
            let mut flow = |j: usize, d: Set| {
                if dirty_in[j] | d != dirty_in[j] {
                    dirty_in[j] |= d;
                    changed = true;
                }
            };
            match blocks[k].term {
                // The call stores everything dirty; reloads are clean.
                Term::Call { .. } => {}
                Term::Indirect(_) => (0..n).for_each(|j| flow(j, end)),
                ref term => {
                    for t in successors(term) {
                        if let Dest::Local(j) = ctx.dest(c, t) {
                            flow(j, end);
                        }
                    }
                }
            }
        }
    }

    // Backward: liveness, with every store a use.
    let mut live_in = vec![0 as Set; n];
    let after_call = |k: usize, live_in: &[Set]| match blocks[k].term {
        Term::Call { ret, .. } => match ctx.dest(c, ret) {
            Dest::Local(j) => live_in[j],
            // A tail transfer or a probe stores the dirty set, which is
            // empty after a call.
            Dest::Other(_) | Dest::Probe => 0,
        },
        _ => 0,
    };
    let mut changed = true;
    while changed {
        changed = false;
        for k in (0..n).rev() {
            let block = blocks[k];
            let (end, before) = scan(block, dirty_in[k]);
            let to = |t: u64, live_in: &[Set]| match ctx.dest(c, t) {
                Dest::Local(j) => live_in[j],
                Dest::Other(_) | Dest::Probe => end,
            };
            let mut live = term_reads(&block.term)
                | match block.term {
                    Term::Jump(t) => to(t, &live_in),
                    Term::Branch {
                        taken, fallthrough, ..
                    }
                    | Term::BranchIf {
                        taken, fallthrough, ..
                    } => to(taken, &live_in) | to(fallthrough, &live_in),
                    Term::Indirect(_) => live_in.iter().fold(end, |s, &l| s | l),
                    Term::Call { target, .. } => {
                        end | (after_call(k, &live_in) & !target_mod(ctx, modified, target))
                    }
                    Term::Ret(_) | Term::Exit { .. } | Term::Fault { .. } => end,
                };
            let ops: Vec<&Op> = block.insns.iter().flat_map(|i| i.ops.iter()).collect();
            for (op, &d) in ops.into_iter().zip(&before).rev() {
                let e = op_effect(op);
                live = (live & !e.kills) | e.uses | if e.may_leave { d } else { 0 };
            }
            if live != live_in[k] {
                live_in[k] = live;
                changed = true;
            }
        }
    }

    let reload = (0..n)
        .map(|k| match blocks[k].term {
            Term::Call { target, .. } => {
                after_call(k, &live_in) & target_mod(ctx, modified, target)
            }
            _ => 0,
        })
        .collect();
    let generic = live_in.iter().fold(0, |s, &l| s | l);
    let fast = own_entry(prog, ctx.part, c).map(|k| (k, live_in[k]));
    ChunkSync {
        dirty_in,
        live_in,
        reload,
        fast,
        generic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BinOp, FlagOp, Temp, Width};

    #[test]
    fn weights() {
        assert_eq!(weight(ALL), 20);
        assert_eq!(weight(FLAGS | 1), 5);
        assert_eq!(weight(0), 0);
    }

    #[test]
    fn effects() {
        let set = Op::SetReg {
            reg: 3,
            value: Value::Reg(3),
        };
        assert_eq!(
            op_effect(&set),
            Effect {
                uses: 1 << 3,
                kills: 1 << 3,
                writes: 1 << 3,
                may_leave: false
            }
        );
        let flags = |op, when| Op::Flags {
            op,
            width: Width::W64,
            a: Value::Reg(0),
            b: Value::Imm(1),
            r: Value::Temp(Temp(0)),
            when,
            live: true,
        };
        let full = op_effect(&flags(FlagOp::Sub, None));
        assert_eq!((full.uses, full.kills, full.writes), (1, FLAGS, FLAGS));
        let inc = op_effect(&flags(FlagOp::Inc, None));
        assert_eq!((inc.uses, inc.kills), (1 | FLAGS, 0));
        let cond = op_effect(&flags(FlagOp::Sub, Some(Value::Reg(1))));
        assert_eq!((cond.uses, cond.kills), (0b11, 0));
        let dead = Op::Flags {
            live: false,
            ..flags(FlagOp::Sub, None)
        };
        assert_eq!(op_effect(&dead), Effect::default());
        let movs = op_effect(&Op::RepString {
            op: StrOp::Movs,
            width: Width::W8,
            rep: Rep::Rep,
            delta: Value::Imm(1),
        });
        assert_eq!(movs.uses, RSI | RDI | RCX);
        assert_eq!(movs.writes, RSI | RDI | RCX);
        assert_eq!(movs.kills, 0);
        let bin = Op::Bin {
            dst: Temp(0),
            op: BinOp::Add,
            a: Value::Reg(4),
            b: Value::Reg(5),
        };
        assert_eq!(op_effect(&bin).uses, 0b11_0000);
    }
}
