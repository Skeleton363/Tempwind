# Phase 2 status: Wine underneath

**Phase 2 is planned (2026-11-06). M1, "Wine in CI, and the boundary spike", is done (2026-11-13, merged as skeleton363/tempwind#10 and #11). M2, "Wine's Unix side boots", is done (2026-11-14; the draft PR is pending). M3, "Boot: hello.exe on Wine", is next.**
- M1: WP1 (the Wine CI build), WP2 (the census, fixtures and translation of Wine's DLL closure) and WP3 (the boundary spike) are done.
- M2: WP0 to WP5 are merged and the close is under "M2" below. Its PR is "[Phase 2] Wine's Unix side boots" (draft, pending).

The milestones are in `docs/plan.md`, "Phase 2 milestones", and the research pass is in `docs/dev/research.md`, "2026-11-06: Phase 2, Wine underneath". A new session starts only with a new milestone (the owner, 2026-11-07).

- **Session 1 (2026-11-06 and 07):**
  - planned Phase 2: three Explore runs, then a Plan-agent review of the draft, whose six blockers are folded in below;
  - the owner decided on one session per milestone and on a basic raise path in Phase 2;
  - M1: the spike, then the Wine CI build. Paused overnight, resumed in the same session.
- **The same session, resumed 2026-11-13:** WP2's review follow-up, then the M1 close.
- **Session 2 (from 2026-11-13), M2 only:** the research pass and the plan (three Explore runs and a Plan-agent review), a compaction at the plan's approval (trial 6), then the contracts and the work packages.

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
    - A checked write to a write-copy page doesn't turn it into a written page, as the fault would. Neither do translated code's writes (M3).
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

**Start:** read `docs/plan.md` ("Phase 2 milestones", M3), this note's M2 section ("Decisions taken when planning M2", "Second resume", "The close"), `ARCHITECTURE.md`, `docs/design/wine-unix.md` (v1 with M2's amendments) and `abi.md` v5. Then do the research pass into `docs/dev/research.md`.

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
- KUSER's version for a per-program Windows version.

**How:** commit the contracts before agents start. Briefs come from M3's work-package table, as in M2: each names its files, its contract sections, its tests and a WIP track, and asks the agent to check its worktree base first.

**Inputs:**
- `tests/targets/wine.lock` (`wine-4c6ec0d`);
- `tests/targets/wine-unix.lock`;
- `tests/wine/prepare.sh` for `target/wine`, which needs `zstd`;
- `native/spike/boundary/build.sh` for `test:browser`.
