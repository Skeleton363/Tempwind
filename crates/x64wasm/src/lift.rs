//! Decoding (iced-x86) and lowering of guest instructions into the IR.
//!
//! Code is discovered by recursive traversal from the entry points: direct
//! branch and call targets inside the region, conditional-branch
//! fall-throughs and call return sites become block leaders. Blocks are then
//! formed by decoding linearly from every leader until a terminator or the
//! next leader.
//!
//! Lowering is split by instruction family: `lift/lower.rs` holds the
//! per-instruction state, the op builders and the dispatch by mnemonic,
//! `lift/operand.rs` registers, effective addresses, operand reads and
//! writes, and each other submodule one family.

mod alu;
mod atomic;
mod bit;
mod branch;
mod cc;
mod data;
mod ext;
mod lower;
mod misc;
mod muldiv;
mod operand;
mod shift;
mod stack;
mod string;
mod x87;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use iced_x86::{Code, Decoder, DecoderError, FlowControl, Formatter, Instruction, IntelFormatter};

use crate::abi;
use crate::ir::{Block, CallTarget, Insn, JumpTable, Program, Term};
use crate::switch;
use crate::{CodeRegion, Error};
use lower::Lower;

/// The instruction can't be lowered (yet).
#[derive(Debug)]
pub struct Unsupported;

type Lowered<T> = Result<T, Unsupported>;

/// One decoded and lowered instruction.
struct Decoded {
    insn: Insn,
    /// How control continues after the ops.
    flow: Flow,
    /// For an unsupported instruction that would continue to the next one
    /// (it isn't a jump, return or trap): the next address. Discovery
    /// continues there, so the rest of the function is translated.
    after: Option<u64>,
}

enum Flow {
    /// Falls through to the next instruction.
    Next,
    /// Ends the block.
    End(Term),
}

impl Flow {
    /// Addresses that become block leaders: branch targets, call targets and
    /// return sites.
    fn successors(&self) -> Vec<u64> {
        match self {
            Flow::Next => Vec::new(),
            Flow::End(term) => match *term {
                Term::Jump(t) => vec![t],
                Term::Branch {
                    taken, fallthrough, ..
                }
                | Term::BranchIf {
                    taken, fallthrough, ..
                } => vec![taken, fallthrough],
                Term::Call {
                    target: CallTarget::Direct(t),
                    ret,
                } => vec![ret, t],
                Term::Call {
                    target: CallTarget::Indirect(_),
                    ret,
                } => vec![ret],
                Term::Indirect(_) | Term::Ret(_) | Term::Exit { .. } | Term::Fault { .. } => {
                    Vec::new()
                }
            },
        }
    }
}

/// What a lift reads: code regions to decode, memory to read jump tables
/// from, and addresses that already have blocks elsewhere.
#[derive(Clone, Copy, Debug)]
pub struct Source<'a> {
    /// Executable code; only this is decoded.
    pub code: &'a [CodeRegion<'a>],
    /// Readable memory for jump tables (an image's sections; may include
    /// the code).
    pub data: &'a [CodeRegion<'a>],
    /// Sorted addresses that count as outside the code.
    pub known: &'a [u64],
}

struct Lifter<'a> {
    regions: &'a [CodeRegion<'a>],
    data: &'a [CodeRegion<'a>],
    /// One decoder per region.
    decoders: Vec<Decoder<'a>>,
    /// Sorted addresses that already have blocks elsewhere: never decoded.
    known: &'a [u64],
    formatter: IntelFormatter,
    want_text: bool,
    /// Decoded instructions by address; `None` = can't decode there (outside
    /// the region or truncated by its end).
    cache: HashMap<u64, Option<Decoded>>,
}

impl<'a> Lifter<'a> {
    fn region_of(&self, addr: u64) -> Option<usize> {
        self.regions
            .iter()
            .position(|r| addr.wrapping_sub(r.base) < r.bytes.len() as u64)
    }

    fn is_known(&self, addr: u64) -> bool {
        self.known.binary_search(&addr).is_ok()
    }

    /// `addr` is code to translate here: inside a region and not known.
    fn contains(&self, addr: u64) -> bool {
        self.region_of(addr).is_some() && !self.is_known(addr)
    }

    /// The instruction at `addr`, decoded but not lowered.
    fn raw(&mut self, addr: u64) -> Option<Instruction> {
        let r = self.region_of(addr)?;
        let decoder = &mut self.decoders[r];
        decoder
            .set_position((addr - self.regions[r].base) as usize)
            .ok()?;
        decoder.set_ip(addr);
        let insn = decoder.decode();
        (decoder.last_error() == DecoderError::None).then_some(insn)
    }

    /// Raw instructions from `addr` up to the first control transfer
    /// (included), at most `max`.
    fn raw_run(&mut self, addr: u64, max: usize) -> Vec<Instruction> {
        let mut out = Vec::new();
        let mut pc = addr;
        while out.len() < max {
            let Some(insn) = self.raw(pc) else { break };
            pc = insn.next_ip();
            let flow = insn.flow_control();
            out.push(insn);
            if flow != FlowControl::Next {
                break;
            }
        }
        out
    }

