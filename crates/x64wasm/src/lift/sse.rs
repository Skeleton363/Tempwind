//! SSE and SSE2 without floating-point arithmetic: moves, bitwise ops,
//! packed integer arithmetic, compares, shifts, shuffles, unpacks, packs,
//! extracts and inserts, `ldmxcsr`/`stmxcsr` (`docs/design/translator.md`,
//! "SSE and SSE2 without floating-point arithmetic"). The XMM registers
//! live in the state block; lane i of every wasm shape is element i.

use iced_x86::{Code, Mnemonic, OpKind, Register};

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::abi::{self, FAULT_GENERAL_PROTECTION};
use crate::ir::{
    BinOp, Lanes, Op, VBinOp, VExtractOp, VLoc, VShiftOp, VTemp, VWidth, Value, Width,
};

/// Interleaves the low (`high` false) or high halves of two vectors in
/// units of `unit` bytes: dst, src, dst, src ...
fn unpack(unit: usize, high: bool) -> [u8; 16] {
    let mut lanes = [0u8; 16];
    let base = if high { 8 } else { 0 };
    for (i, l) in lanes.iter_mut().enumerate() {
        let k = i / unit; // output element
        let from_src = k % 2 == 1;
        let elem = base / unit + k / 2;
        let byte = elem * unit + i % unit;
        *l = (byte + if from_src { 16 } else { 0 }) as u8;
    }
    lanes
}

/// Byte lanes for 32-bit elements `sel` (4 indexes into a 0-31 byte space
/// in units of dwords: 0-3 dst, 4-7 src).
fn dwords(sel: [u8; 4]) -> [u8; 16] {
    let mut lanes = [0u8; 16];
    for (i, l) in lanes.iter_mut().enumerate() {
        *l = sel[i / 4] * 4 + (i % 4) as u8;
    }
    lanes
}

/// Byte lanes for 16-bit elements `sel` (8 indexes into words 0-15).
fn words(sel: [u8; 8]) -> [u8; 16] {
    let mut lanes = [0u8; 16];
    for (i, l) in lanes.iter_mut().enumerate() {
        *l = sel[i / 2] * 2 + (i % 2) as u8;
    }
    lanes
}

/// The mnemonics this module lowers (with XMM operands only: the MMX forms
/// share them and stay unsupported).
pub(super) fn is_sse(m: Mnemonic) -> bool {
    use Mnemonic as M;
    matches!(
        m,
        M::Movdqa
            | M::Movdqu
            | M::Movaps
            | M::Movups
            | M::Movapd
            | M::Movupd
            | M::Movntdq
            | M::Movntps
            | M::Movntpd
            | M::Movnti
            | M::Movd
            | M::Movq
            | M::Movss
            | M::Movsd
            | M::Movhps
            | M::Movhpd
            | M::Movlps
            | M::Movlpd
            | M::Movhlps
            | M::Movlhps
            | M::Pand
            | M::Andps
            | M::Andpd
            | M::Por
            | M::Orps
            | M::Orpd
            | M::Pxor
            | M::Xorps
            | M::Xorpd
            | M::Pandn
            | M::Andnps
            | M::Andnpd
            | M::Paddb
            | M::Paddw
            | M::Paddd
            | M::Paddq
            | M::Psubb
            | M::Psubw
            | M::Psubd
            | M::Psubq
            | M::Paddsb
            | M::Paddsw
            | M::Paddusb
            | M::Paddusw
            | M::Psubsb
            | M::Psubsw
            | M::Psubusb
            | M::Psubusw
            | M::Pavgb
            | M::Pavgw
            | M::Pminub
            | M::Pmaxub
            | M::Pminsw
            | M::Pmaxsw
            | M::Pmullw
            | M::Pmulhw
            | M::Pmulhuw
            | M::Pmuludq
            | M::Pmaddwd
            | M::Psadbw
            | M::Pcmpeqb
            | M::Pcmpeqw
            | M::Pcmpeqd
            | M::Pcmpgtb
            | M::Pcmpgtw
            | M::Pcmpgtd
            | M::Packsswb
            | M::Packuswb
            | M::Packssdw
            | M::Psllw
            | M::Pslld
            | M::Psllq
            | M::Psrlw
            | M::Psrld
            | M::Psrlq
            | M::Psraw
            | M::Psrad
            | M::Pslldq
            | M::Psrldq
            | M::Pshufd
            | M::Pshuflw
            | M::Pshufhw
            | M::Shufps
            | M::Shufpd
            | M::Punpcklbw
            | M::Punpcklwd
            | M::Punpckldq
            | M::Punpcklqdq
            | M::Punpckhbw
            | M::Punpckhwd
            | M::Punpckhdq
            | M::Punpckhqdq
            | M::Unpcklps
            | M::Unpcklpd
            | M::Unpckhps
            | M::Unpckhpd
            | M::Pextrw
            | M::Pinsrw
            | M::Pmovmskb
            | M::Movmskps
            | M::Movmskpd
            | M::Ldmxcsr
            | M::Stmxcsr
    )
}

