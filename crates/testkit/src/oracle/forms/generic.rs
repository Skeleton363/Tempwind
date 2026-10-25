//! The generic form generator: one form per iced `Code` and operand variant
//! (register or memory), with operands drawn from the `Code`'s
//! `OpCodeInfo`. It covers the integer forms whose operands are registers,
//! memory and immediates, and the SSE/SSE2 forms without floating-point
//! arithmetic. Instructions with implicit memory or control flow (string
//! ops, stack, branches, div/idiv faults) have their own generators in
//! `special.rs`.
//!
//! Per case:
//! - registers of every width, the legacy 8-bit set (AH-BH) in half of the
//!   cases;
//! - edge values first (`EDGE_PAIRS`, the count edges of shifts, the XMM
//!   lane patterns), then random ones;
//! - memory operands cycle through the 9 address modes; a share of them uses
//!   an FS or GS override (with an FS/GS base input) or the 67h prefix
//!   (32-bit address);
//! - RFLAGS bits the SDM leaves undefined leave `flags_mask` (`sdm_undefined`,
//!   cross-checked with iced's `rflags_undefined()`).

use iced_x86::{
    Code, ConditionCode, EncodingKind, InstructionInfoFactory, MemorySize, Mnemonic,
    OpCodeOperandKind as K, RflagsBits,
};

use super::*;
use crate::census::{form_name, in_profile};

/// One generic form: a `Code` and whether its r/m operand is memory.
#[derive(Clone, Copy, Debug)]
pub(super) struct Spec {
    pub code: Code,
    pub mem: bool,
}

struct FamilySpec {
    name: &'static str,
    mnemonics: &'static [Mnemonic],
    count: usize,
}

use Mnemonic as M;

/// Cases per generic form.
const COUNT: usize = 32;
/// Shifts and rotates: count classes 0, 1, other and the width edges.
const SHIFT_COUNT: usize = 48;

const SETCC: &[Mnemonic] = &[
    M::Seto,
    M::Setno,
    M::Setb,
    M::Setae,
    M::Sete,
    M::Setne,
    M::Setbe,
    M::Seta,
    M::Sets,
    M::Setns,
    M::Setp,
    M::Setnp,
    M::Setl,
    M::Setge,
    M::Setle,
    M::Setg,
];

const CMOVCC: &[Mnemonic] = &[
    M::Cmovo,
    M::Cmovno,
    M::Cmovb,
    M::Cmovae,
    M::Cmove,
    M::Cmovne,
    M::Cmovbe,
    M::Cmova,
    M::Cmovs,
    M::Cmovns,
    M::Cmovp,
    M::Cmovnp,
    M::Cmovl,
    M::Cmovge,
    M::Cmovle,
    M::Cmovg,
];

/// The generic families, in commit order (`docs/design/fixtures.md`).
const FAMILIES: &[FamilySpec] = &[
    // ALU, logic, test, inc/dec/neg/not, adc/sbb.
    fam("adc", &[M::Adc], COUNT),
    fam("sbb", &[M::Sbb], COUNT),
    fam("and", &[M::And], COUNT),
    fam("or", &[M::Or], COUNT),
    fam("xor", &[M::Xor], COUNT),
    fam("test", &[M::Test], COUNT),
    fam("inc", &[M::Inc], COUNT),
    fam("dec", &[M::Dec], COUNT),
    fam("neg", &[M::Neg], COUNT),
    fam("not", &[M::Not], COUNT),
    // Extensions.
    fam("movzx", &[M::Movzx], COUNT),
    fam("movsx", &[M::Movsx], COUNT),
    fam("movsxd", &[M::Movsxd], COUNT),
    fam("cbw", &[M::Cbw, M::Cwde, M::Cdqe], COUNT),
    fam("cwd", &[M::Cwd, M::Cdq, M::Cqo], COUNT),
    // setcc, cmovcc.
    fam("setcc", SETCC, COUNT),
    fam("cmovcc", CMOVCC, COUNT),
    // Shifts and rotates.
    fam("shl", &[M::Shl, M::Sal], SHIFT_COUNT),
    fam("shr", &[M::Shr], SHIFT_COUNT),
    fam("sar", &[M::Sar], SHIFT_COUNT),
    fam("rol", &[M::Rol], SHIFT_COUNT),
    fam("ror", &[M::Ror], SHIFT_COUNT),
    fam("rcl", &[M::Rcl], SHIFT_COUNT),
    fam("rcr", &[M::Rcr], SHIFT_COUNT),
    fam("shld", &[M::Shld], SHIFT_COUNT),
    fam("shrd", &[M::Shrd], SHIFT_COUNT),
    // Multiplies (div/idiv are special).
    fam("mul", &[M::Mul], COUNT),
    fam("imul", &[M::Imul], COUNT),
    // Bit ops.
    fam("bt", &[M::Bt], COUNT),
    fam("bts", &[M::Bts], COUNT),
    fam("btr", &[M::Btr], COUNT),
    fam("btc", &[M::Btc], COUNT),
    fam("bsf", &[M::Bsf], COUNT),
    fam("bsr", &[M::Bsr], COUNT),
    fam("bswap", &[M::Bswap], COUNT),
    // Exchanges.
    fam("xchg", &[M::Xchg], COUNT),
    fam("xadd", &[M::Xadd], COUNT),
    fam("cmpxchg", &[M::Cmpxchg], COUNT),
];

