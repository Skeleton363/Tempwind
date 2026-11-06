//! Structured layout of a chunk's blocks by loop nesting
//! (`docs/design/translator.md`, "Jumps and branches", since M6).
//!
//! [`plan`] turns a chunk's CFG into a [`Plan`] the emitter walks:
//!
//! - **Loop forest:** recursive SCC decomposition. Each non-trivial SCC (or
//!   a block that jumps to itself) is a loop; one block is its header (for an
//!   irreducible SCC, the one with the most edges from outside); the loop's
//!   body is decomposed again without the header.
//! - **Order:** as LLVM's CFGSort orders blocks: at each level the blocks and
//!   (collapsed) child loops are sorted topologically, ignoring back edges to
//!   the level's header, preferring the lowest chunk index among ready nodes.
//!   So every loop's blocks are contiguous, with its header first, and every
//!   edge that is not a back edge goes forward.
//! - **Nesting:** every loop is a wasm `loop`. Every block is an entry, so
//!   every node of a level (except its first) is a target of that level's
//!   dispatch; its `block` therefore opens at the level's top, which is where
//!   CFGStackify puts it too (the nearest common dominator of its
//!   predecessors). Forward edges are `br` out of these `block`s, back edges
//!   `br` to the `loop`.
//! - **Selector:** an edge into a loop's interior (an irreducible edge, a
//!   generic entry, a dynamic re-entry) sets the selector local to the
//!   target's layout position and goes to the loop's top. The top of a loop
//!   with more than one block holds a [`Item::Select`]: `sel == NONE` falls
//!   into the header (one compare per iteration); otherwise a `br_table` on
//!   `sel - start` routes to the target's node, and nested loops route again
//!   level by level. The selector is `NONE` whenever a block's code runs.
//! - **Root dispatch:** the chunk's top holds the root [`Item::Select`], a
//!   `br_table` on the `entry` parameter (a chunk block index). Local indirect
//!   jumps set `entry` and `br` to `$dispatch`, which is then a `loop` around
//!   the root level ([`Plan::dispatch_loop`]); otherwise nothing branches
//!   back to it and it needs no `loop`.
//!
//! Label depths in a [`Transfer`] count only the labels of [`Plan::items`]
//! open at that point (outermost first); the emitter adds its own (`$leave`
//! outside, `if`s inside a block's code).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::func::sccs;
use crate::ir::{Program, Term};

/// The selector's value when no interior entry is pending.
pub(crate) const NONE: u32 = u32::MAX;

/// A chunk's layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    /// Chunk block indices in layout order.
    pub order: Vec<usize>,
    /// Layout position of each chunk block (the inverse of `order`).
    pub pos: Vec<usize>,
    /// The loop forest; a parent comes before its children.
    pub loops: Vec<Loop>,
    /// Innermost loop of each chunk block.
    pub loop_of: Vec<Option<usize>>,
    /// The structure, in emission order.
    pub items: Vec<Item>,
    /// Per chunk block: each successor inside the chunk (chunk block index,
    /// each once) and how control gets there from the end of its code.
    pub edges: Vec<Vec<(usize, Transfer)>>,
    /// Per chunk block (the `entry` value): the root dispatch's route.
    pub entries: Vec<Transfer>,
    /// Per chunk block ending in an indirect jump: the depth of `$dispatch`
    /// from its code (set `entry`, then `br`).
    pub indirect: Vec<Option<u32>>,
    /// `$dispatch` is a `loop` (something re-enters through it).
    pub dispatch_loop: bool,
}

/// One loop of the forest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Loop {
    /// Chunk block index of the header.
    pub header: usize,
    pub parent: Option<usize>,
    /// Layout positions `start..end`; `order[start]` is the header.
    pub start: usize,
    pub end: usize,
    /// More than one block has edges from outside the loop.
    pub irreducible: bool,
    /// The selector dispatch at the loop's top, indexed by `sel - start`
    /// (empty for a one-block loop, which has no interior).
    pub routes: Vec<Transfer>,
}

/// One element of the structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    /// `loop $dispatch` around the root level.
    DispatchLoop,
    /// `loop` of a loop of the forest.
    Loop(usize),
    /// `block` ending right before the item that follows its `End`.
    Block,
    /// The dispatch at the top of the root level (`None`, on `entry`) or of a
    /// loop (on the selector, only when it isn't `NONE`).
    Select(Option<usize>),
    /// The code of a chunk block.
    Code(usize),
    End,
}

