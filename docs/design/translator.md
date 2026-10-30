# Translator design: function-level translation (Phase 1)

Status: **decided in the Phase 1 plan; implemented in M1; linking and images in M2.** The module ABI is in `abi.md` (v3). This note says how guest code maps onto wasm functions and why.

## Problem

Phase 0 put a whole code region into one wasm function: locals for the guest registers, a `loop` + `br_table` dispatcher over every block, and a compiled binary search for indirect targets. It is fast inside a loop, but it does not scale:

- Cranelift took 516 ms for one function of 10k instructions / 2k blocks, and did not finish in minutes on a 201k-instruction region.
- V8 limits a function body to 7,654,321 bytes and 50,000 locals, and TurboFan's cost also grows faster than linearly with function size.
- A real program is hundreds of thousands of instructions (7zz.exe: about 2 MB of code).

So the unit of translation has to be smaller than a region, and control transfers between units have to be cheap.

## Pipeline

1. **Decode and lift** (`lift.rs`, with lowering by instruction family in `lift/` from M3), as in Phase 0: recursive traversal from the entry points; direct branch and call targets inside the region, branch fall-throughs and call return sites are block leaders; blocks are formed by decoding linearly. Each guest instruction lowers to IR ops; each block ends in one terminator.
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
- **Discovery past unsupported instructions:** an unsupported instruction ends its block with the `EXIT_UNSUPPORTED` exit, but the next address is still discovered as a block of the same function (`Block::after_exit`). Jumps, returns, `ud2`, `int3` and invalid bytes stop discovery as before. So a module's structure doesn't depend on which instructions are supported, and measurements see whole functions: 94.6 % of 7zz.exe's census instructions.
- **Jump tables:** absolute tables are already covered by relocation targets. GCC's relative tables (`lea base; movsxd idx, [base + idx*4]; add idx, base; jmp idx` behind a bounds check) are recovered by pattern, and their targets become block leaders. Every target must lie in executable code, otherwise the whole table is rejected. In 7zz.exe, 133 of 138 tables are recovered. A target that recovery misses is translated at run time.
- **Run-time translation:** a lookup miss inside an executable section returns `EXIT_LEFT_CODE` to the runtime, which translates from that address with the wasm32 build of the translator, links the module and resumes (`runtime.md`). Known addresses are passed in, so nothing is translated twice.
- **Faults (NULL check, store check):** `EXIT_FAULT` with the fault address and kind in the state block, from M3/M6.

## Instruction lowering and lazy flags (M3)

Status: **design for M3, reviewed before the lowering.** Semantics are the SDM's (Vol. 1 §3.4.3 and appendix A for the flags, Vol. 2 for each instruction). Where the SDM leaves a flag or register undefined, `abi.md` lets translated code produce any value and the fixtures don't compare it (`fixtures.md`, "Undefined results"); the choices below are the cheap ones.

### Code layout

- `lift.rs` keeps discovery and block formation. `lift/lower.rs` holds the per-instruction state (`Lower`), the op builders and the dispatch by mnemonic; `lift/operand.rs` registers, effective addresses, operand reads and writes. Each family has its module: `data` (mov, lea), `stack` (push, pop), `branch` (call, ret, jmp, jcc, jrcxz, loop), `alu` (add, sub, cmp, adc, sbb, and, or, xor, test, inc, dec, neg, not), `ext` (movzx, movsx, movsxd, cbw … cqo, bswap), `cc` (setcc, cmovcc), `shift` (shl, shr, sar, rol, ror, rcl, rcr, shld, shrd), `muldiv`, `bit` (bt, bts, btr, btc, bsf, bsr), `atomic` (xchg, xadd, cmpxchg), `string`, `misc` (nop forms, pause, prefetch, ud2, cmc, clc, stc, std, cld), `sse`, `x87`.
- `emit/flags.rs` holds the lazy-flag lowering and the `flags` function; vector ops go into `emit/simd.rs`, helper functions into `emit/helpers.rs`.
- Every family writes through `write()`, so 32-bit register writes zero-extend, 8/16-bit writes merge and AH-BH address bits 8-15, as in M1.

### IR additions

- `BinOp` gains `ShrS`, `Rotl`, `Rotr` (64-bit), `Mul`, `DivU`, `DivS`, `RemU`, `RemS` and the comparisons `Eq`, `Ne`, `LtU`, `LtS` (result 0 or 1). Shift counts are taken mod 64, as in wasm; the lifter never relies on a count of 64 (it writes `x >> (64 - c)` with c possibly 0 as `(x >> 1) >> (63 - c)` or excludes c = 0).
- `Op::Un { dst, op, a }` with `Clz`, `Ctz`, `Sext8`, `Sext16`, `Sext32`; `Op::Select { dst, cond, a, b }` (`cond != 0 ? a : b`).
- `Op::Cond { dst, cond }`: a condition (0 or 1) of the current flag record. It is a flag reader.
- `Op::LoadState { dst, width, offset }`, `Op::StoreState { width, offset, value }`: fields of the state block (DF in RFLAGS, FS/GS base, MXCSR, FCW/FSW/FTW, pieces of XMM registers).
- `Op::FaultIf { cond, kind }` and `Term::Fault { kind, rip }` (Faults, below).
- `Op::DivWide { q, r, fault, hi, lo, d, signed }`: 128/64-bit division through the `div128` helper.
- `Op::RepString { op, width, rep }`: a string instruction with its repeat loop.
- `Op::Flags` gains `when: Option<Value>` (a conditional record); `Op::FlagsUpdate { clear, set, flip, when }` writes an eager record from the previous one.
- `Term::Branch`'s condition becomes `Test::Flags(Cond)` (all branches so far) or `Test::NonZero(Value)` (jrcxz, loop).
- Vector temporaries and ops (SSE, below).