/// SSE and SSE2 without floating-point arithmetic (`translator.md`, "SSE
/// and SSE2 without floating-point arithmetic").
const SSE_FAMILIES: &[FamilySpec] = &[
    fam(
        "sse_move",
        &[
            M::Movd,
            M::Movq,
            M::Movsd,
            M::Movss,
            M::Movaps,
            M::Movups,
            M::Movapd,
            M::Movupd,
            M::Movdqa,
            M::Movdqu,
            M::Movhps,
            M::Movhpd,
            M::Movlps,
            M::Movlpd,
            M::Movhlps,
            M::Movlhps,
            M::Movntdq,
            M::Movnti,
            M::Movntps,
            M::Movntpd,
        ],
        COUNT,
    ),
    fam(
        "sse_logic",
        &[
            M::Pand,
            M::Pandn,
            M::Por,
            M::Pxor,
            M::Andps,
            M::Andpd,
            M::Andnps,
            M::Andnpd,
            M::Orps,
            M::Orpd,
            M::Xorps,
            M::Xorpd,
            M::Pmovmskb,
            M::Movmskps,
            M::Movmskpd,
            M::Ldmxcsr,
            M::Stmxcsr,
        ],
        COUNT,
    ),
    fam(
        "sse_arith",
        &[
            M::Paddb,
            M::Paddw,
            M::Paddd,
            M::Paddq,
            M::Psubb,
            M::Psubw,
            M::Psubd,
            M::Psubq,
            M::Paddsb,
            M::Paddsw,
            M::Paddusb,
            M::Paddusw,
            M::Psubsb,
            M::Psubsw,
            M::Psubusb,
            M::Psubusw,
            M::Pavgb,
            M::Pavgw,
            M::Pminub,
            M::Pmaxub,
            M::Pminsw,
            M::Pmaxsw,
            M::Pmullw,
            M::Pmulhw,
            M::Pmulhuw,
            M::Pmuludq,
            M::Pmaddwd,
            M::Psadbw,
        ],
        COUNT,
    ),
    fam(
        "sse_cmp",
        &[
            M::Pcmpeqb,
            M::Pcmpeqw,
            M::Pcmpeqd,
            M::Pcmpgtb,
            M::Pcmpgtw,
            M::Pcmpgtd,
        ],
        COUNT,
    ),
    fam(
        "sse_shift",
        &[
            M::Psllw,
            M::Pslld,
            M::Psllq,
            M::Psrlw,
            M::Psrld,
            M::Psrlq,
            M::Psraw,
            M::Psrad,
            M::Pslldq,
            M::Psrldq,
        ],
        COUNT,
    ),
    fam(
        "sse_shuffle",
        &[
            M::Pshufd,
            M::Pshuflw,
            M::Pshufhw,
            M::Shufps,
            M::Shufpd,
            M::Punpcklbw,
            M::Punpcklwd,
            M::Punpckldq,
            M::Punpcklqdq,
            M::Punpckhbw,
            M::Punpckhwd,
            M::Punpckhdq,
            M::Punpckhqdq,
            M::Unpcklps,
            M::Unpcklpd,
            M::Unpckhps,
            M::Unpckhpd,
            M::Packsswb,
            M::Packssdw,
            M::Packuswb,
            M::Pextrw,
            M::Pinsrw,
        ],
        COUNT,
    ),
];

const fn fam(name: &'static str, mnemonics: &'static [Mnemonic], count: usize) -> FamilySpec {
    FamilySpec {
        name,
        mnemonics,
        count,
    }
}

/// Codes left out on purpose, with the reason.
fn excluded(code: Code) -> Option<&'static str> {
    let name = format!("{code:?}");
    if name.ends_with("_F6r1") || name.ends_with("_F7r1") {
        return Some("TEST's /1 alias encodings: not in the SDM's opcode map");
    }
    match code {
        Code::Bswap_r16 => Some("bswap r16: the SDM leaves the result undefined"),
        Code::Movsxd_r16_rm16 | Code::Movsxd_r32_rm32 => {
            Some("movsxd without REX.W: discouraged by the SDM, differs between vendors")
        }
        _ => None,
    }
}

fn supported_kind(k: K) -> bool {
    matches!(
        k,
        K::r8_or_mem
            | K::r16_or_mem
            | K::r32_or_mem
            | K::r64_or_mem
            | K::r8_reg
            | K::r16_reg
            | K::r32_reg
            | K::r64_reg
            | K::r8_opcode
            | K::r16_opcode
            | K::r32_opcode
            | K::r64_opcode
            | K::r16_rm
            | K::r32_rm
            | K::r64_rm
            | K::al
            | K::cl
            | K::ax
            | K::eax
            | K::rax
            | K::imm8
            | K::imm8_const_1
            | K::imm8sex16
            | K::imm8sex32
            | K::imm8sex64
            | K::imm16
            | K::imm32
            | K::imm32sex64
            | K::imm64
            | K::mem
            | K::mem_offs
            | K::xmm_reg
            | K::xmm_rm
            | K::xmm_or_mem
    )
}

fn has_rm(k: K) -> bool {
    matches!(
        k,
        K::r8_or_mem | K::r16_or_mem | K::r32_or_mem | K::r64_or_mem | K::xmm_or_mem
    )
}

fn is_mem_only(k: K) -> bool {
    matches!(k, K::mem | K::mem_offs)
}

