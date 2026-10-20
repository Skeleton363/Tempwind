# Translated-module ABI (Phase 0)

This is the contract between the translator (`crates/x64wasm`), the oracle (`crates/testkit`) and the JS hosts (`runtime/`). If you change it, update this file and every user in the same change. The constants live in `crates/x64wasm/src/abi.rs`.

## Module shape

A translated module:

- **Imports** `env.memory`, a memory64 linear memory (`i64` addresses).
  - It is non-shared by default. With `Options::shared_memory` it is shared, and its maximum is 262144 pages (16 GiB).
  - Its minimum is 1 page. Hosts may provide a larger memory.
- **Exports** `run: (state: i64) -> i32`.
  - `state` is the guest address of a CPU state block (below).
  - `run` loads the guest registers from the block, executes translated code starting at `state.rip`, stores the registers back, and returns an exit reason.
- Guest virtual address equals the offset in `env.memory`, i.e. identity mapping (see `guest-memory.md`).

## CPU state block

All fields are little-endian. Size: `0x200` bytes. It must be 16-byte aligned.

| Offset | Size | Field |
|---|---|---|
| `0x000` | 16 × 8 | GPRs in x86 encoding order: RAX, RCX, RDX, RBX, RSP, RBP, RSI, RDI, R8 … R15 |
| `0x080` | 8 | RIP |
| `0x088` | 8 | RFLAGS. On exit, status flags (CF PF AF ZF SF OF) hold the architectural result. Every other bit is preserved from entry. |
| `0x090` | 4 | exit reason (the same value `run` returns) |
| `0x094` | 0x6C | reserved, zero |
| `0x100` | 16 × 16 | XMM0-XMM15 (reserved for Phase 1) |

## Exit reasons

| Value | Name | Meaning |
|---|---|---|
| 1 | `EXIT_LEFT_CODE` | Control reached a guest address with no translated block: a fall-through past the region, a branch/call target outside it, or a `ret` to an unknown address. `rip` holds that address. Hosts translate more code or stop. |
| 2 | `EXIT_UNSUPPORTED` | The instruction at `rip` is not supported yet. Registers hold the state before it. |

A test function returns to the host by `ret`ing to a sentinel return address that the host pushed (`SENTINEL_RET = 0x0000_7FFF_FFFF_0000`). The host then expects exit 1 with `rip == SENTINEL_RET`.

## Spike manifest (`testkit spike --out DIR`)

`testkit spike` writes `DIR/<name>.wasm` plus `DIR/manifest.json`. The JS hosts run every case and compare results. u64 values are `"0x…"` hex strings.

```json
{
  "abi": { "stateSize": 512, "gpr": 0, "rip": 128, "rflags": 136, "exit": 144 },
  "cases": [
    {
      "name": "sum_loop",
      "wasm": "sum_loop.wasm",
      "memoryBytes": "0x400000",
      "state": "0x300000",
      "init": {
        "rip": "0x100000",
        "rflags": "0x202",
        "regs": { "rcx": "0x64", "rsp": "0x2ff000" },
        "mem": [ { "addr": "0x2ff000", "hex": "0000ffff ff7f0000" } ]
      },
      "expect": {
        "exit": 1,
        "rip": "0x7fffffff0000",
        "regs": { "rax": "0x13ba" },
        "mem": [ { "addr": "0x200000", "hex": "…" } ]
      }
    }
  ]
}
```

- `regs` lists only the registers that are set or checked. Unlisted registers start at 0 and aren't checked.
- `rflags` is set on entry. On exit only `expect.rflags` (if present) is checked, under `expect.rflagsMask`.
- Hosts allocate at least `memoryBytes` of memory64, write the init memory and state, call `run(state)`, then check `exit`, `rip`, `regs` and `mem`.
- `hex` may contain spaces, which are ignored.

## Commands (contract between subagents and CI)

| Command | What |
|---|---|
| `cargo fmt --all --check` / `cargo clippy --workspace --all-targets -- -D warnings` | Lint |
| `cargo test --workspace` | Translator tests (wasmtime) and replay of every L1 fixture |
| `cargo run -p testkit --release -- record` | Re-record L1 fixtures natively (x86-64 Linux only) into `crates/testkit/fixtures/l1/` |
| `cargo run -p testkit --release -- record --check` | Re-record in memory and fail if anything differs from the committed fixtures |
| `cargo run -p testkit --release -- replay` | Replay fixtures through the translator and print a summary |
| `cargo run -p testkit --release -- spike --out target/spike` | Write spike wasm and manifest for the JS hosts |
| `npm ci` then `npm run test:node` | Node: run `target/spike` cases |
| `npm run test:browser` | Playwright, headless Chromium: translator spike, threads spike, WebGPU spike |
| `native/spike/run.sh` | Emscripten probe (Docker `emscripten/emsdk`) vs native gcc. Fails on any LP64 mismatch. |
| `tests/native/build.sh` | MinGW-build `target/native/hello.exe`. Expected output is in `tests/native/hello.expected`. |
| `scripts/fetch-artifact.sh TAG ASSET [DIR]` | Download a release asset of this repo |
