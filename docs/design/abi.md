# Translated-module ABI (v3, Phase 1)

This is the contract between the translator (`crates/x64wasm`, `crates/x64wasm-rt`), the oracle and test hosts (`crates/testkit`) and the JS runtime (`runtime/`). If you change it, update this file and every user in the same change. The constants live in `crates/x64wasm/src/abi.rs` and `runtime/src/abi.mjs`.

- **v1 (Phase 0)** put a whole code region into one wasm function.
- **v2 (M1)** made one wasm function per guest function, with guest calls and returns mapped to wasm calls and returns. The design is in `translator.md`.
- **v3 (M2)** links modules together. Several modules (the AOT translation of an image, and translations made at run time) share one funcref table, one call-depth counter and one block lookup table in guest memory, so control passes between them without leaving wasm. Host functions (the Win32 shim, `runtime.md`) sit in the same table and are called like guest functions. The state block, the statuses and the trampoline are as in v2.

## Module shape

A translated module imports, in this order:

| Import | Type | Use |
|---|---|---|
| `env.memory` | memory64 (`i64` addresses) | Guest memory. Guest virtual address = offset (identity mapping, `guest-memory.md`). Non-shared by default; shared with `Options::shared_memory`, with a maximum of 262144 pages (16 GiB). The import's minimum is 1 page. |
| `env.table` | table of `funcref`, `i32` index, minimum 0 | The process-wide table: host functions and the chunks of every module. |
| `env.table_base` | global `i32`, immutable | Table index of this module's chunk 0. |
| `env.lookup` | global `i64`, immutable | Guest address of the lookup header (below). |
| `env.depth` | global `i32`, mutable | Nested guest calls on the wasm stack (call-depth guard), shared by all modules. |

- **Exports:** `run: (state: i64) -> i32`, unchanged since v1. `state` is the guest address of a CPU state block. `run` executes translated code from `state.rip` until an exit, stores the registers back and returns the exit reason.
- **Custom section `x64wasm.blocks`:** the module's blocks, which the host inserts into the lookup table (format below).
- **Requires** the Wasm 3.0 tail calls (`return_call`, `return_call_indirect`) and memory64 (threads when shared). Node 24, Chromium 141 and wasmtime 50 have them.

**Inside the module** (not host contract):
- Functions:
  - function 0 is `run`, 1 `lookup`, 2 `flags`;
  - 3 + i is chunk i, at table index `table_base + i`, which an active element segment with offset `global.get $table_base` writes;
  - helper functions such as `div128` (128-by-64-bit division) follow the chunks; they are emitted only when used and are not in the table;
  - guest functions (chunks) have the type `(state: i64, entry: i32, ret: i64) -> i32`. `entry` is the index of the block to start at, and every block is a valid entry. `ret` is the return address the caller expects: a `ret` that pops exactly this value returns to the wasm caller.
- Globals 0-2 are the imported `table_base`, `lookup` and `depth`. Only `run` is exported.
- The name section names every function, local, the table and the globals. A chunk is named after its guest function's entry: `fn_0x140001000`, or `fn_0x140001000_c1` and so on for the chunks of a large function that don't hold the entry.

## Linking a module (host steps)

1. Read `x64wasm.blocks`, and grow `env.table` to at least `table_base + chunks` slots.
2. Instantiate with the five imports.
3. Insert every block of `x64wasm.blocks` into the lookup table: key `addr`, func `table_base + chunk`, entry `entry`. If the key is already there, keep the existing slot: the first translation of an address wins.

A module's blocks are reachable only after step 3.

## Lookup table

**Header** at `env.lookup` (32 bytes, 8-byte aligned):

| Offset | Type | Field |
|---|---|---|
| `+0` | u64 | `slots`: guest address of the slot array, 16-byte aligned |
| `+8` | u64 | `mask`: number of slots − 1; the number of slots is a power of two |
| `+16` | u64 | `used`: occupied slots. Kept by the host; translated code ignores it. |
| `+24` | u64 | reserved, 0 |

**Slot** (16 bytes): `key` u64 (a guest address; 0 = empty), `func` u32 (table index), `entry` u32.

**Probe for address `a`:**

```text
i = ((a * 0x9E37_79B9_7F4A_7C15) mod 2^64 >> 32) & mask
loop:
  key = slot[i].key
  key == a -> hit: (slot[i].func, slot[i].entry)
  key == 0 -> miss
  i = (i + 1) & mask
```

Translated code reads the header on every probe.

