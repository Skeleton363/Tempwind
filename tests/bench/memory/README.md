# Guest-memory micro-benchmark

This benchmark measures the cost of the guest-memory options in `docs/design/guest-memory.md`: identity mapping (option A) against a translation table (option B), plus hybrids and the extra checks option A may need.

`core.js` hand-assembles one small WebAssembly module per variant, with no toolchain needed. Each module runs six loops over a 256 MiB guest buffer:

- **hot**: loads cycling through a 32 KiB window. The window stays in L1, so this shows pure address-computation cost.
- **seq**: sequential 8-byte loads.
- **rand**: independent loads at LCG-random slots.
- **chase**: dependent loads along a random cycle.
- **hotst**: stores cycling through the same 32 KiB window.
- **store**: sequential 8-byte stores.

It reports the median ns per access.

| Variant | Address computation per access |
|---|---|
| `m32` | memory32; guest address wrapped to i32 (baseline) |
| `m64` | memory64; guest VA == wasm offset (option A) |
| `m64-flat` | one-level table, 4-byte entry per 64 KiB page, 2^40 VA (option B) |
| `m64-2level` | two-level table, 2^48 VA (option B) |
| `m64-tlb1` | option B behind a one-entry cache in locals (hybrid) |
| `m64-tlb256` | option B behind a 256-entry software TLB in memory (hybrid) |
| `m64-nullcheck` | option A plus `va < 0x10000` check with a cold fault branch |
| `m64-bias` | option A with offset = VA - 0x10000 (NULL traps out of bounds) |
| `m64-wcheck` | option A; each store tests a 1-byte-per-4-KiB page-flag table (SMC / write-watch) |

Every variant must compute identical checksums; the runner fails otherwise.

It also measures:

- growing a memory64 to 15 GiB, shared and non-shared, with RSS in Node;
- first-touch cost;
- how many 16 GiB-maximum memories one agent can hold;
- whether a maximum above 262144 pages is accepted.

## Run

```sh
node tests/bench/memory/bench.mjs                      # Node 24 (pinned in .nvmrc; memory64 needs no flag)
node --wasm-memory64-trap-handling tests/bench/memory/bench.mjs --no-growth   # Node 22 only; on by default since V8 13.6 (Node 24)
node tests/bench/memory/bench.mjs --browser            # headless Chromium via --dump-dom
node tests/bench/memory/bench.mjs --runs 5 --json out.json --variants m64,m64-flat
```

Options:

- `--browser [PATH]` runs `bench.html` in headless Chromium. The binary is, in order: the explicit `PATH`; the `CHROME_PATH` environment variable when non-empty; Playwright's own Chromium (`chromium.executablePath()` from `@playwright/test`, so run `npm ci` first). The chosen binary is printed on stderr and in the report header. A path that does not exist is an error, not a fallback.
- `--runs N` takes the median of N complete runs. The default is 1; the recorded results use 5.
- `--reps N` sets the repetitions per run (default 5).

`bench.html` also works when opened directly from `file://`.

A full run of all variants takes about 1 minute per run in Node and needs about 400 MiB of RAM.

## Results

Recorded results are in `results/*.json`: `node24.json` (Node 24.22, V8 13.6, median of 5 runs, all six loops, memory-count probe and growth), `chromium141.json` (headless Chromium 141), and the older `node22*.json`. The summary and its interpretation are in `docs/design/guest-memory.md`.
