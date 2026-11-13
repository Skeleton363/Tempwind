//! Forms that Wine's DLL closure adds to the scope (Phase 2, M1;
//! `instruction-scope.md`, "Wine closure census"): pushfq and popfq, the
//! stores of a segment selector, cmpxchg16b and enter. The x87 forms and
//! fxsave are in `x87.rs`, lahf and sahf are generic `flagop` forms.

use super::special::Special;
use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum Spec {
    Pushfq,
    Popfq,
    /// mov r/m16, Sreg (`Mov_r32m16_Sreg` with memory, `Mov_rm16_Sreg`
    /// with a 16-bit register).
    MovSreg {
        code: Code,
        mem: bool,
    },
    Cmpxchg16b,
    Enter,
}

const STACK_FLAGS_COUNT: usize = 48;
const SREG_COUNT: usize = 36;
const CMPXCHG16B_COUNT: usize = 40;
const ENTER_COUNT: usize = 48;

/// Indexes of the cmpxchg16b cases with a misaligned m128 (#GP).
const MISALIGNED: [usize; 2] = [21, 34];

/// RFLAGS.NT and RFLAGS.ID: popfq loads them at CPL 3, with no effect in
/// 64-bit user mode.
const NT: u64 = 1 << 14;
const ID: u64 = 1 << 21;

/// RFLAGS bits that popfq ignores at CPL 3 with IOPL 0: IF, IOPL, RF, VM,
/// VIF, VIP and the reserved bits 3, 5, 15 and 22-63 (bit 1 reads 1).
const POPF_IGNORED: u64 =
    flags::IF | 3 << 12 | 1 << 16 | 1 << 17 | 1 << 19 | 1 << 20 | 1 << 3 | 1 << 5 | 1 << 15;

pub(super) fn forms(out: &mut Vec<Form>) {
    let mut add = |id: &str, family: &'static str, count: usize, spec: Spec| {
        out.push(Form {
            id: id.to_string(),
            family,
            count,
            kind: Kind::Special(Special::Wine(spec)),
        });
    };
    add("Pushfq/reg", "pushf", STACK_FLAGS_COUNT, Spec::Pushfq);
    add(
        "Mov_r32m16_Sreg/mem",
        "mov_sreg",
        SREG_COUNT,
        Spec::MovSreg {
            code: Code::Mov_r32m16_Sreg,
            mem: true,
        },
    );
    add(
        "Mov_rm16_Sreg/reg",
        "mov_sreg",
        SREG_COUNT,
        Spec::MovSreg {
            code: Code::Mov_rm16_Sreg,
            mem: false,
        },
    );
    add(
        "Cmpxchg16b_m128/mem",
        "cmpxchg16b",
        CMPXCHG16B_COUNT,
        Spec::Cmpxchg16b,
    );
    add("Enterq_imm16_imm8/reg", "enter", ENTER_COUNT, Spec::Enter);
    add("Popfq/reg", "pushf", STACK_FLAGS_COUNT, Spec::Popfq);
}

/// Encodes, checks the decoding (and the effective address) and builds the
/// case; a fault leaves RIP at the instruction.
fn done(
    form: &Form,
    b: Builder,
    instr: Instruction,
    fault: Option<FaultKind>,
    verify_ea: Option<u64>,
) -> Result<Option<GenCase>> {
    let case = finish(Finish {
        form,
        b,
        instr,
        target: fault.map(|_| CODE_ADDR),
        verify_ea,
    })?;
    Ok(case.map(|mut c| {
        c.expect_fault = fault;
        c
    }))
}

const SREGS: [Register; 6] = [
    Register::ES,
    Register::CS,
    Register::SS,
    Register::DS,
    Register::FS,
    Register::GS,
];