/// An instruction of `code` with no operands set (for table lookups).
fn bare(code: Code) -> Instruction {
    let mut i = Instruction::default();
    i.set_code(code);
    i
}

/// The in-profile, 64-bit, legacy-encoded `Code`s of `mnemonics` the generic
/// generator handles, in `Code` order.
fn codes_of(mnemonics: &[Mnemonic], sse: bool) -> Vec<Code> {
    Code::values()
        .filter(|c| mnemonics.contains(&c.mnemonic()))
        .filter(|&c| {
            let info = c.op_code();
            info.mode64()
                && info.encoding() == EncodingKind::Legacy
                && excluded(c).is_none()
                && info.op_kinds().iter().all(|&k| supported_kind(k))
                && bare(c).cpuid_features().iter().all(|&f| in_profile(f))
                // The string forms of movsd live in the strings family.
                && (!sse || c.mnemonic() != M::Movsd || info.op_kinds().contains(&K::xmm_reg))
        })
        .collect()
}

fn push_code(out: &mut Vec<Form>, family: &'static str, count: usize, code: Code) {
    let kinds = code.op_code().op_kinds();
    let variants: &[bool] = if kinds.iter().any(|&k| has_rm(k)) {
        &[false, true]
    } else if kinds.iter().any(|&k| is_mem_only(k)) {
        &[true]
    } else {
        &[false]
    };
    for &mem in variants {
        out.push(Form {
            id: format!("{code:?}/{}", if mem { "mem" } else { "reg" }),
            family,
            count,
            kind: Kind::Generic(Spec { code, mem }),
        });
    }
}

/// Every generic form, family by family. After each family, `special`
/// adds the forms that follow it in the commit order.
pub(super) fn forms(out: &mut Vec<Form>, special: &mut dyn FnMut(&str, &mut Vec<Form>)) {
    for (f, sse) in FAMILIES
        .iter()
        .map(|f| (f, false))
        .chain(SSE_FAMILIES.iter().map(|f| (f, true)))
    {
        for code in codes_of(f.mnemonics, sse) {
            push_code(out, f.name, f.count, code);
        }
        special(f.name, out);
    }
}

/// The generic forms of `mnemonics`, for families defined elsewhere.
pub(super) fn forms_of(out: &mut Vec<Form>, family: &'static str, mnemonics: &[Mnemonic]) {
    for code in codes_of(mnemonics, false) {
        push_code(out, family, COUNT, code);
    }
}

/// The generic forms of the given `Code`s.
pub(super) fn forms_of_codes(out: &mut Vec<Form>, family: &'static str, codes: &[Code]) {
    for &code in codes {
        push_code(out, family, COUNT, code);
    }
}

// ---------------------------------------------------------------------------
// Values