/// How control reaches a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Transfer {
    pub sel: Sel,
    pub jump: Jump,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sel {
    /// Leave the selector as it is.
    Keep,
    /// Set it to this layout position (an entry into a loop's interior).
    Set(u32),
    /// Set it to `NONE` (the dispatch has arrived).
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Jump {
    /// Continue with the next item.
    Fall,
    /// `br` to this label depth.
    Br(u32),
}

/// Lays out the chunk made of the program blocks `blocks` (chunk block `k` is
/// `prog.blocks[blocks[k]]`). `index` maps block addresses to program block
/// indices. Edges are jumps, branches and return sites to blocks of the
/// chunk; every block is an entry.
pub(crate) fn plan(prog: &Program, index: &HashMap<u64, usize>, blocks: &[usize]) -> Plan {
    let n = blocks.len();
    let local: HashMap<usize, usize> = blocks.iter().enumerate().map(|(k, &b)| (b, k)).collect();
    // Executed edges only: a subset of the edges `sccs` follows (which also
    // has the discovery-only edge after an exit), so every cycle here lies
    // inside one of its components.
    let mut succ: Vec<Vec<usize>> = blocks
        .iter()
        .map(|&b| {
            let ts = match prog.blocks[b].term {
                Term::Jump(t) => [Some(t), None],
                Term::Branch {
                    taken, fallthrough, ..
                }
                | Term::BranchIf {
                    taken, fallthrough, ..
                } => [Some(taken), Some(fallthrough)],
                Term::Call { ret, .. } => [Some(ret), None],
                Term::Indirect(_) | Term::Ret(_) | Term::Exit { .. } | Term::Fault { .. } => {
                    [None, None]
                }
            };
            ts.into_iter()
                .flatten()
                .filter_map(|t| index.get(&t).and_then(|j| local.get(j)).copied())
                .collect()
        })
        .collect();
    for s in &mut succ {
        s.sort_unstable();
        s.dedup();
    }
    let mut preds = vec![Vec::new(); n];
    for (k, s) in succ.iter().enumerate() {
        for &t in s {
            preds[t].push(k);
        }
    }

    let mut b = Builder {
        prog,
        index,
        blocks,
        local: &local,
        succ: &succ,
        preds: &preds,
        loops: Vec::new(),
        loop_of: vec![None; n],
        order: Vec::with_capacity(n),
    };
    let all: Vec<usize> = (0..n).collect();
    let nodes = b.decompose(&all, None);
    b.lay_out(nodes);
    let Builder {
        mut loops,
        loop_of,
        order,
        ..
    } = b;
    let mut pos = vec![0; n];
    for (q, &k) in order.iter().enumerate() {
        pos[k] = q;
    }
    let dispatch_loop = blocks
        .iter()
        .any(|&b| matches!(prog.blocks[b].term, Term::Indirect(_)));

    // Items and the label each opening item stands for.
    let mut items = Vec::new();
    if dispatch_loop {
        items.push(Item::DispatchLoop);
    }
    emit_level(&mut items, &order, &loops, &loop_of, None, 0, n);
    if dispatch_loop {
        items.push(Item::End);
    }

    let tree = Tree {
        loops: &loops,
        loop_of: &loop_of,
        pos: &pos,
    };
    let mut edges = vec![Vec::new(); n];
    let mut entries = vec![None; n];
    let mut indirect = vec![None; n];
    let mut routes: Vec<Vec<Transfer>> = loops
        .iter()
        .map(|l| {
            let len = if l.end - l.start > 1 {
                l.end - l.start
            } else {
                0
            };
            vec![
                Transfer {
                    sel: Sel::Clear,
                    jump: Jump::Fall
                };
                len
            ]
        })
        .collect();
    // Open labels: `Some(position)` for a `block` ending at that position,
    // `None` for a `loop` (with its loop index, `None` for `$dispatch`).
    let mut stack: Vec<Label> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        match *it {
            Item::DispatchLoop => stack.push(Label::Loop(None)),
            Item::Loop(l) => stack.push(Label::Loop(Some(l))),
            Item::Block => stack.push(Label::Block(block_end(&items, i, &pos))),
            Item::End => {
                stack.pop();
            }
            Item::Select(None) => {
                for (k, e) in entries.iter_mut().enumerate() {
                    let (sel, to) = tree.descend(None, k, Sel::Keep);
                    *e = Some(Transfer {
                        sel,
                        jump: resolve(&stack, to, 0),
                    });
                }
            }
            Item::Select(Some(l)) => {
                let lp = &loops[l];
                for q in lp.start + 1..lp.end {
                    let (sel, to) = tree.descend(Some(l), order[q], Sel::Clear);
                    routes[l][q - lp.start] = Transfer {
                        sel,
                        jump: resolve(&stack, to, usize::MAX),
                    };
                }
            }
            Item::Code(s) => {
                let next = pos[s] + 1;
                for &t in &succ[s] {
                    let (sel, to) = tree.edge(s, t);
                    edges[s].push((
                        t,
                        Transfer {
                            sel,
                            jump: resolve(&stack, to, next),
                        },
                    ));
                }
                if matches!(prog.blocks[blocks[s]].term, Term::Indirect(_)) {
                    let d = stack
                        .iter()
                        .rev()
                        .position(|&l| l == Label::Loop(None))
                        .expect("indirect jump without $dispatch");
                    indirect[s] = Some(d as u32);
                }
            }
        }
    }
    for (lp, r) in loops.iter_mut().zip(routes) {
        lp.routes = r;
    }
    Plan {
        order,
        pos,
        loops,
        loop_of,
        items,
        edges,
        entries: entries.into_iter().map(Option::unwrap).collect(),
        indirect,
        dispatch_loop,
    }
}