    fn read_data(&self, addr: u64, len: usize) -> Option<&'a [u8]> {
        self.data.iter().find_map(|r| {
            let off = usize::try_from(addr.checked_sub(r.base)?).ok()?;
            r.bytes.get(off..off.checked_add(len)?)
        })
    }

    /// The jump table of the block at `leader`, if it matches the pattern,
    /// one of `bounds` checks its index, and every entry is a code address.
    fn jump_table(&mut self, leader: u64, bounds: &[switch::Bound]) -> Option<JumpTable> {
        let insns = self.raw_run(leader, switch::MAX_BLOCK_INSNS);
        let c = switch::match_block(&insns)?;
        let count = bounds
            .iter()
            .filter(|b| b.operand == c.index_source)
            .map(|b| b.count)
            .max()?;
        let bytes = self.read_data(c.table, count as usize * 4)?;
        let mut targets = Vec::with_capacity(count as usize);
        for e in bytes.chunks_exact(4) {
            let rel = i32::from_le_bytes([e[0], e[1], e[2], e[3]]);
            let t = c.table.wrapping_add(rel as i64 as u64);
            self.region_of(t)?;
            targets.push(t);
        }
        Some(JumpTable {
            jmp: c.jmp,
            table: c.table,
            targets,
        })
    }

    fn decode(&mut self, addr: u64) -> Option<&Decoded> {
        if !self.cache.contains_key(&addr) {
            let decoded = self.decode_uncached(addr);
            self.cache.insert(addr, decoded);
        }
        self.cache[&addr].as_ref()
    }

    fn decode_uncached(&mut self, addr: u64) -> Option<Decoded> {
        if self.is_known(addr) {
            return None;
        }
        let r = self.region_of(addr)?;
        let region = self.regions[r];
        let offset = (addr - region.base) as usize;
        let decoder = &mut self.decoders[r];
        decoder.set_position(offset).ok()?;
        decoder.set_ip(addr);
        let insn = decoder.decode();
        let error = decoder.last_error();
        let text = if self.want_text {
            let mut text = String::new();
            self.formatter.format(&insn, &mut text);
            text
        } else {
            String::new()
        };
        match error {
            DecoderError::None => {}
            DecoderError::NoMoreBytes => return None,
            _ => {
                // Invalid encoding: the CPU raises #UD there.
                let mut d = unsupported(addr, insn.len() as u32, "(bad)".into());
                d.flow = Flow::End(Term::Fault {
                    kind: abi::FAULT_INVALID_OPCODE,
                    rip: addr,
                });
                return Some(d);
            }
        }
        let mut lower = Lower {
            insn: &insn,
            bytes: &region.bytes[offset..offset + insn.len()],
            ops: Vec::new(),
            temps: 0,
            vtemps: 0,
            ea: None,
            seg_ea: None,
        };
        match lower.lower() {
            Ok(flow) => Some(Decoded {
                insn: Insn {
                    addr,
                    len: insn.len() as u32,
                    text,
                    ops: lower.ops,
                    temps: lower.temps,
                    vtemps: lower.vtemps,
                },
                flow,
                after: None,
            }),
            Err(Unsupported) => {
                let continues = matches!(
                    insn.flow_control(),
                    FlowControl::Next
                        | FlowControl::Call
                        | FlowControl::IndirectCall
                        | FlowControl::ConditionalBranch
                        | FlowControl::XbeginXabortXend
                );
                let mut d = unsupported(addr, insn.len() as u32, text);
                d.after = continues.then(|| insn.next_ip());
                Some(d)
            }
        }
    }
}

fn unsupported(addr: u64, len: u32, text: String) -> Decoded {
    Decoded {
        insn: Insn {
            addr,
            len,
            text,
            ops: Vec::new(),
            temps: 0,
            vtemps: 0,
        },
        flow: Flow::End(Term::Exit {
            reason: abi::EXIT_UNSUPPORTED,
            rip: addr,
        }),
        after: None,
    }
}

/// Discovers and lowers all code reachable from `entries`.
pub fn lift(region: CodeRegion<'_>, entries: &[u64], want_text: bool) -> Result<Program, Error> {
    lift_regions(&[region], &[], entries, want_text)
}

/// Like [`lift`], over a [`Source`]: several code regions, memory that jump
/// tables are read from, and known addresses.
pub fn lift_source(src: &Source<'_>, entries: &[u64], want_text: bool) -> Result<Program, Error> {
    lift_impl(src.code, src.data, src.known, entries, want_text)
}