The IR dump gets a form for each; existing ops keep their text, so snapshots change only where new ops appear.

### Lazy flag record

`cc_op = 1 + 4*kind + width` as in M1, with kinds 0 (add) and 1 (sub) unchanged, so existing modules, snapshots and stored states keep their meaning; `cc_op = 0` (`CC_EAGER`) still means "materialized in `cc_a`". `a`, `b` and `r` are zero-extended to the width; SIGN is the width's sign bit, msb(x) = x & SIGN ≠ 0.

| kind | `FlagOp` | a | b | r | CF | OF |
|---|---|---|---|---|---|---|
| 0 | `Add` | dest | source | result | r < a | msb((a ^ r) & (b ^ r)) |
| 1 | `Sub` | dest | source | result | a < b | msb((a ^ b) & (a ^ r)) |
| 2 | `Logic` | result | 0 | result | 0 | 0 |
| 3 | `Adc` | dest | source | result | msb((a & b) \| ((a ^ b) & ~r)) | as `Add` |
| 4 | `Sbb` | dest | source | result | msb((~a & b) \| (~(a ^ b) & r)) | as `Sub` |
| 5 | `Inc` | dest | CF before (0/1) | result | b | r == SIGN |
| 6 | `Dec` | dest | CF before (0/1) | result | b | a == SIGN |
| 7 | `Shl` | value | count (≥ 1) | result | (a << (b - 1)) & SIGN ≠ 0 | msb(a ^ r) |
| 8 | `Shr` | value | count (≥ 1) | result | (a >> (b - 1)) & 1 | msb(a ^ r) |
| 9 | `Sar` | value | count (≥ 1) | result | (sext(a) >>s (b - 1)) & 1 | msb(a ^ r) |
| 10 | `Mul` | overflow (0/1) | 0 | low half | a | a |

For every kind: PF = even parity of `r & 0xff`, ZF = (r == 0), SF = msb(r), AF = (a ^ b ^ r) & 0x10.

- AF is exact for add, sub, adc and sbb, and for inc and dec too, because their `b` (0 or 1) doesn't reach bit 4. Logic gives 0 (a = r, b = 0); for shifts and mul AF is undefined.
- The adc/sbb carry is the carry (borrow) out of the msb of a full adder (subtractor), with the carry into the msb recovered as msb(a ^ b ^ r). So the record needs no carry-in. `Add`'s r < a would be wrong for adc when b + carry wraps (b = mask, carry 1, r = a). OF and AF are the same as without a carry.
- Shifts: OF is defined only for a count of 1, and msb(a ^ r) gives the SDM's value there for all three (shl: msb(r) xor CF with CF = msb(a); shr: msb(a); sar: 0). `Shl`/`Shr` serve shld/shrd too, with a = the destination: CF is the last bit shifted out of it, OF (count 1) the sign change. Counts above the width (8/16-bit, up to 31) leave shl/shr's CF undefined; sar's CF is then the sign, which the sign-extended formula gives.
- `Mul`: CF = OF = the overflow bit; SF, ZF and PF come from the low half (undefined per the SDM).

**The `flags` function** keeps its index (2) and signature `(op, a, b, r) -> i64`: the eager case as now; then `width = (op - 1) & 3`, `kind = (op - 1) >> 2`, a `br_table` on the kind computes `CF | OF << 11`, and PF, AF, ZF and SF are computed once for all kinds.

**Static conditions.** `known_cc` covers every kind and `CC_EAGER`. The generic path builds conditions from per-kind CF and OF and the common ZF/SF/PF; fast paths: `Sub` as now (be, l, le from the operands); `Logic`: b and o are 0, be = zf, l = sf, le = zf | sf; `Inc`/`Dec`: b = `cc_b`; `Mul`: o and b = `cc_a`; eager: bits of `cc_a`, without calling `flags`.

### Partial records, readers and liveness

Records are written three ways:
- **Full:** `Op::Flags` without `when`, kind other than `Inc`/`Dec`. All six status flags.
- **Partial:** keep some flags of the previous record, which the emitter reads, only when the new record is live. `Inc`/`Dec` copy its CF into `cc_b` (the static `cf` if its kind is known, else `flags(…) & 1`); the IR's `b` is a placeholder. `FlagsUpdate` materializes it as F (`cc_a` if eager, else `flags`) and stores the eager record `cc_a = ((F & !clear) | set) ^ flip`.
- **Conditional:** `when = Some(v)`: the record (full or partial) is written only if v ≠ 0. Used for counts in CL, where a masked count of 0 leaves every flag alone. After it, `known_cc` is unknown.

`analyze_flags`, scanning backward:
- `Op::Cond` is a reader: live = true.
- A full record: its `live` = live; then live = false.
- A partial or conditional record: its `live` = live, and live is unchanged. If it is live, the previous record is read or may survive; if it is dead, so is the previous one.
- Terminators as in M1, except that `Branch` with `Test::NonZero` reads no flags: its live-out is the live-in of its two successors (true for an unknown one).
- Dead records emit nothing, and `known_cc` forgets the kind, as in M1.

### Flags per instruction

"Previous record" says when an instruction reads the record before it.

