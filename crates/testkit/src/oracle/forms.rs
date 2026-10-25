//! Case generation: instruction forms, encodings and inputs. Platform
//! independent and fully deterministic (seeded per form); the native recorder
//! then supplies the outputs.

use anyhow::{Context, Result, bail, ensure};
use iced_x86::{
    Code, Decoder, DecoderOptions, Encoder, Formatter, Instruction, IntelFormatter, MemoryOperand,
    OpKind, Register,
};
use x64wasm::abi::{SENTINEL_RET, flags};

use super::fixture::{FaultKind, FpState, Input, MemBlock, Undefined};
use super::rng::Rng;
use super::{CODE_ADDR, DATA_ADDR, DATA_SIZE, SEED};

/// Output RFLAGS bits compared for every form recorded so far: all six status
/// flags (written or preserved, but always architecturally defined) and DF.
pub const FLAGS_MASK: u64 = flags::STATUS | flags::DF;

/// Memory operands of non-stack forms land in this part of the data window.
const OPERAND_LO: u64 = 0x100;
const OPERAND_HI: u64 = 0x7f00;
/// RSP of stack forms lies in this part of the data window.
const STACK_LO: u64 = 0x8100;
const STACK_HI: u64 = 0xfe00;
/// Branch, call and return targets never land in the first 16 bytes of the
/// code page, so they never point back into the instruction itself.
const NO_TARGET_LEN: u64 = 16;

mod generic;
mod special;

/// One generated case, before recording.
#[derive(Clone, Debug)]
pub struct GenCase {
    pub form: String,
    pub bytes: Vec<u8>,
    pub asm: String,
    pub flags_mask: u64,
    /// Register bits the SDM leaves undefined for this case.
    pub undefined: Undefined,
    pub input: Input,
    /// RIP the generator predicts after the instruction; the recorder checks it.
    pub expect_rip: u64,
    /// The exception the generator predicts; the recorder checks it.
    pub expect_fault: Option<FaultKind>,
}

pub struct Family {
    pub name: &'static str,
    pub cases: Vec<GenCase>,
}

/// Input values of registers a case does not set.
pub fn default_gprs() -> [u64; 16] {
    let mut rng = Rng::for_stream(SEED, "default_gprs");
    std::array::from_fn(|_| rng.next_u64())
}

// ---------------------------------------------------------------------------
// Registers

mod regs {
    use iced_x86::Register::{self, *};

    pub const GPR64: [Register; 16] = [
        RAX, RCX, RDX, RBX, RSP, RBP, RSI, RDI, R8, R9, R10, R11, R12, R13, R14, R15,
    ];
    pub const GPR32: [Register; 16] = [
        EAX, ECX, EDX, EBX, ESP, EBP, ESI, EDI, R8D, R9D, R10D, R11D, R12D, R13D, R14D, R15D,
    ];
    pub const GPR16: [Register; 16] = [
        AX, CX, DX, BX, SP, BP, SI, DI, R8W, R9W, R10W, R11W, R12W, R13W, R14W, R15W,
    ];
    /// Low byte registers (need REX for indices 4-15).
    pub const GPR8: [Register; 16] = [
        AL, CL, DL, BL, SPL, BPL, SIL, DIL, R8L, R9L, R10L, R11L, R12L, R13L, R14L, R15L,
    ];
    /// Encodable without REX.
    pub const GPR8_LEGACY: [Register; 8] = [AL, CL, DL, BL, AH, CH, DH, BH];
    /// High byte registers, of RAX..RBX.
    pub const GPR8_HIGH: [Register; 4] = [AH, CH, DH, BH];
}

use regs::{GPR8, GPR8_HIGH, GPR8_LEGACY, GPR16, GPR32, GPR64};

const RSP_INDEX: usize = 4;

/// (full register index, bit shift, width) of a general-purpose register.
fn slot(r: Register) -> (usize, u32, u32) {
    for (table, bits) in [(&GPR64, 64), (&GPR32, 32), (&GPR16, 16), (&GPR8, 8)] {
        if let Some(i) = table.iter().position(|&x| x == r) {
            return (i, 0, bits);
        }
    }
    let i = GPR8_HIGH
        .iter()
        .position(|&x| x == r)
        .expect("general-purpose register");
    (i, 8, 8)
}

fn gpr(bits: u32, index: usize) -> Register {
    match bits {
        64 => GPR64[index],
        32 => GPR32[index],
        16 => GPR16[index],
        _ => GPR8[index],
    }
}

/// A random register of width `bits`. For 8-bit: `Some(true)` picks from the
/// legacy set (AL..BL, AH..BH), `Some(false)` from the REX set (AL..BL,
/// SPL..DIL, R8B..R15B), `None` from either.
fn pick_reg(rng: &mut Rng, bits: u32, legacy8: Option<bool>) -> Register {
    if bits != 8 {
        return gpr(bits, rng.below(16) as usize);
    }
    match legacy8 {
        Some(true) => rng.pick(&GPR8_LEGACY),
        Some(false) => rng.pick(&GPR8),
        None => {
            let i = rng.below(20) as usize;
            if i < 16 { GPR8[i] } else { GPR8_LEGACY[i - 12] }
        }
    }
}

fn acc(bits: u32) -> Register {
    gpr(bits, 0)
}

// ---------------------------------------------------------------------------
// Values

