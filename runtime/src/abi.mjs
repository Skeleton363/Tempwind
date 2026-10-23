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