| Instruction | Defined | Unchanged | Undefined | Previous record |
|---|---|---|---|---|
| add, sub, cmp, neg (`Sub`, a = 0), xadd (`Add`), cmpxchg (`Sub` of accumulator and dest) | all six | | | no |
| adc, sbb | all six | | | CF (`Op::Cond` b) as the carry-in, before the new record |
| and, or, xor, test | CF = OF = 0, SF, ZF, PF | | AF (0) | no |
| inc, dec | OF, SF, ZF, AF, PF | CF | | CF, only if the record is live |
| shl/sal, shr, sar; c = count & 31 (& 63 for 64-bit) | c ≠ 0: CF, SF, ZF, PF; OF if c = 1 | all if c = 0 | AF; OF if c > 1; CF of shl/shr if c ≥ width | no (conditional record for CL) |
| rol, ror | c ≠ 0 (before mod width): CF; OF if c = 1 | SF, ZF, AF, PF; all if c = 0 | OF if c > 1 | `FlagsUpdate` |
| rcl, rcr; t = c mod 9 (8-bit), mod 17 (16-bit), c (32/64) | t ≠ 0: CF; OF if c = 1 | SF, ZF, AF, PF; all if t = 0 | OF if c ≠ 1 | CF as the carry-in, then `FlagsUpdate` |
| shld, shrd | c ≠ 0: CF, SF, ZF, PF; OF if c = 1 | all if c = 0 | AF; OF if c > 1; 16-bit with c > 16: all, and the result | no |
| mul, imul | CF = OF = high half significant | | SF, ZF, AF, PF (from the low half) | no |
| bt, bts, btr, btc | CF = the selected bit | ZF | OF, SF, AF, PF (kept) | `FlagsUpdate` |
| bsf, bsr | ZF = (source == 0) | | CF, OF, SF, AF, PF (`Logic` of the source) | no |
| cmc, clc, stc | CF | the rest | | `FlagsUpdate` (flip, clear, set CF) |
| div, idiv | | | all six (record left as it is) | no |
| std, cld | DF only (state RFLAGS bit 10) | status flags | | no |
| mov forms, lea, push, pop, xchg, not, bswap, extensions, setcc, cmovcc, movs/stos/lods, loop, jrcxz, nops, SSE moves and integer ops, fninit | none | all | | setcc, cmovcc, loope/loopne read through `Op::Cond` |

### Integer results