fn mask(bits: u32) -> u64 {
    if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

fn sext(v: u64, bits: u32) -> u64 {
    let shift = 64 - bits;
    (((v << shift) as i64) >> shift) as u64
}

fn smax(bits: u32) -> u64 {
    mask(bits) >> 1
}

fn smin(bits: u32) -> u64 {
    1u64 << (bits - 1)
}

/// Edge values of a `bits`-wide operand.
fn edges(bits: u32) -> Vec<u64> {
    let mut out = Vec::new();
    for v in [
        0,
        1,
        u64::MAX,
        smax(bits),
        smin(bits),
        0x7f,
        0x80,
        0xff,
        0x7fff,
        0x8000,
        0xffff,
        0x7fff_ffff,
        0x8000_0000,
        0xffff_ffff,
        0x1_0000_0000,
        0x0f,
        0x10,
    ] {
        let v = v & mask(bits);
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

/// A random `bits`-wide value with a bias towards interesting shapes.
fn rand_val(rng: &mut Rng, bits: u32) -> u64 {
    let v = match rng.below(10) {
        0 => rng.below(16),
        1 => rng.below(16).wrapping_neg(),
        2 | 3 => {
            let e = edges(bits);
            rng.pick(&e)
        }
        4 => {
            // Runs of ones and zeros: long carry and borrow chains.
            let a = rng.below(64);
            let b = rng.below(64);
            let (lo, hi) = (a.min(b), a.max(b));
            (u64::MAX >> (63 - (hi - lo))) << lo
        }
        5 => rng.next_u64() & 0x0f0f_0f0f_0f0f_0f0f,
        _ => rng.next_u64(),
    };
    v & mask(bits)
}

/// A full 64-bit value for the untouched part of a register.
fn rand_full(rng: &mut Rng) -> u64 {
    match rng.below(8) {
        0 => 0,
        1 => u64::MAX,
        _ => rng.next_u64(),
    }
}

#[derive(Clone, Copy)]
enum E {
    Z,
    One,
    M1,
    Max,
    Min,
    C(u64),
}

fn ev(e: E, bits: u32) -> u64 {
    let v = match e {
        E::Z => 0,
        E::One => 1,
        E::M1 => u64::MAX,
        E::Max => smax(bits),
        E::Min => smin(bits),
        E::C(c) => c,
    };
    v & mask(bits)
}

/// Fixed (destination, source) pairs every two-operand form starts with:
/// carries, signed overflow, equality, AF.
const EDGE_PAIRS: [(E, E); 17] = [
    (E::Z, E::Z),
    (E::One, E::M1),
    (E::M1, E::One),
    (E::Max, E::One),
    (E::Min, E::One),
    (E::Min, E::M1),
    (E::M1, E::M1),
    (E::Max, E::Max),
    (E::Min, E::Min),
    (E::Max, E::M1),
    (E::Z, E::One),
    (E::One, E::Z),
    (E::Min, E::Max),
    (E::Max, E::Min),
    (E::C(0x0f), E::C(0x01)),
    (E::C(0x80), E::C(0x80)),
    (E::C(0x7f), E::C(0x01)),
];

/// (destination, source) values for case `idx` of a `bits`-wide form.
fn pick_pair(rng: &mut Rng, idx: usize, bits: u32) -> (u64, u64) {
    if let Some(&(a, b)) = EDGE_PAIRS.get(idx) {
        return (ev(a, bits), ev(b, bits));
    }
    let e = edges(bits);
    let m = mask(bits);
    match rng.below(8) {
        0 => (rng.pick(&e), rng.pick(&e)),
        1 => (rand_val(rng, bits), rng.pick(&e)),
        2 => (rng.pick(&e), rand_val(rng, bits)),
        3 => {
            let a = rand_val(rng, bits);
            (a, a)
        }
        4 => {
            // a + b close to 0 / 2^bits: carries out of every bit.
            let a = rng.next_u64() & m;
            let delta = rng.range_i64(-3, 3) as u64;
            (a, a.wrapping_neg().wrapping_add(delta) & m)
        }
        5 => {
            let a = rng.next_u64() & m;
            (a, !a & m)
        }
        _ => (rand_val(rng, bits), rand_val(rng, bits)),
    }
}

/// `v` (a `bits`-wide value) as an `imm_bits` immediate, if the sign-extended
/// immediate reproduces it.
fn as_imm(v: u64, bits: u32, imm_bits: u32) -> Option<i64> {
    let signed = sext(v, bits) as i64;
    if imm_bits >= bits {
        return Some(signed);
    }
    let lo = -(1i64 << (imm_bits - 1));
    let hi = (1i64 << (imm_bits - 1)) - 1;
    (lo..=hi).contains(&signed).then_some(signed)
}

/// A random signed `imm_bits` immediate.
fn rand_imm(rng: &mut Rng, imm_bits: u32) -> i64 {
    if imm_bits == 64 {
        return rand_val(rng, 64) as i64;
    }
    if rng.chance(1, 3) {
        let e = edges(imm_bits);
        return sext(rng.pick(&e), imm_bits) as i64;
    }
    sext(rand_val(rng, imm_bits), imm_bits) as i64
}

fn random_flags(rng: &mut Rng) -> u64 {
    let df = if rng.chance(1, 4) { flags::DF } else { 0 };
    flags::RESERVED1 | flags::IF | (rng.next_u64() & flags::STATUS) | df
}

// ---------------------------------------------------------------------------
// Case builder

struct Builder {
    gprs: [u64; 16],
    touched: [bool; 16],
    rflags: u64,
    fs_base: u64,
    gs_base: u64,
    fp: FpState,
    image: Vec<u8>,
    defined: Vec<bool>,
}

impl Builder {
    fn new(rng: &mut Rng, defaults: &[u64; 16]) -> Self {
        Builder {
            gprs: *defaults,
            touched: [false; 16],
            rflags: random_flags(rng),
            fs_base: 0,
            gs_base: 0,
            fp: FpState::DEFAULT,
            image: vec![0; DATA_SIZE as usize],
            defined: vec![false; DATA_SIZE as usize],
        }
    }

    fn set_full(&mut self, index: usize, value: u64) {
        self.gprs[index] = value;
        self.touched[index] = true;
    }

    /// Sets register `r` to `value` (its width). The first time a full
    /// register is touched, its other bits become random.
    fn set_operand(&mut self, rng: &mut Rng, r: Register, value: u64) {
        let (i, shift, bits) = slot(r);
        if !self.touched[i] {
            self.set_full(i, rand_full(rng));
        }
        let m = mask(bits) << shift;
        self.gprs[i] = (self.gprs[i] & !m) | ((value << shift) & m);
    }

    fn offset(addr: u64, len: u64) -> Result<usize> {
        ensure!(
            addr >= DATA_ADDR && addr + len <= DATA_ADDR + DATA_SIZE,
            "range {addr:#x}+{len:#x} is outside the data window"
        );
        Ok((addr - DATA_ADDR) as usize)
    }

    /// Marks `addr..addr+len` as compared input, filling new bytes randomly.
    fn define(&mut self, rng: &mut Rng, addr: u64, len: u64) -> Result<()> {
        let at = Self::offset(addr, len)?;
        for i in at..at + len as usize {
            if !self.defined[i] {
                self.defined[i] = true;
                self.image[i] = rng.next_u64() as u8;
            }
        }
        Ok(())
    }

    fn put(&mut self, addr: u64, bytes: &[u8]) -> Result<()> {
        let at = Self::offset(addr, bytes.len() as u64)?;
        self.image[at..at + bytes.len()].copy_from_slice(bytes);
        self.defined[at..at + bytes.len()].fill(true);
        Ok(())
    }

    fn input(&self) -> Input {
        let mut mem = Vec::new();
        let mut i = 0;
        while i < self.defined.len() {
            if !self.defined[i] {
                i += 1;
                continue;
            }
            let start = i;
            while i < self.defined.len() && self.defined[i] {
                i += 1;
            }
            mem.push(MemBlock {
                addr: DATA_ADDR + start as u64,
                bytes: self.image[start..i].to_vec(),
            });
        }
        Input {
            gprs: self.gprs,
            rflags: self.rflags,
            fs_base: self.fs_base,
            gs_base: self.gs_base,
            fp: self.fp,
            mem,
        }
    }
}

// ---------------------------------------------------------------------------
// Memory operands

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddrMode {
    Base,
    BaseDisp8,
    BaseDisp32,
    BaseIndex,
    BaseIndexDisp8,
    BaseIndexDisp32,
    IndexDisp32,
    Abs,
    Rip,
}

const ALL_MODES: [AddrMode; 9] = [
    AddrMode::Base,
    AddrMode::BaseDisp8,
    AddrMode::BaseDisp32,
    AddrMode::BaseIndex,
    AddrMode::BaseIndexDisp8,
    AddrMode::BaseIndexDisp32,
    AddrMode::IndexDisp32,
    AddrMode::Abs,
    AddrMode::Rip,
];

fn rand_disp8(rng: &mut Rng) -> i64 {
    if rng.chance(1, 4) {
        rng.pick(&[-128, 127, 0, -1, 1])
    } else {
        rng.range_i64(-128, 127)
    }
}

fn rand_disp32(rng: &mut Rng) -> i64 {
    match rng.below(6) {
        0 => rng.pick(&[
            i32::MIN as i64,
            i32::MAX as i64,
            0,
            -1,
            0x7f,
            0x80,
            -0x81,
            0x7fff_fff8,
        ]),
        1 => rng.range_i64(-0x1000, 0x1000),
        _ => rng.range_i64(i32::MIN as i64, i32::MAX as i64),
    }
}

fn rand_index_val(rng: &mut Rng) -> u64 {
    match rng.below(5) {
        0 => rng.below(0x1000),
        1 => rng.below(0x1000).wrapping_neg(),
        2 => rng.below(1 << 32),
        _ => rng.next_u64(),
    }
}

fn pick_base(rng: &mut Rng, allow_rsp: bool) -> usize {
    loop {
        let i = rng.below(16) as usize;
        if i != RSP_INDEX || allow_rsp {
            return i;
        }
    }
}

fn pick_index(rng: &mut Rng, not: Option<usize>) -> usize {
    loop {
        let i = rng.below(16) as usize;
        if i != RSP_INDEX && Some(i) != not {
            return i;
        }
    }
}

fn mem_op(
    base: Register,
    index: Register,
    scale: u32,
    disp: i64,
    displ_size: u32,
) -> MemoryOperand {
    MemoryOperand::with_base_index_scale_displ_size(base, index, scale, disp, displ_size)
}

/// A memory operand in `mode` whose effective address is `ea`: picks the
/// registers and displacement, then solves the register values (64-bit
/// wrap-around included).
fn solve_addr(
    rng: &mut Rng,
    b: &mut Builder,
    mode: AddrMode,
    ea: u64,
    allow_rsp: bool,
) -> MemoryOperand {
    use AddrMode::*;
    let scale = rng.pick(&[1u32, 2, 4, 8]);
    match mode {
        Base | BaseDisp8 | BaseDisp32 => {
            let base = pick_base(rng, allow_rsp);
            let (disp, size) = match mode {
                Base => (0, 0),
                BaseDisp8 => (rand_disp8(rng), 1),
                _ => (rand_disp32(rng), 8),
            };
            b.set_full(base, ea.wrapping_sub(disp as u64));
            mem_op(GPR64[base], Register::None, 1, disp, size)
        }
        BaseIndex | BaseIndexDisp8 | BaseIndexDisp32 => {
            let base = pick_base(rng, allow_rsp);
            let index = pick_index(rng, Some(base));
            let (disp, size) = match mode {
                BaseIndex => (0, 0),
                BaseIndexDisp8 => (rand_disp8(rng), 1),
                _ => (rand_disp32(rng), 8),
            };
            let iv = rand_index_val(rng);
            b.set_full(index, iv);
            let bv = ea
                .wrapping_sub(disp as u64)
                .wrapping_sub(iv.wrapping_mul(u64::from(scale)));
            b.set_full(base, bv);
            mem_op(GPR64[base], GPR64[index], scale, disp, size)
        }
        IndexDisp32 => {
            let index = pick_index(rng, None);
            let scale64 = u64::from(scale);
            let mut disp = rand_disp32(rng).min(i32::MAX as i64 - 8);
            disp += (ea.wrapping_sub(disp as u64) % scale64) as i64;
            let base_iv = ea.wrapping_sub(disp as u64) / scale64;
            let wrap = if scale == 1 {
                0
            } else {
                rng.below(scale64).wrapping_mul(u64::MAX / scale64 + 1)
            };
            b.set_full(index, base_iv.wrapping_add(wrap));
            mem_op(Register::None, GPR64[index], scale, disp, 8)
        }
        Abs => mem_op(Register::None, Register::None, 1, ea as i64, 8),
        Rip => mem_op(Register::RIP, Register::None, 1, ea as i64, 8),
    }
}

/// A memory operand of `size` bytes somewhere in the operand area, with the
/// bytes around it defined. Returns the operand and its effective address.
fn gen_mem(
    rng: &mut Rng,
    b: &mut Builder,
    size: u64,
    allow_rsp: bool,
) -> Result<(MemoryOperand, u64)> {
    let lo = DATA_ADDR + OPERAND_LO;
    let hi = DATA_ADDR + OPERAND_HI - size;
    let mut ea = lo + rng.below(hi - lo);
    if rng.chance(3, 4) {
        ea &= !(size - 1);
    }
    let mode = rng.pick(&ALL_MODES);
    let op = solve_addr(rng, b, mode, ea, allow_rsp);
    b.define(rng, ea - 8, size + 16)?;
    Ok((op, ea))
}

fn pick_rsp(rng: &mut Rng) -> u64 {
    let v = DATA_ADDR + STACK_LO + rng.below(STACK_HI - STACK_LO);
    if rng.chance(7, 8) { v & !7 } else { v }
}

/// Sets RSP for a stack form and defines the stack slots around it.
fn setup_stack(rng: &mut Rng, b: &mut Builder) -> Result<u64> {
    let rsp = pick_rsp(rng);
    b.set_full(RSP_INDEX, rsp);
    b.define(rng, rsp - 16, 32)?;
    Ok(rsp)
}

/// The 8-byte memory operand of push/pop/call m64. Sometimes RSP-relative,
/// in which case the effective address is not verified here (the recording
/// decides; push and call use RSP before the push, pop uses it after the pop).
fn stack_mem(rng: &mut Rng, b: &mut Builder, rsp: u64) -> Result<(MemoryOperand, Option<u64>)> {
    if rng.chance(1, 5) {
        let mut disp = rng.range_i64(-48, 48);
        if rng.chance(3, 4) {
            disp &= !7;
        }
        let at = rsp.wrapping_add(disp as u64);
        b.define(rng, at - 8, 32)?;
        return Ok((mem_op(Register::RSP, Register::None, 1, disp, 1), None));
    }
    let (op, ea) = gen_mem(rng, b, 8, false)?;
    Ok((op, Some(ea)))
}

// ---------------------------------------------------------------------------
// Branch targets

/// A canonical user-space target outside the instruction.
fn pick_target(rng: &mut Rng) -> u64 {
    loop {
        let t = match rng.below(6) {
            0 => rng.below(1 << 47),
            1 => CODE_ADDR + NO_TARGET_LEN + rng.below(0x2000),
            2 => rng.below(CODE_ADDR),
            3 => rng.pick(&[
                0,
                0x7fff_ffff_ffff,
                SENTINEL_RET,
                CODE_ADDR - 1,
                CODE_ADDR + 0x1000,
                DATA_ADDR,
            ]),
            _ => rng.below(1 << 32),
        };
        if !(CODE_ADDR..CODE_ADDR + NO_TARGET_LEN).contains(&t) {
            return t;
        }
    }
}

/// A relative branch target for an instruction of `len` bytes with a `rel8`
/// or `rel32` displacement: never inside the instruction, never negative.
fn pick_rel_target(rng: &mut Rng, len: u64, rel8: bool) -> u64 {
    let next = CODE_ADDR + len;
    loop {
        let disp = if rel8 {
            if rng.chance(1, 3) {
                rng.pick(&[0, 127, -128, -3, 1, 2])
            } else {
                rng.range_i64(-128, 127)
            }
        } else {
            match rng.below(4) {
                0 => rng.pick(&[
                    0,
                    i32::MAX as i64,
                    -(next as i64),
                    -(len as i64) - 1,
                    0x7f,
                    0x80,
                    -0x80,
                    -0x81,
                ]),
                1 => rng.range_i64(-0x1000, 0x1000),
                _ => rng.range_i64(-(next as i64), i32::MAX as i64),
            }
        };
        let target = next.wrapping_add(disp as u64);
        if target < (1 << 47) && !(CODE_ADDR..next).contains(&target) {
            return target;
        }
    }
}

/// Whether condition code `cc` (0 = O ... 15 = G) holds for `rflags`.
pub fn condition(cc: u8, rflags: u64) -> bool {
    let f = |bit: u64| rflags & bit != 0;
    let (cf, zf, sf, of, pf) = (
        f(flags::CF),
        f(flags::ZF),
        f(flags::SF),
        f(flags::OF),
        f(flags::PF),
    );
    let base = match cc >> 1 {
        0 => of,
        1 => cf,
        2 => zf,
        3 => cf || zf,
        4 => sf,
        5 => pf,
        6 => sf != of,
        _ => zf || sf != of,
    };
    base ^ (cc & 1 == 1)
}

// ---------------------------------------------------------------------------
// Forms

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Opnd {
    Reg(u32),
    Acc(u32),
    Mem(u32),
    Imm(u32),
}

impl Opnd {
    fn bits(self) -> u32 {
        match self {
            Opnd::Reg(b) | Opnd::Acc(b) | Opnd::Mem(b) | Opnd::Imm(b) => b,
        }
    }
}

#[derive(Clone, Debug)]
enum Kind {
    /// Two-operand mov/add/sub/cmp. One of `codes` is picked per case.
    Alu {
        codes: Vec<Code>,
        dst: Opnd,
        src: Opnd,
    },
    Lea {
        bits: u32,
        mode: AddrMode,
    },
    PushReg,
    PopReg,
    PushImm(Code, u32),
    PushMem,
    PopMem,
    Jcc {
        code: Code,
        cc: u8,
        rel8: bool,
    },
    JmpRel(bool),
    JmpReg,
    JmpMem,
    CallRel,
    CallReg,
    CallMem,
    Ret,
    RetImm,
    /// The generic generator (`generic.rs`).
    Generic(generic::Spec),
    /// The hand-written generators (`special.rs`).
    Special(special::Special),
}

struct Form {
    id: String,
    family: &'static str,
    count: usize,
    kind: Kind,
}

/// Codes of one ALU mnemonic, indexed by width (8, 16, 32, 64).
struct AluCodes {
    name: &'static str,
    rm_r: [Code; 4],
    r_rm: [Code; 4],
    /// 80 /n ib (8-bit) and 83 /n ib (16/32/64-bit).
    rm_imm8: [Code; 4],
    /// 81 /n iw/id (16/32/64-bit).
    rm_imm: [Code; 3],
    /// AL/AX/EAX/RAX short forms.
    acc: [Code; 4],
}

const ADD: AluCodes = AluCodes {
    name: "add",
    rm_r: [
        Code::Add_rm8_r8,
        Code::Add_rm16_r16,
        Code::Add_rm32_r32,
        Code::Add_rm64_r64,
    ],
    r_rm: [
        Code::Add_r8_rm8,
        Code::Add_r16_rm16,
        Code::Add_r32_rm32,
        Code::Add_r64_rm64,
    ],
    rm_imm8: [
        Code::Add_rm8_imm8,
        Code::Add_rm16_imm8,
        Code::Add_rm32_imm8,
        Code::Add_rm64_imm8,
    ],
    rm_imm: [
        Code::Add_rm16_imm16,
        Code::Add_rm32_imm32,
        Code::Add_rm64_imm32,
    ],
    acc: [
        Code::Add_AL_imm8,
        Code::Add_AX_imm16,
        Code::Add_EAX_imm32,
        Code::Add_RAX_imm32,
    ],
};

const SUB: AluCodes = AluCodes {
    name: "sub",
    rm_r: [
        Code::Sub_rm8_r8,
        Code::Sub_rm16_r16,
        Code::Sub_rm32_r32,
        Code::Sub_rm64_r64,
    ],
    r_rm: [
        Code::Sub_r8_rm8,
        Code::Sub_r16_rm16,
        Code::Sub_r32_rm32,
        Code::Sub_r64_rm64,
    ],
    rm_imm8: [
        Code::Sub_rm8_imm8,
        Code::Sub_rm16_imm8,
        Code::Sub_rm32_imm8,
        Code::Sub_rm64_imm8,
    ],
    rm_imm: [
        Code::Sub_rm16_imm16,
        Code::Sub_rm32_imm32,
        Code::Sub_rm64_imm32,
    ],
    acc: [
        Code::Sub_AL_imm8,
        Code::Sub_AX_imm16,
        Code::Sub_EAX_imm32,
        Code::Sub_RAX_imm32,
    ],
};

const CMP: AluCodes = AluCodes {
    name: "cmp",
    rm_r: [
        Code::Cmp_rm8_r8,
        Code::Cmp_rm16_r16,
        Code::Cmp_rm32_r32,
        Code::Cmp_rm64_r64,
    ],
    r_rm: [
        Code::Cmp_r8_rm8,
        Code::Cmp_r16_rm16,
        Code::Cmp_r32_rm32,
        Code::Cmp_r64_rm64,
    ],
    rm_imm8: [
        Code::Cmp_rm8_imm8,
        Code::Cmp_rm16_imm8,
        Code::Cmp_rm32_imm8,
        Code::Cmp_rm64_imm8,
    ],
    rm_imm: [
        Code::Cmp_rm16_imm16,
        Code::Cmp_rm32_imm32,
        Code::Cmp_rm64_imm32,
    ],
    acc: [
        Code::Cmp_AL_imm8,
        Code::Cmp_AX_imm16,
        Code::Cmp_EAX_imm32,
        Code::Cmp_RAX_imm32,
    ],
};

const WIDTHS: [u32; 4] = [8, 16, 32, 64];

const MOV_COUNT: usize = 48;
const ALU_COUNT: usize = 40;
const LEA_COUNT: usize = 32;
const STACK_COUNT: usize = 48;
const JCC_COUNT: usize = 24;

fn alu(
    id: String,
    family: &'static str,
    count: usize,
    codes: &[Code],
    dst: Opnd,
    src: Opnd,
) -> Form {
    Form {
        id,
        family,
        count,
        kind: Kind::Alu {
            codes: codes.to_vec(),
            dst,
            src,
        },
    }
}

fn forms() -> Vec<Form> {
    use Opnd::*;
    let mut out = Vec::new();

    // mov
    let mov_rm_r = [
        Code::Mov_rm8_r8,
        Code::Mov_rm16_r16,
        Code::Mov_rm32_r32,
        Code::Mov_rm64_r64,
    ];
    let mov_r_rm = [
        Code::Mov_r8_rm8,
        Code::Mov_r16_rm16,
        Code::Mov_r32_rm32,
        Code::Mov_r64_rm64,
    ];
    let mov_rm_imm = [
        Code::Mov_rm8_imm8,
        Code::Mov_rm16_imm16,
        Code::Mov_rm32_imm32,
        Code::Mov_rm64_imm32,
    ];
    let n = MOV_COUNT;
    for (i, w) in WIDTHS.into_iter().enumerate() {
        out.push(alu(
            format!("mov_r{w}_r{w}"),
            "mov",
            n,
            &[mov_rm_r[i], mov_r_rm[i]],
            Reg(w),
            Reg(w),
        ));
    }
    out.push(alu(
        "mov_r64_imm32".into(),
        "mov",
        n,
        &[Code::Mov_rm64_imm32],
        Reg(64),
        Imm(32),
    ));
    out.push(alu(
        "mov_r64_imm64".into(),
        "mov",
        n,
        &[Code::Mov_r64_imm64],
        Reg(64),
        Imm(64),
    ));
    out.push(alu(
        "mov_r32_imm32".into(),
        "mov",
        n,
        &[Code::Mov_r32_imm32, Code::Mov_rm32_imm32],
        Reg(32),
        Imm(32),
    ));
    out.push(alu(
        "mov_r16_imm16".into(),
        "mov",
        n,
        &[Code::Mov_r16_imm16, Code::Mov_rm16_imm16],
        Reg(16),
        Imm(16),
    ));
    out.push(alu(
        "mov_r8_imm8".into(),
        "mov",
        n,
        &[Code::Mov_r8_imm8, Code::Mov_rm8_imm8],
        Reg(8),
        Imm(8),
    ));
    for (i, w) in WIDTHS.into_iter().enumerate() {
        out.push(alu(
            format!("mov_r{w}_m{w}"),
            "mov",
            n,
            &[mov_r_rm[i]],
            Reg(w),
            Mem(w),
        ));
    }
    for (i, w) in WIDTHS.into_iter().enumerate() {
        out.push(alu(
            format!("mov_m{w}_r{w}"),
            "mov",
            n,
            &[mov_rm_r[i]],
            Mem(w),
            Reg(w),
        ));
    }
    for (i, w) in WIDTHS.into_iter().enumerate() {
        let iw = w.min(32);
        out.push(alu(
            format!("mov_m{w}_imm{iw}"),
            "mov",
            n,
            &[mov_rm_imm[i]],
            Mem(w),
            Imm(iw),
        ));
    }

    // add, sub, cmp
    for c in [&ADD, &SUB, &CMP] {
        let (m, n) = (c.name, ALU_COUNT);
        for (i, w) in WIDTHS.into_iter().enumerate() {
            out.push(alu(
                format!("{m}_r{w}_r{w}"),
                m,
                n,
                &[c.rm_r[i], c.r_rm[i]],
                Reg(w),
                Reg(w),
            ));
        }
        for (i, w) in WIDTHS.into_iter().enumerate() {
            out.push(alu(
                format!("{m}_r{w}_imm8"),
                m,
                n,
                &[c.rm_imm8[i]],
                Reg(w),
                Imm(8),
            ));
        }
        for (i, w) in [16u32, 32, 64].into_iter().enumerate() {
            let iw = w.min(32);
            out.push(alu(
                format!("{m}_r{w}_imm{iw}"),
                m,
                n,
                &[c.rm_imm[i]],
                Reg(w),
                Imm(iw),
            ));
        }
        for (i, w) in WIDTHS.into_iter().enumerate() {
            out.push(alu(
                format!("{m}_r{w}_m{w}"),
                m,
                n,
                &[c.r_rm[i]],
                Reg(w),
                Mem(w),
            ));
        }
        for (i, w) in WIDTHS.into_iter().enumerate() {
            out.push(alu(
                format!("{m}_m{w}_r{w}"),
                m,
                n,
                &[c.rm_r[i]],
                Mem(w),
                Reg(w),
            ));
        }
        for (i, w) in WIDTHS.into_iter().enumerate() {
            out.push(alu(
                format!("{m}_m{w}_imm8"),
                m,
                n,
                &[c.rm_imm8[i]],
                Mem(w),
                Imm(8),
            ));
        }
        for (i, w) in [16u32, 32, 64].into_iter().enumerate() {
            let iw = w.min(32);
            out.push(alu(
                format!("{m}_m{w}_imm{iw}"),
                m,
                n,
                &[c.rm_imm[i]],
                Mem(w),
                Imm(iw),
            ));
        }
        for (i, (w, r)) in [(8u32, "al"), (16, "ax"), (32, "eax"), (64, "rax")]
            .into_iter()
            .enumerate()
        {
            let iw = w.min(32);
            out.push(alu(
                format!("{m}_{r}_imm{iw}"),
                m,
                n,
                &[c.acc[i]],
                Acc(w),
                Imm(iw),
            ));
        }
    }

    // lea
    for bits in [64u32, 32] {
        for (name, mode) in [
            ("b", AddrMode::Base),
            ("bd8", AddrMode::BaseDisp8),
            ("bd32", AddrMode::BaseDisp32),
            ("bi", AddrMode::BaseIndex),
            ("bid8", AddrMode::BaseIndexDisp8),
            ("bid32", AddrMode::BaseIndexDisp32),
            ("id32", AddrMode::IndexDisp32),
            ("abs", AddrMode::Abs),
            ("rip", AddrMode::Rip),
        ] {
            out.push(Form {
                id: format!("lea_r{bits}_{name}"),
                family: "lea",
                count: LEA_COUNT,
                kind: Kind::Lea { bits, mode },
            });
        }
    }

    // push, pop
    let stack = |id: &str, family: &'static str, kind: Kind| Form {
        id: id.to_string(),
        family,
        count: STACK_COUNT,
        kind,
    };
    out.push(stack("push_r64", "push", Kind::PushReg));
    out.push(stack(
        "push_imm8",
        "push",
        Kind::PushImm(Code::Pushq_imm8, 8),
    ));
    out.push(stack(
        "push_imm32",
        "push",
        Kind::PushImm(Code::Pushq_imm32, 32),
    ));
    out.push(stack("push_m64", "push", Kind::PushMem));
    out.push(stack("pop_r64", "pop", Kind::PopReg));
    out.push(stack("pop_m64", "pop", Kind::PopMem));

    // jcc
    let jcc = [
        ("jo", Code::Jo_rel8_64, Code::Jo_rel32_64),
        ("jno", Code::Jno_rel8_64, Code::Jno_rel32_64),
        ("jb", Code::Jb_rel8_64, Code::Jb_rel32_64),
        ("jae", Code::Jae_rel8_64, Code::Jae_rel32_64),
        ("je", Code::Je_rel8_64, Code::Je_rel32_64),
        ("jne", Code::Jne_rel8_64, Code::Jne_rel32_64),
        ("jbe", Code::Jbe_rel8_64, Code::Jbe_rel32_64),
        ("ja", Code::Ja_rel8_64, Code::Ja_rel32_64),
        ("js", Code::Js_rel8_64, Code::Js_rel32_64),
        ("jns", Code::Jns_rel8_64, Code::Jns_rel32_64),
        ("jp", Code::Jp_rel8_64, Code::Jp_rel32_64),
        ("jnp", Code::Jnp_rel8_64, Code::Jnp_rel32_64),
        ("jl", Code::Jl_rel8_64, Code::Jl_rel32_64),
        ("jge", Code::Jge_rel8_64, Code::Jge_rel32_64),
        ("jle", Code::Jle_rel8_64, Code::Jle_rel32_64),
        ("jg", Code::Jg_rel8_64, Code::Jg_rel32_64),
    ];
    for (cc, (name, rel8, rel32)) in jcc.into_iter().enumerate() {
        for (suffix, code, is8) in [("rel8", rel8, true), ("rel32", rel32, false)] {
            out.push(Form {
                id: format!("{name}_{suffix}"),
                family: "jcc",
                count: JCC_COUNT,
                kind: Kind::Jcc {
                    code,
                    cc: cc as u8,
                    rel8: is8,
                },
            });
        }
    }

    // jmp
    out.push(stack("jmp_rel8", "jmp", Kind::JmpRel(true)));
    out.push(stack("jmp_rel32", "jmp", Kind::JmpRel(false)));
    out.push(stack("jmp_r64", "jmp", Kind::JmpReg));
    out.push(stack("jmp_m64", "jmp", Kind::JmpMem));

    // call, ret
    out.push(stack("call_rel32", "call", Kind::CallRel));
    out.push(stack("call_r64", "call", Kind::CallReg));
    out.push(stack("call_m64", "call", Kind::CallMem));
    out.push(stack("ret", "ret", Kind::Ret));
    out.push(stack("ret_imm16", "ret", Kind::RetImm));

    // M3: the generic forms, with the special ones at their place.
    generic::forms(&mut out, &mut special::after);
    out
}

// ---------------------------------------------------------------------------
// Generation

enum Op {
    Reg(Register),
    Mem(MemoryOperand),
    Imm(i64),
}

fn build2(code: Code, dst: &Op, src: &Op, imm64: bool) -> Result<Instruction> {
    let r = match (dst, src) {
        (Op::Reg(d), Op::Reg(s)) => Instruction::with2(code, *d, *s),
        (Op::Reg(d), Op::Mem(m)) => Instruction::with2(code, *d, *m),
        (Op::Mem(m), Op::Reg(s)) => Instruction::with2(code, *m, *s),
        (Op::Reg(d), Op::Imm(i)) if imm64 => Instruction::with2(code, *d, *i as u64),
        (Op::Reg(d), Op::Imm(i)) => Instruction::with2(code, *d, *i as i32),
        (Op::Mem(m), Op::Imm(i)) => Instruction::with2(code, *m, *i as i32),
        _ => bail!("unsupported operand combination for {code:?}"),
    };
    r.map_err(|e| anyhow::anyhow!("{code:?}: {e}"))
}

/// Encodes at `CODE_ADDR`. `None` if this operand combination has no encoding
/// (e.g. AH together with a REX prefix): the caller draws a new case.
fn encode(instr: &Instruction) -> Option<Vec<u8>> {
    let mut encoder = Encoder::new(64);
    encoder.encode(instr, CODE_ADDR).ok()?;
    Some(encoder.take_buffer())
}

fn formatter() -> IntelFormatter {
    let mut f = IntelFormatter::new();
    let o = f.options_mut();
    o.set_space_after_operand_separator(true);
    o.set_hex_prefix("0x");
    o.set_hex_suffix("");
    o.set_uppercase_hex(false);
    o.set_rip_relative_addresses(true);
    o.set_branch_leading_zeros(false);
    f
}

struct Finish<'a> {
    form: &'a Form,
    b: Builder,
    instr: Instruction,
    /// `None`: fall through to the next instruction.
    target: Option<u64>,
    /// Effective address the memory operand must resolve to, if checked.
    verify_ea: Option<u64>,
}

fn finish(f: Finish<'_>) -> Result<Option<GenCase>> {
    let Some(bytes) = encode(&f.instr) else {
        return Ok(None);
    };
    let decoded = Decoder::with_ip(64, &bytes, CODE_ADDR, DecoderOptions::NONE).decode();
    ensure!(
        !decoded.is_invalid() && decoded.len() == bytes.len() && decoded.code() == f.instr.code(),
        "{}: {:02x?} decodes to {:?} ({} bytes), expected {:?}",
        f.form.id,
        bytes,
        decoded.code(),
        decoded.len(),
        f.instr.code()
    );
    if let Some(ea) = f.verify_ea {
        let op = (0..decoded.op_count())
            .find(|&i| decoded.op_kind(i) == OpKind::Memory)
            .context("no memory operand")?;
        let gprs = f.b.gprs;
        let got = decoded.virtual_address(op, 0, |reg, _, _| {
            if reg.is_segment_register() {
                // Flat segments in 64-bit mode (no FS/GS overrides here).
                return Some(0);
            }
            GPR64.iter().position(|&r| r == reg).map(|i| gprs[i])
        });
        ensure!(
            got == Some(ea),
            "{}: {:02x?} addresses {got:x?}, planned {ea:#x}",
            f.form.id,
            bytes
        );
    }
    let mut asm = String::new();
    formatter().format(&decoded, &mut asm);
    let next = CODE_ADDR + bytes.len() as u64;
    Ok(Some(GenCase {
        form: f.form.id.clone(),
        asm,
        flags_mask: FLAGS_MASK,
        undefined: Undefined::NONE,
        input: f.b.input(),
        expect_rip: f.target.unwrap_or(next),
        expect_fault: None,
        bytes,
    }))
}

fn gen_alu(
    rng: &mut Rng,
    form: &Form,
    idx: usize,
    defaults: &[u64; 16],
    codes: &[Code],
    dst: Opnd,
    src: Opnd,
) -> Result<Option<GenCase>> {
    let code = rng.pick(codes);
    let bits = dst.bits();
    let mut b = Builder::new(rng, defaults);
    let (mut a, mut v) = pick_pair(rng, idx, bits);
    let mut imm = 0;
    if let Opnd::Imm(ib) = src {
        imm = match as_imm(v, bits, ib) {
            Some(i) if ib == 64 || idx < EDGE_PAIRS.len() || rng.chance(1, 2) => i,
            _ => rand_imm(rng, ib),
        };
        v = (imm as u64) & mask(bits);
    }
    if form.family == "mov" {
        // The old destination value only matters as something to overwrite:
        // keep it different from the source so a skipped write shows.
        a = rand_full(rng) & mask(bits);
        if a == v {
            a = !v & mask(bits);
        }
    }
    let legacy8 = (dst == Opnd::Reg(8) && src == Opnd::Reg(8)).then(|| rng.chance(1, 2));
    let d = match dst {
        Opnd::Reg(w) => {
            let r = pick_reg(rng, w, legacy8);
            b.set_operand(rng, r, a);
            Op::Reg(r)
        }
        Opnd::Acc(w) => {
            b.set_operand(rng, acc(w), a);
            Op::Reg(acc(w))
        }
        _ => Op::Imm(0),
    };
    let s = match src {
        Opnd::Reg(w) => {
            let r = match (&d, rng.chance(1, 8)) {
                (Op::Reg(dr), true) => *dr,
                _ => pick_reg(rng, w, legacy8),
            };
            b.set_operand(rng, r, v);
            Op::Reg(r)
        }
        Opnd::Imm(_) => Op::Imm(imm),
        _ => Op::Imm(0),
    };
    let mut verify_ea = None;
    let (d, s) = match (dst, src) {
        (Opnd::Mem(w), _) | (_, Opnd::Mem(w)) => {
            let size = u64::from(w / 8);
            let (m, ea) = gen_mem(rng, &mut b, size, true)?;
            let value = if matches!(dst, Opnd::Mem(_)) { a } else { v };
            b.put(ea, &value.to_le_bytes()[..size as usize])?;
            verify_ea = Some(ea);
            if matches!(dst, Opnd::Mem(_)) {
                (Op::Mem(m), s)
            } else {
                (d, Op::Mem(m))
            }
        }
        _ => (d, s),
    };
    let instr = build2(code, &d, &s, src == Opnd::Imm(64))?;
    finish(Finish {
        form,
        b,
        instr,
        target: None,
        verify_ea,
    })
}

/// A register value for LEA inputs: anything, wrap-around included.
fn rand_addr_val(rng: &mut Rng) -> u64 {
    match rng.below(4) {
        0 => rng.below(0x1_0000),
        1 => rand_val(rng, 64),
        _ => rng.next_u64(),
    }
}

fn gen_lea(
    rng: &mut Rng,
    form: &Form,
    defaults: &[u64; 16],
    bits: u32,
    mode: AddrMode,
) -> Result<Option<GenCase>> {
    use AddrMode::*;
    let code = if bits == 64 {
        Code::Lea_r64_m
    } else {
        Code::Lea_r32_m
    };
    let mut b = Builder::new(rng, defaults);
    let dst = pick_reg(rng, bits, None);
    let dst_val = rand_val(rng, bits);
    b.set_operand(rng, dst, dst_val);
    let scale = rng.pick(&[1u32, 2, 4, 8]);
    let (has_base, has_index) = match mode {
        Base | BaseDisp8 | BaseDisp32 => (true, false),
        BaseIndex | BaseIndexDisp8 | BaseIndexDisp32 => (true, true),
        IndexDisp32 => (false, true),
        Abs | Rip => (false, false),
    };
    let (disp, displ_size) = match mode {
        Base | BaseIndex => (0, 0),
        BaseDisp8 | BaseIndexDisp8 => (rand_disp8(rng), 1),
        _ => (rand_disp32(rng), 8),
    };
    let base = if has_base {
        let i = pick_base(rng, true);
        let value = rand_addr_val(rng);
        b.set_full(i, value);
        GPR64[i]
    } else {
        Register::None
    };
    let index = if has_index {
        let i = pick_index(rng, None);
        if Some(i) != GPR64.iter().position(|&r| r == base) || rng.chance(1, 4) {
            let value = rand_addr_val(rng);
            b.set_full(i, value);
        }
        GPR64[i]
    } else {
        Register::None
    };
    let instr = if mode == Rip {
        // Two passes: the length (and so the next RIP) does not depend on the
        // target, the displacement does.
        let probe = Instruction::with2(
            code,
            dst,
            mem_op(Register::RIP, Register::None, 1, CODE_ADDR as i64, 8),
        )?;
        let Some(len) = encode(&probe).map(|b| b.len() as u64) else {
            return Ok(None);
        };
        let target = (CODE_ADDR + len).wrapping_add(disp as u64);
        Instruction::with2(
            code,
            dst,
            mem_op(Register::RIP, Register::None, 1, target as i64, 8),
        )?
    } else {
        let scale = if has_index { scale } else { 1 };
        Instruction::with2(code, dst, mem_op(base, index, scale, disp, displ_size))?
    };
    finish(Finish {
        form,
        b,
        instr,
        target: None,
        verify_ea: None,
    })
}

fn gen_case(
    rng: &mut Rng,
    form: &Form,
    idx: usize,
    defaults: &[u64; 16],
) -> Result<Option<GenCase>> {
    let mut b = Builder::new(rng, defaults);
    let plain = |form, b, instr, target, verify_ea| {
        finish(Finish {
            form,
            b,
            instr,
            target,
            verify_ea,
        })
    };
    match &form.kind {
        Kind::Generic(spec) => return generic::case(rng, form, idx, defaults, *spec),
        Kind::Special(s) => return special::case(rng, form, idx, defaults, *s),
        _ => {}
    }
    match &form.kind {
        Kind::Generic(_) | Kind::Special(_) => unreachable!(),
        Kind::Alu { codes, dst, src } => gen_alu(rng, form, idx, defaults, codes, *dst, *src),
        Kind::Lea { bits, mode } => gen_lea(rng, form, defaults, *bits, *mode),
        Kind::PushReg | Kind::PopReg => {
            let rsp = setup_stack(rng, &mut b)?;
            let reg = rng.below(16) as usize;
            if reg != RSP_INDEX {
                let value = rand_val(rng, 64);
                b.set_full(reg, value);
            }
            let code = if matches!(form.kind, Kind::PushReg) {
                Code::Push_r64
            } else {
                let value = rand_val(rng, 64);
                b.put(rsp, &value.to_le_bytes())?;
                Code::Pop_r64
            };
            plain(form, b, Instruction::with1(code, GPR64[reg])?, None, None)
        }
        Kind::PushImm(code, imm_bits) => {
            setup_stack(rng, &mut b)?;
            let imm = rand_imm(rng, *imm_bits);
            plain(form, b, Instruction::with1(*code, imm as i32)?, None, None)
        }
        Kind::PushMem | Kind::PopMem => {
            let rsp = setup_stack(rng, &mut b)?;
            let (m, ea) = stack_mem(rng, &mut b, rsp)?;
            let pop = matches!(form.kind, Kind::PopMem);
            if let Some(ea) = ea {
                let value = rand_val(rng, 64);
                b.put(ea, &value.to_le_bytes())?;
            }
            if pop {
                let value = rand_val(rng, 64);
                b.put(rsp, &value.to_le_bytes())?;
            }
            let code = if pop { Code::Pop_rm64 } else { Code::Push_rm64 };
            plain(form, b, Instruction::with1(code, m)?, None, ea)
        }
        Kind::Jcc { code, cc, rel8 } => {
            let taken = idx.is_multiple_of(2);
            let mut tries = 0;
            while condition(*cc, b.rflags) != taken {
                b.rflags = random_flags(rng);
                tries += 1;
                ensure!(tries < 10_000, "no flags make {} {}", form.id, taken);
            }
            let len = if *rel8 { 2 } else { 6 };
            let target = pick_rel_target(rng, len, *rel8);
            let instr = Instruction::with_branch(*code, target)?;
            let expect = if taken { target } else { CODE_ADDR + len };
            plain(form, b, instr, Some(expect), None)
        }
        Kind::JmpRel(rel8) => {
            let (code, len) = if *rel8 {
                (Code::Jmp_rel8_64, 2)
            } else {
                (Code::Jmp_rel32_64, 5)
            };
            let target = pick_rel_target(rng, len, *rel8);
            let instr = Instruction::with_branch(code, target)?;
            plain(form, b, instr, Some(target), None)
        }
        Kind::JmpReg => {
            let rsp = setup_stack(rng, &mut b)?;
            let reg = rng.below(16) as usize;
            let target = if reg == RSP_INDEX {
                rsp
            } else {
                let t = pick_target(rng);
                b.set_full(reg, t);
                t
            };
            let instr = Instruction::with1(Code::Jmp_rm64, GPR64[reg])?;
            plain(form, b, instr, Some(target), None)
        }
        Kind::JmpMem => {
            let rsp = setup_stack(rng, &mut b)?;
            let (m, ea) = stack_mem(rng, &mut b, rsp)?;
            let target = pick_target(rng);
            match ea {
                Some(ea) => b.put(ea, &target.to_le_bytes())?,
                None => b.put(
                    rsp.wrapping_add(m.displacement as u64),
                    &target.to_le_bytes(),
                )?,
            }
            let instr = Instruction::with1(Code::Jmp_rm64, m)?;
            plain(form, b, instr, Some(target), ea)
        }
        Kind::CallRel => {
            setup_stack(rng, &mut b)?;
            let target = pick_rel_target(rng, 5, false);
            let instr = Instruction::with_branch(Code::Call_rel32_64, target)?;
            plain(form, b, instr, Some(target), None)
        }
        Kind::CallReg => {
            let rsp = setup_stack(rng, &mut b)?;
            let reg = rng.below(16) as usize;
            let target = if reg == RSP_INDEX {
                rsp
            } else {
                let t = pick_target(rng);
                b.set_full(reg, t);
                t
            };
            let instr = Instruction::with1(Code::Call_rm64, GPR64[reg])?;
            plain(form, b, instr, Some(target), None)
        }
        Kind::CallMem => {
            let rsp = setup_stack(rng, &mut b)?;
            let (m, ea) = stack_mem(rng, &mut b, rsp)?;
            let target = pick_target(rng);
            match ea {
                Some(ea) => b.put(ea, &target.to_le_bytes())?,
                // RSP-relative: the operand is read before the push.
                None => b.put(
                    rsp.wrapping_add(m.displacement as u64),
                    &target.to_le_bytes(),
                )?,
            }
            let instr = Instruction::with1(Code::Call_rm64, m)?;
            plain(form, b, instr, Some(target), ea)
        }
        Kind::Ret | Kind::RetImm => {
            let rsp = setup_stack(rng, &mut b)?;
            let target = pick_target(rng);
            b.put(rsp, &target.to_le_bytes())?;
            let instr = if matches!(form.kind, Kind::Ret) {
                Instruction::with(Code::Retnq)
            } else {
                let imm = if rng.chance(1, 2) {
                    rng.pick(&[0u32, 8, 1, 0x7fff, 0x8000, 0xffff, 0xfff8])
                } else {
                    rng.below(0x1_0000) as u32
                };
                Instruction::with1(Code::Retnq_imm16, imm)?
            };
            plain(form, b, instr, Some(target), None)
        }
    }
}

/// The M3 fixture groups, in commit order: each lands in one "Fixtures:
/// <group>" commit (`docs/design/fixtures.md`). The Phase 0 forms belong to
/// no group and are always recorded.
pub const GROUPS: [&str; 12] = [
    "alu", "ext", "cc", "shift", "muldiv", "bit", "xchg", "string", "loop", "misc", "sse", "fninit",
];

/// How many of `GROUPS` `testkit record` records: the groups whose fixtures
/// are committed. The others are generated by the tests only.
pub const RECORDED_GROUPS: usize = 7;

/// The group of an M3 form, by family; `None` for the Phase 0 forms.
fn group(form: &Form) -> Option<usize> {
    if !matches!(form.kind, Kind::Generic(_) | Kind::Special(_)) {
        return None;
    }
    let name = match form.family {
        "adc" | "sbb" | "and" | "or" | "xor" | "test" | "inc" | "dec" | "neg" | "not" => "alu",
        "movzx" | "movsx" | "movsxd" | "cbw" | "cwd" => "ext",
        "setcc" | "cmovcc" => "cc",
        "shl" | "shr" | "sar" | "rol" | "ror" | "rcl" | "rcr" | "shld" | "shrd" => "shift",
        "mul" | "imul" | "div" | "idiv" => "muldiv",
        "bt" | "bts" | "btr" | "btc" | "bsf" | "bsr" | "bswap" => "bit",
        "xchg" | "xadd" | "cmpxchg" => "xchg",
        "movs" | "stos" | "lods" | "cmps" | "scas" => "string",
        "loop" => "loop",
        // The census adds mov moffs and push r/m64 with a register to the
        // Phase 0 families.
        "flagop" | "ud2" | "nop" | "mov" | "push" => "misc",
        f if f.starts_with("sse_") => "sse",
        "fninit" => "fninit",
        f => panic!("{}: family {f} has no group", form.id),
    };
    GROUPS.iter().position(|&g| g == name)
}

/// Generates every case of the recorded families, in file order.
pub fn generate() -> Result<Vec<Family>> {
    generate_groups(RECORDED_GROUPS)
}

/// Generates every case of the Phase 0 forms and those of the first
/// `groups` of `GROUPS`, in file order: families in the order their first
/// form appears (the Phase 0 families first).
pub fn generate_groups(groups: usize) -> Result<Vec<Family>> {
    let defaults = default_gprs();
    let mut families: Vec<Family> = Vec::new();
    for form in forms() {
        if group(&form).is_some_and(|g| g >= groups) {
            continue;
        }
        if !families.iter().any(|f| f.name == form.family) {
            families.push(Family {
                name: form.family,
                cases: Vec::new(),
            });
        }
        let family = families
            .iter_mut()
            .find(|f| f.name == form.family)
            .expect("family just added");
        let mut rng = Rng::for_stream(SEED, &form.id);
        for idx in 0..form.count {
            let mut attempts = 0;
            let case = loop {
                if let Some(case) = gen_case(&mut rng, &form, idx, &defaults)
                    .with_context(|| format!("{} case {idx}", form.id))?
                {
                    break case;
                }
                attempts += 1;
                ensure!(
                    attempts < 100,
                    "{} case {idx}: no encodable operands",
                    form.id
                );
            };
            family.cases.push(case);
        }
    }
    Ok(families)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic() {
        let a = generate().unwrap();
        let b = generate().unwrap();
        for (fa, fb) in a.iter().zip(&b) {
            assert_eq!(fa.cases.len(), fb.cases.len());
            for (ca, cb) in fa.cases.iter().zip(&fb.cases) {
                assert_eq!(ca.bytes, cb.bytes);
                assert_eq!(ca.input, cb.input);
            }
        }
    }

    #[test]
    fn every_group_generates() {
        let families = generate_groups(GROUPS.len()).unwrap();
        let recorded = generate().unwrap();
        assert!(families.len() > recorded.len() || RECORDED_GROUPS == GROUPS.len());
        for form in forms() {
            assert!(form.count >= 32 || group(&form).is_none(), "{}", form.id);
        }
    }

    /// Every form of the M3 scope list has a generator.
    #[test]
    fn m3_scope_is_generated() {
        let ids: std::collections::BTreeSet<String> = forms()
            .iter()
            .filter(|f| group(f).is_some())
            // String forms carry their repeat prefix after a space.
            .map(|f| f.id.split(' ').next().unwrap().to_string())
            .collect();
        let missing: Vec<&str> = include_str!("forms/m3_scope.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .filter(|l| !ids.contains(*l))
            .collect();
        assert!(missing.is_empty(), "no generator for {missing:?}");
    }

    #[test]
    fn every_jcc_form_is_taken_and_not_taken() {
        let families = generate().unwrap();
        let jcc = families.iter().find(|f| f.name == "jcc").unwrap();
        let mut forms: std::collections::BTreeMap<&str, (bool, bool)> = Default::default();
        for case in &jcc.cases {
            let next = CODE_ADDR + case.bytes.len() as u64;
            let e = forms.entry(&case.form).or_default();
            if case.expect_rip == next {
                e.1 = true;
            } else {
                e.0 = true;
            }
        }
        assert_eq!(forms.len(), 32);
        assert!(forms.values().all(|&(t, n)| t && n));
    }

    #[test]
    fn helpers() {
        assert_eq!(sext(0x80, 8), 0xffff_ffff_ffff_ff80);
        assert_eq!(as_imm(0xff, 8, 8), Some(-1));
        assert_eq!(as_imm(0xffff_ff80, 32, 8), Some(-128));
        assert_eq!(as_imm(0x80, 32, 8), None);
        assert!(condition(4, flags::ZF));
        assert!(!condition(5, flags::ZF));
        assert!(condition(12, flags::SF));
        assert!(!condition(12, flags::SF | flags::OF));
        assert_eq!(slot(Register::AH), (0, 8, 8));
        assert_eq!(slot(Register::R15L), (15, 0, 8));
    }
}
