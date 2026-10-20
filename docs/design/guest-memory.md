# Guest memory layout for 64-bit programs

Phase 0, item 7. Status: **decided, option A (identity mapping below 16 GiB)**. The decision, the conditions for revisiting it, and the Phase 1/2 work list are at the end.

## The problem

- x64 PE images default to bases above 4 GiB. Windows and Wine assume a 47-bit (128 TiB) user address space.
- A wasm memory64 is a single contiguous range `[0, size)`. It has no holes, no aliasing, no per-page protection, and it can only grow.
- V8 rejects a maximum above 262144 pages (16 GiB): "value 262145 is above the upper bound 262144", measured in Node 22 and Chromium 141.
- Shared memories (which we need for threads) must declare that maximum when they are created.

## Facts this note relies on

**Image bases.** I measured MinGW locally with x86_64-w64-mingw32-gcc 13 and binutils 2.41.90, using `objdump -p`:

| Producer | EXE base | DLL base | Flags |
|---|---|---|---|
| MSVC link.exe defaults | 0x1_4000_0000 | 0x1_8000_0000 | DYNAMICBASE, HIGHENTROPYVA, NXCOMPAT by default. `/FIXED` strips relocations; `/LARGEADDRESSAWARE:NO` is allowed |
| MinGW-w64 (binutils 2.41.90) | 0x1_4000_0000 | auto-image-base from a hash of the DLL name, measured 0x2_41b9_0000 to 0x3_b3c7_0000 (inside 8-16 GiB) | DYNAMIC_BASE, HIGH_ENTROPY_VA, NX_COMPAT, LARGE_ADDRESS_AWARE; `.reloc` is kept even with `--disable-dynamicbase`; ld rejects `--disable-large-address-aware` for x86-64 |
| Old MinGW (binutils < 2.36) | 0x40_0000 | low | no DYNAMIC_BASE; EXEs usually without relocations |
| Wine 11 PE DLLs (`dlls/*/Makefile.in`) | | ntdll 0x1_7000_0000, kernelbase 0x1_7400_0000, kernel32 0x1_7800_0000; the rest are relocatable | |

**Wine's own layout.** From wine-11.0:

- `KUSER_SHARED_DATA` is at 0x7ffe_0000 (`dlls/ntdll/unix/virtual.c`, `user_shared_data`). Every PE syscall thunk reads `0x7ffe0308` and calls through `0x7ffe1000` (`include/wine/asm.h` `__ASM_SYSCALL_FUNC`). Both addresses are absolute and fixed.
- The PEB and the first TEBs come from one block allocated `MEM_TOP_DOWN` below 2 GiB (`virtual.c` ~L4054, `limit_2g - 1`).
- The preloader reserves, for x86-64 (`loader/preloader.c` L107-122):
  - the DOS area 0x1_0000-0x11_0000;
  - the low area 0x11_0000-0x6800_0000, released once the EXE is mapped;
  - 0x7f00_0000-0x7fff_0000;
  - 0x7fff_fe00_0000-0x7fff_ffff_0000, for top-down allocations and Wine's own *virtual heap* (`alloc_virtual_heap`, ~L3600).
- `user_space_limit = 0x7fff_ffff_0000` (`virtual.c` L191-193).
- wineserver hands out ASLR addresses for relocated 64-bit DLLs from 0x6000_0000_0000 + 16 TiB, top-down (`server/mapping.c` `init_memory`, `assign_map_address`). The same DLL gets the same address in every process.
- Relocated EXEs keep their preferred base if it is free. DLLs flagged DYNAMIC_BASE go top-down (`virtual.c` `map_image_view`).

**Wine's Unix side shares the address space with PE code.** Syscalls, unixlib calls (`__wine_unix_call(handle, code, args)`) and `KeUserModeCallback` pass raw guest pointers. The C code then dereferences them, including pointers nested in structures (`UNICODE_STRING.Buffer`, `IO_STATUS_BLOCK`, `OBJECT_ATTRIBUTES` ...). The Emscripten-compiled Unix side therefore has to see guest memory at the guest's addresses, or every dereference in Wine's Unix code has to be translated. See `native/README.md`.

**What Emscripten gives instead of mmap.** Measured by `native/spike/posix.c`:

- `MAP_FIXED` and address hints fail with EINVAL.
- Reserving 1 GiB `PROT_NONE` takes 0.6-2.2 s, because the range is committed and zeroed.
- 64 GiB fails.
- `mprotect` succeeds but enforces nothing.
- Address 0 is readable.
- Two `MAP_SHARED` views of a file don't alias.

So the guest-memory manager has to be ours, whichever option we pick.

## Options

**A. Identity mapping.** Guest VA == wasm offset. Everything the process uses lives below 16 GiB:

