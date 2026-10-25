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

/// Indexes of the registers that instructions use implicitly.
pub const RAX: Gpr = 0;
pub const RCX: Gpr = 1;
pub const RDX: Gpr = 2;
/// Index of RSP.
pub const RSP: Gpr = 4;
pub const RSI: Gpr = 6;
pub const RDI: Gpr = 7;

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

/// 64-bit binary operations (wrapping). Shift and rotate counts are taken
/// mod 64, as in wasm; the comparisons give 0 or 1. The divisions are only
/// emitted after a [`Op::FaultIf`] that excludes a zero divisor (and, for
/// `DivS`, `i64::MIN / -1`), so they never trap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Shl,
    ShrU,
    ShrS,
    Rotl,
    Rotr,
    Mul,
    DivU,
    DivS,
    RemU,
    RemS,
    Eq,
    Ne,
    LtU,
    LtS,
}

impl BinOp {
    pub fn name(self) -> &'static str {
        match self {
            BinOp::Add => "add",
            BinOp::Sub => "sub",
            BinOp::And => "and",
            BinOp::Or => "or",
            BinOp::Xor => "xor",
            BinOp::Shl => "shl",
            BinOp::ShrU => "shr_u",
            BinOp::ShrS => "shr_s",
            BinOp::Rotl => "rotl",
            BinOp::Rotr => "rotr",
            BinOp::Mul => "mul",
            BinOp::DivU => "div_u",
            BinOp::DivS => "div_s",
            BinOp::RemU => "rem_u",
            BinOp::RemS => "rem_s",
            BinOp::Eq => "eq",
            BinOp::Ne => "ne",
            BinOp::LtU => "lt_u",
            BinOp::LtS => "lt_s",
        }
    }

    /// The value of `a op b`, or `None` where it would trap (division by
    /// zero, `i64::MIN / -1`).
    pub fn eval(self, a: u64, b: u64) -> Option<u64> {
        Some(match self {
            BinOp::Add => a.wrapping_add(b),
            BinOp::Sub => a.wrapping_sub(b),
            BinOp::And => a & b,
            BinOp::Or => a | b,
            BinOp::Xor => a ^ b,
            BinOp::Shl => a.wrapping_shl(b as u32),
            BinOp::ShrU => a.wrapping_shr(b as u32),
            BinOp::ShrS => (a as i64).wrapping_shr(b as u32) as u64,
            BinOp::Rotl => a.rotate_left((b & 63) as u32),
            BinOp::Rotr => a.rotate_right((b & 63) as u32),
            BinOp::Mul => a.wrapping_mul(b),
            BinOp::DivU => a.checked_div(b)?,
            BinOp::DivS => (a as i64).checked_div(b as i64)? as u64,
            BinOp::RemU => a.checked_rem(b)?,
            BinOp::RemS => {
                if b == 0 {
                    return None;
                }
                (a as i64).wrapping_rem(b as i64) as u64
            }
            BinOp::Eq => (a == b) as u64,
            BinOp::Ne => (a != b) as u64,
            BinOp::LtU => (a < b) as u64,
            BinOp::LtS => ((a as i64) < (b as i64)) as u64,
        })
    }
}

/// 64-bit unary operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Clz,
    Ctz,
    /// Sign-extends the low 8, 16 or 32 bits.
    Sext8,
    Sext16,
    Sext32,
}

impl UnOp {
    pub fn name(self) -> &'static str {
        match self {
            UnOp::Clz => "clz",
            UnOp::Ctz => "ctz",
            UnOp::Sext8 => "sext8",
            UnOp::Sext16 => "sext16",
            UnOp::Sext32 => "sext32",
        }
    }

    pub fn eval(self, a: u64) -> u64 {
        match self {
            UnOp::Clz => a.leading_zeros() as u64,
            UnOp::Ctz => a.trailing_zeros() as u64,
            UnOp::Sext8 => a as i8 as i64 as u64,
            UnOp::Sext16 => a as i16 as i64 as u64,
            UnOp::Sext32 => a as i32 as i64 as u64,
        }
    }

    /// The sign extension from `width` (`None` for 64 bits).
    pub fn sext(width: Width) -> Option<UnOp> {
        match width {
            Width::W8 => Some(UnOp::Sext8),
            Width::W16 => Some(UnOp::Sext16),
            Width::W32 => Some(UnOp::Sext32),
            Width::W64 => None,
        }
    }
}

