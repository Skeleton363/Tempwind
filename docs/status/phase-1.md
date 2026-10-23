# Phase 1 status: Translator

**M1 (function-level translation and target census) is done; next is M2 (PE loading).** Phase 1 is done when all instruction fixtures pass, the 64-bit 7-Zip CLI (`7zz.exe`) compresses and extracts exactly like native, and CoreMark and `7z b` are tracked in CI. Milestones are in `docs/plan.md`.

- **Session 1 (2026-10-22/23):** planned Phase 1 and built M1 with two parallel tracks: the translator (deep-worker) and the targets, PE reader and census (builder).

## Lessons for future sessions

Phase 0's lessons still hold. New ones from M1:

- **Contracts first worked again.** `abi.md` v2, `translator.md` and the `census` CLI skeleton were committed before the agents started, so the two tracks never collided.
- **A CI-iterating agent can commit its own paths.** Use `git commit -- <paths>` and a plain push. Because the working tree is shared, the coordinator's checkpoint commits and the agent's commits land on one local branch and never diverge.
- **Run `rustfmt` on your own files, not `cargo fmt --all`, while another agent works.** The workspace-wide form reformats the other agent's half-written files.
- **iced-x86 decodes `F3 0F BC`/`BD` as `tzcnt`/`lzcnt` by default.** Under a profile without BMI1 and LZCNT, the CPU runs them as `bsf`/`bsr`, and MinGW's gdtoa uses them. Every decoder in the repo must use `x64wasm::DECODER_OPTIONS`.
- **CoreMark only prints "Correct operation validated" for runs of 10 s or more.** Shorter runs count the time check as an error, so the smoke test checks the CRCs instead.
- **Stalled apt mirrors hang runner jobs for 15+ minutes.** `scripts/apt-install.sh` limits each attempt and retries without the Azure mirror; every workflow installs packages through it.
- **Ubuntu's `wine64` package installs its launcher only as `/usr/lib/wine/wine64`.**
- **V8 measurement pitfalls:**
  - `--js-flags` given to Playwright's headless shell has no effect.
  - In Node, `await WebAssembly.compile` never settles under `--no-wasm-lazy-compilation`; use the synchronous `WebAssembly.Module`.
- **wasmtime timings of tight loops depend on code placement on this VM's Xeon.** The same machine code ran 1.9× or 3.0× native depending on the guest base address. Compare at several bases before calling something a regression.

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

## Decisions taken in M1

- **Translator defaults** (measurements below; reasons in `translator.md`):

  | Option | Default |
  |---|---|
  | `max_function_insns` | 2000 |
  | `max_chunk_insns` | 20000 |
  | `call_depth_limit` | 500 |

  The depth limit is 500 rather than the planned ~1000: wasmtime's default stack fits 1487 frames and a Chromium Worker 1879. 500 keeps a 3× margin for frames below the trampoline and for M2's host calls.
- **Flag liveness runs over the whole program.** Calls, returns, indirect jumps and exits read the flags; direct jumps are followed across function and chunk boundaries. This is sound because every boundary stores the whole lazy record.
- **A jump, call or return to an address without a block unwinds; it does not exit.** The trampoline's lookup miss then exits with `EXIT_LEFT_CODE`, so hosts see what they saw in Phase 0, and M2 can resolve such addresses across modules.
- **`F3 0F BC`/`F3 0F BD` decode as `bsf`/`bsr`** (`x64wasm::DECODER_OPTIONS`), as the profile executes them. The census uses the same decoding.
- **Targets:**
  - Sources and pins:
    - 7-Zip 25.01, `7z2501-src.tar.xz`, sha256 `ed087f83ee789c1ea5f39c464c55a5c9d4008deb0efe900814f2df262b82c36e`;
    - CoreMark commit `1f483d5b8316753a742cbf5590caf5bd0a4e4777` (`main` on 2026-10-23), tarball sha256 `4067e7f260218df13f2875d8f821a8ff32a84153c6e0759d7c89f96c0a8bf987`;
    - both pinned on first use from CI and mirrored as `src-7zip-25.01` and `src-coremark-1f483d5`.
  - Build:
    - 7-Zip's own `cmpl_gcc.mak`, with build-system tweaks only, all documented in `tests/targets/7zip/build.sh`: cross-compile variables, case shims for Windows header and library names, symbols kept;
    - `--no-insert-timestamp`, so the builds are bit-for-bit reproducible;
    - our own MIT Win64 CoreMark port with a QueryPerformanceCounter timer.
