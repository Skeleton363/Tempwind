# Phase 1 status: Translator

**Phase 1 is done: M1-M6 are built, and M6 (speed and close-out) is in PR #9, which the owner merges.** Phase 1's exit criteria hold: all instruction fixtures pass, the 64-bit 7-Zip CLI (`7zz.exe`) compresses and extracts exactly like native (138 of 138 matrix steps), and CoreMark and `7z b` are tracked in CI. Milestones are in `docs/plan.md`; the next phase is Phase 2, Wine underneath.

- **Session 1 (2026-10-22/23):** planned Phase 1 and built M1 with two parallel tracks: the translator (deep-worker) and the targets, PE reader and census (builder).
- **Session 1, continued (2026-10-23/24):**
  - commits in the owner's name and the working-efficiently rules;
  - then M2 with three agents: the Rust side and the JS runtime (deep-worker each), and the test program and CI (builder);
  - paused once and resumed.
- **Session 1, continued (2026-10-24/30):** M3 with the oracle, translator and CRT-runtime agents (deep-worker each) and the builder, paused twice to stay within the owner's weekly usage.
- **Session 1, continued (2026-10-30/31):** M4 with the soft-float, fixtures, SSE-translator and x87-translator agents (deep-worker each) and the builder, paused twice for the owner's usage limits and once by a worker restart.
- **Session 1, continued (2026-10-30 to 11-04):**
  - M5 with the shim and the guest-threads agents (deep-worker each) and the builder (the matrix, the workflow changes, the review fixes);
  - paused for the owner's usage limits, and resumed after a session restart.
- **Session 1, continued (2026-11-04 to 11-06):** M6 in five work packages (WP1 and WP5 builder; WP2, WP3a, WP3b and WP4 deep-worker), paused twice for the owner's usage limits. Along the way: the workflow trials, a ledger that measures real usage, and the license change (the owner's decisions).

## Phase 1 summary

- **Built:** an x86-64 to WebAssembly (memory64) translator (iced-x86 decoding, our IR, wasm-encoder), whole-image AOT plus a run-time translator built for wasm32, bit-exact soft float for SSE and x87, and a JS runtime with a Win32 shim and cooperative guest threads.
- **Correctness:** 59,936 L1 instruction cases recorded on a real CPU pass. The soft float matched the CPU in 850M cases. 7-Zip 25.01 matches native Windows on all 138 matrix steps.
- **Speed** (V8 in Node, against wine64 on the same machine): CoreMark at 0.42 of native and `7z b` at 0.40. In one run, M6's code is 1.44× and 1.20× faster than the code before it.
- **What carries into Phase 2:** the translator, the loader, host calls, threads, the oracle and the five test levels. The Win32 shim is scaffolding that Wine replaces.

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
- **A completion notice's token count is not a run's cost.** It is the context size of the run's last request. A run's real usage is in its transcript (`node scripts/agent-usage.mjs --agent ID`), and cache reads are most of it.
- **The container's disk is a fixed allowance, and `target/` eats it.** In M6 it reached 100 % (53 MB free) with `target/` at 16 GB, a worktree's own `target/` at 4.2 GB and 2.9 GB of scratch files, and a test run was blocked. Cargo keeps every superseded test executable (about 280 MB each) in `target/debug/deps`. At each milestone close, free the space as `docs/dev/commands.md`, "Disk", says: merged worktrees, old scratch directories and superseded test executables. That freed 10 GB in M6.
- **Measure a speedup within one run, against the reference build.** This VM's speed drifts by up to 10 % between sessions (wine64 itself ran 8-9 % slower on 2026-11-06), so ratios from different sessions don't compare. `tests/bench/memchecks.mjs --variants none,reference` interleaves the two builds.

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
    - our own Win64 CoreMark port with a QueryPerformanceCounter timer.
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
  - **Threads:** cooperative, see below. (The first plan stopped at thread creation; 7-Zip needs threads even with `-mmt1`.)
  - **`GetProcAddress`:** stops on a function the shim lacks rather than returning NULL, so no run takes a fallback that native Windows wouldn't. `LoadLibrary*W` and `GetModuleHandleW` follow the same rule for a system DLL outside the shim's table (from the spec review).
  - **Machine profile:** fixed and documented: 1 processor, UTC, an empty registry and environment, redirected stdout.
- **Cooperative guest threads with JSPI, in M5** (your decision, 2026-10-31). 7-Zip always runs a second thread for 7z archives with BCJ, BCJ2 or AES, and for xz, whatever `-mmt` says. `-mtf=off` makes native 7-Zip fail, so no switch avoids it. So guest threads take turns on one JS thread, suspending their wasm stacks at blocking calls through JSPI (`runtime.md`, "Guest threads (M5)").
  - It keeps all 138 matrix steps, and lets common real-world archives extract.
  - Real parallel threads stay in Phase 4.
- **The command matrix** (`tests/sevenzip/README.md`):
  - deterministic inputs and switches;
  - expected outputs committed from a native run, with real Windows gating them in CI and Node gating the translated run;
  - encryption is checked by round trip only, since its salt and IV are random.
- **`expected/` comes from Wine, and Windows confirms it.** Two Wine runs gave identical results (472 files, 5.7 MB). The native matrix on windows-x64 (CI run 41138603006) matched them byte for byte, so no correction was needed.
- **Values the spec left open are fixed in `runtime.md`** ("Values the implementation fixes"): the volume, code pages 1252 and 437, module pseudo-bases, `GetVersion` by manifest, memory, processor features and `FormatMessageW`.
- **`RtlGetVersion` reports the real version** (10.0.26100, a workstation), which `7z b` prints; `GetVersion` keeps the manifest rule.
- **Workflow:**
  - **Research** (your decision, 2026-10-31): a research pass over official and community sources at the start of each milestone, and a digest every two weeks (the "Workflow research digest" routine), logged in `docs/dev/research.md`.
  - **Structure:** fixed rules in `.claude/agents/`, path-scoped rules in `.claude/rules/`, procedures as skills (`/pause`, `milestone-close`, `diverge`), deny rules in `.claude/settings.json`, and `ARCHITECTURE.md` as the map. `CLAUDE.md` keeps only what every session needs.
