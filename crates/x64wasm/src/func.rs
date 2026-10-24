//! Partitioning of a lifted program into guest functions and chunks (one
//! wasm function per chunk). See `docs/design/translator.md`.
//!
//! - **Entries** are the given entry points plus every direct call target
//!   that has a block.
//! - **Ownership.** Functions are processed in entry address order. Each one
//!   claims, breadth-first, the blocks it reaches through jumps, branches and
//!   call return sites. It never follows a call target and never enters
//!   another function's entry or a block already claimed. An edge to a block
//!   of another function is a tail transfer.
//! - **Chunks.** A function with more than [`Options::max_function_insns`]
//!   instructions is cut into chunks. Strongly connected components of its
//!   control-flow graph (loop nests) are packed whole, in address order; only
//!   a component larger than [`Options::max_chunk_insns`] is cut, into
//!   address-ordered pieces of at most that size.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt::Write as _;

use crate::Options;
use crate::ir::{Block, CallTarget, Program, Term};

/// A guest function: an entry and the chunks holding its blocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    /// Guest address of the entry block.
    pub entry: u64,
    /// Indices into [`Partition::chunks`], in order.
    pub chunks: Vec<usize>,
}

/// A chunk: the blocks of one wasm function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// Index into [`Partition::functions`].
    pub function: usize,
    /// Indices into [`Program::blocks`], sorted by address (which is also
    /// block order, since program blocks are sorted). A block's position in
    /// this list is its `entry` index.
    pub blocks: Vec<usize>,
}

/// The function and chunk structure of a program.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Partition {
    /// In entry address order.
    pub functions: Vec<Function>,
    /// Chunks in function order; a chunk's index is its table index.
    pub chunks: Vec<Chunk>,
    /// For every program block: (chunk index, entry index within the chunk).
    pub block_pos: Vec<(u32, u32)>,
}

impl Partition {
    /// Name of a chunk's wasm function: `fn_<entry>` for the chunk holding
    /// the function entry, `fn_<entry>_c<i>` for the others.
    pub fn chunk_name(&self, prog: &Program, chunk: usize) -> String {
        let c = &self.chunks[chunk];
        let func = &self.functions[c.function];
        let holds_entry = c.blocks.iter().any(|&b| prog.blocks[b].addr == func.entry);
        if holds_entry {
            format!("fn_{:#x}", func.entry)
        } else {
            let i = func.chunks.iter().position(|&x| x == chunk).unwrap();
            format!("fn_{:#x}_c{i}", func.entry)
        }
    }

    /// The IR dump grouped by function and chunk. `first_wasm_index` is the
    /// wasm function index of chunk 0.
    pub fn dump(&self, prog: &Program, first_wasm_index: u32) -> String {
        let mut out = String::new();
        for func in &self.functions {
            for (i, &c) in func.chunks.iter().enumerate() {
                if !out.is_empty() {
                    out.push('\n');
                }
                let _ = write!(out, "function {:#x}", func.entry);
                if func.chunks.len() > 1 {
                    let _ = write!(out, ", chunk {i} of {}", func.chunks.len());
                }
                let _ = writeln!(
                    out,
                    ": wasm function {} ({})",
                    first_wasm_index + c as u32,
                    self.chunk_name(prog, c)
                );
                for &b in &self.chunks[c].blocks {
                    let _ = write!(out, "\n{}", prog.blocks[b]);
                }
            }
        }
        out
    }
}

/// Blocks a block continues to inside its function: jump and branch
/// targets, a call's return site (never a call target), and the code after
/// an unsupported instruction.
fn local_successors(block: &Block) -> impl Iterator<Item = u64> {
    let (a, b) = match block.term {
        Term::Jump(t) => (Some(t), None),
        Term::Branch {
            taken, fallthrough, ..
        } => (Some(taken), Some(fallthrough)),
        Term::Call { ret, .. } => (Some(ret), None),
        Term::Indirect(_) | Term::Ret(_) | Term::Exit { .. } => (block.after_exit, None),
    };
    a.into_iter().chain(b)
}

