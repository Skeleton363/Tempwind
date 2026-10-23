# Instruction scope for Phase 1

Status: **rule decided (Phase 1 plan); census numbers from M1.** Phase 1's done-when says "all instruction fixtures pass". This note says which instructions that covers, and how CI enforces it.

## Targets

The scope comes from the real programs Phase 1 must run, built with MinGW-w64 (GCC 13.2, `msvcrt.dll` CRT, win32 threads) by `.github/workflows/targets.yml`:

- `7zz.exe`: 7-Zip's standalone console (Alone2 bundle), the Phase 1 7-Zip CLI;
- `coremark.exe`: CoreMark with our Win64 port;
- the L2 test programs in `tests/native/` (MinGW CRT startup included).

## CPUID profile

What translated code reports through `cpuid` in Phase 1:

| Advertised | Not advertised |
|---|---|
| x86-64 baseline: x87 (FPU), MMX, SSE, SSE2, CMOV, CX8, FXSR, SYSCALL bit as on any x86-64 CPU, plus CMPXCHG16B and LAHF/SAHF in 64-bit mode | SSE3, SSSE3, SSE4.1, SSE4.2, POPCNT, AVX, AVX2, AVX-512, FMA, F16C, BMI1/2, LZCNT, MOVBE, AES-NI, PCLMULQDQ, SHA, RDRAND/RDSEED, XSAVE/OSXSAVE |

- Programs that dispatch on CPUID (7-Zip's AES, SHA, CRC and match-finder paths, for example) take their generic paths.
- Their results don't depend on the path taken, so the output still matches a native run on a CPU with every feature.
- Raising the profile later only adds forms to the scope.

## The rule: "all instruction fixtures pass"

1. **Scope S** is every iced-x86 `Code` (one encoding form, for example `Add_rm64_imm8`) that appears in the census of the pinned targets and whose `cpuid_features()` are all in the profile, minus these exclusions:
   - privileged or system instructions that never run in user mode (`hlt`, `in`/`out`, `cli`/`sti`, control and debug register moves, …);
   - `syscall`/`sysenter` (PE user code doesn't use them in Phase 1);
   - nondeterministic instructions (`cpuid`, `rdtsc`, `rdtscp`, `rdrand`, `rdseed`), which get dedicated tests instead of recorded fixtures (`cpuid` must return the profile above).
2. **Fixtures.** Every Code in S has recorded L1 fixtures in `crates/testkit/fixtures/l1/`:
   - covering each operand-kind combination (register or memory, every width the Code allows);
   - for memory operands, all 9 address modes of the oracle (base, base+disp8, base+disp32, base+index, base+index+disp8, base+index+disp32, index+disp32, absolute, RIP-relative);
   - at least 32 cases each, edge values first;
   - with `flags_mask` excluding the flags the SDM leaves undefined (iced's `rflags_undefined()` as a cross-check).
3. **Replay.** `testkit replay` passes 100 % with 0 unsupported, and `testkit record --check` reproduces every fixture on GitHub's runner.
4. **Enforcement.** `testkit census --check` runs in CI on the pinned targets. It fails if any Code in S has no fixtures, so the rule holds automatically whenever the targets change.

**Fixtures first.** An instruction PR's first commit adds the generator and the recorded fixtures; the translation comes after, and replay shows the progress.

## Census

The census of the CI-built targets:
- counts per `Code`, per mnemonic and per CPUID feature;
- groups them into classes: integer, x87, MMX, SSE, SSE2, SSE3 to SSE4.2, AVX and later, AES/PCLMUL/SHA, other;
- lists the imports, which define the Win32 shim's surface.

`testkit census` produces it, and its `census.json` and `census.md` ship in each `targets-<sha7>` release.

Pending: the tables for the first `targets-<sha7>` release are added in M1.