/// Like [`lift`], over several code regions (an image's executable
/// sections). Addresses in `known` (sorted) count as outside the code: they
/// are never decoded, and transfers to them stay transfers to an address
/// without a block here (lookup transfers in the emitted module).
/// Jump tables are read from the code regions.
pub fn lift_regions(
    regions: &[CodeRegion<'_>],
    known: &[u64],
    entries: &[u64],
    want_text: bool,
) -> Result<Program, Error> {
    lift_impl(regions, regions, known, entries, want_text)
}

fn lift_impl(
    regions: &[CodeRegion<'_>],
    data: &[CodeRegion<'_>],
    known: &[u64],
    entries: &[u64],
    want_text: bool,
) -> Result<Program, Error> {
    let mut lifter = Lifter {
        regions,
        data,
        decoders: regions
            .iter()
            .map(|r| Decoder::with_ip(64, r.bytes, r.base, crate::DECODER_OPTIONS))
            .collect(),
        known,
        formatter: IntelFormatter::new(),
        want_text,
        cache: HashMap::new(),
    };
    for &entry in entries {
        if lifter.region_of(entry).is_none() {
            return Err(Error::EntryOutsideRegion(entry));
        }
        if lifter.is_known(entry) {
            return Err(Error::EntryKnown(entry));
        }
    }

    // Pass 1: traversal. Collect leaders.
    let mut leaders: BTreeSet<u64> = entries.iter().copied().collect();
    let mut work: Vec<u64> = entries.to_vec();
    let mut visited: BTreeSet<u64> = BTreeSet::new();
    // (before, cmp, jcc) addresses: conditional branches and the two
    // instructions before them, candidates for jump-table bounds.
    let mut branches: Vec<(Option<u64>, u64, u64)> = Vec::new();
    let mut bounds: HashMap<u64, Vec<switch::Bound>> = HashMap::new();
    let mut jump_tables: BTreeMap<u64, JumpTable> = BTreeMap::new();
    let mut checked = 0;
    loop {
        while let Some(start) = work.pop() {
            let mut pc = start;
            let mut prev = None;
            let mut prev2 = None;
            while visited.insert(pc) {
                let Some(decoded) = lifter.decode(pc) else {
                    break;
                };
                let next = pc.wrapping_add(decoded.insn.len as u64);
                let after = decoded.after;
                let (successors, is_branch) = match &decoded.flow {
                    Flow::Next => {
                        prev2 = prev;
                        prev = Some(pc);
                        pc = next;
                        continue;
                    }
                    flow => (
                        flow.successors(),
                        matches!(flow, Flow::End(Term::Branch { .. })),
                    ),
                };
                if let (true, Some(p)) = (is_branch, prev) {
                    branches.push((prev2, p, pc));
                }
                for target in successors.into_iter().chain(after) {
                    if lifter.contains(target) && leaders.insert(target) {
                        work.push(target);
                    }
                }
                break;
            }
        }
        // Jump tables behind the bounds found so far; their targets are
        // more code to discover.
        let mut dirty = BTreeSet::new();
        for &(before, cmp, jcc) in &branches[checked..] {
            let before = before.and_then(|b| lifter.raw(b));
            if let (Some(cmp), Some(jcc)) = (lifter.raw(cmp), lifter.raw(jcc))
                && let Some((block, found)) = switch::bound(before.as_ref(), &cmp, &jcc)
            {
                bounds.entry(block).or_default().extend(found);
                dirty.insert(block);
            }
        }
        checked = branches.len();
        for block in dirty {
            let Some(table) = lifter.jump_table(block, &bounds[&block]) else {
                continue;
            };
            for &t in &table.targets {
                if lifter.contains(t) && leaders.insert(t) {
                    work.push(t);
                }
            }
            jump_tables.entry(table.jmp).or_insert(table);
        }
        if work.is_empty() {
            break;
        }
    }

    // Pass 2: form blocks.
    let known = lifter.known;
    let mut blocks = Vec::with_capacity(leaders.len());
    let mut indirect_jumps: BTreeSet<u64> = jump_tables.keys().copied().collect();
    for &leader in &leaders {
        let mut insns = Vec::new();
        let mut pc = leader;
        let mut after_exit = None;
        let term = loop {
            let Some(decoded) = lifter.decode(pc) else {
                // Outside the region or cut off by its end.
                break Term::Exit {
                    reason: abi::EXIT_LEFT_CODE,
                    rip: pc,
                };
            };
            insns.push(decoded.insn.clone());
            after_exit = decoded.after.filter(|&a| leaders.contains(&a));
            let next = pc.wrapping_add(decoded.insn.len as u64);
            match &decoded.flow {
                Flow::End(term) => break term.clone(),
                Flow::Next if leaders.contains(&next) || known.binary_search(&next).is_ok() => {
                    break Term::Jump(next);
                }
                Flow::Next => pc = next,
            }
        };
        match term {
            Term::Indirect(_) => {
                if let Some(last) = insns.last() {
                    indirect_jumps.insert(last.addr);
                }
            }
            Term::Exit {
                reason: abi::EXIT_UNSUPPORTED,
                rip,
            } => {
                // Decode on past the unsupported instruction to the block's
                // real end.
                let run = lifter.raw_run(rip, 16);
                if let Some(last) = run.last()
                    && last.code() == Code::Jmp_rm64
                {
                    indirect_jumps.insert(last.ip());
                }
            }
            _ => {}
        }
        blocks.push(Block {
            addr: leader,
            insns,
            term,
            after_exit,
        });
    }
    Ok(Program {
        blocks,
        jump_tables: jump_tables.into_values().collect(),
        indirect_jumps: indirect_jumps.len(),
    })
}
