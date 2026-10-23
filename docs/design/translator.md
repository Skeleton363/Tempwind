# Translator design: function-level translation (Phase 1)

Status: **decided in the Phase 1 plan; implemented in M1; linking and images in M2.** The module ABI is in `abi.md` (v3). This note says how guest code maps onto wasm functions and why.

## Problem

Phase 0 put a whole code region into one wasm function: locals for the guest registers, a `loop` + `br_table` dispatcher over every block, and a compiled binary search for indirect targets. It is fast inside a loop, but it does not scale:

- Cranelift took 516 ms for one function of 10k instructions / 2k blocks, and did not finish in minutes on a 201k-instruction region.
- V8 limits a function body to 7,654,321 bytes and 50,000 locals, and TurboFan's cost also grows faster than linearly with function size.
- A real program is hundreds of thousands of instructions (7zz.exe: about 2 MB of code).

So the unit of translation has to be smaller than a region, and control transfers between units have to be cheap.

## Pipeline

1. **Decode and lift** (`lift.rs`), as in Phase 0: recursive traversal from the entry points; direct branch and call targets inside the region, branch fall-throughs and call return sites are block leaders; blocks are formed by decoding linearly. Each guest instruction lowers to IR ops; each block ends in one terminator.
2. **Flag liveness** (`ir.rs`), as in Phase 0, over the whole program: calls, returns, indirect jumps and exits are readers; direct jumps are followed into their target, even across a function or chunk boundary. That is sound because every boundary stores the whole lazy record (M1), so a record that no path reads is dead wherever the path continues.
3. **Partition into functions** (`func.rs`): group blocks into guest functions, and cut large functions into chunks.
4. **Emit** (`emit.rs`): one wasm function per chunk, the table, `lookup`, `flags`, `run`.

## Terminators

The IR keeps call and return explicit, because they map to wasm call and return:

| Guest | IR terminator | Notes |
|---|---|---|
| `jmp rel` | `Jump(target)` | |
| `jcc rel` | `Branch { cond, taken, fallthrough }` | |
| `jmp r/m64` | `Indirect(value)` | |
| `call rel32` | `Call { target: Direct(addr), ret: next }` | The ops push `next` first, as native does. |
| `call r/m64` | `Call { target: Indirect(value), ret: next }` | The target is read before the push (snapshot), as native does. |
| `ret`, `ret imm16` | `Ret(value)` | The ops load the return address and adjust RSP first; `value` is the popped address. |
| unsupported or out of region | `Exit { reason, rip }` | |

## Functions and chunks

**Function entries** are the given entry points and every direct call target inside the region. For images (M2), `Pe::function_entries()` adds the entry point, exports, `.pdata` function starts, TLS callbacks and code addresses found through relocations, and jump-table recovery adds switch targets (below).

**Block ownership.** Functions are processed in address order. Each one claims, breadth-first, the blocks it reaches through `Jump`, `Branch` and the return site of `Call`. It never follows a call target, and never enters a block that is another function's entry or already claimed. Every block belongs to exactly one function. An edge to a block of another function is a **tail transfer**. (Every block is reachable from some entry without following call targets, because the lifter only makes leaders of branch targets, return sites and call targets. A block nobody claims would still become a function of its own.)

**Every block is an entry.** The dispatch `br_table` at the top of each wasm function already lists every block (it serves backward branches), so making any block startable costs nothing in the function. `lookup` maps every block address to `(function, block index)`. This makes re-entry possible anywhere: after an unwind, at a return site whose wasm frame is gone, or at an indirect target in the middle of a function.

**Chunks.** A function with more than `Options::max_function_insns` guest instructions (the soft limit; default below) is cut into chunks:
- The units are the strongly connected components of the function's CFG (its jump, branch and return-site edges), taken in the order of their first block. They are packed into the current chunk until the next unit would take it past the soft limit; then a new chunk starts. So a cut never splits a loop nest, and hot loops stay inside one wasm function.
- A unit larger than the hard cap (`Options::max_chunk_insns`, 10× the soft limit by default) is cut anyway, in address order, into pieces of at most the hard cap. Correctness does not depend on where cuts fall: every block is an entry. (A single block is never cut, so a block longer than the hard cap is a chunk of its own.)
- A chunk's blocks are sorted by address, but a chunk need not be a contiguous address range, since loop nests can interleave.
- Edges between chunks are tail transfers.

