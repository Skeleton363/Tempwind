//! Writes test vectors for the wasm build of the soft float, computed by the
//! native Rust API: `cargo run -p x64wasm-fp --release --example vectors --
//! --out target/fp/vectors.json`. `tests/node/fp.test.mjs` replays them
//! against `x64wasm_fp.wasm`.
//!
//! Format: `[{"fn", "args": ["0x…", …], "result": "0x…", "status": "0x…"}]`,
//! every value as the hex of its unsigned bits (an i32 as 32 bits, an i64
//! as 64), per export edge cases first, then random ones. A tenth of the
//! cases unmask random exceptions, so FP_UNMASKED and ES are covered too.

#[path = "../tests/common/mod.rs"]
mod common;

use common::*;
use std::fmt::Write as _;
use x64wasm_fp as fp;

/// Cases per export: edge pairs sampled first, then random ones.
const EDGE: usize = 400;
const RANDOM: usize = 400;

/// An argument or result: its bits and whether it is an i64.
#[derive(Clone, Copy)]
enum V {
    I32(u32),
    I64(u64),
}
fn hex(v: V) -> String {
    match v {
        V::I32(x) => format!("\"{x:#x}\""),
        V::I64(x) => format!("\"{x:#x}\""),
    }
}

struct Out {
    json: String,
    n: usize,
}
impl Out {
    fn push(&mut self, name: &str, args: &[V], result: V, status: u32) {
        if self.n > 0 {
            self.json.push_str(",\n");
        }
        let args: Vec<String> = args.iter().map(|&a| hex(a)).collect();
        write!(
            self.json,
            "{{\"fn\":\"{name}\",\"args\":[{}],\"result\":{},\"status\":\"{status:#x}\"}}",
            args.join(","),
            hex(result)
        )
        .unwrap();
        self.n += 1;
    }
}

