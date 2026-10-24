//! Helper functions that follow the chunks (`abi.md`, "Inside the module"):
//! emitted only when used, not in the table.

use wasm_encoder::{BlockType, Function, InstructionSink, ValType};

/// Local names of `div128`, in index order.
pub(super) const DIV128_LOCALS: [&str; 19] = [
    "hi", "lo", "d", "signed", "q", "r", "s", "vn1", "vn0", "un32", "un10", "un1", "un0", "q1",
    "q0", "rhat", "un21", "neg_n", "neg_d",
];

const HI: u32 = 0;
const LO: u32 = 1;
const D: u32 = 2;
const SIGNED: u32 = 3;
const Q: u32 = 4;
const R: u32 = 5;
const S: u32 = 6;
const VN1: u32 = 7;
const VN0: u32 = 8;
const UN32: u32 = 9;
const UN10: u32 = 10;
const UN1: u32 = 11;
const UN0: u32 = 12;
const Q1: u32 = 13;
const Q0: u32 = 14;
const RHAT: u32 = 15;
const UN21: u32 = 16;
const NEG_N: u32 = 17;
const NEG_D: u32 = 18;

const B: i64 = 1 << 32;

fn fault(f: &mut InstructionSink<'_>) {
    f.i64_const(0);
    f.i64_const(0);
    f.i32_const(1);
    f.return_();
}

/// `q = q - 1; rhat = rhat + vn1` if `q >= 2^32 || q * vn0 > (rhat << 32) + un`.
fn correct(f: &mut InstructionSink<'_>, q: u32, un: u32) {
    f.local_get(q);
    f.i64_const(B);
    f.i64_ge_u();
    f.local_get(q);
    f.local_get(VN0);
    f.i64_mul();
    f.local_get(RHAT);
    f.i64_const(32);
    f.i64_shl();
    f.local_get(un);
    f.i64_add();
    f.i64_gt_u();
    f.i32_or();
}

/// Applies [`correct`] up to twice, the second time only while `rhat` is
/// below 2^32 (Hacker's Delight, `divlu`).
fn corrections(f: &mut InstructionSink<'_>, q: u32, un: u32) {
    let step = |f: &mut InstructionSink<'_>| {
        f.local_get(q);
        f.i64_const(1);
        f.i64_sub();
        f.local_set(q);
        f.local_get(RHAT);
        f.local_get(VN1);
        f.i64_add();
        f.local_set(RHAT);
    };
    correct(f, q, un);
    f.if_(BlockType::Empty);
    step(f);
    f.local_get(RHAT);
    f.i64_const(B);
    f.i64_lt_u();
    f.if_(BlockType::Empty);
    correct(f, q, un);
    f.if_(BlockType::Empty);
    step(f);
    f.end();
    f.end();
    f.end();
}