/// The operation a lazy flag record describes (`docs/design/translator.md`,
/// "Lazy flag record"). `a`, `b` and `r` are zero-extended to the width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagOp {
    /// `r = a + b`
    Add,
    /// `r = a - b` (also CMP, NEG with `a = 0`)
    Sub,
    /// AND, OR, XOR, TEST (and BSF/BSR's ZF): `a = r`, `b = 0`.
    Logic,
    /// `r = a + b + carry`
    Adc,
    /// `r = a - b - carry`
    Sbb,
    /// `r = a + 1`; `b` is the CF from before, which the emitter fills in.
    Inc,
    /// `r = a - 1`; `b` as for `Inc`.
    Dec,
    /// `r = a << b`, `b` the masked count (≥ 1). Also SHLD (`a` = dest).
    Shl,
    /// `r = a >> b`. Also SHRD.
    Shr,
    /// `r = a >>s b`.
    Sar,
    /// MUL, IMUL: `a` = overflow (0 or 1), `b = 0`, `r` = low half.
    Mul,
}

impl FlagOp {
    pub const ALL: [FlagOp; 11] = [
        FlagOp::Add,
        FlagOp::Sub,
        FlagOp::Logic,
        FlagOp::Adc,
        FlagOp::Sbb,
        FlagOp::Inc,
        FlagOp::Dec,
        FlagOp::Shl,
        FlagOp::Shr,
        FlagOp::Sar,
        FlagOp::Mul,
    ];

    /// The kind number in the `cc_op` encoding.
    pub const fn kind(self) -> u32 {
        match self {
            FlagOp::Add => 0,
            FlagOp::Sub => 1,
            FlagOp::Logic => 2,
            FlagOp::Adc => 3,
            FlagOp::Sbb => 4,
            FlagOp::Inc => 5,
            FlagOp::Dec => 6,
            FlagOp::Shl => 7,
            FlagOp::Shr => 8,
            FlagOp::Sar => 9,
            FlagOp::Mul => 10,
        }
    }

    /// The runtime encoding of a record kind (the `cc_op` local): 0 means the
    /// flags are held materialized in `cc_a`; `1 + 4*kind + width` otherwise.
    pub const fn cc_op(self, width: Width) -> u32 {
        1 + 4 * self.kind() + width.index()
    }

    /// Inc and Dec keep the previous record's CF.
    pub const fn is_partial(self) -> bool {
        matches!(self, FlagOp::Inc | FlagOp::Dec)
    }

    pub fn name(self) -> &'static str {
        match self {
            FlagOp::Add => "add",
            FlagOp::Sub => "sub",
            FlagOp::Logic => "logic",
            FlagOp::Adc => "adc",
            FlagOp::Sbb => "sbb",
            FlagOp::Inc => "inc",
            FlagOp::Dec => "dec",
            FlagOp::Shl => "shl",
            FlagOp::Shr => "shr",
            FlagOp::Sar => "sar",
            FlagOp::Mul => "mul",
        }
    }

    /// Reference model: the status flags of a record, as the `flags`
    /// function computes them.
    pub fn eval(self, width: Width, a: u64, b: u64, r: u64) -> u64 {
        use abi::flags::*;
        let sign = width.sign_bit();
        let msb = |x: u64| x & sign != 0;
        let (cf, of) = match self {
            FlagOp::Add => (r < a, msb((a ^ r) & (b ^ r))),
            FlagOp::Sub => (a < b, msb((a ^ b) & (a ^ r))),
            FlagOp::Logic => (false, false),
            FlagOp::Adc => (msb((a & b) | ((a ^ b) & !r)), msb((a ^ r) & (b ^ r))),
            FlagOp::Sbb => (msb((!a & b) | (!(a ^ b) & r)), msb((a ^ b) & (a ^ r))),
            FlagOp::Inc => (b & 1 != 0, r == sign),
            FlagOp::Dec => (b & 1 != 0, a == sign),
            FlagOp::Shl => (msb(a.wrapping_shl(b.wrapping_sub(1) as u32)), msb(a ^ r)),
            FlagOp::Shr => (
                a.wrapping_shr(b.wrapping_sub(1) as u32) & 1 != 0,
                msb(a ^ r),
            ),
            FlagOp::Sar => {
                let s = (a ^ sign).wrapping_sub(sign) as i64;
                (
                    s.wrapping_shr(b.wrapping_sub(1) as u32) & 1 != 0,
                    msb(a ^ r),
                )
            }
            FlagOp::Mul => (a != 0, a != 0),
        };
        let mut f = (a ^ b ^ r) & AF;
        if cf {
            f |= CF;
        }
        if of {
            f |= OF;
        }
        if (r & 0xff).count_ones().is_multiple_of(2) {
            f |= PF;
        }
        if r == 0 {
            f |= ZF;
        }
        if msb(r) {
            f |= SF;
        }
        f
    }
}

