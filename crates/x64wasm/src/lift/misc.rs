//! Flag and no-op instructions: `ud2`, `cmc`, `clc`, `stc`, `std`, `cld`;
//! the nop forms, `pause`, `prefetch*` (no access, never fault); `cpuid`.

use iced_x86::Mnemonic;

use super::lower::Lower;
use super::{Flow, Lowered, Unsupported};
use crate::abi::{self, flags::CF, flags::DF};
use crate::ir::{BinOp, RAX, RCX, RDX, Term, Value, Width};

/// RBX (encoding 3).
const RBX: u8 = 3;

/// The brand string of leaves 0x80000002-0x80000004, NUL-padded.
const BRAND: &[u8; 48] = b"Tempwind virtual x86-64 CPU\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

/// `cpuid` results by leaf: EAX, EBX, ECX, EDX (`instruction-scope.md`,
/// "`cpuid` results"). Other leaves return zeros.
fn cpuid_leaves() -> [(u32, [u32; 4]); 7] {
    let brand = |leaf: usize| -> [u32; 4] {
        core::array::from_fn(|r| {
            let at = 16 * leaf + 4 * r;
            u32::from_le_bytes(BRAND[at..at + 4].try_into().unwrap())
        })
    };
    [
        // Highest basic leaf; vendor "TempwindVCPU" in EBX, EDX, ECX.
        (0, [0x1, 0x706D_6554, 0x5550_4356, 0x646E_6977]),
        // Family 6; CMPXCHG16B; FPU, CX8, CMOV, MMX, FXSR, SSE, SSE2.
        (1, [0x600, 0, 0x2000, 0x0780_8101]),
        (0x8000_0000, [0x8000_0004, 0, 0, 0]),
        // LAHF/SAHF; SYSCALL, NX, LM.
        (0x8000_0001, [0, 0, 0x1, 0x2010_0800]),
        (0x8000_0002, brand(0)),
        (0x8000_0003, brand(1)),
        (0x8000_0004, brand(2)),
    ]
}

impl Lower<'_> {
    pub(super) fn misc(&mut self, m: Mnemonic) -> Lowered<Flow> {
        match m {
            Mnemonic::Nop
            | Mnemonic::Pause
            | Mnemonic::Endbr64
            | Mnemonic::Prefetchnta
            | Mnemonic::Prefetcht0
            | Mnemonic::Prefetcht1
            | Mnemonic::Prefetcht2
            | Mnemonic::Prefetchw
            | Mnemonic::Prefetch => {}
            Mnemonic::Ud2 => {
                return Ok(Flow::End(Term::Fault {
                    kind: abi::FAULT_INVALID_OPCODE,
                    rip: self.insn.ip(),
                }));
            }
            Mnemonic::Cmc => self.flags_update(0, Value::Imm(0), CF, None),
            Mnemonic::Clc => self.flags_update(CF, Value::Imm(0), 0, None),
            Mnemonic::Stc => self.flags_update(0, Value::Imm(CF), 0, None),
            Mnemonic::Cpuid => self.cpuid(),
            Mnemonic::Std | Mnemonic::Cld => {
                // DF lives in the state's RFLAGS while code runs.
                let rflags = self.load_state(Width::W64, abi::STATE_RFLAGS);
                let v = if m == Mnemonic::Std {
                    self.bin(BinOp::Or, rflags, Value::Imm(DF))
                } else {
                    self.bin(BinOp::And, rflags, Value::Imm(!DF))
                };
                self.store_state(Width::W64, abi::STATE_RFLAGS, v);
            }
            _ => return Err(Unsupported),
        }
        Ok(Flow::Next)
    }

    /// `cpuid`: a lookup of EAX in [`cpuid_leaves`] at run time. Writes EAX,
    /// EBX, ECX and EDX as 32-bit registers; ECX (the subleaf) is ignored
    /// and the flags are unchanged.
    fn cpuid(&mut self) {
        let leaf = self.read_reg(RAX, Width::W32);
        let hits: Vec<(Value, [u32; 4])> = cpuid_leaves()
            .into_iter()
            .map(|(l, regs)| (self.bin(BinOp::Eq, leaf, Value::Imm(l as u64)), regs))
            .collect();
        let values: Vec<Value> = (0..4)
            .map(|r| {
                hits.iter().rev().fold(Value::Imm(0), |rest, &(hit, regs)| {
                    let v = Value::Imm(regs[r] as u64);
                    if v == rest {
                        rest
                    } else {
                        self.select(hit, v, rest)
                    }
                })
            })
            .collect();
        for (reg, v) in [RAX, RBX, RCX, RDX].into_iter().zip(values) {
            self.set_reg(reg, v);
        }
    }
}
