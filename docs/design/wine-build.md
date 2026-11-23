# Wine build (Phase 2)

How CI builds Wine 11 from the dl.winehq.org tarball, what it publishes, and the data the rest of Phase 2 builds on: the DLL closure, the prefix and the conformance baselines. Written for M1 (`docs/status/phase-2.md`). Wine's Unix side, built with Emscripten, is a separate contract (`wine-unix.md`, from M2).

## Source
- **The tarball:** `https://dl.winehq.org/wine/source/11.0/wine-11.0.tar.xz`.
  - It is pinned in `tests/targets/sources.json` under `"wine"`: version, url, sha256, and the sha512 that `https://dl.winehq.org/wine/source/11.0/sha512sums.asc` lists.
  - Trust on first use, as for the targets: an empty sha256 fails the job, which then prints what it found.
  - It is mirrored once as the release `src-wine-11.0`, with the tarball and its `.sha256`.
- **Unmodified.** The PE side and the native build use the pristine tarball. Our patches (LGPL, `native/wine/`, from M2) apply only to the Emscripten build of the Unix side.

## Native build (`.github/workflows/wine.yml`, ubuntu-24.04)
- **Toolchain:**
  - for the PE side, the MinGW-w64 GCC 13 packages the targets use (`gcc-mingw-w64-x86-64-win32`, recorded by `tests/targets/build-info.sh`);
  - for the Unix side, Ubuntu's gcc;
  - flex and bison;
  - packages installed through `scripts/apt-install.sh`.
- **configure:**
  - `--enable-archs=x86_64` (64-bit only; no i386 PE side);
  - `--without-x --without-wayland --without-freetype --without-fontconfig --without-opengl --without-vulkan --without-xcomposite --without-alsa --without-pulse --without-oss --without-gstreamer --without-ffmpeg --without-cups --without-sane --without-gphoto --without-usb --without-v4l2 --without-netapi --without-krb5 --without-gssapi --without-gnutls --without-opencl --without-pcap --without-dbus --without-udev --without-sdl --without-capi`;
  - tests enabled (the default);
  - **why minimal:** the Emscripten port has none of these libraries either, so the native baseline and our port run the same code paths. It also makes CI faster.
- **Build:** `make -j$(nproc)` under ccache. The cache key covers the tarball sha256, the configure flags and the compiler versions.
- **Install:** `make install DESTDIR=… prefix=/opt/wine-11.0`.
- **Symbols:** PE modules keep their COFF symbol tables, stripped with `--strip-debug` only. The translator names chunks after COFF symbols (`abi.md`, "Module shape").
- **Native CRT:** Wine's own modules link against `ucrtbase`, and its test executables against `msvcrt` (`tools/makedep.c` L2238). Both are in the closure.

## Release `wine-<sha7>`
`sha7` is the commit of the workflow run. Assets, each with a `.sha256`:

| Asset | Contents |
|---|---|
| `wine-pe-x86_64.tar.zst` | `lib/wine/x86_64-windows/*` from the install: every PE DLL, EXE and driver. Also `tests/kernel32_test.exe` and `tests/ntdll_test.exe`. |
| `wine-native.tar.zst` | The native install tree (`opt/wine-11.0/…`). It runs on ubuntu-24.04, the session image, for baselines, prefix creation and `/diverge` traces (`WINEDEBUG=+relay`). |
| `wine-prefix.tar.zst` | The prefix that this build's `wineboot --init` made (below). |
| `closure.json` | The DLL closure (below). |
| `baseline-windows.json`, `baseline-wine.json` | Conformance baselines (below). |
| `loaddll-traces.tar.zst` | The raw `+loaddll` traces the closure was computed from. |
| `build-info.json` | The tarball's sha256 and sha512, toolchain package versions, configure flags, the run URL. |

- `tests/targets/wine.lock` names the release that sessions and later jobs use, as `targets.lock` does.
- To fetch: `scripts/fetch-artifact.sh "$(cat tests/targets/wine.lock)" wine-pe-x86_64.tar.zst`.
- **Publishing order:**
  - `publish-build` creates the release as soon as the build passes: the PE modules, the native tree, the prefix and `build-info.json`.
  - The same run's `publish-results` then adds the baselines, the traces and `closure.json`. It runs even when one of those jobs failed, so whatever exists is published, and a missing asset is a warning in the run.
  - **Why:** workflow artifacts can't be downloaded in a session (their storage host is outside the session's network allowlist), so a release is the only way a session gets the build. Run 42306869519 built Wine, but its late `baseline-wine` failure left nothing published.
