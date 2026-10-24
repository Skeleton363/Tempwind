//! Decoding (iced-x86) and lowering of guest instructions into the IR.
//!
//! Code is discovered by recursive traversal from the entry points: direct
//! branch and call targets inside the region, conditional-branch
//! fall-throughs and call return sites become block leaders. Blocks are then
//! formed by decoding linearly from every leader until a terminator or the
//! next leader.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use iced_x86::{
    Code, ConditionCode, Decoder, DecoderError, FlowControl, Formatter, Instruction,
    IntelFormatter, Mnemonic, OpKind, Register,
};

use crate::abi;
use crate::ir::{
    BinOp, Block, CallTarget, Cond, FlagOp, Gpr, Insn, JumpTable, Op, Program, RSP, Temp, Term,
    Value, Width,
};
use crate::switch;
use crate::{CodeRegion, Error};

/// The instruction can't be lowered (yet).
#[derive(Debug)]
pub struct Unsupported;

type Lowered<T> = Result<T, Unsupported>;

/// One decoded and lowered instruction.
struct Decoded {
    insn: Insn,
    /// How control continues after the ops.
    flow: Flow,
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
                } => vec![taken, fallthrough],
                Term::Call {
                    target: CallTarget::Direct(t),
                    ret,
                } => vec![ret, t],
                Term::Call {
                    target: CallTarget::Indirect(_),
                    ret,
                } => vec![ret],
                Term::Indirect(_) | Term::Ret(_) | Term::Exit { .. } => Vec::new(),
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
                // Invalid encoding: exit with UNSUPPORTED at it (it would #UD).
                return Some(unsupported(addr, insn.len() as u32, "(bad)".into()));
            }
        }
        let mut lower = Lower {
            insn: &insn,
            bytes: &region.bytes[offset..offset + insn.len()],
            ops: Vec::new(),
            temps: 0,
            ea: None,
        };
        match lower.lower() {
            Ok(flow) => Some(Decoded {
                insn: Insn {
                    addr,
                    len: insn.len() as u32,
                    text,
                    ops: lower.ops,
                    temps: lower.temps,
                },
                flow,
            }),
            Err(Unsupported) => Some(unsupported(addr, insn.len() as u32, text)),
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
        },
        flow: Flow::End(Term::Exit {
            reason: abi::EXIT_UNSUPPORTED,
            rip: addr,
        }),
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
    // (cmp, jcc) address pairs: conditional branches and the instruction
    // before them, candidates for jump-table bounds.
    let mut branches: Vec<(u64, u64)> = Vec::new();
    let mut bounds: HashMap<u64, Vec<switch::Bound>> = HashMap::new();
    let mut jump_tables: BTreeMap<u64, JumpTable> = BTreeMap::new();
    let mut checked = 0;
    loop {
        while let Some(start) = work.pop() {
            let mut pc = start;
            let mut prev = None;
            while visited.insert(pc) {
                let Some(decoded) = lifter.decode(pc) else {
                    break;
                };
                let next = pc.wrapping_add(decoded.insn.len as u64);
                match &decoded.flow {
                    Flow::Next => {
                        prev = Some(pc);
                        pc = next;
                    }
                    flow => {
                        if let (Flow::End(Term::Branch { .. }), Some(p)) = (flow, prev) {
                            branches.push((p, pc));
                        }
                        for target in flow.successors() {
                            if lifter.contains(target) && leaders.insert(target) {
                                work.push(target);
                            }
                        }
                        break;
                    }
                }
            }
        }
        // Jump tables behind the bounds found so far; their targets are
        // more code to discover.
        let mut dirty = BTreeSet::new();
        for &(cmp, jcc) in &branches[checked..] {
            if let (Some(cmp), Some(jcc)) = (lifter.raw(cmp), lifter.raw(jcc))
                && let Some((block, bound)) = switch::bound(&cmp, &jcc)
            {
                bounds.entry(block).or_default().push(bound);
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
        let term = loop {
            let Some(decoded) = lifter.decode(pc) else {
                // Outside the region or cut off by its end.
                break Term::Exit {
                    reason: abi::EXIT_LEFT_CODE,
                    rip: pc,
                };
            };
            insns.push(decoded.insn.clone());
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
        });
    }
    Ok(Program {
        blocks,
        jump_tables: jump_tables.into_values().collect(),
        indirect_jumps: indirect_jumps.len(),
    })
}

/// A general-purpose register operand: which register and which part.
#[derive(Clone, Copy, Debug)]
enum Part {
    Q,
    D,
    W,
    Lo,
    Hi,
}

