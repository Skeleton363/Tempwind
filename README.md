# Tempwind

Tempwind (working name) runs unmodified 64-bit Windows programs in the browser. It works by translating x86-64 to WebAssembly, running Wine 11 underneath, and mapping Direct3D to WebGPU. Everything runs on your own device and is served from static hosting. You bring your own programs; this project hosts only open-source code.

**Status:** Phase 0 (foundations). Nothing runs Windows programs yet.

- Plan: [docs/plan.md](docs/plan.md)
- Status per phase: [docs/status/](docs/status/)
- How the project is built (by AI sessions, one phase at a time): [CLAUDE.md](CLAUDE.md)

## Layout

| Path | What |
|---|---|
| `crates/x64wasm` | x86-64 → WebAssembly translator (decode, IR, Wasm) |
| `crates/x64wasm-cli` | CLI: translate, dump IR and WAT |
| `crates/testkit` | Instruction oracle, fixtures, replay |
| `runtime/` | JS runtime (Node and browser hosts) |
| `native/` | Wine's Unix side built with Emscripten (Phase 2+) |
| `web/` | Desktop shell (Phase 3+) |
| `tests/` | Program, Wine and browser tests |
| `docs/` | Plan, design notes, status |

## License

The project's own code is MIT (see [LICENSE](LICENSE)). Wine and other dependencies keep their own licenses. Our patches to Wine are published under the LGPL.
