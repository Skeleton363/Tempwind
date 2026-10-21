# Phase 0 status: Foundations

- **Draft PR:** https://github.com/Skeleton363/Tempwind/pull/1 ("[Phase 0] Foundations"), branch `claude/epic-hypatia-9y2839`.
- **State:** all ten Phase 0 items are implemented, and CI is green on Linux and Windows.
- **Session 1 (2026-10-20):** built everything.
- **Session 2 (2026-10-21):** validated the cloud-environment changes.

## Session 2: environment validation

- **`tools/cloud-setup.sh`, cold run.** Started from a near-fresh state: MinGW (gcc, g++), wabt, binaryen, Rust 1.97.0, Node 24 and the emsdk image all removed. Ran it from `/`.
  - 151 s, exit 0, every step OK: apt 21 s, rust 22 s, node 5 s, emsdk pull 100 s.
  - Warm reruns take 5–6 s.
  - Afterwards: no dockerd, and no `docker.pid`, `docker.sock` or `containerd.pid`.
- **Two bugs fixed in the version pasted on 2026-10-20** (commit `87f8b70`):
  1. The stop step killed itself. Its `pkill -f 'containerd --config …'` matched its own `bash -c` command line, so it reported FAILED; dockerd had already shut down cleanly. It now uses containerd's pid file.
  2. Login shells still got Node 22, because the image's `/etc/profile.d/nodejs.sh` prepends `/opt/node22/bin`. The script now writes `/etc/profile.d/zz-node24.sh`. Login and non-login shells both resolve Node 24.22.0.
  - **The user needs to paste the current `tools/cloud-setup.sh` into the environment again.**
- **SessionStart hook.** Verified for real: this session's dockerd was started by the hook (`/tmp/dockerd.log`, PPID 1).
  - From a stopped daemon, `CLAUDE_CODE_REMOTE=true .claude/hooks/session-start.sh` prints `{"async": true, …}` and dockerd answers within 2 s.
  - A second run is a no-op. With `CLAUDE_CODE_REMOTE` unset it does nothing.
  - `tools/start-dockerd.sh` alone works too.
- **Disk carries over.** Resuming a paused session kept the session-1 disk: scratchpad, tools and image were still there. So a changed setup script only takes effect in newly built environments, and has to be tested by running it, as above.
- **Remaining when this was written:** `tests/bench/memory/bench.mjs --browser` still hard-coded a Chromium path, and the Node 24 benchmark run was not recorded yet. A builder agent is fixing both.

## What works

| # | Item | Evidence |
|---|---|---|
| 1 | Rust workspace (`crates/x64wasm`, `x64wasm-cli`, `testkit`), JS runtime skeleton (`runtime/`), layout as in CLAUDE.md | `cargo test --workspace` green locally and in CI |
| 2 | **Translator spike.** mov/add/sub/cmp/lea/push/pop/jcc/call/ret (plus jmp, nop) at 8/16/32/64 bits → one memory64 module per code region. Runs in wasmtime, Node and headless Chromium (page and Worker). | 4 spike programs pass everywhere, including `high_address` (code at 0x1_4000_1000, stack above 4 GiB, ~5 GiB memory64); 55k native-vs-wasm comparisons; IR/WAT snapshots |
| 3 | **Instruction oracle.** ptrace single-step recorder on x86-64 Linux, 6480 cases in 10 families, replayed through the translator | `record --check` reproduces the fixtures byte for byte locally and on GitHub's runner; replay 6480/6480 |
| 4 | **Browser spike.** COOP/COEP server, `crossOriginIsolated`, shared memory64 (max 262144 pages), Worker blocking in wasm `memory.atomic.wait32`, woken by `Atomics.notify` | `tests/browser/threads.spec.mjs` green locally and in CI |
| 5 | **WebGPU spike.** Triangle rendered through SwiftShader in headless Chromium, verified by pixel readback | `tests/browser/webgpu.spec.mjs`: adapter `swiftshader`, centre red, corners clear colour |
| 6 | **Emscripten spike.** `-sMEMORY64 -pthread` probe vs gcc x86_64-linux: the LP64 core matches, pthreads work | `native/spike/run.sh` green in CI; table in `native/README.md` |
| 7 | **Guest memory design note**, with measurements | `docs/design/guest-memory.md`: decision A (identity mapping below 16 GiB) |
| 8 | **Artifact pipeline.** `artifacts.yml` publishes `hello.exe` as release `hello-<sha7>`; `scripts/fetch-artifact.sh` downloads it here with sha256 verified | Release `hello-c96388b`; download from this session in 0.8 s |
| 9 | **CI.** `linux-x64` runs items 2-6; `mingw-build` + `windows-x64` run the MinGW-built `hello.exe` natively and diff its output | All three jobs green |
| 10 | This note | |

