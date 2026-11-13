/* Generated from crates/x64wasm/src/abi.rs by `cargo run -p x64wasm-cli -- gen-header`;
   do not edit. The translated-module ABI: docs/design/abi.md. */
#ifndef TEMPWIND_ABI_H
#define TEMPWIND_ABI_H

#include <stdint.h>

/* CPU state block (abi.md, "CPU state block"): byte offsets. */
#define TW_STATE_GPR(i) (UINT64_C(0x0) + UINT64_C(0x8) * (uint64_t)(i))
#define TW_STATE_RIP UINT64_C(0x80)
#define TW_STATE_RFLAGS UINT64_C(0x88)
#define TW_STATE_EXIT UINT64_C(0x90)
#define TW_STATE_CC_OP UINT64_C(0x94)
#define TW_STATE_CC_A UINT64_C(0x98)
#define TW_STATE_CC_B UINT64_C(0xa0)
#define TW_STATE_CC_R UINT64_C(0xa8)
#define TW_STATE_FS_BASE UINT64_C(0xb0)
#define TW_STATE_GS_BASE UINT64_C(0xb8)
#define TW_STATE_FAULT_ADDR UINT64_C(0xc0)
#define TW_STATE_FAULT_KIND UINT64_C(0xc8)
#define TW_STATE_XMM UINT64_C(0x100)
#define TW_STATE_MXCSR UINT64_C(0x200)
#define TW_STATE_FCW UINT64_C(0x204)
#define TW_STATE_FSW UINT64_C(0x206)
#define TW_STATE_FTW UINT64_C(0x208)
#define TW_STATE_FPR UINT64_C(0x210)
#define TW_STATE_SIZE UINT64_C(0x400)

/* Statuses of a guest function (abi.md, "Statuses"). */
#define TW_STATUS_RETURNED 0
#define TW_STATUS_UNWIND 1
#define TW_STATUS_EXIT 2

/* Exit reasons (abi.md, "Exit reasons"), in state.exit. */
#define TW_EXIT_LEFT_CODE UINT32_C(0x1)
#define TW_EXIT_UNSUPPORTED UINT32_C(0x2)
#define TW_EXIT_FAULT UINT32_C(0x3)
#define TW_EXIT_HOST UINT32_C(0x4)

/* Fault kinds, in state.fault_kind. */
#define TW_FAULT_READ UINT32_C(0x1)
#define TW_FAULT_WRITE UINT32_C(0x2)
#define TW_FAULT_EXECUTE UINT32_C(0x3)
#define TW_FAULT_DIVIDE UINT32_C(0x4)
#define TW_FAULT_INVALID_OPCODE UINT32_C(0x5)
#define TW_FAULT_GENERAL_PROTECTION UINT32_C(0x6)

/* Page-state table (abi.md, "Page-state table"): one byte per page. */
#define TW_PAGE_CODE 1
#define TW_PAGE_WATCH 2
#define TW_PAGE_FLAGS_SHIFT 12

/* Lookup table (abi.md, "Lookup table"): header offsets, slot size, hash. */
#define TW_LOOKUP_SLOTS UINT64_C(0x0)
#define TW_LOOKUP_MASK UINT64_C(0x8)
#define TW_LOOKUP_USED UINT64_C(0x10)
#define TW_LOOKUP_HEADER_SIZE UINT64_C(0x20)
#define TW_LOOKUP_SLOT_SIZE UINT64_C(0x10)
#define TW_LOOKUP_HASH_MUL UINT64_C(0x9e3779b97f4a7c15)
#define TW_LOOKUP_HASH_SHIFT 32

/* The slot index a lookup probe for key starts at; probing continues at
   (i + 1) & mask. */
static inline uint64_t tw_lookup_hash(uint64_t key, uint64_t mask) {
  return ((key * TW_LOOKUP_HASH_MUL) >> TW_LOOKUP_HASH_SHIFT) & mask;
}

/* Addresses and limits. */
#define TW_NO_RET UINT64_C(0xffffffffffffffff)
#define TW_SENTINEL_RET UINT64_C(0x7fffffff0000)
#define TW_NULL_LIMIT UINT64_C(0x10000)
#define TW_MAX_MEMORY_PAGES UINT64_C(0x40000)
#define TW_NO_DISPATCHER UINT64_C(0xdead000000000000)

/* CPU profile: what the translator's cpuid and fxsave return, and XCR0.
   TW_CPUID_LEAVES initializes rows of { leaf, eax, ebx, ecx, edx }; other
   leaves return zeros. */
#define TW_CPUID_LEAF_COUNT 7
#define TW_CPUID_LEAVES { \
  { 0x00000000u, 0x00000001u, 0x706d6554u, 0x55504356u, 0x646e6977u }, \
  { 0x00000001u, 0x00000600u, 0x00000000u, 0x00002000u, 0x07808101u }, \
  { 0x80000000u, 0x80000004u, 0x00000000u, 0x00000000u, 0x00000000u }, \
  { 0x80000001u, 0x00000000u, 0x00000000u, 0x00000001u, 0x20100800u }, \
  { 0x80000002u, 0x706d6554u, 0x646e6977u, 0x72697620u, 0x6c617574u }, \
  { 0x80000003u, 0x36387820u, 0x2034362du, 0x00555043u, 0x00000000u }, \
  { 0x80000004u, 0x00000000u, 0x00000000u, 0x00000000u, 0x00000000u } \
}
#define TW_MXCSR_MASK UINT32_C(0xffff)
#define TW_XCR0 UINT64_C(0x3)

#endif /* TEMPWIND_ABI_H */
