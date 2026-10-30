//! Differential tests of the SSE functions against the CPU: each case runs
//! the real instruction (ldmxcsr, the op, stmxcsr) and compares the result
//! bits and the whole MXCSR. All exceptions are masked (unmasked ones would
//! trap); `unmasked` in the unit tests covers FP_UNMASKED.
#![cfg(target_arch = "x86_64")]

mod common;

use common::*;
use core::arch::asm;
use core::fmt::Debug;
use x64wasm_fp as fp;

/// Random cases per function in the default run.
const RANDOM: usize = 40_000;

macro_rules! hw_bin {
    ($name:ident, $t:ty, $insn:literal) => {
        fn $name(a: $t, b: $t, m: u32) -> ($t, u32) {
            let mut s = [0u32, m, 0];
            let mut x = a;
            unsafe {
                asm!("stmxcsr [rcx]", "ldmxcsr [rcx+4]", $insn, "stmxcsr [rcx+8]", "ldmxcsr [rcx]",
                    in("rcx") s.as_mut_ptr(), inout("xmm0") x, in("xmm1") b, options(nostack));
            }
            (x, s[2])
        }
    };
}
macro_rules! hw_un {
    ($name:ident, $ti:ty, $to:ty, $insn:literal) => {
        fn $name(a: $ti, m: u32) -> ($to, u32) {
            let mut s = [0u32, m, 0];
            let x: $to;
            unsafe {
                asm!("stmxcsr [rcx]", "ldmxcsr [rcx+4]", $insn, "stmxcsr [rcx+8]", "ldmxcsr [rcx]",
                    in("rcx") s.as_mut_ptr(), out("xmm0") x, in("xmm1") a, options(nostack));
            }
            (x, s[2])
        }
    };
}
macro_rules! hw_i2f {
    ($name:ident, $ti:ty, $to:ty, $insn:literal) => {
        fn $name(a: $ti, m: u32) -> ($to, u32) {
            let mut s = [0u32, m, 0];
            let x: $to;
            unsafe {
                asm!("stmxcsr [rcx]", "ldmxcsr [rcx+4]", $insn, "stmxcsr [rcx+8]", "ldmxcsr [rcx]",
                    in("rcx") s.as_mut_ptr(), out("xmm0") x, in("rax") a as i64, options(nostack));
            }
            (x, s[2])
        }
    };
}
macro_rules! hw_f2i {
    ($name:ident, $ti:ty, $insn:literal) => {
        fn $name(a: $ti, m: u32) -> (u64, u32) {
            let mut s = [0u32, m, 0];
            let x: u64;
            unsafe {
                asm!("stmxcsr [rcx]", "ldmxcsr [rcx+4]", $insn, "stmxcsr [rcx+8]", "ldmxcsr [rcx]",
                    in("rcx") s.as_mut_ptr(), out("rax") x, in("xmm1") a, options(nostack));
            }
            (x, s[2])
        }
    };
}
macro_rules! hw_comi {
    ($name:ident, $t:ty, $insn:literal) => {
        fn $name(a: $t, b: $t, m: u32) -> (u32, u32) {
            let mut s = [0u32, m, 0];
            let f: u64;
            unsafe {
                asm!("stmxcsr [rcx]", "ldmxcsr [rcx+4]", $insn, "stmxcsr [rcx+8]", "pushfq", "pop rax",
                    "ldmxcsr [rcx]",
                    in("rcx") s.as_mut_ptr(), out("rax") f, in("xmm0") a, in("xmm1") b);
            }
            ((f & 0x8D5) as u32, s[2])
        }
    };
}

