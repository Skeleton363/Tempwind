// Constants mirroring crates/x64wasm/src/abi.rs. See docs/design/abi.md.

export const MEMORY_MODULE = 'env';
export const MEMORY_NAME = 'memory';
export const RUN_EXPORT = 'run';

export const PAGE_SIZE = 65536;
/** Maximum memory64 size browsers allow, in 64 KiB pages (16 GiB). */
export const MAX_MEMORY_PAGES = 262144;

export const STATE_GPR = 0x000;
export const STATE_RIP = 0x080;
export const STATE_RFLAGS = 0x088;
export const STATE_EXIT = 0x090;
export const STATE_CC_OP = 0x094;
export const STATE_CC_A = 0x098;
export const STATE_CC_B = 0x0a0;
export const STATE_CC_R = 0x0a8;
export const STATE_FS_BASE = 0x0b0;
export const STATE_GS_BASE = 0x0b8;
export const STATE_FAULT_ADDR = 0x0c0;
export const STATE_FAULT_KIND = 0x0c8;
export const STATE_XMM = 0x100;
export const STATE_MXCSR = 0x200;
export const STATE_FCW = 0x204;
export const STATE_FSW = 0x206;
export const STATE_FTW = 0x208;
export const STATE_FPR = 0x210;
export const STATE_SIZE = 0x400;

export const EXIT_LEFT_CODE = 1;
export const EXIT_UNSUPPORTED = 2;
export const EXIT_FAULT = 3;
export const EXIT_HOST = 4;

export const FAULT_READ = 1;
export const FAULT_WRITE = 2;
export const FAULT_EXECUTE = 3;

export const STATUS_RETURNED = 0;
export const STATUS_UNWIND = 1;
export const STATUS_EXIT = 2;
export const NO_RET = 0xffff_ffff_ffff_ffffn;

export const SENTINEL_RET = 0x0000_7fff_ffff_0000n;

export const GPR_NAMES = [
  'rax', 'rcx', 'rdx', 'rbx', 'rsp', 'rbp', 'rsi', 'rdi',
  'r8', 'r9', 'r10', 'r11', 'r12', 'r13', 'r14', 'r15',
];

/** @param {number} index 0 = RAX ... 15 = R15 */
export const gprOffset = (index) => STATE_GPR + 8 * index;

export const FLAGS = {
  CF: 1n << 0n, RESERVED1: 1n << 1n, PF: 1n << 2n, AF: 1n << 4n, ZF: 1n << 6n,
  SF: 1n << 7n, TF: 1n << 8n, IF: 1n << 9n, DF: 1n << 10n, OF: 1n << 11n,
};
FLAGS.STATUS = FLAGS.CF | FLAGS.PF | FLAGS.AF | FLAGS.ZF | FLAGS.SF | FLAGS.OF;

// v3 linking (docs/design/abi.md): imports of module `env` besides `memory`.
export const TABLE_NAME = 'table';
export const TABLE_BASE_NAME = 'table_base';
export const LOOKUP_NAME = 'lookup';
export const DEPTH_NAME = 'depth';

// Lookup table: header at `env.lookup`, 16-byte slots (key u64, func u32, entry u32).
export const LOOKUP_SLOTS = 0;
export const LOOKUP_MASK = 8;
export const LOOKUP_USED = 16;
export const LOOKUP_HEADER_SIZE = 32;
export const LOOKUP_SLOT_SIZE = 16;
export const LOOKUP_HASH_MUL = 0x9e37_79b9_7f4a_7c15n;
export const LOOKUP_HASH_SHIFT = 32n;
/** First slot index probed for `key` (BigInt); `mask` is a BigInt. */
export const lookupHash = (key, mask) =>
  (BigInt.asUintN(64, key * LOOKUP_HASH_MUL) >> LOOKUP_HASH_SHIFT) & mask;

// `x64wasm.blocks` custom section.
export const BLOCKS_SECTION = 'x64wasm.blocks';
export const BLOCKS_VERSION = 1;
export const BLOCKS_HEADER_SIZE = 16;
export const BLOCKS_ENTRY_SIZE = 16;

// Run-time translator (x64wasm_rt.wasm) request and result layouts.
export const RT_REQUEST_SIZE = 40;
export const RT_REQ_CODE_BASE = 0;
export const RT_REQ_CODE_PTR = 8;
export const RT_REQ_CODE_LEN = 12;
export const RT_REQ_ENTRY = 16;
export const RT_REQ_KNOWN_PTR = 24;
export const RT_REQ_KNOWN_LEN = 28;
export const RT_REQ_FLAGS = 32;
export const RT_RESULT_SIZE = 20;
export const RT_RES_STATUS = 0;
export const RT_RES_WASM_PTR = 4;
export const RT_RES_WASM_LEN = 8;
export const RT_RES_MSG_PTR = 12;
export const RT_RES_MSG_LEN = 16;

// Phase 1 process address map (docs/design/runtime.md).
export const GUEST_HEAP_BASE = 0x1000_0000n;
export const STACK_TOP = 0x7ff0_0000n;
export const PEB_ADDR = 0x7ffd_0000n;
export const TEB_ADDR = 0x7ffd_1000n;
export const KUSER_SHARED_DATA = 0x7ffe_0000n;
export const HOST_BASE = 0x8000_0000n;
export const LOOKUP_HEADER_ADDR = 0x8000_0000n;
export const MAIN_STATE_ADDR = 0x8000_1000n;
export const THUNK_BASE = 0x8001_0000n;
export const THUNK_SIZE = 16n;
export const MAX_THUNKS = 4096;
export const RUNTIME_HEAP_BASE = 0x8010_0000n;
export const HOST_FUNC_INDEX = 0;
export const PROCESS_ID = 0x1000;
export const THREAD_ID = 0x1004;
