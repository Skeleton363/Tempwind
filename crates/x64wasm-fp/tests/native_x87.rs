//! Differential tests of the x87 functions against the CPU. Each case runs
//! fninit, fldcw, loads the operands (fld m80 copies any encoding without an
//! exception), clears the exception flags and condition codes through
//! fnstenv/fldenv, runs the instruction, reads FSW with fnstsw, stores the
//! result and leaves the stack empty, restoring the caller's FCW. All
//! exceptions are masked; `unmasked.rs` covers ES.
#![cfg(target_arch = "x86_64")]

mod common;

use common::*;
use core::arch::asm;
use x64wasm_fp as fp;

/// Random cases per function and operand form in the default run.
const RANDOM: usize = 20_000;

#[repr(C)]
struct Scratch {
    env: [u8; 28],
    fsw: u16,
    save: u16,
    fcw: u16,
    preset: u16,
}

/// (a, b, fcw, condition-code preset) -> (16 bytes at the output, FSW).
type Hw = fn(&[u64; 2], &[u64; 2], u32, u16) -> ([u64; 2], u16);

macro_rules! x87 {
    ($name:ident, $pre:literal, $op:literal, $post:literal) => {
        fn $name(a: &[u64; 2], b: &[u64; 2], fcw: u32, preset: u16) -> ([u64; 2], u16) {
            let mut out = [0u64; 2];
            let mut s = Scratch { env: [0; 28], fsw: 0, save: 0, fcw: fcw as u16, preset };
            unsafe {
                asm!(
                    "fnstcw word ptr [rcx+30]",
                    "fninit",
                    "fldcw word ptr [rcx+32]",
                    $pre,
                    "fnstenv [rcx]",
                    "and word ptr [rcx+4], 0x3800",
                    "mov ax, word ptr [rcx+34]",
                    "or word ptr [rcx+4], ax",
                    "fldenv [rcx]",
                    $op,
                    "fnstsw word ptr [rcx+28]",
                    $post,
                    "fnclex",
                    "fldcw word ptr [rcx+30]",
                    in("rcx") &mut s, in("rsi") a.as_ptr(), in("rdi") b.as_ptr(), in("rdx") out.as_mut_ptr(),
                    out("rax") _,
                    out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                    out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                    options(nostack),
                );
            }
            (out, s.fsw)
        }
    };
}

const LD2: &str = "";
macro_rules! rr {
    ($name:ident, $op:literal) => {
        x87!(
            $name,
            "fld tbyte ptr [rdi]\nfld tbyte ptr [rsi]",
            $op,
            "fstp tbyte ptr [rdx]\nfstp st(0)"
        );
    };
}
macro_rules! rm {
    ($name:ident, $op:literal) => {
        x87!($name, "fld tbyte ptr [rsi]", $op, "fstp tbyte ptr [rdx]");
    };
}

