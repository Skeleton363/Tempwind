# native/wine/

Wine 11's ntdll Unix part and wineserver, built with Emscripten 6.0.12 as one wasm64 module that runs in a Windows process's Worker. The contract is `docs/design/wine-unix.md`; the boundary to translated code is `docs/design/abi.md` v5, "Wine boundary".

## License

Everything in this directory is LGPL-2.1-or-later, as Wine is (`COPYING.LIB`, from the wine-11.0 tarball): the patches against Wine (`patches/`), our new Unix-side files (`src/`), the C harnesses (`tests/`), `config.h`, `rename-server.h`, `imports.txt` and the build script. The JS host (`runtime/src/wine/`) and the Node tests (`tests/wine/`) are the project's own code.

## Contents

- `build.sh`: the build (section 2 of the contract).
- `config.h`: the hand-curated configure result, a non-Linux POSIX profile of the 6.0.12 sysroot.
- `patches/NNNN-area-what.patch`: diffs against the pristine tarball, applied in name order with no fuzz. `00xx` headers and build, `01xx` the virtual backend, `02xx` the server, `03xx` ntdll startup.
- `rename-server.h`: renames the server's globals that collide with ntdll's (gate 3).
- `imports.txt`: the wasm imports the module may have, each with its reason (gate 2).
- `src/`: `boot.c`, `green.c`, `green_pthread.c`, `inproc.c`, `signal_wasm64.c`, `stubs.c`, `version.c`, `virtual_wasm.c`, `tempwind_private.h` (their shared declarations) and `library.js` (the module's JS imports).
- `tests/`: C harnesses (`harness.h` and one `NAME.c` per harness), run by `tests/wine/NAME.test.mjs`.

## Build

Needs Docker (`scripts/emcc.sh` runs `emscripten/emsdk:6.0.12`), curl, patch, xz, gzip and Node 24.

```sh
native/wine/build.sh [--out DIR] [--jobs N] [--census]   # DIR defaults to target/wine-unix
npm run test:wine                                         # tests/wine/*.test.mjs against DIR (or TW_WINE_UNIX_DIR)
```

`build.sh` downloads the tarball named under `"wine"` in `tests/targets/sources.json` into `DIR/src/` and checks its sha256, extracts what it compiles and applies the patches (again only when the tarball or a patch changed), then compiles and links in one `make` inside the Emscripten container: `DIR/wine-unix.{mjs,wasm}` and `DIR/test-NAME.{mjs,wasm}` for each harness. Then the three gates: no undefined symbols (the link), every import listed in `imports.txt`, no unrenamed server/ntdll collision (`--census` prints the list). It writes `DIR/build-info.json`. Logs are in `DIR/obj/make.log`. `DIR` must be inside the repository, which the container mounts.

## Patch workflow

Edit a copy of the extracted tree, then write the diff with `a/` and `b/` prefixes into the patch of your range, below its header (what it changes and why, and the section it implements). A patch that doesn't apply cleanly to the pristine tarball fails the build.