impl Lower<'_> {
    /// The state-block slot of an XMM register operand.
    fn xmm(&self, operand: u32) -> Lowered<u64> {
        if self.insn.op_kind(operand) != OpKind::Register {
            return Err(Unsupported);
        }
        let r = self.insn.op_register(operand);
        let i = (r as u32).wrapping_sub(Register::XMM0 as u32);
        if !r.is_xmm() || i >= 16 {
            return Err(Unsupported);
        }
        Ok(abi::STATE_XMM + 16 * i as u64)
    }

    fn is_mem(&self, operand: u32) -> bool {
        self.insn.op_kind(operand) == OpKind::Memory
    }

    /// The memory operand's address; with `aligned`, #GP(0) unless it is
    /// 16-byte aligned (legacy-SSE m128 operands).
    fn sse_addr(&mut self, aligned: bool) -> Lowered<Value> {
        let addr = self.ea()?;
        if aligned {
            let low = self.bin(BinOp::And, addr, Value::Imm(15));
            self.fault_if(low, FAULT_GENERAL_PROTECTION);
        }
        Ok(addr)
    }

    fn vload(&mut self, width: VWidth, loc: VLoc) -> VTemp {
        let dst = self.vtemp();
        self.push_op(Op::VLoad { dst, width, loc });
        dst
    }

    fn vstore(&mut self, loc: VLoc, width: VWidth, src: VTemp) {
        self.push_op(Op::VStore { loc, width, src });
    }

    fn vbin(&mut self, op: VBinOp, a: VTemp, b: VTemp) -> VTemp {
        let dst = self.vtemp();
        self.push_op(Op::VBin { dst, op, a, b });
        dst
    }

    fn vshuffle(&mut self, a: VTemp, b: VTemp, lanes: [u8; 16]) -> VTemp {
        let dst = self.vtemp();
        self.push_op(Op::VShuffle { dst, a, b, lanes });
        dst
    }

    fn vzero(&mut self) -> VTemp {
        let dst = self.vtemp();
        self.push_op(Op::VConst { dst, value: 0 });
        dst
    }

    /// Reads an XMM register or memory operand (128-bit memory operands
    /// aligned unless `unaligned`).
    fn vsrc(&mut self, operand: u32, width: VWidth, unaligned: bool) -> Lowered<VTemp> {
        if self.is_mem(operand) {
            let aligned = width == VWidth::W128 && !unaligned;
            let addr = self.sse_addr(aligned)?;
            Ok(self.vload(width, VLoc::Mem(addr)))
        } else {
            let slot = self.xmm(operand)?;
            Ok(self.vload(width, VLoc::State(slot as u32)))
        }
    }

    /// The operand an instruction writes: its register slot, or memory.
    fn vdst(&mut self, operand: u32, aligned: bool) -> Lowered<VLoc> {
        if self.is_mem(operand) {
            Ok(VLoc::Mem(self.sse_addr(aligned)?))
        } else {
            Ok(VLoc::State(self.xmm(operand)? as u32))
        }
    }

    pub(super) fn sse(&mut self, m: Mnemonic) -> Lowered<Flow> {
        use Mnemonic as M;
        use VBinOp as V;
        let bin = match m {
            M::Pand | M::Andps | M::Andpd => Some(V::And),
            M::Por | M::Orps | M::Orpd => Some(V::Or),
            M::Pxor | M::Xorps | M::Xorpd => Some(V::Xor),
            M::Pandn | M::Andnps | M::Andnpd => Some(V::AndNot),
            M::Paddb => Some(V::Add(Lanes::B)),
            M::Paddw => Some(V::Add(Lanes::W)),
            M::Paddd => Some(V::Add(Lanes::D)),
            M::Paddq => Some(V::Add(Lanes::Q)),
            M::Psubb => Some(V::Sub(Lanes::B)),
            M::Psubw => Some(V::Sub(Lanes::W)),
            M::Psubd => Some(V::Sub(Lanes::D)),
            M::Psubq => Some(V::Sub(Lanes::Q)),
            M::Paddsb => Some(V::AddSatS(Lanes::B)),
            M::Paddsw => Some(V::AddSatS(Lanes::W)),
            M::Paddusb => Some(V::AddSatU(Lanes::B)),
            M::Paddusw => Some(V::AddSatU(Lanes::W)),
            M::Psubsb => Some(V::SubSatS(Lanes::B)),
            M::Psubsw => Some(V::SubSatS(Lanes::W)),
            M::Psubusb => Some(V::SubSatU(Lanes::B)),
            M::Psubusw => Some(V::SubSatU(Lanes::W)),
            M::Pavgb => Some(V::AvgU(Lanes::B)),
            M::Pavgw => Some(V::AvgU(Lanes::W)),
            M::Pminub => Some(V::MinU(Lanes::B)),
            M::Pmaxub => Some(V::MaxU(Lanes::B)),
            M::Pminsw => Some(V::MinS(Lanes::W)),
            M::Pmaxsw => Some(V::MaxS(Lanes::W)),
            M::Pmullw => Some(V::MulLo16),
            M::Pmulhw => Some(V::MulHiS16),
            M::Pmulhuw => Some(V::MulHiU16),
            M::Pmuludq => Some(V::MulU32),
            M::Pmaddwd => Some(V::MaddWd),
            M::Psadbw => Some(V::SadBw),
            M::Pcmpeqb => Some(V::Eq(Lanes::B)),
            M::Pcmpeqw => Some(V::Eq(Lanes::W)),
            M::Pcmpeqd => Some(V::Eq(Lanes::D)),
            M::Pcmpgtb => Some(V::GtS(Lanes::B)),
            M::Pcmpgtw => Some(V::GtS(Lanes::W)),
            M::Pcmpgtd => Some(V::GtS(Lanes::D)),
            M::Packsswb => Some(V::PackSsWb),
            M::Packuswb => Some(V::PackUsWb),
            M::Packssdw => Some(V::PackSsDw),
            _ => None,
        };
        if let Some(op) = bin {
            return self.sse_packed(|l, a, b| l.vbin(op, a, b));
        }
        let unpacks = match m {
            M::Punpcklbw => Some(unpack(1, false)),
            M::Punpcklwd => Some(unpack(2, false)),
            M::Punpckldq | M::Unpcklps => Some(unpack(4, false)),
            M::Punpcklqdq | M::Unpcklpd => Some(unpack(8, false)),
            M::Punpckhbw => Some(unpack(1, true)),
            M::Punpckhwd => Some(unpack(2, true)),
            M::Punpckhdq | M::Unpckhps => Some(unpack(4, true)),
            M::Punpckhqdq | M::Unpckhpd => Some(unpack(8, true)),
            _ => None,
        };
        if let Some(lanes) = unpacks {
            return self.sse_packed(|l, a, b| l.vshuffle(a, b, lanes));
        }
        match m {
            M::Movdqa | M::Movaps | M::Movapd | M::Movntdq | M::Movntps | M::Movntpd => {
                self.sse_mov128(true)
            }
            M::Movdqu | M::Movups | M::Movupd => self.sse_mov128(false),
            M::Movnti => {
                let w = self.width_of(0)?;
                let v = self.read(1, w)?;
                self.write(0, w, v)?;
                Ok(Flow::Next)
            }
            M::Movd | M::Movq => self.sse_movdq(),
            M::Movss | M::Movsd => self.sse_movs(if m == M::Movss {
                VWidth::W32
            } else {
                VWidth::W64
            }),
            M::Movhps | M::Movhpd | M::Movlps | M::Movlpd => {
                let high = matches!(m, M::Movhps | M::Movhpd);
                self.sse_half(high)
            }
            M::Movhlps | M::Movlhps => {
                // movhlps: dst.lo = src.hi; movlhps: dst.hi = src.lo.
                let dst = self.xmm(0)?;
                let src = self.xmm(1)?;
                let (from, to) = if m == M::Movhlps {
                    (src + 8, dst)
                } else {
                    (src, dst + 8)
                };
                let v = self.load_state(Width::W64, from);
                self.store_state(Width::W64, to, v);
                Ok(Flow::Next)
            }
            M::Psllw
            | M::Pslld
            | M::Psllq
            | M::Psrlw
            | M::Psrld
            | M::Psrlq
            | M::Psraw
            | M::Psrad => self.sse_shift(m),
            M::Pslldq | M::Psrldq => {
                let slot = self.xmm(0)?;
                let n = self.insn.immediate8() as usize;
                let a = self.vload(VWidth::W128, VLoc::State(slot as u32));
                let z = self.vzero();
                let mut lanes = [16u8; 16];
                for (i, l) in lanes.iter_mut().enumerate() {
                    let from = if m == M::Pslldq {
                        i.checked_sub(n)
                    } else {
                        Some(i + n).filter(|&j| j < 16)
                    };
                    if let Some(j) = from {
                        *l = j as u8;
                    }
                }
                let r = self.vshuffle(a, z, lanes);
                self.vstore(VLoc::State(slot as u32), VWidth::W128, r);
                Ok(Flow::Next)
            }
            M::Pshufd | M::Pshuflw | M::Pshufhw => {
                let imm = self.insn.immediate8();
                let sel = |k: u8| (imm >> (2 * k)) & 3;
                let lanes = match m {
                    M::Pshufd => dwords([sel(0), sel(1), sel(2), sel(3)]),
                    M::Pshuflw => words([sel(0), sel(1), sel(2), sel(3), 4, 5, 6, 7]),
                    _ => words([0, 1, 2, 3, 4 + sel(0), 4 + sel(1), 4 + sel(2), 4 + sel(3)]),
                };
                let dst = self.xmm(0)?;
                let src = self.vsrc(1, VWidth::W128, false)?;
                let r = self.vshuffle(src, src, lanes);
                self.vstore(VLoc::State(dst as u32), VWidth::W128, r);
                Ok(Flow::Next)
            }
            M::Shufps | M::Shufpd => {
                let imm = self.insn.immediate8();
                let lanes = if m == M::Shufps {
                    let sel = |k: u8| (imm >> (2 * k)) & 3;
                    dwords([sel(0), sel(1), 4 + sel(2), 4 + sel(3)])
                } else {
                    let q0 = imm & 1;
                    let q1 = 2 + ((imm >> 1) & 1);
                    dwords([2 * q0, 2 * q0 + 1, 2 * q1, 2 * q1 + 1])
                };
                self.sse_packed(|l, a, b| l.vshuffle(a, b, lanes))
            }
            M::Pextrw => {
                // The SSE4.1 form with a memory destination is outside the
                // CPUID profile.
                if !matches!(
                    self.insn.code(),
                    Code::Pextrw_r32_xmm_imm8 | Code::Pextrw_r64_xmm_imm8
                ) {
                    return Err(Unsupported);
                }
                let slot = self.xmm(1)?;
                let k = (self.insn.immediate8() & 7) as u64;
                let v = self.load_state(Width::W16, slot + 2 * k);
                let w = self.width_of(0)?;
                self.write(0, w, v)?;
                Ok(Flow::Next)
            }
            M::Pinsrw => {
                let slot = self.xmm(0)?;
                let k = (self.insn.immediate8() & 7) as u64;
                let v = self.read(1, Width::W16)?;
                self.store_state(Width::W16, slot + 2 * k, v);
                Ok(Flow::Next)
            }
            M::Pmovmskb | M::Movmskps | M::Movmskpd => {
                let lanes = match m {
                    M::Pmovmskb => Lanes::B,
                    M::Movmskps => Lanes::D,
                    _ => Lanes::Q,
                };
                let src = self.vsrc(1, VWidth::W128, false)?;
                let dst = self.temp();
                self.push_op(Op::VExtract {
                    dst,
                    op: VExtractOp::Bitmask(lanes),
                    src,
                });
                let w = self.width_of(0)?;
                self.write(0, w, Value::Temp(dst))?;
                Ok(Flow::Next)
            }
            M::Ldmxcsr => {
                let addr = self.sse_addr(false)?;
                let v = self.load(Width::W32, addr);
                // Reserved bits (31-16, MXCSR_MASK = 0xFFFF) raise #GP(0).
                let reserved = self.bin(BinOp::And, v, Value::Imm(0xffff_0000));
                self.fault_if(reserved, FAULT_GENERAL_PROTECTION);
                self.store_state(Width::W32, abi::STATE_MXCSR, v);
                Ok(Flow::Next)
            }
            M::Stmxcsr => {
                let addr = self.sse_addr(false)?;
                let v = self.load_state(Width::W32, abi::STATE_MXCSR);
                self.store(Width::W32, addr, v);
                Ok(Flow::Next)
            }
            _ => Err(Unsupported),
        }
    }

    /// `xmm = f(xmm, xmm/m128)`, the memory source 16-byte aligned.
    fn sse_packed(&mut self, f: impl FnOnce(&mut Self, VTemp, VTemp) -> VTemp) -> Lowered<Flow> {
        if self.insn.op_count() != 2 && self.insn.op_kind(2) != OpKind::Immediate8 {
            return Err(Unsupported);
        }
        let slot = self.xmm(0)?;
        let a = self.vload(VWidth::W128, VLoc::State(slot as u32));
        let b = self.vsrc(1, VWidth::W128, false)?;
        let r = f(self, a, b);
        self.vstore(VLoc::State(slot as u32), VWidth::W128, r);
        Ok(Flow::Next)
    }

    /// 128-bit moves, between registers or to or from memory.
    fn sse_mov128(&mut self, aligned: bool) -> Lowered<Flow> {
        let v = self.vsrc(1, VWidth::W128, !aligned)?;
        let dst = self.vdst(0, aligned)?;
        self.vstore(dst, VWidth::W128, v);
        Ok(Flow::Next)
    }

    /// movd and movq between XMM registers, GPRs and memory.
    fn sse_movdq(&mut self) -> Lowered<Flow> {
        let code = self.insn.code();
        match code {
            // movq xmm, xmm/m64 (F3 0F 7E) and movq xmm/m64, xmm (66 0F D6):
            // the low 64 bits; a register destination's upper half is zeroed.
            Code::Movq_xmm_xmmm64 | Code::Movq_xmmm64_xmm => {
                let v = self.vsrc(1, VWidth::W64, true)?;
                if self.is_mem(0) {
                    let dst = self.vdst(0, false)?;
                    self.vstore(dst, VWidth::W64, v);
                } else {
                    let dst = self.xmm(0)?;
                    self.vstore(VLoc::State(dst as u32), VWidth::W128, v);
                }
                Ok(Flow::Next)
            }
            // movd/movq xmm, r/m32/64: zero-extended to 128 bits.
            Code::Movd_xmm_rm32 | Code::Movq_xmm_rm64 => {
                let w = if code == Code::Movd_xmm_rm32 {
                    Width::W32
                } else {
                    Width::W64
                };
                let v = self.read(1, w)?;
                let dst = self.xmm(0)?;
                let t = self.vtemp();
                self.push_op(Op::VFromGpr { dst: t, value: v });
                self.vstore(VLoc::State(dst as u32), VWidth::W128, t);
                Ok(Flow::Next)
            }
            // movd/movq r/m32/64, xmm: the low lane.
            Code::Movd_rm32_xmm | Code::Movq_rm64_xmm => {
                let w = if code == Code::Movd_rm32_xmm {
                    Width::W32
                } else {
                    Width::W64
                };
                let src = self.xmm(1)?;
                let v = self.load_state(w, src);
                self.write(0, w, v)?;
                Ok(Flow::Next)
            }
            _ => Err(Unsupported),
        }
    }

    /// movss, movsd: from memory, zero the rest; between registers, merge
    /// the low lane; to memory, a narrow store.
    fn sse_movs(&mut self, width: VWidth) -> Lowered<Flow> {
        if self.insn.op_count() != 2 {
            return Err(Unsupported);
        }
        let v = self.vsrc(1, width, true)?;
        if self.is_mem(0) || !self.is_mem(1) {
            let dst = self.vdst(0, false)?;
            self.vstore(dst, width, v);
        } else {
            let dst = self.xmm(0)?;
            self.vstore(VLoc::State(dst as u32), VWidth::W128, v);
        }
        Ok(Flow::Next)
    }

    /// movhps/movhpd and movlps/movlpd with m64: the high or low half; the
    /// other half is kept.
    fn sse_half(&mut self, high: bool) -> Lowered<Flow> {
        let off = if high { 8 } else { 0 };
        if self.is_mem(0) {
            let src = self.xmm(1)?;
            let v = self.load_state(Width::W64, src + off);
            let addr = self.sse_addr(false)?;
            self.store(Width::W64, addr, v);
        } else {
            let dst = self.xmm(0)?;
            if !self.is_mem(1) {
                return Err(Unsupported);
            }
            let addr = self.sse_addr(false)?;
            let v = self.load(Width::W64, addr);
            self.store_state(Width::W64, dst + off, v);
        }
        Ok(Flow::Next)
    }

    /// psll/psrl/psra by imm8 or by the low 64 bits of xmm/m128.
    fn sse_shift(&mut self, m: Mnemonic) -> Lowered<Flow> {
        use Mnemonic as M;
        let op = match m {
            M::Psllw => VShiftOp::Shl(Lanes::W),
            M::Pslld => VShiftOp::Shl(Lanes::D),
            M::Psllq => VShiftOp::Shl(Lanes::Q),
            M::Psrlw => VShiftOp::ShrU(Lanes::W),
            M::Psrld => VShiftOp::ShrU(Lanes::D),
            M::Psrlq => VShiftOp::ShrU(Lanes::Q),
            M::Psraw => VShiftOp::ShrS(Lanes::W),
            M::Psrad => VShiftOp::ShrS(Lanes::D),
            _ => return Err(Unsupported),
        };
        let slot = self.xmm(0)?;
        let count = match self.insn.op_kind(1) {
            OpKind::Immediate8 => Value::Imm(self.insn.immediate8() as u64),
            OpKind::Memory => {
                let addr = self.sse_addr(true)?;
                self.load(Width::W64, addr)
            }
            _ => {
                let src = self.xmm(1)?;
                self.load_state(Width::W64, src)
            }
        };
        let a = self.vload(VWidth::W128, VLoc::State(slot as u32));
        let dst = self.vtemp();
        self.push_op(Op::VShift { dst, op, a, count });
        self.vstore(VLoc::State(slot as u32), VWidth::W128, dst);
        Ok(Flow::Next)
    }
}