// ST(0) = ST(0) op ST(1), as bytes (D8 /r with a register operand), so no
// assembler's reading of fsub/fsubr operand order matters.
rr!(hw_fadd, ".byte 0xD8, 0xC1");
rr!(hw_fmul, ".byte 0xD8, 0xC9");
rr!(hw_fsub, ".byte 0xD8, 0xE1");
rr!(hw_fsubr, ".byte 0xD8, 0xE9");
rr!(hw_fdiv, ".byte 0xD8, 0xF1");
rr!(hw_fdivr, ".byte 0xD8, 0xF9");
rm!(hw_fadd_m32, "fadd dword ptr [rdi]");
rm!(hw_fmul_m32, "fmul dword ptr [rdi]");
rm!(hw_fsub_m32, "fsub dword ptr [rdi]");
rm!(hw_fsubr_m32, "fsubr dword ptr [rdi]");
rm!(hw_fdiv_m32, "fdiv dword ptr [rdi]");
rm!(hw_fdivr_m32, "fdivr dword ptr [rdi]");
rm!(hw_fadd_m64, "fadd qword ptr [rdi]");
rm!(hw_fmul_m64, "fmul qword ptr [rdi]");
rm!(hw_fsub_m64, "fsub qword ptr [rdi]");
rm!(hw_fsubr_m64, "fsubr qword ptr [rdi]");
rm!(hw_fdiv_m64, "fdiv qword ptr [rdi]");
rm!(hw_fdivr_m64, "fdivr qword ptr [rdi]");
rm!(hw_fiadd_m16, "fiadd word ptr [rdi]");
rm!(hw_fimul_m16, "fimul word ptr [rdi]");
rm!(hw_fisub_m16, "fisub word ptr [rdi]");
rm!(hw_fisubr_m16, "fisubr word ptr [rdi]");
rm!(hw_fidiv_m16, "fidiv word ptr [rdi]");
rm!(hw_fidivr_m16, "fidivr word ptr [rdi]");
rm!(hw_fiadd_m32, "fiadd dword ptr [rdi]");
rm!(hw_fimul_m32, "fimul dword ptr [rdi]");
rm!(hw_fisub_m32, "fisub dword ptr [rdi]");
rm!(hw_fisubr_m32, "fisubr dword ptr [rdi]");
rm!(hw_fidiv_m32, "fidiv dword ptr [rdi]");
rm!(hw_fidivr_m32, "fidivr dword ptr [rdi]");
rm!(hw_fsqrt, ".byte 0xD9, 0xFA");
rm!(hw_frndint, ".byte 0xD9, 0xFC");
rr!(hw_fscale, ".byte 0xD9, 0xFD");
rr!(hw_fprem, ".byte 0xD9, 0xF8");
rr!(hw_fprem1, ".byte 0xD9, 0xF5");
x87!(
    hw_fld_m32,
    "",
    "fld dword ptr [rdi]",
    "fstp tbyte ptr [rdx]"
);
x87!(
    hw_fld_m64,
    "",
    "fld qword ptr [rdi]",
    "fstp tbyte ptr [rdx]"
);
x87!(
    hw_fild_m16,
    "",
    "fild word ptr [rdi]",
    "fstp tbyte ptr [rdx]"
);
x87!(
    hw_fild_m32,
    "",
    "fild dword ptr [rdi]",
    "fstp tbyte ptr [rdx]"
);
x87!(
    hw_fild_m64,
    "",
    "fild qword ptr [rdi]",
    "fstp tbyte ptr [rdx]"
);
x87!(
    hw_fstp_m32,
    "fld tbyte ptr [rsi]",
    "fstp dword ptr [rdx]",
    ""
);
x87!(
    hw_fstp_m64,
    "fld tbyte ptr [rsi]",
    "fstp qword ptr [rdx]",
    ""
);
x87!(
    hw_fistp_m16,
    "fld tbyte ptr [rsi]",
    "fistp word ptr [rdx]",
    ""
);
x87!(
    hw_fistp_m32,
    "fld tbyte ptr [rsi]",
    "fistp dword ptr [rdx]",
    ""
);
x87!(
    hw_fistp_m64,
    "fld tbyte ptr [rsi]",
    "fistp qword ptr [rdx]",
    ""
);
x87!(
    hw_fcom,
    "fld tbyte ptr [rdi]\nfld tbyte ptr [rsi]",
    ".byte 0xD8, 0xD1",
    "fstp st(0)\nfstp st(0)"
);
x87!(
    hw_fucom,
    "fld tbyte ptr [rdi]\nfld tbyte ptr [rsi]",
    ".byte 0xDD, 0xE1",
    "fstp st(0)\nfstp st(0)"
);
x87!(
    hw_fcom_m32,
    "fld tbyte ptr [rsi]",
    "fcom dword ptr [rdi]",
    "fstp st(0)"
);
x87!(
    hw_fcom_m64,
    "fld tbyte ptr [rsi]",
    "fcom qword ptr [rdi]",
    "fstp st(0)"
);
x87!(
    hw_ficom_m16,
    "fld tbyte ptr [rsi]",
    "ficom word ptr [rdi]",
    "fstp st(0)"
);
x87!(
    hw_ficom_m32,
    "fld tbyte ptr [rsi]",
    "ficom dword ptr [rdi]",
    "fstp st(0)"
);

