# Research log

Each milestone's planning starts with a short research pass over official and community sources for its domain and our workflow. Log one dated entry per pass. Each item gives: the source (with link), the finding, why it matters here, and the decision with the reason.

## 2026-10-31: Claude Code workflow

### Official

Sources:
- https://code.claude.com/docs/en/best-practices
- https://code.claude.com/docs/en/skills
- https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/claude-4-best-practices

| Finding | Relevance | Decision |
|---|---|---|
| Keep CLAUDE.md short | Ours had grown to about 1,500 words, mostly commands | Trimmed to about 875 words; commands moved to `docs/dev/commands.md`. Adopted |
| Path-scoped rules (`.claude/rules/` with `paths:`) | Area guidance only matters in that area | `translator.md`, `oracle.md`, `runtime.md`. Adopted |
| Procedures as skills | We ran pause, milestone close and divergence hunts by hand | `/pause`, `/milestone-close`, `/diverge`. Adopted |
| Never-rules as deny rules or hooks | Prose rules get forgotten | Deny rules in `.claude/settings.json` (fixtures, `playwright install`, `service docker start`, force push). Adopted |
| Fixed agent instructions live in agent definitions | Every brief repeated the same rules | Added to `deep-worker.md` and `builder.md`. Adopted |
| Anti-overengineering and anti-test-fitting guidance | Matches "no scope creep" and "general solutions" | In the agent rules. Adopted |
| Review by a fresh agent against the spec, skipping style | Catches spec drift the author misses | Two-stage review in `/milestone-close`. Adopted |
| State files and git as memory for long tasks | Sessions don't share memory | Already practice (`docs/status/`, WIP commits); kept |
| Just-in-time context; subagents return condensed summaries | Keeps the main context small | Already policy ("Working efficiently"); kept |

Also adopted: `ARCHITECTURE.md` (below).

### Community

| Source | Finding | Decision |
|---|---|---|
| https://github.com/obra/superpowers | Brainstorm, plan, a fresh subagent per task with two-stage review; systematic debugging; verification before done | Adopted as tailored skills (`/milestone-close`, `/diverge`), not installed |
| https://github.com/hesreallyhim/awesome-claude-code | Curated index of skills, hooks and workflows | The index to watch at each milestone's research pass |
| Practitioner cost reports | Subagents cost 2-3x, agent teams 4-15x; set small models, keep context-heavy work in the main loop, use read-only reviewers | Kept as policy ("Delegation and effort") |
| https://matklad.github.io/2021/02/06/ARCHITECTURE.md.html | A short map (overview, codemap, invariants, cross-cutting concerns) orients newcomers | `ARCHITECTURE.md` adopted |

### Not adopted yet

- TypeScript checking for `runtime/` (`// @ts-check` plus `tsc`): later, at M6 close-out, when the runtime's shape settles.
- More network domains: not needed; the allowed list covers current work.

## 2026-11-04: Claude Code workflow digest

Proposals only. Items 1-2 rest on undated Claude 5 guides: new to this log (its last entry cites the Claude 4 guide), maybe older than the window (2026-10-31 to 11-04). Item 3 is from the window.

