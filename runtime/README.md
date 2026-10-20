# @tempwind/runtime

Phase 0 JS runtime skeleton (plain ES modules, JSDoc). ABI: `docs/design/abi.md`.

- `src/abi.mjs` constants mirroring `crates/x64wasm/src/abi.rs`; `src/memory.mjs` memory64 creation with API-form detection.
- `src/run.mjs` `runCase(wasmBytes, spec)` / `runManifest`; works in Node, pages and workers.
- `node --experimental-wasm-memory64 runtime/src/node-host.mjs target/spike` (flag not needed on Node 24+).
- `src/browser-host.mjs` and `src/worker.mjs` run a manifest in a page or Worker.
- `src/wasm-mini.mjs` tiny wasm encoder used for hand-made test modules.