- **CoreMark smoke check:** short runs never print "Correct operation validated" (the under-10 s check counts as an error). The smoke test checks the performance-run seed CRC and the list, matrix and state CRCs instead.
- **`pause` and multi-byte `nop` are baseline**, since they have no CPUID bit. `xgetbv` is a user-mode instruction (XSAVE, out of profile), not a privileged one.

## What works

| Item | Evidence |
|---|---|
| **Function-level translation** (ABI v2):<br>• one wasm function per guest function or chunk, every block an entry<br>• direct calls as wasm `call`, indirect calls as `call_indirect`<br>• `ret` checked against the expected return address<br>• tail transfers as `return_call`<br>• unwind and exit statuses through the `run` trampoline<br>• call-depth guard | The Phase 0 suites pass unchanged in meaning: replay of all 6480 L1 fixtures through the new emitter, 9 differential `spike_insns` tests, 12 `spike_semantics` tests and the Phase 0 spike cases. Snapshots are regenerated, plus a new `call_ret_tail` snapshot. |
| **New spike programs:**<br>• `fib_rec`<br>• `deep_rec` (depth 100,000)<br>• `tail_rec` (100,000 tail jumps)<br>• `ret_tricks`<br>• `call_table`<br>• `shared_tail`<br>• `longjmp_like`<br>• `big_function` (2,806 instructions, 2 chunks) | All 12 cases are checked natively (`cases_match_native`). They pass in wasmtime under 6 option sets, including every block as its own chunk and every call unwinding. They also pass in Node and in Chromium page and Worker. |
| **Partitioning** (`func.rs`): ownership, shared tails, chunks that keep loop nests whole | 7 unit tests, including 200 random rounds checking that no loop below the hard cap is ever split. |
| **Scaling:** Phase 0's 201k-instruction region, which didn't compile in minutes in Phase 0 | Now 102 chunks. Cranelift takes 9.2-9.8 s, V8 eager TurboFan 2.0-2.1 s, and V8 by default 10-15 ms to compile plus about 53 ms to the first run. |
| **Targets in CI** (`.github/workflows/targets.yml`):<br>• source mirror (trust on first use)<br>• MinGW builds of 7zz.exe and coremark.exe<br>• census and `targets-<sha7>` releases<br>• native smoke tests on windows-2025 and under wine64 on Linux | Runs 39983593568 and 39987562814 (PR head `7613c87`): all 4 jobs green; `targets-4b411bf` and `targets-7613c87` published with identical binaries. `tests/targets/targets.lock` names `targets-7613c87`, whose census uses the profile's decoding. The Windows job does `7zz i`, 7z and zip round trips with `diff -r`, and the CoreMark CRC check; the Wine job does the same. |
| **PE reader** (`x64wasm::pe`): headers, sections, imports, exports, relocations, `.pdata`, TLS, COFF symbols; `function_entries()` for M2 | 14 self-contained tests, including corruption and truncation sweeps. Matches `objdump -p` on 7zz.exe: 186 imports and 8,567 DIR64 relocations. |
| **Census** (`testkit census`): per-`Code`/mnemonic/feature/class counts, profile and exclusions, imports, out-of-profile functions by name, `--check` against the fixtures | 7 unit tests. Run on the `targets-4b411bf` binaries; tables in `docs/design/instruction-scope.md`. |

### Test summary (local, end of session 1, Node 24.22.0)

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: all pass.
  - x64wasm: 11 unit, 14 pe, 4 snapshots, 7 translate.
  - testkit: 15 unit (census and oracle), 15 spike, 9 spike_insns, 12 spike_semantics, replay, replay_harness, replay_inputs.
  - spike_bench: 2 ignored, as they are measurements.