hw_bin!(hw_addss, u32, "addss xmm0, xmm1");
hw_bin!(hw_subss, u32, "subss xmm0, xmm1");
hw_bin!(hw_mulss, u32, "mulss xmm0, xmm1");
hw_bin!(hw_divss, u32, "divss xmm0, xmm1");
hw_bin!(hw_minss, u32, "minss xmm0, xmm1");
hw_bin!(hw_maxss, u32, "maxss xmm0, xmm1");
hw_bin!(hw_addsd, u64, "addsd xmm0, xmm1");
hw_bin!(hw_subsd, u64, "subsd xmm0, xmm1");
hw_bin!(hw_mulsd, u64, "mulsd xmm0, xmm1");
hw_bin!(hw_divsd, u64, "divsd xmm0, xmm1");
hw_bin!(hw_minsd, u64, "minsd xmm0, xmm1");
hw_bin!(hw_maxsd, u64, "maxsd xmm0, xmm1");
hw_un!(hw_sqrtss, u32, u32, "sqrtss xmm0, xmm1");
hw_un!(hw_sqrtsd, u64, u64, "sqrtsd xmm0, xmm1");
hw_un!(hw_cvtss2sd, u32, u64, "cvtss2sd xmm0, xmm1");
hw_un!(hw_cvtsd2ss, u64, u32, "cvtsd2ss xmm0, xmm1");
hw_i2f!(hw_cvtsi2ss32, i32, u32, "cvtsi2ss xmm0, eax");
hw_i2f!(hw_cvtsi2ss64, i64, u32, "cvtsi2ss xmm0, rax");
hw_i2f!(hw_cvtsi2sd64, i64, u64, "cvtsi2sd xmm0, rax");
hw_f2i!(hw_cvtss2si32, u32, "cvtss2si eax, xmm1");
hw_f2i!(hw_cvttss2si32, u32, "cvttss2si eax, xmm1");
hw_f2i!(hw_cvtsd2si32, u64, "cvtsd2si eax, xmm1");
hw_f2i!(hw_cvttsd2si32, u64, "cvttsd2si eax, xmm1");
hw_f2i!(hw_cvtss2si64, u32, "cvtss2si rax, xmm1");
hw_f2i!(hw_cvttss2si64, u32, "cvttss2si rax, xmm1");
hw_f2i!(hw_cvtsd2si64, u64, "cvtsd2si rax, xmm1");
hw_f2i!(hw_cvttsd2si64, u64, "cvttsd2si rax, xmm1");
hw_comi!(hw_comiss, u32, "comiss xmm0, xmm1");
hw_comi!(hw_ucomiss, u32, "ucomiss xmm0, xmm1");
hw_comi!(hw_comisd, u64, "comisd xmm0, xmm1");
hw_comi!(hw_ucomisd, u64, "ucomisd xmm0, xmm1");

macro_rules! hw_cmp_all {
    ($name:ident, $t:ty, $insn:literal) => {
        fn $name(a: $t, b: $t, pred: u32, m: u32) -> (u32, u32) {
            macro_rules! one {
                ($p:literal) => {{
                    let mut s = [0u32, m, 0];
                    let mut x = a;
                    unsafe {
                        asm!("stmxcsr [rcx]", "ldmxcsr [rcx+4]", concat!($insn, ", ", $p), "stmxcsr [rcx+8]",
                            "ldmxcsr [rcx]",
                            in("rcx") s.as_mut_ptr(), inout("xmm0") x, in("xmm1") b, options(nostack));
                    }
                    ((x != 0) as u32, s[2])
                }};
            }
            match pred {
                0 => one!("0"),
                1 => one!("1"),
                2 => one!("2"),
                3 => one!("3"),
                4 => one!("4"),
                5 => one!("5"),
                6 => one!("6"),
                _ => one!("7"),
            }
        }
    };
}
hw_cmp_all!(hw_cmpss, u32, "cmpss xmm0, xmm1");
hw_cmp_all!(hw_cmpsd, u64, "cmpsd xmm0, xmm1");