/// FSW bits compared: the flags, SF, ES and C1 for arithmetic; all four
/// condition codes for compares and fprem.
const ARITH: u32 = 0x02FF;
const CC_ALL: u32 = 0x47FF;

/// An operand: its bytes in memory and its value for the soft float.
#[derive(Clone, Copy, Debug)]
struct Opnd {
    kind: u32,
    m: u64,
    se: u32,
}
impl Opnd {
    fn mem(&self) -> [u64; 2] {
        match self.kind {
            0 => [self.m, self.se as u64],
            1 => [self.m & 0xFFFF_FFFF, 0],
            3 => [self.m & 0xFFFF, 0],
            4 => [self.m & 0xFFFF_FFFF, 0],
            _ => [self.m, 0],
        }
    }
}
fn f80(v: (u64, u32)) -> Opnd {
    Opnd {
        kind: 0,
        m: v.0,
        se: v.1,
    }
}
fn of_kind(kind: u32, bits: u64) -> Opnd {
    let m = match kind {
        3 => bits as i16 as i64 as u64,
        4 => bits as i32 as i64 as u64,
        _ => bits,
    };
    Opnd { kind, m, se: 0 }
}

/// What the soft float returns: (result value, status).
type Soft<'a> = &'a dyn Fn(Opnd, Opnd, u32) -> (u64, u32);
/// How to read the CPU's output: (result value, se or 0) from the 16 bytes.
#[derive(Clone, Copy)]
enum Out {
    F80,
    Cmp,
    Store(u32),
}

struct Group<'a> {
    name: &'a str,
    soft: Soft<'a>,
    hw: Hw,
    out: Out,
    mask: u32,
}

/// The host CPU's vendor string (cpuid leaf 0).
fn host_vendor() -> String {
    #[allow(unused_unsafe)]
    let r = unsafe { core::arch::x86_64::__cpuid(0) };
    let b: Vec<u8> = [r.ebx, r.edx, r.ecx]
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .collect();
    String::from_utf8_lossy(&b).into_owned()
}

/// Whether the cases the SDM leaves to the implementation are compared bit
/// for bit: only on the vendor the model follows (GenuineIntel). FP_VENDOR
/// overrides the host's string, to run the other path on an Intel host.
fn exact_host() -> bool {
    static EXACT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *EXACT.get_or_init(|| {
        let host = host_vendor();
        let v = std::env::var("FP_VENDOR")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| host.clone());
        let exact = v == "GenuineIntel";
        eprintln!(
            "host CPU vendor: {host}{}; implementation-dependent x87 cases {}",
            if v != host {
                format!(" (treated as {v})")
            } else {
                String::new()
            },
            if exact {
                "compared bit for bit"
            } else {
                "checked against the SDM's guarantees"
            }
        );
        exact
    })
}

/// Cases whose result the SDM leaves to the implementation.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Dep {
    /// fprem/fprem1 with an exponent difference of 64 or more: the
    /// reduction N is "an implementation-dependent number between 32 and 63".
    PartialRem,
    /// Two NaNs of the same kind with equal significands and different
    /// signs: table 4-7 picks the larger significand, which doesn't decide.
    NanTie,
}

fn is_nan80(o: &Opnd) -> bool {
    o.kind == 0 && o.se & 0x7FFF == 0x7FFF && o.m >> 63 == 1 && o.m << 1 != 0
}

fn impl_dependent(name: &str, a: &Opnd, b: &Opnd, soft_status: u32) -> Option<Dep> {
    // Only functions with an 80-bit result return a NaN operand.
    let nan_result = !name.starts_with("f80_cmp");
    if nan_result
        && is_nan80(a)
        && is_nan80(b)
        && a.m << 1 == b.m << 1
        && (a.se ^ b.se) & 0x8000 != 0
    {
        return Some(Dep::NanTie);
    }
    if name.starts_with("f80_prem") && soft_status & 0x400 != 0 {
        return Some(Dep::PartialRem);
    }
    None
}