## Lowering

### Registers and flags
- Inside a wasm function, the 16 GPRs and the lazy flag record (`cc_op`, `cc_a`, `cc_b`, `cc_r`) live in locals, as in Phase 0.
- At function entry (any entry) they are loaded from the state block.
- At every boundary (call, return, tail transfer, unwind, exit) they are stored to the state block. After a call returns with `STATUS_RETURNED` they are reloaded, since the callee may have changed any of them.
- M1 loads and stores all 20 values at every boundary. M6 narrows this with liveness (store only what was written since the last sync, reload only what is read before being written).

### Direct call to a translated function
```text
<ops: push return address>
if depth >= CALL_DEPTH_LIMIT: store; state.rip = target; return UNWIND
store
depth += 1
status = call $g(state, entry_of(target), ret_addr)
depth -= 1
if status != RETURNED: return status         ; state already stored by the callee
load
<continue at the return-site block>          ; usually the next block: a fall-through
```
A direct call to an address outside the module probes the lookup table (`abi.md` v3) and becomes an indirect call through the shared table on a hit; on a miss it stores the state, sets `state.rip = target` and returns `UNWIND`.

### Indirect call
Like a direct call, but `(func, entry) = lookup(target)` first, a probe of the process-wide lookup table:
- on a hit, the call is `call_indirect` through the shared table;
- on a miss, `state.rip = target` and `UNWIND`.

The return address is already on the guest stack either way.

### Return
```text
<ops: load return address into t, adjust RSP>
store
if t == ret: return RETURNED
state.rip = t; return UNWIND
```
In code: `rip = t; status = (t != ret)` (RETURNED is 0, UNWIND is 1), then the function's shared epilogue, which stores the registers and the flag record, and RIP unless the status is RETURNED. Unwinds and exits leave through the same epilogue; a chunk that never uses it doesn't get one.
`ret` is the function's third parameter, the address its wasm caller expects. Returns to anywhere else unwind to the trampoline, which resumes at the popped address through `lookup`. This keeps every unusual control flow correct: longjmp, C++ and SEH unwinding, return-address tricks, coroutines and stack switching. Only their speed changes.

### Jumps and branches
- To a block of the same wasm function: forward `br`, backward `br` to the dispatch loop with the block index, a self-loop as its own `loop`. This is the Phase 0 scheme.
- To a block of another wasm function: store, then `return_call $g(state, entry, ret)`. `ret` passes through unchanged, because the tail-called code returns to our caller.
- To an address outside the module: probe the lookup table; a hit becomes `return_call_indirect`, a miss stores, sets `state.rip = target` and unwinds.
- `Indirect(value)`: `lookup(value)`. A hit in the same chunk becomes a local dispatch; a hit elsewhere becomes `return_call_indirect`; a miss becomes `UNWIND`.

### Exits
`Exit { reason, rip }`: store, `state.rip = rip`, `state.exit = reason`, return `EXIT`. Callers pass `EXIT` up untouched, and `run` materializes RFLAGS. The lifter produces `Exit` for unsupported instructions and for falling off the end of the region; a jump, call or return to an address without a block unwinds instead, and the trampoline's lookup miss then exits with `EXIT_LEFT_CODE`.

### Call-depth guard
Guest recursion maps onto the wasm stack, which is about 1 MB in V8 (less in some workers). The mutable global `depth` counts nested guest calls. Past `Options::call_depth_limit` (default below), a call stores the state, sets `rip` to the target and unwinds instead. The trampoline then runs the callee at depth 0, and its eventual return unwinds again and resumes the caller at its return site. Deep recursion costs speed, never a wasm stack overflow, which would lose the registers held in locals.

Once the guard has fired, every frame below the cut returns through the trampoline: its wasm frame is gone, so its `ret` no longer matches. A recursion of depth d with limit L therefore costs about d − L trampoline round trips on the way back up (`deep_rec`, d = 100,000, still runs in milliseconds).

`call_depth_limit: None` removes the guard; the measurement of how deep V8 can go uses it. Nothing else should.

### The trampoline
See `abi.md`. It is the only place that starts guest code from the host, and the only consumer of `UNWIND` and `EXIT`.

## Invariants