- `cargo run -p testkit --release -- record --check`: "10 fixture files match a fresh recording".
- `cargo run -p testkit --release -- replay`: 6480/6480, 0 unsupported.
- `npm run test:node`: 10/10, with 12/12 spike cases.
- `npm run test:browser`: 4/4, with 12 spike cases in both page and Worker.
- `native/spike/run.sh`: PASS, 19 known differences, 0 failures.
- `testkit census` on the `targets-4b411bf` binaries: as in `instruction-scope.md`. `--check` fails as expected (130 of 491 in-scope forms have fixtures).

## Measurements

Machine: the session's 4-core Xeon @ 2.8 GHz VM. Release builds, each 3 runs. Raw numbers were checked against the logs. Phase 0's build was rebuilt on the same machine for comparison.

**201k-instruction region** (201,253 instructions, 41,252 blocks, 730 KB of code; 2 guest functions):

| `max_function_insns` | chunks | wasm (MB) | Cranelift (s) | V8 eager TurboFan (s) | V8 eager Liftoff (ms) |
|---|---|---|---|---|---|
| 500 | 406 | 5.17 | 8.1-8.6 | 1.53-1.72 | 69-110 |
| 1000 | 204 | 5.03 | 8.5-9.6 | 1.76-1.88 | 70-76 |
| **2000** | 102 | 4.96 | 9.2-9.8 | 2.00-2.11 | 70-83 |
| 5000 | 42 | 4.90 | 11.7-12.5 | 2.29-2.48 | 80-95 |
| 10000 | 22 | 4.89 | 14.4-16.1 | 1.66-1.93 | 91-93 |
| 20000 | 12 | 4.88 | 18.6-19.2 | 2.0-2.1 | 83-113 |

- V8's default lazy compile takes 10-22 ms, and the first `run` about 53 ms at every size.
- In wasmtime, compile, instantiate and the first run take 10.3-10.7 s together. The single `lookup` (a binary search over 41k blocks, about 1 MB of wasm) accounts for about 1-3 s of Cranelift's time.

**Translation:**
- 0.31-0.35 M guest instructions/s without the IR dump, 0.19-0.20 M/s with it. Phase 0 on the same machine: 0.37-0.45 and 0.23 M/s.
- Lifting is about 80 % of the time.
- 24.6 wasm bytes per guest instruction, 6.78 per code byte (Phase 0: 21.9 and 6.0).

**Run time vs native** (native = the same machine code run natively):

| Case | Native | wasmtime | V8 default | V8 TurboFan | V8 Liftoff only |
|---|---|---|---|---|---|
| `sum_loop`, n = 10^8 | 31-35 ms | 2.8-3.0× | 1.23-1.28× | 1.23-1.24× | 7.9× |
| `fib_rec(30)` | 6.4-6.6 ms | 20.8-21.7× | 12.4-12.6× | 10.3-10.7× | 31× |
| `byte_calls`, 1 MiB | 10.3-10.5 ms | 6.8-6.9× | 3.3-3.7× | 2.5-2.6× | 9.3-9.8× |

- **`sum_loop` in wasmtime:** the compiled loop is instruction for instruction the same as Phase 0's.
  - Phase 0's own build ran 1.86-2.06× on this machine, not the recorded 1.02-1.14×.
  - M1 runs 62-63 ms (about 1.9×) at guest bases 0x8000_0000 and 0x1_4000_1000, and 93-107 ms at bases near 0x10_0000. That is code placement, not a regression.
  - In V8, M1 is faster than Phase 0's module: 39.6-41.3 vs 46.7-47.3 ms.
- **Calls cost 10-20× native** because each call and return pair moves about 80 values through the state block. M6's liveness work targets this.

**Deepest guest recursion without the guard** (frames of a small function / of a 2000-instruction function):

| Host | Small | 2000-instruction |
|---|---|---|
| wasmtime (512 KiB stack) | 1487 | 1487-1488 |
| Chromium 141 Worker | 1879-1919 | 2275 |
| Chromium page | 3679-3839 | 3583 |
| Node main thread, default | 3711-3839 | 3839-4095 |
| Node main thread, TurboFan | 8963 | 8656 |

