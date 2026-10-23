# Phase 1 status: Translator

**In progress: M1 (function-level translation and target census).** Done when all instruction fixtures pass, the 64-bit 7-Zip CLI (`7zz.exe`) compresses and extracts exactly like native, and CoreMark and `7z b` are tracked in CI. Milestones are in `docs/plan.md`.

- **Session 1 (2026-10-22/23):** planned Phase 1 and started M1.

## Decisions taken when planning (2026-10-22)

- **Milestones:** M1 function-level translation and target census → M2 PE loading → M3 integer instructions and CRT startup → M4 SSE, x87 and CoreMark → M5 7-Zip exactly like native → M6 speed and close-out. Translator architecture first, then PE loading, then the rest.
- **7-Zip without Wine:** a Win32 shim written in JS in `runtime/`, run in Node. The rejected alternatives were C shim DLLs translated as guest code, and Wine's PE DLLs early over an NT-syscall shim.
  - Imports resolve to thunks that call into the host.
  - The shim implements exactly the import census of the targets.
  - Unimplemented imports stop loudly.
  - Phase 1 is single-threaded (`-mmt1`).
- **Native references:**
  - Real Windows (windows-2025) is the reference of truth.
  - On the Linux runner, "native" is the same `.exe` under Ubuntu's wine64. It serves as the same-machine speed baseline and as a reported, non-gating byte-level cross-check.
- **7-Zip CLI:** `7zz.exe`, the standalone Alone2 bundle (all formats, no 7z.dll), built with MinGW-w64 GCC 13 without asm.
- **Instruction scope:** every iced `Code` in the census of the pinned targets (7zz.exe, coremark.exe, L2 programs) within the Phase 1 CPUID profile, minus documented exclusions.
  - Every such form has recorded fixtures.
  - Replay is 100 % with 0 unsupported.
  - `testkit census --check` enforces the rule in CI.
  - Details: `docs/design/instruction-scope.md`.
- **CPUID profile:** the x86-64 baseline (x87, MMX, SSE, SSE2, CMOV, CX8, FXSR) plus CMPXCHG16B and LAHF/SAHF. Nothing from SSE3 up, no AVX, AES, SHA, PCLMUL or BMI.
- **Benchmarks:**
  - CoreMark from M4 and `7zz b -mmt1` from M5, run translated in Node 24.
  - Native baselines: under wine64 on the same Linux runner, and on windows-2025.
  - History: one JSON line per main commit on an orphan `bench-data` branch.
  - PR runs warn (never fail) when a ratio worsens by more than 10 %.
- **Translator architecture:** one wasm function per guest function, with ABI v2 (`docs/design/abi.md`, `docs/design/translator.md`):
  - every block is an entry;
  - guest call/ret map to wasm call/return, with a return-address check;
  - tail transfers use `return_call`;
  - everything unusual unwinds to the trampoline;
  - a call-depth guard keeps guest recursion off the wasm stack limit.

## M1 progress

| Track | What | State |
|---|---|---|
| Contracts | ABI v2, `translator.md`, targets layout, plan | done |
| A | Function-level translation, new spike programs, measurements | in progress |
| B | Target builds in CI, PE reader, census, instruction scope | in progress |

## Preliminary census (planning, from MinGW libraries)

Measured read-only with `x86_64-w64-mingw32-objdump` on this image's MinGW-w64 11 / GCC 13.2 (win32 threads, `msvcrt.dll` CRT). The full census of the real targets replaces this table in M1.

| Object | Instructions | Mnemonics | Notable |
|---|---:|---:|---|
| `crt2.o` (EXE startup, always linked) | 284 | 26 | `mov rax, gs:[0x30]` (TEB), `lock cmpxchg`, `xchg m64, r64` |
| `libmingw32.a` | 1730 | 46 | `fninit`, `movups`/`movsd` XMM saves, `rep stos` |
| `libmingwex.a` (partly linked) | 54894 | 225 | x87 2566 (incl. `fnstcw`/`fldcw`), SSE 4412, `ldmxcsr`/`stmxcsr`, `cpuid`, `rep movs` |
| `libgcc.a` / `libstdc++.a` (partly linked) | 123k / 353k | 169 / 169 | some x87, much SSE, no AVX |

## What works

Pending: filled in as M1 lands.

## Measurements

Pending: M1 records the measurements listed in `docs/design/translator.md`.

## Known issues and risks

- Phase 0's translator risks still apply until their milestones: `lock` and atomics, FS/GS, the 67h prefix, precise faults.

## Next step

Finish M1 (this session), then M2: PE loading.