/// Runtime `cc_op` value meaning "flags are materialized in `cc_a`".
pub const CC_EAGER: u32 = 0;

/// An instruction-local v128 temporary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VTemp(pub u32);

/// Where a vector access goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VLoc {
    /// The state block at this offset (an XMM register's slot, or a part of it).
    State(u32),
    /// Guest memory.
    Mem(Value),
}

/// Width of a vector load or store: the narrow ones touch the low lane only
/// (loads zero the rest).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VWidth {
    W32,
    W64,
    W128,
}

/// Lane shape of a packed integer operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lanes {
    B,
    W,
    D,
    Q,
}

impl Lanes {
    pub const fn bits(self) -> u32 {
        match self {
            Lanes::B => 8,
            Lanes::W => 16,
            Lanes::D => 32,
            Lanes::Q => 64,
        }
    }
}

/// Binary vector operations with x86 semantics (`a` is the destination
/// operand, `b` the source).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VBinOp {
    And,
    Or,
    Xor,
    /// `!a & b` (PANDN).
    AndNot,
    Add(Lanes),
    Sub(Lanes),
    AddSatS(Lanes),
    AddSatU(Lanes),
    SubSatS(Lanes),
    SubSatU(Lanes),
    Eq(Lanes),
    GtS(Lanes),
    MinU(Lanes),
    MaxU(Lanes),
    MinS(Lanes),
    MaxS(Lanes),
    AvgU(Lanes),
    /// PMULLW
    MulLo16,
    /// PMULHW
    MulHiS16,
    /// PMULHUW
    MulHiU16,
    /// PMULUDQ
    MulU32,
    /// PMADDWD
    MaddWd,
    /// PSADBW
    SadBw,
    /// PACKSSWB
    PackSsWb,
    /// PACKUSWB
    PackUsWb,
    /// PACKSSDW
    PackSsDw,
}

/// Packed shifts with x86 counts: an unsigned 64-bit count; logical shifts
/// by at least the lane width give 0, arithmetic ones fill with the sign.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VShiftOp {
    Shl(Lanes),
    ShrU(Lanes),
    ShrS(Lanes),
}

/// A scalar taken from a vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VExtractOp {
    /// 64-bit lane 0 or 1.
    Lane64(u8),
    /// The sign bits of the 8-, 32- or 64-bit lanes.
    Bitmask(Lanes),
}

/// String instructions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrOp {
    Movs,
    Stos,
    Lods,
}

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
    /// `dst = op a`
    Un { dst: Temp, op: UnOp, a: Value },
    /// `dst = cond != 0 ? a : b`
    Select {
        dst: Temp,
        cond: Value,
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
    /// `dst` = zero-extended load of `width` from the state block at `offset`.
    LoadState {
        dst: Temp,
        width: Width,
        offset: u32,
    },
    /// Stores the low `width` bits of `value` to the state block at `offset`.
    StoreState {
        width: Width,
        offset: u32,
        value: Value,
    },
    /// `dst` = 1 if `cond` holds for the current flag record, else 0. A
    /// flag reader.
    Cond { dst: Temp, cond: Cond },
    /// Records the status-flag inputs of an operation (lazy flags). `a`, `b`
    /// and `r` are zero-extended to `width`. With `when`, the record is
    /// written only if `when != 0`. `live` is false when no reader can
    /// observe this record (set by [`Program::analyze_flags`]).
    Flags {
        op: FlagOp,
        width: Width,
        a: Value,
        b: Value,
        r: Value,
        when: Option<Value>,
        live: bool,
    },
    /// Writes an eager record from the previous one, materialized as F:
    /// `((F & !clear) | set) ^ flip`. With `when`, only if `when != 0`.
    FlagsUpdate {
        clear: u64,
        set: Value,
        flip: u64,
        when: Option<Value>,
        live: bool,
    },
    /// If `cond != 0`, the instruction faults with `kind` (EXIT_FAULT, RIP
    /// at the instruction, the state from before it).
    FaultIf { cond: Value, kind: u32 },
    /// 128-by-64-bit division of `hi:lo` by `d` through the `div128`
    /// helper: quotient, remainder, and 1 in `fault` if `d` is 0 or the
    /// quotient doesn't fit in 64 bits (signed or unsigned).
    DivWide {
        q: Temp,
        r: Temp,
        fault: Temp,
        hi: Value,
        lo: Value,
        d: Value,
        signed: bool,
    },
    /// A string instruction: one iteration, or with `rep` as many as RCX
    /// says. `delta` is +size or -size (from DF).
    RepString {
        op: StrOp,
        width: Width,
        rep: bool,
        delta: Value,
    },
    /// `dst` = a vector load (narrow loads zero the rest).
    VLoad {
        dst: VTemp,
        width: VWidth,
        loc: VLoc,
    },
    /// Stores `src` (its low lane for the narrow widths).
    VStore {
        loc: VLoc,
        width: VWidth,
        src: VTemp,
    },
    /// `dst = a op b`
    VBin {
        dst: VTemp,
        op: VBinOp,
        a: VTemp,
        b: VTemp,
    },
    /// `dst` = bytes of `a` (lanes 0-15) and `b` (16-31) picked by `lanes`.
    VShuffle {
        dst: VTemp,
        a: VTemp,
        b: VTemp,
        lanes: [u8; 16],
    },
    /// `dst = a` shifted by the unsigned 64-bit `count`.
    VShift {
        dst: VTemp,
        op: VShiftOp,
        a: VTemp,
        count: Value,
    },
    /// `dst` = a constant.
    VConst { dst: VTemp, value: u128 },
    /// `dst` = `value` in the low 64 bits, the rest 0.
    VFromGpr { dst: VTemp, value: Value },
    /// `dst` = a scalar of `src`.
    VExtract {
        dst: Temp,
        op: VExtractOp,
        src: VTemp,
    },
}