### 1. Hard caps instead of prose
- Source: [Opus 5 guide](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/prompting-claude-opus-5#controlling-subagent-spawning), [changelog 2.1.316](https://code.claude.com/docs/en/changelog), [week 37](https://code.claude.com/docs/en/whats-new/2026-w37.md), [HN thread](https://news.ycombinator.com/item?id=50284255).
- Says: Opus 5 "delegates to subagents more readily"; Claude Code can cap subagent concurrency and depth; `CLAUDE_CODE_SUBAGENT_EFFORT` sets effort for agents naming none; `maxEffortLevel` caps effort.
- Why: our limits are prose ("never max effort"). The HN thread (anecdotal, unchecked) says background-agent summaries take a large share of spend.
- Proposal: `.claude/settings.json` `env`: `CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS=3`, `CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH=1`, `CLAUDE_CODE_SUBAGENT_EFFORT=medium`; plus `"maxEffortLevel": "xhigh"`. Confirm semantics on the [SDK page](https://code.claude.com/docs/en/agent-sdk/subagents#cap-subagent-depth-concurrency-and-spend) first (not read).
- Benefit/cost: a ceiling on fan-out, the fastest way to burn plan usage; four lines, overridable per session.

### 2. Effort sweep for deep-worker
- Source: [Effort](https://platform.claude.com/docs/en/build-with-claude/effort), [Opus 5.5 guide](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/prompting-claude-opus-5-5#calibrate-effort).
- Says: Opus 5.5 defaults to `medium` and "matches or exceeds" Opus 5 at `high` on coding, with fewer tokens; sweep, and reserve `xhigh`/`max` for measured gains. Sonnet 5.5 advises `medium` for well-specified agentic coding; Haiku 5.5 warns `low` can skip searches in long prompts.
- Why: `deep-worker` (`opus`, `high`) is our costliest agent; the `builder` and `Explore` pins already fit.
- Proposal: pilot `effort: medium` in `.claude/agents/deep-worker.md` on one known-hard `/diverge` case; compare `/usage` tokens and test results with `high`. Only if `opus` resolves to 5.5 (check `/model`).
- Benefit/cost: fewer thinking tokens per turn; two runs; "retry one tier up" bounds the risk.

### 3. Two harness changes
- Source: [changelog 2.1.317 and 2.1.318](https://code.claude.com/docs/en/changelog).
- Says: re-reading an unchanged file now returns it whole (the stub is gone); background subagents move shell commands to the background after 3 minutes.
- Why: re-reads cost full price; cargo and 7-Zip suites may be backgrounded.
- Proposal: add to the fixed rules in `.claude/agents/builder.md` and `deep-worker.md`: "Read a file once; re-read only after a change or compaction. Commands past 3 minutes go to the background: wait; never re-run or report done meanwhile."
- Benefit/cost: fewer duplicate reads and runs; about 30 words per agent.

**Checked, nothing new:** engineering blog (newest post 2026-10-13, unrelated); awesome-claude-code mirrors; Reddit (blocked). Keep `/milestone-close`: a fresh-agent diff review is the writer-verifier pattern the Opus 5 guide calls effective.

### Decisions (2026-11-05; the owner asked the coordinator to decide, with a cheap way to measure)
- **Item 1:**
  - Adopted: at most 3 subagents at once, and no nested spawning.
  - Adopted, at `high` rather than the proposed `medium`: the default effort for subagents that name none (Plan, general-purpose reviewers). They do judgment work, and `high` is still well below the session's `max`.
  - Not adopted: `maxEffortLevel`, since it would also cap the main session.
  - The docs confirmed the semantics: the SDK page "Cap subagent depth, concurrency, and spend"; the changelog for 2.1.316 and 2.1.317.
- **Item 2:** a pilot in M6. deep-worker runs at `medium` for WP3a and WP4, and at `high` for WP2 and WP3b. Decided at the close-out.
- **Item 3:** adopted in the builder and deep-worker fixed rules (changelog 2.1.317 and 2.1.318).
- **Measurement:** `docs/dev/agent-runs.md` and `docs/dev/agent-runs.jsonl`, a ledger of the token figures the harness already reports, plus rework and review findings as the quality signals.

## 2026-11-04: M6, translator speed

The M6 planning pass, with 4 searches. Several hosts are blocked here (github.com pages of other repos, spidermonkey.dev), so some items rest on search summaries.

### Sources
- **v86's design notes** ("how it works"; search summary). Its wasm JIT emits deep `br_table`s with locals live across the whole function, which browsers handle poorly, and memory use climbs. Our backward branches go through a function-wide `br_table` in the same way. Decision: WP3, loop structuring.
- **QEMU TCG** (`tcg/README`; search summary).
  - Liveness runs per basic block and removes dead condition-code computations.
  - Globals are stored before a helper call unless the helper is marked as not reading or writing them, and reloaded only if it may write them.

  Decision: WP2 stores only dirty values and reloads only live ones, with MOD summaries for direct callees.
- **SpiderMonkey, "Is Memory64 actually worth using?"** (spidermonkey.dev, 2025-01-15; search summary).
  - Memory64 can't use guard pages there, so every access pays a bounds check.
  - V8 enables memory64 trap handling by default since 13.6 (`tests/bench/memory/README.md`), so Node and Chrome don't pay it.

  Decision: it matters once Firefox is a target. M6 measures only our own NULL and store checks.
- **LLVM's WebAssembly backend** (CFGSort, CFGStackify, FixIrreducibleControlFlow) **and Binaryen's Relooper** (search summaries). These are the standard ways to turn a CFG into wasm loops and blocks; irreducible flow keeps a dispatch block. Decision: WP3 follows CFGSort and CFGStackify, with a selector local for entries into a loop's interior.

### Not adopted
- **XMM caching in locals:** neither benchmark is floating point. Revisit if profiles show XMM traffic.
- **USE summaries for callees:** they would only save the caller's stores, and they would need cold-path stores when a callee unwinds.

## 2026-11-06: mattpocock/skills (a Claude Code skill library)

**Source:** github.com/mattpocock/skills (MIT; README and seven SKILL.md files, read as single files through raw.githubusercontent.com). Asked by the owner.

**Leads that match problems we have.** They are trials, judged by `docs/dev/agent-runs.md`:
- **`writing-for-agents`:** always-loaded lines cost on every turn. Prefer pointers, positive targets and pruning: no duplication, nothing the environment already shows, no instructions the model follows anyway. Trial 1.
- **`retro`:** improvements by severity after a session. A mechanical mistake gets a deterministic check; a judgement call becomes a reviewer rule, since the reviewer has the least context pressure. Trial 2.
- **`handoff`:** a hand-off note that references artifacts instead of repeating them. Trial 3.
- **`pr`:** a merge-danger call (one-way or two-way door, blast radius). Trial 4.
- **`diagnosing-bugs`:** phases with completion criteria. The first is a tight command that goes red on the bug, before any hypothesis. Trial 5.

**Not adopted:**
- `grill-me` and `grill-with-docs`: plan mode and the coordinator's questions cover alignment. A glossary may help briefs later, if briefs show terminology drift.
- `to-spec`, `to-tickets`, `triage` and `wayfinder`: issue-tracker flows, while we plan in `docs/plan.md`.
- `implement-spec`: maximum concurrency, against our caps.
- `code-review`: our two-stage review already covers standards and spec; running both at once would save time, not tokens.
- `research`: we have this log and the digest.
- `prototype`, `wizard`, `teach`, `to-questionnaire`, `wait-what`, `ask-matt` and the setup skill.

## 2026-11-06: Phase 2, Wine underneath

The Phase 2 planning pass: 6 searches, plus reads of single files streamed from the wine-11.0 tarball (`curl … | tar -xJO wine-11.0/<path>`). gitlab.winehq.org answers automated fetches with an Anubis block page, so the tarball is the way to read Wine's source. Phase 0's analysis (`native/README.md`) still holds; these are the additions.

### Official
- **Wine 11.0 release** (January 2026; release coverage on linuxiac and Phoronix).
  - New WoW64 is fully supported and there is one unified loader. NTSync needs Linux 6.14's driver.
  - Decision: we build x86_64 only (`--enable-archs=x86_64`). NTSync doesn't apply.
- **The wine-11.0 tarball** (dl.winehq.org).
  - x86_64 image bases: ntdll 0x1_7000_0000, kernelbase 0x1_7400_0000, kernel32 0x1_7800_0000 (`dlls/*/Makefile.in`). They lie in the guest high region. Other DLLs take the linker default, so M1 lists the closure's bases and overlaps.
  - The first process without a parent always runs `wineboot.exe --init` as a child process (`dlls/ntdll/unix/env.c`, `build_initial_params` → `run_wineboot`), unless `\KernelObjects\__wineboot_event` already exists. Decision: Phase 2 has no child processes, so a patch skips it. The prefix comes from native wineboot in CI, plus a generated overlay for wineboot's per-boot keys.
  - `ConsoleHandle` becomes `CONSOLE_HANDLE_SHELL` only when fd 0, 1 or 2 is a tty (`env.c` L1352-1370). Only then does kernelbase start `conhost.exe` (`kernelbase/console.c` L2356). Decision: fds 0-2 are non-tty byte devices, so there is no conhost.
  - The kernel32 and ntdll tests import user32 and advapi32. Decision: win32u's Unix side must load (M4), with the null driver.
  - There are 33 kernel32 and 26 ntdll test units. Each prints `%04x:%s:%s %d tests executed (%d marked as todo, %d as flaky, %d failures), %d skipped.` (`include/wine/test.h` L791). Decision: the conformance runner parses that line per unit.
  - Unix side size: ntdll/unix is 45.8k lines, plus `signal_x86_64.c` (3.5k), which we replace. The server is 50.2k and win32u 105.7k. None of them uses `__thread`. Decision: green threads need no TLS emulation.
  - The PE side's `RtlCaptureContext` uses `pushfq` and `fxsave`. Decision: M1's census expects new forms like these.
  - The server maps an image at its preferred base and moves it only on a conflict, then keeps that address for the file (`server/mapping.c`, `get_image_map_address`). Decision: AOT is keyed by the base actually used, which is deterministic.
- **Emscripten 6.0.8** (2026-08-20; changelog mirror): JSPI is no longer experimental. Our pinned emsdk is 6.0.12. Decision: Phase 2 relies on JSPI through Emscripten-compiled frames, and M1's spike proves it.
- **Firefox JSPI**: Mozilla's intent to ship (2026-06-10, via a secondary source) enables it by default in Firefox 153. Chrome has shipped it since 137. Decision: Phase 2 targets Chromium; Firefox is measured in Phase 3.

### Community
- **Hacker News** (searched: Wine, WebAssembly, browser): no port of Wine's Unix side to Emscripten turned up. The nearest projects emulate a whole Linux. Boxedwine is off-limits under the clean-room rule, and we read nothing of its code.

### Review
- **A Plan-agent review of the draft** found six blockers. All are folded into the plan (`docs/status/phase-2.md`):
  - JS frames between `promising` and `Suspending`, so the Unix side owns the run loop;
  - the green-thread save set;
  - fds passed in-process must be duplicated;
  - symbol collisions between the server and ntdll;
  - the server's page size and DLL range;
  - volatile registry keys.

### Not adopted
- **A wineserver in its own Worker with a SharedArrayBuffer channel** (Phase 0's risk 6). With one Windows process per run it is pure cost, and Phase 4 needs its cross-worker form anyway.
- **Emscripten pthreads for Wine's threads:** each would be a Worker, with no shared table. Phase 2 keeps cooperative threads; parallel threads are Phase 4.

## 2026-11-13: Phase 2 M2, Wine's Unix side

The M2 planning pass, in a fresh session. Sources:
- the wine-11.0 tarball (dl.winehq.org), unpacked in the scratchpad: `dlls/ntdll/unix/`, `server/`, `include/`, `programs/wineboot/`;
- the `wine-4c6ec0d` release: the `+loaddll` traces and `wine-prefix.path`;
- Emscripten 6.0.12's own source, inside the pinned `emscripten/emsdk:6.0.12` image (`tools/system_libs.py`, `src/lib/*.js`, `system/lib/`);
- Emscripten's ChangeLog through raw.githubusercontent.com (newreleases.io is not reachable from a session);
- searches on wasm exception handling in Node and Chrome, wasm-ld `--wrap`, community ports of Wine to Emscripten, and the Claude Code changelog.

### Emscripten 6.0.12
- **Releases since the pin:** 6.0.13 (2026-10-19) and 7.0.0 (2026-11-06) change no default we use. 7.0.0 caps a pipe's buffer at 64 KiB.
  - Decision: stay on 6.0.12, which the spike proved. Revisit at M6. If we move to 7.x, the server's partial-write path for a large reply would need `main_loop` (`wine-unix.md`, "Risks").
- **`-sSHARED_MEMORY` without `-pthread` links the `-ww` libc variant.** That variant leaves out both musl's `thread/` directory and `library_pthread_stub.c` (`tools/system_libs.py` L705-737 and L1238-1270), so every `pthread_*` function Wine calls is undefined at link time.
  - Decision: the single-thread part of our green pthread layer moves from M4 into M2's build skeleton.
- **`-sALLOW_MEMORY_GROWTH` must stay.** Without it, `updateMemoryViews` asserts that it runs only once when `ASSERTIONS` is on, and `emscripten_resize_heap` aborts (`src/runtime_common.js` L156, `src/lib/libcore.js` L252).
  - Decision: keep the spike's flag. The glue's views then refresh themselves, and `sbrk` is capped by `--wrap` instead (below).
- **The layout with `-sGLOBAL_BASE`** is `--no-stack-first`: static data, then the stack (64 KiB by default), then the heap. The stack grows down into Wine's globals.
  - Decision: `-sSTACK_SIZE=1MB` and `-sSTACK_OVERFLOW_CHECK=2`.
- **`sbrk` grows the memory** (through `emscripten_resize_heap`) only when the break passes the memory's current size. Once the virtual backend has grown the memory past 4 GiB, a later `malloc` would take guest pages above the host region. wasm-ld supports `--wrap=<symbol>`.
  - Decision: `-sINITIAL_MEMORY=4GB`, and `-Wl,--wrap=sbrk` with a wrapper that refuses to move the break past 0x1_0000_0000. The heap never needs to grow the memory, so the virtual backend is the only grower. This settles the spike's first open item.
- **`environ`** is built by a libc constructor during instantiation, from `ENV`.
  - Decision: the JS host fills `ENV` in `preRun`.
- **Wasm exceptions:** the legacy encoding is still Emscripten's default. exnref needs Chrome 137 and is behind a flag in Node 24.
  - Decision: keep the legacy encoding.

### Wine 11.0
- **Where native Wine maps modules** (`hello.exe__run.log`, `+loaddll`):
  - hello.exe at its preferred base, 0x1_4000_0000;
  - ntdll at 0x6FFF_FFC0_0000, kernelbase at 0x6FFF_FF54_0000, kernel32 at 0x6FFF_FFA2_0000, msvcrt at 0x6FFF_FF06_0000.

  Every DYNAMIC_BASE DLL that has relocations is marked `ImageDynamicallyRelocated` by the server (`server/mapping.c` ~L942). The client then asks the server for a slot (`dlls/ntdll/unix/virtual.c` L3403, `get_image_map_address`), and the server assigns one top-down from its ASLR range (`assign_map_address`, L1273). **This corrects** M1's note that Wine keeps the first DLL at 0x1_8000_0000 and the Phase 2 pass's "maps an image at its preferred base and moves it only on a conflict" (2026-11-06 above), which holds only for images without that flag.
  - Decision: our port follows `guest-memory.md` instead. An image maps at its preferred base when that range is free, and otherwise gets a deterministic slot from the server's range, now [0x1_0000_0000, 0x1_7000_0000) top-down (`wine-unix.md`, "Address map and image placement").
- **The boot path** (`dlls/ntdll/unix/loader.c`): `__wine_main` (L2278) → `init_paths` (L449, `dladdr`) → `virtual_init` → `init_environment` (`env.c` L824: `locale.nls`, `l_intl.nls`) → `start_main_thread` (L1908).
  - `start_main_thread` runs `virtual_alloc_first_teb` (which commits KUSER), `signal_init_threading`, `dbg_init`, `server_init_process`, `virtual_map_user_shared_data`, `init_cpu_info`, `init_files`, `init_startup_info` (→ `build_initial_params` → `load_main_exe` maps the exe, and `run_wineboot`), `init_thread_stack`, `NtCreateKeyedEvent`, `load_ntdll`, `load_apiset_dll`, `server_init_process_done` and `signal_start_thread`.
  - Decision: our `boot` export runs this path unchanged up to `signal_start_thread`, with patches only where a step needs the host (paths, KUSER, wineboot, the server).
- **The x86-64 thread start** (`signal_x86_64.c`, `init_syscall_frame`, ~L2893):
  - a CONTEXT at `((StackBase − 0x28) & ~15) − sizeof(CONTEXT)`, with Rip = `RtlUserThreadStart`, Rcx = the entry, Rdx = the PEB, Rsp = StackBase − 0x28, EFlags 0x200, the FPU control word 0x27f and MXCSR 0x1f80;
  - the syscall frame, with RIP = `LdrInitializeThunk`, RCX = the CONTEXT, RSP = the CONTEXT − 8, and GS = the TEB.
  - Decision: M2's boot builds exactly this state in the main thread's state block and stops there.
- **apisetschema is data:** `load_apiset_dll` maps the file `SEC_COMMIT` at any free address and sets `PEB->ApiSetMap`. It is never mapped as an image.
  - Decision: the done-when says "apisetschema's API set mapped (`PEB->ApiSetMap`)", in `plan.md` and the status note.
- **wineboot's per-boot work** (`programs/wineboot/wineboot.c`): `create_user_shared_data` (L319) writes NtBuildNumber, NtProductType, ProductTypeIsValid, NtMajorVersion, NtMinorVersion, SuiteMask and NtSystemRoot into KUSER. It also creates the hardware, dynamic and computer-name keys and the volatile environment, sets up KnownDlls and starts services.exe. `virtual_init_user_shared_data` (`virtual.c` L4422: SystemCall = 1 and the CPU fields) is called only from `run_wineboot`.
  - Decision: the wineboot skip does the KUSER part in-process; the keys come from the overlay; KnownDlls and services are M4.
- **KUSER in a single address space:** the client maps the server's section with `mmap(MAP_FIXED)` (`virtual_map_user_shared_data`, L4393). Emscripten refuses that with EINVAL, and Wine then calls `exit(1)` (L4407). `virtual_init_user_shared_data` writes through a second mapping (L4438). The server sets its own pointer to the section at `server/mapping.c` L1502.
  - Decision: one page at 0x7ffe0000, committed by the first TEB allocation, which both sides write directly.
- **The transport:**
  - the client writes a request to `request_fd` and reads the reply from `reply_fd`, and waits on `wait_fd` (`dlls/ntdll/unix/server.c`: `send_request` L183, `wait_reply` L276, `server_call_unlocked` L288, `wait_select_reply` L351);
  - the server runs `read_request` → `call_req_handler` → `send_reply` (`server/request.c` L328, L292, L250);
  - a new client comes from `master_socket_poll_event` (L543): `create_process`, then `create_thread(-1, process, NULL)`, which makes the request pipe and passes its fd with `send_client_fd`; the client's `init_thread` request passes two more fds;
  - fds the server polls need `main_loop` (`server/fd.c` L2241);
  - `get_next_timeout` is at most 16 ms, since the KUSER time is refreshed on a timer (`fd.c` L898, `set_user_shared_data_time` L380), and the registry is saved every 30 s.
  - Decision: keep Wine's pipes and fd bookkeeping; the client calls the server's `read_request` synchronously after each `send_request`; waits block through `green_block`; the scheduler ignores the KUSER tick and the registry save when it looks for the next timeout.
- **The server's cwd and paths:** the server changes the process's cwd with `fchdir` and returns to `server_dir_fd` afterwards. `get_initial_directory` reads `getcwd()` (`env.c` L1403). The server finds `l_intl.nls` through `server_argv0` and `DATADIR`.
  - Decision: `boot` records the launch directory before the server starts, and `get_initial_directory` uses it; the server loads `l_intl.nls` from a constant path.
- **CPU and processors:** `system.c`'s x86 code runs the host's `cpuid`, `xgetbv` and `fxsave`, and `sysconf` reports the host's cores.
  - Decision: the CPU information comes from the translator's CPUID table, emitted into a C header by `x64wasm-cli gen-header`, and `NumberOfProcessors` is 1.
- **The prefix** was made at `/home/runner/wine-prefix` for the user `runner` (`wine-prefix.path`). Its registry holds that path.
  - Decision: the Unix side runs with the same `WINEPREFIX`, `HOME` and `USER`.
- **The porting surface:** 27 `__x86_64__` sites in ntdll's Unix part and the server; 3 `__TRY` sites, all in `virtual.c`; the missing system headers belong to other platforms or are `HAVE_`-guarded Linux ones; the blocking calls are `select` and `usleep` sleeps, `sched_yield`, `futex_wait`, `poll` and the server waits (`sync.c` `NtDelayExecution` sleeps with `select`).

### Community
- **No port of Wine's Unix side to Emscripten turned up.** Boxedwine is off-limits under the clean-room rule, and we read nothing of it. "Wine-Assembly" is a WAT interpreter for Windows 98 programs and doesn't apply.
- **Claude Code 2.1.319-2.1.323** adds `CLAUDE_CODE_DISABLE_AGENT_PROGRESS_SUMMARIES` and `denyInsteadOfPrompting`.
  - Leads only: no evidence ties either to a cost of ours. Not adopted.

### Review
- **A Plan-agent review of the draft** found 2 blockers and 9 should-fix items, all folded into the contracts:
  - the blockers: the `-ww` libc has no pthread stubs; dropping `ALLOW_MEMORY_GROWTH` trips `updateMemoryViews`' run-once assertion;
  - the should-fix items: ntdll is `ImageDynamicallyRelocated`, so image placement is M2's; apisetschema is data; KUSER needs patches, not a mapping; CPU information from the translator's table; `getcwd` after the server's `fchdir`; `ENV` in `preRun`; the deadlock check ignores the KUSER tick and the registry save; `STACK_OVERFLOW_CHECK`; file ownership between work packages; write-back of writable shared views.

### Workflow
- **The context of this session:** research and planning filled 483,687 of 1,000,000 tokens before any work began (`get_session`, `context_usage`), and every later step re-reads that context. Auto-compaction would only start at 901,600.
  - Decision: the owner's change, trial 6 in `agent-runs.md`: context checkpoints with requested compactions.

### Not adopted
- **A socket or a separate Worker for the server:** one Windows process per run makes either pure cost (2026-11-06's decision holds).
- **Rebasing the closure DLLs in M2:** M3 decides it with the translator's evidence.

## 2026-11-14: claude-mem (persistent memory for Claude Code)

**Sources:**
- github.com/thedotmack/claude-mem, v15.18.3 (Apache-2.0). Its README was read as a single file through raw.githubusercontent.com.
- A search for users' reports.

Asked by the owner.

**What it is:**
- **Capture:** hooks on session start, prompts, tool use, subagent stop and session end feed a local worker (Bun). An "observer" model turns the tool activity into observations, and writes a summary when a session ends.
- **Storage and recall:** the observations go to `~/.claude-mem` (SQLite, plus Chroma through uv). From the second session in a project, each session starts with an index of them, with details on demand.
- **Where the observer runs:**
  - the maker's hosted service, with a 30-day trial and then a subscription;
  - an OpenRouter or Gemini key;
  - or the user's Anthropic plan.
- **Sharing and telemetry:** Cloud Sync to cmem.ai is opt-in. Anonymous telemetry is on by default.

**Not adopted:**
- **The benefit doesn't reach our sessions.** Each milestone runs in a fresh cloud container, and `~/.claude-mem` goes with it. Carrying memory over needs Cloud Sync, a third-party service that would receive our sessions' contents, and our code is all rights reserved.
- **It duplicates the hand-off.** Sessions don't share memory by design: decisions live in reviewed, versioned files (the status note, the contracts with their source lines, this log and the ledger). Unreviewed summaries would compete with them. M2's three wrong statements about Wine were corrected in the contract; an observer would have kept the wrong versions too.
- **It adds cost where our limit binds.** Its hooks see all our tool activity, the agents' included (about 1,300 requests in M2), and its observer model processes it.
  - On our plan, it spends the same 5-hour window that stopped M2 twice. The other options are paid services.
  - The one user report found 5.5k-42k tokens of overhead a session; its own figures disagree.
  - The "10x" claims compare against dumping the full context, not against working without it.
- **Our measured costs are inside a session,** and it doesn't touch them: the coordinator's context (120M cache reads in M2) and the research pass's source reading. Trial 6 and its research-pass lead do.
- **Setup:** it installs Bun and uv on first run. In our cloud environment that likely needs a setup script and new network hosts.

**What would change this:** a close that traces rework to lost context (trial 6's measure, 0 so far), or the ledger showing a milestone re-reading the sources the previous one read. Then run a one-milestone trial with telemetry off and the observer on our plan, measured against M2's usage per session.