**Host rules:**
- Keep `2 * used <= mask + 1`. To grow, build a new slot array, then update `slots` and `mask`.
- Keys are never removed in Phase 1.
- Phase 1 runs one thread, so no locking.
- Address 0 is never a key: the NULL region holds no code.

## Transfers

- **To a block of the same module:** as in v2. Inside a chunk this is `br` or the dispatch loop; to another chunk it is `call` or `return_call` with that chunk's function index.
- **To any other address** (direct calls and jumps whose target is outside the module, and all indirect calls and jumps):
  - probe the lookup table;
  - on a hit, `call_indirect` or `return_call_indirect` through `env.table` with the slot's func and entry (a hit in the current chunk is a local dispatch);
  - on a miss, store the state, set `state.rip` to the target and return `STATUS_UNWIND`.
- So once the host has inserted a run-time translation's blocks, the image calls it and it calls the image, all without unwinding.

## Host functions

A host function is a wasm function of the guest-function type in `env.table`. In JS it is an imported JS function that a tiny module re-exports. Lookup slots map guest addresses (the import thunks, `runtime.md`) to it, with entry = an id the host chooses.

Translated code calls it exactly like a guest function:
- the state block holds the GPRs and the lazy flag record (RIP isn't stored);
- `[RSP]` holds the return address the guest pushed;
- `ret` is what the wasm caller expects.

It does its work, reading arguments by the Win64 convention: RCX, RDX, R8, R9, then argument i ≥ 4 at `[RSP + 0x28 + 8 * (i - 4)]`. Then it ends in one of two ways:
- **Return to the guest:** write the results (RAX) into the state block, then perform the guest's `ret`: `t = [RSP]`, `RSP += 8`. If `t == ret`, return `STATUS_RETURNED`. Otherwise store `state.rip = t` and return `STATUS_UNWIND`.
- **Stop:** store `state.rip` (the thunk address) and `state.exit = EXIT_HOST`, and return `STATUS_EXIT`.

Other GPRs and the lazy flag record stay unchanged.

## Statuses (between translated functions)

Every guest function returns one of:

| Value | Name | Meaning | State block on return |
|---|---|---|---|
| 0 | `STATUS_RETURNED` | A `ret` popped exactly the `ret` argument. The caller continues at its return site. | GPRs and the lazy flag record are stored. RIP is not written. |
| 1 | `STATUS_UNWIND` | Control continues at `state.rip`, which the trampoline looks up. Used for returns to an unexpected address, lookup misses and calls past the depth limit. | Full state, RIP included. |
| 2 | `STATUS_EXIT` | Stop and return to the host with the reason in `state.exit`. | Full state, RIP and exit reason included. |

A caller that gets a status other than 0 from a call stores nothing further and passes the status up unchanged. After `STATUS_RETURNED` it reloads the registers and the lazy flag record from the state block, since the callee may have changed any of them. The trampoline is the only consumer of 1 and 2.

## Trampoline (`run`)

1. Initialize the lazy flag record from RFLAGS: `cc_op = 0` (materialized), `cc_a = rflags`. Set `depth = 0`.
2. Loop:
   - Probe the lookup table for `state.rip`. On a miss, the exit reason is `EXIT_LEFT_CODE`.
   - Otherwise, call the slot's function through the table with its entry and `ret = NO_RET`.
   - `STATUS_UNWIND`: set `depth = 0` and continue the loop.
   - `STATUS_RETURNED`: only possible if the guest returned to `NO_RET` itself. Set `state.rip = NO_RET` and continue; the probe then misses.
   - `STATUS_EXIT`: the reason is in `state.exit`.
3. Materialize RFLAGS: `rflags = (rflags & !STATUS) | flags(cc_op, cc_a, cc_b, cc_r)`. Store the exit reason in `state.exit` and return it.

`NO_RET = 0xFFFF_FFFF_FFFF_FFFF`, a non-canonical address that no `call` can push. Every module's `run` behaves the same, so a host may use any of them.

## CPU state block

All fields are little-endian. Size: `0x400` bytes. It must be 16-byte aligned; 64-byte alignment is recommended.

| Offset | Size | Field | Notes |
|---|---|---|---|
| `0x000` | 16 × 8 | GPRs in x86 encoding order: RAX, RCX, RDX, RBX, RSP, RBP, RSI, RDI, R8 … R15 | |
| `0x080` | 8 | RIP | |
| `0x088` | 8 | RFLAGS | Host view. On exit, the status flags (CF PF AF ZF SF OF) hold the architectural result and every other bit is preserved from entry. While translated code runs, the status flags live in the lazy record below; the other bits (DF, …) stay here. |
| `0x090` | 4 | exit reason | The same value `run` returns. |
| `0x094` | 4 | `cc_op` | Lazy flag record kind: 0 = materialized in `cc_a`, else `1 + 4*op + width` (`ir::FlagOp::cc_op`). Valid at function boundaries during a run; `run` sets it up from RFLAGS on entry. |
| `0x098` | 8 | `cc_a` | Lazy record operand a (or the materialized flags). |
| `0x0a0` | 8 | `cc_b` | Lazy record operand b. |
| `0x0a8` | 8 | `cc_r` | Lazy record result. |
| `0x0b0` | 8 | FS base | Used by FS-relative addressing (M3). |
| `0x0b8` | 8 | GS base | Used by GS-relative addressing (M3). The runtime sets it to the TEB. |
| `0x0c0` | 8 | fault address | For `EXIT_FAULT`. |
| `0x0c8` | 4 | fault kind | For `EXIT_FAULT`. Memory faults: `FAULT_READ` 1, `FAULT_WRITE` 2, `FAULT_EXECUTE` 3. Exceptions: `FAULT_DIVIDE` 4 (#DE), `FAULT_INVALID_OPCODE` 5 (#UD), `FAULT_GENERAL_PROTECTION` 6 (#GP). |
| `0x0cc` | 0x34 | reserved, zero | |
| `0x100` | 16 × 16 | XMM0-XMM15 | From M3, the XMM registers live here while translated code runs, read and written with `v128` loads and stores. M4 may cache them in locals, synced at boundaries like the GPRs. |
| `0x200` | 4 | MXCSR | Lives here from M3 (`ldmxcsr`, `stmxcsr`; M4's SSE arithmetic reads it). |
| `0x204` | 2 | x87 FCW | Lives here from M3 (`fninit`, `fnstcw`, `fldcw`). |
| `0x206` | 2 | x87 FSW | TOP in bits 11-13. `fninit` resets it from M3; M4 uses it. |
| `0x208` | 2 | x87 tag word (full, 2 bits per register) | `fninit` sets it to empty (0xFFFF) from M3; M4 uses it. |
| `0x20a` | 6 | reserved, zero | |
| `0x210` | 8 × 16 | x87 physical registers R0-R7, 80 bits each in a 16-byte slot | Physical order, not ST(i) order: ST(i) = R[(TOP + i) mod 8]. From M3, translated code preserves them (`fninit` leaves their contents; replay compares them); M4 uses them. |
| `0x290` | 0x170 | reserved, zero | Room for YMM upper halves and other state later. |

Hosts allocate `STATE_SIZE` bytes and zero the reserved fields.

## Instruction semantics (Phase 1)

- **`lock`:** an ordinary read-modify-write, which is exact with one thread. Atomic lowering comes with threads (Phase 4).
- **Segment overrides:** FS and GS add the base from the state block (`0x0b0`, `0x0b8`) to the effective address. CS, DS, ES and SS overrides change nothing in 64-bit mode.
- **Address-size override (67h):** the effective address is computed in 32 bits and zero-extended.
- **DF and the other non-status flags** live in the state block's RFLAGS while code runs. `std` and `cld` write it, and string instructions read it.
- **Undefined results:** where the SDM leaves a flag or a register undefined, translated code may produce any value, and fixtures don't check it.

## Exit reasons

| Value | Name | Meaning |
|---|---|---|
| 1 | `EXIT_LEFT_CODE` | Control reached a guest address with no block in the lookup table: a fall-through past the code, a target nothing translated yet, or a `ret` to an unknown address. `rip` holds that address. Hosts translate more code (run-time translation) or stop. |
| 2 | `EXIT_UNSUPPORTED` | The instruction at `rip` is not supported yet. Registers hold the state before it. |
| 3 | `EXIT_FAULT` | A guest fault: fault address and kind in the state block. RIP is the faulting instruction, and every register holds its value from before it. Translated code emits it from M3 for #DE (`div`, `idiv`), #UD (`ud2` and bytes that don't decode) and #GP, with fault address 0. An instruction that decodes but is outside the CPUID profile stays `EXIT_UNSUPPORTED`. M2's runtime raises it for execution outside executable sections. NULL and store checks come with M6. |
| 4 | `EXIT_HOST` | A host function stopped execution: process exit, an unimplemented API, a shim error. The host knows why. |

A test function returns to the host by `ret`ing to a sentinel return address that the host pushed (`SENTINEL_RET = 0x0000_7FFF_FFFF_0000`). The host then expects exit 1 with `rip == SENTINEL_RET`.

## `x64wasm.blocks` section

Little-endian:

| Offset | Type | Field |
|---|---|---|
| `+0` | u32 | version, 1 |
| `+4` | u32 | `chunks`: number of chunk functions, i.e. the table slots the module needs |
| `+8` | u32 | `n`: number of blocks |
| `+12` | u32 | reserved, 0 |
| `+16` | n × 16 | blocks, sorted by address: `addr` u64, `chunk` u32 (relative to `table_base`), `entry` u32 |

Every translated block appears exactly once.

## Run-time translator (`x64wasm_rt.wasm`)

`crates/x64wasm-rt` builds the translator for `wasm32-unknown-unknown` as a cdylib. Besides `memory`, it exports:

| Export | Use |
|---|---|
| `rt_alloc(size: i32) -> i32` | A 16-byte-aligned buffer in the translator's memory. |
| `rt_free(ptr: i32, size: i32)` | Frees a buffer from `rt_alloc`. |
| `rt_translate(request: i32) -> i32` | Translates, and returns a pointer to the result. The result stays valid until the next call. |

**Request** (40 bytes, little-endian):

| Offset | Type | Field |
|---|---|---|
| `+0` | u64 | `code_base`: guest address of the first code byte |
| `+8` | u32 | `code_ptr`: the code bytes, in the translator's memory |
| `+12` | u32 | `code_len` |
| `+16` | u64 | `entry`: guest address to translate from |
| `+24` | u32 | `known_ptr`: sorted u64 array of guest addresses that already have blocks |
| `+28` | u32 | `known_len`: number of addresses in it |
| `+32` | u32 | flags, 0 |
| `+36` | u32 | reserved, 0 |

**Result** (20 bytes):

| Offset | Type | Field |
|---|---|---|
| `+0` | u32 | status: 0 ok, 1 error |
| `+4` | u32 | `wasm_ptr` |
| `+8` | u32 | `wasm_len`: a v3 module with `x64wasm.blocks` (status 0) |
| `+12` | u32 | `msg_ptr` |
| `+16` | u32 | `msg_len`: UTF-8 error text (status 1) |

**Semantics:**
- It translates, with `Options::default()`, what is reachable from `entry` inside `[code_base, code_base + code_len)`.
- An address in `known` counts as outside the code: a transfer to it, or a fall-through into it, is a lookup transfer, and nothing is translated twice.
- Status 1 (error) when `entry` is itself in `known`, `flags` is not 0, `known` isn't sorted, the code range wraps past the end of the address space, or translation fails.

## Test hosts

`crates/testkit` (wasmtime) and `runtime/src/run.mjs` (Node, browser) run spike cases and fixtures, one module per instance. For each instance they:
- create the table and the globals;
- place the lookup header and slots in pages they add after the case's `memoryBytes`;
- link the module as above.

The guest code of a case sees no difference from v2.

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
- `rflags` is set on entry. On exit, `rflags` is checked only if `expect.rflags` is present, under `expect.rflagsMask`.
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
| `cargo run -p testkit --release -- census FILE... [--check]` | Instruction and import census of PE files |
| `cargo run -p x64wasm-cli --release -- translate-pe FILE --out-dir DIR [--entries all\|entry]` | AOT-translate a PE image: `DIR/<stem>.wasm` (v3 module) and `DIR/<stem>.plan.json` (load plan, `runtime.md`) |
| `cargo build -p x64wasm-rt --release --target wasm32-unknown-unknown` | The run-time translator, `target/wasm32-unknown-unknown/release/x64wasm_rt.wasm` |
| `tests/pe/build.sh` | Build the M2 test program `target/pe/basic.exe`; its expected output is in `tests/pe/` |
| `npm ci` then `npm run test:node` | Node: spike cases and the PE runtime tests |
| `npm run test:browser` | Playwright, headless Chromium: translator spike, threads spike, WebGPU spike |
| `native/spike/run.sh` | Emscripten probe (Docker `emscripten/emsdk`) vs native gcc. Fails on any LP64 mismatch. |
| `tests/native/build.sh` | MinGW-build `target/native/hello.exe`. Expected output is in `tests/native/hello.expected`. |
| `scripts/fetch-artifact.sh TAG ASSET [DIR]` | Download a release asset of this repo |