### Test summary (local, session 2, Node 24.22.0)

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: all suites pass.
  - x64wasm: 3 unit, 3 snapshot, 5 translate.
  - testkit: 5 spike, 9 spike_insns, 12 spike_semantics, 8 oracle unit, plus replay, replay_harness and replay_inputs. spike_bench: 2 ignored, they are measurements.
- `cargo run -p testkit --release -- record --check`: "10 fixture files match a fresh recording".
- `npm run test:node`: 10/10.
- `npm run test:browser`: 4/4 (host self-test, threads, translator page+Worker, WebGPU) with Playwright's default browser. Also 4/4 with `CHROME_PATH` pointing at Playwright's full Chromium build.
- `native/spike/run.sh`: PASS, 0 failures, 19 known differences, 25 s. The optional host-Node run works now that host Node is 24.
- CI: green on every push since `3e2039c`.

## Measurements

| What | Result |
|---|---|
| sum_loop n=10^8, wasmtime vs native (same bytes) | 30.7–33.4 ms vs 29.4–30.2 ms = **1.02–1.14×** |
| Translation speed | 0.62 M guest instructions/s (0.38 M/s with the IR dump) |
| Code size | 21.9 wasm bytes per guest instruction (6.0 per guest code byte); spike modules ~1 KB |
| Cranelift, one function of 10k instructions / 2k blocks | 516 ms. A 201k-instruction region as one function did not finish in minutes. |
| Oracle | 6480 cases, 2.30 MB of fixtures. Recording ~1 s; replay ~4.6 s (release) |
| Atomics wake across a Worker | first wake 0.15–1.0 ms; ping-pong round trip median 0.02–0.04 ms, p95 0.03–0.09 ms |
| WebGPU on SwiftShader | adapter `{vendor: google, architecture: swiftshader}`; 64×64 triangle 1682 px; test ~0.2 s |
| Guest memory, ns per access (Chromium 141 / Node 22) | identity mapping (`m64`) ≈ memory32 on cache-missing loads. Translation table: 1.8–3.0× on hot/sequential loads, 1.5–1.8× behind a one-entry cache. NULL check +18% on hot loads. Full tables in `docs/design/guest-memory.md` |
| memory64 growth | grow to 15 GiB < 0.1 ms (shared); no RSS until touched |
| CI wall time | linux-x64 ~2.5 min warm cache (~9 min cold), windows-x64 ~15 s, artifacts ~40 s |

## Decisions

- **Module ABI** (`docs/design/abi.md`, `crates/x64wasm/src/abi.rs`):
  - The module imports `env.memory` (memory64) and exports `run(state: i64) -> i32`.
  - CPU state block: 0x200 bytes, GPRs at 0x00, RIP at 0x80, RFLAGS at 0x88, exit at 0x90, XMM at 0x100.
  - Exit reasons: 1 = left translated code, 2 = unsupported. Test functions return by `ret` to `SENTINEL_RET` = 0x7fff_ffff_0000.
- **Translator:**
  - iced-x86 decoding, then our own register-transfer IR, then wasm-encoder.
  - GPRs in i64 locals.
  - Lazy flags (QEMU cc_op style), with a jcc fused to its flag producer in the same block.
  - One wasm function per region: nested blocks plus `br_table` dispatch, and a compiled binary search for indirect targets.
  - Unsupported instructions exit with the state as it was before them.
- **Oracle:**
  - ptrace single-step of a child with fixed mappings: code at 0x10_0000, data at 0x20_0000 (64 KiB). Replay puts the state block at 0x30_0000 in 4 MiB of memory.
  - SplitMix64 seeded `"Tempwind"`, with a per-form stream.
  - One JSON file per family. `default_gprs` keeps the files small; the header is ignored by `--check`.
  - Fixtures are never hand-edited.
