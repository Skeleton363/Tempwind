# @tempwind/runtime

JS runtime (plain ES modules, JSDoc). Contracts: `docs/design/abi.md` (ABI v3) and `docs/design/runtime.md` (what the runtime keeps, and the Wine host's boundary).

- `src/abi.mjs` constants mirroring `crates/x64wasm/src/abi.rs`, plus PEB/TEB offsets (checked against Wine 11 `include/winternl.h`) and Win32 values; append-only.
- `src/memory.mjs` memory64 creation with API-form detection; `src/guest-memory.mjs` bounds-checked typed access to guest memory.
- `src/link.mjs` the v3 linker: funcref table (slot 0 = host-call function), `table_base`/`lookup`/`depth` globals, lookup table in guest memory (first wins, rehash), sorted key mirror, `x64wasm.blocks` parsing, `hostFunction` wrapping.
- `src/plan.mjs` load plan parsing, checks against the PE file (sha256, machine, address map) and the address map.
- `src/rt.mjs` wraps `x64wasm_rt.wasm` and holds `requestFlags`. `src/wine/` the Wine host (`run-wine.mjs` runs a program: `node runtime/src/wine/run-wine.mjs FILE.exe`).
- `src/run.mjs` `runCase(wasmBytes, spec)` / `runManifest` for spike cases (one v3 module per instance, lookup table after `memoryBytes`); works in Node, pages and workers.
- `node runtime/src/node-host.mjs target/spike` runs a spike manifest (Node 24+, pinned in `.nvmrc`).
- `src/browser-host.mjs` and `src/worker.mjs` run a manifest in a page or Worker.
- `src/wasm-mini.mjs` tiny wasm encoder for hand-made test modules, including the v3 module shape.