/// The SDM's partial remainder for a given N: ST(0) - ST(1) * QQ * 2^(D-N)
/// with QQ = trunc(ST(0) / ST(1) / 2^(D-N)), D the exponent difference.
/// None when that remainder needs bits below the denormal LSB (no
/// implementation can produce it).
fn partial_rem(a: &Opnd, b: &Opnd, n: u32) -> Option<(u64, u32)> {
    // (normalized significand, exponent of its bit 0)
    let unpack = |o: &Opnd| {
        let e = (o.se & 0x7FFF).max(1) as i32 - 16383 - 63;
        let lz = o.m.leading_zeros();
        (o.m << lz, e - lz as i32)
    };
    let ((ma, ea), (mb, eb)) = (unpack(a), unpack(b));
    let d = ea - eb;
    let sign = a.se & 0x8000;
    let r = ((ma as u128) << n) % mb as u128;
    if r == 0 {
        return Some((0, sign));
    }
    // Drop trailing zero bits, so an exact denormal isn't rejected.
    let tz = r.trailing_zeros();
    let r = r >> tz;
    let base = eb + d - n as i32 + tz as i32;
    let top = 127 - r.leading_zeros() as i32;
    let biased = base + top + 16383;
    if biased >= 1 {
        Some(((r << (63 - top)) as u64, sign | biased as u32))
    } else if base >= -16445 {
        Some(((r << (base + 16445)) as u64, sign))
    } else {
        None
    }
}

/// What the SDM guarantees for an implementation-dependent case: `soft` is
/// (result, status) of the model, `cpu` is (result, se, FSW).
fn sdm_ok(dep: Dep, a: &Opnd, b: &Opnd, soft: (u64, u32), cpu: (u64, u32, u32), mask: u32) -> bool {
    let (_, st) = soft;
    let (hr, hse, fsw) = cpu;
    match dep {
        Dep::NanTie => {
            // Either operand, quieted; flags and C1 as the model says (fprem
            // leaves C0 and C3 alone on a NaN result).
            let mask = mask & !0x4100;
            let q = |o: &Opnd| (o.m | 1 << 62, o.se);
            ((hr, hse) == q(a) || (hr, hse) == q(b)) && fsw & mask == st & mask
        }
        Dep::PartialRem => {
            // C2 = 1 with C0, C3, C1 clear; the same exception flags; the
            // sign of ST(0); and the remainder of some N in [32, 63].
            fsw & 0x4700 == 0x400
                && fsw & 0xFF == st & 0xFF
                && hse & 0x8000 == a.se & 0x8000
                && (32..=63).any(|n| partial_rem(a, b, n) == Some((hr, hse)))
        }
    }
}

fn run(g: &Group, cases: impl Iterator<Item = (Opnd, Opnd, u32)>, presets: &[u16]) -> usize {
    let mut n = 0;
    let mut nbad = 0;
    let mut bad = Vec::new();
    for (a, b, fcw) in cases {
        for &preset in presets {
            n += 1;
            let (r, st) = (g.soft)(a, b, fcw);
            let (out, fsw) = (g.hw)(&a.mem(), &b.mem(), fcw, preset);
            let (hr, hse) = match g.out {
                Out::F80 => (out[0], out[1] as u32 & 0xFFFF),
                Out::Cmp => ((fsw as u32 & 0x4500) as u64, 0),
                Out::Store(1) => (out[0] & 0xFFFF_FFFF, 0),
                Out::Store(3) => (out[0] as i16 as i64 as u64, 0),
                Out::Store(4) => (out[0] as i32 as i64 as u64, 0),
                Out::Store(_) => (out[0], 0),
            };
            // fprem leaves C0 and C3 unchanged (SDM: undefined) when its
            // result is a NaN.
            let nan = st >> 16 & 0x7FFF == 0x7FFF && r << 1 != 0;
            let mask = if g.name.starts_with("f80_prem") && nan {
                g.mask & !0x4100
            } else {
                g.mask
            };
            let soft = (r, st >> 16, st & mask);
            let cpu = (hr, hse, fsw as u32 & mask);
            let ok = match impl_dependent(g.name, &a, &b, st) {
                Some(dep) if !exact_host() => {
                    sdm_ok(dep, &a, &b, (r, st), (hr, hse, fsw as u32), g.mask)
                }
                _ => soft == cpu,
            };
            if !ok {
                nbad += 1;
                if bad.len() < 12 {
                    bad.push(format!(
                        "  {}(a {:#x}/{:#x}, b {:#x}/{:#x} kind {}, fcw {fcw:#x}, cc {preset:#x}): soft {soft:x?}, cpu {cpu:x?}",
                        g.name, a.m, a.se, b.m, b.se, b.kind
                    ));
                }
            }
        }
    }
    assert!(
        nbad == 0,
        "{}: {nbad} of {n} cases differ:\n{}",
        g.name,
        bad.join("\n")
    );
    n
}