fn gpr(reg: Register) -> Lowered<(Gpr, Part)> {
    if !reg.is_gpr() {
        return Err(Unsupported);
    }
    let full = reg.full_register();
    let index = full as u32 - Register::RAX as u32;
    if index >= 16 {
        return Err(Unsupported);
    }
    let part = match reg {
        Register::AH | Register::CH | Register::DH | Register::BH => Part::Hi,
        _ => match reg.size() {
            8 => Part::Q,
            4 => Part::D,
            2 => Part::W,
            1 => Part::Lo,
            _ => return Err(Unsupported),
        },
    };
    Ok((index as Gpr, part))
}

fn gpr64(reg: Register) -> Lowered<Gpr> {
    match gpr(reg)? {
        (g, Part::Q) => Ok(g),
        _ => Err(Unsupported),
    }
}

fn cond(cc: ConditionCode) -> Lowered<Cond> {
    Ok(match cc {
        ConditionCode::o => Cond::O,
        ConditionCode::no => Cond::No,
        ConditionCode::b => Cond::B,
        ConditionCode::ae => Cond::Ae,
        ConditionCode::e => Cond::E,
        ConditionCode::ne => Cond::Ne,
        ConditionCode::be => Cond::Be,
        ConditionCode::a => Cond::A,
        ConditionCode::s => Cond::S,
        ConditionCode::ns => Cond::Ns,
        ConditionCode::p => Cond::P,
        ConditionCode::np => Cond::Np,
        ConditionCode::l => Cond::L,
        ConditionCode::ge => Cond::Ge,
        ConditionCode::le => Cond::Le,
        ConditionCode::g => Cond::G,
        _ => return Err(Unsupported),
    })
}

/// Lowering state for one instruction.
struct Lower<'i> {
    insn: &'i Instruction,
    bytes: &'i [u8],
    ops: Vec<Op>,
    temps: u32,
    /// The memory operand's effective address, once computed.
    ea: Option<Value>,
}