1. Whenever a status other than `RETURNED` leaves a function, the state block holds the complete architectural state: GPRs, RIP, lazy flags.
2. The guest stack in guest memory holds exactly what native execution would have there: return addresses included.
3. Every translated block address is in `lookup`, so execution can resume at any block.
4. `depth` equals the number of guest calls whose wasm frames are live (or the trampoline has just reset it).

## Module layout

Function 0 is `run`, 1 `lookup`, 2 `flags`, and 3 + i is chunk i at table index `table_base + i`; globals 0-2 are the imported `table_base`, `lookup` and `depth` (see `abi.md`). Chunks are named after their guest function (`fn_0x140001000`, `fn_0x140001000_c1`, …) in the name section, so profiles show guest addresses. `Translation::functions` lists every guest function with its chunks (wasm index and block addresses), and the IR dump is grouped the same way.

## Defaults (M1)

Chosen from the M1 measurements (4-core Xeon @ 2.8 GHz cloud VM, release builds, each 3 runs; full numbers in `docs/status/phase-1.md`):

| Option | Default | Why |
|---|---|---|
| `max_function_insns` | 2000 | On Phase 0's 201k-instruction region, compile time grows with chunk size above it: Cranelift 9.2-9.8 s at 2000, 11.7-12.5 s at 5000, 14.4-16.1 s at 10000, about 19 s at 20000; V8 TurboFan (eager) 2.0-2.1 s at 2000, 2.3-2.5 s at 5000. Below it the gain is at most about 10 % (Cranelift 8.1-8.6 s at 500), for 1.6 % (1000) to 4.4 % (500) more wasm and more cross-chunk state syncs. V8's default lazy Liftoff path doesn't care (10-15 ms compile, about 53 ms to the first run at every size). |
| `max_chunk_insns` | 20000 | 10× the soft limit, as planned. |
| `call_depth_limit` | 500 | Deepest guest recursion without the guard (one guest call = one wasm frame, the frame of a small or of a 2000-instruction function): wasmtime with its default 512 KiB stack 1487; Chromium 141 Worker 1879-1919 (`deep_fat`: 2275); Chromium page 3583-3839; Node 24 main thread 3585-3711 under `--liftoff-only` (the largest frames), 8656-8963 under TurboFan. 500 leaves 3× margin in wasmtime, 3.8× in a Worker and 7× on a page, for the frames below the trampoline and, from M2, host calls that run on the same stack. |

## Linking and images (M2)

- **Several modules per process** (the AOT image, run-time translations) share the funcref table, the `depth` global and a hash-table `lookup` in guest memory, which translated code probes inline and the runtime fills (`abi.md` v3). Direct calls inside a module stay direct.
- **Host calls:** an import's IAT slot holds a thunk address whose lookup slot names the host-call function, so API calls are ordinary indirect calls into JS and don't unwind the wasm stack.
- **Whole images** (`x64wasm::image`): one module per image, decoding only inside executable sections. Unsupported instructions, among them the out-of-profile code of 7zz.exe's CPUID-dispatched functions, translate to `EXIT_UNSUPPORTED` exits like anywhere else, so they cost nothing unless reached.
- **Jump tables:** absolute tables are already covered by relocation targets. GCC's relative tables (`lea base; movsxd idx, [base + idx*4]; add idx, base; jmp idx` behind a bounds check) are recovered by pattern, and their targets become block leaders. A target that recovery misses is translated at run time.
- **Run-time translation:** a lookup miss inside an executable section returns `EXIT_LEFT_CODE` to the runtime, which translates from that address with the wasm32 build of the translator, links the module and resumes (`runtime.md`). Known addresses are passed in, so nothing is translated twice.
- **Faults (NULL check, store check):** `EXIT_FAULT` with the fault address and kind in the state block, from M3/M6.

## Measurements (M1)

Recorded in `docs/status/phase-1.md`:
- Phase 0's 201k-instruction generated region, translated and compiled in wasmtime (Cranelift) and in V8 (Node 24: `WebAssembly.compile`, then first-call and TurboFan time).
- Translation speed (guest instructions/s) and wasm bytes per guest instruction.
- `sum_loop` n=10^8 vs native: must not regress from Phase 0's 1.02-1.14×.
- Call-heavy micro-benchmarks, translated vs native, in wasmtime and Node:
  - recursive fib(30);
  - a small function called once per byte of a 1 MiB buffer, CoreMark's `crcu8` pattern.
- The deepest guest recursion V8 survives with the guard disabled, which sets `CALL_DEPTH_LIMIT` with margin.
