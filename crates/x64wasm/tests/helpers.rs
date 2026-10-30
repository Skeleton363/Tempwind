//! The `flags` function and the `div128` helper, run in wasmtime against
//! reference models written from the SDM's definitions.

use wasmtime::{Config, Engine, Instance, Module, Store, TypedFunc};
use x64wasm::abi::flags::{AF, CF, OF, PF, SF, STATUS, ZF};
use x64wasm::ir::{FlagOp, Width};

const WIDTHS: [Width; 4] = [Width::W8, Width::W16, Width::W32, Width::W64];

/// `div128(hi, lo, d, signed) -> (q, r, fault)`
type Div128 = TypedFunc<(i64, i64, i64, i32), (i64, i64, i32)>;

struct Helpers {
    store: Store<()>,
    instance: Instance,
    flags: TypedFunc<(i32, i64, i64, i64), i64>,
    div128: Div128,
}

impl Helpers {
    fn new() -> Self {
        let mut config = Config::new();
        config.wasm_multi_value(true);
        let engine = Engine::new(&config).unwrap();
        let module = Module::new(&engine, x64wasm::emit::helpers_test_module()).unwrap();
        let mut store = Store::new(&engine, ());
        let instance = Instance::new(&mut store, &module, &[]).unwrap();
        let flags = instance.get_typed_func(&mut store, "flags").unwrap();
        let div128 = instance.get_typed_func(&mut store, "div128").unwrap();
        Helpers {
            store,
            instance,
            flags,
            div128,
        }
    }

    fn flags(&mut self, op: FlagOp, w: Width, a: u64, b: u64, r: u64) -> u64 {
        self.flags
            .call(
                &mut self.store,
                (op.cc_op(w) as i32, a as i64, b as i64, r as i64),
            )
            .unwrap() as u64
    }

    fn div128(&mut self, hi: u64, lo: u64, d: u64, signed: bool) -> Option<(u64, u64)> {
        let (q, r, fault) = self
            .div128
            .call(
                &mut self.store,
                (hi as i64, lo as i64, d as i64, signed as i32),
            )
            .unwrap();
        (fault == 0).then_some((q as u64, r as u64))
    }
}

/// A small deterministic generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

/// Edge values of a width first, then random ones.
fn operands(w: Width, rng: &mut Rng) -> Vec<u64> {
    let m = w.mask();
    let s = w.sign_bit();
    let mut v = vec![
        0,
        1,
        2,
        0x0f,
        0x10,
        0x7f,
        0x80,
        0xff,
        s - 1,
        s,
        s + 1,
        m - 1,
        m,
        0x5555_5555_5555_5555,
        0xaaaa_aaaa_aaaa_aaaa,
    ];
    for _ in 0..24 {
        v.push(rng.next());
    }
    v.iter_mut().for_each(|x| *x &= m);
    v
}

fn signed(w: Width, x: u64) -> i128 {
    let n = w.bits();
    ((x as i128) << (128 - n)) >> (128 - n)
}

fn fits(w: Width, x: i128) -> bool {
    let n = w.bits();
    x >= -(1i128 << (n - 1)) && x < (1i128 << (n - 1))
}

/// PF, ZF, SF of a result.
fn szp(w: Width, r: u64) -> u64 {
    let mut f = 0;
    if (r & 0xff).count_ones().is_multiple_of(2) {
        f |= PF;
    }
    if r == 0 {
        f |= ZF;
    }
    if r & w.sign_bit() != 0 {
        f |= SF;
    }
    f
}

fn bit(cond: bool, flag: u64) -> u64 {
    if cond { flag } else { 0 }
}

/// Checks `flags` for a record against the SDM's flags under `defined`.
#[allow(clippy::too_many_arguments)]
fn check(
    h: &mut Helpers,
    op: FlagOp,
    w: Width,
    rec: (u64, u64, u64),
    expected: u64,
    defined: u64,
    what: &str,
) {
    let got = h.flags(op, w, rec.0, rec.1, rec.2);
    assert_eq!(
        got & defined,
        expected & defined,
        "{what}: {op:?}{} record {rec:x?}: got {got:#x}, expected {expected:#x} under {defined:#x}",
        w.bits()
    );
    assert_eq!(got & !STATUS, 0, "{what}: bits outside STATUS");
    // The reference model in ir.rs computes the same value, undefined bits
    // included (the static paths in chunks follow it).
    assert_eq!(got, op.eval(w, rec.0, rec.1, rec.2), "{what}: FlagOp::eval");
}

