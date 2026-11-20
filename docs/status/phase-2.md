# Phase 2 status: Wine underneath

**Phase 2 is planned (2026-11-06). M1, "Wine in CI, and the boundary spike", is done (2026-11-13, merged as skeleton363/tempwind#10 and #11). M2, "Wine's Unix side boots", is done (2026-11-14, merged as skeleton363/tempwind#12 and #13). M3, "Boot: hello.exe on Wine", is planned (2026-11-14) and in progress.**
- M1: WP1 (the Wine CI build), WP2 (the census, fixtures and translation of Wine's DLL closure) and WP3 (the boundary spike) are done.
- M2: WP0 to WP5 are merged and the close is under "M2" below.
- M3: the decisions and the work packages are under "M3" below.

The milestones are in `docs/plan.md`, "Phase 2 milestones", and the research pass is in `docs/dev/research.md`, "2026-11-06: Phase 2, Wine underneath". A new session starts only with a new milestone (the owner, 2026-11-07).

- **Session 1 (2026-11-06 and 07):**
  - planned Phase 2: three Explore runs, then a Plan-agent review of the draft, whose six blockers are folded in below;
  - the owner decided on one session per milestone and on a basic raise path in Phase 2;
  - M1: the spike, then the Wine CI build. Paused overnight, resumed in the same session.
- **The same session, resumed 2026-11-13:** WP2's review follow-up, then the M1 close.
- **Session 2 (from 2026-11-13), M2 only:** the research pass and the plan (three Explore runs and a Plan-agent review), a compaction at the plan's approval (trial 6), then the contracts and the work packages.
- **Session 3 (from 2026-11-14), M3 only:** the research pass (three Explore runs read the sources, trial 6's lead) and the plan (a Plan-agent review), then a compaction before the contracts.

## Decisions taken when planning (2026-11-06)

### Scope of the done-when
- **"64-bit console programs run in the browser":**
  - the programs are hello.exe, l2_int, l2_crt, l2_float, CoreMark (CRCs validated) and the 7-Zip matrix (138 steps);
  - they run on Wine's DLLs with both memory checks on, in Node and in headless Chromium (a Worker);
  - output and exit codes match native (`tests/pe`, `tests/native`, `tests/sevenzip/expected/`).
- **"kernel32/ntdll test pass rates are tracked":**
  - every unit of `kernel32_test.exe` (33 units) and `ntdll_test.exe` (26) runs translated in CI;
  - each unit gets a JSON result: executed, failures, todo, skipped, exit, time;
  - the baselines are windows-2025, the reference of truth, and native Wine 11 on Linux;
  - history goes to an orphan `conformance-data` branch, and PRs warn on regressions, as `bench-data` does.
- **"Wine's PE DLLs are built in CI from the dl.winehq.org tarball":**
  - the tarball is the unmodified, pinned wine-11.0;
  - it is built with Ubuntu's MinGW GCC 13, the targets' toolchain;
  - the result is published as `wine-<sha7>` release assets.

### Sessions: one per milestone (the owner's decision)
- **Each milestone starts in a fresh session.** It reads `docs/plan.md`, this note and `ARCHITECTURE.md`, and ends with `/milestone-close` (or `/pause`) and a hand-off. `CLAUDE.md` says so now.
- **Why:** Phase 1 ran in one session: 818M cache-read tokens over M1-M5, $415 by the M6 close. Every step re-read the whole context (`docs/dev/agent-runs.md`, "The largest lever").
- **Measure:** the `main` lines of M2's fresh session (trial 3).

### The boundary between translated code and the Unix side (`abi.md` v5)
- **The JSPI stack discipline.**
  - **The Unix side owns the run loop.** `signal_wasm64.c` imports the translator's `run` directly (wasm to wasm) and handles its exits.
  - `EXIT_LEFT_CODE` calls a translate import, which is `Suspending` when compiling is async.
  - Nested guest calls (`KeUserModeCallback`, later APCs) reuse the same loop.
  - JS enters the Unix side only through `WebAssembly.promising` exports: boot, and one per green thread.
  - No JS frame may sit between a `promising` entry and a `Suspending` import, because V8 cannot suspend JS frames.
  - Phase 1's JS run loop (`process.mjs`) doesn't carry over to Wine processes.
- **Syscall and unix-call entry.**
  - The PE side is unmodified. Syscalls take Wine's own thunk path, `testb $1,0x7ffe0308; jne; call *0x7ffe1000`. The Unix side sets KUSER `SystemCall` = 1.
  - Unix calls go through `__wine_unix_call_dispatcher`.
  - Both pointers hold guest addresses that the lookup table maps to **raw Unix-side wasm exports** (from `instance.exports`, not Emscripten's BigInt JS wrappers) placed in `env.table`. They use abi.md's host-function contract and the guest-function type `(state i64, entry i32, ret i64) -> i32`.
  - A raw `syscall` instruction either exits or stops loudly; abi.md v5 decides which.
- **Green threads.**
  - A switch saves and restores `__stack_pointer` (plus the stack base and end), `errno` (one global without pthreads) and the current state-block pointer.
  - The green library replaces Emscripten's no-pthread stubs: mutexes, condition variables, `pthread_once` and keys. It asserts if a thread suspends while holding a mutex.
- **Evidence first:** M1's boundary spike proves the discipline in Node and Chromium before M2's contracts. If it fails, the architecture is revised then.

### The Unix side (`docs/design/wine-unix.md`, from M2)
- **The pieces:** Wine 11's ntdll Unix part and wineserver from M2, and win32u from M4.
- **The build:**
  - emcc, wasm64, importing the runtime's shared memory (= guest memory), `GLOBAL_BASE=0x80000000`;
  - no Emscripten pthreads; wasm exceptions for `setjmp`/`longjmp`;
  - unixlibs linked statically behind a table instead of `dlopen`;
  - our own small build over the tarball (Phase 0's recommendation), with a patch series and new files in `native/wine/` (LGPL, public).
- **The new machine layer:**
  - `signal_wasm64.c`:
    - the run loop;
    - syscalls through generated typed wrappers, since `call_indirect` is signature-strict;
    - unix calls, thread start and `NtContinue`;
    - contexts to and from the state block, with EFlags materialized from the lazy record and the FXSAVE area;
  - a `virtual.c` backend over the page-state table with no mmap. Guest-pointer validation replaces `__TRY` (M2), since a bad pointer does not fault;
  - green pthreads;
  - the in-process server transport.
- **wineserver runs in-process, linked into the same module.**
  - Requests are calls.
  - An fd passed between "server" and "client" is `dup()`ed, since each side closes its own copy.
  - Server timers run from the scheduler's idle loop.
  - Patch list:
    - an `nm` census of symbol collisions (for example `user_shared_data` and `main`), resolved by a generated rename header;
    - the server's page size fixed at 0x1000 (`server/mapping.c` L273, where `sysconf` says 64 KiB);
    - the ASLR DLL range moved into [0x1_0000_0000, 0x4_0000_0000) (L275);
    - an exit path that replaces the SIGQUIT thread kill (`server/thread.c` ~L1627).
  - **Why in-process:** with one Windows process per run, a SharedArrayBuffer channel, a shared FS and shm replication are pure cost now, and Phase 4 has to build their cross-worker forms anyway. Revisit with child processes in Phase 4.
- **One Worker per Windows process.** Guest threads stay cooperative, as in Phase 1; parallel threads are Phase 4.
- **No child processes.**
  - `NtCreateUserProcess` fails with an NT error status.
  - A patch skips the first process's `wineboot --init`.
  - The prefix comes from native Wine 11's `wineboot --init` in CI.
  - wineboot's per-boot volatile keys come from a generated `.reg` overlay loaded with the prefix: the computer name, the hardware description from our CPUID profile, and the volatile environment.
- **Startup details:**
  - fds 0-2 are custom non-tty byte devices. Emscripten's defaults are line-buffered ttys, which would start conhost and break `7z -so`.
  - `dladdr`-based path discovery is replaced by constant paths: `/wine/lib/wine/x86_64-windows`, `/wine/share/wine` (NLS, `wine.inf`), and `$WINEPREFIX`. M2's contract sets it to the path the prefix was made at, `/home/runner/wine-prefix`, since the `.reg` files hold it (`wine-unix.md`, section 8).
  - `sbrk` is capped below 0x1_0000_0000.
  - `memory.grow` has one owner, the virtual backend. It grows through an import that refreshes every view, Emscripten's and the runtime's.
- **Time is real time,** as in Wine; Phase 1's logical timeouts (`runtime.md`, "Timeouts") don't carry over.
  - KUSER time is updated at every syscall entry and thread switch.
  - A ticker Worker that writes it into the shared memory comes in M4, if a measured case such as a `GetTickCount` spin needs it.
- **Where it builds:**
  - while being ported, the Unix side builds in the session through `scripts/emcc.sh`, for compile feedback;
  - CI builds and publishes it as `wine-unix-<sha7>`;
  - the full Wine build (about 700 PE modules) is CI only.

### Windows exceptions: a basic raise path in Phase 2, M5 (the owner's decision)
- A fault exit (NULL or access from the checks, #DE, #UD, `int3`) or `NtRaiseException` is dispatched by Wine's own PE code (`KiUserExceptionDispatcher`), from the translator's precise context.
- Handlers resume through `NtContinue` and unwinding. Unhandled exceptions end the process as on Wine.
- Phase 4 keeps the rest: C++ exceptions across host frames, guard pages, stack overflow, vectored handlers verified, and fault kinds recorded natively.
- Before M5, faults stop loudly as in Phase 1. `docs/plan.md`'s Phase 4 row carries a note.

### Code, images and policies
- **AOT modules for the closure DLLs:**
  - produced by `translate-pe` from the published PE files, in CI jobs and sessions. They are not release assets, since they change with the translator;
  - linked when the Unix side maps an image at the base its module was translated for;
  - the catalog keys are path, a precomputed sha256 (WebCrypto is async-only) and base;
  - an image mapped elsewhere falls back to run-time translation, and the fallback is counted. The deterministic base it was mapped at can be AOT-translated with `translate-pe --base`.
- **Invalidation:** "first translation wins" gains an explicit `invalidate(range)`.
  - It runs on unmap, and whenever the Unix side writes guest pages marked PAGE_CODE: file reads, `NtWriteVirtualMemory`, image mapping.
  - Translated stores to code pages stay `EXIT_UNSUPPORTED` until the JIT work of Phase 4.
- **Unsupported paths in our port** return a real NT error status, never success, and print `tempwind: unsupported: <what>`.
  - Program tests fail on such a line.
  - Conformance runs count them.
- **The JS shim is frozen:** no new APIs. It is deleted at M4's close, once Wine runs every program the shim ran.
- **Memory checks:** both are on by default from M3, after the two store-check gaps close (`phase-1.md`, "Known issues"):
  - every page of a `rep` range is checked;
  - PAGE_CODE is set on every translated region, AOT or run-time.

### Milestones and agents
- **The table:** `docs/plan.md`, "Phase 2 milestones". The done-when details:
  - **M1, PE gate:**
    - `wine.yml` publishes `wine-<sha7>`: the PE modules, the test exes, the prefix and a native Wine 11 build;
    - the Windows and native-Wine baseline JSON;
    - the closure listed with each DLL's preferred base, the overlaps, the highest base and the boot memory size it implies;
    - `census --check` passes on the closure (new forms have fixtures, replay 100 %);
    - every closure DLL translates and compiles in V8, sizes and times measured.
  - **M1, boundary gate,** in Node and a Chromium Worker. An Emscripten wasm64 module (imported shared memory64, `GLOBAL_BASE` 2 GiB, wasm EH):
    - owns the loop and calls a translated module's `run`;
    - is called from translated code through a raw export in `env.table`;
    - suspends through a `Suspending` import while a second green thread runs with its own save set, then resumes;
    - makes a nested guest call;
    - grows memory, and JS and translated code see the growth.
  - **M2:** the Unix side builds in CI and locally with no undefined symbols beyond the documented imports. It boots to `LdrInitializeThunk` in Node without running guest code:
    - ntdll.dll and hello.exe mapped at their preferred bases, and apisetschema's API set mapped (`PEB->ApiSetMap`), since apisetschema is data;
    - registry and overlay loaded;
    - NLS loaded and KUSER filled;
    - the main thread's state block at `LdrInitializeThunk`.

    A C harness covers the virtual backend (reserve, commit, protect, free, query, the page-state table, pointer validation) and server waits with timeouts on one thread.
  - **M3 to M6:** as in the table. M4 names its thread tests: Phase 1's thread tests ported to Wine, and the matrix steps that need a second thread.
- **Agents:** at most two deep-workers at once, three subagents in all. Contracts are committed before agents start.
- **M1's work packages:**
  - the contracts (coordinator): `docs/design/wine-build.md`, the closure in `instruction-scope.md`, and the boundary section of `abi.md` v5;
  - WP1 (builder): `wine.yml`, the baselines, the `+loaddll` closure traces, `tests/targets/wine.lock`, and the native build packaged for `/diverge`;
  - WP3 (deep-worker): the boundary spike, in parallel with WP1;
  - WP2 (deep-worker), after WP1's first release: census, fixtures, lowering, translation of the closure and the base table.

## M1: Wine in CI, and the boundary spike (done 2026-11-13)

### Decisions taken in M1
- **Sessions reach CI results only through releases.**
  - Workflow artifacts are stored on Azure blob hosts that the session's network allowlist doesn't include (a 403 from the proxy, 2026-11-07).
  - Opening `*.blob.core.windows.net` would admit anyone's Azure storage, so we don't. CI publishes whatever sessions need as release assets.
- **`wine.yml` publishes early.**
  - `publish-build` creates `wine-<sha7>` as soon as the build passes.
  - `publish-results` adds the baselines, the traces and `closure.json` as their jobs finish, and still runs when one of them failed. A missing asset is a warning.
  - `closure` runs whenever the build passed, with static edges only if there are no traces (`wine-build.md`, "Release").
  - Why: run 42306869519 built Wine, but its late `baseline-wine` failure left nothing published.
- **`scripts/release-publish.sh`:** creates the release if needed, keeps assets already there, and retries every API call. The releases API refused a run's token with a transient 403, "Resource not accessible by integration", for the second time (the publish of run 42353561868). The same call succeeded minutes later.
- **The conformance runner** lists units from stdout only, and `parseList` takes only the list's indented names. Under Wine, the `+loaddll` lines on stderr had become 176 bogus "units" per exe. A test reproduces the mixed output.

### WP1, the Wine CI build: done
- **The CI history:**
  - run 42306869519: the build passed, then `baseline-wine` failed on `basic.exe`'s designed exit code 42 under `bash -e`;
  - run 42353561868: everything passed except a transient publish 403;
  - run 42355029562 (`wine-bf443f0`): the new publishing, but bogus units in the Wine baseline;
  - run 42356976933 (`wine-b4ecc3c`): correct baselines, then the closure summary failed on a `| head` broken pipe under `pipefail`;
  - run 42357971957 (`wine-ccff087`): every job green, every asset published;
  - run 43385076350 (`wine-4c6ec0d`, started by the closure's path fix): every job green in 14 min, with ccache warm.
- **`tests/targets/wine.lock`** names `wine-4c6ec0d` (2026-11-13).
  - Its `closure.json` lists the same 83 modules as `wine-ccff087`'s and `wine-bf443f0`'s, with the same bases, sizes and edges.
  - The test exes' paths are now tree-relative (`tests/kernel32_test.exe`).
  - `census --closure ... --check` passes on it locally, with the same totals.

### WP2, census and fixtures: done
- **The census** of the closure (`wine-bf443f0`, the same modules as `wine-ccff087`): 83 modules, 3,779,886 instructions, 663 Codes, 797 in-scope forms, all with fixtures. Replay passes 60,752 of 60,752 cases, and `record --check` reproduces all 80 files.
- **16 new forms,** recorded and lowered (`translator.md`, "Wine closure forms"):
  - executed code: `pushfq`; `mov r/m16, Sreg`, register and memory; `fxsave` (RtlCaptureContext); `cmpxchg16b` (the SList functions); `fprem` (oleaut32); and in `ntdll_test.exe`, `popfq` and `mov rm16, Sreg`;
  - data the census decodes (RTTI tables inside `__asm_dummy_*` `.pdata` ranges): `lahf`, `sahf`, `enter`, `fadd m64`, `fiadd m32`, `fsub m32`, `fsubr m64`, `fdivr m32`, `fcomp st(i)`.
- **Exclusions:**
  - `int3` is a debug trap; `int imm8` is privileged or system;
  - `movsxd` without REX.W is a new class, "vendor-dependent": data only, and its generator has excluded it since M3;
  - out of profile, by function: `test_extended_context*`, `func_time`, `read_tsc_frequency` and `write_minidump`.
- **The recorder** starts a fresh tracee after each `popf` case: Linux leaves TF set after a traced `popf` (`arch/x86/kernel/step.c`).
- **Translation:** all 82 modules with code translate; `apisetschema.dll` is data only. The only unsupported sites are excluded or out-of-profile forms: `syscall` 1,798, `int3` 12, `cli`/`sti`/`in` in RTTI data, `rdtsc(p)`, `int 29h`, and one `vmovups`.
- **CI:** ci.yml's census step covers the closure from the `wine.lock` release (`census --closure FILE --tree DIR`); dispatch run 42360530592 at `a1beaa6` is green.
- **Decisions at review:**
  - segment selectors are the Linux values: CS 0x33, SS 0x2B, DS/ES/FS/GS 0. That is what native Wine's PE code reads and what the oracle records; Windows has DS = ES = GS = SS = 0x2B and FS = 0x53;
  - the `movsxd` exclusion stays;
  - **popfq is changed:** NT and ID stay in RFLAGS, and an image with TF or AC set exits `EXIT_UNSUPPORTED` instead of being silently dropped. Single-step and alignment checks aren't modelled. Done in the follow-up, below;
  - `closure.json`'s root paths become tree-relative, as the schema says. Done in the follow-up.

### WP2's review follow-up: done (2026-11-13)
- **Who:** a fresh deep-worker, not the WP2 agent the pause had stopped; resuming that one would have reloaded its 2.6 MB transcript with a cold cache.
- **popfq** (`d9d644c` contract, `b53770f` code):
  - NT and ID now load from the image into RFLAGS;
  - an image with TF or AC set exits `EXIT_UNSUPPORTED` before any change, through `FaultIf` with the new kind `ir::UNSUPPORTED` (0; the fault kinds start at 1);
  - `translator.md` ("Faults", "RFLAGS images") and `fixtures.md` say so.
- **The recorder follows NT and ID through popfq:** the CPU keeps both and ptrace reads them back. So `pushf.json`'s popfq cases now set and compare them, re-recorded with `testkit record` (`flags_mask` adds NT and ID).
- **A pushfq case can't start with ID set:** `PTRACE_SETREGS` drops ID from the input RFLAGS, while NT survives.
  - `crates/testkit/tests/rflags_paths.rs` covers instead:
    - the popfq-then-pushfq round trip of NT and ID;
    - the TF/AC exit, with RIP, RSP, RFLAGS and memory unchanged.
  - Both tests fail on the old lowering.
- **Closure paths** (`e836020` contract, `4c6ec0d` code):
  - a root inside `--tree` gets a tree-relative `path`, with a unit test;
  - a root outside keeps the path it was given, and `census --closure` still matches those by file name (`wine-build.md`).
- **Tests:**
  - fmt and clippy clean;
  - `cargo test` 228 passed, 8 ignored;
  - replay 60,752 of 60,752;
  - `record --check` reproduces all 80 files;
  - Node 261 of 261;
  - ci.yml dispatch 43385084634 green.
- **Rework at review:** none.

### The close (2026-11-13)
- **Verification** (`/milestone-close` step 1, run by a builder on the code at `4c6ec0d`): every command passes.
  - fmt; clippy, on the workspace and on `x64wasm-fp` for wasm32;
  - `cargo test --workspace`: 228 passed, 8 ignored;
  - `record --check`: all 80 files reproduce;
  - L1 replay: 60,752 of 60,752 cases in each of the four modes;
  - census `--check`:
    - 7zz and CoreMark: 487 of 487 forms;
    - basic, hello, l2_crt, l2_float and l2_int: 25, 230, 237, 355 and 272 forms, all covered;
    - Wine's closure (83 modules, `wine-4c6ec0d`): 797 of 797;
  - `npm run test:node` 261 of 261, and `npm run test:browser` 5 of 5;
  - the 7-Zip matrix in translated mode: 138 of 138 steps, identical to `tests/sevenzip/expected` (`compare.mjs`).
- **Spec review, 9 items:**
  - `wine.lock` is moved;
  - the missing CI translation gate is recorded for M3 (Known issues);
  - seven contract gaps are closed by documenting the published data in `wine-build.md` and `instruction-scope.md`:
    - the baseline wrapper;
    - `closure.json`'s extra fields and `data_files_note`;
    - data files listed from the install tree;
    - `+loaddll` during the Wine baseline;
    - the status rules;
    - the prefix path;
    - the int3 counts.
- **Code review** (`/code-review medium`), 5 findings:
  - **Fixed, with tests:**
    - `testkit closure` fails on a PE image whose tables don't parse, where it used to skip it. Run on `wine-4c6ec0d`'s tree and traces, the stricter closure gives exactly the published module list.
    - Delay imports without a name table list no functions, where they used to read the delay IAT as names.
    - `release-publish.sh` adds a missing `.sha256`, made from the release's own copy (a dry run with a stub `gh` covers it).
    - `run-baseline.mjs` rejects an `--env` without `=`.
  - **A comment:** the census classes `int imm8` by Code, `CD 03` included.
  - **Documented:** native Wine's 8-bit exit status (Known issues).
- **After the fixes:** fmt, clippy, the new and affected tests, and `npm run test:node` (261 of 261) pass.
- **CI on the PR is green** at `8a39380`: `ci`, `bench`, `targets` and `wine`.
  - The `wine` run published `wine-8a39380`, built with the stricter closure code. Its `closure.json` lists the same modules, paths, bases, sizes and edges as `wine-4c6ec0d`'s.
  - So `wine.lock` stays at `wine-4c6ec0d`.
- **The retro** is in `docs/dev/agent-runs.md`, "Retro: Phase 2 M1".

## M2: Wine's Unix side boots (done 2026-11-14)

M2 delivered Wine's Unix side (ntdll's Unix part and the in-process wineserver) as one Emscripten wasm64 module, built locally and in CI. In Node it boots to `LdrInitializeThunk` without running guest code: ntdll and hello.exe mapped, `PEB->ApiSetMap` set, the registry and NLS loaded, KUSER filled, the main thread's state block ready. Four C harnesses and a boot test cover the virtual memory, sections, server waits and the boot.

Session 2 (from 2026-11-13) started fresh from M1's hand-off. The research pass is in `docs/dev/research.md`, "2026-11-13: Phase 2 M2, Wine's Unix side". The contracts are `docs/design/wine-unix.md` v1 and `abi.md` v5's M2 parts ("Trampoline", and under "Wine boundary": memory growth, the host region, the C header, the host entries).

### Decisions taken when planning M2
- **Emscripten stays 6.0.12.** 6.0.13 and 7.0.0 change no default we use, and 7.0.0 caps pipes at 64 KiB, which the in-process server's replies would hit. Revisit at M6.
- **The spike's first open item, `sbrk`:** `INITIAL_MEMORY` is 4 GiB and `sbrk` is wrapped to refuse a break past 0x1_0000_0000, so the heap never grows the memory and the virtual backend is the only grower (`wine-unix.md`, section 5).
- **The spike's second open item, `depth`:** `run` keeps the caller's `depth` and restores it on return and after an unwind, so the depth guard bounds the wasm stack across nested runs (`abi.md`, "Trampoline").
- **The green pthread layer's single-thread part moves into M2's build skeleton.** The `-ww` libc that `-sSHARED_MEMORY` links has no pthread functions at all. Threads stay M4.
- **The transport keeps Wine's pipes.** After writing a request, the client calls the server's `read_request` synchronously; waits block in `green_block` (`wine-unix.md`, section 7).
- **Images:** the preferred base when it is free, otherwise a deterministic slot from the server's `ranges64`, now [0x1_0000_0000, 0x1_7000_0000) top-down. M3 still decides rebasing the closure DLLs.
- **KUSER is one page** that the guest and the in-process server share; the wineboot skip writes wineboot's seven fields. The oracle is a dump of KUSER from native Wine 11 (`kuser-dump.exe`), compared except the clock's fields and the CPU profile's.
- **The prefix runs at its recorded path,** `/home/runner/wine-prefix`, with `HOME=/home/runner` and `USER=runner`.
- **Image placement and section views are tested through the server,** in the `section` harness after the virtual and wait packages merge, since `NtCreateSection` needs the server. The virtual harness tests the backend without it.
- **Context checkpoints (the owner's change, trial 6 in `docs/dev/agent-runs.md`):** planning filled 483,687 of 1,000,000 tokens before any work, so the coordinator reads `context_usage` at each checkpoint and asks for `/compact` above 35 %.

### First pause (2026-11-13, 14:57-18:30 UTC)
The owner's 5-hour limit stopped WP0 and WP1 after about 10 minutes. Their work went to `m2-wp0-wip` (`2888331`) and `m2-wp1-wip` (`fa891e1`), and both were resumed by message ("Resumed" below). The second pause is under "Second resume".

### Resumed (2026-11-13, 18:30 UTC)
- WP0 and WP1 were resumed by `SendMessage` in their worktrees, keeping their context. The owner ran `/compact` at the boundary from planning to execution.
- **WP0 merged (`f0c0827`):**
  - `abi.rs` defines the CPUID table, `MXCSR_MASK`, `XCR0` and `NO_DISPATCHER` once. `x64wasm-cli gen-header` writes `native/wine/include/tempwind/abi.h`, and `crates/x64wasm/tests/c_header.rs` fails on drift.
  - `run` keeps the caller's `depth`. The L5 snapshots changed only in `run`.
  - The spike runs at 4 GiB with `__wrap_sbrk` and takes its constants from `abi.h`. New checks: `depth` across a nested run (on v4's `run` it fails, 1 before and 0 after), the heap cap after growth (`malloc` gives NULL with `ENOMEM`, and neither the break nor the memory moves), and C finding lookup slots with `tw_lookup_hash`.
  - Tests: `cargo test --workspace` 231 passed, `npm run test:node` 261 of 261, `wine-boundary` 5 of 5 runs in Node and in the Chromium Worker.
  - Measured: creating the 4 GiB shared memory64 takes a median 0.3 ms in Node and 1.0 ms in the Chromium Worker (`abi.md`, "The spike (M1)").
- **Decision, from WP0's report:** `depth` joins the green-thread save set (`abi.md`, "Green threads"). C restores it on the resumed stack, and a new thread starts with 0. M3 adds it, since the global exists once translated code runs, and M4's test with two threads checks it.
- **WP1 merged (`7d55b61`):**
  - `native/wine/build.sh` compiles 69 sources (17 of ntdll's Unix part, 44 of the server, 8 of `src/`) with 3 patches, 2 stubs (`emscripten_thread_sleep` and `__get_tp`, both stopping loudly) and 7 renames plus `main`. All three gates pass: no undefined symbols; 79 imports, all in `imports.txt` (77 in `wine-unix`, plus `green_block` and `memory_grown` in `test-link`); 7 collisions, all renamed.
  - `wine-unix.wasm` is 922,558 bytes raw and 335,684 gzipped. A clean build takes 14 s with a warm Emscripten cache, the tarball's download included.
  - `npm run test:wine` passes 4 of 4: the module instantiates with 4 GiB and only listed imports, the harness checks pass, and the two loud stops work (`pthread_create`, and a suspension while holding a mutex). The coordinator rebuilt and retested the merge of WP0 and WP1.
  - **Contract changes, committed before the code** (`wine-unix.md`, section 2): objects compile with `-sSHARED_MEMORY`, since the shared-memory link refuses objects without atomics and bulk memory. The link adds `-Wl,--no-gc-sections`, since with section GC wasm-ld drops what the skeleton entries don't reach and never reports its undefined symbols (the first link was a 19 KB module).
  - **Follow-ups:** `NtCurrentTeb()` is `thread.c`'s, so `signal_wasm64.c` doesn't define it, and `src/tempwind_private.h` joins the tree (both in `wine-unix.md`, section 1). Some reasons in `imports.txt` are WP1's best guesses (`__call_sighandler`, `_setitimer_js` and the socket syscalls), for WP3 to confirm. WP5 adds `native/wine/build.sh` and `npm run test:wine` to `docs/dev/commands.md`.
- **WP2, WP3 and WP5 started** (18:55 UTC), each in its own worktree from `ff6318b`, on the tracks `m2-wp2-wip`, `m2-wp3-wip` and `m2-wp5-wip`. WP2 stops after its base maps (`virtual_init`, the first TEB with the KUSER page, reserve, commit and free). The coordinator merges them, tells WP3 to merge the PR branch, then resumes WP2 for the rest of the `virtual` checks. WP5 runs native Wine for the KUSER oracle in the container if it can, and otherwise reports what is missing.
- **WP5 merged (`c8d0cf2`):**
  - `tests/wine/prepare.sh [--fetch]`, `tests/wine/kuser-dump.c`, `.github/workflows/wine-unix.yml` (a build with Emscripten's cache, `npm run test:wine`, and the release `wine-unix-<sha7>`), the commands in `docs/dev/commands.md`, and an empty `tests/targets/wine-unix.lock` for the close.
  - Native Wine 11 runs in the container once `zstd` is installed (`apt-get install -y zstd`, needed for the `wine.lock` assets). `prepare.sh` writes a 4096-byte `kuser.bin`.
  - **The coordinator's review fixed two things (`b209caa`):**
    - The push trigger now takes `main` and `claude/**` only, like the other Wine workflows. WP5's own push started a run on its WIP branch.
    - Native Wine runs on a copy of the prefix. It had rewritten the fetched `user.reg` and `system.reg` for the user it ran as (71 and 34 changed lines: the shell folders and the version keys), and the boot test reads that prefix. After the fix, `prepare.sh` exits 0, the prefix stays as fetched, and `npm run test:wine` passes 4 of 4.
- **Worktrees:** WP5's worktree started at `main` (`d9f3e71`), not at the PR branch's head, and it reset itself to `ff6318b`. WP2's and WP3's started at `ff6318b`. Later briefs ask the agent to check its base first.
- **Checkpoints (trial 6):** read at the turn's start: 116,079 tokens (11.6 %) at WP0's merge, 178,658 (17.9 %) at WP1's.

### Second resume (2026-11-13, 23:31 UTC)
The owner paused at 19:08 UTC (the second pause). WP2 and WP3 were resumed by message with their context. The trial 6 `/compact` came at the hand-over from planning to execution (29.1 % before it).
- **WP2 part 1, the base maps (merged, `4f1e0d1`):**
  - What it is:
    - `0101-ntdll-virtual-backend.patch` sends `virtual.c`'s `mmap`, `munmap`, `mprotect`, `madvise` and `msync` to `virtual_wasm.c`. It also sets the limits to 16 GiB, takes the static reserve list in `virtual_init`, and moves Wine's bookkeeping into the host heap.
    - The backend keeps a record of maps, one byte per 4 KiB page (4 MiB of host heap). It handles file views with write-back and the stop on a second writable view, and provides `tw_page_flags` and `tw_page_watch`.
    - The contract part came first, as its own commit (`58b9a46`): the guest regions start reserved, a map outside them fails, and a fixed map zeroes only the pages that may hold data.
  - Checks: `init_first_teb`, `kuser_committed`, `reserve_commit_free` and `fixed_high_grows`. Initialisation alone leaves the memory at 4 GiB.
  - On the merge: `build.sh` passes its three gates (78 imports) in 23 s with a warm cache, and `wine-unix.wasm` is 926,858 bytes (337,539 gzipped). `npm run test:wine` passes 6 of 6.
- **Contract decisions from WP2's questions (`3840819`, `wine-unix.md` section 4):**
  - 64-bit Wine never lowers `user_space_limit` for an EXE without `LARGE_ADDRESS_AWARE` (`virtual_set_large_address_space`), and neither do we. The old sentence said it did, "as in Wine".
  - An image based above 4 GiB that maps at neither its preferred base nor its slot searches the image range [0x1_0000_0000, 0x1_7000_0000) first. Wine's own first range ends at 16 GiB, so a top-down search there would grow the memory to 16 GiB.
  - The server's `ranges64` change in `server/mapping.c` is WP3's (section 1: one owner per file).
- **Checkpoint (trial 6):** 322,877 tokens (32.3 %) at the turn's start, before WP2 part 1's merge.
- **WP3, the in-process server (merged, `5122ac2`):**
  - **`0201-server-inproc.patch`** (`server/*` and `unix/server.c`):
    - The server starts inside `server_init_process`: `create_server_dir`; `init_memory` with 4 KiB pages, `ranges64` [0x1_0000_0000, 0x1_7000_0000) and KUSER at 0x7ffe0000; `init_directories`, `init_threading` and `init_registry`.
    - After `send_request`, the client calls `inproc_server_request`, which reads the one pending request and handles it.
    - An empty `wait_fd` read waits in `green.c` until `send_thread_wakeup` wakes the queue.
    - `thread_get_inflight_fd` `dup`s the fd by number, and `send_client_fd` queues a dup for `wine_server_receive_fd`.
    - The scheduler's timeout entries skip the registry save.
    - SIGQUIT exits. `main_loop`, `receive_fd`, partial reply writes and other signals stop loudly.
  - **`green.c`:**
    - waits on queues with deadlines, sleeps and yield;
    - `select`, `poll`, `usleep`, `nanosleep` and `sched_yield` in place of libc's;
    - `tw_next_timeout` and `tw_run_timeouts`.
  - **`unix-host.mjs`** has the scheduler: a deadlock rejects the waiting entry with `tempwind: green: deadlock`.
  - **The contract part came first** (`38d8cc9`: `wine-unix.md` sections 7 and 9, and `abi.md`).
  - **Checks:**
    - `event_timeout`, `event_signaled`, `timer_fires` and `zero_timeout_polls`;
    - `delay_not_alertable` and `delay_alertable`;
    - the loud stop `wait_forever_stops`.

    The non-alertable delay allows 1 ms of slack. Wine counts that deadline with `NtQuerySystemTime`, whose clock (Emscripten's `Date.now`) has a 1 ms resolution.
  - **On the merge:** the three gates pass, 78 imports, in 17 s. `wine-unix.wasm` is 927,271 bytes (337,544 gzipped). `npm run test:wine` passes 8 of 8 in 0.6 s, with `TW_WINE_DIR` pointing at a prepared `target/wine`. The review found no defects.
  - **Found for later:**
    - The server's `process_sigkill` would `kill()` the process itself, since the client and the server share one pid. M2 never reaches it; in M4 it becomes an exit path or a loud stop.
    - `imports.txt` keeps a line for `__syscall_poll`, which no module imports now. Gate 2 only checks a subset, so it does no harm.
    - For WP4: `virtual_map_user_shared_data` must keep the KUSER page that the server writes.
- **Checkpoint (trial 6):** 175,787 tokens (17.6 %) at the start of the turn before WP3's merge. The previous reading was 32.3 %, and no `/compact` ran in between: the reading's boundary marker had changed.
- **WP2 part 2, with the `section` checks (merged, `ff7d5b3`):**
  - **`0102-ntdll-image-placement.patch`:** every image tries its preferred base first. Only then does a relocated DLL ask the server for a slot. After that comes the image range above 4 GiB, then anywhere.
  - **`0103-ntdll-virtual-checks.patch`:**
    - `PAGE_WATCH` follows `VPROT_WRITEWATCH`;
    - queries report the NULL region, the gap and the host region as reserved;
    - KUSER is written in place (`virtual_map_user_shared_data` maps nothing);
    - the three `__TRY` sites check the pages against the views: a checked write records itself on write-watched pages, and a buffer wholly in the host region passes.

    The contract part came first (`77ba740`, section 6).
  - **The `section` harness** (moved from WP4):
    - hello.exe maps at 0x1_4000_0000;
    - `sas.dll` maps at its preferred base, 0x1_8000_0000;
    - `softpub.dll`, whose base is taken, gets the first `ranges64` slot, 0x1_6ffe_0000;
    - a third view comes from the image-range search;
    - a writable shared file view is written back on flush and unmap.

    The `virtual` harness has 12 checks, plus `second_writable_view` run alone (a loud stop). The close added a 13th, `reserve_high_no_growth`.
  - **On the merge:** the three gates pass, 78 imports, in 19 s. `wine-unix.wasm` is 926,806 bytes (337,476 gzipped). `npm run test:wine` passes 10 of 10 in 0.5 s. The review found no defects.
  - **Found for later:**
    - A view at the server's slot returns `STATUS_SUCCESS`, because the server compares against `map_addr` (Wine's ASLR semantics). So once a mapping has a slot, a later view at its preferred base returns `STATUS_IMAGE_NOT_AT_BASE`.
    - A checked write to a write-copy page doesn't turn it into a written page. Neither do translated code's writes. (Corrected in M3's planning: Wine 11's fault doesn't either, so this matches Wine; `wine-unix.md` section 6.)
    - `MEM_TOP_DOWN` with no limit starts at 16 GiB. The ceiling from `guest-memory.md` is M3, and `wine-unix.md` section 4 now says so. `guest-memory.md`'s non-LAA sentence is corrected to match section 4.
  - **For WP4:**
    - `virtual_init_user_shared_data` stays untested until `do_cpuid` takes the CPU profile; it still stops loudly.
    - The in-process start skips `server_connect`'s `setup_config_dir` (now in section 7), so the boot set must bring `dosdevices`.
- **Checkpoint (trial 6):** 247,562 tokens (24.8 %) at the start of the turn, before WP2 part 2's merge.
- **WP4, the boot (merged, `83aa097`):**
  - **`0301-ntdll-startup.patch`:**
    - constant paths (`LIBDIR`, no `dladdr`);
    - the dispatcher slots get `TW_NO_DISPATCHER`, and `__wine_unixlib_handle` an opaque handle;
    - the initial directory is the launch directory that `boot` recorded;
    - the wineboot skip;
    - `NtCreateUserProcess` returns `STATUS_NOT_SUPPORTED` with a `tempwind: unsupported:` line;
    - the CPU comes from the translator's profile (`do_cpuid`, `do_xgetbv`, the DAZ probe, one processor), replacing `0002`'s loud stops.
  - **The thread start:**
    - `boot.c` runs `__wine_main` with `WINELOADERNOEXEC=1`. It returns through `longjmp` once `signal_start_thread` (`signal_wasm64.c`) has built the CONTEXT and the state block, as `init_syscall_frame` does.
    - `signal_init_process` commits 0x7ffe1000 and writes the poison dispatcher address there.
  - **The overlay and the boot set:** `native/wine/prefix/overlay.reg` holds the computer name `TEMPWIND`, the hardware description from the CPU profile, and the volatile environment of the prefix's user, `runner`. `runtime/src/wine/boot-set.mjs` writes the boot set into MEMFS and enters `boot`.
  - **The contract part came first** (`0c144bd`, sections 8 and 10).
  - **Checks:**
    - seven in the harness: `rip_ldr_initialize_thunk`, `context_and_state`, `images_at_preferred_bases`, `apiset_map`, `registry_values`, `nls_loaded` and `kuser`;
    - the Node test also checks that the boot opens only its boot set, and measures the boot.

    KUSER matches `kuser.bin` except the clock fields, the CPU-profile fields and `NumberOfPhysicalPages`.
  - **The one check WP4 couldn't finish:** `registry_values` needs the server to load the overlay, which takes a change in `server/registry.c`, a `02xx` file. WP4 proposed the diff without making it, and the coordinator added it as `0202` (`4f8ca56`). That commit lands before the merge, so each commit on the PR branch passes.
  - **History:** WP4's track has the contract commit after the code. Its force-push to reorder `m2-wp4-wip` was denied, so it made a local branch `m2-wp4-final` with the same tree as two commits, contract first. The coordinator merged that branch; `m2-wp4-wip` stays as pushed.
  - **On the merge:** 9 patches; the three gates pass; 79 imports (WP4 added `__syscall_faccessat` and `__syscall_umask`); 21 s. `wine-unix.wasm` is 976,581 bytes (355,625 gzipped). `npm run test:wine` passes 12 of 12. The review found no defects.
  - **Measurements:**
    - Boot to `LdrInitializeThunk` in Node takes 75 to 91 ms: a median of about 76 ms over WP4's 6 runs, and 78.0 to 82.6 ms over the coordinator's 4.
    - Memory after boot is 94,219 pages of 64 KiB (5.75 GiB). Its top is ntdll's end (0x1_7000_0000 + 0xae000); nothing else grew it.
  - **WP4's decisions, accepted at review:**
    - The computer name is `TEMPWIND`, not the runner VM's name from the prefix.
    - `NumberOfPhysicalPages` joins the KUSER exceptions, since it is the host's memory.
    - The wineboot skip writes the version as constants (10.0.19045, `NtProductWinNt`, suite mask 0, `C:\windows`), because the PEB isn't set at that point. They match `kuser.bin`. A per-program Windows version (M3 or later) would need them from the registry, as wineboot reads them.
- **Checkpoint (trial 6):** 291,812 tokens (29.2 %) at the start of the turn, before WP4's merge.

### Work packages
At most two deep-workers at once and three subagents in all. Each brief names its files, the contract sections and the tests that decide "done", and a WIP track `m2-wpN-wip`. Agents commit their own paths only and use their own `--out` directories.

| WP | Agent | When | Owns | Done when |
|---|---|---|---|---|
| WP0: boundary items (merged, `f0c0827`) | deep-worker | ∥ WP1 | `gen-header` and `abi.h`, with the drift test; the CPUID table and the other new constants in `abi.rs`; `run` keeping the caller's `depth` (`emit.rs`, L5 snapshots regenerated); the spike at 4 GiB with `--wrap=sbrk` | cargo tests pass; a spike check shows `depth` equal before and after a nested run, and fails on the old `run`; a spike check shows `malloc` returning NULL at the cap after growth; `wine-boundary` passes in Node and in the Chromium Worker; the time to create the 4 GiB memory is measured |
| WP1: build skeleton (merged, `7d55b61`) | deep-worker | first | `build.sh`, `config.h`, `imports.txt`, `rename-server.h`, `00xx`, `src/` skeletons (`green_pthread.c` working; `stubs.c`, `version.c`, `library.js`; `signal_wasm64.c`, `green.c`, `inproc.c`, `boot.c` and `virtual_wasm.c` with `grow_to` and `__wrap_sbrk`), `tests/harness.h`, `runtime/src/wine/unix-host.mjs` (memory, raw exports, imports, `preRun` ENV, the stdio devices, MEMFS), `tests/wine/link.test.mjs`, `npm run test:wine` | everything compiles and links with the three gates; the `link` test passes |
| WP2: virtual backend (merged, `4f1e0d1` and `ff7d5b3`) | deep-worker | after WP1 | `01xx`, `virtual_wasm.c`, `tests/virtual.c`, `tests/wine/virtual.test.mjs`; from WP3's merge, `tests/section.c` and `tests/wine/section.test.mjs` (moved from WP4: image placement through the server is `virtual.c`'s behavior) | the `virtual` and `section` checks (`wine-unix.md`, section 10). Its base maps (`virtual_init`, the first TEB, reserve, commit and free) come first and are merged early, since the wait harness needs them |
| WP3: in-process server (merged, `5122ac2`) | deep-worker | after WP1, ∥ WP2 | `02xx` (`server/*` and `dlls/ntdll/unix/server.c`), `inproc.c`, `green.c`, the scheduler in `unix-host.mjs`, `tests/wait.c`, `tests/wine/wait.test.mjs` | the `wait` checks |
| WP4: boot (merged, `83aa097`; the overlay load `0202`, `4f8ca56`) | deep-worker | after WP2 and WP3 | `03xx` (`loader.c`, `env.c`, `system.c`, `process.c`, `thread.c`), `boot.c`, `signal_wasm64.c`, `overlay.reg`, the boot set in `unix-host.mjs`, `tests/boot.c` and its Node test | the `boot` checks |
| WP5: CI and prepare (merged, `c8d0cf2`) | builder | after WP1, ∥ the others | `tests/wine/prepare.sh`, `tests/wine/kuser-dump.c`, `.github/workflows/wine-unix.yml`, `docs/dev/commands.md` | run locally under the job's shell first; green on the PR branch, with `wine-unix-<sha7>` published |

- **Hand-offs:** a `src/` skeleton or a Wine file passes from WP1 to its area's owner when WP1 merges, and `unix-host.mjs` passes from WP1 to WP3 to WP4. `00xx` is frozen once WP1 merges: a later header fix goes in a new patch in the fixing package's range. `config.h`, `stubs.c`, `imports.txt` and `rename-server.h` take appended lines from any package, each with its reason.
- **The coordinator:** reviews each diff, merges each track into the PR branch with a context checkpoint after the merge, sets `tests/targets/wine-unix.lock`, then runs `/milestone-close`.

### The close (2026-11-14)
- **Verification** (`/milestone-close` step 1, run by a builder at `0c6ee1b`, the code of `3ea7ee4`): every step passes.
  - fmt; clippy, on the workspace and on `x64wasm-fp` for wasm32;
  - `cargo test --workspace`: 231 passed, 8 ignored;
  - `record --check`: all 80 files reproduce;
  - L1 replay: 60,752 of 60,752;
  - census `--check`:
    - 7zz and CoreMark: 487 of 487 forms;
    - basic, hello, l2_crt, l2_float and l2_int: 25, 230, 237, 355 and 272 forms, all covered;
    - Wine's closure (83 modules from 9 roots, `wine-4c6ec0d`): 797 of 797;
  - `npm run test:node` 261 of 261, and `npm run test:browser` 5 of 5;
  - the 7-Zip matrix in translated mode: 138 of 138, identical to `tests/sevenzip/expected`;
  - `native/wine/build.sh`: the three gates pass (7 collisions renamed, no undefined symbols, 79 imports listed), and `wine-unix.wasm` is 976,581 bytes (355,625 gzipped);
  - `npm run test:wine` 12 of 12, with the boot to `LdrInitializeThunk` in 74.7 ms.
- **A fresh checkout's prerequisites,** which the verification ran first: `tests/pe/build.sh` and `tests/native/build.sh` before their `prepare.sh`, and `native/spike/boundary/build.sh` for `test:browser`. Without `target/wine-spike`, `test:browser` failed once.
- **The 5-hour limit** stopped both close agents (verification and spec review) at about 00:27 UTC. They resumed by message at 13:15 UTC. The disk was down to 2.9 GB, and freeing finished worktrees' build output (WP0's and WP3's `target/`) brought it to 15 GB.
- **The spec review (step 2.1) listed 14 items:**
  - **One must-fix:** `wine-unix.md` section 4 said a query above 16 GiB reports the range as reserved. Wine returns `STATUS_INVALID_PARAMETER` (its working-set-limit check, `virtual.c` L5740), and the test pins Wine's behaviour, so the contract changed (`fb2133b`).
  - **Contract wording fixed in that commit:**
    - `l_intl.nls` is found through `unicode.c`'s `DATADIR` fallback;
    - the caller `chdir`s before `boot`;
    - `WINEDEBUG` only comes through `opts.env`;
    - the boot set takes the whole `nls/` directory;
    - `pthread_sigmask` is libc's;
    - the link's `--no-entry` and `--error-limit=0`.
  - **CI:** `wine-unix.yml` now also runs on the files its build and tests read: `tests/native/hello.c`, `tests/targets/sources.json` and `wine.lock`, `runtime/src/abi.mjs` and `memory.mjs`, `scripts/emcc.sh`, `fetch-artifact.sh` and `release-publish.sh`, and `package*.json`.
  - The rest are under "Known issues and risks".
- **The code review (step 2.2, `/code-review medium`)** found 9 issues.
  - **Six were fixed by a builder** (`e46fced`). The x87 and reservation fixes have tests, which failed before the fix:
    - the thread start's x87 tag word was 0, which marks all eight registers in use; it is now 0xffff, as the JS host's `initFpState` sets it, and `boot.c` checks FTW and FSW;
    - a `PROT_NONE` reservation grew the memory to its end; now only an accessible map or a file view grows it (the new `virtual` check `reserve_high_no_growth`);
    - `map_file` recorded a writable view before copying it, so a failed `pread` left a record that would write zeros back;
    - `drop_records` cut a view before its tail record existed, so a failed `dup` or `realloc` lost the tail's write-back;
    - after a deadlock, `green.c`'s waiter lists point into dead frames, so the scheduler now refuses every later call;
    - stdin dropped what a read didn't take.

    `boot.test.mjs` now uses `abi.mjs`'s offsets. After the fixes, `build.sh` passes and `test:wine` passes 12 of 12 (`test:node` 261 of 261), and `wine-unix.wasm` is 976,632 bytes (355,633 gzipped).
  - **One became a contract rule** (`abi.md`, `08a95a6`): a `run` that ends by a trap or a throwing import leaves `depth` at the inner value, and its host restores it, as `callNested` does. Today a trap ends the process.
  - **Two were recorded** (below): `find_free`'s quadratic search, and the `boot` argv the host never frees.
- **Checkpoints (trial 6):** 32.3 % before WP2 part 1's merge; 17.6 % before WP3's; 24.8 % before WP2 part 2's; 29.2 % before WP4's; 35.9 % at the restart after the limit, which is over 35 %, so a `/compact` was asked for.

## M3: Boot: hello.exe on Wine (planned 2026-11-14, in progress)

Session 3 started fresh from M2's hand-off. The research pass is in `docs/dev/research.md`, "2026-11-14: Phase 2 M3, boot", with every Wine line the decisions below rest on. M3 makes the M2 boot run: translated PE code (Wine's unmodified DLLs and the program) under the Unix side, through syscalls and unix calls, to process exit. Done when hello.exe and the three L2 programs match native on Wine in Node with both checks on, and hello.exe in a Chromium Worker.

### Decisions taken when planning M3
1. **The closure DLLs are rebased at packaging** (M2's owed decision).
   - Every closure DLL except ntdll, kernelbase and kernel32 (their own bases) and apisetschema (data) gets a unique base, packed downward from 0x1_7000_0000 in [`TW_IMAGE_RANGE_END` = 0x1_6000_0000, 0x1_7000_0000). EXEs keep theirs.
   - The server's `ranges64` and the image-range search end at `TW_IMAGE_RANGE_END`, one constant in `abi.rs` that 0102, 0201 and `section.c` use.
   - Why: 60 closure DLLs share 0x1_8000_0000, so a process would relocate all but one to load-order-dependent slots and miss their AOT modules; 0x1_8000_0000 is also MSVC's default for users' own DLLs. The 58 DLLs packed span 26.1 MiB. This replaces "Known issues"' idea of packing from 0x1_8000_0000 up.
   - The rebase tool is a self-contained LGPL crate under `native/wine/` (the owner's decision: rebased DLLs are modified LGPL binaries Phase 3 would ship). It keeps the DOS stub, `.reloc` and `DYNAMIC_BASE`, recomputes the checksum and writes `rebase.json`.
2. **The AOT catalog:** one module per rebased file (`translate-pe --shared --null-check --store-check`), keyed by path, sha256 and base, plus a boot-set manifest (path, size, sha256). The JS host links a module when the Unix side reports its image mapped at that base; otherwise run-time translation, counted. CI translates the whole closure and compiles every module in V8 (the translation gate).
3. **Dispatcher addresses** `TW_SYSCALL_DISPATCHER` and `TW_UNIX_CALL_DISPATCHER` in the syscall page (proposed 0x7ffe1100 and 0x7ffe1110), written at 0x7ffe1000 and into ntdll's two pointers, mapped by JS to the raw exports `tw_syscall_dispatcher` and `tw_unix_call_dispatcher`.
4. **Syscall dispatch:** EAX id (table bits 12-13, number 0-11), R10, RDX, R8, R9, and 0-based argument i ≥ 4 at `[RSP + 8 + 0x28 + 8*(i-4)]`. Generated typed wrappers from `ntsyscalls.h` and `winternl.h` (arity checked against argbytes; `LDT_ENTRY` copied from its register; the six stub syscalls stop loudly until M5; tables 1-3 empty until M4). KUSER's clock refreshed at entry; `+syscall` traces. The frame record is the state block, with Wine's frame `rsp` and `rip`; the return follows Wine's `restore_flags`: INTEGER loads every GPR (RAX included), otherwise RAX = status; CONTROL returns `STATUS_UNWIND` with the full state (lazy flags materialized), otherwise the guest's `ret`; 0x48 restores the FP state.
5. **Contexts:** `NtContinue`, `signal_set_full_context`, `NtSetContextThread` and `NtGetContextThread` for the current thread, ported against the frame record, FP included. `call_user_apc_dispatcher` stops loudly until M4.
6. **Unix calls:** RCX handle, EDX code, R8 args; only ntdll's `unix_call_funcs` (an unknown handle stops loudly); the same frame and return rules.
7. **The run loop** (C): `run` through `tw_run`; `EXIT_LEFT_CODE` → `tw_translate(rip, base, size)` with the executable view around rip; `EXIT_FAULT` and `EXIT_UNSUPPORTED` (store-check hits included) stop loudly with the kind, address and page flags until M5 and Phase 4.
8. **`tw_run` is bound to a bridge module** from `x64wasm`: its `run` calls `env.table[TW_RUN_INDEX]`, which starts as a placeholder returning `EXIT_LEFT_CODE`. A JS function there would put JS frames between `boot`'s `promising` entry and `green_block`'s `Suspending` import; and `env.lookup` (immutable) and `page_flags` live in the host region, which exists only after the Unix side.
9. **`env.depth`:** the bridge exports `depth_get` and `depth_set`, which the Unix side imports (wasm to wasm). `depth` joins `green_block`'s save set, restored in C; `boot` resets it after a longjmp out of guest code.
10. **Process exit:** a patch hooks `exit_process` and `abort_process` before `get_unix_exit_code`, records the full Windows status and longjmps to `boot`, which returns it (`flags` 0 = run, 1 = `BOOT_STOP_AT_ENTRY`). One instance boots once; each program gets a fresh one. Thread exit and callback return are M4's.
11. **stdio as FIFOs:** fds 0-2 report `S_IFIFO`, non-tty, so Windows sees `FILE_TYPE_PIPE` and msvcrt `WX_PIPE`, as in a native run captured by a pipe; M2's char devices made msvcrt treat stdout as a tty. M4's `7z -so` needs it.
12. **The boot set uses our own lazy file node** (Emscripten's lazy files throw on a stat before the first read, and Wine stats first): size from the manifest, the whole file read on the first read. Every rebased closure module and `data_files` entry is lazy; the prefix, overlay and program are written up front. The host `chdir`s to the program's directory in `drive_c` (`C:\tempwind`, or `opts.cwd`). `WINEDEBUG` only through `opts.env`.
13. **`MEM_TOP_DOWN` ceiling:** a guest allocation with no limit searches below the end of the highest mapped image first (patch 0104).
14. **Write-copy needs no change:** Wine 11 reports `PAGE_WRITECOPY` after a write too (no `VPROT_WRITTEN`; `virtual_handle_fault` touches only guard and write-watch pages). M2's "as the fault would" is corrected.
15. **KUSER's version stays constant:** native wineboot writes the prefix default, never a program's version; per-program versions are PEB-only and come from unmodified PE code.
16. **One initial FP state:** `INIT_MXCSR`, `INIT_FCW`, `INIT_FTW` in `abi.rs` → `abi.h` and `abi.mjs`, with a Node drift test.
17. **Invalidation by tombstones:** key `TW_LOOKUP_TOMBSTONE` (neither `NO_RET` nor `TW_NO_DISPATCHER`; its function stops loudly). The probe misses only on key 0, so translated code doesn't change. A range removes every block of every module overlapping it (blocks call each other directly). The backend calls `tw_invalidate(base, size)` before it unmaps, remaps or writes `PAGE_CODE` pages; `PAGE_CODE` marks only executable pages. Translated stores to code stop loudly until Phase 4.
18. **The store check covers every page of a `rep` range;** both checks are on in Wine processes.
19. **M2's leftovers:** `STATUS_IMAGE_NOT_AT_BASE` after a slot is Wine's semantics, moot once rebased (recorded); the overlay's `PROCESSOR_*` values and KUSER's `XState` and `ActiveProcessorCount` get checks in WP1.

### Decided while writing the contracts (2026-11-14)
The contracts are `abi.md` v5 (final), `wine-unix.md` v2, and the M3 parts of `translator.md`, `guest-memory.md` and `wine-build.md`. Writing them settled these details. Each Wine line they cite was read again in the sha256-checked tarball first.
- **The bridge exports `flags_materialize` too.**
  - Wine's context calls need the exact EFLAGS of the syscall frame, and only the translator can compute it from the lazy flag record.
  - So the syscall dispatcher calls the bridge's copy of the trampoline's own materialization. C never reimplements the lazy flags.
- **The bridge fills two reserved table slots:**
  - `TW_RUN_INDEX` (0) with the placeholder `run`, which returns `EXIT_LEFT_CODE`;
  - `TW_TOMBSTONE_INDEX` (1) with the tombstone's stop.

  Host functions and modules start at `TW_TABLE_RESERVED` (2).
- **The tombstone key** is `0xDEAD_0000_0000_0001`. A probe for it hits its slot and stops loudly; on x86-64 a jump to a non-canonical address raises #GP anyway.
- **The normal syscall return** performs the guest's `ret` with `STATUS_RETURNED`. Wine jumps to the frame's `rip`, `t - 0xb`. For Wine's own thunks both are a `ret` with the same RSP.
- **Patches:**
  - The dispatcher addresses go into 0301, edited in place like 0102 and 0201.
  - The exit hook is the new 0302.
- **M3's imports are declared once**, in `library.js`, `imports.txt` and `tempwind_private.h`, by the coordinator's constants commit. A work package appends any other import at the end of the list in its own commit.
- **`tw_image_mapped(fd, base, size)`:** JS finds the file's MEMFS path from the fd. There is no `tw_image_unmapped`, since `tw_invalidate` covers unmaps.
- **The run-time translator gets request flag bit 5, `shared_memory`.** A module that imports a shared memory must declare it shared.
- **`TW_NO_DISPATCHER`** is no longer written from M3. It is removed with its last use.
- **The catalog and manifests are built, not published:**
  - a session builds the catalog for the modules the test programs load, or all of them with `--all`;
  - CI builds the whole closure and compiles it in V8 as the translation gate.
- **A risk for M4:** KUSER's clock advances only at syscall entries, server requests and green switches. A guest reading only KUSER sees time stand still: kernel32's `GetTickCount` is such a read (`wine-unix.md`, section 13).

### Work packages
At most two deep-workers at once and three subagents in all. Deep-workers start in this order: WP1 and WP2, then WP3 when WP2 merges, then WP5 when WP1 and WP3 merge. Each brief names its files, the contract sections, its tests and a WIP track `m3-wpN-wip`, and asks the agent to check its worktree base first. Before any agent starts, the coordinator commits the contracts (docs), then the `abi.rs` constants with the regenerated `abi.h`, `abi.mjs` and the `tw_run_loop` declaration.

| WP | Agent | When | Owns | Done when |
|---|---|---|---|---|
| WP1: dispatchers and contexts (C) | deep-worker | after the contracts | `signal_wasm64.c` (in run mode `signal_start_thread` calls `tw_run_loop`), `run.c` as a loud stub (it passes to WP5 at the merge), the wrapper generator and its step in `build.sh`, `+syscall` tracing, patches 0102 and 0201 (`TW_IMAGE_RANGE_END`), 0104 (top-down) and 0301 (dispatcher addresses, edited in place), the virtual backend's `tw_image_mapped` and `tw_invalidate` calls, `tests/syscall.c` and `tests/wine/syscall.test.mjs`, the `section`, `virtual` and `boot` harness updates | the three gates; `test:wine` with `syscall` checks on synthetic state blocks (register and stack args with ≥ 7 arguments, the variant entries, a bad id, a stub's loud stop, `NtContinue` returning UNWIND with the full state, a Get/Set round trip with x87 and XMM, a unix call, KUSER's clock advanced), the top-down check, the `PROCESSOR_*` and `XState` checks |
| WP2: translator side | deep-worker | after the contracts (small) | `crates/x64wasm` (the rep-range store check; the bridge: `run`, the placeholder, `depth_get`, `depth_set`), `crates/x64wasm-cli` (`translate-pe --shared` if missing, `bridge`), `crates/x64wasm-rt` (a shared-memory flag), the `abi.mjs`↔`abi.h` Node test, L5 snapshots | a rep store check over three or more pages that fails on the old lowering; bridge tests; replay 100 %; `test:node` |
| WP3: JS host | deep-worker | after WP2 merges | `runtime/src/wine/*` (memory, table, depth, bridge, the Linker in the host region, host functions and slots, `tw_translate`, image notifications and the catalog, `tw_invalidate`, `PAGE_CODE`, FIFO stdio, lazy nodes, chdir, scheduler, exit code), `runtime/src/link.mjs` (injected table, depth and header; tombstones; invalidate), `run-wine.mjs`, `tests/wine/programs.test.mjs`, `boot.test.mjs` | unit tests: invalidate (the whole-module rule, rehash), catalog hit and fallback, a lazy node's stat before its first read in Node, FIFO `fstat`; `test:node` unchanged; `programs.test.mjs` wired |
| WP4: packaging and CI | builder | after the contracts; the catalog step after WP2 | the LGPL rebase crate under `native/wine/`, packing and `rebase.json`, the catalog builder and the boot-set manifest, `tests/wine/prepare.sh`, `.github/workflows/wine-unix.yml` (catalog gate, programs test, path triggers), `docs/dev/commands.md` | rebase tests (every DIR64 target moves by the delta; otherwise only header fields change); the whole closure's catalog builds and compiles in V8 locally under the job's shell; green in CI |
| WP5: run path and first light | deep-worker | after WP1 and WP3 merge | `run.c` (`tw_run_loop`), `boot.c` (run mode, the exit longjmp, the depth reset), `green.c` (depth), the exit-hook patch 0302; then integration fixes in `native/wine/**` and `runtime/src/wine/**`, with `/diverge` against native Wine (`+relay`, `+syscall`, `+loaddll`); translator defects go to the coordinator, fixtures first | hello.exe, then the three L2 programs, pass `programs.test.mjs` in Node with both checks; boot-to-exit time, AOT hits and fallbacks and bytes loaded measured |
| WP6: browser | builder | after WP5's hello passes and WP4 merges | `tests/browser/wine-hello.spec.mjs` and its worker, `server.mjs`, a step in `wine-unix.yml` | hello.exe matches in a Chromium Worker, locally and in CI |

- **Hand-offs:** `run.c` passes from WP1 to WP5 at WP1's merge. `library.js`, `imports.txt`, `config.h`, `stubs.c`, `rename-server.h` and `tempwind_private.h` take appended lines from any package, each with its reason.
- **The coordinator** reviews each diff, merges each track into the PR branch with a context checkpoint after the merge, and deletes merged worktrees' `target/`.

### Checkpoints (trial 6)
- **At the plan's approval:** 373,677 tokens (37.4 %). The planning stretch cost $14.31 and 32.9M cache reads, 24.2M of them the four agents', so the coordinator read about 8.7M (M2's planning: 27.5M). A compaction was asked for before the contracts.
- **After the compaction:** 100,894 tokens (10.1 %).
- **After the contracts, the first briefs and WP4's first pass:** 479,531 tokens (48 %), $42.73 and 107.5M cache reads for the whole session.
  - Writing the contracts cost the most. It took many turns at a large context, reading the docs and Wine's lines, so about 65M cache reads.
  - A compaction was asked for.
- **At the owner's pause:** 101,742 tokens (10.2 %) after the second compaction. The whole session, agents included, cost $52.90 and 138.2M cache reads.
- **After WP1's merge (2026-11-20):** 193,840 tokens (19.4 %); $63.06 and 156.6M cache reads for the whole session.
- **After WP2's merge and WP3's brief:** 258,581 tokens (25.9 %); $67.99 and 172.5M cache reads. That is above 25 %, so a compaction was asked for.
- **After WP4's merge (the third compaction came first):** the reading hadn't refreshed. It still showed 326,965 tokens, last updated at 15:27 UTC, before the compaction, so this checkpoint has no fresh token figure. The whole session so far: $76.78 and 194.4M cache reads.
- **After WP3's merge:** 215,546 tokens (21.6 %); $88.48 and 228.2M cache reads for the whole session.

### Progress (2026-11-14)
- **Pushed to the PR branch:**
  - `986f037`, the contracts;
  - `b3e3bc5`, the constants;
  - `b1f6262`, `createUnixHost`'s `opts.imports`. M3's imports get defaults there: the bridge's and `tw_translate` stop loudly, while `tw_image_mapped` and `tw_invalidate` do nothing. So WP1 and WP3 don't both edit the hook;
  - `fe73fe5`, the research note on caveman (not adopted).
  - The `wine-unix` workflow passed at `b3e3bc5` and `b1f6262`.
- **Shared inputs:** `target/wine` in the main tree is prepared (`tests/wine/prepare.sh --fetch`; `zstd` installed). Agents read it through `TW_WINE_DIR`.
- **WP1 and WP2** run in worktrees, on the tracks `m3-wp1-wip` and `m3-wp2-wip`.
- **Added to WP2: ntdll's `pop qword [rcx+0x44]` at 0x1_7001_0992.**
  - It is `RtlCaptureContext` saving EFlags (`dlls/ntdll/signal_x86_64.c` L147).
  - With both checks on, `translate-pe` panics there at `lift/lower.rs:237`: "memory check of an address computed after a side effect". The destination address is computed after RSP's update.
  - It is the only pop-to-memory form in the programs' closure DLLs (objdump).
  - The fix comes with tests: an RSP-based destination too, and a store-check hit leaving RSP and memory unchanged. WP2's report must show the whole closure translating with all three flags.
- **WP4's first pass** is on `m3-wp4-wip`, at `f0ed274` (based on `b1f6262`). It is reviewed, not merged.
  - **What works:**
    - `pe-rebase`: 10 unit tests; it rebases the real closure's 59 DLLs, and `--verify` passes;
    - `tests/wine/catalog.mjs`: the default scope's 7 modules built and compiled in V8, with a stub without `--shared`;
    - `boot-set.json`: 155 files;
    - the `prepare.sh` steps and the workflow's catalog gate.
  - **To do when it is resumed, after WP2 merges:**
    - fix the doc comment that calls the crate's `RANGE_END` `TW_IMAGE_RANGE_END`. `RANGE_START`, 0x1_6000_0000, is that constant;
    - add a test that reads `TW_IMAGE_RANGE_END` from `abi.h`, the file only, since the crate stays independent;
    - drop the gate step's second rebase, which `prepare.sh` already ran and verified;
    - run `prepare.sh` end to end with `bridge` and `--shared`, and the gate with `--all`.

## M3 progress (paused 2026-11-20)
The owner paused M3 again at 15:47 UTC on 2026-11-20. The five-hour rate limit read `allowed_warning` at 15:41 and resets at 19:50 UTC. Nothing is uncommitted, no agent is running, and every side branch's work is merged. The decisions and the work-package table (each package's files and done-when) are in the M3 section above. The first pause and the resume follow this summary, for the record.

**Committed** (the PR branch `claude/nice-albattani-dpo4jx`):
- **WP1 to WP4 are merged.** WP1 is `c67bc06`, WP2 `7bbb63a` with its follow-up `41360d2`, WP4 `30dcc06`, and WP3 `272fa1f`.
- **Fixes:**
  - `3035af6`, abi.md's tombstone sentence;
  - `ee7884f`, which removes `TW_NO_DISPATCHER`;
  - `e53700d`, boot's run mode;
  - `4226986`, the wasm32 build in `prepare.sh`.
- **Passing at `272fa1f`:**
  - `test:node`: 265 of 265;
  - `test:wine`: 32 of 32. The four programs stop at "tw_run_loop (M3 WP5)", with the program and ntdll linked from the catalog;
  - `pe-rebase`: 11 of 11;
  - the catalog gate: all 77 modules compile in V8.
- **Also passing:** `cargo test` (244 of 244) at WP2's merge.
- **CI:** `wine-unix` is green at `30dcc06`, `e53700d` and `4226986`. It was running at `272fa1f` when the session paused.

**In progress:** nothing. WP5 and WP6 haven't started.

**How to resume**
- **In a fresh session:**
  1. **Prepare the shared inputs** in the main tree, since they lived only in this container:
     - `scripts/apt-install.sh zstd gcc-mingw-w64-x86-64` (the workflow's packages);
     - `tools/start-dockerd.sh`;
     - `tests/wine/prepare.sh --all`. It builds the Unix side, the L2 programs, the rebase, `bridge.wasm`, the whole closure's catalog and the wasm32 run-time translator and floating-point helper. It took 50 s here.

     For `test:node` also run `cargo run -q -p testkit --release -- spike --out target/spike`, `tests/pe/prepare.sh` and `tests/native/prepare.sh`.
  2. **Brief WP5** (deep-worker, effort high):
     - **Sources:** its row in "Work packages" above, decisions 2-4, 7-10 and 17, and "Resumed (2026-11-20)" below. That section has WP1's open questions, the boot fix and WP3's notes for WP5.
     - **Track:** `m3-wp5-wip`, from the PR branch head (`git fetch origin claude/nice-albattani-dpo4jx && git checkout -B m3-wp5 FETCH_HEAD`).
     - **What the brief must say:**
       - read `target/wine` through `TW_WINE_DIR` without rewriting it, and run its programs, not `target/native`'s (sha256);
       - native Wine 11 for `/diverge` is in `target/wine/native`; `prepare.sh` step 3 shows how to run it;
       - a translator defect goes to the coordinator with a fixture first, and a contract defect stops the agent with a report;
       - "done" is the row's done-when, with `programs.test.mjs` switched from the run loop's stop to stdout (CRLF) and the exit code, and `run-wine.mjs --stats` for the measurements.
  3. **Then:** WP6 (builder) after WP5's hello passes, then `/milestone-close`.
- **In this session,** while its container lives: the inputs above are prepared, and a draft of WP5's brief is in the session's scratchpad, as `wp5-brief.md`.
- **Open item:** the `wine-unix` run at `c67bc06` hung in its prepare step. Read its log once it times out: the native Wine run in `prepare.sh` has no timeout of its own.

### First pause (2026-11-14)
The owner paused M3 at 20:15 UTC, when the five-hour rate limit read `allowed_warning`. Nothing was left uncommitted: each track's work was on its side branch, and no work package was merged yet. What follows was the state then; the resume below supersedes it.

**Committed**
- **The PR branch** `claude/nice-albattani-dpo4jx` has:
  - the contracts (`986f037`: `abi.md` v5, `wine-unix.md` v2);
  - the constants (`b3e3bc5`);
  - the host's `opts.imports` (`b1f6262`);
  - notes.

  The `wine-unix` workflow passed at `b3e3bc5` and `b1f6262`.
- **The side branches** are all based on `b1f6262`. The coordinator hasn't reviewed or tested WP1's and WP2's commits yet.

**Per track**
- **WP1, `m3-wp1-wip` at `b8ffe57`** (deep-worker):
  - **Done:**
    - `d996c62` is a contract change, to review at the merge: the top-down ceiling and `tw_image_mapped` checks move to the `section` harness, which has the server's image views;
    - `2e6013a` (WIP): the dispatchers, contexts, wrapper generator and patches.
  - **Saved at the pause** (`b8ffe57`):
    - `tests/syscall.c`, half written and never run;
    - unverified changes to the `section` and `virtual` harnesses, `gen-syscalls.mjs` and `tests/wine/m3-imports.mjs`.
  - **Left:** the syscall harness and `tests/wine/syscall.test.mjs`, then the row's done-when (the three gates, `test:wine`).
- **WP2, `m3-wp2-wip` at `a85ed5d`** (deep-worker):
  - **Done:**
    - `0b53058`, the failing rep test;
    - `26f652f`, the store check over every page of a `rep` range;
    - `974778a`, the `pop m64` fix for ntdll's `RtlCaptureContext`.
  - **Saved at the pause** (`a85ed5d`): the bridge (`bridge.rs` and its tests) and `translate-pe --shared`, unverified. The `pop m64` regression test was being written.
  - **Left:**
    - finish those, and add the `bridge` command;
    - the run-time translator's `shared_memory` flag;
    - the `abi.mjs`↔`abi.h` test and the L5 snapshots;
    - then the row's done-when, and the whole closure translating with all three flags.
- **WP4, `m3-wp4-wip` at `f0ed274`** (builder): its first pass is reviewed. The four items under "Progress (2026-11-14)" come once WP2 merges.
- **WP3, WP5 and WP6** haven't started.

**How to resume**
- **In this session,** while its container lives: the stopped agents can be messaged, and they keep their context. A resumed run's ledger line counts only its increment, with `--since`.
- **In a fresh session:**
  1. Prepare the shared inputs, which lived only in this container: `scripts/apt-install.sh zstd`, then `tests/wine/prepare.sh --fetch` in the main tree. Agents read `target/wine` through `TW_WINE_DIR`.
  2. Brief WP1 and WP2 again from their rows in the table, the contracts and this section. Each brief starts with `git fetch origin m3-wpN-wip && git checkout -B m3-wpN FETCH_HEAD`, then reads the WIP commits' messages. It pushes with `git push origin HEAD:refs/heads/m3-wpN-wip`.
  3. Then follow the table's order:
     - WP4 and WP3 when WP2 merges;
     - WP5 after WP1 and WP3;
     - WP6 after WP5's hello and WP4.
  4. For each report: review the diff against the contracts, merge it into the PR branch, log its ledger line, and checkpoint.
- **Ledger:** the three runs' segments up to the pause and a `main` snapshot are logged. Transcripts live only as long as the container, so they couldn't wait.

### Resumed (2026-11-20)
The owner resumed M3 at 15:01 UTC, in the same session. The machine had restarted, but its disk survived, so WP1 and WP2 were messaged and kept their context.
- **WP1 is merged** (`c67bc06`, from `35e2f08`).
  - **Checks:** the coordinator rebuilt the final commit. The four build gates pass (258 syscalls, 6 of them stubs), and `test:wine` passes 16 of 16, the `syscall` checks included.
  - **Patches:**
    - 0104 adds the top-down ceiling;
    - 0105 calls `tw_image_mapped` from `virtual_map_image`;
    - 0301 also gives the unix-call dispatcher ntdll's table (`tw_unix_call_funcs`, `tw_unix_call_count`).

    `tw_invalidate` is called from the backend, `virtual_wasm.c`, with no patch.
  - **Run mode:** `tw_boot_flags`, set by `boot`, picks it in `signal_start_thread`. `tw_run_loop` stays WP5's loud stub.
  - **Open questions from WP1's report**, not checked against Wine 11 yet (for WP5's `/diverge`, if they matter):
    - which values `NtGetContextThread` should return for RCX, RDX and R8-R11 during a syscall (ours: the values at entry);
    - MXCSR's reserved bits in a context: we mask them, where an `fxrstor` would fault;
    - RAX's upper half after a syscall: we zero-extend the 32-bit status.
  - **`TW_NO_DISPATCHER`** had no use left. It was removed after WP2's merge (`ee7884f`).
- **WP2 is merged** (`7bbb63a`, from `67e9f5a`).
  - **Checks**, rerun by the coordinator on the final commit: fmt and both clippy runs are clean; `cargo test` passes 244 of 244, with replay at 100 %; `test:node` passes 263 of 263.
  - **The closure**, by WP2's report, translates with `--shared --null-check --store-check`: 134 of 135 DLL files (apisetschema is data) and the six root programs.
  - **What changed:**
    - the store check of a `rep` range tests every page (`Op::PageRangeCheck`; the failing test came first, in `0b53058`);
    - `pop m64` computes and checks its address before RSP changes;
    - the bridge module (`bridge.rs`), whose `flags_materialize` uses the trampoline's own emitter, and `x64wasm-cli bridge`;
    - `translate-pe --shared` and the run-time translator's flag bit 5;
    - the `abi.h`↔`abi.mjs` Node test (54 constants).
  - **What WP2 found:**
    - abi.md's tombstone sentence was wrong. A transfer to `LOOKUP_TOMBSTONE` hits a tombstone only when one lies on its probe path; otherwise it misses, and the run loop stops. It stops loudly either way, and abi.md now says so (`3035af6`).
    - The load plan's options dropped `--shared`, while abi.md (L262) has the runtime pass a plan's options as the request flags. WP2's follow-up adds `sharedMemory` (runtime.md first, `420e541`; code `ed0543c`; `requestFlags` in `process.mjs`), merged as `41360d2` after the coordinator reran the tests it touches. WP2 is done.
- **WP4 is merged** (`30dcc06`, from `ec260f6`). WP4 is done.
  - **The four items** from its first pass's review are done (`ec260f6`): the doc comments name `TW_IMAGE_RANGE_END` correctly, a test reads it from `abi.h` and checks it equals `RANGE_START`, and the gate step no longer rebases a second time. WP4 also ran `prepare.sh --all` end to end.
  - **Checks**, rerun by the coordinator on the merged tree:
    - fmt, clippy and `pe-rebase`'s 11 tests pass;
    - `prepare.sh --all` took 50 s, including the Unix side's 20 s build. It rebased and verified 59 DLLs, wrote the 903-byte `bridge.wasm`, and the merged translator retranslated all 77 modules (`--shared --null-check --store-check`);
    - the gate passes: all 77 modules compile in V8, 165.1 MiB of wasm. `boot-set.json` lists 155 files;
    - `test:wine` passes 16 of 16.
  - **What it adds:**
    - `native/wine/rebase` (`pe-rebase`, LGPL), a workspace member;
    - `tests/wine/catalog.mjs`, which writes the catalog and the boot-set manifest;
    - `prepare.sh`'s packaging step;
    - the workflow's catalog gate, plus path triggers for `crates/x64wasm*` and `tests/native/**`.
- **boot's run mode** (`e53700d`, by the coordinator):
  - **The problem,** from WP3: `boot` still refused flags 0, so run mode never reached `tw_run_loop`.
  - **The fix:** `boot` now accepts 0, as `wine-unix.md` section 8 (L210) specifies. Any value other than 0 or 1 stops loudly.
  - **Checks:** a scratch boot of hello.exe with flags 0 stops at "tw_run_loop (M3 WP5)". The gates pass, and `test:wine` passes 16 of 16.
  - **Who owns `boot.c`:** it stays WP5's, apart from this line.
- **`prepare.sh` builds `x64wasm_rt.wasm` and `x64wasm_fp.wasm`** for wasm32 (`4226986`, by the coordinator). WP3 found that its Wine processes load them, but neither `prepare.sh` nor the workflow built them.
- **WP3 is merged** (`272fa1f`, from `ee5635f`). WP3 is done.
  - **Checks**, rerun by the coordinator on the merge candidate:
    - the test inputs were rebuilt first (`target/spike`, `tests/pe/prepare.sh`, `tests/native/prepare.sh`);
    - `test:node` passes 265 of 265, the same tests as before;
    - `test:wine` passes 32 of 32.
  - **What it adds:**
    - `runtime/src/wine/wine-process.mjs`, which builds a process in section 12's order: memory, `env.table`, `env.depth`, the bridge, the Unix side, then the Linker with its header and slots in the host region (raw `malloc`) and the dispatchers' lookup slots;
    - in `link.mjs`, an injected table, `depth` and page-state table, plus tombstones, `invalidate` (a module goes whole, and only slots still pointing into it become tombstones) and a rehash that drops tombstones and frees the old slots;
    - `aot-catalog.mjs`, which matches a module by path, sha256 and base. A module the catalog promises but that can't be linked stops with a `tempwind: catalog:` line;
    - `lazy-node.mjs`, whose files take their size from the manifest and load whole on first access, through `fs` in Node or a synchronous XHR in a Worker;
    - fds 0-2 as FIFOs, which `link.test.mjs` now expects (decision 11);
    - `run-wine.mjs`, documented in `commands.md`, with `--stats`.
  - **Tests:** the linker tests (invalidate, tombstone reuse, a slot another module owns, rehash) and the host tests (FIFO stdio, a lazy node's stat before its first read, catalog hit and fallback). The programs test is in place: all four programs boot in run mode to "tw_run_loop (M3 WP5)", and the program and ntdll are linked from the catalog, with no fallback and no run-time translation.
  - **Contract:** `wine-build.md` now says a program's sha256 comes from its own bytes, so `run-wine.mjs` matches a program the manifest doesn't list (`de3e6c5`). It landed on the track after the code that implements it.
  - **For WP5:** `target/native/*.exe` don't match the catalog's sha256, since each MinGW build's bytes differ. Use `TW_WINE_DIR`'s programs, as the test does.
- **Next:**
  - WP5, the run path and first light. It is briefed when the owner says to go on (see below);
  - WP6 after WP5's hello (WP4 is merged).
- **Rate limit:** the five-hour limit read `allowed_warning` at 15:41 UTC, and it resets at 19:50 UTC. The owner paused at that reading on 2026-11-14, so WP5 waits for the owner's word.
- **CI (`wine-unix`):**
  - **At `ee7884f`,** which contains WP1's and WP2's merges, it passed in about 3 minutes.
  - **At WP4's merge `30dcc06`** it passed in 2.5 minutes, catalog gate included. The gate step took 3 s. Its `--check --all` fails on any closure file missing from the catalog, so the pass covers the whole closure. The log's tail doesn't reach the gate's own lines, and the full log's host is blocked by the session's proxy.
  - **At `c67bc06`** the run has sat in its prepare step since 15:09 UTC and was still running at 15:35. The job's timeout is 60 minutes. Its log will say which command hung; the native Wine run in `prepare.sh` has no timeout of its own.
- **Disk:** 20 GB free once the merged worktrees' `target/` directories were deleted (WP2's held 12 GB).

## What works

| Item | Evidence |
|---|---|
| **The Wine boundary** (M1 WP3, `native/spike/boundary/`, `abi.md` "Wine boundary").<br>An Emscripten wasm64 module (imported shared memory64, `GLOBAL_BASE` 2 GiB, wasm EH, no pthreads, no `-sJSPI`):<br>• owns the run loop over translated code<br>• is entered through raw exports in `env.table`<br>• runs two green threads that suspend through `green_block` and get their save set back<br>• makes a nested guest call<br>• grows memory while JS, its glue and translated code all see the growth | `tests/node/wine-boundary.test.mjs` and `tests/browser/wine-boundary.spec.mjs` (a Chromium Worker) pass locally and in CI (run 42306776103 at `0182d2f`). A build without the save-set restores fails the checks. A JS frame between `promising` and `Suspending` throws `SuspendError: trying to suspend JS frames`: Node 24.22 first runs the import and drops its Promise, while Chromium 153 refuses before calling it. |

| **The Wine CI build** (M1 WP1, `wine.yml`, `wine-build.md`):<br>• the pinned, mirrored wine-11.0 tarball<br>• 64-bit and minimal, with MinGW GCC 13<br>• the PE modules, the native tree, the prefix<br>• Windows and native-Wine baselines<br>• `+loaddll` traces and the DLL closure<br>• published as `wine-<sha7>` | Releases `wine-bf443f0` to `wine-4c6ec0d`. `tests/targets/wine.lock` names `wine-4c6ec0d`, which has every asset. A session fetches the assets with `fetch-artifact.sh` (sha256 checked), and the native Wine 11 from the release runs `hello.exe` here, matching `hello.expected`. `tests/node/conformance-parse.test.mjs` covers the parser, the Wine trace lines after the list included. |
| **Wine's DLL closure under the translator** (M1 WP2; `instruction-scope.md`, "Wine closure census"; `translator.md`, "Wine closure forms"):<br>• the census of the closure's 83 modules finds 797 in-scope forms, and every one has L1 fixtures (16 new forms recorded and lowered)<br>• every module with code translates with `translate-pe` and compiles in V8 | CI's census step checks the `wine.lock` closure (`census --closure ... --check`). Replay passes 60,752 of 60,752 cases, and `record --check` reproduces all 80 files. The translation and V8 numbers below were measured once, in a session (WP2), not in CI. |

| **Wine's Unix side boots** (M2, `native/wine/`, `wine-unix.md`).<br>• ntdll's Unix part plus the in-process wineserver build with Emscripten 6.0.12 as one wasm64 module: 9 patches and three link gates<br>• built locally and in CI (`wine-unix.yml` publishes `wine-unix-<sha7>`)<br>• in Node it boots to `LdrInitializeThunk` without running guest code:<br>&nbsp;&nbsp;– ntdll and hello.exe at their preferred bases<br>&nbsp;&nbsp;– `PEB->ApiSetMap` in apisetschema's view<br>&nbsp;&nbsp;– the prefix registry plus the overlay<br>&nbsp;&nbsp;– NLS<br>&nbsp;&nbsp;– KUSER equal to native Wine 11's, except the listed fields<br>&nbsp;&nbsp;– the main thread's state block ready | `tests/targets/wine-unix.lock` names `wine-unix-b2d6c58`. `npm run test:wine` passes 12 of 12, from the C harnesses and their Node tests:<br>• `link`<br>• `virtual`: 13 checks, plus a loud stop<br>• `section`: 3<br>• `wait`: 6, plus the deadlock stop<br>• `boot`: 7, plus the opened-files list and the measured boot |

## Measurements

- **The Wine CI build** (`wine.yml`, ubuntu-24.04, 4 vCPU):
  - the build step takes 22.5 min cold, and 1.5-2.0 min with ccache warm;
  - the build job takes 26 min cold and 4.6-5.7 min warm, `make install` and packing included;
  - the Windows baseline takes 3 min; the native-Wine baseline 6 min (31 min while it ran bogus units);
  - a whole run with ccache warm takes 14 min (run 43385076350).
- **Conformance baselines** (`wine-4c6ec0d`, the `wine.lock` release; units per exe; checks = executed − failures over executed):

  | Platform | kernel32_test.exe | ntdll_test.exe |
  |---|---|---|
  | windows-2025 (truth) | 24/33 units ok, 9 failed; checks 19,566,392 / 19,570,626 | 15/26 ok, 10 failed, 1 crashed; checks 5,421,348 / 5,421,446 |
  | native Wine 11 (`--without-x`) | 31/33 ok, 2 failed; checks 5,312,746 / 5,312,795 | 23/26 ok, 2 failed, 1 crashed; checks 5,330,032 / 5,330,037 |

  - Windows varies between runs: kernel32 had 23 units ok in `wine-b4ecc3c` and 24 in the two runs since, and its executed checks went from 15.8 M to 19.0 M and 19.6 M. Native Wine's executed checks vary by about 6,000.

  - Wine's tests aren't clean on Windows Server 2025: units fail on server-edition differences.
  - Under native Wine, `todo_wine` marks absorb Wine's known gaps.
  - Our port is measured against both (M5).
- **The closure translated, in V8** (WP2, `images.mjs`, 75 Wine modules):
  - 153.5 MB of wasm;
  - compile times: 258 ms by default (lazy), 1,978 ms eager (Liftoff), 57,063 ms eager TurboFan;
  - the largest: `kernel32_test.exe` 23.4 MB, `ntdll_test.exe` 16.5, `windowscodecs.dll` 8.7, `ucrtbase.dll` 7.0, `user32.dll` 6.3.
- **The DLL closure** (`wine-bf443f0`'s `closure.json`; `wine-ccff087`'s and `wine-4c6ec0d`'s are the same):
  - 83 modules, 96 unresolved names: test-generated temporary DLLs (`ldr*.tmp`, `ldr*.dll`) and similar;
  - `highest_end` 0x1_808b_c000 and `memory_bytes` 6.45 GB, if everything loads at its preferred base;
  - **base overlaps:**
    - 60 DLLs share the MinGW default base 0x1_8000_0000 and overlap each other (1,825 pairs);
    - only ntdll (0x1_7000_0000), kernelbase (0x1_7400_0000) and kernel32 (0x1_7800_0000) have their own;
    - the 20 EXEs share 0x1_4000_0000, which is harmless, since each is its own process.

- **The boundary spike** (this container, Node 24.22 and Chromium 153 headless):
  - creating the shared memory64 of 2 GiB + 64 MiB takes 0.3-1.4 ms, since the pages are reserved, not committed;
  - compiling and instantiating the Unix side through Emscripten's glue takes 2.4-4 ms;
  - a `green_block` suspend and resume round trip takes 1.2 µs in Node and 0.2 µs in Chromium (10,000 rounds).

- **Wine's Unix side** (M2, Node 24, this container):
  - `wine-unix.wasm` is 976,632 bytes (355,633 gzipped) after the close's fixes (976,581 and 355,625 at `3ea7ee4`);
  - `build.sh` takes 17-23 s locally with a warm Emscripten cache; the CI job, prepare included, takes about 3 minutes;
  - `new WebAssembly.Module()` takes 2.8 ms, since V8 compiles lazily, so the compile time lands in the boot; the `link` test (instantiate with the 4 GiB memory, then a raw `malloc`/`free`) takes about 32 ms;
  - the boot to `LdrInitializeThunk` takes 74.7-91 ms, with medians of about 76-79 ms over the runs;
  - memory after boot is 94,219 pages of 64 KiB (5.75 GiB); the top is ntdll's end;
  - 9 patches (`0001`-`0003`, `0101`-`0103`, `0201`-`0202` and `0301`), 2 stubs, 79 imports in `imports.txt`; `test:wine` runs in about 0.5 s.

## Known issues and risks
- **The Unix-side port is the bulk:** about 100k lines in M2-M3, and 106k more with win32u in M4. The hard parts are the machine layer, the virtual backend and the server transport (Phase 0's risks 2, 4 and 6 in `native/README.md`).
- **Wine's DLLs overlap at one default base** (the measurements above). So in a process most of them can't load at their preferred base, and an AOT module translated for that base would miss.
  - **Corrected in M2's research** (`research.md`, 2026-11-13): native Wine doesn't keep the first DLL at 0x1_8000_0000. It moves every DYNAMIC_BASE DLL that has relocations (`ImageDynamicallyRelocated`) into its ASLR range, ntdll included (0x6FFF_FFC0_0000 in `hello.exe__run.log`). Our port maps an image at its preferred base when it is free and otherwise at a deterministic slot in [0x1_0000_0000, 0x1_7000_0000) (`wine-unix.md`, section 4).
  - **To decide in M3's contract, with evidence:** rebase the closure DLLs to unique, packed bases when packaging (from 0x1_8000_0000 up, which keeps the memory near 6 GiB), or translate at the observed, deterministic base (`translate-pe --base`).
  - The observed base depends on each program's load order, which argues for rebasing.
  - The guest memory at boot is not a problem: about 6 GiB, well below 16.
- **Session disk:** the Wine source (hundreds of MB) plus build outputs go under `target/`.
- **Boot cost per program run:** the matrix runs 138 processes. Measure in M3.
- **Out of reach until Phase 4:** test units that need child processes, explorer or services. They are tracked, not fixed.
- **Found by the spike, for M2** (`abi.md`, "Wine boundary"):
  - Emscripten's `sbrk` grows the memory too (`emscripten_resize_heap`), which conflicts with "one owner of `memory.grow`";
  - `run` resets `env.depth` on entry, so a nested loop loses the outer call depth.
- **Segment selectors differ from Windows** (DS, ES, FS, GS read 0, as on Linux and native Wine). Only code that inspects selectors, such as anti-debug or VM checks, can tell.
- **The translated closure is 153.5 MB of wasm.** M6 measures its compressed download and caching. The two test exes (40 MB) are only needed for conformance runs.
- **No CI job translates the closure yet.**
  - CI checks the closure's forms (the census) and their semantics (replay). A translator change that broke one closure DLL's translation would only show in a session.
  - M3 builds the closure's AOT modules in CI, which makes translation a gate there (the M1 spec review).
- **Native Wine's exit status has 8 bits.** So the `wine-11.0-native` baseline counts a main process that crashes after its child processes printed summaries as `failed`, not `crashed` (`wine-build.md`, "Conformance baselines").
  - `wine-4c6ec0d` has no such unit.
  - M5's runner can tell them apart, from Wine's own crash output, if a case appears.
- **M2's, for M3:**
  - the `MEM_TOP_DOWN` ceiling;
  - a write-copy page isn't converted on a checked write, nor on translated code's writes;
  - after a mapping has a slot, a view at its preferred base returns `STATUS_IMAGE_NOT_AT_BASE`;
  - the JS `boot()` doesn't `chdir`; only the test does;
  - `WINEDEBUG` only comes through `opts.env`;
  - the boot set takes all of `nls/`, not `closure.json`'s `data_files`, and file contents load up front;
  - the wineboot skip's version constants;
  - `overlay.reg` is hand-written: `registry_values` checks `Identifier` and `FeatureSet`, not `PROCESSOR_IDENTIFIER`, `PROCESSOR_LEVEL` or `PROCESSOR_REVISION`;
  - the KUSER test skips `XState` and `ActiveProcessorCount`, rather than comparing them with the profile (one processor, no xstate).
- **M2's, for M4:**
  - the server's `process_sigkill` would `kill()` the process itself;
  - every SIGQUIT exits with code 0, the console's CTRL_BREAK included;
  - `get_image_map_address` is called inside the virtual mutex, harmless while server calls never suspend.
- **M2's, tests:**
  - no test yet for `NtCreateUserProcess`'s loud stop, a Wine error rejecting `boot`, or `boot` flags other than 1;
  - numbers that no contract fixes: `section.c`'s slot address and 8 GiB bound, the first TEB block in `virtual.c`, the allocator reusing a block in `link.test.mjs`, `boot.test.mjs`'s hard-coded state offsets and its `/tmp/.wine-0`.
- **M2's, other:**
  - `imports.txt`'s `__syscall_poll` line is unused;
  - `find_free` (a map at no fixed address) tries every 64 KiB start and checks each page, which is quadratic. Wine rarely calls it, since `map_view` picks its own addresses;
  - `boot-set.mjs` never frees `boot`'s argv, a few hundred bytes for a boot that runs once;
  - Emscripten 7.0 caps pipes at 64 KiB, so a partial reply write would then need `main_loop`.
- **Carried from Phase 1** (`phase-1.md`, "Known issues"):
  - the two store-check gaps, which close in M3;
  - Firefox is untested (Phase 3);
  - calls and floating point are slow.

## Next step: M3, "Boot: hello.exe on Wine", in a new session

**M3 is planned and under way, paused on 2026-11-20 with WP1 to WP4 merged.** Its decisions and work packages are under "M3" above, and they supersede what follows where the two differ. Resume from "How to resume" in "M3 progress (paused 2026-11-20)", not from the start below.

**Start:** read `docs/plan.md` ("Phase 2 milestones", M3), this note's M2 section ("Decisions taken when planning M2", "Second resume", "The close"), `ARCHITECTURE.md`, `docs/design/wine-unix.md` (v1 with M2's amendments) and `abi.md` v5. Then do the research pass into `docs/dev/research.md`. M2's retro (`docs/dev/agent-runs.md`) has a lead for it: give the source reading to a subagent, since planning M2 took 27.5M of the coordinator's 112.8M cache reads, at 313k a request.

**M3's scope.** plan.md's done-when: hello.exe and the L2 programs run on Wine in Node, matching native, with both checks on; hello.exe does the same in a Chromium Worker. `wine-unix.md` section 11 adds:
- the typed syscall wrappers;
- the unix-call dispatcher;
- translating the closure;
- the run loop (`tw_run`);
- the dispatcher slots now holding `TW_NO_DISPATCHER`: ntdll's `__wine_syscall_dispatcher` and `__wine_unix_call_dispatcher`, and 0x7ffe1000;
- `boot` modes beyond `BOOT_STOP_AT_ENTRY`;
- `depth` in the green-thread save set (`abi.md`).

**Decisions M3 owes** (see "Known issues and risks"):
- whether to rebase the closure DLLs, decided with the translator's evidence;
- the `MEM_TOP_DOWN` ceiling;
- write-copy semantics for translated writes;
- the host's `chdir` before `boot`, and `WINEDEBUG`;
- narrowing the boot set to `data_files`, and loading file contents on demand;
- KUSER's version for a per-program Windows version;
- one definition of the initial FP state for C and JS, for example from `gen-header`: M2's x87 tag-word bug came from two copies.

**How:** commit the contracts before agents start. Briefs come from M3's work-package table, as in M2: each names its files, its contract sections, its tests and a WIP track, and asks the agent to check its worktree base first.

**Inputs:**
- `tests/targets/wine.lock` (`wine-4c6ec0d`);
- `tests/targets/wine-unix.lock`;
- `tests/wine/prepare.sh` for `target/wine`, which needs `zstd`;
- `native/spike/boundary/build.sh` for `test:browser`.