/// Runs every case through both and panics with the first mismatches.
fn check<A: Copy + Debug, R: PartialEq + Debug>(
    name: &str,
    cases: impl Iterator<Item = (A, u32)>,
    soft: impl Fn(A, u32) -> (R, u32),
    hw: impl Fn(A, u32) -> (R, u32),
) -> usize {
    let mut n = 0;
    let mut bad = Vec::new();
    let mut nbad = 0;
    for (a, m) in cases {
        n += 1;
        let (s, h) = (soft(a, m), hw(a, m));
        if s != h {
            nbad += 1;
            if bad.len() < 12 {
                bad.push(format!("  {name}({a:x?}, mxcsr {m:#x}): soft {s:x?}, cpu {h:x?}"));
            }
        }
    }
    assert!(nbad == 0, "{name}: {nbad} of {n} cases differ:\n{}", bad.join("\n"));
    n
}

fn pairs<T: Copy>(xs: &[T], modes: &[u32], seed: u64) -> Vec<((T, T), u32)> {
    let mut r = Rng(seed);
    let mut v = Vec::new();
    for &a in xs {
        for &b in xs {
            for &mode in modes {
                v.push(((a, b), mxcsr(mode, sticky(&mut r))));
            }
        }
    }
    v
}
fn singles<T: Copy>(xs: &[T], modes: &[u32], seed: u64) -> Vec<(T, u32)> {
    let mut r = Rng(seed);
    xs.iter().flat_map(|&a| modes.iter().map(move |&m| (a, m))).map(|(a, m)| (a, mxcsr(m, sticky(&mut r)))).collect()
}
fn random_mode(r: &mut Rng) -> u32 {
    let m = r.below(16) as u32;
    mxcsr(m, sticky(r))
}

