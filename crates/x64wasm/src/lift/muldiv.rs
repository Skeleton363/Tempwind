//! Multiplication and division: `mul`, `imul` (one, two and three
//! operands), `div`, `idiv` (`docs/design/translator.md`, "Integer
//! results" and "Faults").

use iced_x86::Mnemonic;

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::abi::FAULT_DIVIDE;
use crate::ir::{BinOp, FlagOp, Op, RAX, RDX, Value, Width};

impl Lower<'_> {
    /// The high 64 bits of the unsigned 128-bit product `a * b`, from four
    /// 32-bit partial products.
    fn mul_high_u(&mut self, a: Value, b: Value) -> Value {
        const LO: Value = Value::Imm(0xffff_ffff);
        const S32: Value = Value::Imm(32);
        let a_lo = self.bin(BinOp::And, a, LO);
        let a_hi = self.bin(BinOp::ShrU, a, S32);
        let b_lo = self.bin(BinOp::And, b, LO);
        let b_hi = self.bin(BinOp::ShrU, b, S32);
        let ll = self.bin(BinOp::Mul, a_lo, b_lo);
        let lh = self.bin(BinOp::Mul, a_lo, b_hi);
        let hl = self.bin(BinOp::Mul, a_hi, b_lo);
        let hh = self.bin(BinOp::Mul, a_hi, b_hi);
        // The carry out of the low half: (ll >> 32) + lo(lh) + lo(hl) < 3 * 2^32.
        let mid = self.bin(BinOp::ShrU, ll, S32);
        let lh_lo = self.bin(BinOp::And, lh, LO);
        let hl_lo = self.bin(BinOp::And, hl, LO);
        let mid = self.bin(BinOp::Add, mid, lh_lo);
        let mid = self.bin(BinOp::Add, mid, hl_lo);
        let lh_hi = self.bin(BinOp::ShrU, lh, S32);
        let hl_hi = self.bin(BinOp::ShrU, hl, S32);
        let mid_hi = self.bin(BinOp::ShrU, mid, S32);
        let h = self.bin(BinOp::Add, hh, lh_hi);
        let h = self.bin(BinOp::Add, h, hl_hi);
        self.bin(BinOp::Add, h, mid_hi)
    }

    /// The high 64 bits of the signed 128-bit product: the unsigned high
    /// half minus `(a >>s 63) & b` and `(b >>s 63) & a`.
    fn mul_high_s(&mut self, a: Value, b: Value) -> Value {
        let h = self.mul_high_u(a, b);
        let sa = self.bin(BinOp::ShrS, a, Value::Imm(63));
        let sa = self.bin(BinOp::And, sa, b);
        let sb = self.bin(BinOp::ShrS, b, Value::Imm(63));
        let sb = self.bin(BinOp::And, sb, a);
        let h = self.bin(BinOp::Sub, h, sa);
        self.bin(BinOp::Sub, h, sb)
    }

    /// A signed product of `x` and `y` (zero-extended from `w`): its low
    /// half (zero-extended), 1 if the full product doesn't fit in `w`, and
    /// the high half (64-bit) or the whole sign-extended product (narrower).
    fn imul_lo(&mut self, w: Width, x: Value, y: Value) -> (Value, Value, Value) {
        if w == Width::W64 {
            let lo = self.bin(BinOp::Mul, x, y);
            let hi = self.mul_high_s(x, y);
            let sign = self.bin(BinOp::ShrS, lo, Value::Imm(63));
            let of = self.bin(BinOp::Ne, hi, sign);
            (lo, of, hi)
        } else {
            // The product of the sign-extended operands is exact in 64 bits.
            let sx = self.sext(x, w);
            let sy = self.sext(y, w);
            let p = self.bin(BinOp::Mul, sx, sy);
            let lo = self.mask(p, w);
            let back = self.sext(lo, w);
            let of = self.bin(BinOp::Ne, p, back);
            (lo, of, p)
        }
    }

    /// `mul r/m` and the one-operand `imul r/m`: the double-width product of
    /// the accumulator into AX, DX:AX, EDX:EAX or RDX:RAX.
    pub(super) fn mul_acc(&mut self, signed: bool) -> Lowered<Flow> {
        let w = self.width_of(0)?;
        let bits = w.bits() as u64;
        let src = self.read(0, w)?;
        let acc = self.read_reg(RAX, w);
        let (lo, hi, of) = match (w, signed) {
            (Width::W64, false) => {
                let lo = self.bin(BinOp::Mul, acc, src);
                let hi = self.mul_high_u(acc, src);
                let of = self.bin(BinOp::Ne, hi, Value::Imm(0));
                (lo, hi, of)
            }
            (Width::W64, true) => {
                let (lo, of, hi) = self.imul_lo(w, acc, src);
                (lo, hi, of)
            }
            (_, false) => {
                let p = self.bin(BinOp::Mul, acc, src);
                let lo = self.mask(p, w);
                let hi = self.bin(BinOp::ShrU, p, Value::Imm(bits));
                let of = self.bin(BinOp::Ne, hi, Value::Imm(0));
                (lo, hi, of)
            }
            (_, true) => {
                let (lo, of, p) = self.imul_lo(w, acc, src);
                let hi = self.bin(BinOp::ShrU, p, Value::Imm(bits));
                (lo, self.mask(hi, w), of)
            }
        };
        self.flags(FlagOp::Mul, w, of, Value::Imm(0), lo);
        if w == Width::W8 {
            // AX = AH:AL
            let ah = self.bin(BinOp::Shl, hi, Value::Imm(8));
            let ax = self.bin(BinOp::Or, ah, lo);
            self.write_reg(RAX, Width::W16, ax);
        } else {
            self.write_reg(RAX, w, lo);
            self.write_reg(RDX, w, hi);
        }
        Ok(Flow::Next)
    }

    /// `imul`: one operand (into the accumulator), two (`r *= r/m`) or
    /// three (`r = r/m * imm`); the last two keep the low half.
    pub(super) fn imul(&mut self) -> Lowered<Flow> {
        let (x, y) = match self.insn.op_count() {
            1 => return self.mul_acc(true),
            2 => (0, 1),
            3 => (1, 2),
            _ => return Err(Unsupported),
        };
        let w = self.width_of(0)?;
        let x = self.read(x, w)?;
        let y = self.read(y, w)?;
        let (lo, of, _) = self.imul_lo(w, x, y);
        self.flags(FlagOp::Mul, w, of, Value::Imm(0), lo);
        self.write(0, w, lo)?;
        Ok(Flow::Next)
    }

    /// `div`, `idiv`: #DE for a zero divisor or a quotient that doesn't fit.
    /// The flags are undefined and the record stays as it is.
    pub(super) fn div(&mut self, m: Mnemonic) -> Lowered<Flow> {
        let signed = m == Mnemonic::Idiv;
        let w = self.width_of(0)?;
        let bits = w.bits() as u64;
        let d = self.read(0, w)?;
        if w == Width::W64 {
            let [q, r, fault] = [self.temp(), self.temp(), self.temp()];
            self.push_op(Op::DivWide {
                q,
                r,
                fault,
                hi: Value::Reg(RDX),
                lo: Value::Reg(RAX),
                d,
                signed,
            });
            self.fault_if(Value::Temp(fault), FAULT_DIVIDE);
            self.set_reg(RAX, Value::Temp(q));
            self.set_reg(RDX, Value::Temp(r));
            return Ok(Flow::Next);
        }
        // The dividend in 64 bits: AX, DX:AX or EDX:EAX.
        let (n, hi) = if w == Width::W8 {
            let ax = self.read_reg(RAX, Width::W16);
            let ah = self.bin(BinOp::ShrU, ax, Value::Imm(8));
            (ax, ah)
        } else {
            let lo = self.read_reg(RAX, w);
            let hi = self.read_reg(RDX, w);
            let h = self.bin(BinOp::Shl, hi, Value::Imm(bits));
            (self.bin(BinOp::Or, h, lo), hi)
        };
        let (q, r) = if signed {
            let nw = match w {
                Width::W8 => Width::W16,
                Width::W16 => Width::W32,
                _ => Width::W64,
            };
            let n = self.sext(n, nw);
            let d = self.sext(d, w);
            let zero = self.bin(BinOp::Eq, d, Value::Imm(0));
            let fault = if w == Width::W32 {
                // i64::MIN / -1 would trap in wasm (its quotient doesn't fit anyway).
                let min = self.bin(BinOp::Eq, n, Value::Imm(1 << 63));
                let m1 = self.bin(BinOp::Eq, d, Value::Imm(u64::MAX));
                let both = self.bin(BinOp::And, min, m1);
                self.bin(BinOp::Or, zero, both)
            } else {
                zero
            };
            self.fault_if(fault, FAULT_DIVIDE);
            let q = self.bin(BinOp::DivS, n, d);
            let back = self.sext(q, w);
            let fault = self.bin(BinOp::Ne, q, back);
            self.fault_if(fault, FAULT_DIVIDE);
            let r = self.bin(BinOp::RemS, n, d);
            (self.mask(q, w), self.mask(r, w))
        } else {
            // hi >= d covers d = 0 and a quotient wider than the operand.
            let fits = self.bin(BinOp::LtU, hi, d);
            let fault = self.bin(BinOp::Xor, fits, Value::Imm(1));
            self.fault_if(fault, FAULT_DIVIDE);
            (self.bin(BinOp::DivU, n, d), self.bin(BinOp::RemU, n, d))
        };
        if w == Width::W8 {
            let ah = self.bin(BinOp::Shl, r, Value::Imm(8));
            let ax = self.bin(BinOp::Or, ah, q);
            self.write_reg(RAX, Width::W16, ax);
        } else {
            self.write_reg(RAX, w, q);
            self.write_reg(RDX, w, r);
        }
        Ok(Flow::Next)
    }
}
