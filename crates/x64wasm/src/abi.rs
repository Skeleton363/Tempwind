//! The ABI between translated modules and their hosts (v2). See
//! `docs/design/abi.md` and `docs/design/translator.md`.
//!
//! A translated module imports `env.memory` (memory64) and exports
//! `run(state: i64) -> i32`. Guest virtual addresses equal offsets in that
//! memory. `state` is the guest address of a [`STATE_SIZE`]-byte CPU state
//! block laid out as below. Inside the module, each guest function is a wasm
//! function `(state: i64, entry: i32, ret: i64) -> i32` returning a status.

/// Module name of the imported guest memory.
pub const MEMORY_MODULE: &str = "env";
/// Field name of the imported guest memory.
pub const MEMORY_NAME: &str = "memory";
/// Name of the exported entry function `(state: i64) -> i32`.
pub const RUN_EXPORT: &str = "run";

/// Maximum memory64 size browsers allow, in 64 KiB pages (16 GiB).
pub const MAX_MEMORY_PAGES: u64 = 262_144;

/// Offset of the 16 general-purpose registers (x86 encoding order, u64 each).
pub const STATE_GPR: u64 = 0x000;
/// Offset of RIP (u64).
pub const STATE_RIP: u64 = 0x080;
/// Offset of RFLAGS (u64).
pub const STATE_RFLAGS: u64 = 0x088;
/// Offset of the exit reason (u32), the same value `run` returns.
pub const STATE_EXIT: u64 = 0x090;
/// Offset of the lazy flag record kind (u32): 0 = materialized in `cc_a`,
/// else `ir::FlagOp::cc_op`. Valid at function boundaries during a run.
pub const STATE_CC_OP: u64 = 0x094;
/// Offset of the lazy flag record's operand a (u64), or the materialized flags.
pub const STATE_CC_A: u64 = 0x098;
/// Offset of the lazy flag record's operand b (u64).
pub const STATE_CC_B: u64 = 0x0a0;
/// Offset of the lazy flag record's result (u64).
pub const STATE_CC_R: u64 = 0x0a8;
/// Offset of the FS segment base (u64), reserved for M3.
pub const STATE_FS_BASE: u64 = 0x0b0;
/// Offset of the GS segment base (u64; the TEB on Windows), reserved for M3.
pub const STATE_GS_BASE: u64 = 0x0b8;
/// Offset of the fault address (u64), reserved for fault exits.
pub const STATE_FAULT_ADDR: u64 = 0x0c0;
/// Offset of the fault kind (u32), reserved for fault exits.
pub const STATE_FAULT_KIND: u64 = 0x0c8;
/// Offset of XMM0-XMM15 (16 bytes each), reserved for M3/M4.
pub const STATE_XMM: u64 = 0x100;
/// Offset of MXCSR (u32), reserved for M3/M4.
pub const STATE_MXCSR: u64 = 0x200;
/// Offset of the x87 control word (u16), reserved for M3/M4.
pub const STATE_FCW: u64 = 0x204;
/// Offset of the x87 status word (u16, TOP in bits 11-13), reserved.
pub const STATE_FSW: u64 = 0x206;
/// Offset of the full x87 tag word (u16, 2 bits per register), reserved.
pub const STATE_FTW: u64 = 0x208;
/// Offset of the x87 physical registers R0-R7 (80 bits each in a 16-byte
/// slot; ST(i) = R[(TOP + i) mod 8]), reserved.
pub const STATE_FPR: u64 = 0x210;
/// Size of the state block in bytes. The block must be 16-byte aligned.
pub const STATE_SIZE: u64 = 0x400;

/// Byte offset of general-purpose register `index` (0 = RAX ... 15 = R15).
pub const fn gpr_offset(index: u32) -> u64 {
    STATE_GPR + 8 * index as u64
}