impl Lower<'_> {
    fn temp(&mut self) -> Temp {
        let t = Temp(self.temps);
        self.temps += 1;
        t
    }

    fn push_op(&mut self, op: Op) {
        self.ops.push(op);
    }

    fn bin(&mut self, op: BinOp, a: Value, b: Value) -> Value {
        if let (Value::Imm(x), Value::Imm(y)) = (a, b) {
            return Value::Imm(match op {
                BinOp::Add => x.wrapping_add(y),
                BinOp::Sub => x.wrapping_sub(y),
                BinOp::And => x & y,
                BinOp::Or => x | y,
                BinOp::Xor => x ^ y,
                BinOp::Shl => x.wrapping_shl(y as u32),
                BinOp::ShrU => x.wrapping_shr(y as u32),
            });
        }
        let dst = self.temp();
        self.push_op(Op::Bin { dst, op, a, b });
        Value::Temp(dst)
    }

    fn mask(&mut self, v: Value, width: Width) -> Value {
        match width {
            Width::W64 => v,
            w => self.bin(BinOp::And, v, Value::Imm(w.mask())),
        }
    }

    fn load(&mut self, width: Width, addr: Value) -> Value {
        let dst = self.temp();
        self.push_op(Op::Load { dst, width, addr });
        Value::Temp(dst)
    }

    fn store(&mut self, width: Width, addr: Value, value: Value) {
        self.push_op(Op::Store { width, addr, value });
    }

    fn set_reg(&mut self, reg: Gpr, value: Value) {
        self.push_op(Op::SetReg { reg, value });
    }

    /// Copies a value into a temporary so later register writes don't change it.
    fn snapshot(&mut self, v: Value) -> Value {
        match v {
            Value::Reg(_) => {
                let dst = self.temp();
                self.push_op(Op::Mov { dst, src: v });
                Value::Temp(dst)
            }
            v => v,
        }
    }

    /// True if a 67h (address-size) legacy prefix is present.
    fn has_addr32_prefix(&self) -> bool {
        for &b in self.bytes {
            match b {
                0x67 => return true,
                0x26 | 0x2e | 0x36 | 0x3e | 0x64 | 0x65 | 0x66 | 0xf0 | 0xf2 | 0xf3 => {}
                _ => return false,
            }
        }
        false
    }

    /// Effective address of the memory operand (no segment base).
    fn ea_offset(&mut self) -> Lowered<Value> {
        if let Some(v) = self.ea {
            return Ok(v);
        }
        if self.has_addr32_prefix() {
            return Err(Unsupported);
        }
        let insn = self.insn;
        let base = insn.memory_base();
        let index = insn.memory_index();
        let disp = insn.memory_displacement64();
        let v = if base == Register::RIP {
            // iced already resolved RIP-relative operands to an absolute address.
            if index != Register::None {
                return Err(Unsupported);
            }
            Value::Imm(disp)
        } else {
            let mut acc: Option<Value> = None;
            if base != Register::None {
                acc = Some(Value::Reg(gpr64(base)?));
            }
            if index != Register::None {
                let mut x = Value::Reg(gpr64(index)?);
                let shift = match insn.memory_index_scale() {
                    1 => 0,
                    2 => 1,
                    4 => 2,
                    8 => 3,
                    _ => return Err(Unsupported),
                };
                if shift != 0 {
                    x = self.bin(BinOp::Shl, x, Value::Imm(shift));
                }
                acc = Some(match acc {
                    Some(a) => self.bin(BinOp::Add, a, x),
                    None => x,
                });
            }
            match acc {
                None => Value::Imm(disp),
                Some(a) if disp == 0 => a,
                Some(a) => self.bin(BinOp::Add, a, Value::Imm(disp)),
            }
        };
        self.ea = Some(v);
        Ok(v)
    }

    /// Guest address of the memory operand. FS/GS-relative accesses are not
    /// supported yet; other segments have base 0 in 64-bit mode.
    fn ea(&mut self) -> Lowered<Value> {
        match self.insn.memory_segment() {
            Register::FS | Register::GS => Err(Unsupported),
            _ => self.ea_offset(),
        }
    }

    fn width_of(&self, operand: u32) -> Lowered<Width> {
        match self.insn.op_kind(operand) {
            OpKind::Register => {
                Width::from_bytes(self.insn.op_register(operand).size()).ok_or(Unsupported)
            }
            OpKind::Memory => Width::from_bytes(self.insn.memory_size().size()).ok_or(Unsupported),
            _ => Err(Unsupported),
        }
    }

    /// Reads an operand, zero-extended from `width`.
    fn read(&mut self, operand: u32, width: Width) -> Lowered<Value> {
        match self.insn.op_kind(operand) {
            OpKind::Register => {
                let (g, part) = gpr(self.insn.op_register(operand))?;
                let r = Value::Reg(g);
                Ok(match part {
                    Part::Q => r,
                    Part::D => self.mask(r, Width::W32),
                    Part::W => self.mask(r, Width::W16),
                    Part::Lo => self.mask(r, Width::W8),
                    Part::Hi => {
                        let shifted = self.bin(BinOp::ShrU, r, Value::Imm(8));
                        self.mask(shifted, Width::W8)
                    }
                })
            }
            OpKind::Memory => {
                let addr = self.ea()?;
                Ok(self.load(width, addr))
            }
            OpKind::Immediate8
            | OpKind::Immediate16
            | OpKind::Immediate32
            | OpKind::Immediate64
            | OpKind::Immediate8to16
            | OpKind::Immediate8to32
            | OpKind::Immediate8to64
            | OpKind::Immediate32to64 => {
                Ok(Value::Imm(self.insn.immediate(operand) & width.mask()))
            }
            _ => Err(Unsupported),
        }
    }

    /// Writes `value` (zero-extended from `width`) to an operand with x86
    /// partial-register semantics.
    fn write(&mut self, operand: u32, width: Width, value: Value) -> Lowered<()> {
        match self.insn.op_kind(operand) {
            OpKind::Register => {
                let (g, part) = gpr(self.insn.op_register(operand))?;
                let r = Value::Reg(g);
                let merged = match part {
                    // 32-bit writes zero-extend; `value` is already zero-extended.
                    Part::Q | Part::D => value,
                    Part::W | Part::Lo => {
                        let kept = self.bin(BinOp::And, r, Value::Imm(!width.mask()));
                        self.bin(BinOp::Or, kept, value)
                    }
                    Part::Hi => {
                        let kept = self.bin(BinOp::And, r, Value::Imm(!0xff00));
                        let shifted = self.bin(BinOp::Shl, value, Value::Imm(8));
                        self.bin(BinOp::Or, kept, shifted)
                    }
                };
                self.set_reg(g, merged);
                Ok(())
            }
            OpKind::Memory => {
                let addr = self.ea()?;
                self.store(width, addr, value);
                Ok(())
            }
            _ => Err(Unsupported),
        }
    }

    fn push64(&mut self, value: Value) {
        let sp = self.bin(BinOp::Sub, Value::Reg(RSP), Value::Imm(8));
        self.store(Width::W64, sp, value);
        self.set_reg(RSP, sp);
    }

    fn lower(&mut self) -> Lowered<Flow> {
        let insn = self.insn;
        if insn.has_lock_prefix() {
            return Err(Unsupported);
        }
        let next = insn.next_ip();
        match insn.mnemonic() {
            Mnemonic::Nop => Ok(Flow::Next),
            Mnemonic::Mov => {
                if insn.op_count() != 2 {
                    return Err(Unsupported);
                }
                let w = self.width_of(0)?;
                let v = self.read(1, w)?;
                self.write(0, w, v)?;
                Ok(Flow::Next)
            }
            m @ (Mnemonic::Add | Mnemonic::Sub | Mnemonic::Cmp) => {
                if insn.op_count() != 2 {
                    return Err(Unsupported);
                }
                let w = self.width_of(0)?;
                let a = self.read(0, w)?;
                let b = self.read(1, w)?;
                let (bin, fop) = if m == Mnemonic::Add {
                    (BinOp::Add, FlagOp::Add)
                } else {
                    (BinOp::Sub, FlagOp::Sub)
                };
                let raw = self.bin(bin, a, b);
                let r = self.mask(raw, w);
                self.push_op(Op::Flags {
                    op: fop,
                    width: w,
                    a,
                    b,
                    r,
                    live: true,
                });
                if m != Mnemonic::Cmp {
                    self.write(0, w, r)?;
                }
                Ok(Flow::Next)
            }
            Mnemonic::Lea => {
                if insn.op_kind(0) != OpKind::Register || insn.op_kind(1) != OpKind::Memory {
                    return Err(Unsupported);
                }
                let w = self.width_of(0)?;
                if w == Width::W8 {
                    return Err(Unsupported);
                }
                let addr = self.ea_offset()?;
                let v = self.mask(addr, w);
                self.write(0, w, v)?;
                Ok(Flow::Next)
            }
            Mnemonic::Push => {
                let v = match insn.code() {
                    Code::Push_r64 | Code::Pushq_imm8 | Code::Pushq_imm32 | Code::Push_rm64 => {
                        self.read(0, Width::W64)?
                    }
                    _ => return Err(Unsupported),
                };
                self.push64(v);
                Ok(Flow::Next)
            }
            Mnemonic::Pop => {
                if !matches!(insn.code(), Code::Pop_r64 | Code::Pop_rm64) {
                    return Err(Unsupported);
                }
                let v = self.load(Width::W64, Value::Reg(RSP));
                let sp = self.bin(BinOp::Add, Value::Reg(RSP), Value::Imm(8));
                self.set_reg(RSP, sp);
                // A memory destination's address uses RSP after the increment.
                self.write(0, Width::W64, v)?;
                Ok(Flow::Next)
            }
            Mnemonic::Call => match insn.code() {
                Code::Call_rel32_64 => {
                    self.push64(Value::Imm(next));
                    Ok(Flow::End(Term::Call {
                        target: CallTarget::Direct(insn.near_branch_target()),
                        ret: next,
                    }))
                }
                Code::Call_rm64 => {
                    let target = self.read(0, Width::W64)?;
                    let target = self.snapshot(target);
                    self.push64(Value::Imm(next));
                    Ok(Flow::End(Term::Call {
                        target: CallTarget::Indirect(target),
                        ret: next,
                    }))
                }
                _ => Err(Unsupported),
            },
            Mnemonic::Ret => {
                let extra = match insn.code() {
                    Code::Retnq => 0,
                    Code::Retnq_imm16 => insn.immediate16() as u64,
                    _ => return Err(Unsupported),
                };
                let target = self.load(Width::W64, Value::Reg(RSP));
                let sp = self.bin(BinOp::Add, Value::Reg(RSP), Value::Imm(8 + extra));
                self.set_reg(RSP, sp);
                Ok(Flow::End(Term::Ret(target)))
            }
            Mnemonic::Jmp => match insn.code() {
                Code::Jmp_rel8_64 | Code::Jmp_rel32_64 => {
                    Ok(Flow::End(Term::Jump(insn.near_branch_target())))
                }
                Code::Jmp_rm64 => {
                    let target = self.read(0, Width::W64)?;
                    Ok(Flow::End(Term::Indirect(target)))
                }
                _ => Err(Unsupported),
            },
            _ if insn.is_jcc_short_or_near() => {
                if insn.op_kind(0) != OpKind::NearBranch64 {
                    return Err(Unsupported);
                }
                Ok(Flow::End(Term::Branch {
                    cond: cond(insn.condition_code())?,
                    taken: insn.near_branch_target(),
                    fallthrough: next,
                }))
            }
            _ => Err(Unsupported),
        }
    }
}
