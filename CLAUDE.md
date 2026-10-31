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
Core verification; the full reference is `docs/dev/commands.md` (add each command there when it first works). Area rules are in `.claude/rules/`; procedures are skills in `.claude/skills/` (`/pause`, `/milestone-close`, `/diverge`). Design of the whole: `ARCHITECTURE.md`.
- Lint and tests (includes L1 replay, spike tests, L5 snapshots): `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- L1 oracle: `cargo run -p testkit --release -- record --check` (fresh recording must equal the fixtures); `-- replay` prints a per-family summary. Fixtures come only from `record`.
- Census: `cargo run -p testkit --release -- census FILE... --check` fails while in-scope forms lack fixtures.
- JS (Node 24): `npm ci`, `npm run test:node`, `npm run test:browser`. They need `target/spike`, `tests/pe/prepare.sh` and `tests/native/prepare.sh` outputs. Never run `playwright install`; the image has Chromium.
- 7-Zip matrix (needs `target/targets/7zz.exe`): `node tests/sevenzip/matrix.mjs --mode native|wine|translated --out OUT`, then `node tests/sevenzip/compare.mjs OUT tests/sevenzip/expected`.
- Release assets: `scripts/fetch-artifact.sh TAG ASSET [DIR]`.

## How we work
- One phase at a time, in order. Before starting, read docs/plan.md and the latest docs/status/ note.
- Each milestone's planning starts with a short research pass over official and community sources for its domain and our workflow, logged in `docs/dev/research.md`.
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
- Setup, Docker, Emscripten, Node and Chrome details: `docs/dev/commands.md`, "Environment and builds". Never use `service docker start`; never hard-code a browser path.
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