/// Splits `prog` into functions and chunks.
pub fn partition(prog: &Program, entries: &[u64], opts: &Options) -> Partition {
    let index = prog.index_map();
    let n = prog.blocks.len();

    let mut starts: BTreeSet<usize> = entries
        .iter()
        .filter_map(|e| index.get(e).copied())
        .collect();
    for block in &prog.blocks {
        if let Term::Call {
            target: CallTarget::Direct(t),
            ..
        } = block.term
            && let Some(&i) = index.get(&t)
        {
            starts.insert(i);
        }
    }

    // Ownership: breadth-first claims in entry address order.
    let mut owner: Vec<Option<usize>> = vec![None; n];
    let mut members: Vec<(usize, Vec<usize>)> = Vec::new();
    let claim = |start: usize,
                 owner: &mut Vec<Option<usize>>,
                 members: &mut Vec<(usize, Vec<usize>)>,
                 starts: &BTreeSet<usize>| {
        let f = members.len();
        let mut blocks = vec![start];
        owner[start] = Some(f);
        let mut queue = VecDeque::from([start]);
        while let Some(b) = queue.pop_front() {
            for t in local_successors(&prog.blocks[b]) {
                if let Some(&j) = index.get(&t)
                    && owner[j].is_none()
                    && !starts.contains(&j)
                {
                    owner[j] = Some(f);
                    blocks.push(j);
                    queue.push_back(j);
                }
            }
        }
        blocks.sort_unstable();
        members.push((start, blocks));
    };
    for &start in &starts {
        claim(start, &mut owner, &mut members, &starts);
    }
    // Every block is reachable from an entry without following call
    // targets, so this only guards against a lifter change breaking that.
    for b in 0..n {
        if owner[b].is_none() {
            starts.insert(b);
            claim(b, &mut owner, &mut members, &starts);
        }
    }
    members.sort_by_key(|(start, _)| *start);

    let mut part = Partition {
        functions: Vec::with_capacity(members.len()),
        chunks: Vec::new(),
        block_pos: vec![(0, 0); n],
    };
    for (f, (start, blocks)) in members.into_iter().enumerate() {
        let mut func = Function {
            entry: prog.blocks[start].addr,
            chunks: Vec::new(),
        };
        for chunk_blocks in chunks_of(prog, &index, &blocks, opts) {
            let c = part.chunks.len();
            for (j, &b) in chunk_blocks.iter().enumerate() {
                part.block_pos[b] = (c as u32, j as u32);
            }
            func.chunks.push(c);
            part.chunks.push(Chunk {
                function: f,
                blocks: chunk_blocks,
            });
        }
        part.functions.push(func);
    }
    part
}

/// Cuts one function (its blocks, sorted) into chunks, each sorted.
fn chunks_of(
    prog: &Program,
    index: &HashMap<u64, usize>,
    blocks: &[usize],
    opts: &Options,
) -> Vec<Vec<usize>> {
    let insns = |b: usize| prog.blocks[b].insns.len();
    let total: usize = blocks.iter().map(|&b| insns(b)).sum();
    if total <= opts.max_function_insns {
        return vec![blocks.to_vec()];
    }
    let soft = opts.max_function_insns.max(1);
    let hard = opts.max_chunk_insns.max(soft);

    // Strongly connected components, packed in order of their first block.
    let mut units = sccs(prog, index, blocks);
    for unit in &mut units {
        unit.sort_unstable();
    }
    units.sort_by_key(|u| u[0]);

    fn flush(cur: &mut Vec<usize>, cur_insns: &mut usize, out: &mut Vec<Vec<usize>>) {
        if !cur.is_empty() {
            cur.sort_unstable();
            out.push(std::mem::take(cur));
            *cur_insns = 0;
        }
    }
    let mut out = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut cur_insns = 0;
    for unit in units {
        let size: usize = unit.iter().map(|&b| insns(b)).sum();
        if size > hard {
            // Too large to keep whole: cut it in address order.
            flush(&mut cur, &mut cur_insns, &mut out);
            for b in unit {
                if cur_insns > 0 && cur_insns + insns(b) > hard {
                    flush(&mut cur, &mut cur_insns, &mut out);
                }
                cur.push(b);
                cur_insns += insns(b);
            }
            flush(&mut cur, &mut cur_insns, &mut out);
            continue;
        }
        if cur_insns > 0 && cur_insns + size > soft {
            flush(&mut cur, &mut cur_insns, &mut out);
        }
        cur.extend(unit);
        cur_insns += size;
    }
    flush(&mut cur, &mut cur_insns, &mut out);
    out
}

