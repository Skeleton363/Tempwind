//! The translator's IR: a small linear register-transfer language between
//! decoding (iced-x86) and code generation (wasm-encoder).
//!
//! Shape:
//! - A [`Program`] is a list of basic [`Block`]s sorted by guest address.
//! - A block holds guest instructions ([`Insn`]) in order and ends in one
//!   [`Term`] (jump, conditional branch, call, return, indirect branch or
//!   exit). Calls and returns stay explicit because they map to wasm calls
//!   and returns; their ops already did the stack work (push the return
//!   address; pop it and adjust RSP), exactly as native code does.
//! - Each instruction lowers to [`Op`]s over 64-bit values. A [`Value`] is a
//!   guest register (read at the point the op executes), an instruction-local
//!   temporary, or an immediate. Temporaries never live across instruction
//!   boundaries; guest registers carry all state between instructions.
//! - Narrow (8/16/32-bit) arithmetic is lowered to 64-bit ops plus explicit
//!   masks, and partial-register writes to explicit merges, so the code
//!   generator stays a direct mapping. Values handed to a narrow write or to
//!   [`Op::Flags`] are always zero-extended to their width.
//! - Status flags are lazy: [`Op::Flags`] records the operation, width, both
//!   operands and the result; flags are derived from that record only when a
//!   condition or an exit needs them. [`Program::analyze_flags`] marks records
//!   that no later reader can observe as dead.

use std::collections::HashMap;
use std::fmt::{self, Write as _};

use crate::abi;

/// A general-purpose register in x86 encoding order (0 = RAX ... 15 = R15).
pub type Gpr = u8;

/// Index of RSP.
pub const RSP: Gpr = 4;

/// Operand width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Width {
    W8,
    W16,
    W32,
    W64,
}

impl Width {
    pub fn from_bytes(bytes: usize) -> Option<Width> {
        Some(match bytes {
            1 => Width::W8,
            2 => Width::W16,
            4 => Width::W32,
            8 => Width::W64,
            _ => return None,
        })
    }

    pub const fn bits(self) -> u32 {
        match self {
            Width::W8 => 8,
            Width::W16 => 16,
            Width::W32 => 32,
            Width::W64 => 64,
        }
    }

    /// All-ones mask of this width.
    pub const fn mask(self) -> u64 {
        match self {
            Width::W64 => u64::MAX,
            w => (1u64 << w.bits()) - 1,
        }
    }

    /// The sign bit of this width.
    pub const fn sign_bit(self) -> u64 {
        1u64 << (self.bits() - 1)
    }

    /// 0..=3 for 8/16/32/64.
    pub const fn index(self) -> u32 {
        match self {
            Width::W8 => 0,
            Width::W16 => 1,
            Width::W32 => 2,
            Width::W64 => 3,
        }
    }
}

/// An instruction-local temporary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Temp(pub u32);

/// An operand of an op.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    /// The full 64-bit guest register, as it is when the op executes.
    Reg(Gpr),
    Temp(Temp),
    Imm(u64),
}

/// 64-bit binary operations (wrapping).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Shl,
    ShrU,
}

/// The operation a lazy flag record describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagOp {
    /// `r = a + b`
    Add,
    /// `r = a - b` (also CMP)
    Sub,
}

impl FlagOp {
    /// The runtime encoding of a record kind (the `cc_op` local): 0 means the
    /// flags are held materialized in `cc_a`; `1 + 4*op + width` otherwise.
    pub const fn cc_op(self, width: Width) -> u32 {
        let op = match self {
            FlagOp::Add => 0,
            FlagOp::Sub => 1,
        };
        1 + 4 * op + width.index()
    }
}

/// Runtime `cc_op` value meaning "flags are materialized in `cc_a`".
pub const CC_EAGER: u32 = 0;

/// One IR operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// `dst = src`
    Mov { dst: Temp, src: Value },
    /// `dst = a op b`
    Bin {
        dst: Temp,
        op: BinOp,
        a: Value,
        b: Value,
    },
    /// `dst = zero-extended load of `width` at guest address `addr``
    Load {
        dst: Temp,
        width: Width,
        addr: Value,
    },
    /// Stores the low `width` bits of `value` at guest address `addr`.
    Store {
        width: Width,
        addr: Value,
        value: Value,
    },
    /// Writes a full 64-bit guest register.
    SetReg { reg: Gpr, value: Value },
    /// Records the status-flag inputs of an operation (lazy flags). `a`, `b`
    /// and `r` are zero-extended to `width`. `live` is false when no reader
    /// can observe this record (set by [`Program::analyze_flags`]).
    Flags {
        op: FlagOp,
        width: Width,
        a: Value,
        b: Value,
        r: Value,
        live: bool,
    },
}

/// x86 condition codes, in encoding order (the low nibble of `Jcc`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    O = 0,
    No = 1,
    B = 2,
    Ae = 3,
    E = 4,
    Ne = 5,
    Be = 6,
    A = 7,
    S = 8,
    Ns = 9,
    P = 10,
    Np = 11,
    L = 12,
    Ge = 13,
    Le = 14,
    G = 15,
}

