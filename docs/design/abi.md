# Translated-module ABI (v2, Phase 1)

This is the contract between the translator (`crates/x64wasm`), the oracle and test hosts (`crates/testkit`) and the JS hosts (`runtime/`). If you change it, update this file and every user in the same change. The constants live in `crates/x64wasm/src/abi.rs` and `runtime/src/abi.mjs`.

v1 (Phase 0) put a whole code region into one wasm function. v2 keeps the host-facing surface of v1 (`run(state)`, the GPR/RIP/RFLAGS/exit fields, the exit reasons) and changes the inside of the module: one wasm function per guest function, with guest calls and returns mapped to wasm calls and returns. The design and its reasons are in `translator.md`.

## Module shape

A translated module:

- **Imports** `env.memory`, a memory64 linear memory (`i64` addresses).
  - It is non-shared by default. With `Options::shared_memory` it is shared, and its maximum is 262144 pages (16 GiB).
  - Its minimum is 1 page. Hosts may provide a larger memory.
- **Exports** `run: (state: i64) -> i32`, unchanged from v1.
  - `state` is the guest address of a CPU state block (below).
  - `run` executes translated code starting at `state.rip` until an exit, stores the registers back, and returns the exit reason.
- Guest virtual address equals the offset in `env.memory`, i.e. identity mapping (see `guest-memory.md`).
- **Requires** the Wasm 3.0 tail-call instructions (`return_call`, `return_call_indirect`) in addition to memory64 (and threads when shared). Node 24, Chromium 141 and wasmtime 50 have them.

Internal to the module, and not part of the host contract until M2 links modules together:

| Item | Shape | Use |
|---|---|---|
| Guest functions | `(state: i64, entry: i32, ret: i64) -> i32` (one type for all) | One per guest function, or per chunk of a large one. `entry` is the index of the block to start at; every block is a valid entry. `ret` is the return address the caller expects; a `ret` that pops exactly this value returns to the wasm caller. |
| Table | `funcref`, every guest function | `call_indirect` / `return_call_indirect` for indirect transfers and the trampoline. |
| `lookup` | `(rip: i64) -> (func: i32, entry: i32)` | Guest address of any translated block to (table index, entry). `func = -1` if there is none. |
| `flags` | `(op: i32, a: i64, b: i64, r: i64) -> i64` | Materializes the status flags of a lazy flag record (as in v1). |
| `depth` | mutable `i32` global | Number of nested guest calls on the wasm stack (call-depth guard). |

Index layout in M1: function 0 is `run`, 1 `lookup`, 2 `flags`, and 3 + i is chunk i, which sits at table index i (one table, exactly as long as the number of chunks). Global 0 is `depth`. Nothing but `run` is exported. The name section names every function, local, the table (`chunks`) and the global; a chunk is named after its guest function's entry, `fn_0x140001000`, or `fn_0x140001000_c1` and so on for the chunks of a large function that don't hold the entry.

## Statuses (between translated functions)

Every guest function returns one of:

| Value | Name | Meaning | State block on return |
|---|---|---|---|
| 0 | `STATUS_RETURNED` | A `ret` popped exactly the `ret` argument. The caller continues at its return site. | GPRs and the lazy flag record are stored. RIP is not written. |
| 1 | `STATUS_UNWIND` | Control continues at `state.rip`, which the trampoline looks up. Used for returns to an unexpected address, indirect targets this module doesn't know, direct targets outside the module, and calls past the depth limit. | Full state, RIP included. |
| 2 | `STATUS_EXIT` | Stop and return to the host with the reason in `state.exit`. | Full state, RIP and exit reason included. |

A caller that gets a status other than 0 from a call stores nothing further and passes the status up unchanged. The trampoline is the only consumer of 1 and 2.

## Trampoline (`run`)

1. Initialize the lazy flag record from RFLAGS: `cc_op = 0` (materialized), `cc_a = rflags`. Set `depth = 0`.
2. Loop:
   - `lookup(state.rip)`. On a miss: exit reason `EXIT_LEFT_CODE`.
   - Call the function through the table with `entry` from the lookup and `ret = NO_RET`.
   - `STATUS_UNWIND`: set `depth = 0`, continue the loop.
   - `STATUS_RETURNED`: only possible if the guest returned to `NO_RET` itself; set `state.rip = NO_RET` and continue (the lookup then misses).
   - `STATUS_EXIT`: the reason is in `state.exit`.
3. Materialize RFLAGS: `rflags = (rflags & !STATUS) | flags(cc_op, cc_a, cc_b, cc_r)`. Store the exit reason in `state.exit` and return it.

`NO_RET = 0xFFFF_FFFF_FFFF_FFFF`, a non-canonical address no `call` can push.

## CPU state block