- **Assets are never replaced.** An asset already on a tag stays as it is, and a new build is a new tag. An asset that is there without its `.sha256` (an earlier attempt stopped between the two uploads) gets one made from the release's copy, which may come from another build of the same commit.
- **Retries:** both publish jobs go through `scripts/release-publish.sh`, which retries every API call. The releases API has twice refused a run's token with a transient 403, "Resource not accessible by integration".
- **Without traces,** `closure` runs on the static edges alone and says so in `closure.json`'s `data_files_note` (below).

## The DLL closure
The set of PE modules that a Phase 2 run may load. It is the scope of the census (`instruction-scope.md`) and of AOT translation.

- **Roots:**
  - `tests/pe/basic.exe`;
  - `tests/native`'s hello, l2_int, l2_crt and l2_float;
  - `coremark.exe` and `7zz.exe` (`targets.lock`);
  - `kernel32_test.exe` and `ntdll_test.exe`.
- **Static edges:**
  - import and delay-load import tables;
  - export forwarders (`other.dll!fn` adds other.dll);
  - `api-ms-win-*` and `ext-ms-win-*` names, resolved through Wine 11's apiset schema to their host DLL. A name with no host is listed, not added.
- **Dynamic edges:** every module that native Wine 11 loads while running a root, from `WINEDEBUG=+loaddll` traces.
  - The programs run with their usual arguments.
  - Each test unit runs on its own.
  - This catches `LoadLibrary` and the delay loads that run.
- **Always in:** `apisetschema.dll`, which is data and is mapped by the Unix side.
- **The candidates** are the files in `lib/wine/x86_64-windows`. One without the `MZ` signature is data and is skipped. A PE image whose headers or tables don't parse fails the closure, since dropping it would silently drop its edges.
- **`closure.json` schema 1:**

  ```
  { "schema": 1, "wine": "wine-<sha7>",
    "modules": [ { "name": "kernelbase.dll", "path": "lib/wine/x86_64-windows/kernelbase.dll",
                   "base": "0x174000000", "size": "0x…", "via": ["static", "loaddll"],
                   "roots": ["hello.exe", "…"] } ],
    "overlaps": [ ["a.dll", "b.dll"] ],
    "highest_end": "0x…",
    "memory_bytes": …,
    "data_files": [ "share/wine/nls/locale.nls", "share/wine/wine.inf", "…" ] }
  ```

  - `path` is relative to the tree (`tests/kernel32_test.exe`), however `--root` names a root inside it; a root outside the tree keeps the path it was given (`roots/basic.exe`), and `census --closure` matches such roots to its FILES by file name.
  - `via`: `root` (a `--root`), `always` (`apisetschema.dll`), `static`, `loaddll`.
  - `overlaps` lists the pairs of DLLs whose `[base, base + size)` ranges intersect. Roots are left out, since no two load together. `root_overlaps` lists each root with the DLLs it loads that intersect it.
  - `highest_end` is the largest `base + size`, and `lowest_base` the smallest `base`. `sum_of_sizes` adds up every module's size.
  - `memory_bytes` is the guest memory that loading every module at its preferred base implies: `highest_end` rounded up to 64 KiB. `memory_bytes_definition` repeats this in the file.
  - `unresolved` lists each name with no file in the tree and no apiset host, with up to 8 modules or traces that name it (`(loaddll kernel32_test.exe)`). Most are temporary DLLs the tests create.
  - `trace_files` counts the `+loaddll` traces read; 0 means static edges only.
  - `data_files` lists the install tree's `share/wine/nls/*.nls` and `share/wine/wine.inf`. That is a superset of what the Unix side reads, not narrowed by a `+file` trace. They are not translated. M2's FS layout includes them, and may narrow the list from a trace.
  - `data_files_note` is the closure job's `--note`: how `data_files` were listed, and "Static edges only" when there were no traces.

## Packaging for the Unix side (Phase 2 M3)
What a Wine process loads besides the release:
- the closure's DLLs, rebased to unique bases;
- an AOT module for each PE file;
- a manifest of the boot set.

They are built from `wine.lock`'s release and the translator at the commit: by `tests/wine/prepare.sh` in a session, and by the `wine-unix` workflow in CI. None of them is published.

### The rebase (`native/wine/rebase/`)
- **The tool** is a Rust crate under `native/wine/`, LGPL-2.1-or-later like the rest of that directory. It is a workspace member with its own `license` field. It reads PE files itself and depends on no crate of ours.
- **Input:** the release's PE tree and its `closure.json`.
- **What moves:** every closure DLL moves, except four:
  - `ntdll.dll`, `kernelbase.dll` and `kernel32.dll`, which keep their own bases (0x1_7000_0000, 0x1_7400_0000, 0x1_7800_0000);
  - `apisetschema.dll`, which is data.

  EXEs keep their bases too.