/// A label of the structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Label {
    /// A `block` ending right before the node at this layout position.
    Block(usize),
    /// A `loop`: a loop of the forest, or `$dispatch`.
    Loop(Option<usize>),
}

/// Where a transfer goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum To {
    /// The node starting at this layout position, forward.
    Node(usize),
    /// The top of this loop, backward.
    Back(usize),
}

/// A fall-through when the target is the next position (`next`), else a `br`
/// to the label for it.
fn resolve(stack: &[Label], to: To, next: usize) -> Jump {
    let label = match to {
        To::Node(q) if q == next => return Jump::Fall,
        To::Node(q) => Label::Block(q),
        To::Back(l) => Label::Loop(Some(l)),
    };
    let d = stack
        .iter()
        .rev()
        .position(|&x| x == label)
        .unwrap_or_else(|| panic!("no label for {to:?}"));
    Jump::Br(d as u32)
}

/// Layout position of the node that follows the `End` of the `block` at
/// `items[i]`.
fn block_end(items: &[Item], i: usize, pos: &[usize]) -> usize {
    let after = items[i..]
        .iter()
        .scan(0i32, |d, it| {
            match it {
                Item::DispatchLoop | Item::Loop(_) | Item::Block => *d += 1,
                Item::End => *d -= 1,
                _ => {}
            }
            Some(*d)
        })
        .position(|d| d == 0)
        .expect("unclosed block");
    // The first block code after the `End`.
    let k = items[i + after + 1..]
        .iter()
        .find_map(|it| match it {
            Item::Code(k) => Some(*k),
            _ => None,
        })
        .expect("block ends after the last code");
    pos[k]
}

/// Emits the level of loop `l` (`None`: the root), layout positions
/// `start..end`.
fn emit_level(
    items: &mut Vec<Item>,
    order: &[usize],
    loops: &[Loop],
    loop_of: &[Option<usize>],
    l: Option<usize>,
    start: usize,
    end: usize,
) {
    // The nodes of this level: (start position, child loop or None).
    let mut nodes = Vec::new();
    let mut q = start;
    if l.is_some() {
        nodes.push((q, None));
        q += 1;
    }
    while q < end {
        match child_of(loops, loop_of[order[q]], l) {
            Some(c) => {
                nodes.push((q, Some(c)));
                q = loops[c].end;
            }
            None => {
                nodes.push((q, None));
                q += 1;
            }
        }
    }
    if let Some(l) = l {
        items.push(Item::Loop(l));
    }
    let interior = end - start > 1;
    for _ in 1..nodes.len() {
        items.push(Item::Block);
    }
    if l.is_none() || interior {
        items.push(Item::Select(l));
    }
    for (i, &(q, c)) in nodes.iter().enumerate() {
        if i > 0 {
            items.push(Item::End);
        }
        match c {
            Some(c) => emit_level(items, order, loops, loop_of, Some(c), q, loops[c].end),
            None => items.push(Item::Code(order[q])),
        }
    }
    if l.is_some() {
        items.push(Item::End);
    }
}

/// The child of `level` (a loop, or `None` for the root) on the way from
/// `level` down to `inner`, or `None` if `inner` is `level` itself.
fn child_of(loops: &[Loop], mut inner: Option<usize>, level: Option<usize>) -> Option<usize> {
    let mut child = None;
    while inner != level {
        let i = inner.expect("not inside the level");
        child = Some(i);
        inner = loops[i].parent;
    }
    child
}

struct Tree<'a> {
    loops: &'a [Loop],
    loop_of: &'a [Option<usize>],
    pos: &'a [usize],
}