const ALL_MODES: [u32; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

type Bin32 = fn(u32, u32, u32) -> (u32, u32);
type Bin64 = fn(u64, u64, u32) -> (u64, u32);

fn bin32() -> [(&'static str, Bin32, Bin32); 6] {
    [
        ("f32_add", fp::f32_add, hw_addss),
        ("f32_sub", fp::f32_sub, hw_subss),
        ("f32_mul", fp::f32_mul, hw_mulss),
        ("f32_div", fp::f32_div, hw_divss),
        ("f32_min", fp::f32_min, hw_minss),
        ("f32_max", fp::f32_max, hw_maxss),
    ]
}
fn bin64() -> [(&'static str, Bin64, Bin64); 6] {
    [
        ("f64_add", fp::f64_add, hw_addsd),
        ("f64_sub", fp::f64_sub, hw_subsd),
        ("f64_mul", fp::f64_mul, hw_mulsd),
        ("f64_div", fp::f64_div, hw_divsd),
        ("f64_min", fp::f64_min, hw_minsd),
        ("f64_max", fp::f64_max, hw_maxsd),
    ]
}

fn run_bin(random: usize, seed: u64, edges: bool) -> usize {
    let mut n = 0;
    let e32 = with_neg32(F32_EDGES);
    let e64 = with_neg64(F64_EDGES);
    for (i, (name, soft, hw)) in bin32().into_iter().enumerate() {
        if edges {
            let c = pairs(&e32, &ALL_MODES, seed + i as u64);
            n += check(name, c.into_iter(), |(a, b), m| soft(a, b, m), |(a, b), m| hw(a, b, m));
        }
        let mut r = Rng(seed ^ (i as u64 + 100));
        let c = (0..random).map(|_| ((rand_f32(&mut r), rand_f32(&mut r)), random_mode(&mut r)));
        n += check(name, c, |(a, b), m| soft(a, b, m), |(a, b), m| hw(a, b, m));
    }
    for (i, (name, soft, hw)) in bin64().into_iter().enumerate() {
        if edges {
            let c = pairs(&e64, &ALL_MODES, seed + 50 + i as u64);
            n += check(name, c.into_iter(), |(a, b), m| soft(a, b, m), |(a, b), m| hw(a, b, m));
        }
        let mut r = Rng(seed ^ (i as u64 + 200));
        let c = (0..random).map(|_| ((rand_f64(&mut r), rand_f64(&mut r)), random_mode(&mut r)));
        n += check(name, c, |(a, b), m| soft(a, b, m), |(a, b), m| hw(a, b, m));
    }
    n
}

fn run_cmp(random: usize, seed: u64, edges: bool) -> usize {
    let mut n = 0;
    let e32 = with_neg32(F32_EDGES);
    let e64 = with_neg64(F64_EDGES);
    let modes = [0, 8, 3, 12];
    for pred in 0..8u32 {
        if edges {
            let c = pairs(&e32, &modes, seed + pred as u64);
            n += check("f32_cmp", c.into_iter(), |(a, b), m| fp::f32_cmp(a, b, pred, m), |(a, b), m| hw_cmpss(a, b, pred, m));
            let c = pairs(&e64, &modes, seed + pred as u64);
            n += check("f64_cmp", c.into_iter(), |(a, b), m| fp::f64_cmp(a, b, pred, m), |(a, b), m| hw_cmpsd(a, b, pred, m));
        }
        let mut r = Rng(seed ^ (pred as u64 + 300));
        let c = (0..random / 8).map(|_| ((rand_f32(&mut r), rand_f32(&mut r)), random_mode(&mut r)));
        n += check("f32_cmp", c, |(a, b), m| fp::f32_cmp(a, b, pred, m), |(a, b), m| hw_cmpss(a, b, pred, m));
        let c = (0..random / 8).map(|_| ((rand_f64(&mut r), rand_f64(&mut r)), random_mode(&mut r)));
        n += check("f64_cmp", c, |(a, b), m| fp::f64_cmp(a, b, pred, m), |(a, b), m| hw_cmpsd(a, b, pred, m));
    }
    for sig in 0..2u32 {
        let (h32, h64): (Bin32Cmp, Bin64Cmp) =
            if sig == 1 { (hw_comiss, hw_comisd) } else { (hw_ucomiss, hw_ucomisd) };
        if edges {
            let c = pairs(&e32, &modes, seed + 20 + sig as u64);
            n += check("f32_comi", c.into_iter(), |(a, b), m| fp::f32_comi(a, b, sig, m), |(a, b), m| h32(a, b, m));
            let c = pairs(&e64, &modes, seed + 20 + sig as u64);
            n += check("f64_comi", c.into_iter(), |(a, b), m| fp::f64_comi(a, b, sig, m), |(a, b), m| h64(a, b, m));
        }
        let mut r = Rng(seed ^ (sig as u64 + 400));
        let c = (0..random / 2).map(|_| ((rand_f32(&mut r), rand_f32(&mut r)), random_mode(&mut r)));
        n += check("f32_comi", c, |(a, b), m| fp::f32_comi(a, b, sig, m), |(a, b), m| h32(a, b, m));
        let c = (0..random / 2).map(|_| ((rand_f64(&mut r), rand_f64(&mut r)), random_mode(&mut r)));
        n += check("f64_comi", c, |(a, b), m| fp::f64_comi(a, b, sig, m), |(a, b), m| h64(a, b, m));
    }
    n
}
type Bin32Cmp = fn(u32, u32, u32) -> (u32, u32);
type Bin64Cmp = fn(u64, u64, u32) -> (u32, u32);

fn run_unary(random: usize, seed: u64, edges: bool) -> usize {
    let mut n = 0;
    let e32 = with_neg32(F32_EDGES);
    let e64 = with_neg64(F64_EDGES);
    let mut r = Rng(seed ^ 500);
    macro_rules! go {
        ($name:literal, $edges:expr, $gen:expr, $soft:expr, $hw:expr) => {{
            if edges {
                n += check($name, singles(&$edges, &ALL_MODES, seed).into_iter(), $soft, $hw);
            }
            let c: Vec<_> = (0..random).map(|_| ($gen(&mut r), random_mode(&mut r))).collect();
            n += check($name, c.into_iter(), $soft, $hw);
        }};
    }
    go!("f32_sqrt", e32, rand_f32, fp::f32_sqrt, hw_sqrtss);
    go!("f64_sqrt", e64, rand_f64, fp::f64_sqrt, hw_sqrtsd);
    go!("f32_to_f64", e32, rand_f32, fp::f32_to_f64, hw_cvtss2sd);
    go!("f64_to_f32", e64, rand_f64, fp::f64_to_f32, hw_cvtsd2ss);
    go!("i32_to_f32", I32_EDGES.iter().flat_map(|&x| [x, x.wrapping_neg()]).collect::<Vec<_>>(), rand_i32, fp::i32_to_f32, hw_cvtsi2ss32);
    let i64e: Vec<i64> = I64_EDGES.iter().flat_map(|&x| [x, x.wrapping_neg()]).collect();
    go!("i64_to_f32", i64e, rand_i64, fp::i64_to_f32, hw_cvtsi2ss64);
    go!("i64_to_f64", i64e, rand_i64, fp::i64_to_f64, hw_cvtsi2sd64);
    let s32 = |f: fn(u32, u32, u32) -> (i32, u32), t| move |a, m| { let (x, s) = f(a, m, t); (x as u32 as u64, s) };
    let d32 = |f: fn(u64, u32, u32) -> (i32, u32), t| move |a, m| { let (x, s) = f(a, m, t); (x as u32 as u64, s) };
    let s64 = |f: fn(u32, u32, u32) -> (i64, u32), t| move |a, m| { let (x, s) = f(a, m, t); (x as u64, s) };
    let d64 = |f: fn(u64, u32, u32) -> (i64, u32), t| move |a, m| { let (x, s) = f(a, m, t); (x as u64, s) };
    go!("f32_to_i32", e32, rand_f32, s32(fp::f32_to_i32, 0), hw_cvtss2si32);
    go!("f32_to_i32/t", e32, rand_f32, s32(fp::f32_to_i32, 1), hw_cvttss2si32);
    go!("f64_to_i32", e64, rand_f64, d32(fp::f64_to_i32, 0), hw_cvtsd2si32);
    go!("f64_to_i32/t", e64, rand_f64, d32(fp::f64_to_i32, 1), hw_cvttsd2si32);
    go!("f32_to_i64", e32, rand_f32, s64(fp::f32_to_i64, 0), hw_cvtss2si64);
    go!("f32_to_i64/t", e32, rand_f32, s64(fp::f32_to_i64, 1), hw_cvttss2si64);
    go!("f64_to_i64", e64, rand_f64, d64(fp::f64_to_i64, 0), hw_cvtsd2si64);
    go!("f64_to_i64/t", e64, rand_f64, d64(fp::f64_to_i64, 1), hw_cvttsd2si64);
    n
}

#[test]
fn sse_arith_vs_cpu() {
    let n = run_bin(RANDOM, 1, true);
    eprintln!("sse arith: {n} cases");
}

#[test]
fn sse_compare_vs_cpu() {
    let n = run_cmp(RANDOM, 2, true);
    eprintln!("sse compare: {n} cases");
}

#[test]
fn sse_unary_vs_cpu() {
    let n = run_unary(RANDOM, 3, true);
    eprintln!("sse unary and conversions: {n} cases");
}

/// Millions of random cases: `cargo test -p x64wasm-fp --release --test
/// native_sse -- --ignored --nocapture` (FP_LONG=N sets the per-function
/// count, default 1,000,000).
#[test]
#[ignore]
fn sse_long_random() {
    let k: usize = std::env::var("FP_LONG").ok().and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let seed = 0x5EED_0000 ^ std::process::id() as u64;
    let n = run_bin(k, seed, false) + run_cmp(k, seed, false) + run_unary(k, seed, false);
    eprintln!("sse long random (seed {seed:#x}): {n} cases");
}