/// Strongly connected components of the subgraph induced by `blocks`
/// (iterative Tarjan), each a list of program block indices.
fn sccs(prog: &Program, index: &HashMap<u64, usize>, blocks: &[usize]) -> Vec<Vec<usize>> {
    let local: HashMap<usize, usize> = blocks.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    let succ: Vec<Vec<usize>> = blocks
        .iter()
        .map(|&b| {
            local_successors(&prog.blocks[b])
                .filter_map(|t| index.get(&t).and_then(|j| local.get(j)).copied())
                .collect()
        })
        .collect();
    const NONE: usize = usize::MAX;
    let n = blocks.len();
    let mut order = vec![NONE; n];
    let mut low = vec![0; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut out = Vec::new();
    let mut next = 0;
    for root in 0..n {
        if order[root] != NONE {
            continue;
        }
        // (node, next successor to visit)
        let mut call: Vec<(usize, usize)> = vec![(root, 0)];
        order[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(top) = call.last_mut() {
            let v = top.0;
            if let Some(&w) = succ[v].get(top.1) {
                top.1 += 1;
                if order[w] == NONE {
                    order[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    call.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(order[w]);
                }
                continue;
            }
            call.pop();
            if let Some(&(parent, _)) = call.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == order[v] {
                let mut comp = Vec::new();
                loop {
                    let w = stack.pop().unwrap();
                    on_stack[w] = false;
                    comp.push(blocks[w]);
                    if w == v {
                        break;
                    }
                }
                out.push(comp);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Cond, Insn, Value};

    /// A program of blocks with `insns` placeholder instructions each.
    fn prog(blocks: &[(u64, usize, Term)]) -> Program {
        let blocks = blocks
            .iter()
            .map(|(addr, insns, term)| Block {
                addr: *addr,
                insns: (0..*insns as u64)
                    .map(|i| Insn {
                        addr: addr + i,
                        len: 1,
                        text: String::new(),
                        ops: Vec::new(),
                        temps: 0,
                    })
                    .collect(),
                term: term.clone(),
                after_exit: None,
            })
            .collect();
        Program {
            blocks,
            ..Program::default()
        }
    }

    fn opts(max_function_insns: usize, max_chunk_insns: usize) -> Options {
        Options {
            max_function_insns,
            max_chunk_insns,
            ..Options::default()
        }
    }

    /// Block addresses of every function's chunks.
    fn shape(p: &Program, part: &Partition) -> Vec<(u64, Vec<Vec<u64>>)> {
        part.functions
            .iter()
            .map(|f| {
                let chunks = f
                    .chunks
                    .iter()
                    .map(|&c| {
                        part.chunks[c]
                            .blocks
                            .iter()
                            .map(|&b| p.blocks[b].addr)
                            .collect()
                    })
                    .collect();
                (f.entry, chunks)
            })
            .collect()
    }

    fn call(target: u64, ret: u64) -> Term {
        Term::Call {
            target: CallTarget::Direct(target),
            ret,
        }
    }

    fn branch(taken: u64, fallthrough: u64) -> Term {
        Term::Branch {
            cond: Cond::E,
            taken,
            fallthrough,
        }
    }

    const RET: Term = Term::Ret(Value::Imm(0));

    /// Checks the invariants every partition has: each block in exactly one
    /// chunk, at the position `block_pos` says, chunks sorted.
    fn check_invariants(p: &Program, part: &Partition) {
        let mut seen = vec![false; p.blocks.len()];
        for (c, chunk) in part.chunks.iter().enumerate() {
            assert!(chunk.blocks.windows(2).all(|w| w[0] < w[1]));
            assert!(part.functions[chunk.function].chunks.contains(&c));
            for (j, &b) in chunk.blocks.iter().enumerate() {
                assert!(!seen[b], "block {b} in two chunks");
                seen[b] = true;
                assert_eq!(part.block_pos[b], (c as u32, j as u32));
            }
        }
        assert!(seen.iter().all(|&s| s), "every block is in a chunk");
    }

    #[test]
    fn calls_split_functions_and_return_sites_stay() {
        let p = prog(&[
            (0x10, 2, call(0x40, 0x15)),
            (0x15, 1, Term::Jump(0x20)),
            (0x20, 1, RET),
            (0x40, 1, branch(0x50, 0x45)),
            (0x45, 1, Term::Jump(0x50)),
            (0x50, 1, RET),
        ]);
        let part = partition(&p, &[0x10], &Options::default());
        check_invariants(&p, &part);
        assert_eq!(
            shape(&p, &part),
            [
                (0x10, vec![vec![0x10, 0x15, 0x20]]),
                (0x40, vec![vec![0x40, 0x45, 0x50]]),
            ]
        );
    }

    #[test]
    fn shared_tail_belongs_to_the_first_function() {
        // main calls b and a (b first, at the higher address); both jump to
        // the shared epilogue at 0x80. a also jumps straight into b's entry,
        // which stays b's.
        let p = prog(&[
            (0x10, 1, call(0x60, 0x15)),
            (0x15, 1, call(0x40, 0x1a)),
            (0x1a, 1, RET),
            (0x40, 1, branch(0x60, 0x48)),
            (0x48, 1, Term::Jump(0x80)),
            (0x60, 1, Term::Jump(0x80)),
            (0x80, 1, RET),
        ]);
        let part = partition(&p, &[0x10], &Options::default());
        check_invariants(&p, &part);
        assert_eq!(
            shape(&p, &part),
            [
                (0x10, vec![vec![0x10, 0x15, 0x1a]]),
                (0x40, vec![vec![0x40, 0x48, 0x80]]),
                (0x60, vec![vec![0x60]]),
            ]
        );
    }

    #[test]
    fn given_entries_and_call_targets_are_functions() {
        // 0x30 is a given entry inside the code that 0x10 jumps to: it stays
        // its own function. Call targets without a block don't count.
        let p = prog(&[
            (0x10, 1, call(0x9999, 0x20)),
            (0x20, 1, Term::Jump(0x30)),
            (0x30, 1, RET),
        ]);
        let part = partition(&p, &[0x10, 0x30], &Options::default());
        check_invariants(&p, &part);
        assert_eq!(
            shape(&p, &part),
            [(0x10, vec![vec![0x10, 0x20]]), (0x30, vec![vec![0x30]]),]
        );
    }

    #[test]
    fn small_functions_are_one_chunk() {
        let p = prog(&[(0x10, 5, Term::Jump(0x20)), (0x20, 5, RET)]);
        let part = partition(&p, &[0x10], &opts(10, 100));
        assert_eq!(shape(&p, &part), [(0x10, vec![vec![0x10, 0x20]])]);
        let part = partition(&p, &[0x10], &opts(9, 100));
        assert_eq!(shape(&p, &part), [(0x10, vec![vec![0x10], vec![0x20]])]);
    }

    /// A function made of loops: loop `i` is `sizes[i]` blocks of `insns`
    /// instructions, the last branching back to the first. Returns the
    /// program and the block addresses of each loop.
    fn loops(sizes: &[usize], insns: usize) -> (Program, Vec<Vec<u64>>) {
        let mut blocks = Vec::new();
        let mut groups = Vec::new();
        let mut addr = 0x1000u64;
        for &size in sizes {
            let first = addr;
            let mut group = Vec::new();
            for k in 0..size {
                let next = addr + 0x10;
                let term = if k + 1 == size {
                    branch(first, next)
                } else {
                    Term::Jump(next)
                };
                blocks.push((addr, insns, term));
                group.push(addr);
                addr = next;
            }
            groups.push(group);
        }
        blocks.push((addr, 1, RET));
        (prog(&blocks), groups)
    }

    #[test]
    fn chunk_cuts_never_split_a_loop_below_the_hard_cap() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for round in 0..200 {
            let sizes: Vec<usize> = (0..1 + next() % 30)
                .map(|_| 1 + (next() % 12) as usize)
                .collect();
            let insns = 1 + (next() % 4) as usize;
            let soft = 1 + (next() % 40) as usize;
            let hard = soft + (next() % 60) as usize;
            let (p, groups) = loops(&sizes, insns);
            let part = partition(&p, &[0x1000], &opts(soft, hard));
            check_invariants(&p, &part);
            assert_eq!(part.functions.len(), 1);
            let index = p.index_map();
            let chunk_of = |addr: u64| part.block_pos[index[&addr]].0;
            for group in &groups {
                let size = group.len() * insns;
                let chunks: BTreeSet<u32> = group.iter().map(|&a| chunk_of(a)).collect();
                if size <= hard {
                    assert_eq!(chunks.len(), 1, "round {round}: loop {group:x?} split");
                }
            }
            let total = p.insn_count();
            for chunk in &part.chunks {
                let n: usize = chunk.blocks.iter().map(|&b| p.blocks[b].insns.len()).sum();
                assert!(
                    n <= hard.max(insns),
                    "round {round}: chunk of {n} > hard {hard}"
                );
                if n > soft && total > soft {
                    // Only one loop (or a piece of one) exceeds the soft limit.
                    let owners: BTreeSet<usize> = chunk
                        .blocks
                        .iter()
                        .map(|&b| {
                            groups
                                .iter()
                                .position(|g| g.contains(&p.blocks[b].addr))
                                .unwrap_or(usize::MAX)
                        })
                        .collect();
                    assert_eq!(
                        owners.len(),
                        1,
                        "round {round}: oversized chunk mixes loops"
                    );
                }
            }
        }
    }

    #[test]
    fn a_loop_above_the_hard_cap_is_cut() {
        let (p, _) = loops(&[3, 50, 3], 2);
        let part = partition(&p, &[0x1000], &opts(10, 30));
        check_invariants(&p, &part);
        let sizes: Vec<usize> = part
            .chunks
            .iter()
            .map(|c| c.blocks.iter().map(|&b| p.blocks[b].insns.len()).sum())
            .collect();
        // 6, then the 100-instruction loop in pieces of 30, then 6 + the
        // final block.
        assert_eq!(sizes, [6, 30, 30, 30, 10, 7]);
    }

    #[test]
    fn names_and_dump_show_the_grouping() {
        let (p, _) = loops(&[2, 2], 3);
        let part = partition(&p, &[0x1000], &opts(6, 6));
        assert_eq!(part.chunks.len(), 3);
        assert_eq!(part.chunk_name(&p, 0), "fn_0x1000");
        assert_eq!(part.chunk_name(&p, 1), "fn_0x1000_c1");
        let dump = part.dump(&p, 3);
        assert!(dump.starts_with("function 0x1000, chunk 0 of 3: wasm function 3 (fn_0x1000)\n"));
        assert!(dump.contains("function 0x1000, chunk 2 of 3: wasm function 5 (fn_0x1000_c2)\n"));
    }
}
