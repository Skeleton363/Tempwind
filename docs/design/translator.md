# Translator design: function-level translation (Phase 1)

Status: **decided in the Phase 1 plan; implemented in M1.** The module ABI is in `abi.md` (v2). This note says how guest code maps onto wasm functions and why.

## Problem

Phase 0 put a whole code region into one wasm function: locals for the guest registers, a `loop` + `br_table` dispatcher over every block, and a compiled binary search for indirect targets. It is fast inside a loop, but it does not scale:

- Cranelift took 516 ms for one function of 10k instructions / 2k blocks, and did not finish in minutes on a 201k-instruction region.
- V8 limits a function body to 7,654,321 bytes and 50,000 locals, and TurboFan's cost also grows faster than linearly with function size.
- A real program is hundreds of thousands of instructions (7zz.exe: about 2 MB of code).

So the unit of translation has to be smaller than a region, and control transfers between units have to be cheap.

## Pipeline

1. **Decode and lift** (`lift.rs`), as in Phase 0: recursive traversal from the entry points; direct branch and call targets inside the region, branch fall-throughs and call return sites are block leaders; blocks are formed by decoding linearly. Each guest instruction lowers to IR ops; each block ends in one terminator.
2. **Partition into functions** (new, `lift.rs` or a new `func.rs`): group blocks into guest functions, and cut large functions into chunks.
3. **Flag liveness** (`ir.rs`), as in Phase 0, per wasm function, treating every boundary as a reader.
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

**Function entries** are the given entry points and every direct call target inside the region. (M2 adds exports, `.pdata` function starts, TLS callbacks and address-taken code found through relocations.)

**Block ownership.** Functions are processed in address order. Each one claims, breadth-first, the blocks it reaches through `Jump`, `Branch` and the return site of `Call`. It never follows a call target, and never enters a block that is another function's entry or already claimed. Every block belongs to exactly one function. An edge to a block of another function is a **tail transfer**.

**Every block is an entry.** The dispatch `br_table` at the top of each wasm function already lists every block (it serves backward branches), so making any block startable costs nothing in the function. `lookup` maps every block address to `(function, block index)`. This makes re-entry possible anywhere: after an unwind, at a return site whose wasm frame is gone, or at an indirect target in the middle of a function.

**Chunks.** A function with more than `Options::max_function_insns` guest instructions (default set by measurement, starting at about 2000) is cut into chunks, in block address order:
- A cut never splits a strongly connected component of the function's CFG (a loop nest), so hot loops stay inside one wasm function.
- An SCC larger than the hard cap (`Options::max_chunk_insns`, about 10× the soft limit) is cut anyway; correctness does not depend on where cuts fall.
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
A direct call to an address without a translated block (outside the region, or into a different module in M2) stores the state, sets `state.rip = target` and returns `UNWIND`.

### Indirect call
Like a direct call, but `(func, entry) = lookup(target)` first:
- on a hit, the call is `call_indirect` through the table;
- on a miss, `state.rip = target` and `UNWIND`.

The return address is already on the guest stack either way.

### Return
```text
<ops: load return address into t, adjust RSP>
store
if t == ret: return RETURNED
state.rip = t; return UNWIND
```
`ret` is the function's third parameter, the address its wasm caller expects. Returns to anywhere else unwind to the trampoline, which resumes at the popped address through `lookup`. This keeps every unusual control flow correct: longjmp, C++ and SEH unwinding, return-address tricks, coroutines and stack switching. Only their speed changes.

### Jumps and branches
- To a block of the same wasm function: forward `br`, backward `br` to the dispatch loop with the block index, a self-loop as its own `loop`. This is the Phase 0 scheme.
- To a block of another wasm function: store, then `return_call $g(state, entry, ret)`. `ret` passes through unchanged, because the tail-called code returns to our caller.
- To an address without a block: store, `state.rip = target`, `UNWIND`.
- `Indirect(value)`: `lookup(value)`. A hit in the same function becomes a local dispatch; a hit elsewhere becomes `return_call_indirect`; a miss becomes `UNWIND`.

### Exits
`Exit { reason, rip }`: store, `state.rip = rip`, `state.exit = reason`, return `EXIT`. Callers pass `EXIT` up untouched, and `run` materializes RFLAGS.

### Call-depth guard
Guest recursion maps onto the wasm stack, which is about 1 MB in V8 (less in some workers). The mutable global `depth` counts nested guest calls. Past `CALL_DEPTH_LIMIT` (default set by measurement, about 1000), a call stores the state, sets `rip` to the target and unwinds instead. The trampoline then runs the callee at depth 0, and its eventual return unwinds again and resumes the caller at its return site. Deep recursion costs speed, never a wasm stack overflow, which would lose the registers held in locals.

### The trampoline
See `abi.md`. It is the only place that starts guest code from the host, and the only consumer of `UNWIND` and `EXIT`.

## Invariants

1. Whenever a status other than `RETURNED` leaves a function, the state block holds the complete architectural state: GPRs, RIP, lazy flags.
2. The guest stack in guest memory holds exactly what native execution would have there: return addresses included.
3. Every translated block address is in `lookup`, so execution can resume at any block.
4. `depth` equals the number of guest calls whose wasm frames are live (or the trampoline has just reset it).

## What M2 and later change

- **Several modules per process** (AOT image, run-time translations): the table becomes imported and shared, and `lookup` becomes a hash table in the host region of guest memory, probed inline by translated code and filled by the runtime. Direct calls inside a module stay direct.
- **Host calls** (import thunks): a wasm import called inline from translated code, so API calls do not unwind the wasm stack.
- **Faults** (NULL check, store check): `EXIT_FAULT` with the fault address and kind in the state block.

## Measurements (M1)

Recorded in `docs/status/phase-1.md`:
- Phase 0's 201k-instruction generated region, translated and compiled in wasmtime (Cranelift) and in V8 (Node 24: `WebAssembly.compile`, then first-call and TurboFan time).
- Translation speed (guest instructions/s) and wasm bytes per guest instruction.
- `sum_loop` n=10^8 vs native: must not regress from Phase 0's 1.02-1.14×.
- Call-heavy micro-benchmarks, translated vs native, in wasmtime and Node:
  - recursive fib(30);
  - a small function called once per byte of a 1 MiB buffer, CoreMark's `crcu8` pattern.
- The deepest guest recursion V8 survives with the guard disabled, which sets `CALL_DEPTH_LIMIT` with margin.