impl Cond {
    pub const ALL: [Cond; 16] = [
        Cond::O,
        Cond::No,
        Cond::B,
        Cond::Ae,
        Cond::E,
        Cond::Ne,
        Cond::Be,
        Cond::A,
        Cond::S,
        Cond::Ns,
        Cond::P,
        Cond::Np,
        Cond::L,
        Cond::Ge,
        Cond::Le,
        Cond::G,
    ];

    pub fn from_index(i: u32) -> Cond {
        Cond::ALL[(i & 15) as usize]
    }

    /// The opposite condition.
    pub fn negate(self) -> Cond {
        Cond::from_index(self as u32 ^ 1)
    }

    /// True for the conditions that are the negation of their even partner.
    pub fn is_negated(self) -> bool {
        self as u32 & 1 == 1
    }

    pub fn name(self) -> &'static str {
        [
            "o", "no", "b", "ae", "e", "ne", "be", "a", "s", "ns", "p", "np", "l", "ge", "le", "g",
        ][self as usize]
    }

    /// Evaluates the condition on materialized RFLAGS bits.
    pub fn eval(self, rflags: u64) -> bool {
        use abi::flags::*;
        let f = |bit: u64| rflags & bit != 0;
        let base = match self as u32 & !1 {
            0 => f(OF),
            2 => f(CF),
            4 => f(ZF),
            6 => f(CF) || f(ZF),
            8 => f(SF),
            10 => f(PF),
            12 => f(SF) != f(OF),
            _ => f(ZF) || f(SF) != f(OF),
        };
        base != self.is_negated()
    }
}

/// The target of a [`Term::Call`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallTarget {
    /// `call rel32`.
    Direct(u64),
    /// `call r/m64`: the target, read before the return address was pushed.
    Indirect(Value),
}

/// How a block ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    /// Continue at a guest address (a block, or an exit if it has none).
    Jump(u64),
    /// Continue at `taken` if `cond` holds, else at `fallthrough`.
    Branch {
        cond: Cond,
        taken: u64,
        fallthrough: u64,
    },
    /// Continue at a computed guest address (`jmp r/m64`).
    Indirect(Value),
    /// A call. The ops have pushed `ret`, the address of the next
    /// instruction, where control continues when the callee returns.
    Call { target: CallTarget, ret: u64 },
    /// A return to the popped address `value`; the ops have adjusted RSP.
    Ret(Value),
    /// Exit to the host with `reason` and RIP = `rip`.
    Exit { reason: u32, rip: u64 },
}

/// One guest instruction and its lowering.
#[derive(Clone, Debug)]
pub struct Insn {
    pub addr: u64,
    pub len: u32,
    /// Disassembly (Intel syntax), for dumps.
    pub text: String,
    pub ops: Vec<Op>,
    /// Number of temporaries the ops use (`t0 .. t{temps-1}`).
    pub temps: u32,
}

/// A basic block.
#[derive(Clone, Debug)]
pub struct Block {
    pub addr: u64,
    pub insns: Vec<Insn>,
    pub term: Term,
    /// For a block that ends in an unsupported instruction which would
    /// continue to the next one: that next address, a block of the same
    /// function (discovery continues past the exit; execution doesn't).
    pub after_exit: Option<u64>,
}

/// A recovered jump table (`docs/design/translator.md`, "Linking and
/// images").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JumpTable {
    /// Address of the indirect `jmp` that dispatches through it.
    pub jmp: u64,
    /// Address of the table.
    pub table: u64,
    /// Its targets in table order (duplicates included).
    pub targets: Vec<u64>,
}

/// All translated blocks, sorted by address.
#[derive(Clone, Debug, Default)]
pub struct Program {
    pub blocks: Vec<Block>,
    /// Jump tables recovered while decoding, by `jmp` address.
    pub jump_tables: Vec<JumpTable>,
    /// Indirect `jmp`s seen: those that end blocks, and those after an
    /// unsupported instruction in a block (found by decoding on).
    pub indirect_jumps: usize,
}