#[test]
fn flags_arithmetic_and_logic() {
    let mut h = Helpers::new();
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for w in WIDTHS {
        let m = w.mask() as u128;
        for &x in &operands(w, &mut rng) {
            for &y in &operands(w, &mut rng) {
                for carry in [0u64, 1] {
                    let (xu, yu, c) = (x as u128, y as u128, carry as u128);
                    // add / adc
                    let r = ((xu + yu + c) & m) as u64;
                    let f = szp(w, r)
                        | bit(xu + yu + c > m, CF)
                        | bit(!fits(w, signed(w, x) + signed(w, y) + c as i128), OF)
                        | bit((x & 0xf) + (y & 0xf) + carry > 0xf, AF);
                    let op = if carry == 0 { FlagOp::Add } else { FlagOp::Adc };
                    check(&mut h, op, w, (x, y, r), f, STATUS, "add");
                    check(&mut h, FlagOp::Adc, w, (x, y, r), f, STATUS, "adc");
                    // sub / sbb / cmp
                    let r = (xu.wrapping_sub(yu + c) & m) as u64;
                    let f = szp(w, r)
                        | bit(xu < yu + c, CF)
                        | bit(!fits(w, signed(w, x) - signed(w, y) - c as i128), OF)
                        | bit((x & 0xf) < (y & 0xf) + carry, AF);
                    let op = if carry == 0 { FlagOp::Sub } else { FlagOp::Sbb };
                    check(&mut h, op, w, (x, y, r), f, STATUS, "sub");
                    check(&mut h, FlagOp::Sbb, w, (x, y, r), f, STATUS, "sbb");
                }
                // and, or, xor, test (AF undefined)
                for r in [x & y, x | y, x ^ y] {
                    check(
                        &mut h,
                        FlagOp::Logic,
                        w,
                        (r, 0, r),
                        szp(w, r),
                        STATUS & !AF,
                        "logic",
                    );
                }
            }
            // inc, dec keep CF (`b` carries it)
            for cf in [0u64, 1] {
                let r = (x as u128 + 1) as u64 & w.mask();
                let f = szp(w, r)
                    | bit(cf != 0, CF)
                    | bit(!fits(w, signed(w, x) + 1), OF)
                    | bit(x & 0xf == 0xf, AF);
                check(&mut h, FlagOp::Inc, w, (x, cf, r), f, STATUS, "inc");
                let r = x.wrapping_sub(1) & w.mask();
                let f = szp(w, r)
                    | bit(cf != 0, CF)
                    | bit(!fits(w, signed(w, x) - 1), OF)
                    | bit(x & 0xf == 0, AF);
                check(&mut h, FlagOp::Dec, w, (x, cf, r), f, STATUS, "dec");
            }
            // neg: sub from 0
            let r = 0u64.wrapping_sub(x) & w.mask();
            let f = szp(w, r)
                | bit(x != 0, CF)
                | bit(!fits(w, -signed(w, x)), OF)
                | bit(x & 0xf != 0, AF);
            check(&mut h, FlagOp::Sub, w, (0, x, r), f, STATUS, "neg");
            // mul: CF = OF = overflow, the rest undefined
            for ovf in [0u64, 1] {
                check(
                    &mut h,
                    FlagOp::Mul,
                    w,
                    (ovf, 0, x),
                    bit(ovf != 0, CF | OF),
                    CF | OF,
                    "mul",
                );
            }
        }
    }
}