impl Op {
    /// A record that may keep flags of the previous one.
    pub fn is_partial_flags(&self) -> bool {
        match self {
            Op::Flags { op, when, .. } => op.is_partial() || when.is_some(),
            Op::FlagsUpdate { .. } => true,
            _ => false,
        }
    }

    /// Writes a flag record.
    pub fn writes_flags(&self) -> bool {
        matches!(self, Op::Flags { .. } | Op::FlagsUpdate { .. })
    }

    /// Has an architectural effect (anything a fault must not see).
    pub fn has_side_effect(&self) -> bool {
        matches!(
            self,
            Op::Store { .. }
                | Op::SetReg { .. }
                | Op::StoreState { .. }
                | Op::Flags { .. }
                | Op::FlagsUpdate { .. }
                | Op::RepString { .. }
                | Op::VStore { .. }
        )
    }
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
    /// Continue at `taken` if `value != 0`, else at `fallthrough` (jrcxz,
    /// loop). Reads no flags.
    BranchIf {
        value: Value,
        taken: u64,
        fallthrough: u64,
    },
    /// Exit to the host with `reason` and RIP = `rip`.
    Exit { reason: u32, rip: u64 },
    /// A fault at the instruction at `rip` (EXIT_FAULT, fault address 0).
    Fault { kind: u32, rip: u64 },
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
    /// Number of v128 temporaries (`v0 .. v{vtemps-1}`).
    pub vtemps: u32,
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

    /// Flag liveness: marks every [`Op::Flags`] and [`Op::FlagsUpdate`]
    /// record whose flags no reader (an [`Op::Cond`], a conditional branch,
    /// a call or return, or any exit to the host or unknown code) can observe
    /// before the next full record overwrites them. Partial and conditional
    /// records read or may keep the previous record, so they don't end it.
    ///
    /// Direct jumps are followed across function and chunk boundaries: every
    /// boundary stores the whole lazy record to the state block, so a record
    /// no path reads is dead wherever the path continues.
    pub fn analyze_flags(&mut self) {
        let index = self.index_map();
        let mut live_in = vec![false; self.blocks.len()];
        let live_at =
            |target: &u64, live_in: &[bool]| index.get(target).is_none_or(|&i| live_in[i]);
        let live_after_term = |term: &Term, live_in: &[bool]| match term {
            Term::Jump(target) => live_at(target, live_in),
            Term::BranchIf {
                taken, fallthrough, ..
            } => live_at(taken, live_in) || live_at(fallthrough, live_in),
            Term::Branch { .. }
            | Term::Indirect(_)
            | Term::Call { .. }
            | Term::Ret(_)
            | Term::Exit { .. }
            | Term::Fault { .. } => true,
        };
        // Backward transfer of one op: readers make the record live, full
        // records kill it, partial and conditional ones pass it through.
        let step = |op: &Op, live: bool| match op {
            Op::Cond { .. } => true,
            op if op.writes_flags() && !op.is_partial_flags() => false,
            _ => live,
        };
        let block_live_in = |block: &Block, live_out: bool| {
            let mut live = live_out;
            for op in block.insns.iter().rev().flat_map(|i| i.ops.iter().rev()) {
                live = step(op, live);
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
                let next = step(op, live);
                if let Op::Flags { live: l, .. } | Op::FlagsUpdate { live: l, .. } = op {
                    *l = live;
                }
                live = next;
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

impl fmt::Display for VTemp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

impl fmt::Display for VLoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VLoc::State(o) => write!(f, "state[{o:#x}]"),
            VLoc::Mem(a) => write!(f, "[{a}]"),
        }
    }
}

impl fmt::Display for VWidth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            VWidth::W32 => "32",
            VWidth::W64 => "64",
            VWidth::W128 => "128",
        })
    }
}

