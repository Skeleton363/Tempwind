# JS runtime: what it keeps

Status: **decided for Phase 2 M4.** Phase 1 ran PE programs on a Win32 shim written in JS (`Process`, a run loop, a host-call function and one module per DLL). Phase 2 M4 deleted it (WP6, the owner's decision of 2026-11-21, `docs/status/phase-2.md`): Wine's PE side and the Wine host (`runtime/src/wine/`, `wine-unix.md` v3) run every program. The Phase 1 design is in git history and `docs/status/phase-1.md`.

## Pieces

| Piece | Job |
|---|---|
| Translator CLI `translate-pe` | AOT-translates a PE into one module, and writes the load plan (below) from `x64wasm::pe`. JS never parses PE files. |
| Linker (`link.mjs`) | Owns the table, the `table_base`/`lookup`/`depth` globals and the lookup table in guest memory. It links modules: AOT modules and run-time translations. |
| Run-time translator (`rt.mjs`) | Wraps `x64wasm_rt.wasm` (`abi.md`); the host calls it when execution reaches code no module has translated. `requestFlags(options)` turns a plan's `options` into the request flags. |
| Floating-point helpers (`fp.mjs`) | `x64wasm_fp.wasm` (`abi.md`, v4): one instance per process, whose exports are the `fp` import namespace of every module linked. |
| Plan and memory (`plan.mjs`, `memory.mjs`, `guest-memory.mjs`, `abi.mjs`) | The load plan's parsing and checks, memory64 creation, bounds-checked guest access, the ABI constants. |
| Spike and selftest hosts (`run.mjs`, `node-host.mjs`, `browser-host.mjs`, `worker.mjs`) | Run the spike programs (`target/spike`) and the selftest in Node, pages and Workers. |
| Wine host (`runtime/src/wine/`) | `wine-process.mjs` creates a Wine process (the boot set, the AOT catalog, host files, the memory checks on); `run-wine.mjs` is the command-line runner (`node runtime/src/wine/run-wine.mjs FILE.exe`); `web-files.mjs` loads the pieces in a browser. Contract: `wine-unix.md` v3. |

The address map of a Wine process is `guest-memory.md` and `wine-unix.md`; Phase 1's map and PEB/TEB layout went with `Process`.

## Load plan (`translate-pe` → `<stem>.plan.json`)

u64 values are `"0x…"` hex strings, as in the spike manifest; addresses are virtual addresses.

```json
{
  "version": 1,
  "file": "basic.exe",
  "sha256": "…",
  "machine": "0x8664",
  "imageBase": "0x140000000",
  "sizeOfImage": "0x6000",
  "sizeOfHeaders": "0x400",
  "entryPoint": "0x140001000",
  "subsystem": 3,
  "stackReserve": "0x200000",
  "stackCommit": "0x1000",
  "sections": [
    { "name": ".text", "va": "0x140001000", "virtualSize": "0x2a4",
      "fileOffset": "0x400", "fileSize": "0x400", "characteristics": "0x60000020" }
  ],
  "imports": [
    { "dll": "KERNEL32.dll",
      "functions": [ { "name": "ExitProcess", "ordinal": null, "iat": "0x140005070" } ] }
  ],
  "tls": null,
  "hasRelocations": true,
  "options": { "nullCheck": false, "storeCheck": false, "counters": false, "syncAll": false, "structured": true, "sharedMemory": false }
}
```

- `tls`, when the image has a TLS directory: `{ "startOfRawData", "endOfRawData", "addressOfIndex", "sizeOfZeroFill", "callbacks": [VA, …] }`, all `"0x…"` strings.
- A section's `virtualSize` is its header's VirtualSize, or SizeOfRawData when that is 0; `fileSize` is SizeOfRawData.
- `hasRelocations` is true when the relocation directory is non-empty and the image isn't marked `IMAGE_FILE_RELOCS_STRIPPED`.
- An import by ordinal has `"name": null` and the ordinal as a number.
- `sha256` is the SHA-256 of the PE file. The runtime refuses a plan whose `sha256` doesn't match the file it was given.
- `options` (M6): the translator options the AOT module was built with that change what the code does at run time, or that a reference run must keep (`syncAll`, `structured`), and whether its memory import is shared (`sharedMemory`, Phase 2 M3: `translate-pe --shared`), so code translated at run time imports the same memory. The runtime passes them as the run-time translator's request flags (`abi.md`), and supplies `env.page_flags` and `env.counters` to every module that imports them. A plan without `options`, or without one of its fields, means the defaults: `structured` true, the others false.

## Benchmarks (M4)

**On Wine from Phase 2 M4:** both benchmarks run their translated side through the Wine host (`run-wine.mjs`, with the catalog's AOT modules and both memory checks, `wine-unix.md` v3), with Phase 1's arguments: CoreMark `0x0 0x0 0x66 0`, and `7zz.exe b 2 -mmt1 -md18`. Each JSON line gains `"host": "wine"`; Phase 1's lines have no `host` and were the shim's. `bench.yml` fetches the `wine.lock` and `wine-unix.lock` releases and prepares `target/wine` before it runs them, and on pull requests it compares the ratio only with the last line of the same host. The rest of this section is Phase 1's, and holds for the native sides and the records.

`tests/bench/coremark.mjs` translates `coremark.exe` (all entries), runs it translated in Node with `0x0 0x0 0x66 0`, and natively under wine64 with the same arguments. Iteration count 0 makes CoreMark calibrate itself to a run of at least 10 seconds, so a valid run prints "Correct operation validated". The script prints one JSON line with:
- the commit and the date;
- Node's version and the CPU model;
- iterations per second, translated and native, each the median of 3 runs;
- their ratio.

A run that isn't validated fails the script.

`.github/workflows/bench.yml` runs it on every push to `main` and on pull requests, and also runs `coremark.exe` natively on windows-2025 for the record.
- **On main,** it appends the JSON line to `coremark.jsonl` on the orphan branch `bench-data`, which the workflow creates on its first run. The commit comes from the workflow's bot, with `contents: write` for this job only. Each benchmark records on its own: if one job fails, the other's result is still appended.
- **On pull requests,** it compares the ratio with the last line on `bench-data` and emits a warning annotation, never a failure, when it is more than 10 % worse.