- **The bases:** the modules are taken in name order. Each takes the highest 64 KiB-aligned base that leaves room for its `SizeOfImage` below the previous one; the first goes below 0x1_7000_0000.
  - The tool fails if a base would fall below `TW_IMAGE_RANGE_END` (0x1_6000_0000).
  - `wine-4c6ec0d`'s closure needs 26.1 MiB of the 256 MiB (`research.md`, 2026-11-14).
- **What changes in a file:**
  - `OptionalHeader.ImageBase`;
  - every `IMAGE_REL_BASED_DIR64` target, by the delta. `ABSOLUTE` entries are padding. Any other relocation type fails the tool, naming the file;
  - `OptionalHeader.CheckSum`, recomputed. Wine compares checksums only to find a module already loaded (`dlls/ntdll/loader.c` L2791) and never validates them.
- **What stays:** everything else, including the DOS stub with Wine's builtin signature, `DYNAMIC_BASE` and the `.reloc` section. The server keys builtin detection and `ImageDynamicallyRelocated` on them (`server/mapping.c` L935-944).
- **A DLL without relocations** can't move, and the tool fails, naming it.
- **Tests,** on real closure DLLs:
  - every DIR64 target moves by the delta;
  - no other byte changes except `ImageBase` and `CheckSum`;
  - the packing is deterministic.
- **`rebase.json` schema 1** lists the modules that moved:

  ```
  { "schema": 1, "wine": "wine-<sha7>", "range": ["0x160000000", "0x170000000"],
    "modules": [ { "name": "msvcrt.dll", "path": "lib/wine/x86_64-windows/msvcrt.dll",
                   "old_base": "0x180000000", "base": "0x16f…", "size": "0x…",
                   "sha256": "<the rebased file's>" } ] }
  ```