- **adc/sbb:** carry = `Cond(B)`; r = (a + b + carry) & mask, (a − b − carry) & mask.
- **Shifts:** the masked count c. Results in 64-bit arithmetic: shl (a << c) & mask; shr a >> c; sar (sext(a) >>s c) & mask, so counts above an 8/16-bit width give 0 or the sign. An immediate count of 0 emits no record, and the destination is written back unchanged (a 32-bit register is still zero-extended: open point 1); a count in CL computes the same formulas (a shift by 0 returns the value) and makes the record conditional on c ≠ 0.
- **rol/ror:** k = c mod width; narrow widths compose shifts (`a >> (w - k)` with w ≤ 32 is 0 at k = 0), 64-bit uses `Rotl`/`Rotr`. rol: CF = lsb(r), OF = msb(r) ^ CF; ror: CF = msb(r), OF = msb(r) ^ bit w−2 of r.
- **rcl/rcr:** for 8/16/32-bit, rotate the (w + 1)-bit value `carry << w | a` by t (at most 33 + 31 bits, so it fits in 64); the new CF is bit w. For 64-bit (t in 1..63), rcl: r = (a << t) | (carry << (t − 1)) | ((a >> 1) >> (64 − t)), CF = (a >> (64 − t)) & 1; rcr symmetrically. A dynamic t = 0 selects the old value. OF: rcl msb(r) ^ new CF; rcr msb(r) ^ bit w−2 of r (equal to the SDM's msb(dest) ^ CF from before the rotation).
- **shld/shrd** (c ≠ 0): ((dst << c) | (src >> (w − c))) & mask and ((dst >> c) | (src << (w − c))) & mask; a dynamic c = 0 selects the old value.
- **mul/imul:** mul r/m8: AX = AL × src; 16/32-bit: DX:AX, EDX:EAX from one 64-bit product; 64-bit: RDX:RAX, the high half from four 32-bit partial products with carries (about 15 ops; wasm has no 64×64→128 multiply). Signed high half = unsigned high − ((a >>s 63) & b) − ((b >>s 63) & a). Overflow: mul when the high half ≠ 0; imul when the full product ≠ the sign extension of its low half. Two- and three-operand imul keep the low half; for widths up to 32 the product of the sign-extended operands is exact in 64 bits.
- **bt\*:** register destination: bit = offset mod width. Memory destination with a register offset: the offset is signed and selects a whole operand, address = ea + (sext(offset) >>s log2(w)) × (w / 8), bit = offset & (w − 1). Immediate offset: imm8 & (w − 1), no address change. bts/btr/btc write the operand back.
- **bsf/bsr:** source ≠ 0: ctz(source), 63 − clz(source) (source zero-extended). Source 0: the destination register keeps all 64 bits (a `Select` on the full register). The SDM leaves it undefined, and fixtures mask it.
- **cmpxchg r/m, r:** t = dest; record `Sub`(accumulator, t). Equal: dest ← src; else accumulator ← t, dest ← t. As in the SDM's pseudocode, dest is written in both cases (a 32-bit register destination zero-extends either way) and the accumulator only on a mismatch (RAX keeps its upper half on success).
- **xadd:** t = dest + src; src ← dest; dest ← t, in that order (the same register for both ends up with t). `Add` record.
- **xchg:** both writes zero-extend for 32-bit operands; `87 C0` (xchg eax, eax) zero-extends RAX, `90` is a nop.
- **lock:** accepted with a memory destination on add, adc, and, btc, btr, bts, cmpxchg, cmpxchg8b/16b, dec, inc, neg, not, or, sbb, sub, xadd, xchg and xor, and lowered as the plain read-modify-write (`abi.md`). Anything else is #UD; iced reports it as an invalid encoding (Faults, below).
- **Strings** (movs, stos, lods; cmps and scas the same way, with a `Sub` record per iteration and repe/repne termination, when a target needs them): `Op::RepString`. The emitter reads DF from the state's RFLAGS once (nothing inside the instruction changes it): delta = DF ? −size : +size. Without rep, one iteration. With rep: while RCX ≠ 0 { one iteration; RCX −= 1 }, so RCX = 0 changes nothing. With DF = 0, two fast paths: rep stosb is `memory.fill(RDI, AL, RCX)`, and rep movs with (RDI − RSI) mod 2^64 ≥ RCX × size (no overlap ahead of the source, where a forward copy equals memmove) is `memory.copy`; everything else is the element loop. Afterwards RCX = 0 and RSI/RDI have moved by RCX × delta. Out-of-bounds accesses trap in both paths (no difference until M6 adds memory checks, which must then check the whole range first). The 67h forms (ECX, ESI, EDI) stay unsupported.
- **jrcxz:** `Test::NonZero(RCX == 0)`. loop: RCX ← RCX − 1 (no flags), `Test::NonZero(RCX ≠ 0)`; loope/loopne also AND it with ZF or not-ZF from `Op::Cond`. The 67h forms (jecxz, ECX) stay unsupported.
- **Extensions:** movzx, movsx, movsxd, cbw/cwde/cdqe, cwd/cdq/cqo through `Sext`/masks (cdq writes EDX and zero-extends RDX). bswap r32/r64 by three mask-and-shift rounds (bswap r16 is undefined).
- **setcc** writes `Cond` to r/m8. **cmovcc** always reads the source (a memory source is read even when the condition is false), then writes `Select(cond, src, dst)`, so a 32-bit destination is zero-extended even when the condition is false.
- **No-ops:** `90`, `0F 1F /0` with any operand (no memory access), pause, prefetch\* (no access, never faults).

### Addressing

- **FS/GS:** ea = offset + `LoadState(STATE_FS_BASE or STATE_GS_BASE)`, a 64-bit add. lea uses the offset only, as now; other segments add nothing.
- **67h:** the offset is computed from the full registers and masked to 32 bits (the low 32 bits of the sum depend only on the low 32 bits of each term), then the FS/GS base is added. EIP-relative: (next RIP + disp32) & 0xFFFF_FFFF. lea with 67h gives that 32-bit offset, truncated to the destination width.

### Faults

- **`Term::Fault { kind, rip }`** works like `Exit` with `state.exit = EXIT_FAULT`, the fault kind, fault address 0 and RIP at the instruction. `ud2` lowers to it with `FAULT_INVALID_OPCODE`, and so do invalid encodings (decoder errors other than a truncated instruction), which were `EXIT_UNSUPPORTED` until now: the CPU raises #UD there (open point 3). Discovery still stops at both.
- **`Op::FaultIf { cond, kind }`:** if cond ≠ 0 the instruction faults. The state must be the one from before the instruction, so the lifter puts every `FaultIf` before the instruction's first side effect (`SetReg`, `Store`, `StoreState`, vector stores, any flag record); only loads and pure computation come before it. Emitted as `if cond { rip = insn; fault kind and address; exit; status = EXIT; br $leave }`, and the shared epilogue stores the untouched registers and flag record. A test checks the order for every faulting form.
- **div/idiv (#DE):** wasm's `div` traps on a zero divisor and `div_s` on INT64_MIN / −1, so `DivU`/`DivS`/`RemU`/`RemS` only appear after the `FaultIf` that excludes those operands.
  - div r/m8: n = AX; fault if d = 0 or AH ≥ d (the quotient doesn't fit in 8 bits); AL = q, AH = r. div 16/32: n = DX:AX or EDX:EAX in 64 bits; fault if d = 0 or the high half ≥ d; q and r to AX/DX or EAX/EDX (32-bit writes zero-extend RAX and RDX).
  - idiv 8/16/32: n sign-extended to 64 bits (32-bit: (EDX << 32 | EAX) as i64), d sign-extended; fault if d = 0; for 32-bit also if n = INT64_MIN and d = −1 (its quotient doesn't fit anyway); q = n /s d; fault if q ≠ its own sign extension from the width, which covers INT_MIN / −1 at every width (−2^31 / −1 = 2^31); r = n rem_s d, with the dividend's sign.
  - div/idiv 64: `DivWide` calls the helper `div128(hi, lo, d, signed) -> (q, r, fault)`. Unsigned: fault if d = 0 or hi ≥ d; hi = 0 is a single `i64.div_u`/`rem_u`; otherwise normalized long division in two 32-bit digit steps (Hacker's Delight `divlu`, with its correction loops unrolled to their bound of two). Signed: 128-bit magnitudes, the unsigned division (fault if the magnitude's high half ≥ |d|), then a fault if |q| > 2^63 − 1 with equal signs or > 2^63 with different signs; q is negated if the signs differ, and r takes the dividend's sign. So RDX:RAX = −2^63 (RDX = −1, RAX = 2^63) divided by −1 faults.
  - Helpers come after the chunks (function 3 + number of chunks), are emitted only when used, are named in the name section and are not in the table (open point 4).
  - The flags after div/idiv are undefined; the record stays as it was.
- **#GP for misaligned SSE operands:** SSE, below.

### SSE and SSE2 without floating-point arithmetic

In scope for M3: every SSE/SSE2 form whose result doesn't depend on floating-point arithmetic: moves, bitwise ops, packed integer add/sub (wrapping and saturating), compares, shifts, multiplies (pmullw, pmulhw, pmulhuw, pmuludq, pmaddwd), averages, min/max, psadbw, shuffles, unpacks, packs, pextrw/pinsrw, pmovmskb, movmskps/movmskpd, ldmxcsr/stmxcsr. FP arithmetic, compares and conversions (addsd, comisd, cvt\*, …) are M4 and stay `EXIT_UNSUPPORTED` until then.

**Registers in the state block.** XMMi lives at state + 0x100 + 16 × i while code runs (`abi.md`), so nothing is synced at boundaries. Lane i of every wasm SIMD shape is element i of the x86 register (both little-endian).

**IR.** `VTemp`: instruction-local v128 temporaries (v128 locals of the chunk, named in the name section). `VLoc` = `State(offset)` | `Mem(addr)`.
- `VLoad { dst, width, loc }`, width 32, 64 or 128; the narrow loads zero the rest (`v128.load32_zero`, `v128.load64_zero`).
- `VStore { loc, width, src }`; the narrow stores write the low lane only (`v128.store32_lane`, `v128.store64_lane`), so merging into a register is a narrow store into its slot.
- `VBin { dst, op, a, b }`, `VShuffle { dst, a, b, lanes: [u8; 16] }`, `VShift { dst, op, a, count }`, `VConst`, `VFromGpr` (an i64 into lane 0, the rest 0), `VExtract { dst, op, src }` (64-bit lane, 16-bit lane zero-extended, or `bitmask`).
- GPR-sized pieces of a register (movd, movq, pextrw, movhps …) use `LoadState`/`StoreState` on its slot directly.

**Alignment.** A legacy-SSE m128 operand must be 16-byte aligned (#GP(0) otherwise), except for movdqu, movups and movupd. So movdqa, movaps, movapd, movntdq, movntps and every packed op with an m128 source (pand xmm, m128; pshufd; psllw xmm, m128; …) emit `FaultIf(addr & 15, FAULT_GENERAL_PROTECTION)` before any write. m32/m64 operands (movd, movq, movss, movsd, movhps/movhpd, movlps/movlpd) have no alignment requirement.

**Moves.**
- movdqa, movdqu, movaps, movups, movapd, movupd: 128-bit load and store.
- movd xmm, r/m32 and movq xmm, r/m64 zero-extend to 128 bits. movd r/m32, xmm and movq r/m64, xmm take the low lane (a 32-bit GPR write zero-extends).
- movq xmm, xmm/m64 (`F3 0F 7E`) and movq xmm, xmm (`66 0F D6`): the low 64 bits, upper half zeroed. movq m64, xmm: a 64-bit store.
- movss, movsd: from memory, zero the rest; register to register, merge the low 32 or 64 bits (a narrow store); to memory, a 32- or 64-bit store.
- movhps/movhpd and movlps/movlpd with m64: load into or store from the high or low 64 bits; the other half is kept. movhlps: dst.lo = src.hi; movlhps: dst.hi = src.lo.
- movntdq, movnti, movntps: plain stores (with the alignment check for the 128-bit ones).

**Bitwise.** pand/andps/andpd → `v128.and`; por/orps/orpd → `or`; pxor/xorps/xorpd → `xor`; pandn/andnps/andnpd: dst = ~dst & src → `v128.andnot(src, dst)`. `pxor x, x` needs no special case.

**Integer arithmetic.**
- paddb/w/d/q, psubb/w/d/q → `i8x16`/`i16x8`/`i32x4`/`i64x2` `add`/`sub` (wrapping).
- paddsb/paddsw, paddusb/paddusw, psubsb/psubsw, psubusb/psubusw → `add_sat_s/u`, `sub_sat_s/u`: the same saturation as x86.
- pavgb/pavgw → `avgr_u`: (a + b + 1) >> 1, as x86. pminub/pmaxub, pminsw/pmaxsw → `min_u`/`max_u`, `min_s`/`max_s`.
- pmullw → `i16x8.mul`. pmulhw/pmulhuw → `i32x4.extmul_low/high_i16x8_s/u`, then one `i8x16.shuffle` that keeps bytes 2-3 of every 32-bit lane. pmuludq → a shuffle that moves 32-bit lanes 0 and 2 to 0 and 1, then `i64x2.extmul_low_i32x4_u`. pmaddwd → `i32x4.dot_i16x8_s` (both wrap to 0x8000_0000 when all four inputs are −32768).
- psadbw: byte differences as `max_u − min_u`, `i16x8.extadd_pairwise_i8x16_u`, `i32x4.extadd_pairwise_i16x8_u`, then the two 32-bit sums of each half added into its 64-bit lane (the upper 48 bits are 0).

**Compares.** pcmpeqb/w/d → `eq`; pcmpgtb/w/d → `gt_s`.

**Shifts.** psllw/d/q, psrlw/d/q and psraw/d by imm8, xmm or m128. The count is unsigned: the imm8 as it is, or for the xmm/m128 forms the whole low 64 bits of the source (a count of 2^32 counts as ≥ 64). wasm takes shift counts mod the lane width, so: logical shifts by ≥ the lane width give 0, and arithmetic shifts use min(count, width − 1). Immediate counts are resolved at translation time (a zero vector or the clamped count). A register count is compared in 64 bits before it is narrowed to i32: `select(shl(a, c), 0, count < w)` and `shr_s(a, min_u(count, w − 1))`. pslldq/psrldq imm8: a byte shuffle with a zero vector; counts above 15 give 0.

**Shuffles, unpacks and packs.**
- pshufd, pshuflw, pshufhw, shufps, shufpd, punpckl/h{bw, wd, dq, qdq}, unpcklps/pd, unpckhps/pd: one `i8x16.shuffle`, with lanes computed from imm8 at translation time (two-source forms index dst as 0-15 and src as 16-31).
- packsswb → `i8x16.narrow_i16x8_s`, packuswb → `i8x16.narrow_i16x8_u`, packssdw → `i16x8.narrow_i32x4_s`. All read signed inputs and saturate as x86 does.

**Extracts and inserts.** pextrw r32, xmm, imm8: a 16-bit load at slot + 2 × (imm & 7), zero-extended. pinsrw xmm, r32/m16, imm8: a 16-bit store. pmovmskb → `i8x16.bitmask`; movmskps/movmskpd → `i32x4.bitmask`/`i64x2.bitmask`, which read sign bits only and are exact for any data.

**MXCSR.** ldmxcsr m32: #GP if a reserved bit (31-16; MXCSR_MASK = 0xFFFF) is set, else a store to 0x200; stmxcsr: a load from it. Nothing else reads MXCSR in M3.

### x87: fninit

fninit (`DB E3`): FCW ← 0x037F, FSW ← 0 (TOP 0, condition codes and exception flags clear), FTW ← 0xFFFF (every register empty, in the state block's full 2-bit format), through `StoreState`. The register contents, MXCSR and the XMM registers are unchanged; the FPU instruction and data pointers are not modeled. finit (`9B DB E3`) is wait + fninit. M4 adds the other x87 forms ("Floating point and x87 (M4)").

### Tests, before each family's code

- The L1 fixtures (agent A) decide when a family is done: `testkit replay` at 100 %, every earlier family still at 100 %.
- In `crates/x64wasm/tests`, with wasmtime as a dev-dependency (already a workspace dependency):
  - the `flags` function against a Rust model, for every kind and width over edge operands (0, 1, SIGN − 1, SIGN, mask, a seeded random set), and every condition through the static and the dynamic path;
  - `div128` against `u128`/`i128`: d = 0, hi ≥ d, INT64_MIN / −1, quotients at 2^63 − 1, 2^63 and 2^64 − 1, and random cases.
- Flag liveness with readers and partial and conditional records, in the style of `dead_flags_are_marked`.
- The order of `FaultIf` before the first side effect, for every faulting form.
- One IR/WAT snapshot per family for a representative sequence (an adc chain, a shift by CL, rep movsb, div, pshufd), added with the family.

### Open points

1. **Count 0 on a 32-bit register** (shifts, rotates, shld/shrd): the destination is written back and so zero-extended. The fixtures with count 0 decide; if the CPU leaves the upper half, the lowering skips the write.
2. **bsf/bsr with a zero source** keep the whole destination register; the fixtures mask it (`undefined`), so either choice passes.
3. **Invalid encodings** become #UD faults (`EXIT_FAULT`) instead of `EXIT_UNSUPPORTED`. That lowers the unsupported-site count in the translation statistics.
4. **`abi.md`, "Inside the module":** needs one line saying helper functions (`div128`) follow the chunks, outside the table.
5. The 67h forms of string ops, loop and jecxz stay unsupported (not in the census).
6. `int1` (`F1`, gap-only in the census) is not lowered (it would raise #DB); it is part of the M5 decision on gap-only forms.

## Floating point and x87 (M4)

Scope: `instruction-scope.md`, "M4 additions to the scope". The standard is M3's: every result bit, every MXCSR and FSW flag, C1 and the tags as the CPU produces them, under every rounding mode, FTZ/DAZ and x87 precision control. The fixtures compare all of them. The exact arithmetic lives in the soft-float helper module (`abi.md`, "Floating-point helper module"); translated code calls it whenever the plain wasm instruction can't be proven exact.

### Code layout

- `lift/sse.rs` gains the SSE/SSE2 floating-point forms; a new `emit/fp.rs` emits their fast and slow paths.
- `lift/x87.rs` gains the x87 forms; a new `emit/x87.rs` emits them.
- `crates/x64wasm-fp` holds the soft float. The emitter imports the helpers a module uses (`fp.*`, v4), and `crates/testkit`'s wasmtime host defines them natively.

### IR

No new value types. FP values travel in `VTemp`s: scalar forms use lane 0 and keep the destination's other lanes, packed forms use every lane, and GPR operands are ordinary temps. New ops, one per class of x86 operation (names up to the implementation): binary arithmetic (add, sub, mul, div, min, max) and sqrt per format (ss, sd, ps, pd), compares with a predicate, comis/ucomis, and conversions by kind. x87 instructions lower to a few macro ops expanded in `emit/x87.rs`. Like `FaultIf`, an op that can exit (an unmasked exception) comes before the instruction's first side effect.

### SSE: fast path and helper

Each floating-point operation either takes the inline fast path, the wasm instruction itself, or calls its helper. The fast path is taken only when the wasm result is bit-exact under the current MXCSR and the operation can't change MXCSR. wasm arithmetic is IEEE round-to-nearest-even with gradual underflow, which is what x86 computes when no special case is involved; NaN results, rounding modes, FTZ/DAZ and the flags are where they differ.

- **add, sub, mul, div:** `(mxcsr & 0x7FA0) == 0x1FA0` (all exceptions masked, RC = nearest, PE already set), no operand denormal, and MIN_NORMAL < |r| < ∞ for the wasm result r. That excludes NaN, infinite, overflowing, zero and tiny results (a result rounded up to MIN_NORMAL can still be tiny). DAZ and FTZ don't matter then, and the only flag the operation could raise, PE, is already set.
- **sqrt:** the same MXCSR test and a positive normal operand.
- **min, max:** no operand NaN or denormal. Then `select(a, b, a < b)` for min and `select(a, b, a > b)` for max: x86 returns the second operand when the two are equal (±0 included). No flag can change, so MXCSR isn't read.
- **cmp\*, comis, ucomis:** no operand NaN or denormal; MXCSR isn't read.
- **Conversions:**
  - int → float: no test when the value is exactly representable (|x| ≤ 2^24 for f32, ≤ 2^53 for f64); otherwise the arithmetic MXCSR test. cvtsi2sd r/m32 and cvtdq2pd are always exact.
  - float → int, truncating: the value in range and either already integral or PE set with PM masked.
  - float → int, rounding: also RC = nearest (`f64.nearest`, which rounds ties to even like x86).
  - cvtss2sd and cvtps2pd: no NaN or denormal operand (the widening is then exact).
  - cvtsd2ss and cvtpd2ps: the arithmetic test on the narrowed result.
- **Packed forms** test every lane with SIMD compares and `all_true`. If any lane fails, the whole instruction takes the slow path.
- **As implemented (M4):** scalar forms share the packed code: the fast path computes on the whole vector (`f64x2.add`, `f32x4.demote_f64x2_zero`, …) and the test reads the lanes the form uses from the lane mask's sign bits (`bitmask & 1` for lane 0, `& 3` for cvtps2pd and cvtpd2ps); the result's lane 0 is merged into the destination with one `i8x16.shuffle`. "No operand denormal" is tested as `|x| ≥ MIN_NORMAL or x = 0`, which also rejects NaN operands (their results fail the result test anyway). cvtsd2ss and cvtpd2ps test only the narrowed result: a denormal f64 operand narrows to 0 and fails it. cmpss … cmppd with imm8 > 7 (reserved bits set in the legacy encoding) stay `EXIT_UNSUPPORTED`. Chunks with floating-point code get two scratch locals, `mxcsr` (i32) and `fpv` (v128), after the vector temporaries. `crates/testkit/tests/sse_fp_paths.rs` checks that the fast path gives what the helper gives (each case at MXCSR 0x1FA0 and 0x1F80) and that unmasked exceptions exit before any change.

MXCSR is loaded from the state block for each instruction that tests it. (M6 can hoist the load: inside a block the fast-path test can only go from false to true.)

**Slow path:** call the helper (once per lane for packed forms, lane 0 first, passing each lane the MXCSR the previous one returned), then `fp.status()`. If bit 31 is set, exit with `EXIT_UNSUPPORTED`, RIP at the instruction and nothing written. Otherwise store the new MXCSR, then the result.

**comis, ucomis:** write an eager flag record (`cc_op = CC_EAGER`, `cc_a` = ZF, PF and CF; OF, SF and AF 0). On the fast path: unordered gives ZF, PF and CF; less gives CF; equal gives ZF. The record is a `FlagsUpdate` that clears every status flag, which flag liveness treats as a full record (it doesn't read the previous one) and the emitter writes without materializing the previous one.

**Alignment:** as in M3, a legacy-SSE m128 operand must be 16-byte aligned, so the packed forms with a memory source emit `FaultIf(addr & 15, FAULT_GENERAL_PROTECTION)` first. Scalar m32/m64 operands have no requirement.

### x87

**State** (`abi.md`): FCW at 0x204, FSW at 0x206 (TOP in bits 11-13), the full tag word at 0x208, R0-R7 at 0x210 + 16 × p (significand at +0, sign and exponent at +8). ST(i) = R[(TOP + i) & 7], computed at run time from FSW.

**Tags** are written with every register write, from the contents, exactly as FSTENV reports them: exponent 0x7FFF → special; exponent 0 → zero if the significand is 0, else special; J bit clear → special (unnormal); else valid. A pop marks the register empty.

**Stack faults:**
- **Push** (fld, fild, fldz, fld1 …): TOP − 1. If the new ST(0) isn't empty, it is an overflow: IE and SF are set and C1 = 1, and with IE masked the register gets the QNaN indefinite (sign 1, exponent 0x7FFF, significand `0xC000_0000_0000_0000`).
- **An empty operand** is an underflow: IE and SF are set and C1 = 0. With IE masked, a register result gets the indefinite, a store to memory writes the destination format's indefinite (the integer indefinite for fist), and a compare reports unordered.
- **Pop:** the tag becomes empty and TOP + 1.

**Operations:**
- Arithmetic, loads from m32/m64 and integers, stores to m32/m64 and integers, compares, frndint, fscale and fprem call their helper. The status's exception flags are ORed into FSW bits 0-5. C1 and, for compares and fprem, C0, C2 and C3 are written as the instruction defines them. Condition codes the SDM leaves undefined keep their value (the fixtures mask them).
- Operations that can't raise are inline: fld m80 and fstp m80 copy all 80 bits; fld st(i), fstp st(i), fxch; fchs and fabs change only the sign; the constants (fldz, fld1, and any others the census adds, rounded by RC); fxam (C3, C2, C0 from the class, C1 = sign, empty included).
- fcomi, fcomip, fucomi and fucomip write an eager flag record from the compare: C3 → ZF, C2 → PF, C0 → CF; OF, SF and AF 0.
- **An unmasked exception** (status bit 7) exits with `EXIT_UNSUPPORTED` before any change, as for SSE. ES and B in FSW follow the CPU (the fixtures decide, for example after fldcw unmasks a flag that is already set). A waiting x87 instruction that finds ES set exits with `EXIT_UNSUPPORTED` too (#MF).
- **Control forms:**
  - fnstcw m16 and fldcw m16. fldcw stores what the CPU keeps (for example, bit 6 reads as 1); the fixtures decide.
  - fnstsw m16 and fnstsw AX.
  - fnclex clears the exception flags, SF, ES and B.
  - fnstenv m28 stores the 32-bit protected-mode environment (FCW, FSW and the full tag word in the low halves of the first three dwords), with the instruction and data pointers, the opcode and the selectors as 0. Then it masks all exceptions.
  - fldenv m28 loads FCW, FSW and the tag word. Non-empty registers get their tags recomputed from their contents; empty ones stay empty.
  - fwait is a no-op unless ES is set.
- **As implemented (M4):**
  - Each stack form lowers to one macro op, `Op::X87` (`ir::X87`: push from memory, ST(i) or a constant; store to memory or ST(i), with or without a pop; fxch; arithmetic `ST(d) = ST(d) op src` with an optional pop; sqrt, frndint, fchs, fabs; fscale; fcom with 0-2 pops; fcomi; fldenv), expanded in `emit/x87.rs` with 14 scratch locals (`x87_*` in the name section). Every expansion runs in the same order: FSW and the ES check (#MF, all of them are waiting forms); TOP, the tag word and FCW; the operands and the stack-fault test; the helper or the inline move and the status check; then the writes (memory, registers with their tags, pops, FSW, the tag word). The non-waiting control forms (fnstcw, fnstsw m16/AX, fnclex, fnstenv, fninit) are plain state and memory ops; fwait is the ES check alone; finit, fstcw, fstsw, fclex and fstenv are that check plus the non-waiting form, and fldcw and fldenv start with it.
  - The same expansions lower the forms next to the scope list, which share every code path with a form that has fixtures: fsub, fsubr, fiadd … fidivr and the remaining register forms of the six operations, fst and fist without a pop, fcom, fucom, ficom, fcompp, fucompp, fcomip, fld1.
  - **The CPU, where the design said less** (fixtures `x87_*`): fcomi, fucomi and fucomip leave C1 unchanged unless the stack faults (then C1 = 0), although the SDM lists C1 as cleared; C0, C2 and C3 are unchanged. fxch with an empty register: each empty one reads as the indefinite (IE, SF, C1 = 0), then the exchange. fchs and fabs of an empty ST(0) give the indefinite itself, with no sign change. A stack fault with the arithmetic `p` forms writes the indefinite to the destination and still pops. fnstenv stores FSW as it was (ES and B included), then clears them in the register. fldcw and fldenv keep the loaded FCW as it is (the reserved bits are masked); fldcw sets ES and B exactly when a set flag is unmasked; fldenv loads FSW unchanged. fld st(i) with both a full ST(7) and an empty source counts as an overflow (C1 = 1).
  - Size: about 400-600 wasm bytes per x87 instruction (the register write with its tag, the FSW update and the fault branch are inline). Moving the shared sequences into module helpers, like `div128`, is an M6 option.

### cpuid

`cpuid` lowers to a lookup of EAX in the table of `instruction-scope.md` ("`cpuid` results"), done at run time, since EAX is usually not a constant. It writes EAX, EBX, ECX and EDX as 32-bit registers and leaves the flags alone. The dedicated test checks every row.

### Tests

- **`crates/x64wasm-fp`:** native differential tests against the CPU (inline `asm!`) for every helper. They cover edge pairs (±0, the smallest and largest denormals and normals, ±1, ±∞, QNaN and SNaN with several payloads, rounding ties, and the limits of the integer conversions) and seeded random cases, under every RC, FTZ/DAZ combination and x87 PC. The default count fits `cargo test`; an `#[ignore]` test runs millions. A Node test checks the wasm build against vectors from the native build.
- **L1 fixtures** for every in-scope form decide when a family is done: replay at 100 %, every earlier family still at 100 %.
- **Snapshots:** an SSE scalar sequence (fast and slow path), a packed form, a conversion, comisd with a branch, and an x87 sequence.
- **`cpuid`:** the dedicated test above.
- **The order of exits before side effects**, for the forms that can exit (unmasked exceptions, #GP for alignment).

## Measurements (M1)

Recorded in `docs/status/phase-1.md`:
- Phase 0's 201k-instruction generated region, translated and compiled in wasmtime (Cranelift) and in V8 (Node 24: `WebAssembly.compile`, then first-call and TurboFan time).
- Translation speed (guest instructions/s) and wasm bytes per guest instruction.
- `sum_loop` n=10^8 vs native: must not regress from Phase 0's 1.02-1.14×.
- Call-heavy micro-benchmarks, translated vs native, in wasmtime and Node:
  - recursive fib(30);
  - a small function called once per byte of a 1 MiB buffer, CoreMark's `crcu8` pattern.
- The deepest guest recursion V8 survives with the guard disabled, which sets `CALL_DEPTH_LIMIT` with margin.
