//! Jump-table recovery for GCC's relative switch tables
//! (`docs/design/translator.md`, "Linking and images"), on raw decoded
//! instructions:
//!
//! ```text
//!   cmp   idx, imm            ; the bound, in the predecessor block
//!   ja    default             ; (or jae; or jbe/jb to the table block)
//! table_block:
//!   [mov/movzx idx32, src]    ; zero extension, before or after the lea
//!   lea   base, [rip + table]
//!   movsxd dst, dword [base + idx*4]
//!   add   dst, base           ; (or add base, dst; jmp base)
//!   jmp   dst
//! ```
//!
//! The bound must check the index (or the register or memory operand the
//! zero extension reads) with an unsigned condition; the table is read from
//! the image, and every target must lie in executable code.

use iced_x86::{
    Code, ConditionCode, FlowControl, Instruction, InstructionInfoFactory, InstructionInfoOptions,
    Mnemonic, OpAccess, OpKind, Register,
};

/// Largest table accepted.
pub(crate) const MAX_ENTRIES: u64 = 4096;
/// Instructions of a table block, up to and including the jump.
pub(crate) const MAX_BLOCK_INSNS: usize = 12;

/// A compared or loaded operand: a full register, or a memory operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operand {
    Reg(Register),
    Mem {
        base: Register,
        index: Register,
        scale: u32,
        displ: u64,
        segment: Register,
        size: usize,
    },
}

fn operand(insn: &Instruction, i: u32) -> Option<Operand> {
    match insn.op_kind(i) {
        OpKind::Register => {
            let r = insn.op_register(i);
            r.is_gpr().then(|| Operand::Reg(r.full_register()))
        }
        OpKind::Memory => Some(Operand::Mem {
            base: insn.memory_base(),
            index: insn.memory_index(),
            scale: insn.memory_index_scale(),
            displ: insn.memory_displacement64(),
            segment: insn.segment_prefix(),
            size: insn.memory_size().size(),
        }),
        _ => None,
    }
}

/// A bound on the index of a table block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bound {
    pub operand: Operand,
    /// Number of valid indices: 0 .. count - 1.
    pub count: u64,
}

/// `cmp op, imm` directly followed by an unsigned conditional branch: the
/// address of the block the branch admits only in-range indices to, and the
/// bounds. `before` is the instruction just before the `cmp`: a register
/// copy of the compared register (`mov r8d, edx; cmp edx, 9`) is bounded
/// too.
pub(crate) fn bound(
    before: Option<&Instruction>,
    cmp: &Instruction,
    jcc: &Instruction,
) -> Option<(u64, Vec<Bound>)> {
    if cmp.mnemonic() != Mnemonic::Cmp
        || cmp.op_count() != 2
        || jcc.ip() != cmp.next_ip()
        || jcc.flow_control() != FlowControl::ConditionalBranch
    {
        return None;
    }
    let bits = match cmp.op0_kind() {
        OpKind::Register => cmp.op0_register().size() * 8,
        OpKind::Memory => cmp.memory_size().size() * 8,
        _ => return None,
    };
    let imm = match cmp.op1_kind() {
        OpKind::Immediate8
        | OpKind::Immediate16
        | OpKind::Immediate32
        | OpKind::Immediate8to16
        | OpKind::Immediate8to32
        | OpKind::Immediate8to64
        | OpKind::Immediate32to64 => cmp.immediate(1),
        _ => return None,
    };
    let imm = if bits >= 64 {
        imm
    } else {
        imm & ((1u64 << bits) - 1)
    };
    let operand = operand(cmp, 0)?;
    let taken = jcc.near_branch_target();
    let (block, count) = match jcc.condition_code() {
        ConditionCode::a => (jcc.next_ip(), imm.checked_add(1)?),
        ConditionCode::ae => (jcc.next_ip(), imm),
        ConditionCode::be => (taken, imm.checked_add(1)?),
        ConditionCode::b => (taken, imm),
        _ => return None,
    };
    if !(1..=MAX_ENTRIES).contains(&count) {
        return None;
    }
    let mut bounds = vec![Bound { operand, count }];
    if let Some(b) = before
        && b.next_ip() == cmp.ip()
        && matches!(
            b.code(),
            Code::Mov_r32_rm32 | Code::Mov_rm32_r32 | Code::Mov_r64_rm64 | Code::Mov_rm64_r64
        )
        && let (Some(dst), Some(src)) = (reg(b, 0), reg(b, 1))
        && Operand::Reg(src.full_register()) == operand
    {
        bounds.push(Bound {
            operand: Operand::Reg(dst.full_register()),
            count,
        });
    }
    Some((block, bounds))
}