- **Close-out review in two stages** (`milestone-close`): a code review (7 correctness findings, all fixed in `5709b87`), then a spec-compliance review: 7 findings (6 fixed in `52adb0e` and `46644f7`, the 7th a stale line in this note), and 6 promised behaviours that had no test got one.

## Decisions taken in M6 (planning, 2026-11-04)

The plan you approved on 2026-11-04. Research: `docs/dev/research.md`, "2026-11-04: M6, translator speed".

- **Measurement first (WP1, builder):**
  - `tests/bench/profile.mjs`: `node --cpu-prof` ticks per wasm function, with chunk names carrying the guest's COFF symbol;
  - `Options::counters`: entries (fast and generic), `$dispatch` iterations, local indirect dispatches, calls, probes, unwinds, values synced, guest loads and stores, plus host calls per import;
  - a multi-block loop bench case.

  The numbers decide the emphasis between WP2 and WP3. They also decide whether memcpy, memset and strlen move from JS into wasm.
- **Register and flag liveness, and the call fast path (WP2, deep-worker; new `sync.rs`):**
  - **Values:** the 16 GPRs plus the flag record as one unit.
  - **Stores:** a forward may-dirty analysis. Calls, returns and tail transfers store their dirty set. Cold leaves pass their own dirty set as a constant mask to one shared epilogue, so a stale local is never stored.
  - **Entries:** backward liveness. `entry == start` loads live-in(start); any other entry loads the chunk's union, so every block stays an entry.
  - **After a call:** reload live-after ∩ MOD(callee). MOD is a fixpoint over the module's direct calls and tail transfers; indirect, probed and host targets count as "all".
  - **Optional:** no depth guard for leaf callees.
  - `sync_all` keeps today's behaviour as the reference.
- **WP2 as built** (`crates/x64wasm/src/sync.rs`, `emit.rs`):
  - one transfer function, `sync::op_effect` (uses, kills, may-writes, may-leave), drives the analyses and the emitter, which replays it op by op. So every store the emitter makes is a use the liveness counted;
  - `pop` and the `rep` string registers (RAX/RCX/RSI/RDI per op, read and written in place) are writes and never kills;
  - dead flag records have no effect. Partial and conditional records don't kill the flag record. `FaultIf`, the SSE floating-point ops and x87 ops may leave with the dirty set from before the op;
  - leaves pass their dirty set in an i32 local `mask` (none under `sync_all`). The epilogue stores the values every leave stores unconditionally, and one `if (mask & group)` per group of values that the same leaves store;
  - the fast entry loads live-in of the function's entry block. Any other entry also loads the rest of the chunk's union inside `if entry != own`;
  - `sync_all` produces byte-identical output to `a745ad6` (checked on 7zz.exe and coremark.exe with counters on and off and three depth limits, and on every spike case);
  - `values_*` count locals (the flag record is four), so they compare with WP1's 20 per sync.
- **WP4 as built** (`lift/lower.rs`, `lift/string.rs`, `lift/sse.rs`, `lift/x87.rs`, `ir.rs`, `sync.rs`, `emit.rs`; runtime `link.mjs`, `process.mjs`):
  - one IR op, `MemCheck { addr, len, write, null, code, when }`, inserted by the lifter's `load`/`store`, `vload`/`vstore` (memory), `x87_op` (memory operands) and the string instructions. `op_effect` gives it its address uses and `may_leave`;
  - a check goes before the instruction's first side effect: an access after one (the store of a read-modify-write, after its flag record) is inserted there and merges into the check of the same bytes. So `add [rbx], rcx` at NULL faults as a read, as the CPU's first access would. The lifter asserts that the address was computed before the side effect. `fnstenv` checks its m28 once and stores unchecked;
  - the store check tests the page byte of the first and the last byte (two `load8_u`, or'ed) for every store wider than a byte;
  - `rep movs`/`stos`/`lods` check the first and the last element (`ptr + (RCX - 1) * delta`), only if RCX != 0. `repe`/`repne` `cmps`/`scas` check only the first, since they may stop early (`repne scasb` with RCX = -1 is the strlen idiom). A string fault comes before any iteration;
  - the runtime allocates the page-state table (4 MiB, for the 16 GiB maximum) from the runtime heap only when a module imports `env.page_flags`, and marks the executable sections PAGE_CODE. A store-check hit stops with "unsupported instruction …, or a store into a code or write-watched page (store check)";
  - run-time translations get the image's options since WP4b: `translate-pe` writes them into the plan (`options`), the runtime passes them as the request flags, and `x64wasm-rt` translates with them (`memory-checks.test.mjs`: code reached only through a computed pointer faults on NULL as AOT code does; `pe.test.mjs`: basic.exe's `hidden` is translated at run time with the store check);
  - **not covered:** accesses the runtime makes for host calls are not checked.
- **Loop structuring (WP3, deep-worker):**
  - loop forests by recursive SCC decomposition (Tarjan, `func.rs`), ordered as LLVM's CFGSort does and emitted as CFGStackify does, with one wasm `loop` per loop;
  - a selector local for entries into a loop's interior;
  - `$dispatch` only where something needs it.

  `structured: false` keeps today's behaviour. It is built in a new `emit/structure.rs` (WP3a, in parallel with WP2), then integrated into `emit.rs` (WP3b).
