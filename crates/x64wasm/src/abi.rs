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
/// Optional (`Options::counters`): address of the counter block, immutable i64.
pub const COUNTERS_NAME: &str = "counters";
/// Optional (`Options::store_check`): address of the page-state table,
/// immutable i64.
pub const PAGE_FLAGS_NAME: &str = "page_flags";
/// Page-state table bit: the page holds translated code.
pub const PAGE_CODE: u8 = 1;
/// Page-state table bit: the page is write-watched.
pub const PAGE_WATCH: u8 = 2;
/// log2 of the page size the page-state table counts in (4 KiB).
pub const PAGE_FLAGS_SHIFT: u32 = 12;
/// Guest accesses below this address fault under `Options::null_check`.
pub const NULL_LIMIT: u64 = 0x1_0000;

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
/// Request field: flags (u32), the `RT_FLAG_*` bits.
pub const RT_REQ_FLAGS: usize = 32;
/// Request flag: translate with `Options::null_check`.
pub const RT_FLAG_NULL_CHECK: u32 = 1 << 0;
/// Request flag: translate with `Options::store_check`.
pub const RT_FLAG_STORE_CHECK: u32 = 1 << 1;
/// Request flag: translate with `Options::counters`.
pub const RT_FLAG_COUNTERS: u32 = 1 << 2;
/// Request flag: translate with `Options::sync_all` (the reference sync).
pub const RT_FLAG_SYNC_ALL: u32 = 1 << 3;
/// Request flag: translate with `Options::structured` false (the reference layout).
pub const RT_FLAG_REFERENCE_LAYOUT: u32 = 1 << 4;
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

/// The brand string of leaves 0x80000002-0x80000004, NUL-padded.
const CPUID_BRAND: &[u8; 48] =
    b"Tempwind virtual x86-64 CPU\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