All fields are little-endian. Size: `0x400` bytes (v1: `0x200`). It must be 16-byte aligned; 64-byte alignment is recommended.

| Offset | Size | Field | Notes |
|---|---|---|---|
| `0x000` | 16 × 8 | GPRs in x86 encoding order: RAX, RCX, RDX, RBX, RSP, RBP, RSI, RDI, R8 … R15 | |
| `0x080` | 8 | RIP | |
| `0x088` | 8 | RFLAGS | Host view. On exit, status flags (CF PF AF ZF SF OF) hold the architectural result and every other bit is preserved from entry. While translated code runs, the status flags live in the lazy record below; non-status bits (DF, …) stay here. |
| `0x090` | 4 | exit reason | The same value `run` returns. |
| `0x094` | 4 | `cc_op` | Lazy flag record kind: 0 = materialized in `cc_a`, else `1 + 4*op + width` (`ir::FlagOp::cc_op`). Valid at function boundaries during a run; `run` sets it up from RFLAGS on entry. |
| `0x098` | 8 | `cc_a` | Lazy record operand a (or the materialized flags). |
| `0x0a0` | 8 | `cc_b` | Lazy record operand b. |
| `0x0a8` | 8 | `cc_r` | Lazy record result. |
| `0x0b0` | 8 | FS base | Reserved for M3 (FS-relative addressing). |
| `0x0b8` | 8 | GS base | Reserved for M3 (GS-relative addressing; the TEB on Windows). |
| `0x0c0` | 8 | fault address | Reserved for fault exits. |
| `0x0c8` | 4 | fault kind | Reserved for fault exits. |
| `0x0cc` | 0x34 | reserved, zero | |
| `0x100` | 16 × 16 | XMM0-XMM15 | Reserved for M3/M4. |
| `0x200` | 4 | MXCSR | Reserved for M3/M4. |
| `0x204` | 2 | x87 FCW | Reserved for M3/M4. |
| `0x206` | 2 | x87 FSW | Reserved; TOP in bits 11-13. |
| `0x208` | 2 | x87 tag word (full, 2 bits per register) | Reserved. |
| `0x20a` | 6 | reserved, zero | |
| `0x210` | 8 × 16 | x87 physical registers R0-R7, 80 bits each in a 16-byte slot | Reserved. Physical order, not ST(i) order: ST(i) = R[(TOP + i) mod 8]. |
| `0x290` | 0x170 | reserved, zero | Room for YMM upper halves and other state later. |

Hosts allocate `STATE_SIZE` bytes and zero the reserved fields.

## Exit reasons

| Value | Name | Meaning |
|---|---|---|
| 1 | `EXIT_LEFT_CODE` | Control reached a guest address with no translated block: a fall-through past the region, a branch/call target outside it, or a `ret` to an unknown address. `rip` holds that address. Hosts translate more code or stop. |
| 2 | `EXIT_UNSUPPORTED` | The instruction at `rip` is not supported yet. Registers hold the state before it. |
| 3 | `EXIT_FAULT` | Reserved: a guest fault (fault address and kind in the state block). |

A test function returns to the host by `ret`ing to a sentinel return address that the host pushed (`SENTINEL_RET = 0x0000_7FFF_FFFF_0000`). The host then expects exit 1 with `rip == SENTINEL_RET`.

## Spike manifest (`testkit spike --out DIR`)

`testkit spike` writes `DIR/<name>.wasm` plus `DIR/manifest.json`. The JS hosts run every case and compare results. u64 values are `"0x…"` hex strings.

```json
{
  "abi": { "stateSize": 1024, "gpr": 0, "rip": 128, "rflags": 136, "exit": 144 },
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
| `cargo run -p testkit --release -- spike --bench --out target/bench/translator` | Write the translator measurement modules and their manifest (the spike format plus `options`, `nativeMs`, `guestInstructions`, `functions`, `chunks`) for `node tests/bench/translator/bench.mjs` |
| `node tests/bench/translator/bench.mjs` / `… --browser` | Translator measurements in V8: Node under four V8 configurations, or the recursion-depth probes in Chromium page and Worker |
| `cargo run -p testkit --release -- census FILE...` | Instruction and import census of PE files (M1) |
| `npm ci` then `npm run test:node` | Node: run `target/spike` cases |
| `npm run test:browser` | Playwright, headless Chromium: translator spike, threads spike, WebGPU spike |
| `native/spike/run.sh` | Emscripten probe (Docker `emscripten/emsdk`) vs native gcc. Fails on any LP64 mismatch. |
| `tests/native/build.sh` | MinGW-build `target/native/hello.exe`. Expected output is in `tests/native/hello.expected`. |
| `scripts/fetch-artifact.sh TAG ASSET [DIR]` | Download a release asset of this repo |