- **Guest memory:** option A. Identity mapping, guest VA == wasm offset, one shared memory64 per process, everything below 16 GiB, host (Emscripten) region at 2–4 GiB. Address map and revisit conditions are in `docs/design/guest-memory.md`.
- **Emscripten:** `emscripten/emsdk:6.0.12`, run through `scripts/emcc.sh` (repo at `/src`, caller's uid, persistent cache). Name Node outputs `.cjs`.
- **Pinned toolchain:**
  - Rust 1.97.0 (`rust-toolchain.toml`).
  - Node 24.22.0 (`.nvmrc`, CI `node-version-file`). Node 22 cannot run Emscripten 6 memory64 output and uses a pre-standard memory64 JS API.
  - Playwright 1.56.1 (Chromium 141), wasmtime 50, iced-x86 1.21, wasm-encoder/wasmparser/wasmprinter 0.262.
- **Browsers:** tests find Chromium through Playwright. `CHROME_PATH` overrides it: an environment variable locally, the `CHROME_PATH` repository variable in CI. Never hard-code a browser path.
- **WebGPU flags** for SwiftShader: `--enable-unsafe-webgpu --enable-unsafe-swiftshader --use-webgpu-adapter=swiftshader`. Vulkan/ANGLE flags were not needed.
- **memory64 JS API:**
  - Node 24 and Chromium 141 accept only `{address: 'i64', initial: BigInt}`.
  - Node 22 accepts only `{index: 'i64'}` with Number sizes, and silently makes a memory32 when given `address`.
  - `runtime/src/memory.mjs` tries each form and verifies the result.
- **CI:**
  - One `linux-x64` job runs items 2-6.
  - `hello.exe` is cross-built on Linux and run natively on `windows-2025`.
  - `.gitattributes` forces LF; Windows checkouts otherwise get CRLF.
- **Releases:** one immutable prerelease per artifact and commit, `<artifact>-<sha7>`. Sessions download with `scripts/fetch-artifact.sh` (REST + curl; `gh release download` is blocked here because GraphQL returns 403). Assets redirect to `objects.githubusercontent.com`, which is reachable.
- **Cloud environment:**
  - `tools/cloud-setup.sh` is the environment's setup script. It is time-boxed and always exits 0.
  - An async SessionStart hook (`.claude/settings.json` → `.claude/hooks/session-start.sh` → `tools/start-dockerd.sh`) starts dockerd in every cloud session. `service docker start` fails here on a ulimit error.

## Known issues and risks

- **Translator:**
  - One wasm function per region does not scale: Cranelift compile time explodes, and V8 has function-size limits. Phase 1 must split regions into several functions.
  - Not yet supported: `lock` and atomics, FS/GS (the TEB), the 67h prefix, an execution budget for interrupting guest loops, and precise faults (out-of-bounds guest access traps the instance).
  - Shared-memory modules are validated but not executed by the Rust tests. The browser threads spike covers shared memory64 separately.
- **Guest memory (option A):**
  - No aliasing views, no hardware-like protection, nothing at or above 16 GiB.
  - Wine's 47-bit layout constants must move below 16 GiB.
  - Revisit conditions are in the design note.
- **Wine's Unix side on wasm64:**
  - `_WIN64` and the `CONTEXT` header branches need a `__wasm64__` case.
  - `signal_x86_64.c` must be rewritten.
  - About 1800 syscalls need typed wrappers, because `call_indirect` traps on signature mismatch.
  - wineserver IPC must move to SharedArrayBuffer channels, signals become cooperative, and fork/exec and dlopen need replacements.
  - Full risk list: `native/README.md` "Risk list for Phase 2".
- **Environment.** A changed `tools/cloud-setup.sh` only takes effect after the user pastes it into the environment settings and a new environment is built. Resumed sessions keep their old disk.
- **`hello.exe` is not bit-reproducible** because of the PE timestamp. Releases are keyed by commit, so this is harmless.
- **WebGPU in CI depends on the SwiftShader that Chromium bundles.** Flags may change between Chromium versions, so re-check them when Playwright is bumped.

## Setup-script changes

The environment's Setup script is `tools/cloud-setup.sh`. Session 2 validated it: 151 s cold, 5 s warm, always exit 0. The user must paste the version from commit `87f8b70` or later. It does the following:

- installs MinGW-w64 x86-64 (gcc and g++), wabt, binaryen, and Rust 1.97.0 with wasm32;
- installs Node 24.22.0 from nodejs.org (sha256-checked) and makes it the default `node` in login and non-login shells;
- pre-pulls `emscripten/emsdk:6.0.12`, then stops dockerd cleanly.

Running dockerd per session is the SessionStart hook's job.

## Next step: Phase 1 (Translator)

Done when all instruction fixtures pass, the 64-bit 7-Zip CLI compresses and extracts exactly like native, and benchmarks are tracked. Suggested order:

1. Split translation into several wasm functions, with dispatch between them, before the instruction set grows.
2. Grow instruction coverage family by family with the oracle. Every new form gets recorded fixtures first; undefined flags go in `flags_mask`.
3. L2: MinGW-built C test programs, translated vs native (`tests/native/` is the seed); then a PE loader for the translator CLI.
4. Build 7-Zip and CoreMark with MinGW in CI (release assets), and track `7z b` and CoreMark in CI.