/// A table block that matched the pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    /// Address of the `jmp`.
    pub jmp: u64,
    /// Address of the table (the `lea` target).
    pub table: u64,
    /// What the index was loaded from: the operand a bound must check.
    pub index_source: Operand,
}

/// `insn` may write (part of) the 64-bit register `full`.
fn writes(insn: &Instruction, full: Register) -> bool {
    let mut factory = InstructionInfoFactory::new();
    let info = factory.info_options(insn, InstructionInfoOptions::NO_MEMORY_USAGE);
    info.used_registers().iter().any(|u| {
        u.register().full_register() == full
            && !matches!(u.access(), OpAccess::Read | OpAccess::CondRead)
    })
}

fn reg(insn: &Instruction, i: u32) -> Option<Register> {
    (insn.op_kind(i) == OpKind::Register).then(|| insn.op_register(i))
}

/// Matches the instructions of a block, from its leader up to its first
/// control transfer.
pub(crate) fn match_block(insns: &[Instruction]) -> Option<Candidate> {
    let n = insns.len();
    if n < 4 {
        return None;
    }
    let (jmp, add, movsxd) = (&insns[n - 1], &insns[n - 2], &insns[n - 3]);
    if jmp.code() != Code::Jmp_rm64 {
        return None;
    }
    let target = reg(jmp, 0)?;
    if !matches!(add.code(), Code::Add_r64_rm64 | Code::Add_rm64_r64) {
        return None;
    }
    let (x, y) = (reg(add, 0)?, reg(add, 1)?);
    if movsxd.code() != Code::Movsxd_r64_rm32
        || movsxd.op1_kind() != OpKind::Memory
        || movsxd.memory_index_scale() != 4
        || movsxd.memory_displacement64() != 0
        || movsxd.segment_prefix() != Register::None
    {
        return None;
    }
    let dst = reg(movsxd, 0)?;
    let base = movsxd.memory_base();
    let index = movsxd.memory_index();
    if !base.is_gpr64() || !index.is_gpr64() || base == dst || base == index {
        return None;
    }
    let sums = (x == dst && y == base) || (x == base && y == dst);
    if x != target || !sums {
        return None;
    }
    let mut table = None;
    let mut index_source = Operand::Reg(index);
    for insn in &insns[..n - 3] {
        if insn.code() == Code::Lea_r64_m
            && reg(insn, 0) == Some(base)
            && insn.is_ip_rel_memory_operand()
            && table.is_none()
        {
            table = Some(insn.ip_rel_memory_address());
            continue;
        }
        // A zero extension of the index.
        let extends = matches!(
            insn.code(),
            Code::Mov_r32_rm32 | Code::Mov_rm32_r32 | Code::Movzx_r32_rm8 | Code::Movzx_r32_rm16
        );
        match reg(insn, 0) {
            Some(r) if extends && r.full_register() == index => {
                index_source = operand(insn, 1)?;
            }
            // Anything else must leave the base and the index alone.
            _ if insn.flow_control() == FlowControl::Next
                && !writes(insn, base)
                && !writes(insn, index) => {}
            _ => return None,
        }
    }
    Some(Candidate {
        jmp: jmp.ip(),
        table: table?,
        index_source,
    })
}
