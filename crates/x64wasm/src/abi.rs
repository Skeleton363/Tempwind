//! The ABI between translated modules and their hosts (v4). See
//! `docs/design/abi.md` and `docs/design/translator.md`.
//!
//! From v4, a module also imports the soft-float helpers it calls from the
//! `fp` namespace ([`FpFunc`]), before its own functions.
//!
//! A translated module imports `env.memory` (memory64), the process-wide
//! funcref table `env.table`, and the globals `env.table_base`, `env.lookup`
//! and `env.depth`; it exports `run(state: i64) -> i32`. Guest virtual
//! addresses equal offsets in that memory. `state` is the guest address of a
//! [`STATE_SIZE`]-byte CPU state block laid out as below. Each guest function
//! is a wasm function `(state: i64, entry: i32, ret: i64) -> i32` returning a
//! status; modules reach each other's functions through the lookup table.

/// Module name of the imported guest memory.
pub const MEMORY_MODULE: &str = "env";
/// Field name of the imported guest memory.
pub const MEMORY_NAME: &str = "memory";
/// Name of the exported entry function `(state: i64) -> i32`.
pub const RUN_EXPORT: &str = "run";
/// Field name (module `env`) of the imported process-wide funcref table.
pub const TABLE_NAME: &str = "table";
/// Field name of the imported immutable `i32` global: the table index of the
/// module's chunk 0.
pub const TABLE_BASE_NAME: &str = "table_base";
/// Field name of the imported immutable `i64` global: the guest address of
/// the lookup header.
pub const LOOKUP_NAME: &str = "lookup";
/// Field name of the imported mutable `i32` global: nested guest calls on the
/// wasm stack, shared by all modules.
pub const DEPTH_NAME: &str = "depth";

/// Maximum memory64 size browsers allow, in 64 KiB pages (16 GiB).
pub const MAX_MEMORY_PAGES: u64 = 262_144;

/// Module name of the floating-point helper imports (v4).
pub const FP_MODULE: &str = "fp";
/// Status bit of the SSE helpers: a raised exception is unmasked; the
/// result is meaningless and translated code exits with `EXIT_UNSUPPORTED`.
pub const FP_UNMASKED: u32 = 1 << 31;

/// A value type in a helper signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FpVal {
    I32,
    I64,
}

macro_rules! fp_funcs {
    ($($variant:ident $name:literal ($($p:ident),*) -> $($r:ident)?;)*) => {
        /// The exports of the floating-point helper module, in the order of
        /// `abi.md`'s tables ("Floating-point helper module"), which is the
        /// order a module imports the ones it uses in.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum FpFunc {
            $($variant,)*
        }

        impl FpFunc {
            pub const ALL: &'static [FpFunc] = &[$(FpFunc::$variant,)*];

            /// The export name.
            pub const fn name(self) -> &'static str {
                match self {
                    $(FpFunc::$variant => $name,)*
                }
            }

            /// Parameter types.
            pub const fn params(self) -> &'static [FpVal] {
                match self {
                    $(FpFunc::$variant => &[$(FpVal::$p),*],)*
                }
            }

            /// Result types.
            pub const fn results(self) -> &'static [FpVal] {
                match self {
                    $(FpFunc::$variant => &[$(FpVal::$r)?],)*
                }
            }
        }
    };
}

fp_funcs! {
    F32Add "f32_add" (I32, I32, I32) -> I32;
    F32Sub "f32_sub" (I32, I32, I32) -> I32;
    F32Mul "f32_mul" (I32, I32, I32) -> I32;
    F32Div "f32_div" (I32, I32, I32) -> I32;
    F32Min "f32_min" (I32, I32, I32) -> I32;
    F32Max "f32_max" (I32, I32, I32) -> I32;
    F32Sqrt "f32_sqrt" (I32, I32) -> I32;
    F64Add "f64_add" (I64, I64, I32) -> I64;
    F64Sub "f64_sub" (I64, I64, I32) -> I64;
    F64Mul "f64_mul" (I64, I64, I32) -> I64;
    F64Div "f64_div" (I64, I64, I32) -> I64;
    F64Min "f64_min" (I64, I64, I32) -> I64;
    F64Max "f64_max" (I64, I64, I32) -> I64;
    F64Sqrt "f64_sqrt" (I64, I32) -> I64;
    F32Cmp "f32_cmp" (I32, I32, I32, I32) -> I32;
    F64Cmp "f64_cmp" (I64, I64, I32, I32) -> I32;
    F32Comi "f32_comi" (I32, I32, I32, I32) -> I32;
    F64Comi "f64_comi" (I64, I64, I32, I32) -> I32;
    F32ToF64 "f32_to_f64" (I32, I32) -> I64;
    F64ToF32 "f64_to_f32" (I64, I32) -> I32;
    I32ToF32 "i32_to_f32" (I32, I32) -> I32;
    I64ToF32 "i64_to_f32" (I64, I32) -> I32;
    I64ToF64 "i64_to_f64" (I64, I32) -> I64;
    F32ToI32 "f32_to_i32" (I32, I32, I32) -> I32;
    F64ToI32 "f64_to_i32" (I64, I32, I32) -> I32;
    F32ToI64 "f32_to_i64" (I32, I32, I32) -> I64;
    F64ToI64 "f64_to_i64" (I64, I32, I32) -> I64;
    F80Add "f80_add" (I64, I32, I64, I32, I32, I32) -> I64;
    F80Sub "f80_sub" (I64, I32, I64, I32, I32, I32) -> I64;
    F80Subr "f80_subr" (I64, I32, I64, I32, I32, I32) -> I64;
    F80Mul "f80_mul" (I64, I32, I64, I32, I32, I32) -> I64;
    F80Div "f80_div" (I64, I32, I64, I32, I32, I32) -> I64;
    F80Divr "f80_divr" (I64, I32, I64, I32, I32, I32) -> I64;
    F80Sqrt "f80_sqrt" (I64, I32, I32) -> I64;
    F80Round "f80_round" (I64, I32, I32) -> I64;
    F80Scale "f80_scale" (I64, I32, I64, I32, I32) -> I64;
    F80Prem "f80_prem" (I64, I32, I64, I32, I32) -> I64;
    F80Prem1 "f80_prem1" (I64, I32, I64, I32, I32) -> I64;
    F80Load "f80_load" (I64, I32, I32) -> I64;
    F80Store "f80_store" (I64, I32, I32, I32) -> I64;
    F80Cmp "f80_cmp" (I64, I32, I64, I32, I32, I32, I32) -> I32;
    Status "status" () -> I32;
}

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
/// A guest fault; the state block holds the fault address and kind.
pub const EXIT_FAULT: u32 = 3;
/// A host function stopped execution (process exit, unimplemented API, shim
/// error); the host knows why.
pub const EXIT_HOST: u32 = 4;

