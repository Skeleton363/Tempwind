# 7-Zip command matrix (M5)

Phase 1's goal: the 64-bit 7-Zip CLI (`7zz.exe`, the pinned targets release) compresses, tests, lists, extracts and hashes exactly like native Windows. This directory holds the matrix that proves it.

## Pieces

| File | Job |
|---|---|
| `tree.mjs DIR` | Writes the input tree, deterministic on every OS (details below). |
| `matrix.mjs --mode native\|wine\|translated --out OUT [--exe 7zz.exe] [...]` | Runs every step in order, in a fresh work directory under OUT that holds the input tree, and records the results in OUT. |
| `compare.mjs OUT EXPECTED` | Compares byte for byte and names the first difference: step, file, offset. Exits non-zero on any difference. |
| `steps.json` | The steps: the 7-Zip arguments and what to keep. |
| `expected/` | The committed results of a native run. |

**Modes:**
- **native:** runs `7zz.exe` directly (Windows).
- **wine:** runs it through `wine64`.
- **translated:** uses `run-pe` with `--dir` on the work directory, the AOT module and plan from `translate-pe`, `--rt` and `--fp`. Each step is one process.

All modes use relative paths inside the work directory, so no host path shows in an output.

## Input tree

Every file is ASCII-named and generated from a seeded PRNG or fixed text, never from `/dev/urandom`. It holds:
- text;
- a compressible file (numbers);
- a 64 KiB incompressible file;
- an empty file;
- x86-64 code bytes, for the BCJ filter;
- two nested directories.

Its total size is about 300 KiB. Every file and directory gets a fixed mtime on an even second, for zip's DOS times.

## Steps

**Every step:**
- passes `-mmt1 -bsp0` (single thread, no progress);
- names compression parameters explicitly (`-mx`, `-md`, `-m0=…`);
- leaves out creation and access times (`-mtc=off -mta=off` where the format has them), so nothing depends on the host clock, RAM or CPU count.

**Archives (`a`):**
- **7z:** LZMA, LZMA2, PPMd, BZip2, Deflate, Copy; levels 1, 5 and 9; solid and non-solid; BCJ on the code file.
- **zip:** Deflate, Deflate64, BZip2, LZMA, Copy.
- **Other formats:** tar; gz, bz2 and xz of single files.

**On each archive:**
- `t`;
- `l -slt`;
- `x` into a fresh directory. The tree is recorded as a manifest: path, size, SHA-256, mtime.

**Hashes:** `h` over the tree, with each hash type 7-Zip offers (`-scrc*`).

**Recorded per step:** stdout (raw bytes, CRLF as Windows writes it), stderr, the exit code, and every archive the step created.

**Excluded:**
- **Encryption:** the salt and IV are random. It gets a round trip only: `a -p` then `x -p`, comparing the extracted tree.
- **`7zz i`:** it prints the CPU's name.
- **Error paths.**

## Expected results and CI

- **Generating them:** `matrix.mjs --mode wine` in a session writes `expected/`. The files are small (archives of a ~300 KiB tree).
- **windows-x64 (gating):** runs `--mode native` on windows-2025 and compares with `expected/`. Real Windows is the reference of truth, so when it disagrees, the expected files are fixed from its results.
- **linux-x64 (gating):** runs `--mode translated` in Node and compares with `expected/`.
- **`--mode wine`:** a non-gating cross-check where Wine is installed.
- **Time budget:** the translated matrix stays under about five minutes in CI.

## `7z b`

`.github/workflows/bench.yml` runs `7zz b -mmt1` with fixed parameters next to CoreMark. It runs translated in Node and natively under wine64 on the same runner (the ratio of the "Tot:" ratings), plus a windows-2025 run for the record. Main appends one JSON line to `sevenzip.jsonl` on the `bench-data` branch. Pull requests warn, never fail, when the ratio is more than 10 % worse.