impl Tree<'_> {
    fn depth(&self, mut l: Option<usize>) -> usize {
        let mut d = 0;
        while let Some(i) = l {
            d += 1;
            l = self.loops[i].parent;
        }
        d
    }

    /// The innermost loop containing both blocks (`None`: the root).
    fn common(&self, a: usize, b: usize) -> Option<usize> {
        let (mut x, mut y) = (self.loop_of[a], self.loop_of[b]);
        let (mut dx, mut dy) = (self.depth(x), self.depth(y));
        while dx > dy {
            x = self.loops[x.unwrap()].parent;
            dx -= 1;
        }
        while dy > dx {
            y = self.loops[y.unwrap()].parent;
            dy -= 1;
        }
        while x != y {
            x = self.loops[x.unwrap()].parent;
            y = self.loops[y.unwrap()].parent;
        }
        x
    }

    /// From the top of `level` towards block `t` inside it: the selector
    /// action and the node of `level` to go to. `arrived` is the selector
    /// action when that node is `t` itself or a loop `t` heads.
    fn descend(&self, level: Option<usize>, t: usize, arrived: Sel) -> (Sel, To) {
        match child_of(self.loops, self.loop_of[t], level) {
            None => (arrived, To::Node(self.pos[t])),
            Some(c) => {
                let lp = &self.loops[c];
                let sel = if lp.header == t {
                    arrived
                } else if arrived == Sel::Clear {
                    // The selector already holds `t`.
                    Sel::Keep
                } else {
                    Sel::Set(self.pos[t] as u32)
                };
                (sel, To::Node(lp.start))
            }
        }
    }

    /// The edge from the code of `s` to `t`.
    fn edge(&self, s: usize, t: usize) -> (Sel, To) {
        let a = self.common(s, t);
        match a {
            Some(a) if self.loops[a].header == t => (Sel::Keep, To::Back(a)),
            _ => self.descend(a, t, Sel::Keep),
        }
    }
}

/// Builds the loop forest and the order.
struct Builder<'a> {
    prog: &'a Program,
    index: &'a HashMap<u64, usize>,
    blocks: &'a [usize],
    local: &'a HashMap<usize, usize>,
    succ: &'a [Vec<usize>],
    preds: &'a [Vec<usize>],
    loops: Vec<Loop>,
    loop_of: Vec<Option<usize>>,
    order: Vec<usize>,
}

/// A node of one level: a block, or a loop with its own nodes (header first).
enum Node {
    Block(usize),
    Loop(usize, Vec<Node>),
}