fn all_fcw() -> Vec<u32> {
    (0..16).map(|i| fcw(i & 3, i >> 2)).collect()
}

fn e80() -> Vec<Opnd> {
    with_neg80(F80_EDGES).into_iter().map(f80).collect()
}
fn edges_of(kind: u32) -> Vec<Opnd> {
    match kind {
        0 => e80(),
        1 => with_neg32(F32_EDGES)
            .into_iter()
            .map(|x| of_kind(1, x as u64))
            .collect(),
        2 => with_neg64(F64_EDGES)
            .into_iter()
            .map(|x| of_kind(2, x))
            .collect(),
        3 => [0i64, 1, -1, 2, 3, 0x7FFF, -0x8000, 0x7FFE, 0x5555, 0x100]
            .iter()
            .map(|&x| of_kind(3, x as u64))
            .collect(),
        4 => I32_EDGES
            .iter()
            .flat_map(|&x| [x, x.wrapping_neg()])
            .map(|x| of_kind(4, x as u64))
            .collect(),
        _ => I64_EDGES
            .iter()
            .flat_map(|&x| [x, x.wrapping_neg()])
            .map(|x| of_kind(5, x as u64))
            .collect(),
    }
}
fn rand_of(kind: u32, r: &mut Rng) -> Opnd {
    match kind {
        0 => f80(rand_f80(r)),
        1 => of_kind(1, rand_f32(r) as u64),
        2 => of_kind(2, rand_f64(r)),
        3 => of_kind(3, rand_i32(r) as u64),
        4 => of_kind(4, rand_i32(r) as u64),
        _ => of_kind(5, rand_i64(r) as u64),
    }
}
fn rand_fcw(r: &mut Rng) -> u32 {
    fcw(r.below(4) as u32, r.below(4) as u32)
}

/// Edge pairs (a from the 80-bit edges, b from `kind`'s) under every PC x
/// RC, then `random` random pairs.
fn pair_cases(kind: u32, random: usize, seed: u64, edges: bool) -> Vec<(Opnd, Opnd, u32)> {
    let mut v = Vec::new();
    if edges {
        let (ea, eb) = (e80(), edges_of(kind));
        for &a in &ea {
            for &b in &eb {
                for f in all_fcw() {
                    v.push((a, b, f));
                }
            }
        }
    }
    let mut r = Rng(seed);
    for _ in 0..random {
        v.push((
            f80(rand_f80(&mut r)),
            rand_of(kind, &mut r),
            rand_fcw(&mut r),
        ));
    }
    v
}

fn bin_soft(
    f: fn(u64, u32, u64, u32, u32, u32) -> (u64, u32),
) -> impl Fn(Opnd, Opnd, u32) -> (u64, u32) {
    move |a: Opnd, b: Opnd, fcw| f(a.m, a.se, b.m, b.se, b.kind, fcw)
}

type Bin = fn(u64, u32, u64, u32, u32, u32) -> (u64, u32);

