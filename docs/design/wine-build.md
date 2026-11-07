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
- **Assets are never replaced.** An asset already on a tag stays as it is, and a new build is a new tag.
- **Retries:** both publish jobs go through `scripts/release-publish.sh`, which retries every API call. The releases API has twice refused a run's token with a transient 403, "Resource not accessible by integration".
- **Without traces,** `closure` runs on the static edges alone and says so in `closure.json`'s `note`.

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

  - `overlaps` lists the pairs whose `[base, base + size)` ranges intersect.
  - `highest_end` is the largest `base + size`.
  - `memory_bytes` is the guest memory that loading every module at its preferred base implies, rounded up to 64 KiB.
  - `data_files` lists the files the Unix side reads, read from the native trace (`+file` or `strace`): NLS, `wine.inf`. They are not translated, and M2's FS layout includes them.

## The prefix
- **How it is made:** with the native build, `WINEPREFIX=$PWD/prefix WINEDEBUG=-all /opt/wine-11.0/bin/wine wineboot --init`, then `wineserver -w`.
- **What is kept:** `system.reg`, `user.reg`, `userdef.reg`, `drive_c` and `dosdevices`, as made. The tarball records the prefix path it was made at, since the `.reg` files contain it.
- **What is added:** wineboot's per-boot volatile keys don't reach the `.reg` files. The Emscripten Unix side adds them from a generated overlay (`wine-unix.md`, M2).

## Conformance baselines
- **Units:** `kernel32_test.exe --list` and `ntdll_test.exe --list` print "Valid test names:", then one name per line.
- **Each unit runs alone:**
  - `<exe> <unit>`, with a fresh temporary working directory;
  - `WINETEST_DEBUG=1`, so the summary line prints;
  - `WINETEST_PLATFORM=wine` under Wine, unset on Windows;
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

  - **`ok`:** a summary line was seen, failures = 0 and the exit code is 0.
  - **`failed`:** a summary line was seen and failures > 0.
  - **`crashed`:** no summary line, or an exception exit code (≥ 0xC0000000).
  - **`timeout`:** the unit was killed at the limit.
  - **`unsupported`** (tempwind only): a `tempwind: unsupported:` line was seen.
- **Rates:** units `ok` / units listed, and checks (executed − failures) / executed, both per exe and both against each baseline. M5's `conformance.yml` records them per main commit on the orphan branch `conformance-data`.