impl Builder<'_> {
    /// Decomposes the chunk blocks `set` (one level, inside loop `parent`)
    /// into nodes in layout order.
    fn decompose(&mut self, set: &[usize], parent: Option<usize>) -> Vec<Node> {
        let prog_set: Vec<usize> = set.iter().map(|&k| self.blocks[k]).collect();
        let comps: Vec<Vec<usize>> = sccs(self.prog, self.index, &prog_set)
            .into_iter()
            .map(|c| {
                let mut c: Vec<usize> = c.iter().map(|b| self.local[b]).collect();
                c.sort_unstable();
                c
            })
            .collect();
        // Topological order of the components (Kahn's, lowest block first).
        let mut comp_of: HashMap<usize, usize> = HashMap::new();
        for (i, c) in comps.iter().enumerate() {
            for &k in c {
                comp_of.insert(k, i);
            }
        }
        let mut indeg = vec![0usize; comps.len()];
        let mut out: Vec<Vec<usize>> = vec![Vec::new(); comps.len()];
        for (i, c) in comps.iter().enumerate() {
            for &k in c {
                for &t in &self.succ[k] {
                    if let Some(&j) = comp_of.get(&t)
                        && j != i
                    {
                        out[i].push(j);
                        indeg[j] += 1;
                    }
                }
            }
        }
        let mut ready: BinaryHeap<Reverse<(usize, usize)>> = comps
            .iter()
            .enumerate()
            .filter(|&(i, _)| indeg[i] == 0)
            .map(|(i, c)| Reverse((c[0], i)))
            .collect();
        let mut nodes = Vec::with_capacity(comps.len());
        while let Some(Reverse((_, i))) = ready.pop() {
            for &j in &out[i] {
                indeg[j] -= 1;
                if indeg[j] == 0 {
                    ready.push(Reverse((comps[j][0], j)));
                }
            }
            nodes.push(self.node(&comps[i], parent));
        }
        assert_eq!(nodes.len(), comps.len(), "component graph has a cycle");
        nodes
    }

    /// The node of one component: a block, or a loop.
    fn node(&mut self, comp: &[usize], parent: Option<usize>) -> Node {
        if comp.len() == 1 && !self.succ[comp[0]].contains(&comp[0]) {
            self.loop_of[comp[0]] = parent;
            return Node::Block(comp[0]);
        }
        // Header: the block with the most edges from outside the component
        // (lowest index on ties, and when nothing outside reaches it).
        let outside = |k: usize| {
            self.preds[k]
                .iter()
                .filter(|p| comp.binary_search(p).is_err())
                .count()
        };
        let header = *comp
            .iter()
            .max_by_key(|&&k| (outside(k), Reverse(k)))
            .unwrap();
        let irreducible = comp.iter().filter(|&&k| outside(k) > 0).count() > 1;
        let l = self.loops.len();
        self.loops.push(Loop {
            header,
            parent,
            start: 0,
            end: 0,
            irreducible,
            routes: Vec::new(),
        });
        self.loop_of[header] = Some(l);
        let rest: Vec<usize> = comp.iter().copied().filter(|&k| k != header).collect();
        let mut inner = vec![Node::Block(header)];
        if !rest.is_empty() {
            inner.extend(self.decompose(&rest, Some(l)));
        }
        Node::Loop(l, inner)
    }

    /// Appends the nodes to `order`, recording loop ranges.
    fn lay_out(&mut self, nodes: Vec<Node>) {
        for node in nodes {
            match node {
                Node::Block(k) => self.order.push(k),
                Node::Loop(l, inner) => {
                    self.loops[l].start = self.order.len();
                    self.lay_out(inner);
                    self.loops[l].end = self.order.len();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Block, CallTarget, Cond, Value};

    /// How a test block ends.
    #[derive(Clone, Debug)]
    enum T {
        Ret,
        Jump(usize),
        Branch(usize, usize),
        Call(usize),
        Indirect,
    }

    const BASE: u64 = 0x1000;

    fn addr(i: usize) -> u64 {
        BASE + 16 * i as u64
    }

    /// A program of the given blocks (targets are program block indices).
    fn prog(terms: &[T]) -> Program {
        let blocks = terms
            .iter()
            .enumerate()
            .map(|(i, t)| Block {
                addr: addr(i),
                insns: Vec::new(),
                term: match *t {
                    T::Ret => Term::Ret(Value::Imm(0)),
                    T::Jump(j) => Term::Jump(addr(j)),
                    T::Branch(a, b) => Term::Branch {
                        cond: Cond::E,
                        taken: addr(a),
                        fallthrough: addr(b),
                    },
                    T::Call(r) => Term::Call {
                        target: CallTarget::Direct(0x9000),
                        ret: addr(r),
                    },
                    T::Indirect => Term::Indirect(Value::Imm(0)),
                },
                after_exit: None,
            })
            .collect();
        Program {
            blocks,
            ..Program::default()
        }
    }

    /// Chunk-local successors of each chunk block, as the emitter sees them.
    fn succs(p: &Program, blocks: &[usize]) -> Vec<Vec<usize>> {
        let local: HashMap<u64, usize> = blocks
            .iter()
            .enumerate()
            .map(|(k, &b)| (p.blocks[b].addr, k))
            .collect();
        blocks
            .iter()
            .map(|&b| {
                let ts: Vec<u64> = match p.blocks[b].term {
                    Term::Jump(t) => vec![t],
                    Term::Branch {
                        taken, fallthrough, ..
                    }
                    | Term::BranchIf {
                        taken, fallthrough, ..
                    } => vec![taken, fallthrough],
                    Term::Call { ret, .. } => vec![ret],
                    _ => vec![],
                };
                ts.iter().filter_map(|t| local.get(t).copied()).collect()
            })
            .collect()
    }

    fn whole(terms: &[T]) -> (Program, Vec<usize>, Plan) {
        let p = prog(terms);
        let blocks: Vec<usize> = (0..terms.len()).collect();
        let plan = plan(&p, &p.index_map(), &blocks);
        check(&p, &blocks, &plan);
        (p, blocks, plan)
    }

    /// Matching `End` of each opening item, and the opening item of each `End`.
    fn matching(items: &[Item]) -> Vec<usize> {
        let mut m = vec![usize::MAX; items.len()];
        let mut stack = Vec::new();
        for (i, it) in items.iter().enumerate() {
            match it {
                Item::DispatchLoop | Item::Loop(_) | Item::Block => stack.push(i),
                Item::End => {
                    let o = stack.pop().expect("unbalanced End");
                    m[o] = i;
                    m[i] = o;
                }
                _ => {}
            }
        }
        assert!(stack.is_empty(), "unclosed item");
        m
    }

    /// Static invariants: order is a permutation, loops are contiguous with
    /// their header first and nest, items are balanced and list every block
    /// once in order, every edge has a transfer.
    fn check(p: &Program, blocks: &[usize], plan: &Plan) {
        let n = blocks.len();
        let mut sorted = plan.order.clone();
        sorted.sort();
        assert_eq!(sorted, (0..n).collect::<Vec<_>>());
        for (q, &k) in plan.order.iter().enumerate() {
            assert_eq!(plan.pos[k], q);
        }
        for (l, lp) in plan.loops.iter().enumerate() {
            assert_eq!(plan.order[lp.start], lp.header);
            assert!(lp.start < lp.end && lp.end <= n);
            for q in lp.start..lp.end {
                // Every block in the range lies in `l` or a loop inside it.
                let mut c = plan.loop_of[plan.order[q]];
                while c != Some(l) {
                    c = plan.loops[c.expect("block outside its loop's range")].parent;
                }
            }
            if let Some(par) = lp.parent {
                assert!(par < l);
                let pp = &plan.loops[par];
                assert!(pp.start < lp.start && lp.end <= pp.end);
            }
        }
        matching(&plan.items);
        let codes: Vec<usize> = plan
            .items
            .iter()
            .filter_map(|it| match it {
                Item::Code(k) => Some(*k),
                _ => None,
            })
            .collect();
        assert_eq!(codes, plan.order);
        let s = succs(p, blocks);
        for k in 0..n {
            let mut want = s[k].clone();
            want.sort();
            want.dedup();
            let mut have: Vec<usize> = plan.edges[k].iter().map(|e| e.0).collect();
            have.sort();
            assert_eq!(have, want, "edges of block {k}");
            let ind = matches!(p.blocks[blocks[k]].term, Term::Indirect(_));
            assert_eq!(plan.indirect[k].is_some(), ind);
            if ind {
                assert!(plan.dispatch_loop);
            }
        }
        assert_eq!(plan.entries.len(), n);
    }

    /// Label stack machine over `plan.items`.
    struct Sim<'a> {
        plan: &'a Plan,
        m: Vec<usize>,
        pc: usize,
        stack: Vec<usize>,
        sel: u32,
        entry: usize,
    }

    impl<'a> Sim<'a> {
        /// Starts at the top of the chunk with `entry`.
        fn new(plan: &'a Plan, entry: usize) -> Self {
            Sim {
                plan,
                m: matching(&plan.items),
                pc: 0,
                stack: Vec::new(),
                sel: NONE,
                entry,
            }
        }

        fn br(&mut self, d: u32) {
            let at = self.stack.len() - 1 - d as usize;
            let label = self.stack[at];
            match self.plan.items[label] {
                Item::Block => {
                    self.pc = self.m[label] + 1;
                    self.stack.truncate(at);
                }
                Item::Loop(_) | Item::DispatchLoop => {
                    self.pc = label + 1;
                    self.stack.truncate(at + 1);
                }
                it => panic!("br to {it:?}"),
            }
        }

        fn apply(&mut self, t: Transfer) {
            match t.sel {
                Sel::Keep => {}
                Sel::Set(v) => self.sel = v,
                Sel::Clear => self.sel = NONE,
            }
            match t.jump {
                Jump::Fall => self.pc += 1,
                Jump::Br(d) => self.br(d),
            }
        }

        /// Runs to the next block's code and returns that block.
        fn run(&mut self) -> usize {
            for _ in 0..4 * self.plan.items.len() + 4 {
                match self.plan.items[self.pc] {
                    Item::DispatchLoop | Item::Loop(_) | Item::Block => {
                        self.stack.push(self.pc);
                        self.pc += 1;
                    }
                    Item::End => {
                        assert_eq!(self.stack.pop(), Some(self.m[self.pc]));
                        self.pc += 1;
                    }
                    Item::Select(None) => self.apply(self.plan.entries[self.entry]),
                    Item::Select(Some(l)) => {
                        if self.sel == NONE {
                            self.pc += 1;
                        } else {
                            let lp = &self.plan.loops[l];
                            let i = (self.sel as usize)
                                .checked_sub(lp.start)
                                .filter(|&i| i < lp.end - lp.start)
                                .expect("selector outside the loop");
                            self.apply(lp.routes[i]);
                        }
                    }
                    Item::Code(k) => {
                        assert_eq!(self.sel, NONE, "selector pending at block {k}");
                        return k;
                    }
                }
            }
            panic!("no block reached");
        }

        /// Takes the edge from block `k` (whose code is current) to `t`.
        fn take(&mut self, k: usize, t: usize) {
            let tr = self.plan.edges[k]
                .iter()
                .find(|e| e.0 == t)
                .unwrap_or_else(|| panic!("edge {k}->{t} unplaced"))
                .1;
            self.apply(tr);
        }

        /// Takes block `k`'s indirect jump to `t`.
        fn indirect(&mut self, k: usize, t: usize) {
            self.entry = t;
            self.br(self.plan.indirect[k].expect("no indirect route"));
        }
    }

    /// Follows `path` (chunk block indices; consecutive ones must be edges,
    /// or `Indirect` jumps) from entry `path[0]`.
    fn walk(p: &Program, blocks: &[usize], plan: &Plan, path: &[usize]) {
        let mut sim = Sim::new(plan, path[0]);
        assert_eq!(sim.run(), path[0]);
        for w in path.windows(2) {
            if matches!(p.blocks[blocks[w[0]]].term, Term::Indirect(_)) {
                sim.indirect(w[0], w[1]);
            } else {
                sim.take(w[0], w[1]);
            }
            assert_eq!(sim.run(), w[1], "after {}->{}", w[0], w[1]);
        }
    }

    fn n_loops(plan: &Plan) -> usize {
        plan.loops.len()
    }

    #[test]
    fn straight_line() {
        let (p, b, plan) = whole(&[T::Jump(1), T::Call(2), T::Ret]);
        assert_eq!(plan.order, [0, 1, 2]);
        assert_eq!(n_loops(&plan), 0);
        assert!(!plan.dispatch_loop);
        for k in 0..2 {
            assert_eq!(
                plan.edges[k],
                [(
                    k + 1,
                    Transfer {
                        sel: Sel::Keep,
                        jump: Jump::Fall
                    }
                )]
            );
        }
        for e in 0..3 {
            walk(&p, &b, &plan, &(e..3).collect::<Vec<_>>());
        }
    }

    #[test]
    fn if_else() {
        // 0: if -> 1 else 2; 1 -> 3; 2 -> 3; 3: ret
        let (p, b, plan) = whole(&[T::Branch(2, 1), T::Jump(3), T::Jump(3), T::Ret]);
        assert_eq!(plan.order, [0, 1, 2, 3]);
        assert_eq!(n_loops(&plan), 0);
        assert!(matches!(plan.edges[1][0].1.jump, Jump::Br(_)));
        walk(&p, &b, &plan, &[0, 1, 3]);
        walk(&p, &b, &plan, &[0, 2, 3]);
        walk(&p, &b, &plan, &[2, 3]);
    }

    #[test]
    fn self_loop() {
        let (p, b, plan) = whole(&[T::Jump(1), T::Branch(1, 2), T::Ret]);
        assert_eq!(n_loops(&plan), 1);
        assert_eq!(plan.loops[0].header, 1);
        assert!(plan.loops[0].routes.is_empty());
        assert!(!plan.items.contains(&Item::Select(Some(0))));
        walk(&p, &b, &plan, &[0, 1, 1, 1, 2]);
    }

    #[test]
    fn nested_loops_break_and_continue() {
        // 0 -> 1 (outer header) -> 2 (inner header) -> {5 break out of
        // both, 3}; 3 -> {2 continue, 4}; 4 -> {1 continue outer, 5}.
        let (p, b, plan) = whole(&[
            T::Jump(1),
            T::Jump(2),
            T::Branch(5, 3),
            T::Branch(2, 4),
            T::Branch(1, 5),
            T::Ret,
        ]);
        assert_eq!(plan.order, [0, 1, 2, 3, 4, 5]);
        assert_eq!(n_loops(&plan), 2);
        let outer = &plan.loops[0];
        let inner = &plan.loops[1];
        assert_eq!((outer.header, outer.start, outer.end), (1, 1, 5));
        assert_eq!((inner.header, inner.start, inner.end), (2, 2, 4));
        assert_eq!(inner.parent, Some(0));
        assert!(!outer.irreducible && !inner.irreducible);
        // continue (3 -> 2) and the outer back edge (4 -> 1) are `br` to loops.
        walk(&p, &b, &plan, &[0, 1, 2, 3, 2, 3, 4, 1, 2, 3, 4, 5]);
        // break from the inner loop out of both (2 -> 5).
        walk(&p, &b, &plan, &[0, 1, 2, 5]);
    }

    #[test]
    fn two_entry_irreducible_loop() {
        // 0 -> {1, 2}; 1 <-> 2; 2 -> 3.
        let (p, b, plan) = whole(&[T::Branch(1, 2), T::Jump(2), T::Branch(1, 3), T::Ret]);
        assert_eq!(n_loops(&plan), 1);
        let lp = &plan.loops[0];
        assert!(lp.irreducible);
        // The entry that isn't the header goes through the selector.
        let other = if lp.header == 1 { 2 } else { 1 };
        let e = plan.edges[0].iter().find(|e| e.0 == other).unwrap().1;
        assert_eq!(e.sel, Sel::Set(plan.pos[other] as u32));
        walk(&p, &b, &plan, &[0, 1, 2, 1, 2, 3]);
        walk(&p, &b, &plan, &[0, 2, 1, 2, 3]);
    }

    #[test]
    fn entries_into_loop_interiors() {
        // Outer loop 1..=4 with inner loop 2..=3; block 6 is a return site
        // inside the inner loop (3 calls, returns to 6, 6 -> 2).
        let (p, b, plan) = whole(&[
            T::Jump(1),
            T::Jump(2),
            T::Branch(4, 3),
            T::Call(6),
            T::Branch(1, 5),
            T::Ret,
            T::Jump(2),
        ]);
        assert_eq!(n_loops(&plan), 2);
        let inner = plan.loop_of[6].unwrap();
        assert_eq!(plan.loops[inner].header, 2);
        assert!(plan.loops[inner].parent.is_some());
        // Root dispatch into the inner loop's interior sets the selector.
        assert_eq!(plan.entries[6].sel, Sel::Set(plan.pos[6] as u32));
        assert_eq!(plan.entries[3].sel, Sel::Set(plan.pos[3] as u32));
        assert_eq!(plan.entries[1].sel, Sel::Keep);
        for e in 0..7 {
            walk(&p, &b, &plan, &[e]);
        }
        walk(&p, &b, &plan, &[6, 2, 3, 6, 2, 4, 1, 2, 4, 5]);
        walk(&p, &b, &plan, &[3, 6, 2, 4, 5]);
    }

    #[test]
    fn indirect_jumps_reenter_through_dispatch() {
        let (p, b, plan) = whole(&[T::Jump(1), T::Indirect, T::Jump(1), T::Ret]);
        assert!(plan.dispatch_loop);
        assert_eq!(plan.items[0], Item::DispatchLoop);
        walk(&p, &b, &plan, &[0, 1, 2, 1, 3]);
        walk(&p, &b, &plan, &[1, 0, 1, 1]);
    }

    #[test]
    fn chunk_subset_ignores_outside_edges() {
        // Program blocks 0..5; the chunk is {1, 3, 4}; 1 -> 3 -> 4 -> 1,
        // and edges to 0, 2 are tail transfers.
        let p = prog(&[T::Ret, T::Branch(3, 0), T::Ret, T::Branch(4, 2), T::Jump(1)]);
        let blocks = vec![1, 3, 4];
        let plan = plan(&p, &p.index_map(), &blocks);
        check(&p, &blocks, &plan);
        assert_eq!(n_loops(&plan), 1);
        walk(&p, &blocks, &plan, &[0, 1, 2, 0, 1]);
        walk(&p, &blocks, &plan, &[2, 0]);
    }

    /// xorshift64*.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    fn random_cfg(rng: &mut Rng) -> (Program, Vec<usize>) {
        let n = 1 + rng.below(28);
        let near = |rng: &mut Rng, i: usize| {
            if rng.below(3) == 0 {
                rng.below(n)
            } else {
                (i + 2 * n + rng.below(5) - 2) % n
            }
        };
        let terms: Vec<T> = (0..n)
            .map(|i| match rng.below(10) {
                0 => T::Ret,
                1 => T::Indirect,
                2 => T::Call(near(rng, i)),
                3..=5 => T::Jump(near(rng, i)),
                _ => T::Branch(near(rng, i), near(rng, i)),
            })
            .collect();
        let p = prog(&terms);
        // Usually the whole program; sometimes a subset (other blocks are
        // other chunks).
        let blocks: Vec<usize> = if rng.below(4) == 0 {
            (0..n).filter(|_| rng.below(4) != 0).collect()
        } else {
            (0..n).collect()
        };
        if blocks.is_empty() {
            return (p, vec![0]);
        }
        (p, blocks)
    }

    #[test]
    fn simulator_on_random_cfgs() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut irreducible = 0;
        let mut nested = 0;
        for _ in 0..3000 {
            let (p, blocks) = random_cfg(&mut rng);
            let plan = plan(&p, &p.index_map(), &blocks);
            check(&p, &blocks, &plan);
            irreducible += plan.loops.iter().filter(|l| l.irreducible).count();
            nested += plan.loops.iter().filter(|l| l.parent.is_some()).count();
            let s = succs(&p, &blocks);
            let n = blocks.len();
            let indirect = |k: usize| matches!(p.blocks[blocks[k]].term, Term::Indirect(_));
            // Every entry, and every edge from every block, then a random
            // continuation.
            let mut starts: Vec<Vec<usize>> = (0..n).map(|k| vec![k]).collect();
            for (k, sk) in s.iter().enumerate() {
                for &t in sk {
                    starts.push(vec![k, t]);
                }
                if indirect(k) {
                    starts.push(vec![k, rng.below(n)]);
                }
            }
            for mut path in starts {
                for _ in 0..40 {
                    let k = *path.last().unwrap();
                    let next = if indirect(k) {
                        Some(rng.below(n))
                    } else if s[k].is_empty() {
                        None
                    } else {
                        Some(s[k][rng.below(s[k].len())])
                    };
                    match next {
                        Some(t) => path.push(t),
                        None => break,
                    }
                }
                walk(&p, &blocks, &plan, &path);
            }
        }
        // The generator exercises the interesting shapes.
        assert!(irreducible > 100, "{irreducible} irreducible loops");
        assert!(nested > 100, "{nested} nested loops");
    }
}