- **WP3b as built** (`emit.rs`, with `emit/structure.rs` from WP3a):
  - with `structured` (the default) the chunk body walks `Plan::items`. The layout's labels go on the emitter's label stack above `$leave`, and their positions are kept in `plan_labels`, so a `Transfer`'s depth becomes a wasm depth however many `if`s and route blocks sit above it;
  - a `Fall` edge inside an `if` (a branch whose two targets are one block) becomes a `br` out of the innermost layout `block`, which is where falling through goes;
  - dispatches (the root one on `entry`, a loop's on the selector) are a `br_table`. Transfers that change the selector go through a stub each (`sel = v; br d`), shared by equal transfers;
  - the selector local holds layout position + 1, so `NONE` is 0: no initialisation, and a loop's check is `local.get sel; if`. It exists only in chunks with a loop that has an interior;
  - `dispatches` also counts an edge that enters a loop's interior through the selector, so it stays comparable across the two layouts (`abi.md`, "Counter block");
  - `structured: false` gives byte-identical modules to `4c02685` (checked on 7zz.exe and coremark.exe, with and without counters);
  - **jump-table targets stay outside the loop forest** (WP3a's open point 1): adding them as edges would not remove `$dispatch`, because the indirect jump still goes through `lookup`, `entry` and the root dispatch. It would also make case blocks interior entries, which costs a selector stub each. `local_indirect` is 5 (CoreMark) and 0 (7z b), so it is not worth doing;
  - every loop with more than one block keeps its selector check (open point 3), since every block is an entry. The contract asks for exactly this.
- **Memory checks (WP4, deep-worker, after WP2):**
  - `null_check` (addr < 0x10000 → `EXIT_FAULT`) and `store_check` (a page byte for code and write-watch → `EXIT_UNSUPPORTED` until invalidation exists), inserted in the lifter's central load and store builders before any side effect;
  - measured last on CoreMark and `7z b` in four variants. The numbers feed `guest-memory.md`'s decision and Phase 2's defaults.
- **Not in M6:** XMM caching in locals.
- **Correctness:**
  - the reference modes join the spike option sweep, with new spike cases (merges before calls, values held across calls in loops, clobbering callees, host calls that unwind, multi-block loops with interior entries);
  - `crates/testkit/tests/sync_diff.rs` runs random CFGs in every mode;
  - L1, L2, CoreMark and the 7-Zip matrix run in both modes.
- **Order:**
  1. the contracts (`translator.md`, `abi.md` :111, `runtime.md`) and an `Options` stub commit;
  2. WP1;
  3. WP2 and WP3a in parallel;
  4. WP3b and WP4;
  5. the close-out (WP5) and PR "[Phase 1] Speed and close-out".

  At most two deep-workers run at once.
- **Estimates, to be measured:** calls drop from 10-20× to about 4-7× native; CoreMark and `7z b` rise from 0.30 and 0.35 to perhaps 0.45-0.6.
- **The research digest routine** now runs on Sonnet 5.5 (your choice). Its effort is set on its page on claude.ai.
- **License (the owner's decision, 2026-11-06):** our own code is all rights reserved (`LICENSE`), and the repository stays public. Nobody may use or build on it without the owner's written permission.
  - **Earlier copies:** copies already made under MIT (clones and forks up to `3a506d7`) keep that license, which can't be revoked.
  - **CI stays free,** because the repository stays public.
  - **What stays open:** dependencies keep their own licenses, and a new one must be permissive or LGPL, never GPL. Our patches to Wine stay LGPL, as Wine's license requires.
  - **The copyright holder** is named as `Skeleton363`, the GitHub account, which was a default of the session. The owner hasn't given a legal name, and can replace it at any time.
  - **For the owner:** copyright in AI-written code is legally unsettled. Before relying on the license commercially, check it with an IP lawyer.
  - **Later (Phase 3, the public demo):** a third-party notices file for the dependencies whose licenses ask for one.

## Decisions taken at the M6 close (2026-11-06)

- **Memory checks** (`guest-memory.md`, "Phase 2 defaults"):
  - identity mapping stays, because the checks cost less than option B would;
  - both checks are on by default in Phase 2;
  - the next cut to measure is no checks on accesses based on RSP.
- **The reference modes reach run-time translation:** the load plan's `options` gained `syncAll` and `structured`, which become the run-time translator's request flags, bits 3 and 4 (`abi.md`, `runtime.md`). `/diverge` uses the reference mode.
  - **In CI:** the spike sweep and `sync_diff` run the reference modes.
  - **By hand at the close:** L1 replay (`testkit replay --sync-all --no-structured`), the L2 programs and the 7-Zip matrix.
- **Not built:**
  - the multi-block-loop bench case (WP1). WP3's effect is measured on CoreMark and `7z b` and by the dispatch counters, and the spike case `loop_branch` tests the shape;
  - no depth guard for leaf callees, which the plan left optional;
  - XMM caching in locals, left out at planning.
- **The counters are defined as built** (`abi.md`, "Counter block"): `mem_loads` and `mem_stores` count integer accesses only, and the flag record counts as 4 values.
- **The effort trial** (`docs/dev/agent-runs.md`): deep-worker stays at `high`. `medium` cost 7 % less by median (3.29M against 3.53M) and needed one rework (WP4b) where `high` needed none. The rule asks for 25 % less with no more rework.
- **Subagents without a set effort run at `high`:** kept. The M6 spec review cost 1.06M, against M5's 2.57M at `max`, and found two real blockers.
- **Review outcomes.**
  - **The spec review:** 11 items; the 2 blockers and the should-fix items are fixed or recorded.
  - **The code review:** 8 candidates; 4 fixed, 2 recorded for Phase 2, 2 not acted on.
  - **Changed semantics:** a read-modify-write that faults on the NULL page now reports FAULT_WRITE, as this machine's CPU reports it (`translator.md`, "Memory checks").
- **The retro** is in `docs/dev/agent-runs.md`, "Retro: M6".

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

| **In-memory file system** (`runtime/src/win32/vfs.mjs`): Win32 and NTFS semantics (names without case, share modes, attributes, times, find masks); `run-pe --dir` mounts a host directory as `C:\work` and writes it back at exit | `vfs.test.mjs`, `kernel32-files.test.mjs` and `run-pe.test.mjs`. |
| **The 7-Zip shim:** handles, kernel32 files and system calls, advapi32, oleaut32, user32, msvcrt additions, ntdll's `RtlGetVersion`, code pages 1252 and 437 | `shim-m5.test.mjs`, `kernel32-system.test.mjs`, `msvcrt.test.mjs` and `ntdll.test.mjs`. |
| **Cooperative guest threads** (`runtime/src/threads.mjs`, JSPI):<br>• `CreateThread` and `_beginthreadex`<br>• events, semaphores and critical sections<br>• waits with timeouts<br>• TLS and errno per thread<br>• a deterministic round-robin<br>• deadlock detection | `threads.test.mjs`, on translated driver code. The matrix's 7z steps with BCJ, BCJ2 or AES and its xz steps need a second thread. |
| **The 7-Zip command matrix** (`tests/sevenzip/`): 138 steps in 40 groups (7z, zip, tar, gz, bz2, xz, the hashes, AES round trips) | Translated in Node: 138/138 identical to `expected/` (output, exit codes and every file written). Native on windows-x64 in CI: identical too. |
| **`7z b`** (`tests/bench/sevenzip.mjs`, `bench.yml`) | Runs to its "Tot:" line translated and under wine64 (measurements below). |
| **The census gate on every program** | `census --check`: 614 of 614 forms, 7zz.exe included. CI gates it. |
| **Register and flag liveness** (M6, `sync.rs`): a boundary stores only dirty values and loads only live ones; after a call, only what the callee may change is reloaded | Values moved per call fell from 80 to about 35 (counters). `sync_diff` runs random CFGs in all four sync and layout combinations, and the spike sweep runs every case in both sync modes. |
| **Structured loops** (M6, `emit/structure.rs`): loop forests, one wasm `loop` per loop, a selector for interior entries | Dispatches fall 50× on CoreMark and 400× on `7z b`. The `irreducible_loop` spike case, and the spike sweep in both layouts. |
| **Memory checks** (M6, off by default):<br>• the NULL page and stores to code pages, checked before the first side effect<br>• one shared exit per chunk<br>• run-time translation with the image's options | `lowering.rs` order tests with the checks on. Node tests for NULL reads and writes, code-page stores and run-time translation. The 7-Zip matrix gives 138/138 with both checks on. |
| **Counters and profiling** (M6): `translate-pe --counters`, `TEMPWIND_TRACE=counts`, `tests/bench/profile.mjs` with COFF symbol names | `pe.test.mjs` and the spike counter tests. |
| **The reference modes, end to end:** `--sync-all` and `--no-structured`, kept in the load plan and passed to the run-time translator | CLI and request tests. In them, the L2 programs and the 7-Zip matrix (138/138) give the native results, and L1 replay passes 59,936 of 59,936 cases. |

### Test summary (local, M6, Node 24.22.0)

- **Rust:** `cargo fmt --all --check`, workspace clippy and the wasm32 clippy of `x64wasm-fp` are clean. `cargo test --workspace`: 221 passed, 0 failed.
- `record --check`: "75 fixture files match a fresh recording".
- `census --check` on 7zz.exe and coremark.exe: 487 of 487 in-scope forms have fixtures.
- L1 replay in the reference modes (`replay --sync-all --no-structured`): 59,936 of 59,936 cases pass.
- `npm run test:node`: 249/249. `npm run test:browser`: 4/4.
- **The translated 7-Zip matrix** gives 138/138 steps identical to `expected/` in three builds:
  - the default build, at the close;
  - `--sync-all --no-structured` and `--null-check --store-check`, both in WP5.

### Test summary (local, M5, Node 24.22.0)

- **Rust** (at `5a45251`; M5's only Rust change, in `census.rs`, came before it):
  - `cargo fmt --all --check`, workspace clippy, and the wasm32 builds of `x64wasm-rt` and `x64wasm-fp`: clean;
  - `cargo test --workspace`: 192 passed, 0 failed.
- `record --check`: "75 fixture files match a fresh recording".
- `census --check`: 614 of 614 forms on every program, 7zz.exe included.
- `npm run test:node`: 230/230.
- `npm run test:browser`: 4/4.
- **The 7-Zip matrix:**
  - translated: 138/138 steps identical to `expected/`;
  - native under wine64: 138/138 identical too, a re-check of `expected/`.

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

**M5** (the session's 4-core Xeon @ 2.10 GHz, Node 24.22.0):
- **`7z b 2 -mmt1 -md18`** (`tests/bench/sevenzip.mjs`, the "Tot:" rating, medians of 3):
  - translated in Node: 1,393 MIPS (runs 1,387-1,394);
  - native under wine64: 4,033 MIPS (runs 3,857-4,852; this VM is noisy);
  - ratio 0.35.
- **The matrix:** 83 s translated, against 354 s under wine64. That compares start-up, not speed: each of the 138 steps starts a new process, and Wine's start-up dominates its runs.
- **7zz.exe's module is unchanged** from M4: M5 didn't change the translator.

**Census** (details in `docs/design/instruction-scope.md`):
- Sizes:
  - functions: 6,289 / 134 / 95 in 7zz / coremark / hello;
  - instructions: 538,849 in all;
  - distinct `Code`s: 443.
- Classes: 97.4 % integer, 1.6 % SSE2, 0.8 % SSE, 74 x87 instructions, no MMX.
- Out of profile: 56 `Code`s, all in 27 CPUID-dispatched functions of 7zz.exe (AES-NI, SHA-NI, AVX2, SSSE3/SSE4.1 paths and the feature probes).
- In scope: 491 forms over 383 `Code`s for the targets; 509 with the M3 L2 programs. After M3, L1 fixtures cover 484 of the 509.
- Imports: 189 distinct functions in 5 DLLs (kernel32 113, msvcrt 54, advapi32 14, oleaut32 6, user32 2).

### WP1 baseline (M6, measurement before changes)

Commands: `node tests/bench/profile.mjs --target coremark|7z` (default options, `node --cpu-prof`, 1 ms samples, self time per function); counters runs are `translate-pe --counters` plus `TEMPWIND_TRACE=counts` (not timed). Profile runs took about 17.0 s (CoreMark) and 39.3 s (`7z b 2 -mmt1 -md18`) of samples. CoreMark calibrates its own length, so its counts vary a little between runs. Function names carry the COFF symbol when the image has one (7zz.exe's hottest functions have none).

**Where the time goes** (self time, % of samples):

| | wasm | JS (shim, runtime) | other (GC, program, idle) |
|---|---|---|---|
| CoreMark | 99.2 % | 0.4 % | 0.4 % |
| 7z b | 98.0 % | 0.5 % | 1.5 % |

JS is not the problem: the host calls are rare (CoreMark 852 in all, 7z 151,286 over about 39 s), so the time is in the translated code itself.

**CoreMark, top 20:** `core_bench_list` 25.5 %; `core_state_transition` 16.3 %; `core_bench_state` 10.6 %; `matrix_mul_matrix_bitextract` 8.5 %; `matrix_test` 7.1 %; `core_list_mergesort` 6.9 %; `matrix_mul_matrix` 6.3 %; `crc16` 3.9 %; `crcu32` 3.5 %; `calc_func` 2.7 %; `cmp_complex` 2.5 %; `flags` (the lazy-flag helper) 2.0 %; `cmp_idx` 1.5 %; `crcu16` 0.9 %; `matrix_mul_vect` 0.7 %; `(program)` 0.2 %; `iterate` 0.1 %; `core_bench_matrix` 0.1 %; `(garbage collector)` 0.1 %; `compileForInternalLoader` 0.1 %.

**7z b, top 20:** `fn_0x140010490` (no symbol) 49.8 %; `fn_0x140013cf0_c1` 18.5 %; `Bt4_MatchFinder_GetMatches` 13.2 %; `CountCpuFreq` 7.2 % (a timing loop that runs for as long as the benchmark asks); `ReadMatchDistances` 3.5 %; `(program)` 1.0 %; `MatchFinder_GetNumAvailableBytes` 0.9 %; `MatchFinder_GetPointerToCurrentPos` 0.8 %; `CrcUpdateT12` 0.7 %; `fn_0x140012980` 0.7 %; `RangeEnc_ShiftLow` 0.7 %; `fn_0x140012a90` 0.5 %; `lookup` 0.5 %; `FillDistancesPrices` 0.3 %; `fn_0x140012a30` 0.3 %; `(garbage collector)` 0.2 %; `(idle)` 0.1 %; `fn_0x140016a90` 0.1 %; `hostcall.mjs` host-call function 0.1 %; `fn_0x140013230` 0.1 %.

**Counters** (whole run):

| Slot | CoreMark | 7z b |
|---|---|---|
| `entries_fast` | 129,580,162 | 143,191,756 |
| `entries_generic` | 45 | 700 |
| `dispatches` | 1,508,290,963 | 1,795,449,283 |
| `local_indirect` | 5 | 0 |
| `calls_direct` | 106,603,578 | 47,654,308 |
| `calls_probed` | 22,692,102 | 95,340,265 |
| `tail_transfers` | 285,328 | 349,152 |
| `unwinds` | 28 | 16 |
| `values_stored` | 5,177,517,380 | 5,723,740,540 |
| `values_loaded` | 5,177,517,380 | 5,723,740,540 |
| `mem_loads` | 4,055,450,111 | 10,154,002,794 |
| `mem_stores` | 1,187,470,980 | 4,330,202,167 |

Derived (calls = direct + probed; rates use the profile run's time as the run time, so they are indicative):

| | CoreMark | 7z b |
|---|---|---|
| `dispatches` per call | 11.7 | 12.6 |
| values moved (stored + loaded) per call | 80.1 | 80.1 |
| values stored per chunk entry | 40.0 | 40.0 |
| probed share of calls | 17.5 % | 66.7 % |
| `dispatches` per second | 89 M | 46 M |
| values moved per second | 609 M | 291 M |
| guest memory loads + stores per second | 308 M | 368 M |

Reading:
- **Every call syncs the whole state:** about 80 values move per call (40 stored, 40 loaded: the caller's store and reload, the callee's load and leave store), whether or not the code touches them.
- **Dispatch is the common branch:** a dispatch every 12 calls or so, that is, loops and branches leave the chunk's fast path often; this is WP3's target (`structured`).
- **7z calls through the lookup table two times in three** (function pointers in the match finder and range coder): that is the `lookup` and `call_indirect` path.
- Counters are not in the timed builds.

### WP2 (M6, register and flag sync)

The same commands as the WP1 baseline: counters from `translate-pe --counters --entries all` with `TEMPWIND_TRACE=counts`; timings from `tests/bench/coremark.mjs` and `tests/bench/sevenzip.mjs` (medians of 3, `wine64` for native). "Before" is `a745ad6` (WP1, translated by its own CLI), run on the same machine in the same session (Xeon @ 2.10 GHz, Node 24.22.0). `values_*` count locals moved: one per GPR and four for the flag record, so a full sync is 20, as in the baseline.

**Counters, after** (CoreMark calibrates its own length: the counted run made 1.70× the baseline's calls; 7z b is deterministic and made exactly the same calls):

| Slot | CoreMark | 7z b |
|---|---|---|
| `entries_fast` | 220,691,490 | 143,191,756 |
| `entries_generic` | 45 | 700 |
| `calls_direct` | 181,559,477 | 47,654,308 |
| `calls_probed` | 38,647,531 | 95,340,265 |
| `values_stored` | 5,581,591,533 | 3,220,705,502 |
| `values_loaded` | 2,060,568,641 | 1,904,564,323 |

| Per call | CoreMark before | CoreMark after | 7z before | 7z after |
|---|---|---|---|---|
| values stored | 40.0 | 25.3 | 40.0 | 22.5 |
| values loaded | 40.0 | 9.4 | 40.0 | 13.3 |
| values moved | 80.1 | 34.7 | 80.1 | 35.8 |

Entries stay fast: generic entries are 45 (CoreMark) and 700 (7z), as before.

**Timings** (medians of 3):

| | before | after | native (`wine64`) | ratio before | ratio after |
|---|---|---|---|---|---|
| CoreMark (iterations/s) | 7,374 | 8,020 | 24,682 | 0.299 | 0.325 |
| `7z b 2 -mmt1 -md18` (MIPS, "Tot:") | 1,657; 1,677; 1,665 | 1,638; 1,697; 1,689 | 5,025 | 0.33 | 0.33-0.34 |

Reading:
- **Values moved per call drop from 80 to about 35.** Loads fall more than stores. Most of what is still stored is the flag record (four locals, dirty after almost every instruction) and callee-saved registers restored with `pop`, which the analysis counts as writes.
- **CoreMark gains 8.8 %; 7z b stays within noise** (three runs of three on each side, alternating; +1 % at most). 7z spends its time in loops (1.8 G dispatches against 143 M calls), and two calls in three are probed, so MOD there is "all". That points at WP3 (loop structuring) for 7z.
- Neither timing was repeated at other guest base addresses.

### WP3 (M6, loop structuring)

The same commands as WP2. "Before" is `structured: false`, whose modules are byte-identical to `4c02685` (WP2). Both sides ran on the same machine in the same session (Xeon @ 2.10 GHz, Node 24.22.0). This machine ran faster than in the WP2 session: native CoreMark was 27,110 against 24,682. CoreMark's counted runs use a fixed 3,000 iterations (`0x0 0x0 0x66 3000`), so both sides make the same calls. The before runs of 7z b and CoreMark used the current `x64wasm_rt.wasm`, which translates only the few blocks found at run time.

**Counters** (every other slot is equal on both sides):

| Slot | CoreMark before | CoreMark after | 7z b before | 7z b after |
|---|---|---|---|---|
| `dispatches` | 63,633,614 | 1,224,000 | 1,795,449,283 | 4,260,355 |
| `local_indirect` | 5 | 5 | 0 | 0 |
| `entries_generic` | 41 | 41 | 700 | 700 |
| calls (direct + probed) | 5,455,886 | 5,455,886 | 142,994,573 | 142,994,573 |
| dispatches per call | 11.7 | 0.22 | 12.6 | 0.030 |

After the change, `dispatches` counts only entries into a loop's interior through the selector. CoreMark's calls are those of its 3,000 iterations; 7z b's calls match WP1.

**Timings** (medians of 3; native under `wine64`):

| | before | after | native | ratio before | ratio after |
|---|---|---|---|---|---|
| CoreMark (iterations/s) | 9,583 | 12,595 | 27,110 | 0.354 | 0.465 |
| `7z b 2 -mmt1 -md18` (MIPS, "Tot:") | 2,075 | 2,388 | 5,477 | 0.379 | 0.436 |

**Translation** (`translate-pe`, `--entries all`):

| | before | after |
|---|---|---|
| 7zz.exe module | 20,615,016 bytes | 21,467,033 bytes (+4.1 %) |
| 7zz.exe with `--counters` | 26,546,143 bytes | 27,082,602 bytes |
| coremark.exe module | 366,247 bytes | 376,369 bytes (+2.8 %) |
| 7zz.exe translate time (in-process, one run) | 1,571 ms | 1,723 ms (+10 %: the plans) |
| V8 compile, 7zz.exe, eager TurboFan (`--no-liftoff --no-wasm-lazy-compilation`, 1 run) | 10.4 s | 6.4 s |
| V8 compile, 7zz.exe, default (lazy: validation) | 15 ms | 17 ms |

Reading:
- **Dispatches drop 50× (CoreMark) and 400× (7z b).** What is left is entries into loop middles: irreducible edges and branches into a loop's body from outside it.
- **CoreMark gains 31 % and 7z b 15 %** in this session's runs. The ratios rise from 0.35 to 0.47 and from 0.38 to 0.44.
- **Optimized compiles are faster:** with no function-wide loop, TurboFan no longer has to keep every local live across the whole chunk. The eager compile of 7zz.exe falls from 10.4 s to 6.4 s. This is one run each; a second run could not be done because the disk filled up.
- Modules grow by 3-4 %, from the route stubs and the `block` nesting.
- `tests/bench/translator/images.mjs` fails on images with `fp` imports: its child process loads no floating-point helper module. So the compile times above come from a direct `new WebAssembly.Module`.
- Neither timing was repeated at other guest base addresses.

### WP4 (M6, memory checks; preliminary)

One run of each, on `74f0308` (WP2 code plus WP4, before WP3b), `tests/bench/coremark.mjs --runs 1` and `tests/bench/sevenzip.mjs --runs 1`, `--skip-native`, Xeon @ 2.10 GHz, Node 24.22.0. Every run validated. The final numbers are WP5's (5 interleaved runs each, on the final code).

| Variant | CoreMark (it/s) | 7z b (MIPS) | coremark.wasm (bytes) | 7zz.wasm (bytes) |
|---|---|---|---|---|
| none | 9,621 | 2,152 | 366,247 | 20,615,016 |
| `null_check` | 8,200 | 1,766 | 552,072 | 33,384,535 |
| `store_check` | 7,809 | 1,643 | 457,531 | 26,803,555 |
| both | 7,430 | 1,536 | 637,828 | 39,212,596 |

Reading (single runs, so only the size of the effect): both checks cost about 23 % on CoreMark and 29 % on 7z b, and the modules grow by 74 % and 90 %. Each check site inlines its own cold exit (the state stores of a fault or an exit), which is most of the growth; a shared exit per chunk would shrink it. With both checks on, the spike sweep, basic.exe, the four L2 programs, CoreMark, 7z b and the translated 7-Zip matrix give the same results.

### WP4b (M6, cheaper check sites; preliminary)

Each check site used to inline its own cold exit. Now it passes `offset << 2 | kind`, the address and the dirty mask to one `$fault` block per chunk (`translator.md`, "Memory checks", "Emission"). Null checks on constant addresses are decided at translation time. The address was already shared with the access (one IR value), so that needed no change. Before = `3a3b01d`, after = this change, both `--entries all`; one run of each, as in WP4.

| Variant | coremark.wasm before → after | 7zz.wasm before → after | CoreMark it/s before / after | 7z b MIPS before / after |
|---|---|---|---|---|
| none | 376,369 → 376,369 | 21,467,033 → 21,467,033 | 12,956 / 12,053 | 2,401 / 2,477 |
| `null_check` | 562,125 → 478,285 (+49 % → +27 %) | 34,222,929 → 28,660,274 (+59 % → +34 %) | 10,114 / 10,171 | 2,026 / 2,036 |
| `store_check` | 467,610 → 452,039 | 27,649,187 → 26,581,515 | 9,928 / 9,533 | 1,877 / 1,925 |
| both | 647,839 → 541,952 (+72 % → +44 %) | 40,044,592 → 32,992,804 (+87 % → +54 %) | 8,989 / 9,240 | 1,705 / 1,711 |

Reading: with both checks, the growth over unchecked code falls by about 40 %. Nearly all of that comes from the shared exit: deciding constant addresses at translation time saves only another ~0.5 % (`null_check` without it: 485,854 and 28,816,622 bytes). Speed is unchanged within the noise of single runs: the unchanged "none" module itself moves 7 % between runs. What is left per site is the test, the address `local.set`, two `i32.const`/`local.set` pairs and the `br`; the store check's two page-byte loads stay as they were. WP5 makes the final measurement.

### M6 final (WP5)

Machine: Intel Xeon @ 2.10 GHz, 4 vCPUs (cloud VM), Node 24.22.0 (V8 13.6.233.17-node.57). Code: the default build of `ff37120` (WP1-WP4b plus the WP5 tooling). Native = the same `.exe` under wine64 on the same machine. Every benchmark run validated. The machine ran nothing else while timing (the load average sat at 1.0-1.8: one busy core of ours, plus the compile threads in the spike runs).

**Against wine64** (`tests/bench/coremark.mjs`, `tests/bench/sevenzip.mjs`, medians of 3):

| | translated | wine64 | ratio | M5 ratio |
|---|---|---|---|---|
| CoreMark (iterations/s) | 10,268 | 24,652 | 0.417 | 0.30 |
| `7z b 2 -mmt1 -md18` (MIPS, "Tot:") | 2,017 | 5,019 | 0.402 | 0.35 |

The M5 ratios come from M5's own sessions, and this machine's speed drifts between sessions (wine64 ran 8-9 % slower here than in the WP3 session). So M6's gain is measured within one run, below.

**M6's gain, in one run** (`node tests/bench/memchecks.mjs --variants none,reference --rounds 5`, interleaved, the order rotated; median, min-max in brackets). The reference build (`--sync-all --no-structured`) is the code generation from before M6: WP2 and WP3b checked that each reference mode gives byte-identical modules to the code before them.

| | default (M6) | reference (before M6) | speedup |
|---|---|---|---|
| CoreMark (iterations/s) | 10,257 (10,045-10,435) | 7,137 (6,893-7,271) | 1.44× |
| `7z b` (MIPS) | 1,983 (1,941-2,042) | 1,652 (1,620-1,672) | 1.20× |
| 7zz.wasm (bytes) | 21,467,033 | 27,942,383 | 0.77× |

Against the same day's wine64, the reference runs at 0.29 (CoreMark) and 0.33 (`7z b`), matching M5's 0.30 and 0.35.

Spread: the CoreMark runs inside the memory-check matrix (variant "none", 5 runs) span 10,355-10,655 it/s (2.9 %); the 7z b runs span 2,014-2,077 MIPS (3.1 %). The WP2-WP4b sections show up to 7 % between sessions on the same build.

**Memory-check matrix** (`node tests/bench/memchecks.mjs --rounds 5`: four variants interleaved, order rotated per round, `--skip-native`, one run per round; median, min-max in brackets):

| Variant | CoreMark (it/s) | vs none | 7z b (MIPS) | vs none | coremark.wasm (bytes) | 7zz.wasm (bytes) |
|---|---|---|---|---|---|---|
| none | 10,448 (10,355-10,655) | 1.00 | 2,028 (2,014-2,077) | 1.00 | 376,369 | 21,467,033 |
| `--null-check` | 8,692 (8,592-8,976) | 0.83 | 1,654 (1,606-1,665) | 0.82 | 478,285 (+27 %) | 28,660,274 (+34 %) |
| `--store-check` | 8,484 (8,405-8,547) | 0.81 | 1,565 (1,556-1,576) | 0.77 | 452,039 (+20 %) | 26,581,515 (+24 %) |
| both | 7,736 (7,673-7,853) | 0.74 | 1,363 (1,345-1,373) | 0.67 | 541,952 (+44 %) | 32,992,804 (+54 %) |

The module sizes equal WP4b's.

**V8 compile time** (`tests/bench/translator/images.mjs --runs 3`, synchronous `new WebAssembly.Module`, medians of 3; "eager TurboFan" = `--no-liftoff --no-wasm-lazy-compilation`; the link phase of `Process.load` stays at 20-30 ms for 7zz.exe and 1-4 ms for coremark.exe in every build):

| Build | Image | wasm bytes | default (lazy) ms | eager TurboFan ms |
|---|---|---|---|---|
| reference (`--sync-all --no-structured`) | 7zz.exe | 27,942,383 | 34.9 | 10,620 |
| | coremark.exe | 464,236 | 0.9 | 144 |
| default | 7zz.exe | 21,467,033 | 37.2 | 7,053 |
| | coremark.exe | 376,369 | 0.8 | 115 |
| default + both checks | 7zz.exe | 32,992,804 | 68.5 | 13,993 |
| | coremark.exe | 541,952 | 1.0 | 201 |

`images.mjs` now supplies `x64wasm_fp.wasm` to the child and takes `--translate-flags` and `--out` for the builds above. The fp module is built with a synchronous `new WebAssembly.Instance`: `loadFp`'s async compile never settled in a child run with `--no-wasm-lazy-compilation`.

**Correctness**: the four L2 programs with `--sync-all --no-structured` (`target/native/<name>-reference/`) give the native output and exit codes. The translated 7-Zip matrix gives 138/138 identical steps with `--sync-all --no-structured` and with `--null-check --store-check`.

**Spike microbenchmarks** (`spike --bench`, then `tests/bench/translator/bench.mjs`; ratio to native, best run; native `sum_loop` 32.6 ms, `fib_rec` 5.6 ms, `byte_calls` 7.6 ms, `dot_product` 14.4 ms):

| Case | V8 default (M5 -> now) | V8 TurboFan (M5 -> now) | V8 Liftoff only (M5 -> now) |
|---|---|---|---|
| `sum_loop`, n = 10^8 | 1.23-1.28x -> 0.95x | 1.23-1.24x -> 1.02x | 7.9x -> 4.06x |
| `fib_rec(30)` | 12.4-12.6x -> 4.37x | 10.3-10.7x -> 4.25x | 31x -> 15.5x |
| `byte_calls`, 1 MiB | 3.3-3.7x -> 1.48x | 2.5-2.6x -> 1.56x | 9.3-9.8x -> 6.56x |
| `dot_product` | (7x in Known issues) -> 6.35x | 6.23x | 8.46x |

The M5 wasmtime column was not re-measured. `bench.mjs` also has an eager configuration, not tabulated here.

**Profile** (`node tests/bench/profile.mjs --target coremark|7z`, top 10 by self time):

| CoreMark (11,307 samples) | % | 7z b (29,632 samples) | % |
|---|---|---|---|
| `core_bench_list` | 18.6 | `fn_0x140010490` (no symbol) | 53.7 |
| `core_state_transition` | 17.3 | `fn_0x140013cf0_c1` | 16.3 |
| `matrix_mul_matrix_bitextract` | 11.5 | `Bt4_MatchFinder_GetMatches` | 11.4 |
| `core_bench_state` | 9.6 | `CountCpuFreq` | 8.7 |
| `core_list_mergesort` | 7.6 | `ReadMatchDistances` | 2.7 |
| `matrix_mul_matrix` | 7.3 | `(program)` | 0.9 |
| `matrix_test` | 5.6 | `MatchFinder_GetNumAvailableBytes` | 0.9 |
| `crc16` | 5.0 | `CrcUpdateT12` | 0.9 |
| `crcu32` | 4.9 | `fn_0x140012980` | 0.7 |
| `flags` (lazy-flag helper) | 2.7 | `fn_0x140012a90` | 0.6 |
| wasm / JS / other | 99.0 / 0.5 / 0.5 | wasm / JS / other | 98.1 / 0.6 / 1.3 |

Caveats: single sessions; the reference-mode and check builds were timed once per variant (compile) or five interleaved times (matrix); nothing was repeated at other guest base addresses. At the M6 close the plan's `options` gained `syncAll` and `structured` (request flags bits 3 and 4), so code translated at run time keeps the reference modes too.

## Known issues and risks

- **Calls are still expensive:** a call moves about 35 values instead of 80 since WP2, and `fib_rec` runs at 4.4× native in V8 (12.5× in M5). The flag record and the registers restored with `pop` make up most of the rest.
- **Guest threads take turns; they don't run in parallel** (Phase 4). A thread switches only at a blocking call, so a thread that spins on memory without one never lets the others run. 7-Zip doesn't do that.
- **The research digest has not reached the repository yet.** Its routine session starts without a checkout, and both runs so far (2026-11-01 and a test on 2026-11-04) stopped there. The owner selects Skeleton363/Tempwind for the "Workflow research digest" routine on claude.ai; until then, research happens at each milestone's start.
- **Floating point is slow:** about 7× native in a dot product (XMM in the state block, an MXCSR load and operand tests per operation). x87 code is large (400-600 wasm bytes per instruction). M6 left XMM caching out by decision; `dot_product` runs at 6.35× native.
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
  - TLS callbacks don't get DLL_PROCESS_DETACH at exit;
  - msvcrt's `fopen` doesn't open existing files, and `MapViewOfFile` isn't implemented;
  - deleting or renaming a file that still has open handles isn't supported;
  - `LoadLibrary*W` loads only the DLLs in the shim's table, and no guest DLLs;
  - `FormatMessageW` has system messages only, without inserts.

  Each of these stops loudly.
- **7zz.exe's module:** 21.5 MB since M6. The reference build is 27.9 MB, and the build with both checks 33.0 MB. Eager TurboFan compiles the default build in 7.1 s.
- **The memory checks cost 26-33 % with both on,** and they are on by default in Phase 2. The cut for accesses based on RSP is the next measurement (`guest-memory.md`).
- **Two gaps in the store check,** found by the M6 code review. Both are harmless in Phase 1, where the checks are off by default and only the main image holds code, and both must close before Phase 2 turns the checks on:
  - a `rep movs` or `rep stos` range is tested at its first and last byte only, so a range over three or more pages writes the pages between unchecked;
  - the runtime marks PAGE_CODE only on the main image's executable sections, once, so code translated from anywhere else (a second image, code written at run time) gets no mark.
- **Firefox is untested.** SpiderMonkey pays a bounds check on every memory64 access, which V8 doesn't. Phase 3's browser targets need a measurement there.
- **5 of 138 jump tables in 7zz.exe aren't recovered:** 4 load the table base earlier in the function, and 1 sits in a handler nothing reaches. Run-time translation covers them.
- **The run-time translator compiles on first use** (about 24 ms). Compiling it while the image loads would hide that.
- **Runtime limits in Phase 1:**
  - images only at their preferred base (no relocation);
  - no SEH or exceptions;
  - KUSER_SHARED_DATA is zeroed;
  - the shim covers what the target programs import and call, nothing more.
- **wasmtime loop timings depend on code placement** on this VM. Benchmark baselines in M4/M5 should come from V8 and same-machine native runs.
- **apt installs in CI were flaky:** the Azure Ubuntu mirror trickled at about 40 KB/s, and 3 targets runs hung or timed out. `scripts/apt-install.sh` now limits each attempt and retries without that mirror; a mock test covered both the stall and the failure path.
- **One release publish failed with a transient 403.** The `targets` run of `6d4536c` was refused by the GitHub releases API with "Resource not accessible by integration", although its token had `contents: write`. `hello-7613c87` published 38 s later, and the same step published `targets-7613c87` on the next commit. If it recurs, re-run the job once, or retry the publish step.

## Next step: Phase 2 (Wine underneath)

`docs/plan.md`: Wine's PE DLLs are built in CI from the dl.winehq.org tarball, 64-bit console programs run in the browser, and kernel32/ntdll test pass rates are tracked.

1. **A fresh session.** The rule is that a session covers a phase, and trial 3's hand-off gets its first test. It starts from `docs/plan.md`, this note and `ARCHITECTURE.md`.
2. **A research pass** (`docs/dev/research.md`): Wine 11's PE/Unix split and syscall dispatch, and Emscripten for the Unix side (threads, signals, mmap).
3. **Phase 2's milestones, contracts first.** The M5 shim is scaffolding that Wine replaces; the loader, host calls, threads, translator and tests carry over.
4. **Settled already:** the memory checks are on by default (`guest-memory.md`), and dependencies stay permissive or LGPL (`LICENSE`, `CLAUDE.md`).
5. **Before the checks go on:** close the two store-check gaps under "Known issues". Every page of a `rep` range must be checked, and every page the runtime translates code from must be marked.
