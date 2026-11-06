# Phase 2 status: Wine underneath

**Phase 2 is planned (2026-11-06). M1 is in progress and paused; the resume path is under "M1 progress (paused 2026-11-06)".** The milestones are in `docs/plan.md`, "Phase 2 milestones", and the research pass is in `docs/dev/research.md`, "2026-11-06: Phase 2, Wine underneath". From Phase 2 on, a session covers one milestone.

- **Session 1 (2026-11-06):**
  - planned Phase 2: three Explore runs, then a Plan-agent review of the draft, whose six blockers are folded in below;
  - the owner decided on one session per milestone and on a basic raise path in Phase 2;
  - M1 follows.

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
  - `dladdr`-based path discovery is replaced by constant paths: `/wine/lib/wine/x86_64-windows`, `/wine/share/wine` (NLS, `wine.inf`), and `$WINEPREFIX=/prefix`.
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
    - ntdll.dll, apisetschema and hello.exe mapped at their bases;
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

## M1 progress (paused 2026-11-06)

The owner paused the session at 23:00 UTC. Everything is committed on the PR branch `claude/gifted-bardeen-a0hot6` (head `a91444e`, plus this note's commit). No pull request is open yet, and the working tree is clean.

### Committed
- **Planning and contracts:**
  - `4eb7d6f`: the plan, this note, research, workflow;
  - `53e166c`: `wine-build.md`, the closure in `instruction-scope.md`, the draft "Wine boundary" in `abi.md`.
- **WP3, the boundary spike: done.**
  - `0182d2f`: CI run 42306776103 passed all three jobs;
  - its results are under "What works" below.
- **WP1, the Wine CI build: committed, but its first run is not finished.**
  - `7cd714a` contains:
    - `.github/workflows/wine.yml`, with the jobs source, build, baseline-windows, baseline-wine, closure and publish;
    - `tests/targets/sources.json`'s `"wine"` pin (sha256 `c07a6857…0701`; sha512 checked against `sha512sums.asc`);
    - `tests/targets/wine-build-info.sh`;
    - `testkit closure` (`crates/testkit/src/closure.rs`);
    - delay-load imports in `x64wasm::pe`, with tests;
    - `tests/conformance/{parse,run-baseline}.mjs` and `tests/node/conformance-parse.test.mjs`.
  - Its first `wine` run, **42306869519** at `7cd714a`:
    - `source` passed;
    - `build` was in its Build step at the pause, and CI keeps running without the session.
  - `targets` run 42306869181 passed.
- **Also committed:** `18b58c6`, the command reference; `a91444e`, the spike's results.

### In progress, and how to resume (for a fresh session)
1. **Read** `docs/plan.md` ("Phase 2 milestones"), then this note's "Decisions" and this section.
2. **WP1, finish it** (builder; the contract is `docs/design/wine-build.md`, its deliverables are below).
   1. Look up run 42306869519 (`mcp__github__actions_list` / `actions_get`, or `gh run view 42306869519`).
   2. If a job failed, read that job's log tail (save it to a file and grep it), fix the cause in `wine.yml` or the scripts, and push. Each Wine build takes about an hour; ccache shortens later ones.
   3. When `wine-<sha7>` is published with every asset in `wine-build.md`'s table, write `tests/targets/wine.lock` with that tag and check that `scripts/fetch-artifact.sh "$(cat tests/targets/wine.lock)" closure.json` works.
   4. Verify locally: `cargo test -p testkit`, `cargo test -p x64wasm`, `npm run test:node` and `cargo clippy --workspace --all-targets -- -D warnings`. They were not confirmed before the pause.
   5. Add to `docs/dev/commands.md`: the fetch command, running the native Wine 11 from `wine-native.tar.zst` in a session, and `testkit closure`.
   6. Record here the build minutes, the closure's module count, its overlaps, `highest_end` and `memory_bytes`, and the baseline units per platform and exe (ok, failed, crashed, timeout).
   - **WP1's deliverables:**
     - `wine.yml` per `wine-build.md`: the source mirror `src-wine-11.0`; the build with the exact configure flags and `--strip-debug` PE packing; the prefix; `build-info.json`; the two baseline jobs (`--list`, one unit per process, `WINETEST_DEBUG=1`, `WINETEST_PLATFORM=wine` under Wine, 120 s limit); `+loaddll` traces of the program roots and every unit; `closure.json`; the publish job;
     - `tests/targets/wine.lock`;
     - the conformance parser and runner.
   - **It is done when** the run is green, the release exists, the lock names it, and the local checks pass.
3. **WP2, after WP1's release** (deep-worker):
   - run `testkit census` over the closure's modules (listed in `closure.json`) with the profile;
   - write fixture generators for the new forms (expected: `fxsave` and `pushfq` from `RtlCaptureContext`), record them with `testkit record` on this x86-64 machine, and lower them in the translator;
   - add the closure to CI's `census --check` (ci.yml, with the closure fetched through `wine.lock`);
   - `translate-pe` every closure DLL and measure V8 compile sizes and times (`tests/bench/translator/images.mjs` is the pattern);
   - write the base and overlap table here.
   - Exclusions follow `instruction-scope.md`'s Phase 2 section.
   - It is done when `census --check` passes on the closure (replay 100 %) and every closure DLL translates and compiles in V8.
4. **Then close M1:** `/milestone-close`, the draft PR "[Phase 2] Wine in CI, and the boundary spike", the retro and the hand-off to M2.

Ledger: WP1's and the main session's lines at the pause are in `docs/dev/agent-runs.jsonl` (milestone `P2-M1`).

## What works

| Item | Evidence |
|---|---|
| **The Wine boundary** (M1 WP3, `native/spike/boundary/`, `abi.md` "Wine boundary").<br>An Emscripten wasm64 module (imported shared memory64, `GLOBAL_BASE` 2 GiB, wasm EH, no pthreads, no `-sJSPI`):<br>• owns the run loop over translated code<br>• is entered through raw exports in `env.table`<br>• runs two green threads that suspend through `green_block` and get their save set back<br>• makes a nested guest call<br>• grows memory while JS, its glue and translated code all see the growth | `tests/node/wine-boundary.test.mjs` and `tests/browser/wine-boundary.spec.mjs` (a Chromium Worker) pass locally and in CI (run 42306776103 at `0182d2f`). A build without the save-set restores fails the checks. A JS frame between `promising` and `Suspending` throws `SuspendError: trying to suspend JS frames`: Node 24.22 first runs the import and drops its Promise, while Chromium 153 refuses before calling it. |

## Measurements

- **The boundary spike** (this container, Node 24.22 and Chromium 153 headless):
  - creating the shared memory64 of 2 GiB + 64 MiB takes 0.3-1.4 ms, since the pages are reserved, not committed;
  - compiling and instantiating the Unix side through Emscripten's glue takes 2.4-4 ms;
  - a `green_block` suspend and resume round trip takes 1.2 µs in Node and 0.2 µs in Chromium (10,000 rounds).

## Known issues and risks
- **The Unix-side port is the bulk:** about 100k lines in M2-M3, and 106k more with win32u in M4. The hard parts are the machine layer, the virtual backend and the server transport (Phase 0's risks 2, 4 and 6 in `native/README.md`).
- **Wine's DLLs may need many new instruction forms,** which would lengthen M1.
- **High default DLL bases** would push the guest memory toward 16 GiB at boot. M1's base table decides whether M3 relocates them low.
- **The CI Wine build** takes about 40-90 minutes per run; ccache helps.
- **Session disk:** the Wine source (hundreds of MB) plus build outputs go under `target/`.
- **Boot cost per program run:** the matrix runs 138 processes. Measure in M3.
- **Out of reach until Phase 4:** test units that need child processes, explorer or services. They are tracked, not fixed.
- **Found by the spike, for M2** (`abi.md`, "Wine boundary"):
  - Emscripten's `sbrk` grows the memory too (`emscripten_resize_heap`), which conflicts with "one owner of `memory.grow`";
  - `run` resets `env.depth` on entry, so a nested loop loses the outer call depth.
- **Carried from Phase 1** (`phase-1.md`, "Known issues"):
  - the two store-check gaps, which close in M3;
  - Firefox is untested (Phase 3);
  - calls and floating point are slow.

## Next step

Resume M1 from "M1 progress (paused 2026-11-06)":
1. finish WP1, whose first `wine` run is 42306869519;
2. then WP2;
3. then close M1.