/// Control reached a guest address with no translated block; RIP holds it.
pub const EXIT_LEFT_CODE: u32 = 1;
/// The instruction at RIP is not supported; registers hold the state before it.
pub const EXIT_UNSUPPORTED: u32 = 2;
/// Reserved: a guest fault; the state block holds the fault address and kind.
pub const EXIT_FAULT: u32 = 3;

/// Status of a guest function: a `ret` popped exactly the caller's expected
/// return address. GPRs and the lazy flag record are stored; RIP is not.
pub const STATUS_RETURNED: i32 = 0;
/// Status of a guest function: continue at `state.rip` through the
/// trampoline. The state block holds the full state.
pub const STATUS_UNWIND: i32 = 1;
/// Status of a guest function: stop with the reason in `state.exit`. The
/// state block holds the full state.
pub const STATUS_EXIT: i32 = 2;

/// The `ret` argument the trampoline passes: a non-canonical address no
/// `call` can push, so no `ret` matches it.
pub const NO_RET: u64 = u64::MAX;

/// Return address hosts push so a test function's final `ret` exits to them.
pub const SENTINEL_RET: u64 = 0x0000_7FFF_FFFF_0000;

/// RFLAGS status-flag bits.
pub mod flags {
    pub const CF: u64 = 1 << 0;
    /// Bit 1 is reserved and always reads as 1.
    pub const RESERVED1: u64 = 1 << 1;
    pub const PF: u64 = 1 << 2;
    pub const AF: u64 = 1 << 4;
    pub const ZF: u64 = 1 << 6;
    pub const SF: u64 = 1 << 7;
    pub const TF: u64 = 1 << 8;
    pub const IF: u64 = 1 << 9;
    pub const DF: u64 = 1 << 10;
    pub const OF: u64 = 1 << 11;
    /// The six arithmetic status flags.
    pub const STATUS: u64 = CF | PF | AF | ZF | SF | OF;
}

/// Register names in encoding order, as used in fixtures and manifests.
pub const GPR_NAMES: [&str; 16] = [
    "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13",
    "r14", "r15",
];

/// A guest CPU state block, in host form.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuState {
    pub gpr: [u64; 16],
    pub rip: u64,
    pub rflags: u64,
    pub exit: u32,
}

impl CpuState {
    /// Serializes to the in-memory layout (`STATE_SIZE` bytes, reserved fields zero).
    pub fn to_bytes(&self) -> [u8; STATE_SIZE as usize] {
        let mut out = [0u8; STATE_SIZE as usize];
        for (i, value) in self.gpr.iter().enumerate() {
            let at = gpr_offset(i as u32) as usize;
            out[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        out[STATE_RIP as usize..][..8].copy_from_slice(&self.rip.to_le_bytes());
        out[STATE_RFLAGS as usize..][..8].copy_from_slice(&self.rflags.to_le_bytes());
        out[STATE_EXIT as usize..][..4].copy_from_slice(&self.exit.to_le_bytes());
        out
    }

    /// Parses the in-memory layout. `bytes` must hold at least `STATE_SIZE` bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let u64_at = |at: u64| u64::from_le_bytes(bytes[at as usize..][..8].try_into().unwrap());
        let mut gpr = [0u64; 16];
        for (i, value) in gpr.iter_mut().enumerate() {
            *value = u64_at(gpr_offset(i as u32));
        }
        CpuState {
            gpr,
            rip: u64_at(STATE_RIP),
            rflags: u64_at(STATE_RFLAGS),
            exit: u32::from_le_bytes(bytes[STATE_EXIT as usize..][..4].try_into().unwrap()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips() {
        let mut state = CpuState {
            rip: 0x1_4000_1000,
            rflags: 0x202 | flags::CF,
            exit: EXIT_LEFT_CODE,
            ..CpuState::default()
        };
        for (i, value) in state.gpr.iter_mut().enumerate() {
            *value = 0x1111_1111_1111_1111u64.wrapping_mul(i as u64 + 1);
        }
        assert_eq!(CpuState::from_bytes(&state.to_bytes()), state);
    }
}