fn sse_mode(r: &mut Rng) -> u32 {
    let m = mxcsr(r.below(16) as u32, sticky(r));
    if r.below(10) == 0 {
        m & !((r.below(64) as u32) << 7)
    } else {
        m
    }
}
fn x87_fcw(r: &mut Rng) -> u32 {
    let f = fcw(r.below(4) as u32, r.below(4) as u32);
    if r.below(10) == 0 {
        f & !(r.below(64) as u32)
    } else {
        f
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out_path = match args.iter().position(|a| a == "--out") {
        Some(i) if i + 1 < args.len() => args[i + 1].clone(),
        _ => {
            eprintln!("usage: vectors --out FILE.json");
            std::process::exit(2);
        }
    };
    let mut o = Out {
        json: String::from("[\n"),
        n: 0,
    };
    let mut r = Rng(0xF00D);
    let e32 = with_neg32(F32_EDGES);
    let e64 = with_neg64(F64_EDGES);
    let e80 = with_neg80(F80_EDGES);
    let i32e: Vec<i32> = I32_EDGES
        .iter()
        .flat_map(|&x| [x, x.wrapping_neg()])
        .collect();
    let i64e: Vec<i64> = I64_EDGES
        .iter()
        .flat_map(|&x| [x, x.wrapping_neg()])
        .collect();

    // An operand from the edges for the first EDGE cases, random after.
    let f32v = |r: &mut Rng, i: usize| if i < EDGE { r.pick(&e32) } else { rand_f32(r) };
    let f64v = |r: &mut Rng, i: usize| if i < EDGE { r.pick(&e64) } else { rand_f64(r) };
    let f80v = |r: &mut Rng, i: usize| if i < EDGE { r.pick(&e80) } else { rand_f80(r) };
    let i32v = |r: &mut Rng, i: usize| if i < EDGE { r.pick(&i32e) } else { rand_i32(r) };
    let i64v = |r: &mut Rng, i: usize| if i < EDGE { r.pick(&i64e) } else { rand_i64(r) };
    let n = EDGE + RANDOM;

    type B32 = fn(u32, u32, u32) -> (u32, u32);
    type B64 = fn(u64, u64, u32) -> (u64, u32);
    let b32: [(&str, B32); 6] = [
        ("f32_add", fp::f32_add),
        ("f32_sub", fp::f32_sub),
        ("f32_mul", fp::f32_mul),
        ("f32_div", fp::f32_div),
        ("f32_min", fp::f32_min),
        ("f32_max", fp::f32_max),
    ];
    for (name, f) in b32 {
        for i in 0..n {
            let (a, b, m) = (f32v(&mut r, i), f32v(&mut r, i), sse_mode(&mut r));
            let (x, s) = f(a, b, m);
            o.push(name, &[V::I32(a), V::I32(b), V::I32(m)], V::I32(x), s);
        }
    }
    let b64: [(&str, B64); 6] = [
        ("f64_add", fp::f64_add),
        ("f64_sub", fp::f64_sub),
        ("f64_mul", fp::f64_mul),
        ("f64_div", fp::f64_div),
        ("f64_min", fp::f64_min),
        ("f64_max", fp::f64_max),
    ];
    for (name, f) in b64 {
        for i in 0..n {
            let (a, b, m) = (f64v(&mut r, i), f64v(&mut r, i), sse_mode(&mut r));
            let (x, s) = f(a, b, m);
            o.push(name, &[V::I64(a), V::I64(b), V::I32(m)], V::I64(x), s);
        }
    }
    for i in 0..n {
        let (a, m) = (f32v(&mut r, i), sse_mode(&mut r));
        let (x, s) = fp::f32_sqrt(a, m);
        o.push("f32_sqrt", &[V::I32(a), V::I32(m)], V::I32(x), s);
        let (a, m) = (f64v(&mut r, i), sse_mode(&mut r));
        let (x, s) = fp::f64_sqrt(a, m);
        o.push("f64_sqrt", &[V::I64(a), V::I32(m)], V::I64(x), s);
    }
    for i in 0..n {
        let (a, b, p, m) = (
            f32v(&mut r, i),
            f32v(&mut r, i),
            r.below(8) as u32,
            sse_mode(&mut r),
        );
        let (x, s) = fp::f32_cmp(a, b, p, m);
        o.push(
            "f32_cmp",
            &[V::I32(a), V::I32(b), V::I32(p), V::I32(m)],
            V::I32(x),
            s,
        );
        let (x, s) = fp::f32_comi(a, b, p & 1, m);
        o.push(
            "f32_comi",
            &[V::I32(a), V::I32(b), V::I32(p & 1), V::I32(m)],
            V::I32(x),
            s,
        );
        let (a, b, p, m) = (
            f64v(&mut r, i),
            f64v(&mut r, i),
            r.below(8) as u32,
            sse_mode(&mut r),
        );
        let (x, s) = fp::f64_cmp(a, b, p, m);
        o.push(
            "f64_cmp",
            &[V::I64(a), V::I64(b), V::I32(p), V::I32(m)],
            V::I32(x),
            s,
        );
        let (x, s) = fp::f64_comi(a, b, p & 1, m);
        o.push(
            "f64_comi",
            &[V::I64(a), V::I64(b), V::I32(p & 1), V::I32(m)],
            V::I32(x),
            s,
        );
    }
    for i in 0..n {
        let m = sse_mode(&mut r);
        let a = f32v(&mut r, i);
        let (x, s) = fp::f32_to_f64(a, m);
        o.push("f32_to_f64", &[V::I32(a), V::I32(m)], V::I64(x), s);
        let t = r.below(2) as u32;
        let (x, s) = fp::f32_to_i32(a, m, t);
        o.push(
            "f32_to_i32",
            &[V::I32(a), V::I32(m), V::I32(t)],
            V::I32(x as u32),
            s,
        );
        let (x, s) = fp::f32_to_i64(a, m, t);
        o.push(
            "f32_to_i64",
            &[V::I32(a), V::I32(m), V::I32(t)],
            V::I64(x as u64),
            s,
        );
        let a = f64v(&mut r, i);
        let (x, s) = fp::f64_to_f32(a, m);
        o.push("f64_to_f32", &[V::I64(a), V::I32(m)], V::I32(x), s);
        let (x, s) = fp::f64_to_i32(a, m, t);
        o.push(
            "f64_to_i32",
            &[V::I64(a), V::I32(m), V::I32(t)],
            V::I32(x as u32),
            s,
        );
        let (x, s) = fp::f64_to_i64(a, m, t);
        o.push(
            "f64_to_i64",
            &[V::I64(a), V::I32(m), V::I32(t)],
            V::I64(x as u64),
            s,
        );
        let a = i32v(&mut r, i);
        let (x, s) = fp::i32_to_f32(a, m);
        o.push("i32_to_f32", &[V::I32(a as u32), V::I32(m)], V::I32(x), s);
        let a = i64v(&mut r, i);
        let (x, s) = fp::i64_to_f32(a, m);
        o.push("i64_to_f32", &[V::I64(a as u64), V::I32(m)], V::I32(x), s);
        let (x, s) = fp::i64_to_f64(a, m);
        o.push("i64_to_f64", &[V::I64(a as u64), V::I32(m)], V::I64(x), s);
    }

    // x87. An operand b of a random kind (arithmetic: f80, m32, m64, m16int,
    // m32int; compares the same).
    let bkind = |r: &mut Rng, i: usize| -> (u64, u32, u32) {
        match r.below(5) as u32 {
            0 => {
                let (m, se) = f80v(r, i);
                (m, se, 0)
            }
            1 => (f32v(r, i) as u64, 0, 1),
            2 => (f64v(r, i), 0, 2),
            3 => (i32v(r, i) as i16 as i64 as u64, 0, 3),
            _ => (i32v(r, i) as i64 as u64, 0, 4),
        }
    };
    type A80 = fn(u64, u32, u64, u32, u32, u32) -> (u64, u32);
    let a80: [(&str, A80); 6] = [
        ("f80_add", fp::f80_add),
        ("f80_sub", fp::f80_sub),
        ("f80_subr", fp::f80_subr),
        ("f80_mul", fp::f80_mul),
        ("f80_div", fp::f80_div),
        ("f80_divr", fp::f80_divr),
    ];
    for (name, f) in a80 {
        for i in 0..n {
            let (am, ase) = f80v(&mut r, i);
            let (bm, bse, k) = bkind(&mut r, i);
            let c = x87_fcw(&mut r);
            let (x, s) = f(am, ase, bm, bse, k, c);
            o.push(
                name,
                &[
                    V::I64(am),
                    V::I32(ase),
                    V::I64(bm),
                    V::I32(bse),
                    V::I32(k),
                    V::I32(c),
                ],
                V::I64(x),
                s,
            );
        }
    }
    type U80 = fn(u64, u32, u32) -> (u64, u32);
    let u80: [(&str, U80); 2] = [("f80_sqrt", fp::f80_sqrt), ("f80_round", fp::f80_round)];
    for (name, f) in u80 {
        for i in 0..n {
            let (am, ase) = f80v(&mut r, i);
            let c = x87_fcw(&mut r);
            let (x, s) = f(am, ase, c);
            o.push(name, &[V::I64(am), V::I32(ase), V::I32(c)], V::I64(x), s);
        }
    }
    type S80 = fn(u64, u32, u64, u32, u32) -> (u64, u32);
    let s80: [(&str, S80); 3] = [
        ("f80_scale", fp::f80_scale),
        ("f80_prem", fp::f80_prem),
        ("f80_prem1", fp::f80_prem1),
    ];
    for (name, f) in s80 {
        for i in 0..n {
            let (am, ase) = f80v(&mut r, i);
            let (bm, bse) = f80v(&mut r, i);
            let c = x87_fcw(&mut r);
            let (x, s) = f(am, ase, bm, bse, c);
            o.push(
                name,
                &[V::I64(am), V::I32(ase), V::I64(bm), V::I32(bse), V::I32(c)],
                V::I64(x),
                s,
            );
        }
    }
    for i in 0..n {
        let c = x87_fcw(&mut r);
        let k = 1 + r.below(5) as u32;
        let bm = match k {
            1 => f32v(&mut r, i) as u64,
            2 => f64v(&mut r, i),
            3 => i32v(&mut r, i) as i16 as i64 as u64,
            4 => i32v(&mut r, i) as i64 as u64,
            _ => i64v(&mut r, i) as u64,
        };
        let (x, s) = fp::f80_load(bm, k, c);
        o.push(
            "f80_load",
            &[V::I64(bm), V::I32(k), V::I32(c)],
            V::I64(x),
            s,
        );
        let (am, ase) = f80v(&mut r, i);
        let (x, s) = fp::f80_store(am, ase, k, c);
        o.push(
            "f80_store",
            &[V::I64(am), V::I32(ase), V::I32(k), V::I32(c)],
            V::I64(x),
            s,
        );
        let (bm, bse, k) = bkind(&mut r, i);
        let q = r.below(2) as u32;
        let (x, s) = fp::f80_cmp(am, ase, bm, bse, k, q, c);
        o.push(
            "f80_cmp",
            &[
                V::I64(am),
                V::I32(ase),
                V::I64(bm),
                V::I32(bse),
                V::I32(k),
                V::I32(q),
                V::I32(c),
            ],
            V::I32(x),
            s,
        );
    }

    o.json.push_str("\n]\n");
    if let Some(dir) = std::path::Path::new(&out_path).parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(&out_path, &o.json).unwrap();
    eprintln!(
        "wrote {} vectors ({} bytes) to {out_path}",
        o.n,
        o.json.len()
    );
}
