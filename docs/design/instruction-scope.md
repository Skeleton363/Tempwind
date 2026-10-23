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

Measured on the `targets-4b411bf` build of 2026-10-23 (`tests/targets/targets.lock` names the current one; the `census.md` and `census.json` assets of that release have the full tables). `testkit census` produces them from the `.pdata` function ranges of each executable, plus a linear sweep of the gaps in executable sections (import thunks, `___chkstk_ms`; `int3`, `nop` and zero padding is counted apart). The builds are bit-for-bit reproducible: `targets-0c3a264` has the same `7zz.exe` and `coremark.exe` hashes.

| | 7zz.exe | coremark.exe | hello.exe (L2) | combined |
|---|---|---|---|---|
| file size (bytes) | 4,032,180 | 265,161 | 251,488 | |
| functions (`.pdata`) | 6,289 | 134 | 95 | 6,518 |
| instructions in functions | 520,823 | 9,668 | 7,482 | 537,973 |
| instructions in gaps | 748 | 64 | 64 | 876 |
| padding instructions (not counted) | 23,317 | 536 | 435 | 24,288 |
| undecodable bytes in functions / gaps | 0 / 85 | 0 / 14 | 0 / 15 | 0 / 114 |
| distinct `Code`s | 436 | 220 | 197 | 443 |

Sources: 7-Zip 25.01 (`7z2501-src.tar.xz`, sha256 `ed087f83...c36e`), CoreMark commit `1f483d5` (tarball sha256 `4067e7f2...f987`), MinGW-w64 GCC 13.2.0 (win32 threads, `msvcrt.dll` CRT), `-O2`, no assembler, symbols kept. All function ranges decode: no invalid byte inside a `.pdata` range.

### Instructions per class

| class | 7zz.exe | coremark.exe | hello.exe | combined | share | distinct `Code`s |
|---|---|---|---|---|---|---|
| integer | 507,906 | 9,530 | 7,365 | 524,801 | 97.4% | 309 |
| SSE2 | 8,320 | 162 | 145 | 8,627 | 1.6% | 61 |
| SSE | 4,388 | 16 | 12 | 4,416 | 0.8% | 7 |
| x87 | 32 | 21 | 21 | 74 | 0.0% | 9 |
| AVX and later (VEX, BMI, ...) | 642 | 3 | 3 | 648 | 0.1% | 32 |
| SSE3 to SSE4.2, POPCNT | 56 | 0 | 0 | 56 | 0.0% | 4 |
| AES, PCLMULQDQ, SHA | 212 | 0 | 0 | 212 | 0.0% | 19 |
| other (`cpuid`, `xgetbv`) | 15 | 0 | 0 | 15 | 0.0% | 2 |

MMX has no instructions in these builds. `mov` alone is 34.5 % of all instructions; the top 15 mnemonics (`mov`, `call`, `je`, `test`, `lea`, `cmp`, `add`, `jmp`, `pop`, `jne`, `nop`, `push`, `xor`, `sub`, `ret`) are 76 %. In all there are 214 distinct mnemonics.

### Outside the CPUID profile

| feature | instructions | mnemonics |
|---|---|---|
| AVX | 335 | `vaesdec`, `vaesenc`, `vmovd`, `vmovdqu`, `vmovq`, `vmovups`, `vpaddq`, `vpxor`, `vzeroupper`, ... |
| AVX2 | 311 | `vbroadcasti128`, `vextracti128`, `vinserti128`, `vpaddd`, `vpshufb`, `vpshufd`, `vperm2i128`, ... |
| SHA | 128 | `sha1msg1`, `sha1msg2`, `sha1nexte`, `sha1rnds4`, `sha256msg1`, `sha256msg2`, `sha256rnds2` |
| AES | 52 | `aesdec`, `aesdeclast`, `aesenc`, `aesenclast` and the `v` forms |
| SSSE3 | 44 | `palignr`, `pshufb` |
| VAES | 32 | `vaesdec`, `vaesdeclast`, `vaesenc`, `vaesenclast` |
| CPUID | 14 | `cpuid` (excluded from fixtures: dedicated tests) |
| SSE4.1 | 12 | `pinsrd`, `pmaxud` |
| BMI1 | 10 | `tzcnt` |
| XSAVE | 1 | `xgetbv` |

- **57 of the 443 `Code`s are outside the profile** (32 AVX and later, 19 crypto, 4 SSE3 and later, 2 other); all of them come from 7-Zip's hardware paths and the CPU feature probes.
- **They are present in the image but not executed.** 30 of 7zz.exe's 6,289 functions contain such instructions: the `*_HW` AES and SHA routines (`AesCbc_Decode_HW_256`, `Sha256_UpdateBlocks_HW`, ...), the `Blake2sp_*_AVX2_Fast` and `*_V128_*` routines, `ShufBytes_*`, and the `CPU_IsSupported_*` probes with `z7_x86_cpuid`. With the profile, `cpuid` reports none of those features, so 7-Zip never calls them. Ahead-of-time translation of whole images (M2) has to translate such functions to a trap stub or leave them out; the census lists them by name (`out_of_profile_functions` in `census.json`).
- **`tzcnt` is `rep bsf`.** The 10 `tzcnt` are in MinGW's `__trailz_D2A` (gdtoa), which is also in `coremark.exe` and `hello.exe`. Its encoding `F3 0F BC` runs as `bsf` on a CPU without BMI1, which the profile describes. iced-x86 decodes it as `tzcnt`; the translator must treat `F3 0F BC` as `bsf` under this profile (they differ when the source is zero).
- `xgetbv` appears once, in 7-Zip's probe; it is out of profile (XSAVE) and unreachable.

### Scope S from this census

- **489 in-scope forms** (a `Code` as register or memory variant) over 382 `Code`s.
- 5 `Code`s are excluded by the rule: `In_AL_imm8`, `Out_imm8_AL`, `Out_DX_AL`, `Hlt` (the first four only in gaps of 7zz.exe, so data bytes) and `Cpuid` (14 occurrences, dedicated test).
- 22 forms occur only in gaps, where the sweep reads some data bytes as code (`Std`, `Cmc`, `Int1`, `Loop_rel8_64_RCX`, ...). They are listed in `census.md` and count toward S as the rule is written. Dropping them is a decision for the M5 gate (they are probably not code); the rule as decided keeps them.
- **L1 fixtures cover 130 of the 489 forms (26.6 %).** `testkit census --check` lists the 359 missing ones; it fails until the instruction PRs add them (M5).

### Imports (the Win32 shim's surface)

| DLL | distinct functions | 7zz.exe | coremark.exe | hello.exe |
|---|---|---|---|---|
| `kernel32.dll` | 113 | 113 | 15 | 13 |
| `msvcrt.dll` | 54 | 51 | 34 | 35 |
| `advapi32.dll` | 14 | 14 | 0 | 0 |
| `oleaut32.dll` | 6 | 6 | 0 | 0 |
| `user32.dll` | 2 | 2 | 0 | 0 |

189 distinct functions in 5 DLLs (7zz.exe imports 186, coremark.exe 49, hello.exe 48). `ntdll`, `ws2_32`, `gdi32`, `shell32` and `ole32` are not imported. 7-Zip's `advapi32` use is registry and token privileges (`Reg*`, `AdjustTokenPrivileges`, `LookupPrivilegeValueW`, `OpenProcessToken`, `Get/SetFileSecurityW`, `SystemFunction036` = `RtlGenRandom`); `oleaut32` is the `BSTR`/`VARIANT` helpers; `user32` is `CharPrevExA` and `CharUpperW`. Names per DLL are in `census.md`.
