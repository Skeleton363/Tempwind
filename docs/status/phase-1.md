# Phase 1 status: Translator

**M1-M4 are done (M4, SSE floating point, x87 and CoreMark: PR #6, merged); M5 (7-Zip exactly like native) is in progress.** Phase 1 is done when all instruction fixtures pass, the 64-bit 7-Zip CLI (`7zz.exe`) compresses and extracts exactly like native, and CoreMark and `7z b` are tracked in CI. Milestones are in `docs/plan.md`.

- **Session 1 (2026-10-22/23):** planned Phase 1 and built M1 with two parallel tracks: the translator (deep-worker) and the targets, PE reader and census (builder).
- **Session 1, continued (2026-10-23/24):**
  - commits in the owner's name and the working-efficiently rules;
  - then M2 with three agents: the Rust side and the JS runtime (deep-worker each), and the test program and CI (builder);
  - paused once and resumed.
- **Session 1, continued (2026-10-24/30):** M3 with the oracle, translator and CRT-runtime agents (deep-worker each) and the builder, paused twice to stay within the owner's weekly usage.
- **Session 1, continued (2026-10-30/31):** M4 with the soft-float, fixtures, SSE-translator and x87-translator agents (deep-worker each) and the builder, paused twice for the owner's usage limits and once by a worker restart.

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
- **A stopped background agent can be resumed.** Sending it a message continues it from its transcript, so a pause costs no re-reading.
- **Checkpoint agents' work when a session must stop on a clean tree.** Commit their paths as pathspec-limited `WIP` commits, and have agents checkpoint about every 20 minutes.
- **Measure on representative code.**
  - While discovery stopped at the first unsupported instruction, 7zz.exe translated 64k of its 521k instructions.
  - Its module then looked about 5× smaller and much faster to compile than the real one.
- **GCC's default PE linker script puts `__CTOR_LIST__`/`__DTOR_LIST__` data into `.text`.** A linear sweep decodes that data as code; `tests/pe/basic.ld` is the default script without it.
- **V8 compile measurements need one process per configuration.** "Eager TurboFan" is `--no-liftoff --no-wasm-lazy-compilation`. `--no-wasm-lazy-compilation` alone compiles every function with Liftoff.
- **BigInt arithmetic is slow in hot JS loops.** Linking 41k blocks took 79 ms with BigInt hashing, and 8.5 ms with 32-bit Numbers.
- **Replay time is wasm compile time.** Nearly every L1 case has its own encoding: 36,831 modules for 44,256 cases, with compiling at about 95 % of the time. Cranelift's fast settings cut replay by about 30 %; caching per form can't help.
- **Integer programs need SSE2 integer forms.** GCC -O2 vectorizes integer loops, and MinGW's CRT startup and printf execute `punpck*`, `pshufd`, `paddd` and `packuswb`. So the split between milestones is "exact integer results" against "floating-point arithmetic".
- **Undefined bits are recorded as the input's values.** That keeps recordings identical across CPU models (the session VM and GitHub's runners).
- **A fresh agent continues a WIP checkpoint well from the approved design docs.** Resuming a stopped agent reloads its whole transcript, about 1.6 MB here.
- **Test against the CPU in-process for exact arithmetic.** `x64wasm-fp`'s inline-asm differential tests ran 850M cases, found the CPU's undocumented choices (C1 after fcomi, fprem's reduction), and made the fixtures pass on the first lowering.
- **Name implementation-dependent behaviour early.** The fixtures and native tests must agree on every vendor CI may use, so mask or vendor-gate what the SDM leaves open before recording.
- **A zeroed state block isn't a thread's state.** With MXCSR = 0 every SSE exception is unmasked; hosts must set the FP defaults a real thread starts with.
- **A worker restart stops background agents, but their transcripts survive.** A message resumes them where they were; the coordinator commits their uncommitted files in the meantime.

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

## Decisions taken in M2

- **Attribution:** commits and PRs carry only the owner's identity, `Skeleton363` with the GitHub no-reply address, unsigned.
  - `.claude/settings.json` turns Claude Code's attribution off, and the SessionStart hook sets the identity.
  - Past commits stay as they are (the owner's choice).
- **Working method:**
  - one session per phase, one PR per milestone;
  - agents only where they pay off;
  - the "Working efficiently" rules in CLAUDE.md.
- **ABI v3** (`abi.md`):
  - Modules link through an imported funcref table, `table_base`, a shared `depth`, and a hash-table lookup in guest memory: header at `env.lookup`, multiplicative hash, linear probing, first translation wins.
  - Transfers out of a module probe that table.
  - Host functions are table entries that perform the guest's `ret`.
  - `EXIT_HOST` is a new exit reason.
  - Each module carries an `x64wasm.blocks` section.
- **Host calls:** an import's IAT slot points at a 16-byte thunk in the host region. The thunk's lookup slot names the host-call function (table slot 0), with the import id as entry.
- **Run-time translation:**
  - A lookup miss in an executable section exits with `EXIT_LEFT_CODE`.
  - The runtime then translates from that address with `x64wasm_rt.wasm`, passing every known block address, links the new module and resumes.
- **The load plan comes from the Rust PE reader** (`translate-pe`). JS never parses PE files.
- **Process layout** (`runtime.md`): offsets checked against Wine 11's `winternl.h`.
  - Memory is the larger of the image end and 4 GiB.
  - Images load at their preferred base.
  - PEB at `0x7FFD_0000`, TEB at `0x7FFD_1000`, stack top at `0x7FF0_0000`.
  - The host region starts at 2 GiB: lookup header, state block, thunks, runtime heap.
- **Shim:** strict, as Microsoft documents the APIs. An unhandled case or an unimplemented API stops the program and names the API.
- **AOT discovery continues past unsupported instructions** (`Block::after_exit`). The instruction still exits at run time, but the rest of its function is translated, so a module's structure doesn't depend on which instructions are supported yet.
- **Jump tables:**
  - GCC's relative tables are recovered by pattern: bounds check, `lea`, `movsxd`, `add`, `jmp`.
  - Every target must lie in executable code, otherwise the whole table is rejected.
  - Tables that recovery misses are found at run time.
- **Out-of-profile functions translate like any other code:** their unsupported instructions become exits.
- **Test program:**
  - `tests/pe/basic.exe` is written in assembly, so it uses only forms with L1 fixtures; `census --check` gates it in CI.
  - The `jmp` fixtures were recorded first.
  - Its exit code lives in `tests/pe/basic.exitcode`.

## Decisions taken in M3

- **Split with M4:** M3 covers every in-scope integer form, plus every SSE/SSE2 form whose result doesn't depend on floating-point arithmetic (moves, bitwise ops, packed integer arithmetic, compares, shifts, multiplies, shuffles, unpacks, packs) and `fninit`. M4 takes the SSE floating-point arithmetic, compares and conversions, and x87. `hello.exe` moves to M4, because it prints floats.
- **Phase 1 semantics** (`abi.md`):
  - `lock` is an ordinary read-modify-write, which is exact with one thread;
  - FS/GS add their bases from the state block, and 67h computes 32-bit addresses;
  - DF lives in the state block's RFLAGS;
  - XMM, MXCSR and the x87 control words live in the state block (M4 may cache XMM in locals);
  - x87 R0-R7 are preserved.
- **Faults:** `FAULT_DIVIDE` (#DE) 4, `FAULT_INVALID_OPCODE` (#UD) 5, `FAULT_GENERAL_PROTECTION` (#GP) 6, raised before any side effect, with RIP at the instruction.
  - Bytes that don't decode raise #UD.
  - Decodable instructions outside the CPUID profile stay `EXIT_UNSUPPORTED`, for clear diagnostics.
- **Lowering** (`translator.md`, "Instruction lowering and lazy flags (M3)"):
  - lazy flag kinds for logic, inc/dec, adc/sbb, shifts, mul;
  - `FlagsUpdate` records for instructions that change only some flags;
  - a `div128` helper after the chunks, outside the table;
  - the 67h forms of string ops, `loop` and `jecxz`, and `int1`, stay unsupported.
- **Fixture format v2** (`docs/design/fixtures.md`):
  - SSE and x87 state, FS/GS bases, faults as outcomes;
  - per-case undefined-register masks;
  - `rep` instructions recorded step by step;
  - undefined bits normalized to the input's values.
- **Generators:** a generic form generator on iced's `OpCodeInfo`, plus hand-written ones for division, string ops, `loop`/`jrcxz` and the miscellaneous forms.
- **Replay** compiles with Cranelift's fast settings; results are identical.
- **CRT runtime** (`runtime.md`):
  - calls from the host into guest code save and restore the state block and `depth`, and nest at most 16 deep;
  - data imports get storage behind their IAT slot;
  - the msvcrt and kernel32 shims are strict, following Microsoft's documentation and Wine's `dlls/msvcrt`;
  - stdout is in text mode (LF → CRLF), and the L2 checks compare the exact CRLF bytes;
  - the file system is empty: `fopen` for reading fails with ENOENT, and writing stops;
  - `Sleep` returns at once, and `_amsg_exit` writes its message to stderr;
  - the `strerror` texts were written from memory, and only the four codes the L2 programs print are checked.

## Decisions taken in M4

- **Scope** (`instruction-scope.md`, "M4 additions to the scope"):
  - every SSE/SSE2 floating-point form, scalar and packed, except the approximations `rcp*`/`rsqrt*` (they differ between Intel and AMD) and the conversions with MMX operands;
  - x87: the census forms, which now include the float L2 program and MinGW's fenv and rounding functions, plus the control forms those use (`fnstcw`, `fldcw`, `fnstsw`, `fnstenv`, `fldenv`, `fnclex`, `fwait`). The transcendentals are excluded, because their results depend on the CPU model.
  - **Why the whole SSE set:** Phase 2's Wine DLLs and later programs use it, and once the soft float and the generators exist, each form costs little. Coming back later costs more.
- **`cpuid` moves from M5 to M4:** MinGW's `fesetround`, `feclearexcept` and `fetestexcept` call `__mingw_has_sse`, which executes it. It returns the profile with a vendor of our own, "TempwindVCPU" (values in `instruction-scope.md`).
- **Exactness:** results, the MXCSR and FSW exception flags, C1 and the x87 tags are bit-exact under every rounding mode, FTZ/DAZ and x87 precision control. An exception that is unmasked and actually occurs exits with `EXIT_UNSUPPORTED`; #XM and #MF come with Windows exceptions in Phase 4.
- **Soft float in a helper module** (`crates/x64wasm-fp`, `x64wasm_fp.wasm`, ABI v4): `no_std` integer-only Rust. Translated modules import the helpers they use as `fp.*`, and testkit links the same functions natively in wasmtime.
  - Plain Rust can be checked natively against the CPU with inline asm on millions of cases. Hand-written wasm like `div128` would mean thousands of lines that only the fixtures check.
- **Inline fast path** (`translator.md`, "Floating point and x87 (M4)"): the plain wasm instruction runs only when its result is provably exact and no flag can change. For arithmetic that means all exceptions masked, round to nearest, PE already set, no denormal operand and a normal result. Everything else calls the helper. x87 arithmetic always calls helpers.
- **No new IR value types:** FP values travel in `VTemp`s.
- **XMM stays in the state block:** caching it in locals waits for M6's liveness work, since every boundary already syncs about 80 values.
- **Float formatting is guest code:** hello.exe, coremark.exe and the L2 programs print floats with MinGW's own printf (pformat and gdtoa), so the shim needs no float formatting. CoreMark needs only `QueryPerformanceCounter` and `QueryPerformanceFrequency` added.
- **CoreMark tracking** follows the decision taken when Phase 1 was planned (`runtime.md`, "Benchmarks (M4)"):
  - `bench.yml` runs it translated in Node 24 and natively under wine64 on the same runner, with a windows-2025 native run for the record;
  - each run calibrates itself to at least 10 s, so it is validated;
  - main appends one JSON line to the orphan `bench-data` branch, and PRs warn when the ratio is more than 10 % worse.

- **A thread's FP state** starts as on Windows x64 (MXCSR 0x1F80, FCW 0x027F, empty x87 stack) and carries over between the host's calls on that thread (`runtime.md`). The runtime used to zero the whole state block, which unmasked every SSE exception.
- **`l2_float` is built with `-ftree-vectorize`:** at plain `-O2`, GCC 13 vectorized none of its loops.
- **Implementation-dependent results stay out of the checks:**
  - which they are: fprem's partial reduction, the sign when two x87 NaNs tie, the reserved FCW bits, and the pointers, selectors and reserved halves that fnstenv stores;
  - the fixtures mask them, and `x64wasm-fp`'s native tests compare them bit for bit only on GenuineIntel, the behaviour the soft float follows.
- **Where this CPU differs from the SDM's text, the recordings win** (`translator.md`):
  - fcomi, fucomi and fucomip leave C1 unchanged;
  - fxch and fchs on an empty register give the indefinite;
  - fnstenv stores ES and B as they were, then clears them.
- **Neighbouring x87 forms come along:** the x87 lowering covers forms that share code with the scope but have no fixtures yet (fsub/fsubr, the fi* arithmetic, fst/fist without pop, fcom/fucom/fcompp, fld1, fstcw/fstsw/fclex/fstenv).

## M5 progress (paused 2026-10-30)

**Committed:**
- **Contracts** (`4562517`): the file system and shim spec in `runtime.md`, the matrix spec in `tests/sevenzip/README.md`, and `int1` excluded, so `census --check` passes on all programs.
- **The matrix** (`f62e516`, `e78ebc5`, `bbb608e`):
  - 138 steps in 40 groups, covering 7z, zip, tar, gz, bz2 and xz, the hashes, and AES round trips;
  - `expected/` (472 files, 5.7 MB), from two identical Wine runs;
  - CI: windows-x64 runs the native matrix and linux-x64 the translated one, both gating, and the census includes 7zz.exe;
  - `7z b 2 -mmt1 -md18` in `bench.yml`.
- **The shim, in progress** (WIP `57ff6b9`, `82c37ec`): the file system and its host mount, handles, kernel32 files and system calls, advapi32, oleaut32, user32, cp437 and msvcrt additions. Agent S was stopped mid-work; at the stop it had a 7-Zip command completing and was comparing its output with Wine.
- **`bench-data`** exists, with the first CoreMark line (on an AMD EPYC 9V45 runner).

**On resume:**
1. **Read the result of CI run 41138603006** (a `workflow_dispatch` of `ci.yml` at `bbb608e`): does the windows-x64 native matrix match `expected/`? Wine may differ from Windows in zip's NTFS extra field or the file attributes. If it differs, correct `expected/` from the run's uploaded results before the shim targets it. The translated linux step fails until the shim is done.
2. **Resume agent S** (or a fresh deep-worker from the committed docs and code) until `matrix.mjs --mode translated` matches `expected/`.
3. **Integration:** measurements, this note, then PR "[Phase 1] 7-Zip exactly like native" with CI green.

## Decisions taken in M5

- **Gap-only forms (the M5 gate):**
  - `int1` (ICEBP, a debugger's #DB trap) joins the scope rule's exclusions;
  - the other 21 gap-only forms stay, and all have fixtures;
  - `census --check` now passes on every program, 7zz.exe included (614 of 614 forms), and CI gates it.
- **Look things up before implementing.** The shim reads three sources before writing a function:
  - Wine's source (the wine-11.0 tarball from dl.winehq.org) for Windows' details;
  - Microsoft's documentation for the contract;
  - 7-Zip's own source (release `src-7zip-25.01`, unpacked by the targets build) for what 7zz actually calls and when.

  That replaces trial-and-error runs. Our code stays our own: Wine is read, not copied. The clean-room list in `CLAUDE.md` still holds.
- **An in-memory file system with Win32 and NTFS semantics** (`runtime.md`, "Files, handles and the 7-Zip shim (M5)"). `run-pe --dir` loads a host directory as `C:\work` and writes it back. It ports to browsers (OPFS) later.
- **The shim stays strict:**
  - **Exceptions:** stop loudly (Phase 4).
  - **Thread creation:** stops loudly (Phase 1 has one thread, `-mmt1`).
  - **`GetProcAddress`:** stops on a function the shim lacks rather than returning NULL, so no run takes a fallback that native Windows wouldn't.
  - **Machine profile:** fixed and documented: 1 processor, UTC, an empty registry and environment, redirected stdout.
- **Cooperative guest threads with JSPI, in M5** (your decision, 2026-10-31). 7-Zip always runs a second thread for 7z archives with BCJ, BCJ2 or AES, and for xz, whatever `-mmt` says. `-mtf=off` makes native 7-Zip fail, so no switch avoids it. So guest threads take turns on one JS thread, suspending their wasm stacks at blocking calls through JSPI (`runtime.md`, "Guest threads (M5)").
  - It keeps all 138 matrix steps, and lets common real-world archives extract.
  - Real parallel threads stay in Phase 4.
- **The command matrix** (`tests/sevenzip/README.md`):
  - deterministic inputs and switches;
  - expected outputs committed from a native run, with real Windows gating them in CI and Node gating the translated run;
  - encryption is checked by round trip only, since its salt and IV are random.

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

| **Linked modules** (ABI v3): shared table, hash lookup in guest memory, calls and jumps between modules, host functions | `crates/testkit/tests/link.rs` has 8 tests: two modules calling each other, a miss then link and resume, host calls that return, unwind or exit, and first translation wins. The Phase 0 and M1 suites also pass on v3 modules: L1 replay, the spike cases in wasmtime, Node, and Chromium page and Worker, and the snapshots. |
| **Whole-image AOT** (`x64wasm::image`, `translate-pe`):<br>• load plan<br>• entries from `function_entries()`<br>• decoding only inside executable sections<br>• jump tables<br>• discovery past unsupported instructions | 7zz.exe and coremark.exe translate, pass wasmparser validation, and compile in wasmtime and V8 (measurements below). 133 of the 138 GCC jump tables in 7zz.exe are recovered; 6 near-miss tests check what must not count as a table. |
| **Run-time translator** (`crates/x64wasm-rt`, wasm32) | 445,419 bytes, 147,513 gzipped. Request handling is unit-tested, and `basic.exe` uses it end to end. |
| **JS runtime** (`runtime/src/`):<br>• linker<br>• load-plan checks and mapping<br>• thunks and IAT<br>• PEB, TEB, stack and TLS<br>• host calls<br>• shim v0 (`GetStdHandle`, `WriteFile`, `ExitProcess`)<br>• run loop and run-time translation<br>• traces and the `run-pe` CLI | 114 Node tests, including `basic.exe` in both AOT variants. |
| **M2 test program** (`tests/pe/basic.exe`). It exercises:<br>• output<br>• recursion 1000 deep, past the depth guard<br>• fib<br>• a function-pointer table<br>• a jump table<br>• a function that only a computed pointer reaches<br>• exit code 42 | It matches `basic.expected` and exit code 42 natively (under Wine 9 here, on Windows in CI) and translated in Node. That holds from all entries (1 run-time translation) and from the entry point only (8). `census --check`: 25 of 25 forms have fixtures. |
| **`jmp` fixtures** | 192 cases (rel8, rel32, r64, m64); replay passes 100 %. |

| **Fixture format v2 and generators** | 67 families, 44,256 cases, recorded natively. `record --check` reproduces all 67 files. |
| **M3 lowering:**<br>• every in-scope integer form<br>• SSE/SSE2 integer, bitwise and move forms<br>• `fninit`<br>• FS/GS and 67h addressing<br>• `lock`<br>• `rep` string ops<br>• #DE, #UD and #GP faults | Replay 44,256/44,256, with 0 unsupported. |
| **CRT runtime:** nested guest calls, data imports, the msvcrt and kernel32 functions of the L2 programs | Node tests, including startup, TLS callbacks, `_initterm`, constructors and `atexit`, and `qsort` calling the guest comparator. |
| **L2 programs** `l2_int.exe` and `l2_crt.exe`: integer code, MinGW CRT startup and printf | Translated in Node, both match their expected CRLF output and their exit codes (0 and 37). Under Wine 9 the native runs match too; Windows checks them natively in CI. |
| **Soft float** (`crates/x64wasm-fp`, `x64wasm_fp.wasm`, 24,671 bytes): SSE f32/f64 and x87 80-bit arithmetic, compares and conversions, with exact flags, all rounding modes, FTZ/DAZ and x87 precision control | Native differential tests against the CPU: 10M cases in `cargo test`, 850M in the long runs, no mismatch. The Node test checks the wasm build on 32,800 vectors. |
| **SSE/SSE2 floating point** (116 forms): inline fast path when provably exact, helpers otherwise, MXCSR flags, comis flags | Replay of `sse_fp_arith`, `sse_fp_cmp` and `sse_fp_cvt`: 12,992/12,992. |
| **x87** (44 forms): stack, tags, stack faults, C1, the control forms, fcomi flags | Replay of the five `x87_*` families: 2,688/2,688. `x87_paths` checks that unmasked and pending exceptions exit before any change. |
| **`cpuid`**: the profile, vendor "TempwindVCPU" | The dedicated test checks every leaf. |
| **hello.exe, `l2_float.exe` and CoreMark** translated | Node: exact CRLF output and exit codes (hello 0, l2_float 23 with 401 lines); CoreMark's run type and CRCs as in `smoke.sh`. All programs translate with 0 unsupported sites. |
| **Runtime for M4:** the `fp` namespace in every host, `run-pe --fp` and `-- ARGS`, `QueryPerformanceCounter`/`Frequency`, the thread's FP state | Node tests (152 in all). |
| **CoreMark benchmark** (`tests/bench/coremark.mjs`, `bench.yml`) | Validated runs translated and under wine64 (measurements below). |

### Test summary (local, M4, Node 24.22.0)

- `cargo fmt --all --check`, workspace clippy, and the wasm32 builds of `x64wasm-rt` and `x64wasm-fp`: clean.
- `cargo test --workspace`: 192 passed, 0 failed. That includes replay of 59,936/59,936 cases with 0 unsupported, the soft float's native tests, `cpuid`, `x87_paths` and the snapshots.
- `record --check`: "75 fixture files match a fresh recording" (on the session's Intel Xeon).
- `census --check`:
  - basic.exe, coremark.exe, hello.exe and the three L2 programs: 426 of 426 forms;
  - 7zz.exe: 482 of 483, missing only the gap-only `int1`.
- `npm run test:node`: 152/152, including hello, l2_float, CoreMark and the fp vectors.
- `npm run test:browser`: 4/4.

### Test summary (local, M3, Node 24.22.0)

- `cargo fmt --all --check`, and clippy for the workspace and for `x64wasm-rt` on wasm32: clean.
- `cargo test --workspace`: 155 passed, 0 failed, 5 ignored (measurements).
- `record --check`: "67 fixture files match a fresh recording". Replay: 44,256/44,256, 0 unsupported.
- `tests/pe` and `tests/native` build and prepare: OK. `census --check` on `basic.exe`: OK.
- `npm run test:node`: 144/144, including both L2 programs. `npm run test:browser`: 4/4.
- Census of the targets plus the L2 programs: fixtures cover 484 of 509 in-scope forms. The 25 left are M4's floating-point and x87 forms and `int1`.

### Test summary (local, M2, Node 24.22.0)

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and clippy on `x64wasm-rt` for wasm32: clean.
- `cargo test --workspace`: 120 passed, 0 failed, 5 ignored (measurements).
- `record --check`: "11 fixture files match a fresh recording" (6,672 cases).
- `census target/pe/basic.exe --check`: 25 of 25 forms have fixtures.
- `tests/pe/build.sh` and `tests/pe/prepare.sh`: OK. The build is reproducible: two builds gave the same sha256.
- `npm run test:node`: 114/114, with 12/12 spike cases.
- `npm run test:browser`: 4/4.
- Census on the targets: fixtures cover 134 of 491 in-scope forms. It fails, as expected until M5.
- `native/spike/run.sh` was not rerun, since `native/` didn't change; CI runs it.

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

**Whole images (M2).** Translations take all entries and use the targets release. For wasmtime, Cranelift's median of 3 is reported; for V8 (Node 24.22.0), the median of 3, with each configuration in its own process.

| | coremark.exe | 7zz.exe |
|---|---|---|
| `translate-pe` wall time / peak RSS | 25 ms / 11 MiB | 1.6-1.7 s / 360 MiB |
| wasm, raw / gzip -9 | 475,759 / 65,592 B | 29,300,487 / 3,502,880 B (59 B per guest instruction) |
| functions / chunks / blocks | 175 / 175 / 3,933 | 7,600 / 7,612 / 208,426 |
| guest instructions (share of the census) | 8,955 (92.6 % of 9,668) | 492,876 (94.6 % of 520,823) |
| `.pdata` functions translated | 134 of 134 | 6,289 of 6,289 |
| indirect jumps / recovered tables (targets) | 42 / 2 (17) | 344 / 133 (6,115) |
| `EXIT_UNSUPPORTED` sites | 1,696: `test` 473, `xor` 286, `movzx` 133 | 81,372: `test` 26,873, `xor` 14,163, `movzx` 7,035, `shl` 4,407, `movups` 4,302 |
| wasmtime, Cranelift compile | 678 ms | 47.6 s |
| V8 compile: default (lazy) / eager Liftoff / eager TurboFan | 1.1 / 10.9 / 142 ms | 48.7 / 464 / 9,140 ms |
| V8 instantiate and link | 1.6 ms | 48.5-51.3 ms |

- **What the sizes contain:** the unsupported sites dominate size and Cranelift time, since each one stores the full state. They shrink as M3 and M4 lower those instructions, and M6 narrows the state syncs. V8's default lazy path compiles 7zz.exe in under 50 ms anyway.
- **Linking** (`runtime/src/link.mjs`, 41,252 blocks): 8.5 ms (4.9 M blocks/s), down from 79 ms with BigInt hashing.
- **Run-time translation** (`basic.exe`):
  - The first translation takes about 24 ms, mostly compiling `x64wasm_rt.wasm`; each later one about 0.6 ms.
  - The entry-point-only variant needs 8 translations, because AOT already finds 46 of its 53 blocks by following direct calls.

**M3:**
- **Replay** of 44,256 cases (36,831 modules): debug test 63 → 50 s, release 42 → 27 s, after switching to Cranelift's fast settings.
- **`EXIT_UNSUPPORTED` sites:**
  - 7zz.exe: 81,372 → 1,202. What's left is out of profile (AVX, AES, SHA, SSSE3, SSE4.1), M4 floating point and x87, 14 `cpuid` and 1 `xgetbv`.
  - coremark.exe: 87.
  - hello.exe, `l2_int.exe` and `l2_crt.exe`: 77 each.
- **7zz.exe after M3:** 129,966 blocks (was 208,426: unsupported instructions no longer split blocks). The module is 27.6 MB, 2.9 MB gzipped, and translates in 1.4 s.

**M4** (the session's 4-core Xeon @ 2.10 GHz, Node 24.22.0):
- **Replay** of 59,936 cases in 75 families takes 41 s in a release build (M3: 27 s for 44,256). M4 adds 15,680 cases (8.5 MB); the fixtures total 26 MB.
- **`EXIT_UNSUPPORTED` sites:**
  - 7zz.exe: 1,202 → 907, all forms outside the Phase 1 scope (the CPUID-dispatched AVX, AES, SHA, SSSE3 and SSE4.1 paths);
  - coremark.exe: 87 → 0;
  - hello.exe and the L2 programs: 0.
- **7zz.exe:** 129,683 blocks; the module is 27.6 MB (2.9 MB gzipped) and translates in 1.8 s.
- **Sizes:**
  - the run-time translator `x64wasm_rt.wasm`: 445,419 → 707,040 bytes;
  - the fp module: 24,671 bytes (9,866 gzipped);
  - an x87 instruction: 400-600 wasm bytes, so about 125 KB of `l2_float`'s 640 KB module.
- **FP fast path:** a dot product of 10^7 doubles (movsd, mulsd, addsd), against native:

  | Engine | Time | × native |
  |---|---|---|
  | V8 default | 88.3 ms | 6.9× |
  | V8 TurboFan | 90.9 ms | 7.0× |
  | V8 Liftoff only | 125.6 ms | 9.8× |
  | wasmtime | 91 ms | 6.5× |
  | native | 12.9 ms | |

  The integer `sum_loop` runs at 1.0× in the same run. The cost is XMM in the state block (loads and stores around every operation), the MXCSR load and the operand tests: M6's work.
- **CoreMark** (`tests/bench/coremark.mjs`, medians of 3 validated runs):
  - translated in Node: 7,379 iterations/s;
  - native under wine64: 24,684 iterations/s;
  - ratio 0.30.

**Census** (details in `docs/design/instruction-scope.md`):
- Sizes:
  - functions: 6,289 / 134 / 95 in 7zz / coremark / hello;
  - instructions: 538,849 in all;
  - distinct `Code`s: 443.
- Classes: 97.4 % integer, 1.6 % SSE2, 0.8 % SSE, 74 x87 instructions, no MMX.
- Out of profile: 56 `Code`s, all in 27 CPUID-dispatched functions of 7zz.exe (AES-NI, SHA-NI, AVX2, SSSE3/SSE4.1 paths and the feature probes).
- In scope: 491 forms over 383 `Code`s for the targets; 509 with the M3 L2 programs. After M3, L1 fixtures cover 484 of the 509.
- Imports: 189 distinct functions in 5 DLLs (kernel32 113, msvcrt 54, advapi32 14, oleaut32 6, user32 2).

## Known issues and risks

- **Calls are expensive** (10-20× native) until M6 syncs only live registers at boundaries.
- **22 forms occur only in gap sweeps** (likely data read as code: `std`, `cmc`, `int1`, `loop`, …). The rule as decided counts them; the M5 gate decides whether to drop them.
- **`testkit census --check` on 7zz.exe** misses only the gap-only `int1`; the M5 gate decides on gap-only forms. CI gates basic.exe, coremark.exe, hello.exe and the L2 programs, which pass.
- **Floating point is slow:** about 7× native in a dot product (XMM in the state block, an MXCSR load and operand tests per operation). x87 code is large (400-600 wasm bytes per instruction). Both are M6's.
- **Unmasked floating-point exceptions** exit with `EXIT_UNSUPPORTED` until Windows exceptions (Phase 4).
- **Checked only on Intel so far:**
  - `record --check`;
  - the soft float's native tests;
  - fnstenv's ES/B recordings.

  The implementation-dependent cases are masked or vendor-aware. CI may run on AMD, and its results decide.
- **x87 forms lowered without fixtures** (the neighbours under "Decisions taken in M4") get fixtures when a census needs them.
- **Shim gaps:**
  - `strerror` is verified only for EACCES, ENOENT, EINVAL and EEXIST;
  - best-fit character mapping is missing;
  - `VirtualQuery` works only inside the image;
  - TLS callbacks don't get DLL_PROCESS_DETACH at exit.

  Each of these stops loudly.
- **7zz.exe's module is large under Cranelift:** 27.6 MB after M4. M6 narrows the state syncs at boundaries.
- **5 of 138 jump tables in 7zz.exe aren't recovered:** 4 load the table base earlier in the function, and 1 sits in a handler nothing reaches. Run-time translation covers them.
- **The run-time translator compiles on first use** (about 24 ms). Compiling it while the image loads would hide that.
- **Runtime limits in Phase 1:**
  - one thread;
  - images only at their preferred base (no relocation);
  - no SEH or exceptions;
  - KUSER_SHARED_DATA is zeroed;
  - shim v0 has three functions.
- **wasmtime loop timings depend on code placement** on this VM. Benchmark baselines in M4/M5 should come from V8 and same-machine native runs.
- **apt installs in CI were flaky:** the Azure Ubuntu mirror trickled at about 40 KB/s, and 3 targets runs hung or timed out. `scripts/apt-install.sh` now limits each attempt and retries without that mirror; a mock test covered both the stall and the failure path.
- **One release publish failed with a transient 403.** The `targets` run of `6d4536c` was refused by the GitHub releases API with "Resource not accessible by integration", although its token had `contents: write`. `hello-7613c87` published 38 s later, and the same step published `targets-7613c87` on the next commit. If it recurs, re-run the job once, or retry the publish step.
- **Phase 0's translator gaps remain until M3:** `lock` and atomics, FS/GS, the 67h prefix, precise faults.

## Next step: M5 (7-Zip exactly like native)

`docs/plan.md`: the command matrix (formats, methods, `t`, `l`, `x`, `h`) matches native Windows byte for byte, every in-scope form has fixtures with replay at 100 %, and `7z b` is tracked.

1. **The gap-only forms:** decide whether the 22 forms seen only in gap sweeps (`int1`, `std`, `cmc`, `loop`, …) stay in scope. That closes `census --check` on 7zz.exe.
2. **The shim for 7zz.exe's imports** (189 functions: kernel32 113, msvcrt 54, advapi32 14, oleaut32 6, user32 2), with an in-memory file system for archives and trees. It stays strict, following Microsoft's documentation and Wine's source.
3. **The command matrix:** native outputs from windows-2025 in CI, translated runs in Node, compared byte for byte, with fixed timestamps so archives are deterministic.
4. **`7zz b -mmt1`** in `bench.yml` next to CoreMark.