fn run_arith(random: usize, seed: u64, edges: bool) -> usize {
    let ops: [(&str, Bin, [Hw; 5]); 6] = [
        (
            "f80_add",
            fp::f80_add,
            [
                hw_fadd,
                hw_fadd_m32,
                hw_fadd_m64,
                hw_fiadd_m16,
                hw_fiadd_m32,
            ],
        ),
        (
            "f80_sub",
            fp::f80_sub,
            [
                hw_fsub,
                hw_fsub_m32,
                hw_fsub_m64,
                hw_fisub_m16,
                hw_fisub_m32,
            ],
        ),
        (
            "f80_subr",
            fp::f80_subr,
            [
                hw_fsubr,
                hw_fsubr_m32,
                hw_fsubr_m64,
                hw_fisubr_m16,
                hw_fisubr_m32,
            ],
        ),
        (
            "f80_mul",
            fp::f80_mul,
            [
                hw_fmul,
                hw_fmul_m32,
                hw_fmul_m64,
                hw_fimul_m16,
                hw_fimul_m32,
            ],
        ),
        (
            "f80_div",
            fp::f80_div,
            [
                hw_fdiv,
                hw_fdiv_m32,
                hw_fdiv_m64,
                hw_fidiv_m16,
                hw_fidiv_m32,
            ],
        ),
        (
            "f80_divr",
            fp::f80_divr,
            [
                hw_fdivr,
                hw_fdivr_m32,
                hw_fdivr_m64,
                hw_fidivr_m16,
                hw_fidivr_m32,
            ],
        ),
    ];
    let mut n = 0;
    for (i, (name, soft, hws)) in ops.into_iter().enumerate() {
        let s = bin_soft(soft);
        for (kind, hw) in [0u32, 1, 2, 3, 4].into_iter().zip(hws) {
            let g = Group {
                name,
                soft: &s,
                hw,
                out: Out::F80,
                mask: ARITH,
            };
            let cases = pair_cases(kind, random, seed + 10 * i as u64 + kind as u64, edges);
            n += run(&g, cases.into_iter(), &[0]);
        }
    }
    n
}

fn run_st01(random: usize, seed: u64, edges: bool) -> usize {
    type St01 = fn(u64, u32, u64, u32, u32) -> (u64, u32);
    let ops: [(&str, St01, Hw, u32); 3] = [
        ("f80_scale", fp::f80_scale, hw_fscale, ARITH),
        ("f80_prem", fp::f80_prem, hw_fprem, CC_ALL),
        ("f80_prem1", fp::f80_prem1, hw_fprem1, CC_ALL),
    ];
    let mut n = 0;
    for (i, (name, f, hw, mask)) in ops.into_iter().enumerate() {
        let s = move |a: Opnd, b: Opnd, fcw| f(a.m, a.se, b.m, b.se, fcw);
        let g = Group {
            name,
            soft: &s,
            hw,
            out: Out::F80,
            mask,
        };
        n += run(
            &g,
            pair_cases(0, random, seed + 100 + i as u64, edges).into_iter(),
            &[0, 0x4700],
        );
    }
    n
}

fn run_unary(random: usize, seed: u64, edges: bool) -> usize {
    type Un = fn(u64, u32, u32) -> (u64, u32);
    let ops: [(&str, Un, Hw); 2] = [
        ("f80_sqrt", fp::f80_sqrt, hw_fsqrt),
        ("f80_round", fp::f80_round, hw_frndint),
    ];
    let mut n = 0;
    for (i, (name, f, hw)) in ops.into_iter().enumerate() {
        let s = move |a: Opnd, _: Opnd, fcw| f(a.m, a.se, fcw);
        let g = Group {
            name,
            soft: &s,
            hw,
            out: Out::F80,
            mask: ARITH,
        };
        let mut r = Rng(seed + 200 + i as u64);
        let mut v = Vec::new();
        if edges {
            for a in e80() {
                for f in all_fcw() {
                    v.push((a, a, f));
                }
            }
        }
        for _ in 0..random {
            let a = f80(rand_f80(&mut r));
            v.push((a, a, rand_fcw(&mut r)));
        }
        n += run(&g, v.into_iter(), &[0]);
    }
    n
}

