# Tempwind plan

Tempwind (working name) runs unmodified 64-bit Windows programs in the browser:
x86-64 to WebAssembly translation, Wine 11 underneath, Direct3D to WebGPU.
It is built entirely by AI sessions, one phase at a time. Each phase records
its state in `docs/status/phase-N.md`.

## Goal

- Run unmodified 64-bit Windows applications of 2010s weight in Chrome and Firefox.
- Everything runs on the user's device, served from static hosting. There is no server-side execution.
- Users bring their own programs. We host only open-source code.
- Wine ships under its LGPL, and our patches to Wine stay public.
- Our own code is MIT.

## Architecture

### 1. Translator (Rust)
- Decoding: iced-x86.
- Our own IR between decoding and code generation.
- Code generation: wasm-encoder, emitting **memory64** WebAssembly.
- When translation happens:
  - **Ahead of time** when a program is added.
  - **At run time** for code the ahead-of-time pass missed.
  - A **profile** recorded at run time feeds the next ahead-of-time pass.
- Translated results are cached in **OPFS**.

### 2. Wine 11.0 underneath
- Wine's x86-64 PE DLLs go through the same translator as the user's programs.
- Wine's Unix side (the Unix part of ntdll, wineserver, win32u) is compiled with **Emscripten**.
- A browser display driver draws to a canvas.
- There is no emulated Linux and no X11.

### 3. Graphics
- wined3d gets a new **WebGPU backend**. It covers Direct3D 9-11 and Wine's Direct2D.
- Shaders go vkd3d-shader → SPIR-V → WGSL (via Naga).
- GPU work runs in its own worker.

### 4. Runtime
- Web Workers plus SharedArrayBuffer, which needs cross-origin isolation (COOP/COEP headers).
- Input, Web Audio, and OPFS storage.

### 5. Shell
- Desktop UI.
- A flow for adding a program.
- Programs are installed once. Each run gets a throwaway session layer on top.
- Google Drive sync.

## Tests

| Level | What | Compared against |
|---|---|---|
| L1 | Every instruction form | Results recorded on a real x86-64 CPU |
| L2 | C programs built with MinGW x86-64, translated vs native | Native runs on Linux and Windows runners |
| L3 | Wine's conformance tests | Recorded baselines |
| L4 | Headless Chrome (WebGPU via SwiftShader, pixel readback) | Expected pixels or screenshots |
| L5 | IR and WAT snapshots | Committed snapshots |

Benchmarks are tracked in CI: **CoreMark** and **7-Zip `7z b`**.

## Phases

Phases run in order. Each has a done-when.

| # | Phase | Done when |
|---|---|---|
| 0 | Foundations | CI green on the draft PR "[Phase 0] Foundations": workspace and skeleton, translator spike, instruction oracle prototype, browser/WebGPU/Emscripten spikes, guest-memory design note, artifact pipeline, Linux + Windows CI, `docs/status/phase-0.md`. |
| 1 | Translator | All instruction fixtures pass. The 64-bit 7-Zip CLI compresses and extracts exactly like native. Benchmarks are tracked. |
| 2 | Wine underneath | Wine's PE DLLs are built in CI from the dl.winehq.org tarball. 64-bit console programs run in the browser. kernel32/ntdll test pass rates are tracked. |
| 3 | Windows on screen + shell | 64-bit Notepad++ installs from its real installer and survives a reload. A public demo runs with open-source apps only. |
| 4 | Threads and hard parts | Threads, Windows exceptions, child processes, and .NET via Wine Mono work. 7-Zip uses all cores. A WinForms app runs. |
| 5 | Graphics | Direct3D 11 samples match native-Wine screenshots. 64-bit VLC plays video. A DirectX 11 indie game runs at 30+ fps on a good PC. |
| 6 | Speed and product | Profile-guided re-translation, fast startup, Drive sync, and a public compatibility list. |

**Out of scope for now:** DirectX 12, anti-cheat, drivers, phones.

## Phase 0 work items

1. Rust workspace and runtime skeleton matching the layout in `CLAUDE.md`.
2. Translator spike: translate mov, add, sub, cmp, jcc, call, ret, push, pop and lea into a memory64 module, and run a test function in Node and headless Chrome.
3. Instruction oracle prototype: record single-instruction results natively on x86-64 Linux, and replay them through the translator.
4. Browser spike: memory64, SharedArrayBuffer, a Web Worker and `Atomics.wait` in headless Chrome, served with COOP/COEP headers.
5. WebGPU spike: draw a triangle in headless Chrome through SwiftShader, and verify it by pixel readback.
6. Emscripten spike (Docker image): build a small C program with `-sMEMORY64` and pthreads. Report pointer, long and size_t sizes against what Wine's x86-64 Unix side assumes.
7. Design note: guest memory layout for 64-bit programs. x64 images default to addresses above 4 GB, and browsers cap 64-bit Wasm memory at 16 GB. Compare rebasing with an address-translation table, and recommend one.
8. Artifact pipeline proof: a CI job that builds something small and publishes it as a release asset of this repo, plus a script sessions use to download it.
9. GitHub Actions:
   - A Linux x86-64 job running items 2-6.
   - A Windows job that runs a MinGW-built x86-64 test `.exe` natively and checks its output.
10. `docs/status/phase-0.md`: what works, measurements, decisions, risks, and any changes the environment's setup script needs.

## Phase 1 milestones

Each milestone ends with CI green, a draft PR "[Phase 1] <milestone>", and `docs/status/phase-1.md` updated. Decisions taken when planning Phase 1:

- 7-Zip runs before Wine on a Win32 shim written in JS in `runtime/`.
- "Native on Linux" is the same `.exe` under Ubuntu's wine64. Real Windows is the reference of truth.
- The 7-Zip CLI is the standalone `7zz.exe` (Alone2 bundle).

| # | Milestone | Done when |
|---|---|---|
| M1 | Function-level translation and target census | One wasm function per guest function (`docs/design/translator.md`, ABI v2 in `docs/design/abi.md`), with guest call/ret mapped to wasm call/return. Phase 0's 201k-instruction region compiles in seconds. CI builds 7zz.exe and coremark.exe with MinGW and publishes them. `testkit census` defines the instruction scope (`docs/design/instruction-scope.md`). |
| M2 | PE loading | AOT translation of whole PE images; JS runtime with import thunks as host calls and run-time translation of missed code. A small MinGW test PE runs translated == native. 7zz.exe and coremark.exe translate and compile in V8 and wasmtime. |
| M3 | Integer instructions and CRT startup | Fixture format v2 and every in-scope integer form, plus the SSE data moves, bitwise ops and `fninit` that integer code uses. MinGW CRT startup and the integer L2 programs run translated == native. |
| M4 | SSE, x87 and CoreMark | In-scope SSE/SSE2 and x87 forms. hello.exe and a float L2 program run translated == native. CoreMark validates translated and is tracked in CI. |
| M5 | 7-Zip exactly like native | The command matrix (formats, methods, `t`, `l`, `x`, `h`) matches native Windows byte for byte. Every in-scope form has fixtures and replay is 100 %. `7z b` is tracked. |
| M6 | Speed and close-out | Register-sync and flag liveness, loop structuring, call fast path. The cost of the guest-memory checks is measured on real code. |

## Working rules

How sessions work, which tools and network hosts are available, clean-room rules and delegation are in `CLAUDE.md`.