impl Program {
    /// Index of the block starting at `addr`.
    pub fn index_map(&self) -> HashMap<u64, usize> {
        self.blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.addr, i))
            .collect()
    }

    /// Number of guest instructions.
    pub fn insn_count(&self) -> usize {
        self.blocks.iter().map(|b| b.insns.len()).sum()
    }

    /// Flag liveness: marks every [`Op::Flags`] record whose flags no reader
    /// (a conditional branch, a call or return, or any exit to the host or
    /// unknown code) can observe before the next record overwrites them.
    ///
    /// Direct jumps are followed across function and chunk boundaries: every
    /// boundary stores the whole lazy record to the state block, so a record
    /// no path reads is dead wherever the path continues.
    pub fn analyze_flags(&mut self) {
        let index = self.index_map();
        let mut live_in = vec![false; self.blocks.len()];
        let live_after_term = |term: &Term, live_in: &[bool]| match term {
            Term::Jump(target) => index.get(target).is_none_or(|&i| live_in[i]),
            Term::Branch { .. }
            | Term::Indirect(_)
            | Term::Call { .. }
            | Term::Ret(_)
            | Term::Exit { .. } => true,
        };
        let block_live_in = |block: &Block, live_out: bool| {
            let mut live = live_out;
            for op in block.insns.iter().rev().flat_map(|i| i.ops.iter().rev()) {
                if let Op::Flags { .. } = op {
                    live = false;
                }
            }
            live
        };
        let mut changed = true;
        while changed {
            changed = false;
            for i in (0..self.blocks.len()).rev() {
                let out = live_after_term(&self.blocks[i].term, &live_in);
                let new = block_live_in(&self.blocks[i], out);
                if new != live_in[i] {
                    live_in[i] = new;
                    changed = true;
                }
            }
        }
        for i in 0..self.blocks.len() {
            let mut live = live_after_term(&self.blocks[i].term, &live_in);
            for op in self.blocks[i]
                .insns
                .iter_mut()
                .rev()
                .flat_map(|insn| insn.ops.iter_mut().rev())
            {
                if let Op::Flags { live: l, .. } = op {
                    *l = live;
                    live = false;
                }
            }
        }
    }

    /// Human-readable dump.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        for (i, block) in self.blocks.iter().enumerate() {
            let _ = write!(out, "{block}");
            if i + 1 < self.blocks.len() {
                out.push('\n');
            }
        }
        out
    }
}

pub fn reg_name(reg: Gpr) -> &'static str {
    abi::GPR_NAMES[reg as usize]
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Reg(r) => f.write_str(reg_name(*r)),
            Value::Temp(t) => write!(f, "t{}", t.0),
            Value::Imm(v) => write!(f, "{v:#x}"),
        }
    }
}

impl fmt::Display for Width {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.bits())
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Op::Mov { dst, src } => write!(f, "t{} = {src}", dst.0),
            Op::Bin { dst, op, a, b } => {
                let name = match op {
                    BinOp::Add => "add",
                    BinOp::Sub => "sub",
                    BinOp::And => "and",
                    BinOp::Or => "or",
                    BinOp::Xor => "xor",
                    BinOp::Shl => "shl",
                    BinOp::ShrU => "shr_u",
                };
                write!(f, "t{} = {name} {a}, {b}", dst.0)
            }
            Op::Load { dst, width, addr } => write!(f, "t{} = load{width} [{addr}]", dst.0),
            Op::Store { width, addr, value } => write!(f, "store{width} [{addr}], {value}"),
            Op::SetReg { reg, value } => write!(f, "{} = {value}", reg_name(*reg)),
            Op::Flags {
                op,
                width,
                a,
                b,
                r,
                live,
            } => {
                let name = match op {
                    FlagOp::Add => "add",
                    FlagOp::Sub => "sub",
                };
                write!(f, "flags.{name}{width} {a}, {b}, {r}")?;
                if !live {
                    f.write_str("  ; dead")?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for Term {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Term::Jump(t) => write!(f, "jump {t:#x}"),
            Term::Branch {
                cond,
                taken,
                fallthrough,
            } => write!(
                f,
                "if {} jump {taken:#x} else {fallthrough:#x}",
                cond.name()
            ),
            Term::Indirect(v) => write!(f, "jump [{v}]"),
            Term::Call { target, ret } => match target {
                CallTarget::Direct(t) => write!(f, "call {t:#x} ret {ret:#x}"),
                CallTarget::Indirect(v) => write!(f, "call [{v}] ret {ret:#x}"),
            },
            Term::Ret(v) => write!(f, "ret [{v}]"),
            Term::Exit { reason, rip } => {
                let name = match *reason {
                    abi::EXIT_LEFT_CODE => "left_code",
                    abi::EXIT_UNSUPPORTED => "unsupported",
                    _ => "?",
                };
                write!(f, "exit {name} rip={rip:#x}")
            }
        }
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "block {:#x}:", self.addr)?;
        for insn in &self.insns {
            writeln!(f, "  {:#x}: {}", insn.addr, insn.text)?;
            for op in &insn.ops {
                writeln!(f, "    {op}")?;
            }
        }
        writeln!(f, "  -> {}", self.term)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cond_negation_pairs() {
        for c in Cond::ALL {
            assert_eq!(c.negate().negate(), c);
            for flags in 0..4096u64 {
                assert_ne!(c.eval(flags), c.negate().eval(flags));
            }
        }
    }

    #[test]
    fn width_masks() {
        assert_eq!(Width::W8.mask(), 0xff);
        assert_eq!(Width::W32.mask(), 0xffff_ffff);
        assert_eq!(Width::W64.mask(), u64::MAX);
        assert_eq!(Width::W16.sign_bit(), 0x8000);
    }
}