fn run_load_store(random: usize, seed: u64, edges: bool) -> usize {
    let mut n = 0;
    let loads: [Hw; 5] = [
        hw_fld_m32,
        hw_fld_m64,
        hw_fild_m16,
        hw_fild_m32,
        hw_fild_m64,
    ];
    let stores: [Hw; 5] = [
        hw_fstp_m32,
        hw_fstp_m64,
        hw_fistp_m16,
        hw_fistp_m32,
        hw_fistp_m64,
    ];
    for (kind, (ld, stw)) in (1u32..=5).zip(loads.into_iter().zip(stores)) {
        let s = |_: Opnd, b: Opnd, fcw| fp::f80_load(b.m, b.kind, fcw);
        let g = Group {
            name: "f80_load",
            soft: &s,
            hw: ld,
            out: Out::F80,
            mask: ARITH,
        };
        let mut r = Rng(seed + 300 + kind as u64);
        let mut v = Vec::new();
        if edges {
            for b in edges_of(kind) {
                for f in all_fcw() {
                    v.push((b, b, f));
                }
            }
        }
        for _ in 0..random {
            let b = rand_of(kind, &mut r);
            v.push((b, b, rand_fcw(&mut r)));
        }
        n += run(&g, v.into_iter(), &[0]);

        let s = move |a: Opnd, _: Opnd, fcw| fp::f80_store(a.m, a.se, kind, fcw);
        let g = Group {
            name: "f80_store",
            soft: &s,
            hw: stw,
            out: Out::Store(kind),
            mask: ARITH,
        };
        let mut v = Vec::new();
        if edges {
            for a in e80() {
                for f in all_fcw() {
                    v.push((a, a, f));
                }
            }
        }
        for _ in 0..random {
            let a = f80(rand_f80(&mut r));
            v.push((a, a, rand_fcw(&mut r)));
        }
        n += run(&g, v.into_iter(), &[0]);
    }
    n
}

fn run_cmp(random: usize, seed: u64, edges: bool) -> usize {
    let forms: [(&str, u32, u32, Hw); 6] = [
        ("f80_cmp/fcom", 0, 0, hw_fcom),
        ("f80_cmp/fucom", 0, 1, hw_fucom),
        ("f80_cmp/fcom m32", 1, 0, hw_fcom_m32),
        ("f80_cmp/fcom m64", 2, 0, hw_fcom_m64),
        ("f80_cmp/ficom m16", 3, 0, hw_ficom_m16),
        ("f80_cmp/ficom m32", 4, 0, hw_ficom_m32),
    ];
    let mut n = 0;
    for (i, (name, kind, quiet, hw)) in forms.into_iter().enumerate() {
        let s = move |a: Opnd, b: Opnd, fcw| {
            let (r, st) = fp::f80_cmp(a.m, a.se, b.m, b.se, b.kind, quiet, fcw);
            (r as u64, st)
        };
        let g = Group {
            name,
            soft: &s,
            hw,
            out: Out::Cmp,
            mask: CC_ALL,
        };
        // PC and RC don't matter to a compare: one control word for the edges.
        let mut cases = pair_cases(kind, random, seed + 400 + i as u64, false);
        if edges {
            for a in e80() {
                for b in edges_of(kind) {
                    cases.push((a, b, fcw(3, 0)));
                }
            }
        }
        n += run(&g, cases.into_iter(), &[0]);
    }
    n
}

#[test]
fn x87_arith_vs_cpu() {
    eprintln!("x87 arith: {} cases", run_arith(RANDOM, 11, true));
}
#[test]
fn x87_scale_prem_vs_cpu() {
    eprintln!(
        "x87 fscale, fprem, fprem1: {} cases",
        run_st01(RANDOM, 12, true)
    );
}
#[test]
fn x87_sqrt_round_vs_cpu() {
    eprintln!("x87 fsqrt, frndint: {} cases", run_unary(RANDOM, 13, true));
}
#[test]
fn x87_load_store_vs_cpu() {
    eprintln!(
        "x87 loads and stores: {} cases",
        run_load_store(RANDOM, 14, true)
    );
}
#[test]
fn x87_compare_vs_cpu() {
    eprintln!("x87 compares: {} cases", run_cmp(RANDOM, 15, true));
}