- the guest's images, heaps and stacks;
- Wine's own structures;
- the Emscripten Unix side and the translator's runtime data.

Relocatable images are rebased when their preferred base is unusable. A guest load is a plain `i64.load` on the computed x86 address.

**B. Address translation table.** Guest VAs keep their full 47-bit range. A table maps each guest page (here 64 KiB) to a physical page in the 16 GiB memory, and every guest access goes through it. B can be flat (one table, limited VA range) or two-level (full range). It also allows aliasing (two VAs on one physical page) and protection (a page's entry can point out of bounds so the access traps).

**Hybrid.** B behind a cache: a one-entry cache per access site, kept in wasm locals, or a small software TLB in memory.

## Measurements

`tests/bench/memory/` (see its README) hand-assembles one module per variant and runs four loops over 256 MiB:

- **seq**: sequential 8-byte loads.
- **rand**: independent LCG-random loads.
- **chase**: dependent pointer chasing over a random cycle.
- **store**: sequential stores.

Table variants put the buffer at guest VA 0x1_4000_0000 and map it with 64 KiB pages. All variants return identical checksums. Results are ns per access, the median of 5 runs × 5 repetitions, on a shared 4-vCPU x86-64 VM; other jobs were running, so expect about ±10% noise. Raw data is in `tests/bench/memory/results/`.

@@BENCH@@

How to read it:

@@READING@@

## Comparison

| Topic | A: identity, below 16 GiB | B: translation table |
|---|---|---|
| **Per-access cost** | @@COST_A@@ | @@COST_B@@ |
| **Wine's Unix side** | Dereferences guest pointers natively. No change to Wine's C code. | Every guest pointer the Unix side touches, nested ones included, would need translating. That means a Binaryen/LLVM pass that rewrites every load and store in the Emscripten output, at B's cost, *and* the Unix side's own data would have to be mapped by the same table. This is the biggest practical argument against B. |
| **Images with fixed bases (no relocations)** | Load at their base if it lies inside the guest regions below. Fixed bases at or above 16 GiB, or inside the host region, cannot run. The common fixed bases fit: 0x40_0000 (old MinGW) and 0x1_4000_0000 (MSVC `/FIXED`). | Any base works. |
| **ASLR / DYNAMICBASE / HIGHENTROPYVA** | We don't randomize. Relocatable images get their preferred base if it is free and inside the guest range, otherwise a deterministic slot. The same image always lands at the same address, which keeps AOT-translated code valid (cache key: image hash + load base). HIGHENTROPYVA *permits* addresses above 4 GiB but doesn't require them. | Same policy possible. Preferred bases are always usable. |
| **Non-LARGEADDRESSAWARE images** | The process's allocations stay below 2 GiB: `user_space_limit` = 0x7fff_0000, as on Windows. The low guest region is 2 GiB. | Same. |
| **Wine's layout** (KUSER_SHARED_DATA 0x7ffe_0000, PEB/TEB below 2 GiB, top-down, virtual heap, server ASLR range) | KUSER, PEB and TEB already fit. We must move `user_space_limit`/`address_space_limit`, the 0x7fff_fe00_0000 virtual-heap/top-down reserve and the server's 0x6000_0000_0000 ASLR range below 16 GiB (layout below). | Wine's layout is unchanged. The table maps whatever Wine picks. |
| **VirtualAlloc reserve/commit** | Reserve is bookkeeping in Wine's view tree. Commit marks pages and grows the memory if needed; fresh pages are already zero. Decommit then recommit must zero explicitly (`memory.fill`), and RSS is never returned to the OS (wasm memory cannot shrink or discard). | Reserve is free. Commit allocates physical pages from a free list. Decommit can recycle physical pages, but they still can't be returned to the OS. |
| **MEM_RESERVE of huge ranges** | Limited to the free guest VA, about 13 GiB. CoreCLR's regions GC (.NET 7+) reserves a 256 GiB range by default (`GCRegionRange`) and would need that setting or a fallback. Phase 4 uses Wine Mono instead. | Reservations of any size work. Only committed pages count against 16 GiB. |
| **Guard pages** | Not enforced by the memory. Thread stacks: commit the whole reservation (free until touched), and detect overflow from RSP against the TEB stack limit in `__chkstk` and large `sub rsp`. App-defined PAGE_GUARD is not supported. | An entry can point out of bounds, so the access traps, if wasm traps can be made precise (below). |
| **Protection** (NOACCESS, READONLY, EXECUTE, write-watch) | No per-access checks by default. NULL page: explicit check (`m64-nullcheck`). Execute: checked when code is translated (DEP), which is free. Write-watch and code pages: store-side flag check (`m64-wcheck`). Writes to read-only data succeed silently, and that is an accepted incompatibility. A debug "strict" mode checks every access against the page-state table at about B's cost. | Separate read and write tables can send NOACCESS/READONLY pages out of bounds. Protection is then free on the fast path, but faults surface as wasm traps that unwind the frame. That needs re-execution from a saved block-entry state to give precise exceptions. |
| **MapViewOfFile aliasing** (one section twice in one process) | Not possible. Read-only views get copies. A second writable view of the same section in one process is copied and synced at flush/unmap, or refused. | Natural: two table entries, one physical page. |
| **Cross-process shared memory** | Each process has its own memory. KUSER_SHARED_DATA and Wine's session shared memory are written by the server into every client's memory; the server holds every process's SharedArrayBuffer. App-level shared sections are a Phase 4 problem: copy-and-sync, or a dedicated shared window behind a range check. | Only if all processes share one physical memory, which puts the sum of all processes under 16 GiB. |
| **Pointer tagging, 47/48-bit assumptions** | All guest pointers are below 2^34, so code that tags the top 16 bits or assumes canonical 47-bit pointers works (NaN-boxing, LuaJIT, JS engines in the guest). Apps that MEM_RESERVE at hard-coded high addresses fail. `GetSystemInfo` reports `lpMaximumApplicationAddress` just below 16 GiB. | Full 47-bit range. Everything works. |
| **Memory growth** | Memory size = highest used address. KUSER at 0x7ffe_0000 alone forces about 2 GiB. Growing is cheap and untouched pages cost no RSS (measured). On Windows hosts V8 commits on grow, so a large memory costs commit charge (not yet measured, see revisit list). | Physical memory can stay dense, sized to what is actually committed. |
| **Threads** | One shared memory per process with maximum 262144 pages. Growth is visible to all workers. Emscripten JS views must be refreshed after growth (`-sGROWABLE_ARRAYBUFFERS` or re-wrapping). | Same, plus the table itself is shared and updates must be ordered with accesses (remap races). |
| **Self-modifying code** | Store-side check of a code-page flag (`m64-wcheck`), or rely on `FlushInstructionCache` plus block checksums for pages that hold code. | The code bit folds into the write table, so it is free if traps are precise. |
| **What the translator emits** | `addr = base + index*scale + disp` in i64 (wraps like x86), a non-negative `disp` folded into the memarg offset, then `i64.load`/`store`. Optional cold checks: `addr < 0x10000` (NULL) or a range check against the guest limit, and on stores a page-flag test. | Shift, table load(s), combine, then the access (2-level: two dependent loads). With a cache: a tag compare and branch per access, a refill on miss. |

## Decision

**Option A: identity mapping, guest VA == wasm offset, one shared memory64 (maximum 16 GiB) per Windows process, everything below 16 GiB.**

- B costs @@B_COST_SHORT@@ on every memory access in the measured loops.
- Worse, B would have to be applied to Wine's Unix side too, because Wine passes raw pointers across the PE/Unix boundary and dereferences them in C.
- A costs nothing on the access path. What it loses: fixed bases above 16 GiB, huge reservations, aliasing views, and hardware-like protection. These are uncommon in 2010s desktop apps, and each has a workaround or a clear failure mode.
- We keep NULL-page and store-side code checks as explicit cold branches whose cost we measure in Phase 1. They are not a table lookup.

### Address map of one process

| Range | Use |
|---|---|
| 0x0_0000_0000-0x0_0000_ffff | NULL region. Never allocated. Guest accesses raise STATUS_ACCESS_VIOLATION via the translator's check. |
| 0x0_0001_0000-0x0_7ffd_ffff | Guest low region, about 2 GiB. Wine's DOS-area reserve, bottom-up VirtualAlloc, heaps, stacks, PEB/TEB (top-down below 2 GiB as today), low fixed-base EXEs (0x40_0000), everything a non-LAA process uses. |
| 0x0_7ffe_0000-0x0_7ffe_ffff | KUSER_SHARED_DATA (0x7ffe_0000) and the syscall-dispatcher slot at 0x7ffe_1000. Fixed by Windows ABI and Wine's thunks. |
| 0x0_7fff_0000-0x0_7fff_ffff | No-access gap, as on Windows. |
| 0x0_8000_0000-0x0_ffff_ffff | **Host region**, 2 GiB, reported to the guest as reserved. The Emscripten Unix side (`GLOBAL_BASE=0x80000000`; data, pthread stacks, a malloc arena capped below 4 GiB) and the translator's runtime data (CPU state blocks, block lookup tables, the page-state table: 1 byte per 4 KiB page = 4 MiB). |
| 0x1_0000_0000-0x3_ffff_ffff | Guest high region, 12 GiB. Images at their preferred base when it lies here (MSVC 0x1_4000_0000/0x1_8000_0000, Wine 0x1_7000_0000 and up, MinGW DLLs 0x2_xxxx_xxxx-0x3_xxxx_xxxx). The server's ASLR range for relocated DLLs moves here, top-down, replacing 0x6000_0000_0000. MEM_TOP_DOWN allocations start from a ceiling just above the highest image, not from 16 GiB, so the memory stays small. |

`user_space_limit` = `address_space_limit` = 0x4_0000_0000 (16 GiB). For non-LAA processes it is 0x7fff_0000.

Why the host region sits at 2-4 GiB:

- The memory has to be at least about 2 GiB anyway because of KUSER_SHARED_DATA.
- Putting the host there costs no extra size.
- It keeps the 2 GiB below it whole for non-LAA programs.
- 64-bit Windows programs do not depend on the 2-4 GiB range.
- A host region at the top (14-16 GiB) would force every process's memory to 16 GiB from the start.

### Revisit this decision if

1. **The checks are expensive in real code.** Phase 1 measures CoreMark and 7-Zip with the NULL check and the store-side code check on and off. If the checks we cannot drop cost more than B-with-cache (`m64-tlb1`) would, B with trap-based protection becomes competitive. That assumes precise trap recovery (re-execution from block entry) is affordable.
2. **The compatibility list hits A's limits.** That means images with fixed bases at or above 16 GiB or inside the host region; reservations larger than about 12 GiB with no fallback; apps that need a second writable view of a section, or cross-process shared sections, to be coherent; or apps that rely on faults from READONLY/NOACCESS/PAGE_GUARD (GCs with protection-based barriers, anti-tamper, custom guard pages).
3. **Windows hosts.** On browsers running on Windows, if V8's commit-on-grow makes multi-GiB memories fail or degrade on machines with a small page file, A must pack harder: rebase every relocatable image into the low region so that memory ≈ 2 GiB + actual use. B could not avoid KUSER at 0x7ffe_0000 either unless it maps it.
4. **Browsers change.** If the memory64 cap rises above 16 GiB, A simply gets more room. If the memory-control proposal ships (`memory.discard`, mapping or protecting ranges), it may give A real protection, discard or aliasing.
5. **The Unix side moves into its own memory** (for isolation or instance-per-DLL). Pointers would then need translating anyway.

## What Phase 1 must implement (translator)

- Identity-mapped memory64 accesses as in `docs/design/abi.md`. Effective addresses use wrapping i64 arithmetic, and only non-negative displacements are folded into memarg offsets.
- A fault exit with precise guest state (new exit reason) carrying the fault address and access type. Emit it from cold branches:
  - the NULL/range check (`addr < 0x10000`, optionally `addr >= guest_limit`);
  - a store-side page-flag check for code pages and write-watch.
  - Make both switchable so CoreMark and 7-Zip can measure them. Keep the explicit branches; a trap would lose the wasm locals.
- The page-state table format (1 byte per 4 KiB page: committed, R/W/X, guard, code, write-watch) as a shared ABI with the Unix side, in the host region.
- DEP: refuse to translate from non-executable pages and raise an execute access violation instead.
- Code invalidation on a code-page store hit: invalidate the blocks for that page and resume.
- An AOT and translation cache keyed by (image hash, load base). Loading is deterministic, so the key is stable.
- Recognition of the syscall thunk shape (`syscall` / `call *0x7ffe1000`) as an exit to the Unix side.

## What Phase 2 must implement (Wine's Unix side and runtime)

- **A `virtual.c` backend without mmap.** Views over the wasm memory, commit = grow if needed plus zero on recommit, protection kept only in the page-state table, and `MEM_RESERVE` failing beyond the guest range. Patch the address constants:
  - `virtual.c` L186-197;
  - the preloader reserve list, which has no preloader here and becomes a static list;
  - `server/mapping.c` `init_memory` ASLR range, moved to [0x1_0000_0000, 0x4_0000_0000) top-down;
  - Wine's virtual heap, moved into the host region or the low region.
- **Emscripten placement.** Import the memory the JS runtime creates (shared, maximum 262144 pages), and set `GLOBAL_BASE=0x80000000`. Cap stacks and the malloc arena below 0x1_0000_0000 with a custom `sbrk` limit. Never use Emscripten's `mmap` for guest memory: it zero-fills and is slow.
- **Guest pointer validation in the Unix side.** The `__TRY`/`__EXCEPT` replacement must use `virtual_check_buffer_for_read/write` and the page-state table, because bad pointers do not fault.
- **Shared pages.** KUSER_SHARED_DATA and the session shared memory are replicated by the server writing into each client's memory. MapViewOfFile gets a policy for second views (copy plus write-back at flush/unmap) and an error for unsupported cases.
- **One memory per Windows process,** created by the JS process launcher, with its own workers and its own Unix-side instance.
- **Stack reservations** committed whole, with stack-overflow detection by limit checks instead of guard pages.
