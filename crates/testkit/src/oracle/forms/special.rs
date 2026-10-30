//! Hand-written generators where the generic one doesn't fit: div/idiv
//! (with #DE), string instructions (rep, repe, repne, DF 0 and 1, count 0),
//! jrcxz and loop, push r/m64 with a register, and fninit.

use iced_x86::{Mnemonic, RepPrefixKind};

use super::generic::{self, FlagFacts};
use super::*;
use crate::oracle::fixture::{abridged_tag, full_tag_word};

#[derive(Clone, Copy, Debug)]
pub(super) enum Special {
    /// div/idiv r/m (reg or memory divisor).
    Div {
        code: Code,
        mem: bool,
    },
    /// A string instruction with its repeat prefix.
    Str {
        code: Code,
        rep: RepPrefixKind,
    },
    /// jrcxz, loop, loope, loopne.
    LoopBr(Code),
    /// push r/m64 with a register operand.
    PushRmReg,
    Fninit,
    /// The x87 forms of M4 (`x87.rs`).
    X87(super::x87::Spec),
}

const DIV_COUNT: usize = 40;
const STRING_COUNT: usize = 40;
const LOOP_COUNT: usize = 32;
const FNINIT_COUNT: usize = 32;

fn special(out: &mut Vec<Form>, id: String, family: &'static str, count: usize, s: Special) {
    out.push(Form {
        id,
        family,
        count,
        kind: Kind::Special(s),
    });
}