/// Millions of random cases: `cargo test -p x64wasm-fp --release --test
/// native_x87 -- --ignored --nocapture` (FP_LONG=N per function and form,
/// default 1,000,000).
#[test]
#[ignore]
fn x87_long_random() {
    let k: usize = std::env::var("FP_LONG")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let seed = 0x87_0000 ^ std::process::id() as u64;
    let n = run_arith(k, seed, false)
        + run_st01(k, seed, false)
        + run_unary(k, seed, false)
        + run_load_store(k, seed, false)
        + run_cmp(k, seed, false);
    eprintln!("x87 long random (seed {seed:#x}): {n} cases");
}

#[allow(dead_code)]
const _: &str = LD2;

/// The vendor-independent checks accept what the SDM allows and nothing
/// else, so a non-Intel host still tests these cases.
#[test]
fn sdm_checks_accept_other_implementations() {
    let a = f80((0xB504_F333_F9DE_6484, 0x3FFF + 100));
    let b = f80((0xC000_0000_0000_0001, 0x4000));
    let (r, st) = fp::f80_prem(a.m, a.se, b.m, b.se, fcw(3, 0));
    assert_eq!(
        impl_dependent("f80_prem", &a, &b, st),
        Some(Dep::PartialRem)
    );
    for n in 32..=63 {
        let (m, se) = partial_rem(&a, &b, n).unwrap();
        assert!(
            sdm_ok(Dep::PartialRem, &a, &b, (r, st), (m, se, 0x400), CC_ALL),
            "N = {n}"
        );
        // C1 set, C2 clear, or the wrong sign: rejected.
        assert!(!sdm_ok(
            Dep::PartialRem,
            &a,
            &b,
            (r, st),
            (m, se, 0x600),
            CC_ALL
        ));
        assert!(!sdm_ok(
            Dep::PartialRem,
            &a,
            &b,
            (r, st),
            (m, se, 0),
            CC_ALL
        ));
        assert!(!sdm_ok(
            Dep::PartialRem,
            &a,
            &b,
            (r, st),
            (m, se ^ 0x8000, 0x400),
            CC_ALL
        ));
    }
    // N outside [32, 63]: rejected unless it gives the same value as a valid
    // N (as when doubling the remainder doesn't wrap).
    let valid: Vec<_> = (32..=63).filter_map(|n| partial_rem(&a, &b, n)).collect();
    let mut rejected = 0;
    for n in [0, 1, 31, 64, 65, 100] {
        let (m, se) = partial_rem(&a, &b, n).unwrap();
        if !valid.contains(&(m, se)) {
            assert!(
                !sdm_ok(Dep::PartialRem, &a, &b, (r, st), (m, se, 0x400), CC_ALL),
                "N = {n}"
            );
            rejected += 1;
        }
    }
    assert!(rejected >= 3);
    // Two QNaNs with equal significands and different signs: either one.
    let a = f80((0xC000_0000_0000_0001, 0x7FFF));
    let b = f80((0xC000_0000_0000_0001, 0xFFFF));
    let (r, st) = fp::f80_add(a.m, a.se, b.m, b.se, 0, fcw(3, 0));
    assert_eq!(impl_dependent("f80_add", &a, &b, st), Some(Dep::NanTie));
    assert!(sdm_ok(
        Dep::NanTie,
        &a,
        &b,
        (r, st),
        (a.m, 0x7FFF, 0),
        ARITH
    ));
    assert!(sdm_ok(
        Dep::NanTie,
        &a,
        &b,
        (r, st),
        (b.m, 0xFFFF, 0),
        ARITH
    ));
    assert!(!sdm_ok(
        Dep::NanTie,
        &a,
        &b,
        (r, st),
        (0xC000_0000_0000_0000, 0xFFFF, 0),
        ARITH
    ));
    assert!(!sdm_ok(
        Dep::NanTie,
        &a,
        &b,
        (r, st),
        (a.m, 0x7FFF, 1),
        ARITH
    ));
}
