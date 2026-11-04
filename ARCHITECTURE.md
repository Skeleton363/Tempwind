# Architecture

Tempwind runs unmodified 64-bit Windows programs in the browser. The long-term stack is x86-64 to WebAssembly translation, Wine 11 underneath, Direct3D to WebGPU, all on the user's device from static hosting (`docs/plan.md`). Phase 1 is the translator plus a JS Win32 shim that runs 7-Zip and CoreMark in Node without Wine. This file is the map for someone new to the repo. Details live in `docs/design/`; status in `docs/status/`.

## Bird's eye view

```
FILE.exe
  -> x64wasm-cli translate-pe
       -> FILE.wasm        translated module (ABI v3/v4)
       -> FILE.plan.json   load plan (sections, imports, entries)
  -> runtime: Linker + Process
       -> guest memory, PEB/TEB, import thunks, run loop
       -> translated code calls the host with a host call
            -> Shim (runtime/src/win32/*) implements the API
  code no module has translated
       -> x64wasm_rt.wasm translates it at run time, Linker links it
  floating point
       -> x64wasm_fp.wasm (soft float behind SSE and x87)
```

Translation is function-level: the translator discovers functions from the entry points, lifts each to IR, and emits one WebAssembly function per guest function (large ones split into chunks). Indirect transfers go through a lookup table from guest address to table index. Guest memory is one linear memory; the guest address space is mapped into it (`docs/design/guest-memory.md`).

## Codemap

### crates/
- `x64wasm`: the translator. `pe.rs` (PE reader), `image.rs` (whole-image discovery and the load plan, `LoadPlan`), `func.rs` (function and chunk splitting), `lift/` and `lift.rs` (decoded instruction to IR), `ir.rs`, `emit/` and `emit.rs` (IR to WebAssembly), `switch.rs` (jump tables), `link.rs` (block tables), `abi.rs` (constants shared with JS). `DECODER_OPTIONS` is the one decoder configuration.
- `x64wasm-cli`: `translate`, `ir`, `wat`, `translate-pe`. Thin; logic stays in `x64wasm`.
- `x64wasm-rt`: the translator built for `wasm32-unknown-unknown`, called by the runtime for run-time translation.
- `x64wasm-fp`: integer-only soft float (SSE and x87), built for wasm32; its differential tests run the real CPU in-process.
- `testkit`: the instruction oracle. `oracle/` records and replays L1 fixtures, `census.rs` counts instruction forms in target images, `spike/` builds the spike programs for the JS hosts, `host.rs` runs modules in wasmtime. Fixtures: `crates/testkit/fixtures/l1/`.

### runtime/src/
- `link.mjs` (`Linker`): table, `lookup`, `depth` globals, block tables; links the AOT module, run-time translations and the host-call module.
- `process.mjs` (`Process`): guest memory, address map, image mapping, PEB/TEB/stack/TLS, import thunks, run loop, exit status.
- `hostcall.mjs` (`HostContext`, `HostStop`): the guest-to-host call path.
- `threads.mjs`: cooperative guest threads (scheduler, waits, locks, deadlock detection).
- `win32/`: the shim, one module per DLL (`kernel32*.mjs`, `msvcrt.mjs`, `advapi32.mjs`, `user32.mjs`, `oleaut32.mjs`), with the in-memory file system `vfs.mjs` and handle table `handles.mjs`.
- `rt.mjs`, `fp.mjs`: wrappers for the run-time translator and the soft float module.
- `plan.mjs`, `abi.mjs`: the load plan and ABI constants on the JS side.
- `node-host.mjs`, `browser-host.mjs`, `worker.mjs`, `vfs-host.mjs`: hosts. `run-pe.mjs` is the command-line runner. `trace.mjs` implements `TEMPWIND_TRACE`.

### Elsewhere
- `tests/`: `pe` and `native` (programs and expected output), `node` (shim and runtime tests), `browser` (Playwright), `sevenzip` (138-step command matrix), `bench`, `targets` (7-Zip and CoreMark pins).
- `native/`: Wine's Unix side with Emscripten (Phase 2+). `web/`: desktop shell (Phase 3+).
- `tools/`, `scripts/`, `.github/workflows/`: environment setup, artifact fetching, CI.

## Contracts

| Contract | Where |
|---|---|
| Module shape, lookup table, transfers, host functions, statuses, trampoline | `docs/design/abi.md` |
| Translator pipeline, terminators, functions and chunks, lowering | `docs/design/translator.md` |
| Fixture format | `docs/design/fixtures.md` |
| Instruction scope and the "all fixtures pass" rule | `docs/design/instruction-scope.md` |
| Process, load plan, host calls, shim | `docs/design/runtime.md` |
| Guest address map | `docs/design/guest-memory.md` |

Contracts are committed before the code that uses them. The ABI is versioned; both Rust (`abi.rs`) and JS (`abi.mjs`) change with it.

## Test levels

- L1: every instruction form, recorded on a real x86-64 CPU by `testkit record`, replayed in wasmtime.
- L2: MinGW C programs (`tests/native`), translated against native output.
- L3: Wine's conformance tests, with recorded baselines (later phases).
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
- The shim never fakes success. Unimplemented imports, `GetProcAddress` misses, exceptions, and blocking inside a host-to-guest call (`callNested`) stop loudly.
- Guest threads are cooperative: they take turns on one JS thread and switch only at blocking host calls, which suspend through JSPI (`threads.mjs`). The schedule is deterministic.
- A new thread's state block carries real FP defaults (MXCSR), not zeros.
- JS never parses PE files; `translate-pe` writes the plan.
- Clean room: never read or copy WebWindows, BottleShip, DirectWebGPU/Theseus or Boxedwine. Work from specs, Wine's source and the programs' own source. Wine's source is read, not copied. Our code is MIT; our Wine patches stay LGPL.

## Cross-cutting concerns

- Depth: the call depth guard (default 500) keeps recursion inside the host stack; see `translator.md`.
- Measurement: translator, linking and CoreMark/7-Zip benchmarks are under `tests/bench`; results go in `docs/status/phase-1.md`.
- Hosts: the same modules run in Node and in browser pages and Workers; V8 flags matter for compile-time measurements (`.claude/rules/runtime.md`).
- Workflow: area rules in `.claude/rules/`, procedures in `.claude/skills/`, agents in `.claude/agents/`, commands in `docs/dev/commands.md`, research log in `docs/dev/research.md`.
