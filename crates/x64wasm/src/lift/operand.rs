//! Operands: general-purpose registers and their parts, effective
//! addresses, and reads and writes with x86 partial-register semantics.

use iced_x86::{OpKind, Register};

use super::lower::Lower;
use super::{Lowered, Unsupported};
use crate::ir::{BinOp, Gpr, Value, Width};

/// A general-purpose register operand: which register and which part.
#[derive(Clone, Copy, Debug)]
pub(super) enum Part {
    Q,
    D,
    W,
    Lo,
    Hi,
}

pub(super) fn gpr(reg: Register) -> Lowered<(Gpr, Part)> {
    if !reg.is_gpr() {
        return Err(Unsupported);
    }
    let full = reg.full_register();
    let index = full as u32 - Register::RAX as u32;
    if index >= 16 {
        return Err(Unsupported);
    }
    let part = match reg {
        Register::AH | Register::CH | Register::DH | Register::BH => Part::Hi,
        _ => match reg.size() {
            8 => Part::Q,
            4 => Part::D,
            2 => Part::W,
            1 => Part::Lo,
            _ => return Err(Unsupported),
        },
    };
    Ok((index as Gpr, part))
}

pub(super) fn gpr64(reg: Register) -> Lowered<Gpr> {
    match gpr(reg)? {
        (g, Part::Q) => Ok(g),
        _ => Err(Unsupported),
    }
}

impl Lower<'_> {
    /// True if a 67h (address-size) legacy prefix is present.
    fn has_addr32_prefix(&self) -> bool {
        for &b in self.bytes {
            match b {
                0x67 => return true,
                0x26 | 0x2e | 0x36 | 0x3e | 0x64 | 0x65 | 0x66 | 0xf0 | 0xf2 | 0xf3 => {}
                _ => return false,
            }
        }
        false
    }

    /// Effective address of the memory operand (no segment base).
    pub(super) fn ea_offset(&mut self) -> Lowered<Value> {
        if let Some(v) = self.ea {
            return Ok(v);
        }
        if self.has_addr32_prefix() {
            return Err(Unsupported);
        }
        let insn = self.insn;
        let base = insn.memory_base();
        let index = insn.memory_index();
        let disp = insn.memory_displacement64();
        let v = if base == Register::RIP {
            // iced already resolved RIP-relative operands to an absolute address.
            if index != Register::None {
                return Err(Unsupported);
            }
            Value::Imm(disp)
        } else {
            let mut acc: Option<Value> = None;
            if base != Register::None {
                acc = Some(Value::Reg(gpr64(base)?));
            }
            if index != Register::None {
                let mut x = Value::Reg(gpr64(index)?);
                let shift = match insn.memory_index_scale() {
                    1 => 0,
                    2 => 1,
                    4 => 2,
                    8 => 3,
                    _ => return Err(Unsupported),
                };
                if shift != 0 {
                    x = self.bin(BinOp::Shl, x, Value::Imm(shift));
                }
                acc = Some(match acc {
                    Some(a) => self.bin(BinOp::Add, a, x),
                    None => x,
                });
            }
            match acc {
                None => Value::Imm(disp),
                Some(a) if disp == 0 => a,
                Some(a) => self.bin(BinOp::Add, a, Value::Imm(disp)),
            }
        };
        self.ea = Some(v);
        Ok(v)
    }

    /// Guest address of the memory operand. FS/GS-relative accesses are not
    /// supported yet; other segments have base 0 in 64-bit mode.
    pub(super) fn ea(&mut self) -> Lowered<Value> {
        match self.insn.memory_segment() {
            Register::FS | Register::GS => Err(Unsupported),
            _ => self.ea_offset(),
        }
    }

    pub(super) fn width_of(&self, operand: u32) -> Lowered<Width> {
        match self.insn.op_kind(operand) {
            OpKind::Register => {
                Width::from_bytes(self.insn.op_register(operand).size()).ok_or(Unsupported)
            }
            OpKind::Memory => Width::from_bytes(self.insn.memory_size().size()).ok_or(Unsupported),
            _ => Err(Unsupported),
        }
    }

    /// Reads an operand, zero-extended from `width`.
    pub(super) fn read(&mut self, operand: u32, width: Width) -> Lowered<Value> {
        match self.insn.op_kind(operand) {
            OpKind::Register => {
                let (g, part) = gpr(self.insn.op_register(operand))?;
                let r = Value::Reg(g);
                Ok(match part {
                    Part::Q => r,
                    Part::D => self.mask(r, Width::W32),
                    Part::W => self.mask(r, Width::W16),
                    Part::Lo => self.mask(r, Width::W8),
                    Part::Hi => {
                        let shifted = self.bin(BinOp::ShrU, r, Value::Imm(8));
                        self.mask(shifted, Width::W8)
                    }
                })
            }
            OpKind::Memory => {
                let addr = self.ea()?;
                Ok(self.load(width, addr))
            }
            OpKind::Immediate8
            | OpKind::Immediate16
            | OpKind::Immediate32
            | OpKind::Immediate64
            | OpKind::Immediate8to16
            | OpKind::Immediate8to32
            | OpKind::Immediate8to64
            | OpKind::Immediate32to64 => {
                Ok(Value::Imm(self.insn.immediate(operand) & width.mask()))
            }
            _ => Err(Unsupported),
        }
    }

    /// Writes `value` (zero-extended from `width`) to an operand with x86
    /// partial-register semantics.
    pub(super) fn write(&mut self, operand: u32, width: Width, value: Value) -> Lowered<()> {
        match self.insn.op_kind(operand) {
            OpKind::Register => {
                let (g, part) = gpr(self.insn.op_register(operand))?;
                let r = Value::Reg(g);
                let merged = match part {
                    // 32-bit writes zero-extend; `value` is already zero-extended.
                    Part::Q | Part::D => value,
                    Part::W | Part::Lo => {
                        let kept = self.bin(BinOp::And, r, Value::Imm(!width.mask()));
                        self.bin(BinOp::Or, kept, value)
                    }
                    Part::Hi => {
                        let kept = self.bin(BinOp::And, r, Value::Imm(!0xff00));
                        let shifted = self.bin(BinOp::Shl, value, Value::Imm(8));
                        self.bin(BinOp::Or, kept, shifted)
                    }
                };
                self.set_reg(g, merged);
                Ok(())
            }
            OpKind::Memory => {
                let addr = self.ea()?;
                self.store(width, addr, value);
                Ok(())
            }
            _ => Err(Unsupported),
        }
    }
}