/// `div128(hi: i64, lo: i64, d: i64, signed: i32) -> (q: i64, r: i64,
/// fault: i32)`: divides the 128-bit `hi:lo` by `d`, unsigned or signed
/// (two's complement). `fault` is 1, and q and r are 0, if `d` is 0 or the
/// quotient doesn't fit in 64 bits (x86 #DE). Never traps.
pub(super) fn emit_div128() -> Function {
    let mut func = Function::new([(13, ValType::I64), (2, ValType::I32)]);
    let mut f = func.instructions();
    // Signed: divide magnitudes.
    f.local_get(SIGNED);
    f.if_(BlockType::Empty);
    {
        f.local_get(HI);
        f.i64_const(0);
        f.i64_lt_s();
        f.local_tee(NEG_N);
        f.if_(BlockType::Empty);
        // hi:lo = -(hi:lo): hi = !hi + (lo == 0), lo = -lo
        f.local_get(HI);
        f.i64_const(-1);
        f.i64_xor();
        f.local_get(LO);
        f.i64_eqz();
        f.i64_extend_i32_u();
        f.i64_add();
        f.local_set(HI);
        f.i64_const(0);
        f.local_get(LO);
        f.i64_sub();
        f.local_set(LO);
        f.end();
        f.local_get(D);
        f.i64_const(0);
        f.i64_lt_s();
        f.local_tee(NEG_D);
        f.if_(BlockType::Empty);
        f.i64_const(0);
        f.local_get(D);
        f.i64_sub();
        f.local_set(D);
        f.end();
    }
    f.end();
    // d == 0 or hi >= d: the quotient doesn't fit.
    f.local_get(D);
    f.i64_eqz();
    f.local_get(HI);
    f.local_get(D);
    f.i64_ge_u();
    f.i32_or();
    f.if_(BlockType::Empty);
    fault(&mut f);
    f.end();
    f.local_get(HI);
    f.i64_eqz();
    f.if_(BlockType::Empty);
    {
        f.local_get(LO);
        f.local_get(D);
        f.i64_div_u();
        f.local_set(Q);
        f.local_get(LO);
        f.local_get(D);
        f.i64_rem_u();
        f.local_set(R);
    }
    f.else_();
    {
        // Normalize: d has its top bit set.
        f.local_get(D);
        f.i64_clz();
        f.local_tee(S);
        f.local_get(D);
        f.local_get(S);
        f.i64_shl();
        f.local_set(D);
        f.drop();
        f.local_get(D);
        f.i64_const(32);
        f.i64_shr_u();
        f.local_set(VN1);
        f.local_get(D);
        f.i64_const(0xffff_ffff);
        f.i64_and();
        f.local_set(VN0);
        // un32 = hi << s | (lo >> (64 - s) when s != 0)
        f.local_get(HI);
        f.local_get(S);
        f.i64_shl();
        f.local_get(LO);
        f.i64_const(64);
        f.local_get(S);
        f.i64_sub();
        f.i64_shr_u();
        f.i64_const(0);
        f.local_get(S);
        f.i64_sub();
        f.i64_const(63);
        f.i64_shr_s();
        f.i64_and();
        f.i64_or();
        f.local_set(UN32);
        f.local_get(LO);
        f.local_get(S);
        f.i64_shl();
        f.local_tee(UN10);
        f.i64_const(32);
        f.i64_shr_u();
        f.local_set(UN1);
        f.local_get(UN10);
        f.i64_const(0xffff_ffff);
        f.i64_and();
        f.local_set(UN0);
        // First digit.
        f.local_get(UN32);
        f.local_get(VN1);
        f.i64_div_u();
        f.local_tee(Q1);
        f.local_get(VN1);
        f.i64_mul();
        f.local_set(RHAT);
        f.local_get(UN32);
        f.local_get(RHAT);
        f.i64_sub();
        f.local_set(RHAT);
        corrections(&mut f, Q1, UN1);
        // un21 = un32 * 2^32 + un1 - q1 * d
        f.local_get(UN32);
        f.i64_const(32);
        f.i64_shl();
        f.local_get(UN1);
        f.i64_add();
        f.local_get(Q1);
        f.local_get(D);
        f.i64_mul();
        f.i64_sub();
        f.local_set(UN21);
        // Second digit.
        f.local_get(UN21);
        f.local_get(VN1);
        f.i64_div_u();
        f.local_tee(Q0);
        f.local_get(VN1);
        f.i64_mul();
        f.local_set(RHAT);
        f.local_get(UN21);
        f.local_get(RHAT);
        f.i64_sub();
        f.local_set(RHAT);
        corrections(&mut f, Q0, UN0);
        // r = (un21 * 2^32 + un0 - q0 * d) >> s; q = q1 * 2^32 + q0
        f.local_get(UN21);
        f.i64_const(32);
        f.i64_shl();
        f.local_get(UN0);
        f.i64_add();
        f.local_get(Q0);
        f.local_get(D);
        f.i64_mul();
        f.i64_sub();
        f.local_get(S);
        f.i64_shr_u();
        f.local_set(R);
        f.local_get(Q1);
        f.i64_const(32);
        f.i64_shl();
        f.local_get(Q0);
        f.i64_add();
        f.local_set(Q);
    }
    f.end();
    // Signed: the quotient's range and the signs.
    f.local_get(SIGNED);
    f.if_(BlockType::Empty);
    {
        f.local_get(NEG_N);
        f.local_get(NEG_D);
        f.i32_ne();
        f.if_(BlockType::Empty);
        {
            // |q| <= 2^63
            f.local_get(Q);
            f.i64_const(i64::MIN);
            f.i64_gt_u();
            f.if_(BlockType::Empty);
            fault(&mut f);
            f.end();
            f.i64_const(0);
            f.local_get(Q);
            f.i64_sub();
            f.local_set(Q);
        }
        f.else_();
        {
            // q <= 2^63 - 1
            f.local_get(Q);
            f.i64_const(0);
            f.i64_lt_s();
            f.if_(BlockType::Empty);
            fault(&mut f);
            f.end();
        }
        f.end();
        f.local_get(NEG_N);
        f.if_(BlockType::Empty);
        f.i64_const(0);
        f.local_get(R);
        f.i64_sub();
        f.local_set(R);
        f.end();
    }
    f.end();
    f.local_get(Q);
    f.local_get(R);
    f.i32_const(0);
    f.end();
    func
}