pub(super) fn case(
    rng: &mut Rng,
    form: &Form,
    idx: usize,
    defaults: &[u64; 16],
    spec: Spec,
) -> Result<Option<GenCase>> {
    let mut b = Builder::new(rng, defaults);
    let finish_with = |b, instr, fault, verify_ea| done(form, b, instr, fault, verify_ea);
    match spec {
        Spec::Pushfq => {
            let rsp = setup_stack(rng, &mut b)?;
            let Some(mut case) = finish_with(b, Instruction::with(Code::Pushfq), None, None)?
            else {
                return Ok(None);
            };
            // The recorder single-steps: the CPU's TF is set while the
            // instruction runs, and pushfq stores it (the kernel hides it
            // only in the registers). TF is never set in a guest's RFLAGS.
            case.undefined.mem = vec![MemBlock {
                addr: rsp - 8 + 1,
                bytes: vec![(flags::TF >> 8) as u8],
            }];
            Ok(Some(case))
        }
        Spec::Popfq => {
            let rsp = setup_stack(rng, &mut b)?;
            // Status flags, DF, NT and ID from the edges first, then random
            // (compared: `flags_mask` has NT and ID; the CPU keeps them and
            // the recorder reads them back); the bits popfq ignores at CPL 3
            // vary in half the cases. TF and AC stay 0: translated code stops
            // at them (`tests/rflags_paths.rs`).
            let set = flags::STATUS | flags::DF | NT | ID;
            let base = match idx {
                0 => 0,
                1 => set,
                2 => flags::STATUS,
                3 => flags::DF,
                4 => NT | ID,
                _ => rng.next_u64() & set,
            };
            let ignored = if idx >= 5 && rng.chance(1, 2) {
                let high = if rng.chance(1, 4) {
                    rng.next_u64() & !((1 << 22) - 1)
                } else {
                    0
                };
                rng.next_u64() & POPF_IGNORED | high
            } else {
                b.rflags & POPF_IGNORED | flags::RESERVED1
            };
            let value = base | ignored;
            b.put(rsp, &value.to_le_bytes())?;
            let case = finish_with(b, Instruction::with(Code::Popfq), None, None)?;
            Ok(case.map(|mut c| {
                c.flags_mask |= NT | ID;
                c
            }))
        }
        Spec::MovSreg { code, mem } => {
            let sreg = SREGS[idx % SREGS.len()];
            let instr = if mem {
                let (m, ea) = gen_mem(rng, &mut b, 2, true)?;
                let instr = Instruction::with2(code, m, sreg)?;
                return finish_with(b, instr, None, Some(ea));
            } else {
                let reg = rng.below(16) as usize;
                if reg == RSP_INDEX {
                    setup_stack(rng, &mut b)?;
                }
                Instruction::with2(code, GPR16[reg], sreg)?
            };
            finish_with(b, instr, None, None)
        }
        Spec::Cmpxchg16b => {
            let lo = DATA_ADDR + OPERAND_LO + 32;
            let hi = DATA_ADDR + OPERAND_HI - 48;
            let mut at = (lo + rng.below(hi - lo)) & !15;
            let misaligned = MISALIGNED.contains(&idx);
            if misaligned {
                at += rng.pick(&[1, 4, 8, 12]);
            }
            let mode = ALL_MODES[idx % ALL_MODES.len()];
            let m = solve_addr(rng, &mut b, mode, at, true);
            b.define(rng, at - 16, 48)?;
            let old_lo = rand_val(rng, 64);
            let old_hi = rand_val(rng, 64);
            let mut bytes = old_lo.to_le_bytes().to_vec();
            bytes.extend(old_hi.to_le_bytes());
            b.put(at, &bytes)?;
            // RDX:RAX: equal, equal in one half only, or different. The
            // address registers are already set; don't change them.
            let (mut want_lo, mut want_hi) = match idx % 4 {
                0 | 2 => (old_lo, old_hi),
                1 => (old_lo, rand_val(rng, 64)),
                _ => (rand_val(rng, 64), old_hi),
            };
            if idx % 8 == 7 {
                (want_lo, want_hi) = (rand_val(rng, 64), rand_val(rng, 64));
            }
            for (r, v) in [
                (0, want_lo),
                (2, want_hi),
                (1, rand_val(rng, 64)),
                (3, rand_val(rng, 64)),
            ] {
                if !b.touched[r] {
                    b.set_full(r, v);
                }
            }
            let mut instr = Instruction::with1(Code::Cmpxchg16b_m128, m)?;
            if rng.chance(1, 2) {
                instr.set_has_lock_prefix(true);
            }
            let fault = misaligned.then_some(FaultKind::Gp);
            finish_with(b, instr, fault, (!misaligned).then_some(at))
        }
        Spec::Enter => {
            // Nesting levels 0-3 and 31 first, then random; the CPU uses
            // the level mod 32.
            let level: u8 = match idx {
                0..=3 => idx as u8,
                4 => 31,
                5 => 32,
                6 => 0xff,
                _ if rng.chance(1, 2) => rng.below(5) as u8,
                _ => rng.below(256) as u8,
            };
            // The CPU checks a write at the final RSP (SDM, ENTER, #PF), so
            // it stays in the data window.
            let size: u16 = match idx % 8 {
                0 => 0,
                1 => 0xe000,
                2 => 8,
                _ => rng.below(0xe000) as u16,
            };
            let n = u64::from(level % 32);
            let rsp = DATA_ADDR + 0xf000 + (rng.below(0xc00) & !7);
            b.set_full(RSP_INDEX, rsp);
            b.define(rng, rsp - 8 * (n + 1) - 8, 8 * (n + 1) + 24)?;
            // The frames the copies come from: elsewhere, or overlapping
            // the new frame (each copy reads before its push writes).
            let rbp = if idx % 6 == 5 {
                rsp + 8 * rng.below(4)
            } else {
                DATA_ADDR + 0x2000 + (rng.below(0x1000) & !7)
            };
            b.set_full(5, rbp);
            b.define(rng, rbp - 8 * n - 8, 8 * n + 16)?;
            let instr =
                Instruction::with2(Code::Enterq_imm16_imm8, u32::from(size), u32::from(level))?;
            finish_with(b, instr, None, None)
        }
    }
}