### The catalog
- **One AOT module per PE file:**
  - each closure DLL, rebased or at its own base;
  - each test program: hello, l2_int, l2_crt and l2_float, and from M4 basic, l2_threads, l2_suspend, coremark and 7zz (`tests/wine/catalog.mjs`'s `PROGRAMS`).

  Each is translated with `x64wasm-cli translate-pe --shared --null-check --store-check` (`translator.md`, "Wine processes"). `apisetschema.dll` is data and gets none.
- **`catalog.json` schema 1:**

  ```
  { "schema": 1, "wine": "wine-<sha7>", "translator": "<commit>",
    "options": ["--shared", "--null-check", "--store-check"],
    "modules": [ { "path": "/wine/lib/wine/x86_64-windows/msvcrt.dll", "sha256": "…",
                   "base": "0x…", "wasm": "msvcrt.dll.wasm", "plan": "msvcrt.dll.plan.json",
                   "wasm_bytes": … } ] }
  ```

  - `path` is the file's path in MEMFS. JS finds that path from the fd that `tw_image_mapped` passes (`abi.md` v5).
  - The programs' paths are under the prefix's `drive_c`, where the host writes them.
  - JS links a module when the image's path, its sha256 and its mapped base all match.
    - The sha256 of a lazy file is the boot-set manifest's. In Node the host checks the file against it on the first read.
    - The program is the caller's file, written up front, so its sha256 comes from its own bytes, hashed before `boot`. A program the manifest doesn't list, such as `run-wine.mjs FILE.exe`, then still matches a catalog module built from the same bytes.
- **The boot-set manifest,** `boot-set.json` schema 1, comes from the same step. Each file has its MEMFS `path`, `source`, `size`, `sha256`, and `lazy`:
  - `source` is relative to `target/wine`;
  - `lazy` is true for the closure's PE files and the `data_files`, which the host loads on their first read (`wine-unix.md`, section 8).

### Layout under `target/wine` (`TW_WINE_DIR`)
- `pe/`, `native/` and `wine-prefix/`: the release, as in M2;
- `rebased/lib/wine/x86_64-windows/`: the DLLs that moved, and `rebase.json`;
- `catalog/`: `catalog.json`, and `<name>.wasm` and `<name>.plan.json` per file;
- `boot-set.json`;
- `prefix.json` (M3): the prefix as made, listed for a host that can't walk `wine-prefix/`, which is the browser host (`wine-unix.md`, section 8).
  - Schema 1: `{ "schema": 1, "path": "<wine-prefix.path>", "files": [ … ], "dirs": [ … ], "links": [ [path, target], … ], "installFiles": [ … ] }`.
  - The values are what `readPrefix` (`runtime/src/wine/node-files.mjs`) returns, in its order. Paths are relative to the prefix. `files` holds the names of the top-level files, whose bytes are `wine-prefix/<name>`. `installFiles` are `drive_c`'s file paths, for the install copies.
  - `tests/wine/catalog.mjs` writes it with `readPrefix`, next to `boot-set.json`. Its `--check` fails when the file differs from a fresh walk.
- `bridge.wasm` (`x64wasm-cli bridge`);
- the test programs' `.exe` files.

### Where it is built
- **In a session:** `tests/wine/prepare.sh` builds the catalog for the modules the test programs load (`closure.json`'s `roots`), or for the whole closure with `--all`.
- **In CI:** `wine-unix.yml` builds the whole closure and compiles every module in V8 (Node 24, `new WebAssembly.Module`). That is the translation gate, and a failure names the file. Then it runs the programs test in Node, and from M3's browser work package hello.exe in Chromium.
  - **From M4** a separate job runs the 7-Zip matrix on our Wine (`tests/sevenzip/README.md`) from the same build, so the programs job stays short and a matrix failure is reported on its own.
  - `bench.yml` prepares `target/wine` the same way, from the `wine.lock` and `wine-unix.lock` releases, for the benchmarks (`runtime.md`, "Benchmarks").

## The prefix
- **How it is made:** with the native build installed at its final path, `/opt/wine-11.0`: `WINEPREFIX=$HOME/wine-prefix WINEDEBUG=-all /opt/wine-11.0/bin/wine wineboot --init`, then `wineserver -w`.
- **What is kept:** `system.reg`, `user.reg`, `userdef.reg`, `drive_c` and `dosdevices`, as made. The tarball holds `wine-prefix/` and `wine-prefix.path`, the path the prefix was made at, since the `.reg` files contain it.
- **What is added:** wineboot's per-boot volatile keys don't reach the `.reg` files. The Emscripten Unix side adds them from a generated overlay (`wine-unix.md`, M2).

## Conformance baselines
- **Units:** `kernel32_test.exe --list` and `ntdll_test.exe --list` print "Valid test names:", then one name per line.
- **Each unit runs alone:**
  - `<exe> <unit>`, with a fresh temporary working directory;
  - `WINETEST_DEBUG=1`, so the summary line prints;
  - `WINETEST_PLATFORM=wine` under Wine, unset on Windows;
  - under native Wine also `WINEDEBUG=+loaddll`, so the same pass gives the closure's dynamic edges; each unit's stderr is kept as its trace. The trace adds a stderr line per module load, and `seconds` includes it;
  - a 120 s limit (300 s for translated runs, which the runner records);
  - stdout and stderr captured.
- **Platforms:**
  - `windows-2025`: the runner, native, the reference of truth;
  - `wine-11.0-native`: the native build on ubuntu-24.04 with the prefix, `--without-x` as built;
  - `tempwind`: our port (M5).
- **Parsing:** every line `^[0-9a-f]{4}:<unit>:… <n> tests executed (<t> marked as todo, <f> as flaky, <e> failure(s)), <s> skipped.` is summed per unit, child processes included (`include/wine/test.h` L791). The process exit code is failures plus todo failures (L798).
- **Results file schema 1:**

  ```
  { "schema": 1, "platform": "windows-2025" | "wine-11.0-native" | "tempwind",
    "exe": "kernel32_test.exe", "wine": "wine-<sha7>", "commit": "<sha>", "run": "<url>",
    "units": [ { "unit": "file", "status": "ok" | "failed" | "crashed" | "timeout" | "unsupported",
                 "executed": 0, "todo": 0, "flaky": 0, "failures": 0, "skipped": 0,
                 "exit": 0, "seconds": 0.0 } ] }
  ```

  The first status whose rule matches is the unit's (`tests/conformance/parse.mjs`):
  - **`timeout`:** the unit was killed at the limit.
  - **`unsupported`:** a `tempwind: unsupported:` line was seen. Every platform is checked, but only our port prints it.
  - **`crashed`:** no summary line, killed by a signal (`exit` −1), or an exception exit code (≥ 0xC0000000).
  - **`failed`:** failures > 0, or failures = 0 with a non-zero exit code.
  - **`ok`:** failures = 0 and the exit code is 0.
  - **Native Wine's exit status is the exit code's low 8 bits** (`wine` is a Unix process), so 0xC0000005 arrives as 5. There, `crashed` rests on the missing summary line. A main process that crashes after its child processes printed their summaries is counted `failed`. No unit in `wine-4c6ec0d`'s baseline is such a case: every `failed` unit's exit code equals its failures.
- **The release assets** `baseline-windows.json` and `baseline-wine.json` wrap one results object per exe: `{ "schema": 1, "platform", "wine", "commit", "run", "results": [ <kernel32_test.exe's>, <ntdll_test.exe's> ] }`.
- **Rates:** units `ok` / units listed, and checks (executed − failures) / executed, both per exe and both against each baseline. M5's `conformance.yml` records them per main commit on the orphan branch `conformance-data`.
