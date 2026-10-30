# Windows in the browser (working name)

Runs unmodified 64-bit Windows programs in the browser: x86-64 to WebAssembly translation, Wine 11 underneath, Direct3D to WebGPU. Everything runs on the user's device from static hosting. Plan: docs/plan.md. Status per phase: docs/status/.

## Layout
- crates/x64wasm: translator (decode, IR, WebAssembly)
- crates/x64wasm-cli: CLI (translate, translate-pe, dump IR and WAT)
- crates/x64wasm-rt: the translator built for wasm32, for run-time translation
- crates/testkit: instruction oracle, fixtures, replay
- runtime/: JS/TS runtime (Node and browser hosts)
- native/: Wine's Unix side built with Emscripten (Phase 2+)
- web/: desktop shell (Phase 3+)
- tests/: program, Wine and browser tests
- docs/: plan.md and status/phase-N.md

## Commands
Keep this section current; add each command when it first works.
- Lint and unit/integration tests (includes L1 fixture replay, spike tests in wasmtime, L5 snapshots): `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- L1 oracle (x86-64 Linux only; format: `docs/design/fixtures.md`): `cargo run -p testkit --release -- record` re-records `crates/testkit/fixtures/l1/`; `-- record --check` diffs a fresh recording against them; `-- replay` prints a per-family summary.
- Spike programs for the JS hosts: `cargo run -p testkit --release -- spike --out target/spike`
- Translator CLI: `cargo run -p x64wasm-cli -- translate|ir|wat --base 0x140001000 --hex "48 01 d8 c3" [-o out.wasm] [--shared] [--max-function-insns N] [--max-chunk-insns N] [--call-depth-limit N | --no-depth-guard]`
- Whole PE images: `cargo run -p x64wasm-cli --release -- translate-pe FILE --out-dir DIR [--entries all|entry]` writes `DIR/<stem>.wasm` (ABI v3) and `DIR/<stem>.plan.json` (load plan). Run-time translator: `cargo build -p x64wasm-rt --release --target wasm32-unknown-unknown`.
- Run a PE program in Node: `node runtime/src/run-pe.mjs --exe FILE.exe --aot FILE.wasm --plan FILE.plan.json [--rt target/wasm32-unknown-unknown/release/x64wasm_rt.wasm]`; `TEMPWIND_TRACE=api,translate,exits` traces to stderr.
- M2 test program: `tests/pe/build.sh` builds `target/pe/basic.exe` (expected: `tests/pe/basic.expected` and `tests/pe/basic.exitcode`); `tests/pe/prepare.sh` writes both AOT variants and the run-time translator, which `npm run test:node` needs.
- Regenerate IR/WAT snapshots: `UPDATE_SNAPSHOTS=1 cargo test -p x64wasm --test snapshots`
- Translator measurements: `cargo test -p testkit --release --test spike_bench -- --ignored --nocapture --test-threads=1` (wasmtime); in V8, `cargo run -p testkit --release -- spike --bench --out target/bench/translator`, then `node tests/bench/translator/bench.mjs [--runs N]` (Node, four V8 configurations) and `node tests/bench/translator/bench.mjs --browser` (recursion depth in Chromium page and Worker).
- Image measurements (needs `7zz.exe`, `coremark.exe` and `census.json` from the targets release in `target/targets/`): wasmtime `cargo test -p x64wasm-cli --release --test images -- --ignored --nocapture`; V8 `node tests/bench/translator/images.mjs [--runs N]`; linking speed `node tests/bench/translator/link.mjs` (after `spike --bench`).
- Phase 1 targets (7zz.exe, coremark.exe, built in CI by `.github/workflows/targets.yml` from the sources pinned in `tests/targets/sources.json`): `scripts/fetch-artifact.sh "$(cat tests/targets/targets.lock)" 7zz.exe` (also `coremark.exe`, `census.json`, `census.md`, `build-info.json`). Mirrored sources: releases `src-7zip-25.01` and `src-coremark-1f483d5`.
- Instruction census: `cargo run -p testkit --release -- census FILE... [--json PATH] [--md PATH] [--check]`; `--check` fails while in-scope forms lack L1 fixtures (scope rule: `docs/design/instruction-scope.md`).
- JS (Node 24): `npm ci`, then `npm run test:node` and `npm run test:browser` (Playwright, headless Chromium). Both need `target/spike`; `test:node` also needs the outputs of `tests/pe/prepare.sh` and `tests/native/prepare.sh`. `CHROME_PATH=/path/to/chrome npm run test:browser` uses another browser. Never run `playwright install` in a session; the image already has Playwright's Chromium.
- Docker: `tools/start-dockerd.sh` (idempotent; the SessionStart hook runs it).
- Emscripten probe (wasm64 vs x86_64-linux type sizes, pthreads): `native/spike/run.sh`; report in `native/spike/out/report.md`.
- Environment setup script: `tools/cloud-setup.sh` (safe to rerun; always exits 0 and prints a per-step summary).
- MinGW test programs (L2): `tests/native/build.sh` builds `target/native/{hello,l2_int,l2_crt}.exe`. The expected outputs are `tests/native/<name>.expected`, with LF line ends; checks compare them as CRLF, because msvcrt's stdout is in text mode. The exit codes are in `<name>.exitcode`. `tests/native/prepare.sh` translates the L2 programs for `npm run test:node`.
- Release assets: `scripts/fetch-artifact.sh TAG ASSET [DIR]` or `scripts/fetch-artifact.sh --latest PREFIX ASSET [DIR]` (sha256-verified). Releases are published by `.github/workflows/artifacts.yml` as `<artifact>-<sha7>`.

## How we work
- One phase at a time, in order. Before starting, read docs/plan.md and the latest docs/status/ note.
- Branch from main. One draft PR per milestone, titled "[Phase N] <what>"; a session covers a phase.
- Commits: author and committer `Skeleton363 <46569584+Skeleton363@users.noreply.github.com>`, unsigned. The SessionStart hook sets this in cloud sessions; check `git config user.email` before the first commit. Commit messages and PR descriptions carry no `Co-Authored-By`, `Claude-Session` or "Generated with Claude Code" lines. Post PR comments only where required.
- Done = all CI checks green, the test summary in your final message, and docs/status/phase-N.md updated (what works, measurements, decisions, known issues, next step).
- Sessions don't share memory: record every decision in docs/status/.
- Commit and push work in progress at least every 30 minutes; the cloud sandbox can reset.
- If something you need is missing (a tool, a domain, a secret, a decision), say exactly what and stop. Don't mock it, work around it or guess.
- Never weaken, skip or delete a test to get CI green; never hand-edit recorded fixtures.
- Ask before merging, changing branch rules, or adding any paid service.

## Network and builds (cloud sessions)
- GitHub works only for this repo; other GitHub repos return 403. Never clone or download from them in a session.
- Available: crates.io, npm, PyPI, Ubuntu apt, Docker Hub, raw.githubusercontent.com (single files), dl.winehq.org, gitlab.winehq.org.
- Wine: the wine-11.0 source tarball from dl.winehq.org.
- Environment: `tools/cloud-setup.sh` is the cloud environment's setup script (MinGW-w64 x86-64, wabt, binaryen, the pinned Rust toolchain with wasm32, Node 24, the emsdk image). When you change it, ask the user to paste the new version into the environment settings.
- Docker: a SessionStart hook (`.claude/settings.json`) starts dockerd in every cloud session, in the background. If `docker info` fails, run `tools/start-dockerd.sh`. Never use `service docker start`; it fails in this sandbox.
- Emscripten: the emscripten/emsdk Docker image (tag pinned in `scripts/emcc.sh`), with the repo mounted at /src. Run tools through `scripts/emcc.sh`.
- Node: 24, pinned in `.nvmrc` for CI and sessions.
- Chrome for headless tests: Playwright's Chromium, preinstalled in the image. Tests find it through Playwright; never hard-code a browser path. Set `CHROME_PATH` to test with another Chrome or Chromium binary (in CI, the `CHROME_PATH` repository variable).
- Heavy builds (Wine, Emscripten builds of Wine), long jobs, and anything needing other GitHub repos run in GitHub Actions. Publish results as artifacts or releases in this repo; sessions download them instead of rebuilding.

## Clean room and licenses
- Never read or copy code from WebWindows, BottleShip, DirectWebGPU/Theseus or Boxedwine/Boxedwine64. Work from specs (Intel SDM, PE/COFF, WebAssembly, WebGPU) and Wine's own source.
- Wine, Emscripten, iced-x86, wasm-encoder, vkd3d-shader and Naga are fine as dependencies under their own licenses. Our own code is MIT; our patches to Wine stay public under the LGPL.

## Delegation and effort
- Searching or reading code: Explore (Haiku, low).
- Routine work (docs, CI, test plumbing, shell UI, small fixes): builder (Sonnet, medium).
- Translator core, Wine system-call layer, memory model and threads, graphics backend, hard bugs: deep-worker (Opus, high).
- If an attempt fails the same tests twice, retry one tier up. Never use max effort unless asked.

## Working efficiently
Quality first: rework is the most expensive waste. Cut the rest:
- Contracts and tests before code: design notes and ABI changes are committed before agents start; fixtures or a failing test come before a fix.
- Read narrowly: Grep, then Read with offset/limit. Never read fixtures, `census.json`, snapshots or CI logs whole; save logs to a file and grep them.
- Keep command output short (`cargo test -q`, `| tail -n 40`); full output only while diagnosing.
- Split work into agents only along artifact boundaries (fixtures, ABI, separate files); never chain agents through summaries; small or tightly coupled work stays in the main loop.
- Delegate large, self-contained work, not lookups one Grep answers. The brief names the files, the contract and the tests that decide "done"; agents report in about 150 words; the coordinator reviews diffs, not whole files.
- Verify before pushing, including CI scripts that can run in the session. Wait for PR events instead of polling CI.