fn when_suffix(f: &mut fmt::Formatter<'_>, when: &Option<Value>, live: bool) -> fmt::Result {
    if let Some(w) = when {
        write!(f, " if {w}")?;
    }
    if !live {
        f.write_str("  ; dead")?;
    }
    Ok(())
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Op::Mov { dst, src } => write!(f, "t{} = {src}", dst.0),
            Op::Bin { dst, op, a, b } => write!(f, "t{} = {} {a}, {b}", dst.0, op.name()),
            Op::Un { dst, op, a } => write!(f, "t{} = {} {a}", dst.0, op.name()),
            Op::Select { dst, cond, a, b } => {
                write!(f, "t{} = select {cond}, {a}, {b}", dst.0)
            }
            Op::Load { dst, width, addr } => write!(f, "t{} = load{width} [{addr}]", dst.0),
            Op::Store { width, addr, value } => write!(f, "store{width} [{addr}], {value}"),
            Op::SetReg { reg, value } => write!(f, "{} = {value}", reg_name(*reg)),
            Op::LoadState { dst, width, offset } => {
                write!(f, "t{} = load{width} state[{offset:#x}]", dst.0)
            }
            Op::StoreState {
                width,
                offset,
                value,
            } => write!(f, "store{width} state[{offset:#x}], {value}"),
            Op::Cond { dst, cond } => write!(f, "t{} = cond.{}", dst.0, cond.name()),
            Op::Flags {
                op,
                width,
                a,
                b,
                r,
                when,
                live,
            } => {
                write!(f, "flags.{}{width} {a}, {b}, {r}", op.name())?;
                when_suffix(f, when, *live)
            }
            Op::FlagsUpdate {
                clear,
                set,
                flip,
                when,
                live,
            } => {
                write!(
                    f,
                    "flags.update clear {clear:#x}, set {set}, flip {flip:#x}"
                )?;
                when_suffix(f, when, *live)
            }
            Op::FaultIf { cond, kind } => write!(f, "fault {kind} if {cond}"),
            Op::DivWide {
                q,
                r,
                fault,
                hi,
                lo,
                d,
                signed,
            } => write!(
                f,
                "t{}, t{}, t{} = div128{} {hi}:{lo}, {d}",
                q.0,
                r.0,
                fault.0,
                if *signed { "_s" } else { "_u" }
            ),
            Op::RepString {
                op,
                width,
                rep,
                delta,
            } => write!(
                f,
                "{}{:?}{width} delta {delta}",
                if *rep { "rep " } else { "" },
                op
            ),
            Op::VLoad { dst, width, loc } => write!(f, "{dst} = vload{width} {loc}"),
            Op::VStore { loc, width, src } => write!(f, "vstore{width} {loc}, {src}"),
            Op::VBin { dst, op, a, b } => write!(f, "{dst} = {op:?} {a}, {b}"),
            Op::VShuffle { dst, a, b, lanes } => {
                write!(f, "{dst} = shuffle {a}, {b}, {lanes:?}")
            }
            Op::VShift { dst, op, a, count } => write!(f, "{dst} = {op:?} {a}, {count}"),
            Op::VConst { dst, value } => write!(f, "{dst} = {value:#x}"),
            Op::VFromGpr { dst, value } => write!(f, "{dst} = from64 {value}"),
            Op::VExtract { dst, op, src } => write!(f, "t{} = {op:?} {src}", dst.0),
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
            Term::BranchIf {
                value,
                taken,
                fallthrough,
            } => write!(f, "if {value} jump {taken:#x} else {fallthrough:#x}"),
            Term::Fault { kind, rip } => write!(f, "fault {kind} rip={rip:#x}"),
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