/// `cpuid` results by leaf: EAX, EBX, ECX, EDX (`instruction-scope.md`,
/// "`cpuid` results"). Other leaves return zeros. The translator's `cpuid`
/// and the C header's `TW_CPUID_LEAVES` both come from this table.
pub fn cpuid_leaves() -> [(u32, [u32; 4]); 7] {
    let brand = |leaf: usize| -> [u32; 4] {
        core::array::from_fn(|r| {
            let at = 16 * leaf + 4 * r;
            u32::from_le_bytes(CPUID_BRAND[at..at + 4].try_into().unwrap())
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

/// The MXCSR_MASK `fxsave` stores (bytes 28-31 of its image): every MXCSR
/// bit is writable, DAZ (bit 6) included.
pub const MXCSR_MASK: u32 = 0xFFFF;

/// The value of XCR0 the profile describes: x87 and SSE state. The profile
/// has no OSXSAVE, so guest code cannot read it; Wine's Unix side reports it.
pub const XCR0: u64 = 3;

/// The non-canonical poison value for host-pointer slots in ntdll.dll
/// (`wine-unix.md`, section 8): a guest call through it misses the lookup,
/// and translating a non-canonical address stops loudly.
pub const NO_DISPATCHER: u64 = 0xDEAD_0000_0000_0000;

/// Where `x64wasm-cli gen-header` writes [`c_header`] by default, relative to
/// the repository root. The file is committed.
pub const C_HEADER_PATH: &str = "native/wine/include/tempwind/abi.h";

/// The command that regenerates [`C_HEADER_PATH`].
pub const C_HEADER_COMMAND: &str = "cargo run -p x64wasm-cli -- gen-header";

/// The C header of this ABI for Wine's Unix side (`abi.md`, "The C header"):
/// every constant from its definition here, with the `TW_` prefix.
pub fn c_header() -> String {
    let u64c = |v: u64| format!("UINT64_C({v:#x})");
    let u32c = |v: u32| format!("UINT32_C({v:#x})");
    let mut out = Vec::new();
    let section = |out: &mut Vec<String>, title: &str| out.push(format!("\n/* {title} */"));

    section(
        &mut out,
        "CPU state block (abi.md, \"CPU state block\"): byte offsets.",
    );
    out.push(format!(
        "#define TW_STATE_GPR(i) ({} + {} * (uint64_t)(i))",
        u64c(STATE_GPR),
        u64c(gpr_offset(1) - gpr_offset(0))
    ));
    for (name, v) in [
        ("RIP", STATE_RIP),
        ("RFLAGS", STATE_RFLAGS),
        ("EXIT", STATE_EXIT),
        ("CC_OP", STATE_CC_OP),
        ("CC_A", STATE_CC_A),
        ("CC_B", STATE_CC_B),
        ("CC_R", STATE_CC_R),
        ("FS_BASE", STATE_FS_BASE),
        ("GS_BASE", STATE_GS_BASE),
        ("FAULT_ADDR", STATE_FAULT_ADDR),
        ("FAULT_KIND", STATE_FAULT_KIND),
        ("XMM", STATE_XMM),
        ("MXCSR", STATE_MXCSR),
        ("FCW", STATE_FCW),
        ("FSW", STATE_FSW),
        ("FTW", STATE_FTW),
        ("FPR", STATE_FPR),
        ("SIZE", STATE_SIZE),
    ] {
        out.push(format!("#define TW_STATE_{name} {}", u64c(v)));
    }

    section(
        &mut out,
        "Statuses of a guest function (abi.md, \"Statuses\").",
    );
    for (name, v) in [
        ("RETURNED", STATUS_RETURNED),
        ("UNWIND", STATUS_UNWIND),
        ("EXIT", STATUS_EXIT),
    ] {
        out.push(format!("#define TW_STATUS_{name} {v}"));
    }

    section(
        &mut out,
        "Exit reasons (abi.md, \"Exit reasons\"), in state.exit.",
    );
    for (name, v) in [
        ("LEFT_CODE", EXIT_LEFT_CODE),
        ("UNSUPPORTED", EXIT_UNSUPPORTED),
        ("FAULT", EXIT_FAULT),
        ("HOST", EXIT_HOST),
    ] {
        out.push(format!("#define TW_EXIT_{name} {}", u32c(v)));
    }

    section(&mut out, "Fault kinds, in state.fault_kind.");
    for (name, v) in [
        ("READ", FAULT_READ),
        ("WRITE", FAULT_WRITE),
        ("EXECUTE", FAULT_EXECUTE),
        ("DIVIDE", FAULT_DIVIDE),
        ("INVALID_OPCODE", FAULT_INVALID_OPCODE),
        ("GENERAL_PROTECTION", FAULT_GENERAL_PROTECTION),
    ] {
        out.push(format!("#define TW_FAULT_{name} {}", u32c(v)));
    }

    section(
        &mut out,
        "Page-state table (abi.md, \"Page-state table\"): one byte per page.",
    );
    out.push(format!("#define TW_PAGE_CODE {PAGE_CODE}"));
    out.push(format!("#define TW_PAGE_WATCH {PAGE_WATCH}"));
    out.push(format!("#define TW_PAGE_FLAGS_SHIFT {PAGE_FLAGS_SHIFT}"));

    section(
        &mut out,
        "Lookup table (abi.md, \"Lookup table\"): header offsets, slot size, hash.",
    );
    for (name, v) in [
        ("SLOTS", LOOKUP_SLOTS),
        ("MASK", LOOKUP_MASK),
        ("USED", LOOKUP_USED),
        ("HEADER_SIZE", LOOKUP_HEADER_SIZE),
        ("SLOT_SIZE", LOOKUP_SLOT_SIZE),
        ("HASH_MUL", LOOKUP_HASH_MUL),
    ] {
        out.push(format!("#define TW_LOOKUP_{name} {}", u64c(v)));
    }
    out.push(format!("#define TW_LOOKUP_HASH_SHIFT {LOOKUP_HASH_SHIFT}"));
    out.push(
        "\n/* The slot index a lookup probe for key starts at; probing continues at\n   \
         (i + 1) & mask. */\n\
         static inline uint64_t tw_lookup_hash(uint64_t key, uint64_t mask) {\n  \
         return ((key * TW_LOOKUP_HASH_MUL) >> TW_LOOKUP_HASH_SHIFT) & mask;\n}"
            .to_string(),
    );

    section(&mut out, "Addresses and limits.");
    out.push(format!("#define TW_NO_RET {}", u64c(NO_RET)));
    out.push(format!("#define TW_SENTINEL_RET {}", u64c(SENTINEL_RET)));
    out.push(format!("#define TW_NULL_LIMIT {}", u64c(NULL_LIMIT)));
    out.push(format!(
        "#define TW_MAX_MEMORY_PAGES {}",
        u64c(MAX_MEMORY_PAGES)
    ));
    out.push(format!("#define TW_NO_DISPATCHER {}", u64c(NO_DISPATCHER)));

    section(
        &mut out,
        "CPU profile: what the translator's cpuid and fxsave return, and XCR0.\n   \
         TW_CPUID_LEAVES initializes rows of { leaf, eax, ebx, ecx, edx }; other\n   \
         leaves return zeros.",
    );
    let leaves = cpuid_leaves();
    out.push(format!("#define TW_CPUID_LEAF_COUNT {}", leaves.len()));
    out.push("#define TW_CPUID_LEAVES { \\".to_string());
    for (i, (leaf, [a, b, c, d])) in leaves.iter().enumerate() {
        let comma = if i + 1 < leaves.len() { "," } else { "" };
        out.push(format!(
            "  {{ {leaf:#010x}u, {a:#010x}u, {b:#010x}u, {c:#010x}u, {d:#010x}u }}{comma} \\"
        ));
    }
    out.push("}".to_string());
    out.push(format!("#define TW_MXCSR_MASK {}", u32c(MXCSR_MASK)));
    out.push(format!("#define TW_XCR0 {}", u64c(XCR0)));

    let mut text = format!(
        "/* Generated from crates/x64wasm/src/abi.rs by `{C_HEADER_COMMAND}`;\n   \
         do not edit. The translated-module ABI: docs/design/abi.md. */\n\
         #ifndef TEMPWIND_ABI_H\n#define TEMPWIND_ABI_H\n\n#include <stdint.h>\n"
    );
    for line in out {
        text.push_str(&line);
        text.push('\n');
    }
    text.push_str("\n#endif /* TEMPWIND_ABI_H */\n");
    text
}

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