/// Fault kind at [`STATE_FAULT_KIND`]: a read.
pub const FAULT_READ: u32 = 1;
/// Fault kind: a write.
pub const FAULT_WRITE: u32 = 2;
/// Fault kind: execution (no executable code at the address).
pub const FAULT_EXECUTE: u32 = 3;
/// Fault kind: divide error (#DE): `div`/`idiv` by zero or with a quotient
/// that doesn't fit.
pub const FAULT_DIVIDE: u32 = 4;
/// Fault kind: invalid opcode (#UD), e.g. `ud2`.
pub const FAULT_INVALID_OPCODE: u32 = 5;
/// Fault kind: general protection (#GP).
pub const FAULT_GENERAL_PROTECTION: u32 = 6;

/// Lookup header field: guest address of the slot array (u64).
pub const LOOKUP_SLOTS: u64 = 0;
/// Lookup header field: number of slots - 1, the slot count being a power of
/// two (u64).
pub const LOOKUP_MASK: u64 = 8;
/// Lookup header field: occupied slots, kept by the host (u64).
pub const LOOKUP_USED: u64 = 16;
/// Size of the lookup header.
pub const LOOKUP_HEADER_SIZE: u64 = 32;
/// Size of a lookup slot: key u64 (0 = empty), func u32, entry u32.
pub const LOOKUP_SLOT_SIZE: u64 = 16;
/// Multiplier of the lookup hash.
pub const LOOKUP_HASH_MUL: u64 = 0x9E37_79B9_7F4A_7C15;
/// Right shift of the lookup hash.
pub const LOOKUP_HASH_SHIFT: u32 = 32;

/// The slot index a lookup probe for `key` starts at; probing continues at
/// `(i + 1) & mask`.
pub const fn lookup_hash(key: u64, mask: u64) -> u64 {
    (key.wrapping_mul(LOOKUP_HASH_MUL) >> LOOKUP_HASH_SHIFT) & mask
}

/// Name of the custom section that lists a module's blocks.
pub const BLOCKS_SECTION: &str = "x64wasm.blocks";
/// Version of the `x64wasm.blocks` format.
pub const BLOCKS_VERSION: u32 = 1;
/// Size of the `x64wasm.blocks` header: version, chunks, n, reserved (u32).
pub const BLOCKS_HEADER_SIZE: usize = 16;
/// Size of an `x64wasm.blocks` entry: addr u64, chunk u32, entry u32.
pub const BLOCKS_ENTRY_SIZE: usize = 16;

/// Size of a run-time translator request (`x64wasm_rt.wasm`).
pub const RT_REQUEST_SIZE: usize = 40;
/// Request field: guest address of the first code byte (u64).
pub const RT_REQ_CODE_BASE: usize = 0;
/// Request field: pointer to the code bytes in the translator's memory (u32).
pub const RT_REQ_CODE_PTR: usize = 8;
/// Request field: number of code bytes (u32).
pub const RT_REQ_CODE_LEN: usize = 12;
/// Request field: guest address to translate from (u64).
pub const RT_REQ_ENTRY: usize = 16;
/// Request field: pointer to a sorted u64 array of addresses that already
/// have blocks (u32).
pub const RT_REQ_KNOWN_PTR: usize = 24;
/// Request field: number of known addresses (u32).
pub const RT_REQ_KNOWN_LEN: usize = 28;
/// Request field: flags, 0 (u32).
pub const RT_REQ_FLAGS: usize = 32;
/// Size of a run-time translator result.
pub const RT_RESULT_SIZE: usize = 20;
/// Result field: 0 ok, 1 error (u32).
pub const RT_RES_STATUS: usize = 0;
/// Result field: pointer to the module bytes (u32).
pub const RT_RES_WASM_PTR: usize = 4;
/// Result field: module length (u32).
pub const RT_RES_WASM_LEN: usize = 8;
/// Result field: pointer to the UTF-8 error text (u32).
pub const RT_RES_MSG_PTR: usize = 12;
/// Result field: error text length (u32).
pub const RT_RES_MSG_LEN: usize = 16;

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
    #[test]
    fn lookup_hash_vectors() {
        // The same vectors are checked in runtime/src/abi.mjs's users.
        assert_eq!(super::lookup_hash(0x1_4000_1000, 0xfff), 0x30e);
        assert_eq!(super::lookup_hash(0x8001_0000, 0xffff), 0xbd54);
        assert_eq!(super::lookup_hash(0x7fff_ffff_0000, 0x3ff), 0xb5);
    }

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
