# L1 fixture format (v2)

Status: **format 2, M3; M4 adds memory masks to `undefined` and the floating-point generator rules.** The contract between the oracle's recorder (`testkit record`), its replay (`testkit replay`, `crates/testkit/tests/replay.rs`) and the generators in `crates/testkit/src/oracle/forms.rs`. Fixtures live in `crates/testkit/fixtures/l1/<family>.json`, one file per mnemonic family. They are written only by `testkit record`, never by hand; `record --check` re-records in memory and diffs.

- **v1 (Phase 0 to M2):** GPRs, RIP, RFLAGS and the data window; one native single step; any signal was a recording error.
- **v2 (M3):** adds the SSE and x87 state, FS and GS bases, faults as expected outcomes, masks of undefined register bits, and multi-step recording for `rep` string instructions. A v1 file is rejected ("re-record").

## Layout

The same in recording and replay; guest address = wasm offset.

| | Address | Size | Contents |
|---|---|---|---|
| code page | `0x100000` | 4 KiB | the instruction, zeros after it |
| data window | `0x200000` | 64 KiB | zero except each case's `input.mem` ranges |
| state block (replay only) | `0x300000` | `0x400` | `abi.md`, "CPU state block" |
| guest memory (replay) | 0 | 4 MiB | |

The file repeats these in `layout`; a file whose layout differs from the build's is rejected.

## File

```text
{
  "header":   {"cpu", "kernel", "recorded", "note"},   ignored by record --check
  "format":   2,
  "family":   "add",
  "seed":     "0x...",
  "layout":   {"code", "data", "data_len", "state", "memory"},
  "defaults": {"fs_base": "0x0", "gs_base": "0x0", "xmm": "0x0", "mxcsr": "0x1f80",
               "fcw": "0x37f", "fsw": "0x0", "ftw": "0xffff", "st": "0x0"},
  "default_gprs": {"rax": ..., ..., "r15": ...},
  "cases": [ case, ... ]
}
```

`defaults` documents the value of every state field a case does not list. It must equal the oracle's constants (checked like `layout`): FXSAVE's state after `fninit` plus the power-up MXCSR, with FS and GS bases 0. `default_gprs` are the generator's random defaults, as in v1.

### Case

```text
{"form": "div_rm64/reg", "asm": "div rcx", "bytes": "48 f7 f1", "flags_mask": "0x400",
 "undefined": {"rdx": "0x..."},                              optional
 "input":  {"rflags": ..., "gprs": {...},
            "fs_base": ..., "gs_base": ...,                   optional
            "fp": {"xmm3": ..., "mxcsr": ..., "st0": ...},    optional
            "mem": [{"addr", "hex"}, ...]},
 "output": {"rip": ..., "rflags": ..., "gprs": {...},
            "fp": {...},                                      optional
            "fault": {"kind": "DE"},                          optional
            "mem": [{"addr", "hex"}, ...]}}
```

- **Values:** u64 values are `"0x..."` strings with minimal digits; XMM values are `"0x"` plus 32 hex digits (bits 127..0), ST values `"0x"` plus 20 hex digits (the 80-bit register: sign and exponent in the top 4 digits, then the 64-bit significand with the J bit). The parser accepts fewer digits. Memory `hex` strings are bytes in address order, grouped by 8.
- **`input.gprs`** lists the registers that differ from `default_gprs`; **`input.fp`** the fields that differ from `defaults`; `fs_base`/`gs_base` appear when not 0.
- **`output.gprs`** and **`output.fp`** list the fields that differ from the input. `output.rflags` and `output.rip` are always present.
- **`fp` field names:** `xmm0` … `xmm15`, `mxcsr`, `fcw`, `fsw`, `ftw`, `st0` … `st7`.
- **Memory:** as in v1. Every `input.mem` range is compared after the instruction; `output.mem` lists, in full, each range that changed. Nothing outside the compared ranges may change, in the data window or the code page.

### x87 registers

