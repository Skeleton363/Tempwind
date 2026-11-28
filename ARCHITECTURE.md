# Architecture

Tempwind runs unmodified 64-bit Windows programs in the browser. The long-term stack is x86-64 to WebAssembly translation, Wine 11 underneath, Direct3D to WebGPU, all on the user's device from static hosting (`docs/plan.md`). Phase 1 ran 7-Zip and CoreMark on a JS Win32 shim; Phase 2 M4 deleted the shim, and Wine runs every program. This file is the map for someone new to the repo. Details live in `docs/design/`; status in `docs/status/`.

## Bird's eye view

```
FILE.exe
  -> x64wasm-cli translate-pe
       -> FILE.wasm        translated module (ABI v3/v4)
       -> FILE.plan.json   load plan (sections, imports, entries)
  -> runtime (runtime/src/wine/): Wine's PE side and Unix side, with the Linker
       -> translated code calls the Unix side through the boundary (abi.md v5)
  code no module has translated
       -> x64wasm_rt.wasm translates it at run time, Linker links it
  floating point
       -> x64wasm_fp.wasm (soft float behind SSE and x87)
```

Translation is function-level: the translator discovers functions from the entry points, lifts each to IR, and emits one WebAssembly function per guest function (large ones split into chunks). Indirect transfers go through a lookup table from guest address to table index. Guest memory is one linear memory; the guest address space is mapped into it (`docs/design/guest-memory.md`).

## Codemap

### crates/
- `x64wasm`: the translator. `pe.rs` (PE reader), `image.rs` (whole-image discovery and the load plan, `LoadPlan`), `func.rs` (function and chunk splitting), `lift/` and `lift.rs` (decoded instruction to IR), `ir.rs`, `emit/` and `emit.rs` (IR to WebAssembly), `switch.rs` (jump tables), `link.rs` (block tables), `abi.rs` (constants shared with JS). `DECODER_OPTIONS` is the one decoder configuration.
- `x64wasm-cli`: `translate`, `ir`, `wat`, `translate-pe`, and `bridge`, which writes the bridge module between a Wine process's Unix side and its translated modules (`abi.md` v5, "The bridge"). Thin; logic stays in `x64wasm`.
- `x64wasm-rt`: the translator built for `wasm32-unknown-unknown`, called by the runtime for run-time translation.
- `x64wasm-fp`: integer-only soft float (SSE and x87), built for wasm32; its differential tests run the real CPU in-process.
- `testkit`: the instruction oracle. `oracle/` records and replays L1 fixtures, `census.rs` counts instruction forms in target images, `spike/` builds the spike programs for the JS hosts, `host.rs` runs modules in wasmtime. Fixtures: `crates/testkit/fixtures/l1/`.

### runtime/src/
- `link.mjs` (`Linker`): table, `lookup`, `depth` globals, block tables; links the AOT module, run-time translations and the host-call module.
- `rt.mjs` (with `requestFlags`), `fp.mjs`: wrappers for the run-time translator and the soft float module.
- `plan.mjs`, `abi.mjs`: the load plan and ABI constants on the JS side. `memory.mjs`, `guest-memory.mjs`: the memory and the guest address map.
- `node-host.mjs`, `browser-host.mjs`, `worker.mjs`, `run.mjs`: the hosts of the spike and selftest programs.
- `wine/`: the Wine host (`wine-process.mjs`, `run-wine.mjs`, the boot set, the AOT catalog, host files, `web-files.mjs` for the browser).

### Elsewhere
- `tests/`: `pe` and `native` (programs and expected output), `node` (runtime tests), `wine` (Wine host tests), `browser` (Playwright), `sevenzip` (138-step command matrix), `bench`, `targets` (7-Zip and CoreMark pins).
- `native/`: Wine's Unix side with Emscripten (Phase 2+). `native/wine/` (LGPL): `build.sh`, which builds ntdll's and win32u's Unix sides and the server into one module, the patch series against the wine-11.0 tarball, our new files in `src/` (the memory backend, the in-process server transport, green threads and their signals, boot, the syscall and unix-call dispatchers, the static table of unix libraries, the run loop), `gen-syscalls.mjs` (the typed syscall wrappers, generated from Wine's headers at build time), the generated `include/tempwind/abi.h`, the C harnesses in `tests/`, and `rebase/`, the tool that gives the closure's DLLs unique bases at packaging (its own LGPL crate). `native/spike/boundary/` is M1's boundary spike. `runtime/src/wine/` is the JS host for the Unix side: the process, the links of the bridge, the AOT catalog's modules and run-time translations, the lazy boot set, and the Node and browser inputs. `tests/wine/` holds its Node tests, `prepare.sh` and `catalog.mjs`, which builds the AOT catalog of the closure (`wine-build.md`, "Packaging"). `web/`: desktop shell (Phase 3+).
- `tools/`, `scripts/`, `.github/workflows/`: environment setup, artifact fetching, CI.

