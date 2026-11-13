# Windows in the browser (working name)

Runs unmodified 64-bit Windows programs in the browser: x86-64 to WebAssembly translation, Wine 11 underneath, Direct3D to WebGPU. Everything runs on the user's device from static hosting. Plan: docs/plan.md. Status per phase: docs/status/. Design of the whole and every directory: `ARCHITECTURE.md`.

## Layout
- crates/: x64wasm (the translator), x64wasm-cli, x64wasm-rt (the translator for wasm32, run-time translation), x64wasm-fp (soft float), testkit (instruction oracle, fixtures, replay)
- runtime/: JS runtime (Node and browser hosts, the Win32 shim); native/: Wine's Unix side with Emscripten (Phase 2+); web/: desktop shell (Phase 3+)
- tests/: program, Wine and browser tests; docs/: plan, status, design

## Commands
Core verification. The oracle, census, 7-Zip matrix, benchmarks, profiling and release assets are in `docs/dev/commands.md`; add each command there when it first works. Area rules: `.claude/rules/`. Procedures: `/pause`, `/milestone-close`, `/diverge` (`.claude/skills/`).
- Lint and tests (L1 replay, spike tests, L5 snapshots): `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- JS (Node 24): `npm ci`, `npm run test:node`, `npm run test:browser`. They need `target/spike` and the outputs of `tests/pe/prepare.sh` and `tests/native/prepare.sh`. The image has Playwright's Chromium.

## How we work
- One phase at a time, in order. Before starting, read docs/plan.md and the latest docs/status/ note.
- Each milestone's planning starts with a short research pass over official and community sources for its domain and our workflow, logged in `docs/dev/research.md`.
- Branch from main. One draft PR per milestone, titled "[Phase N] <what>".
- A session covers one milestone. It starts from docs/plan.md, the latest status note and ARCHITECTURE.md, and ends with `/milestone-close` (or `/pause`) and its hand-off.
- Commits: author and committer `Skeleton363 <46569584+Skeleton363@users.noreply.github.com>`, unsigned. The SessionStart hook sets this in cloud sessions; check `git config user.email` before the first commit. Commit messages and PR descriptions carry no `Co-Authored-By`, `Claude-Session` or "Generated with Claude Code" lines. Post PR comments only where required.
- Done = all CI checks green, the test summary in your final message, and docs/status/phase-N.md updated (what works, measurements, decisions, known issues, next step).
- Sessions don't share memory: record every decision in docs/status/.
- Judge what we do by our own evidence. A change, to code or to the workflow, starts from a problem we saw or a measured opportunity. It names how we'll know it worked, and that evidence decides whether it is kept, reshaped or reverted. A source recommending something is a lead, not a reason. Workflow trials: `docs/dev/agent-runs.md`.
- Commit and push work in progress at least every 30 minutes; the cloud sandbox can reset. WIP goes to a side branch (`git push origin HEAD:refs/heads/<track>-wip`); the PR branch gets only commits that pass. The pre-push hook (`tools/git-hooks`) refuses a WIP head or unformatted Rust there.
- If something you need is missing (a tool, a domain, a secret, a decision), say exactly what and stop. Don't mock it, work around it or guess.
- Never weaken, skip or delete a test to get CI green; never hand-edit recorded fixtures.
- Ask before merging, changing branch rules, or adding any paid service.

## Network and builds (cloud sessions)
- GitHub works only for this repo; other GitHub repos return 403. Never clone or download from them in a session.
- Available: crates.io, npm, PyPI, Ubuntu apt, Docker Hub, raw.githubusercontent.com (single files), dl.winehq.org, gitlab.winehq.org, learn.microsoft.com, Reddit, Hacker News (news.ycombinator.com, hn.algolia.com).
- Wine: the wine-11.0 source tarball from dl.winehq.org.
- Setup, Docker, Emscripten, Node and Chrome: `docs/dev/commands.md`, "Environment and builds".
- Heavy builds (Wine, Emscripten builds of Wine), long jobs, and anything needing other GitHub repos run in GitHub Actions. Publish results as artifacts or releases in this repo; sessions download them instead of rebuilding.

## Clean room and licenses
- Never read or copy code from WebWindows, BottleShip, DirectWebGPU/Theseus or Boxedwine/Boxedwine64. Work from specs (Intel SDM, PE/COFF, WebAssembly, WebGPU) and Wine's own source.
- Wine, Emscripten, iced-x86, wasm-encoder, vkd3d-shader and Naga are fine as dependencies under their own licenses. A new dependency must be permissive (MIT, Apache-2.0, BSD and the like) or LGPL, never GPL. Our own code is all rights reserved (`LICENSE`) in a public repository; our patches to Wine stay public under the LGPL.

## Delegation and effort
- Agents (`.claude/agents/`): Explore (Haiku, low) searches and reads code; builder (Sonnet, medium) does routine work (docs, CI, test plumbing, shell UI, small fixes); deep-worker (Opus, high) does the translator core, Wine's system-call layer, memory model and threads, the graphics backend and hard bugs.
- If an attempt fails the same tests twice, retry one tier up. Never use max effort unless asked.
- Log every finished agent run in `docs/dev/agent-runs.jsonl` (method and current trials: `docs/dev/agent-runs.md`). Subagents run at most 3 at once and never spawn their own (`.claude/settings.json`).

## Working efficiently
Quality first: rework is the most expensive waste. Cut the rest:
- Contracts and tests before code: design notes and ABI changes are committed before agents start; fixtures or a failing test come before a fix.
- Read narrowly: Grep, then Read with offset/limit. Never read fixtures, `census.json`, snapshots or CI logs whole; save logs to a file and grep them. Keep command output short (`cargo test -q`, `| tail -n 40`) except while diagnosing.
- Split work into agents only along artifact boundaries (fixtures, ABI, separate files); never chain agents through summaries; small or tightly coupled work stays in the main loop.
- Delegate large, self-contained work, not lookups one Grep answers. The brief names the files, the contract and the tests that decide "done"; agents report in about 150 words; the coordinator reviews diffs, not whole files.
- Verify before pushing, including CI scripts that can run in the session. Wait for PR events instead of polling CI.