- `ftw` is the **full tag word** (2 bits per physical register R0-R7: 00 valid, 01 zero, 10 special, 11 empty), as in FSTENV and in the state block at `0x208`.
- `st0` … `st7` are in **ST(i) order** (FXSAVE's order). The state block holds physical registers: ST(i) = R[(TOP + i) mod 8], with TOP = FSW bits 11-13. Replay converts with the input FSW's TOP when it writes the block and with the output FSW's TOP when it reads it.
- FXSAVE carries only the abridged tag (one bit per physical register: not empty). The recorder converts:
  - to the CPU: abridged bit *p* = full tag of R*p* ≠ 11;
  - from the CPU: a non-empty register gets the tag FSTENV would report, from its contents (SDM Vol. 1, "x87 FPU tag word" and the FXSAVE section): exponent `0x7fff` → special; exponent 0 → zero if the significand is 0, else special (denormal); otherwise valid if the J bit is set, else special (unnormal).
  - An input tag word that disagrees with this rule for its contents can't be loaded into a real CPU, so the recorder rejects it as a generator bug.
- The contents of empty registers are carried and compared too: they survive `fninit` on real hardware.
- FOP, FIP, FDP and MXCSR_MASK are not recorded or compared. The YMM upper halves and other XSAVE components are not carried; SSE forms leave them unchanged.

### Faults

`output.fault` is present when the instruction raises an exception. The recorder maps the stop signal (from `PTRACE_GETSIGINFO`):

| Signal, `si_code` | `kind` | State block on `EXIT_FAULT` | Fault address |
|---|---|---|---|
| `SIGFPE`, `FPE_INTDIV` | `DE` | kind 4 (`FAULT_DIVIDE`) | 0 |
| `SIGILL`, `ILL_ILLOPN` | `UD` | kind 5 (`FAULT_INVALID_OPCODE`) | 0 |
| `SIGSEGV`, `SI_KERNEL` | `GP` | kind 6 (`FAULT_GENERAL_PROTECTION`) | 0 |
| `SIGSEGV`, `SEGV_MAPERR` or `SEGV_ACCERR` | `PF`, with `"addr"` = `si_addr` | kind 1, 2 or 3 (read, write, execute: siginfo doesn't say which, so replay accepts any of the three) | `addr` |

- Any other stop (`SIGBUS`, x87 or SSE floating-point `SIGFPE` codes, `SIGTRAP` other than the single step, ...) is still a recording error.
- **RIP** is the faulting instruction (`CODE_ADDR`), and every register, flag, the FP state and memory hold their values from before the instruction. For a fault in the first step the recorder checks this. In a later iteration of a `rep` string instruction, the completed iterations stay done (RCX, RSI, RDI and memory as the CPU leaves them), as the SDM specifies; the output records that state.
- The resume flag RF (bit 16), which the CPU sets in the RFLAGS image of a fault, is not recorded: output RFLAGS bits other than the status flags and DF always equal the input (the recorder checks this, ignoring TF and RF).
- Replay expects `EXIT_FAULT` with the mapped kind and address, RIP at the instruction and the output state; without a fault it expects `EXIT_LEFT_CODE` (as in v1).

### Undefined results

- **`flags_mask`** (as in v1): RFLAGS bits whose output is compared. Flags the SDM leaves undefined are outside it.
- **`undefined`**: per register, a mask of result bits the SDM leaves undefined (for example `bsf`/`bsr`'s destination when the source is 0). Names are GPR names and the `fp` field names; values are 64-bit masks (128-bit for `xmm*`, 80-bit for `st*`, 16 or 32 for the others). Omitted when empty.
- **Normalization.** The recorder writes the input's value for every bit outside `flags_mask` and inside `undefined`. What the CPU actually produced there is dropped, because it varies between CPU models: GitHub's runners and a session VM must record the same file. Replay doesn't compare those bits.
- **Memory (M4):** `undefined` may also hold `"mem": [{"addr", "hex"}]`, a bit mask over bytes at `addr` (same `hex` layout as memory). It must lie inside a compared range. It is normalized like registers: output bytes take the input's value under the mask, and replay doesn't compare them. Its use is for fields the SDM leaves reserved or that differ between CPU models, such as the reserved upper halves and the pointer selectors that `fnstenv` writes.

## Floating-point cases (M4)

The format needs no change for SSE floating point and x87: v2 already carries XMM, MXCSR, FCW, FSW, the full tag word and ST0-7. The generators follow these rules:
- **Exceptions stay masked.** An unmasked floating-point exception stops the child with `SIGFPE`, which is a recording error. So every form that can raise one runs with MXCSR bits 7-12 and FCW bits 0-5 set. The control forms (`ldmxcsr`, `fldcw`, `fldenv`) may load other masks, since they raise nothing themselves. An input FSW has ES and B clear unless it is consistent with the input FCW.
- **Control and status vary:**
  - MXCSR: RC (all four modes), FTZ and DAZ, and the six exception flags, which are sticky (so both "PE clear" and "PE already set" occur);
  - FCW: PC (single, double, extended; 01 is reserved) and RC;
  - FSW: TOP, the condition codes and the exception flags.
- **Values, edge pairs first:**
  - ±0, the smallest and largest denormals, MIN_NORMAL and the values next to it, ±1, the largest normal, ±∞;
  - QNaN and SNaN with several payloads;
  - pairs whose exact result is a rounding tie, or overflows, or underflows to a tiny or zero result;
  - integers at the conversion limits (±2^31, ±2^63, and the halves around them);
  - x87: unnormals and pseudo-denormals in memory operands (m80);
  - plus seeded random values.

  Packed forms mix classes across lanes.
- **SSE families (`forms/sse_fp.rs`):** `sse_fp_arith`, `sse_fp_cmp` and `sse_fp_cvt`, 112 cases per form. The first cases make two passes over the edges (40 operand pairs for the two-operand forms, the domain's edge values for the others), each pass under a different RC; packed forms take a different edge in each lane. The rest are random, with equal and nearly equal operands in 2 of 5 two-operand cases. cmp\* cases cycle through the eight predicates. Packed m128 operands are aligned, except two cases per memory form that expect #GP.
- **x87 stack states:** TOP in every position; tags consistent with the contents (as above); empty and full stacks, so that pushes overflow and empty operands underflow.
- **Undefined:** the condition codes the SDM leaves undefined per instruction (for example C0, C2 and C3 after `fadd`) go into `undefined` as an `fsw` mask.

## Recording

The recorder (`crates/testkit/src/oracle/native.rs`) runs a forked child under ptrace. For each case it:

1. writes the code page and the whole data window through `/proc/<pid>/mem`;
2. sets the GPRs, RIP = `CODE_ADDR`, RFLAGS, `fs_base` and `gs_base` with `PTRACE_SETREGS`. The bases must be below `0x7fff_ffff_f000` (the kernel's `TASK_SIZE_MAX`), or the kernel rejects them;
3. sets the FP state with `PTRACE_SETFPREGS` (the 512-byte FXSAVE image): FCW, FSW, abridged FTW, MXCSR, ST0-7 and XMM0-15, with FOP, FIP and FDP 0;
4. single-steps with `PTRACE_SINGLESTEP`. Until RIP leaves `CODE_ADDR` or a fault stops it, the recorder steps again. A `rep` string instruction traps after each iteration under TF (SDM Vol. 3, "Single-step"), so it takes one step per iteration (`rep stosb` with RCX = 5: 5 steps), and one step for a count of 0. More than 65,536 steps is an error;
5. reads the GPRs, RIP, RFLAGS (`PTRACE_GETREGS`), the FP state (`PTRACE_GETFPREGS`) and the data window.

It checks each case against the generator: the predicted RIP and fault kind, no change to RFLAGS bits other than the status flags and DF, no write outside the compared ranges, an unchanged state after a first-step fault, and a consistent input tag word. Then it normalizes the undefined bits.

## Replay

`crates/testkit/src/oracle/replay.rs` compiles one module per distinct encoding (`x64wasm::translate` at `CODE_ADDR`) and, per case:

1. writes the instruction and `input.mem` into fresh guest memory;
2. writes the state block: GPRs, RIP = `CODE_ADDR`, RFLAGS, exit 0, `fs_base` at `0x0b0`, `gs_base` at `0x0b8`, XMM0-15 at `0x100`, MXCSR at `0x200`, FCW, FSW and FTW at `0x204`-`0x208`, R0-R7 at `0x210` (10 bytes each in a 16-byte slot), everything else 0;
3. calls `run`. `EXIT_UNSUPPORTED` counts the case as unsupported;
4. compares the exit reason (and fault kind and address), RIP, every GPR and FP field outside `undefined`, RFLAGS under `flags_mask`, the whole data window and the code page.

`crates/testkit/tests/replay_harness.rs` mutates expectations of every kind (GPR, flag, memory, RIP, XMM, x87, fault) and checks that exactly the mutated cases fail.