**Targets** (`targets-4b411bf`, rebuilt bit for bit as the locked `targets-7613c87`):
- `7zz.exe`: 4,032,180 bytes, sha256 `8f69827c58da9b407644c4439a296fd4062b68c478b951e484f2360eec959da2`.
- `coremark.exe`: 265,161 bytes, sha256 `6b96da27e6df645047a3f7ca284e8a568556f20756f7b05848efdceb18d846e1`.
- Both identical in `targets-0c3a264`.
- CI times: `sources` about 6 s, `build` about 3¼ min (the 7-Zip build 2 min 21 s), `windows-native` 17 s, `linux-wine` 26 s.

**Census** (details in `docs/design/instruction-scope.md`):
- Sizes:
  - functions: 6,289 / 134 / 95 in 7zz / coremark / hello;
  - instructions: 538,849 in all;
  - distinct `Code`s: 443.
- Classes: 97.4 % integer, 1.6 % SSE2, 0.8 % SSE, 74 x87 instructions, no MMX.
- Out of profile: 56 `Code`s, all in 27 CPUID-dispatched functions of 7zz.exe (AES-NI, SHA-NI, AVX2, SSSE3/SSE4.1 paths and the feature probes).
- In scope: 491 forms over 383 `Code`s; L1 fixtures cover 130 (26.5 %).
- Imports: 189 distinct functions in 5 DLLs (kernel32 113, msvcrt 54, advapi32 14, oleaut32 6, user32 2).

## Known issues and risks

- **Calls are expensive** (10-20× native) until M6 syncs only live registers at boundaries.
- **`lookup` is a compiled binary search.** At 41k blocks it is a 1 MB function and 1-3 s of Cranelift time. M2 replaces it with the planned hash table in the host region.
- **Returns to a non-block address exit with `EXIT_LEFT_CODE`** until run-time translation exists (M2). This covers a return address adjusted past a block start.
- **Out-of-profile code in 7zz.exe:** 27 CPUID-dispatched functions. M2's whole-image AOT must turn them into trap stubs or leave them out.
- **22 forms occur only in gap sweeps** (likely data read as code: `std`, `cmc`, `int1`, `loop`, …). The rule as decided counts them; the M5 gate decides whether to drop them.
- **`testkit census --check` fails** (130 of 491) until the instruction milestones add fixtures. CI doesn't gate on it before M5.
- **wasmtime loop timings depend on code placement** on this VM. Benchmark baselines in M4/M5 should come from V8 and same-machine native runs.
- **apt installs in CI were flaky:** the Azure Ubuntu mirror trickled at about 40 KB/s, and 3 targets runs hung or timed out. `scripts/apt-install.sh` now limits each attempt and retries without that mirror; a mock test covered both the stall and the failure path.
- **One release publish failed with a transient 403.** The `targets` run of `6d4536c` was refused by the GitHub releases API with "Resource not accessible by integration", although its token had `contents: write`. `hello-7613c87` published 38 s later, and the same step published `targets-7613c87` on the next commit. If it recurs, re-run the job once, or retry the publish step.
- **Phase 0's translator gaps remain until M3:** `lock` and atomics, FS/GS, the 67h prefix, precise faults.

## Next step: M2 (PE loading)

1. Whole-image AOT translation from `x64wasm::pe`. Function entries from `Pe::function_entries()`: entry point, exports, `.pdata`, TLS callbacks, relocation targets in code. Also jump-table recovery, and trap stubs for out-of-profile functions.
2. A shared table and a hash-table `lookup` in the host region, so several modules (AOT image, run-time translations) link together.
3. A JS runtime process:
   - guest memory, TEB/PEB, stack, static TLS;
   - import thunks as wasm-import host calls;
   - run-time translation of missed code with the translator built for wasm32.
4. Win32 shim v0, and a `-nostdlib` MinGW test PE that runs translated in Node with output equal to native.
5. Translate all of 7zz.exe and coremark.exe, and measure translation and compile time on real code in V8 and wasmtime.