/// Shift and rotate counts, edges first (the masked count classes 0, 1,
/// width - 1, width, width + 1, 31/32, 63/64, and unmasked bits above).
fn count_edges(w: u32) -> Vec<u64> {
    let mut out = Vec::new();
    for c in [
        0,
        1,
        2,
        u64::from(w) - 1,
        u64::from(w),
        u64::from(w) + 1,
        7,
        8,
        9,
        15,
        16,
        17,
        31,
        32,
        33,
        63,
        64,
        0x41,
        0x80,
        0xff,
        0xe0,
        0x21,
    ] {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

fn pick_count(rng: &mut Rng, idx: usize, w: u32) -> u64 {
    let e = count_edges(w);
    if let Some(&c) = e.get(idx) {
        return c;
    }
    match rng.below(3) {
        0 => rng.below(u64::from(w) + 2),
        1 => rng.pick(&e),
        _ => rng.below(256),
    }
}

/// A 128-bit value made of `lane`-bit lanes, each an edge or random value.
fn xmm_lanes(rng: &mut Rng, lane: u32) -> u128 {
    let mut v = 0u128;
    for i in 0..128 / lane {
        let x = rand_val(rng, lane);
        v |= u128::from(x) << (i * lane);
    }
    v
}

/// Edge XMM patterns: every lane width's extremes and sign boundaries.
fn xmm_edges() -> Vec<u128> {
    let rep = |pat: u128, lane: u32| -> u128 {
        let mut v = 0u128;
        for i in 0..128 / lane {
            v |= pat << (i * lane);
        }
        v
    };
    vec![
        0,
        u128::MAX,
        rep(0x80, 8),
        rep(0x7f, 8),
        rep(0x8000, 16),
        rep(0x7fff, 16),
        rep(0x8000_0000, 32),
        rep(0x7fff_ffff, 32),
        rep(0x8000_0000_0000_0000, 64),
        rep(0x01, 8),
        rep(0xff00, 16),
        0x0f0e_0d0c_0b0a_0908_0706_0504_0302_0100,
    ]
}

fn rand_xmm(rng: &mut Rng) -> u128 {
    match rng.below(6) {
        0 => {
            let e = xmm_edges();
            rng.pick(&e)
        }
        1 => xmm_lanes(rng, 8),
        2 => xmm_lanes(rng, 16),
        3 => xmm_lanes(rng, 32),
        4 => xmm_lanes(rng, 64),
        _ => u128::from(rng.next_u64()) | u128::from(rng.next_u64()) << 64,
    }
}

/// (destination, source) XMM values for case `idx`.
fn xmm_pair(rng: &mut Rng, idx: usize) -> (u128, u128) {
    let e = xmm_edges();
    let n = e.len();
    if idx < n {
        return (e[idx], e[(idx * 5 + 3) % n]);
    }
    if idx < 2 * n && rng.chance(1, 2) {
        let a = rand_xmm(rng);
        return (a, a);
    }
    (rand_xmm(rng), rand_xmm(rng))
}

// ---------------------------------------------------------------------------
// Memory operands

/// Segment override of a generic memory case.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Seg {
    None,
    Fs,
    Gs,
}

/// A memory operand of mode `mode` whose linear address is `ea`, with an
/// optional FS/GS override (the base becomes an input) and an optional 67h
/// prefix (32-bit offset). Sets the registers (and segment base) it needs.
fn solve_mem(
    rng: &mut Rng,
    b: &mut Builder,
    mode: AddrMode,
    ea: u64,
    seg: Seg,
    a32: bool,
) -> MemoryOperand {
    use AddrMode::*;
    // The offset the address registers and displacement must produce.
    let offset = if seg == Seg::None {
        ea
    } else {
        let off = match (mode, a32) {
            // A 32-bit offset is zero-extended: 0..=ea keeps the base >= 0.
            (_, true) => rng.below(ea + 1),
            // Sign-extended disp32 (absolute) or RIP + disp32.
            (Abs, false) => {
                if rng.chance(1, 2) {
                    rng.below(ea + 1)
                } else {
                    (rng.range_i64(i32::MIN as i64, -1)) as u64
                }
            }
            (Rip, false) => rng.below(ea + 1),
            // Any 64-bit offset: the base is ea - offset.
            _ => match rng.below(3) {
                0 => rng.below(ea + 1),
                1 => ea.wrapping_sub(rng.below(1 << 40)),
                _ => ea.wrapping_sub(DATA_ADDR),
            },
        };
        let base = ea.wrapping_sub(off);
        match seg {
            Seg::Fs => b.fs_base = base,
            _ => b.gs_base = base,
        }
        off
    };
    let mut m = solve_addr(rng, b, mode, offset, true);
    if a32 {
        let narrow = |r: Register| match r {
            Register::None => Register::None,
            Register::RIP => Register::EIP,
            r => GPR32[GPR64.iter().position(|&x| x == r).expect("64-bit register")],
        };
        m.base = narrow(m.base);
        m.index = narrow(m.index);
        if m.displ_size == 8 {
            m.displ_size = 4;
        }
        if mode == Rip {
            // EIP-relative: the target is the 32-bit offset itself.
            m.displacement = offset as u32 as i64;
        }
    }
    m.segment_prefix = match seg {
        Seg::None => Register::None,
        Seg::Fs => Register::FS,
        Seg::Gs => Register::GS,
    };
    m
}

// ---------------------------------------------------------------------------
// Flags

fn to_rflags(bits: u32) -> u64 {
    let mut out = 0;
    for (iced, x86) in [
        (RflagsBits::OF, flags::OF),
        (RflagsBits::SF, flags::SF),
        (RflagsBits::ZF, flags::ZF),
        (RflagsBits::AF, flags::AF),
        (RflagsBits::CF, flags::CF),
        (RflagsBits::PF, flags::PF),
        (RflagsBits::DF, flags::DF),
    ] {
        if bits & iced != 0 {
            out |= x86;
        }
    }
    out
}

/// Operand facts the SDM's flag rules depend on.
#[derive(Clone, Copy, Default)]
pub(super) struct FlagFacts {
    /// Operand width in bits.
    pub width: u32,
    /// The unmasked shift or rotate count, if the form has one.
    pub count: Option<u64>,
}

/// RFLAGS status bits the SDM (Vol. 2, "Flags Affected" of each
/// instruction) leaves undefined for this case.
pub(super) fn sdm_undefined(m: Mnemonic, f: FlagFacts) -> u64 {
    use flags::{AF, CF, OF, PF, SF, ZF};
    let w = f.width;
    let masked = f.count.map(|c| c & if w == 64 { 63 } else { 31 });
    match m {
        M::And | M::Or | M::Xor | M::Test => AF,
        M::Shl | M::Sal | M::Shr | M::Sar => match masked.unwrap_or(1) {
            0 => 0,
            1 => AF,
            // "CF is undefined for SHL and SHR where the count is greater
            // than or equal to the size of the destination."
            c if c >= u64::from(w) && m != M::Sar => AF | OF | CF,
            _ => AF | OF,
        },
        M::Rol | M::Ror | M::Rcl | M::Rcr => match masked.unwrap_or(1) {
            0 => 0,
            1 => 0,
            _ => OF,
        },
        M::Shld | M::Shrd => match masked.unwrap_or(1) {
            0 => 0,
            1 => AF,
            _ => AF | OF,
        },
        M::Mul | M::Imul => SF | ZF | AF | PF,
        M::Div | M::Idiv => CF | OF | SF | ZF | AF | PF,
        M::Bt | M::Bts | M::Btr | M::Btc => OF | SF | AF | PF,
        M::Bsf | M::Bsr => CF | OF | SF | AF | PF,
        _ => 0,
    }
}

/// Checks the SDM table against iced's `rflags_undefined()`. iced gives one
/// set per `Code`; the SDM's depends on the operands where noted:
/// - shifts, rotates, shld/shrd: a masked count of 0 changes no flag, and
///   OF is defined for a count of 1. iced lists the set of a count above 1
///   (and for the `_1` forms the count-1 set). shl/shr also leave CF
///   undefined for counts at or above the width (8/16-bit forms), which
///   iced doesn't model.
/// - rcl/rcr: iced lists OF as undefined even for the `_1` forms. Not so:
///   "the OF flag is affected only for single-bit rotates".
/// - bsf/bsr: iced lists only CF, OF, SF, AF, PF undefined (ZF defined), as
///   the SDM.
pub(super) fn check_iced(instr: &Instruction, sdm: u64, f: FlagFacts) -> Result<()> {
    let iced = to_rflags(instr.rflags_undefined());
    let m = instr.mnemonic();
    let masked = f.count.map(|c| c & if f.width == 64 { 63 } else { 31 });
    let ok = match masked {
        // Count-dependent forms.
        Some(0) => sdm == 0,
        Some(1) => sdm & !iced == 0,
        Some(c) => {
            // shl/shr's CF for counts at or above the width: not in iced.
            let cf = if matches!(m, M::Shl | M::Sal | M::Shr) && c >= u64::from(f.width) {
                flags::CF
            } else {
                0
            };
            // 8- and 16-bit rcl/rcr: iced reduces the count modulo w + 1
            // first and sees a rotate by 0 (no flag changes) or by 1 (OF
            // defined). The SDM tests the masked count instead: "If the
            // masked count is 0, the flags are not affected. If the masked
            // count is 1, then the OF flag is affected, otherwise [...] the
            // OF flag is undefined." Same for rol/ror by a multiple of w.
            let rotate = matches!(m, M::Rol | M::Ror | M::Rcl | M::Rcr);
            if rotate && f.width < 32 {
                sdm == flags::OF && (iced == 0 || iced == sdm)
            } else {
                sdm & !cf == iced & !cf
            }
        }
        None => sdm == iced,
    };
    ensure!(
        ok,
        "{instr}: the SDM leaves {sdm:#x} undefined, iced {iced:#x} (count {:?})",
        f.count
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Generation

#[derive(Clone, Copy)]
enum Slot {
    Gpr(Register),
    Xmm(Register),
    Mem(u64),
    Imm(K),
}

fn gpr_kind_bits(k: K) -> Option<u32> {
    Some(match k {
        K::r8_or_mem | K::r8_reg | K::r8_opcode | K::al | K::cl => 8,
        K::r16_or_mem | K::r16_reg | K::r16_opcode | K::r16_rm | K::ax => 16,
        K::r32_or_mem | K::r32_reg | K::r32_opcode | K::r32_rm | K::eax => 32,
        K::r64_or_mem | K::r64_reg | K::r64_opcode | K::r64_rm | K::rax => 64,
        _ => return None,
    })
}

fn imm_op_kind(k: K) -> OpKind {
    match k {
        K::imm8 | K::imm8_const_1 => OpKind::Immediate8,
        K::imm8sex16 => OpKind::Immediate8to16,
        K::imm8sex32 => OpKind::Immediate8to32,
        K::imm8sex64 => OpKind::Immediate8to64,
        K::imm16 => OpKind::Immediate16,
        K::imm32 => OpKind::Immediate32,
        K::imm32sex64 => OpKind::Immediate32to64,
        _ => OpKind::Immediate64,
    }
}

/// Bits of an immediate as encoded (8, 16, 32 or 64).
fn imm_bits(k: K) -> u32 {
    match k {
        K::imm8 | K::imm8_const_1 | K::imm8sex16 | K::imm8sex32 | K::imm8sex64 => 8,
        K::imm16 => 16,
        K::imm32 | K::imm32sex64 => 32,
        _ => 64,
    }
}

fn is_shift(m: Mnemonic) -> bool {
    matches!(
        m,
        M::Shl | M::Sal | M::Shr | M::Sar | M::Rol | M::Ror | M::Rcl | M::Rcr | M::Shld | M::Shrd
    )
}

fn is_sse_shift(m: Mnemonic) -> bool {
    matches!(
        m,
        M::Psllw | M::Pslld | M::Psllq | M::Psrlw | M::Psrld | M::Psrlq | M::Psraw | M::Psrad
    ) || matches!(m, M::Pslldq | M::Psrldq)
}

/// Mnemonics that accept `lock` with a memory destination
/// (`translator.md`, "lock"). add, sub and their families predate the
/// prefix coverage.
fn lockable(m: Mnemonic) -> bool {
    matches!(
        m,
        M::Adc
            | M::Sbb
            | M::And
            | M::Or
            | M::Xor
            | M::Inc
            | M::Dec
            | M::Neg
            | M::Not
            | M::Bts
            | M::Btr
            | M::Btc
            | M::Xadd
            | M::Xchg
            | M::Cmpxchg
    )
}

/// Legacy-SSE m128 operands must be 16-byte aligned (#GP otherwise), except
/// for these.
fn unaligned_ok(m: Mnemonic) -> bool {
    matches!(m, M::Movdqu | M::Movups | M::Movupd)
}

/// Indexes of the cases with a misaligned m128 operand (#GP).
const MISALIGNED: [usize; 2] = [20, 27];

/// The memory size of a form's memory operand, in bytes.
fn mem_size(code: Code) -> u64 {
    let ms = code.op_code().memory_size();
    if ms == MemorySize::Unknown {
        0
    } else {
        ms.size() as u64
    }
}

pub(super) fn case(
    rng: &mut Rng,
    form: &Form,
    idx: usize,
    defaults: &[u64; 16],
    spec: Spec,
) -> Result<Option<GenCase>> {
    let code = spec.code;
    let info = code.op_code();
    let m = code.mnemonic();
    let kinds: Vec<K> = info.op_kinds().to_vec();
    let mut b = Builder::new(rng, defaults);
    let legacy8 = Some(rng.chance(1, 2));
    let no_access = matches!(
        m,
        M::Nop | M::Prefetcht0 | M::Prefetcht1 | M::Prefetcht2 | M::Prefetchnta
    );

    // Operand slots.
    let mut slots = Vec::new();
    for &k in &kinds {
        let slot = match k {
            K::al => Slot::Gpr(Register::AL),
            K::cl => Slot::Gpr(Register::CL),
            K::ax => Slot::Gpr(Register::AX),
            K::eax => Slot::Gpr(Register::EAX),
            K::rax => Slot::Gpr(Register::RAX),
            K::xmm_reg | K::xmm_rm => Slot::Xmm(XMM[rng.below(16) as usize]),
            K::xmm_or_mem if !spec.mem => Slot::Xmm(XMM[rng.below(16) as usize]),
            K::xmm_or_mem | K::mem | K::mem_offs => Slot::Mem(mem_size(code)),
            k if has_rm(k) && spec.mem => Slot::Mem(mem_size(code)),
            k if gpr_kind_bits(k).is_some() => {
                Slot::Gpr(pick_reg(rng, gpr_kind_bits(k).unwrap(), legacy8))
            }
            k => Slot::Imm(k),
        };
        slots.push(slot);
    }
    let width = kinds
        .iter()
        .find_map(|&k| gpr_kind_bits(k))
        .unwrap_or_else(|| (mem_size(code) * 8).clamp(8, 64) as u32);
    let sse = slots.iter().any(|s| matches!(s, Slot::Xmm(_))) || kinds.contains(&K::xmm_or_mem);

    // Values. `role` numbers the operands that take the (a, v) pair.
    let count_op = is_shift(m).then(|| kinds.len() - 1);
    let three_op_imul = m == M::Imul && kinds.len() == 3;
    let (pa, pv) = pick_pair(rng, idx, width);
    let (xa, xb) = xmm_pair(rng, idx);
    let mut pair_pos = 0;
    let mut xmm_pos = 0;
    let mut values: Vec<u128> = Vec::new();
    let mut count = None;
    for (i, (&slot, &k)) in slots.iter().zip(&kinds).enumerate() {
        let int_bits = gpr_kind_bits(k).unwrap_or(width);
        let v: u128 = if Some(i) == count_op {
            let c = if k == K::imm8_const_1 {
                1
            } else {
                let mut c = pick_count(rng, idx, width);
                // 16-bit shld/shrd with a masked count above 16: result and
                // flags undefined, also in memory, which fixtures can't mask.
                if matches!(m, M::Shld | M::Shrd) && width == 16 {
                    while c & 31 > 16 {
                        c = pick_count(rng, idx + 1000, width);
                    }
                }
                c
            };
            count = Some(c);
            u128::from(c)
        } else if matches!(slot, Slot::Xmm(_))
            || (sse && matches!(slot, Slot::Mem(_)) && k == K::xmm_or_mem)
        {
            let x = if xmm_pos == 0 { xa } else { xb };
            xmm_pos += 1;
            if is_sse_shift(m) && i == 1 {
                // A count in xmm/m128: edges in the low 64 bits.
                let c = match rng.below(4) {
                    0 => u128::from(pick_count(rng, idx, 64)),
                    1 => {
                        u128::from(rng.pick(&[0u64, 1, 15, 16, 31, 32, 63, 64, 1 << 32, u64::MAX]))
                    }
                    _ => u128::from(rng.below(70)),
                };
                c | (x & !(u128::from(u64::MAX)))
            } else {
                x
            }
        } else if let Slot::Imm(ik) = slot {
            if sse || matches!(m, M::Bt | M::Bts | M::Btr | M::Btc) {
                // Shuffle selectors, lane indexes, shift counts, bit offsets.
                u128::from(if is_sse_shift(m) {
                    pick_count(rng, idx, 16) & 0xff
                } else if rng.chance(1, 4) {
                    rng.pick(&[0u64, 0xff, 0x1b, 0xe4, 0x80, 0x7f, 0x0f, 0xf0])
                } else {
                    rng.below(256)
                })
            } else {
                // The source of the pair, as this immediate can encode it.
                let ib = imm_bits(ik);
                let want = if pair_pos == 0 && !three_op_imul {
                    pa
                } else {
                    pv
                };
                pair_pos += 1;
                let imm = match as_imm(want, int_bits.min(64), ib) {
                    Some(i) if ib == 64 || idx < EDGE_PAIRS.len() || rng.chance(1, 2) => i,
                    _ => rand_imm(rng, ib),
                };
                u128::from(imm as u64 & mask(ib.max(int_bits).min(64)))
            }
        } else {
            let skip = three_op_imul && i == 0;
            let v = if skip {
                rand_val(rng, int_bits)
            } else {
                let v = if pair_pos == 0 { pa } else { pv };
                pair_pos += 1;
                v
            };
            u128::from(v & mask(int_bits))
        };
        values.push(v);
    }

    // bt* with a register offset into memory: the offset is signed and
    // selects a word outside the operand; keep it within +-256 bits.
    let bt_mem = matches!(m, M::Bt | M::Bts | M::Btr | M::Btc)
        && spec.mem
        && matches!(slots.get(1), Some(Slot::Gpr(_)));
    if bt_mem {
        let off = rng.range_i64(-256, 255) as u64 & mask(width);
        values[1] = u128::from(off);
    }

    // Registers first, memory last: the address registers win collisions.
    for (slot, &v) in slots.iter().zip(&values) {
        match *slot {
            Slot::Gpr(r) => b.set_operand(rng, r, v as u64),
            Slot::Xmm(r) => b.fp.xmm[xmm_index(r)] = v,
            _ => {}
        }
    }
    // cmpxchg: the accumulator equals the destination in half the cases.
    if m == M::Cmpxchg && idx.is_multiple_of(2) {
        b.set_operand(rng, acc(width), values[0] as u64);
    }
    let mut expect_fault = (m == M::Ud2).then_some(FaultKind::Ud);
    // ldmxcsr: a valid MXCSR in memory (reserved bits set: #GP).
    if m == M::Ldmxcsr {
        values[0] = u128::from(match idx {
            9 => 0x1_0000,
            10 => 0x8000_0000,
            _ => rng.next_u64() & 0xffff,
        });
        if values[0] >> 16 != 0 {
            expect_fault = Some(FaultKind::Gp);
        }
    }
    if m == M::Stmxcsr {
        b.fp.mxcsr = (rng.next_u64() & 0xffff) as u32;
    }
    // setcc and cmovcc: the condition holds in every other case.
    let cc = bare(code).condition_code();
    if cc != ConditionCode::None {
        let want = idx.is_multiple_of(2);
        let n = cc as u8 - 1;
        let mut tries = 0;
        while condition(n, b.rflags) != want {
            b.rflags = random_flags(rng);
            tries += 1;
            ensure!(tries < 10_000, "no flags make {} {want}", form.id);
        }
    }

    let mut ops = Vec::new();
    let mut verify_ea = None;
    for (i, slot) in slots.iter().enumerate() {
        ops.push(match *slot {
            Slot::Gpr(r) | Slot::Xmm(r) => Op::Reg(r),
            Slot::Imm(K::imm8_const_1) => Op::Imm(1),
            Slot::Imm(_) => Op::Imm(values[i] as u64 as i64),
            Slot::Mem(size) => {
                let mode = ALL_MODES[idx % ALL_MODES.len()];
                // About 1 in 8 cases each with FS, with GS and with 67h.
                let seg = match idx % 8 {
                    5 => Seg::Fs,
                    6 => Seg::Gs,
                    _ => Seg::None,
                };
                let a32 = idx % 7 == 3;
                if no_access && idx % 2 == 1 {
                    // Never accessed: anywhere, mapped or not.
                    let ea = rng.pick(&[0, 0x1000, 0x7fff_ffff_0000, 0x10_0000_0000]);
                    let m = solve_mem(rng, &mut b, mode, ea, Seg::None, false);
                    Op::Mem(m)
                } else {
                    let size = size.max(1);
                    let lo = DATA_ADDR + OPERAND_LO + 64;
                    let hi = DATA_ADDR + OPERAND_HI - size - 64;
                    let mut ea = lo + rng.below(hi - lo);
                    let aligned16 = sse && size == 16 && !unaligned_ok(m);
                    if aligned16 {
                        ea &= !15;
                        if MISALIGNED.contains(&idx) {
                            ea += rng.pick(&[1, 4, 8, 12]);
                            expect_fault = Some(FaultKind::Gp);
                        }
                    } else if rng.chance(3, 4) {
                        ea &= !(size.next_power_of_two() - 1);
                    }
                    let mo = if kinds[i] == K::mem_offs {
                        // moffs: an absolute 64-bit (or 67h: 32-bit) offset.
                        let off = if seg == Seg::None {
                            ea
                        } else {
                            let off = rng.below(ea + 1);
                            match seg {
                                Seg::Fs => b.fs_base = ea - off,
                                _ => b.gs_base = ea - off,
                            }
                            off
                        };
                        let mut mo = mem_op(
                            Register::None,
                            Register::None,
                            1,
                            off as i64,
                            if a32 { 4 } else { 8 },
                        );
                        mo.segment_prefix = match seg {
                            Seg::None => Register::None,
                            Seg::Fs => Register::FS,
                            Seg::Gs => Register::GS,
                        };
                        mo
                    } else {
                        solve_mem(rng, &mut b, mode, ea, seg, a32)
                    };
                    let margin = if bt_mem { 40 } else { 8 };
                    b.define(rng, ea - margin, size + 2 * margin)?;
                    ensure!(size <= 16, "{}: {size}-byte memory operand", form.id);
                    b.put(ea, &values[i].to_le_bytes()[..size as usize])?;
                    verify_ea = Some(ea);
                    Op::Mem(mo)
                }
            }
        });
    }

    // The offset register may also be an address register, which then
    // holds an address: draw again unless the offset stays near.
    if bt_mem {
        let Slot::Gpr(r) = slots[1] else {
            unreachable!()
        };
        let (i, shift, bits) = slot(r);
        let off = sext((b.gprs[i] >> shift) & mask(bits), bits) as i64;
        if !(-256..256).contains(&off) {
            return Ok(None);
        }
    }

    let mut instr = Instruction::default();
    instr.set_code(code);
    for (i, op) in ops.iter().enumerate() {
        let i = i as u32;
        match op {
            Op::Reg(r) => {
                instr.set_op_kind(i, OpKind::Register);
                instr.set_op_register(i, *r);
            }
            Op::Mem(mo) => {
                instr.set_op_kind(i, OpKind::Memory);
                instr.set_memory_base(mo.base);
                instr.set_memory_index(mo.index);
                instr.set_memory_index_scale(mo.scale);
                instr.set_memory_displ_size(mo.displ_size);
                instr.set_memory_displacement64(mo.displacement as u64);
                instr.set_segment_prefix(mo.segment_prefix);
            }
            Op::Imm(v) => {
                instr.set_op_kind(i, imm_op_kind(kinds[i as usize]));
                instr.set_immediate_u64(i, *v as u64);
            }
        }
    }
    if spec.mem && lockable(m) && matches!(slots[0], Slot::Mem(_)) && idx % 4 == 1 {
        instr.set_has_lock_prefix(true);
    }

    // Implicit registers (mul's RDX:RAX, cbw's RAX, ...): random values.
    let mut factory = InstructionInfoFactory::new();
    let used: Vec<Register> = factory
        .info(&instr)
        .used_registers()
        .iter()
        .map(|u| u.register())
        .collect();
    for r in used {
        if let Some(i) = GPR64.iter().position(|&x| x == r.full_register())
            && !b.touched[i]
            && i != RSP_INDEX
        {
            let v = rand_val(rng, 64);
            b.set_full(i, v);
        }
    }

    let Some(bytes) = encode(&instr) else {
        return Ok(None);
    };
    let decoded = Decoder::with_ip(64, &bytes, CODE_ADDR, x64wasm::DECODER_OPTIONS).decode();
    if decoded.is_invalid() || decoded.len() != bytes.len() || decoded.code() != code {
        // E.g. xchg eax, eax encodes as 90 (nop): draw again.
        return Ok(None);
    }
    if let Some(ea) = verify_ea {
        let op = (0..decoded.op_count())
            .find(|&i| decoded.op_kind(i) == OpKind::Memory)
            .context("no memory operand")?;
        let (gprs, fs, gs) = (b.gprs, b.fs_base, b.gs_base);
        let got = decoded.virtual_address(op, 0, |reg, _, _| match reg {
            Register::FS => Some(fs),
            Register::GS => Some(gs),
            r if r.is_segment_register() => Some(0),
            r => {
                if let Some(i) = GPR64.iter().position(|&x| x == r) {
                    Some(gprs[i])
                } else {
                    GPR32
                        .iter()
                        .position(|&x| x == r)
                        .map(|i| gprs[i] & 0xffff_ffff)
                }
            }
        });
        ensure!(
            got == Some(ea),
            "{}: {:02x?} addresses {got:x?}, planned {ea:#x}",
            form.id,
            bytes
        );
    }
    ensure!(
        form_name(&decoded) == form.id,
        "{}: {:02x?} is form {}",
        form.id,
        bytes,
        form_name(&decoded)
    );

    // Flags and undefined results.
    let facts = FlagFacts { width, count };
    let undef_flags = sdm_undefined(m, facts);
    check_iced(&decoded, undef_flags, facts)?;
    let mut undefined = Undefined::NONE;
    if matches!(m, M::Bsf | M::Bsr) {
        let src = values[1] as u64 & mask(width);
        if src == 0 {
            // The SDM: "If the content of the source operand is 0, the
            // content of the destination operand is undefined." Intel keeps
            // all 64 bits of a 32-bit destination, so mask all of them.
            let (i, _, _) = slot(match slots[0] {
                Slot::Gpr(r) => r,
                _ => bail!("bsf/bsr destination is a register"),
            });
            undefined.gprs[i] = if width == 16 { 0xffff } else { u64::MAX };
        }
    }
    let flags_mask = if expect_fault.is_some() {
        // A fault leaves every flag as it was.
        FLAGS_MASK
    } else {
        FLAGS_MASK & !undef_flags
    };
    if expect_fault.is_some() {
        undefined = Undefined::NONE;
    }
    let mut asm = String::new();
    formatter().format(&decoded, &mut asm);
    let next = CODE_ADDR + bytes.len() as u64;
    Ok(Some(GenCase {
        form: form.id.clone(),
        asm,
        flags_mask,
        undefined,
        input: b.input(),
        expect_rip: if expect_fault.is_some() {
            CODE_ADDR
        } else {
            next
        },
        expect_fault,
        bytes,
    }))
}

const XMM: [Register; 16] = [
    Register::XMM0,
    Register::XMM1,
    Register::XMM2,
    Register::XMM3,
    Register::XMM4,
    Register::XMM5,
    Register::XMM6,
    Register::XMM7,
    Register::XMM8,
    Register::XMM9,
    Register::XMM10,
    Register::XMM11,
    Register::XMM12,
    Register::XMM13,
    Register::XMM14,
    Register::XMM15,
];

fn xmm_index(r: Register) -> usize {
    XMM.iter().position(|&x| x == r).expect("an XMM register")
}