/// The forms that follow generic family `after` in the commit order.
pub(super) fn after(after: &str, out: &mut Vec<Form>) {
    match after {
        "imul" => {
            for (family, codes) in [
                (
                    "div",
                    [
                        Code::Div_rm8,
                        Code::Div_rm16,
                        Code::Div_rm32,
                        Code::Div_rm64,
                    ],
                ),
                (
                    "idiv",
                    [
                        Code::Idiv_rm8,
                        Code::Idiv_rm16,
                        Code::Idiv_rm32,
                        Code::Idiv_rm64,
                    ],
                ),
            ] {
                for code in codes {
                    for mem in [false, true] {
                        let id = format!("{code:?}/{}", if mem { "mem" } else { "reg" });
                        special(out, id, family, DIV_COUNT, Special::Div { code, mem });
                    }
                }
            }
        }
        "cmpxchg" => {
            use RepPrefixKind::{None as NoRep, Repe, Repne};
            let strings: [(&'static str, [Code; 4], &[RepPrefixKind]); 5] = [
                (
                    "movs",
                    [
                        Code::Movsb_m8_m8,
                        Code::Movsw_m16_m16,
                        Code::Movsd_m32_m32,
                        Code::Movsq_m64_m64,
                    ],
                    &[NoRep, Repe],
                ),
                (
                    "stos",
                    [
                        Code::Stosb_m8_AL,
                        Code::Stosw_m16_AX,
                        Code::Stosd_m32_EAX,
                        Code::Stosq_m64_RAX,
                    ],
                    &[NoRep, Repe],
                ),
                (
                    "lods",
                    [
                        Code::Lodsb_AL_m8,
                        Code::Lodsw_AX_m16,
                        Code::Lodsd_EAX_m32,
                        Code::Lodsq_RAX_m64,
                    ],
                    &[NoRep, Repe],
                ),
                (
                    "cmps",
                    [
                        Code::Cmpsb_m8_m8,
                        Code::Cmpsw_m16_m16,
                        Code::Cmpsd_m32_m32,
                        Code::Cmpsq_m64_m64,
                    ],
                    &[NoRep, Repe, Repne],
                ),
                (
                    "scas",
                    [
                        Code::Scasb_AL_m8,
                        Code::Scasw_AX_m16,
                        Code::Scasd_EAX_m32,
                        Code::Scasq_RAX_m64,
                    ],
                    &[NoRep, Repe, Repne],
                ),
            ];
            for (family, codes, reps) in strings {
                for code in codes {
                    for &rep in reps {
                        let prefix = match rep {
                            NoRep => "",
                            // F3: rep for movs/stos/lods, repe for cmps/scas.
                            Repe if matches!(family, "cmps" | "scas") => " repe",
                            Repe => " rep",
                            _ => " repne",
                        };
                        let id = format!("{code:?}/mem{prefix}");
                        special(out, id, family, STRING_COUNT, Special::Str { code, rep });
                    }
                }
            }
            for code in [
                Code::Jrcxz_rel8_64,
                Code::Loop_rel8_64_RCX,
                Code::Loope_rel8_64_RCX,
                Code::Loopne_rel8_64_RCX,
            ] {
                special(
                    out,
                    format!("{code:?}/reg"),
                    "loop",
                    LOOP_COUNT,
                    Special::LoopBr(code),
                );
            }
            generic::forms_of(
                out,
                "flagop",
                &[
                    Mnemonic::Cmc,
                    Mnemonic::Clc,
                    Mnemonic::Stc,
                    Mnemonic::Std,
                    Mnemonic::Cld,
                ],
            );
            generic::forms_of(out, "ud2", &[Mnemonic::Ud2]);
            generic::forms_of(
                out,
                "nop",
                &[
                    Mnemonic::Nop,
                    Mnemonic::Pause,
                    Mnemonic::Prefetcht0,
                    Mnemonic::Prefetcht1,
                    Mnemonic::Prefetcht2,
                    Mnemonic::Prefetchnta,
                ],
            );
            // Forms of the Phase 0 families that the census adds.
            generic::forms_of_codes(
                out,
                "mov",
                &[
                    Code::Mov_AL_moffs8,
                    Code::Mov_AX_moffs16,
                    Code::Mov_EAX_moffs32,
                    Code::Mov_RAX_moffs64,
                    Code::Mov_moffs8_AL,
                    Code::Mov_moffs16_AX,
                    Code::Mov_moffs32_EAX,
                    Code::Mov_moffs64_RAX,
                ],
            );
            special(
                out,
                "Push_rm64/reg".into(),
                "push",
                STACK_COUNT,
                Special::PushRmReg,
            );
        }
        "sse_fp_cvt" => super::x87::forms(out),
        "sse_shuffle" => special(
            out,
            "Fninit/reg".into(),
            "fninit",
            FNINIT_COUNT,
            Special::Fninit,
        ),
        _ => {}
    }
}

pub(super) fn case(
    rng: &mut Rng,
    form: &Form,
    idx: usize,
    defaults: &[u64; 16],
    s: Special,
) -> Result<Option<GenCase>> {
    match s {
        Special::Div { code, mem } => gen_div(rng, form, idx, defaults, code, mem),
        Special::Str { code, rep } => gen_string(rng, form, idx, defaults, code, rep),
        Special::LoopBr(code) => gen_loop(rng, form, idx, defaults, code),
        Special::PushRmReg => {
            let mut b = Builder::new(rng, defaults);
            setup_stack(rng, &mut b)?;
            let reg = rng.below(16) as usize;
            if reg != RSP_INDEX {
                let value = rand_val(rng, 64);
                b.set_full(reg, value);
            }
            let instr = Instruction::with1(Code::Push_rm64, GPR64[reg])?;
            done(form, b, instr, None, FLAGS_MASK)
        }
        Special::Fninit => gen_fninit(rng, form, defaults),
        Special::X87(spec) => super::x87::case(rng, form, idx, defaults, spec),
    }
}

/// Encodes, checks the decoding and builds the case.
fn done(
    form: &Form,
    b: Builder,
    instr: Instruction,
    fault: Option<FaultKind>,
    flags_mask: u64,
) -> Result<Option<GenCase>> {
    let Some(mut case) = finish(Finish {
        form,
        b,
        instr,
        target: fault.map(|_| CODE_ADDR),
        verify_ea: None,
    })?
    else {
        return Ok(None);
    };
    case.flags_mask = flags_mask;
    case.expect_fault = fault;
    Ok(Some(case))
}

// ---------------------------------------------------------------------------
// div, idiv

/// Whether `n / d` (a 2w-bit dividend, w-bit divisor) raises #DE.
fn div_faults(signed: bool, w: u32, n: u128, d: u64) -> bool {
    if d == 0 {
        return true;
    }
    if signed {
        let sh = 128 - 2 * w;
        let n = ((n << sh) as i128) >> sh;
        let d = i128::from(sext(d, w) as i64);
        let q = n / d;
        let lim = 1i128 << (w - 1);
        !(-lim..lim).contains(&q)
    } else {
        n / u128::from(d) > u128::from(mask(w))
    }
}

fn gen_div(
    rng: &mut Rng,
    form: &Form,
    idx: usize,
    defaults: &[u64; 16],
    code: Code,
    mem: bool,
) -> Result<Option<GenCase>> {
    let signed = code.mnemonic() == Mnemonic::Idiv;
    let w = match code {
        Code::Div_rm8 | Code::Idiv_rm8 => 8,
        Code::Div_rm16 | Code::Idiv_rm16 => 16,
        Code::Div_rm32 | Code::Idiv_rm32 => 32,
        _ => 64,
    };
    let m = mask(w);
    let mut b = Builder::new(rng, defaults);
    // (high half, low half, divisor): the edges first.
    let (hi, lo, d) = match idx {
        0 => (rand_val(rng, w), rand_val(rng, w), 0),
        1 => (0, 0, 0),
        // Quotient overflow: high half equal to / above the divisor.
        2 => {
            let d = rand_val(rng, w) | 1;
            (d, rand_val(rng, w), d)
        }
        3 => (m, m, 1),
        // INT_MIN / -1.
        4 => (m, smin(w), m),
        5 => (0, rand_val(rng, w), 1),
        6 => (0, m, m),
        7 => (0, smax(w), m),
        8 => (m, m, m),
        9 => (0, 0, rand_val(rng, w) | 1),
        10 => (m, smin(w), 1),
        11 => (m, smin(w), 2),
        _ => {
            let d = loop {
                let d = rand_val(rng, w);
                if d != 0 {
                    break d;
                }
            };
            let lo = rand_val(rng, w);
            let hi = match rng.below(4) {
                // Fits: a sign-extended dividend (idiv) or hi < d (div).
                0 | 1 if signed => {
                    if sext(lo, w) >> 63 == 1 {
                        m
                    } else {
                        0
                    }
                }
                0 | 1 => rng.below(d.max(1)),
                2 => rand_val(rng, w),
                _ => 0,
            };
            (hi, lo, d)
        }
    };
    let legacy8 = Some(rng.chance(1, 2));
    if w == 8 {
        b.set_operand(rng, Register::AX, (hi << 8 | lo) & 0xffff);
    } else {
        b.set_operand(rng, gpr(w, 0), lo);
        b.set_operand(rng, gpr(w, 2), hi);
    }
    let (op, ea) = if mem {
        let size = u64::from(w / 8);
        let (mo, ea) = gen_mem(rng, &mut b, size, true)?;
        b.put(ea, &d.to_le_bytes()[..size as usize])?;
        (Op::Mem(mo), Some(ea))
    } else {
        let r = pick_reg(rng, w, legacy8);
        b.set_operand(rng, r, d);
        (Op::Reg(r), None)
    };
    // Read back what the instruction will see (registers may coincide).
    let get = |r: Register| {
        let (i, shift, bits) = slot(r);
        (b.gprs[i] >> shift) & mask(bits)
    };
    let n: u128 = if w == 8 {
        u128::from(get(Register::AX))
    } else {
        u128::from(get(gpr(w, 2))) << w | u128::from(get(gpr(w, 0)))
    };
    let d = match (&op, ea) {
        (Op::Reg(r), _) => get(*r),
        (_, Some(_)) => d,
        _ => unreachable!(),
    };
    let fault = div_faults(signed, w, n, d).then_some(FaultKind::De);
    let instr = match op {
        Op::Reg(r) => Instruction::with1(code, r)?,
        Op::Mem(mo) => Instruction::with1(code, mo)?,
        Op::Imm(_) => unreachable!(),
    };
    let facts = FlagFacts {
        width: w,
        count: None,
    };
    let undef = generic::sdm_undefined(code.mnemonic(), facts);
    generic::check_iced(&instr, undef, facts)?;
    let flags_mask = if fault.is_some() {
        FLAGS_MASK
    } else {
        FLAGS_MASK & !undef
    };
    let Some(mut case) = finish(Finish {
        form,
        b,
        instr,
        target: fault.map(|_| CODE_ADDR),
        verify_ea: ea,
    })?
    else {
        return Ok(None);
    };
    case.flags_mask = flags_mask;
    case.expect_fault = fault;
    Ok(Some(case))
}

// ---------------------------------------------------------------------------
// Strings

fn gen_string(
    rng: &mut Rng,
    form: &Form,
    idx: usize,
    defaults: &[u64; 16],
    code: Code,
    rep: RepPrefixKind,
) -> Result<Option<GenCase>> {
    use Mnemonic as M;
    let m = code.mnemonic();
    let size: u64 = match code {
        Code::Movsb_m8_m8
        | Code::Stosb_m8_AL
        | Code::Lodsb_AL_m8
        | Code::Cmpsb_m8_m8
        | Code::Scasb_AL_m8 => 1,
        Code::Movsw_m16_m16
        | Code::Stosw_m16_AX
        | Code::Lodsw_AX_m16
        | Code::Cmpsw_m16_m16
        | Code::Scasw_AX_m16 => 2,
        Code::Movsd_m32_m32
        | Code::Stosd_m32_EAX
        | Code::Lodsd_EAX_m32
        | Code::Cmpsd_m32_m32
        | Code::Scasd_EAX_m32 => 4,
        _ => 8,
    };
    let repeated = rep != RepPrefixKind::None;
    let mut b = Builder::new(rng, defaults);
    let df = idx % 2 == 1;
    b.rflags = (b.rflags & !flags::DF) | if df { flags::DF } else { 0 };
    let count: u64 = if repeated {
        const EDGES: [u64; 10] = [0, 0, 1, 1, 2, 3, 7, 8, 16, 31];
        EDGES.get(idx).copied().unwrap_or_else(|| rng.below(49))
    } else {
        1
    };
    if repeated {
        b.set_full(1, count);
    } else {
        let v = rand_full(rng);
        b.set_full(1, v);
    }
    // The elements touched: n of them, forwards from the start (DF = 0) or
    // backwards (DF = 1).
    let n = count.max(1);
    let span = n * size;
    let lo = DATA_ADDR + OPERAND_LO + 16;
    let hi = DATA_ADDR + OPERAND_HI - span - 16;
    let pick = |rng: &mut Rng| {
        let mut p = lo + rng.below(hi - lo);
        if rng.chance(3, 4) {
            p &= !(size - 1);
        }
        p
    };
    // Lowest address of the range starting at `p`.
    let low = |p: u64| if df { p - (n - 1) * size } else { p };
    let reads_si = matches!(m, M::Movsb | M::Movsw | M::Movsd | M::Movsq)
        || matches!(m, M::Lodsb | M::Lodsw | M::Lodsd | M::Lodsq)
        || matches!(m, M::Cmpsb | M::Cmpsw | M::Cmpsd | M::Cmpsq);
    let uses_di = !matches!(m, M::Lodsb | M::Lodsw | M::Lodsd | M::Lodsq);
    let mut si = pick(rng) + if df { span } else { 0 };
    let mut di = pick(rng) + if df { span } else { 0 };
    if matches!(m, M::Movsb | M::Movsw | M::Movsd | M::Movsq) && idx % 4 == 3 && n > 1 {
        // Overlapping source and destination, either way round, by whole
        // elements or by bytes.
        let k = rng.below(n.min(8)) + 1;
        let delta = if rng.chance(1, 2) { k * size } else { k };
        di = if rng.chance(1, 2) {
            si + delta
        } else {
            si - delta
        };
    }
    si = si.clamp(lo + span, hi);
    di = di.clamp(lo + span, hi);
    if !reads_si {
        let v = rand_full(rng);
        b.set_full(6, v);
    } else {
        b.set_full(6, si);
        b.define(rng, low(si) - 8, span + 16)?;
    }
    if uses_di {
        b.set_full(7, di);
        b.define(rng, low(di) - 8, span + 16)?;
    } else {
        let v = rand_full(rng);
        b.set_full(7, v);
    }
    let rax = rand_full(rng);
    b.set_full(0, rax);
    // cmps/scas: make the elements equal (or different) up to a point, so
    // repe and repne stop inside the range as well as run through it.
    let elem = |i: u64, p: u64| if df { p - i * size } else { p + i * size };
    if matches!(m, M::Cmpsb | M::Cmpsw | M::Cmpsd | M::Cmpsq) {
        let stop = rng.below(n + 1);
        for i in 0..n {
            let a = &b.image[(elem(i, si) - DATA_ADDR) as usize..][..size as usize].to_vec();
            let equal = match rep {
                RepPrefixKind::Repne => i == stop,
                _ => i != stop,
            };
            if equal {
                b.put(elem(i, di), a)?;
            } else if b.image[(elem(i, di) - DATA_ADDR) as usize..][..size as usize] == a[..] {
                let mut x = a.clone();
                x[0] ^= 1;
                b.put(elem(i, di), &x)?;
            }
        }
    }
    if matches!(m, M::Scasb | M::Scasw | M::Scasd | M::Scasq) {
        let stop = rng.below(n + 1);
        let a = rax.to_le_bytes();
        for i in 0..n {
            let at = (elem(i, di) - DATA_ADDR) as usize;
            let equal = match rep {
                RepPrefixKind::Repne => i == stop,
                _ => i != stop,
            };
            if equal {
                b.put(elem(i, di), &a[..size as usize])?;
            } else if b.image[at..at + size as usize] == a[..size as usize] {
                let mut x = a[..size as usize].to_vec();
                x[0] ^= 1;
                b.put(elem(i, di), &x)?;
            }
        }
    }
    let r = Register::None;
    let instr = match code {
        Code::Movsb_m8_m8 => Instruction::with_movsb(64, r, rep)?,
        Code::Movsw_m16_m16 => Instruction::with_movsw(64, r, rep)?,
        Code::Movsd_m32_m32 => Instruction::with_movsd(64, r, rep)?,
        Code::Movsq_m64_m64 => Instruction::with_movsq(64, r, rep)?,
        Code::Stosb_m8_AL => Instruction::with_stosb(64, rep)?,
        Code::Stosw_m16_AX => Instruction::with_stosw(64, rep)?,
        Code::Stosd_m32_EAX => Instruction::with_stosd(64, rep)?,
        Code::Stosq_m64_RAX => Instruction::with_stosq(64, rep)?,
        Code::Lodsb_AL_m8 => Instruction::with_lodsb(64, r, rep)?,
        Code::Lodsw_AX_m16 => Instruction::with_lodsw(64, r, rep)?,
        Code::Lodsd_EAX_m32 => Instruction::with_lodsd(64, r, rep)?,
        Code::Lodsq_RAX_m64 => Instruction::with_lodsq(64, r, rep)?,
        Code::Cmpsb_m8_m8 => Instruction::with_cmpsb(64, r, rep)?,
        Code::Cmpsw_m16_m16 => Instruction::with_cmpsw(64, r, rep)?,
        Code::Cmpsd_m32_m32 => Instruction::with_cmpsd(64, r, rep)?,
        Code::Cmpsq_m64_m64 => Instruction::with_cmpsq(64, r, rep)?,
        Code::Scasb_AL_m8 => Instruction::with_scasb(64, rep)?,
        Code::Scasw_AX_m16 => Instruction::with_scasw(64, rep)?,
        Code::Scasd_EAX_m32 => Instruction::with_scasd(64, rep)?,
        Code::Scasq_RAX_m64 => Instruction::with_scasq(64, rep)?,
        _ => bail!("{code:?} is not a string instruction"),
    };
    done(form, b, instr, None, FLAGS_MASK)
}

// ---------------------------------------------------------------------------
// jrcxz, loop

fn gen_loop(
    rng: &mut Rng,
    form: &Form,
    idx: usize,
    defaults: &[u64; 16],
    code: Code,
) -> Result<Option<GenCase>> {
    let mut b = Builder::new(rng, defaults);
    let rcx = match idx {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 0x1_0000_0000,
        4 => 0x1_0000_0001,
        5 => u64::MAX,
        6 => 0xffff_ffff,
        7 => 0,
        8 => 1,
        _ => match rng.below(3) {
            0 => rng.below(4),
            _ => rand_val(rng, 64),
        },
    };
    b.set_full(1, rcx);
    let target = pick_rel_target(rng, 2, true);
    let zf = b.rflags & flags::ZF != 0;
    let next_rcx = rcx.wrapping_sub(1);
    let taken = match code {
        Code::Jrcxz_rel8_64 => rcx == 0,
        Code::Loop_rel8_64_RCX => next_rcx != 0,
        Code::Loope_rel8_64_RCX => next_rcx != 0 && zf,
        _ => next_rcx != 0 && !zf,
    };
    let instr = Instruction::with_branch(code, target)?;
    let expect = if taken { target } else { CODE_ADDR + 2 };
    finish(Finish {
        form,
        b,
        instr,
        target: Some(expect),
        verify_ea: None,
    })
}

// ---------------------------------------------------------------------------
// fninit

/// A random 80-bit register value: normals, zeros, denormals, infinities,
/// NaNs and unnormals.
fn rand_f80(rng: &mut Rng) -> u128 {
    let sign = u128::from(rng.below(2)) << 79;
    let sig = rng.next_u64();
    let (exp, sig) = match rng.below(6) {
        0 => (0, 0),
        1 => (0, sig >> 1),
        2 => (0x7fff, 1 << 63),
        3 => (0x7fff, sig | 1 << 63),
        4 => (1 + rng.below(0x7ffe), sig & !(1 << 63)),
        _ => (1 + rng.below(0x7ffe), sig | 1 << 63),
    };
    sign | u128::from(exp) << 64 | u128::from(sig)
}

fn gen_fninit(rng: &mut Rng, form: &Form, defaults: &[u64; 16]) -> Result<Option<GenCase>> {
    let mut b = Builder::new(rng, defaults);
    let fp = &mut b.fp;
    // FCW bits 6 and 13-15 are reserved; FNSTCW reads bit 6 as 1.
    fp.fcw = (rng.next_u64() as u16 & 0x1f3f) | 0x40;
    fp.fsw = rng.next_u64() as u16;
    for i in 0..8 {
        fp.st[i] = rand_f80(rng);
    }
    let abridged = rng.next_u64() as u8;
    fp.ftw = full_tag_word(abridged, fp);
    debug_assert_eq!(abridged_tag(fp.ftw), abridged);
    fp.mxcsr = (rng.next_u64() & 0xffff) as u32;
    for _ in 0..4 {
        let r = rng.below(16) as usize;
        fp.xmm[r] = u128::from(rng.next_u64()) | u128::from(rng.next_u64()) << 64;
    }
    done(form, b, Instruction::with(Code::Fninit), None, FLAGS_MASK)
}