#[test]
fn flags_shifts() {
    let mut h = Helpers::new();
    let mut rng = Rng(0x0bad_cafe_f00d_d00d);
    for w in WIDTHS {
        let n = w.bits() as u64;
        let max_count = if w == Width::W64 { 63 } else { 31 };
        for &x in &operands(w, &mut rng) {
            for c in 1..=max_count {
                let of_mask = if c == 1 { OF } else { 0 };
                // shl
                let r = ((x as u128) << c) as u64 & w.mask();
                let cf = c <= n && (x >> (n - c)) & 1 != 0;
                let cf_mask = if c <= n { CF } else { 0 };
                let of = (r & w.sign_bit() != 0) != cf;
                let f = szp(w, r) | bit(cf, CF) | bit(of, OF);
                let defined = ZF | SF | PF | cf_mask | of_mask;
                check(&mut h, FlagOp::Shl, w, (x, c, r), f, defined, "shl");
                // shr
                let r = if c >= 64 { 0 } else { x >> c };
                let cf = c <= n && (x >> (c - 1)) & 1 != 0;
                let f = szp(w, r) | bit(cf, CF) | bit(x & w.sign_bit() != 0, OF);
                let defined = ZF | SF | PF | cf_mask | of_mask;
                check(&mut h, FlagOp::Shr, w, (x, c, r), f, defined, "shr");
                // sar: CF is the last bit out, the sign once c >= n
                let s = signed(w, x);
                let r = (s >> c) as u64 & w.mask();
                let cf = (s >> (c - 1)) & 1 != 0;
                let f = szp(w, r) | bit(cf, CF);
                check(
                    &mut h,
                    FlagOp::Sar,
                    w,
                    (x, c, r),
                    f,
                    ZF | SF | PF | CF | of_mask,
                    "sar",
                );
            }
        }
    }
}

#[test]
fn eager_records_are_returned_masked() {
    let mut h = Helpers::new();
    let got = h
        .flags
        .call(&mut h.store, (0, (0xffff_ffff_ffff_ffffu64) as i64, 0, 0))
        .unwrap() as u64;
    assert_eq!(got, STATUS);
}

