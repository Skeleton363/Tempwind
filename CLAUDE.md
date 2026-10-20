# Windows in the browser (working name)

Runs unmodified 64-bit Windows programs in the browser: x86-64 to WebAssembly translation, Wine 11 underneath, Direct3D to WebGPU. Everything runs on the user's device from static hosting. Plan: docs/plan.md. Status per phase: docs/status/.

## Layout
- crates/x64wasm: translator (decode, IR, WebAssembly)
- crates/x64wasm-cli: CLI (translate, dump IR and WAT)
- crates/testkit: instruction oracle, fixtures, replay
- runtime/: JS/TS runtime (Node and browser hosts)
- native/: Wine's Unix side built with Emscripten (Phase 2+)
- web/: desktop shell (Phase 3+)
- tests/: program, Wine and browser tests
- docs/: plan.md and status/phase-N.md

## Commands
Keep this section current; add each command when it first works.

## How we work
- One phase at a time, in order. Before starting, read docs/plan.md and the latest docs/status/ note.
- Branch from main. One draft PR per session, titled "[Phase N] <what>".
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
- Emscripten: the emscripten/emsdk Docker image, with the repo mounted at /src. Run "service docker start" first if Docker isn't running.
- Chrome for headless tests: under /opt/chrome.
- Heavy builds (Wine, Emscripten builds of Wine), long jobs, and anything needing other GitHub repos run in GitHub Actions. Publish results as artifacts or releases in this repo; sessions download them instead of rebuilding.

## Clean room and licenses
- Never read or copy code from WebWindows, BottleShip, DirectWebGPU/Theseus or Boxedwine/Boxedwine64. Work from specs (Intel SDM, PE/COFF, WebAssembly, WebGPU) and Wine's own source.
- Wine, Emscripten, iced-x86, wasm-encoder, vkd3d-shader and Naga are fine as dependencies under their own licenses. Our own code is MIT; our patches to Wine stay public under the LGPL.

## Delegation and effort
- Searching or reading code: Explore (Haiku, low).
- Routine work (docs, CI, test plumbing, shell UI, small fixes): builder (Sonnet, medium).
- Translator core, Wine system-call layer, memory model and threads, graphics backend, hard bugs: deep-worker (Opus, high).
- If an attempt fails the same tests twice, retry one tier up. Never use max effort unless asked.