## Contracts

| Contract | Where |
|---|---|
| Module shape, lookup table, transfers, host functions, statuses, trampoline | `docs/design/abi.md` |
| Translator pipeline, terminators, functions and chunks, lowering | `docs/design/translator.md` |
| Fixture format | `docs/design/fixtures.md` |
| Instruction scope and the "all fixtures pass" rule | `docs/design/instruction-scope.md` |
| Load plan, Linker and run-time translator hosts, the spike hosts | `docs/design/runtime.md` |
| Guest address map | `docs/design/guest-memory.md` |
| Wine build, release layout, DLL closure, prefix, conformance baselines and runs (Phase 2) | `docs/design/wine-build.md` |
| Wine's Unix side with Emscripten: build, patches, memory backend, in-process server, startup, green threads, exceptions (Phase 2) | `docs/design/wine-unix.md` |

Contracts are committed before the code that uses them. The ABI is versioned; both Rust (`abi.rs`) and JS (`abi.mjs`) change with it, and from v5 the C header `native/wine/include/tempwind/abi.h`, which `x64wasm-cli gen-header` generates from `abi.rs`.

## Test levels

- L1: every instruction form, recorded on a real x86-64 CPU by `testkit record`, replayed in wasmtime.
- L2: MinGW C programs (`tests/native`), translated against native output.
- L3: Wine's conformance tests (`tests/conformance`): baselines recorded on windows-2025 and native Wine 11, and from Phase 2 M5 our port's runs, tracked in CI.
- L4: headless Chrome with WebGPU (SwiftShader), pixel checks (later phases).
- L5: IR and WAT snapshots (`crates/x64wasm/tests/snapshots`).
- Program level: CoreMark validates; the 7-Zip matrix compares translated output to native byte for byte.
- Node and browser tests cover the runtime (`tests/node`, `tests/browser`).

## Invariants

- The state block in guest memory is the truth at every boundary (call, return, exit, host call). Registers cached in wasm locals are stored before a boundary and reloaded after. The lazy flags record is stored whole.
- The first translation of an address wins. Linking never replaces a block that already has a table entry.
- A jump, call or return to an address without a block unwinds to the trampoline; the lookup miss exits with `EXIT_LEFT_CODE`.
- `FaultIf` comes before the first side effect of an instruction.
- Every decoder uses `x64wasm::DECODER_OPTIONS`, so the census, the oracle and the translator agree on what an encoding means.
- Fixtures come only from the recorder, never by hand. `record --check` reproduces on any CPU vendor; implementation-dependent results are `undefined`.
- Floating-point results are bit-exact with the CPU. `x64wasm-fp` uses no host floating point.
- Guest threads are cooperative: they take turns on one JS thread and switch only at blocking calls, which suspend through JSPI (green threads in `native/wine/`). The schedule is deterministic.
- A new thread's state block carries real FP defaults (MXCSR), not zeros.
- JS never parses PE files; `translate-pe` writes the plan.
- Clean room: never read or copy WebWindows, BottleShip, DirectWebGPU/Theseus or Boxedwine. Work from specs, Wine's source and the programs' own source. Wine's source is read, not copied. Our code is all rights reserved (`LICENSE`); our Wine patches stay LGPL.

## Cross-cutting concerns

- Depth: the call depth guard (default 500) keeps recursion inside the host stack; see `translator.md`.
- Measurement: translator, linking and CoreMark/7-Zip benchmarks are under `tests/bench`; results go in `docs/status/phase-1.md`.
- Hosts: the same modules run in Node and in browser pages and Workers; V8 flags matter for compile-time measurements (`.claude/rules/runtime.md`).
- Workflow: area rules in `.claude/rules/`, procedures in `.claude/skills/`, agents in `.claude/agents/`, commands in `docs/dev/commands.md`, research log in `docs/dev/research.md`.