#[test]
fn div128_matches_u128_and_i128() {
    let mut h = Helpers::new();
    let mut rng = Rng(0x5eed_5eed_5eed_5eed);
    let edges = [
        0u64,
        1,
        2,
        3,
        0x7fff_ffff,
        0x8000_0000,
        0xffff_ffff,
        0x1_0000_0000,
        0x7fff_ffff_ffff_ffff,
        0x8000_0000_0000_0000,
        0x8000_0000_0000_0001,
        0xffff_ffff_ffff_fffe,
        0xffff_ffff_ffff_ffff,
    ];
    let mut values: Vec<u64> = edges.to_vec();
    for _ in 0..40 {
        values.push(rng.next());
        values.push(rng.next() >> (rng.next() % 64));
    }
    let mut checked = 0;
    for &hi in &values {
        for &lo in &values {
            for &d in &values {
                let n = ((hi as u128) << 64) | lo as u128;
                // unsigned
                let expected = (d != 0)
                    .then(|| (n / d as u128, n % d as u128))
                    .filter(|&(q, _)| q <= u64::MAX as u128)
                    .map(|(q, r)| (q as u64, r as u64));
                assert_eq!(
                    h.div128(hi, lo, d, false),
                    expected,
                    "div {hi:#x}:{lo:#x} / {d:#x}"
                );
                // signed
                let (ns, ds) = (n as i128, d as i64 as i128);
                let expected = (ds != 0)
                    .then(|| ns.checked_div(ds).zip(ns.checked_rem(ds)))
                    .flatten()
                    .filter(|&(q, _)| q >= i64::MIN as i128 && q <= i64::MAX as i128)
                    .map(|(q, r)| (q as i64 as u64, r as i64 as u64));
                assert_eq!(
                    h.div128(hi, lo, d, true),
                    expected,
                    "idiv {hi:#x}:{lo:#x} / {d:#x}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 100_000);
    // INT64_MIN / -1 as a 128-bit dividend: RDX:RAX = -2^63.
    assert_eq!(h.div128(u64::MAX, 1 << 63, u64::MAX, true), None);
    // -2^63 / 1 fits; 2^63 / -1 = -2^63 fits.
    assert_eq!(h.div128(u64::MAX, 1 << 63, 1, true), Some((1 << 63, 0)));
    assert_eq!(h.div128(0, 1 << 63, u64::MAX, true), Some((1 << 63, 0)));
}

/// x86 condition `cc` (encoding order: o, no, b, ae, e, ne, be, a, s, ns,
/// p, np, l, ge, le, g) of materialized flags, from the SDM's definitions.
fn cond_of(cc: u32, f: u64) -> bool {
    let b = |m: u64| f & m != 0;
    let base = match cc >> 1 {
        0 => b(OF),
        1 => b(CF),
        2 => b(ZF),
        3 => b(CF) || b(ZF),
        4 => b(SF),
        5 => b(PF),
        6 => b(SF) != b(OF),
        _ => b(ZF) || b(SF) != b(OF),
    };
    base ^ (cc & 1 != 0)
}

/// Consistent records `(a, b, r)` of kind `op`: what the lifter writes.
fn records(op: FlagOp, w: Width, rng: &mut Rng) -> Vec<(u64, u64, u64)> {
    let m = w.mask();
    let max_count = if w == Width::W64 { 63 } else { 31 };
    let sext = |x: u64| (x ^ w.sign_bit()).wrapping_sub(w.sign_bit());
    let xs = operands(w, rng);
    let ys = operands(w, rng);
    let mut out = Vec::new();
    for &x in &xs {
        for &y in &ys {
            let c = y & 1;
            let n = 1 + y % max_count;
            out.push(match op {
                FlagOp::Add => (x, y, x.wrapping_add(y) & m),
                FlagOp::Sub => (x, y, x.wrapping_sub(y) & m),
                FlagOp::Logic => (x, 0, x),
                FlagOp::Adc => (x, y, x.wrapping_add(y).wrapping_add(c) & m),
                FlagOp::Sbb => (x, y, x.wrapping_sub(y).wrapping_sub(c) & m),
                FlagOp::Inc => (x, c, x.wrapping_add(1) & m),
                FlagOp::Dec => (x, c, x.wrapping_sub(1) & m),
                FlagOp::Shl => (x, n, (x << n) & m),
                FlagOp::Shr => (x, n, x >> n),
                FlagOp::Sar => (x, n, ((sext(x) as i64) >> n) as u64 & m),
                FlagOp::Mul => (c, 0, x),
            });
        }
    }
    out
}

/// Every condition through the static path (a known record kind) agrees
/// with the `flags` function, and the path from materialized flags with
/// the SDM's definitions.
#[test]
fn conditions_static_and_dynamic() {
    let mut h = Helpers::new();
    for c in 0..16u32 {
        let d: TypedFunc<i64, i32> = h
            .instance
            .get_typed_func(&mut h.store, &format!("d_{c}"))
            .unwrap();
        for bits in 0..64u64 {
            let mut f = 0;
            for (i, flag) in [CF, PF, AF, ZF, SF, OF].into_iter().enumerate() {
                if bits >> i & 1 != 0 {
                    f |= flag;
                }
            }
            let got = d.call(&mut h.store, f as i64).unwrap() != 0;
            assert_eq!(got, cond_of(c, f), "dynamic cond {c}, flags {f:#x}");
        }
    }
    let mut rng = Rng(0xc0de_c0de_1234_8765);
    let mut checked = 0;
    for op in FlagOp::ALL {
        for w in WIDTHS {
            let recs = records(op, w, &mut rng);
            for c in 0..16u32 {
                let name = format!("s_{}_{}_{c}", op.kind(), w.index());
                let s: TypedFunc<(i64, i64, i64), i32> =
                    h.instance.get_typed_func(&mut h.store, &name).unwrap();
                for &(a, b, r) in &recs {
                    let f = h.flags(op, w, a, b, r);
                    let got = s
                        .call(&mut h.store, (a as i64, b as i64, r as i64))
                        .unwrap()
                        != 0;
                    assert_eq!(
                        got,
                        cond_of(c, f),
                        "{op:?} {w:?} cond {c}: a={a:#x} b={b:#x} r={r:#x}"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 100_000, "{checked}");
}
