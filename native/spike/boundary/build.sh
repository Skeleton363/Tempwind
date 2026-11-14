#!/usr/bin/env bash
# Build the M1 boundary spike (docs/design/abi.md, "Wine boundary") into
# target/wine-spike/:
#   boundary.mjs, boundary.wasm   the Unix side: boundary.c with Emscripten (wasm64)
#   guest_a.wasm, guest_b.wasm    guest_a.S and guest_b.S, translated with x64wasm-cli
# Needs Docker (scripts/emcc.sh), GNU as and objcopy for x86-64, and cargo.
# Tests: tests/node/wine-boundary.test.mjs, tests/browser/wine-boundary.spec.mjs.
set -euo pipefail
cd "$(dirname "$0")/../../.."
src=native/spike/boundary
out=target/wine-spike
mkdir -p "$out"

# The guest code: flat x86-64 at fixed addresses, translated with shared memory.
as --64 -o "$out/guest_a.o" "$src/guest_a.S"
as --64 -o "$out/guest_b.o" "$src/guest_b.S"
objcopy -O binary -j .text "$out/guest_a.o" "$out/guest_a.bin"
objcopy -O binary -j .text "$out/guest_b.o" "$out/guest_b.bin"
cargo run -q -p x64wasm-cli --release -- translate --shared --base 0x10000000 \
  --entry 0x10000000 --entry 0x10000100 --entry 0x10000200 --entry 0x10000300 \
  --file "$out/guest_a.bin" -o "$out/guest_a.wasm"
cargo run -q -p x64wasm-cli --release -- translate --shared --base 0x10100000 \
  --file "$out/guest_b.bin" -o "$out/guest_b.wasm"

# The Unix side. The flags are the ones abi.md's "Wine boundary" records:
# - wasm64, importing a shared memory64 that JS creates (maximum 16 GiB =
#   262144 pages, initial 4 GiB: the host region [2 GiB, 4 GiB) from the
#   start), static data from 2 GiB; SHARED_MEMORY without pthreads turns on
#   atomics and bulk memory;
# - sbrk wrapped (boundary.c's __wrap_sbrk) so the heap stops at 4 GiB and
#   never grows the memory (wine-unix.md, section 5);
# - the ABI's constants from native/wine/include/tempwind/abi.h;
# - wasm exception handling, also for setjmp/longjmp;
# - no -sJSPI: the driver wraps thread_main in WebAssembly.promising and
#   green_block in WebAssembly.Suspending itself;
# - the raw stack functions exported for starting green threads.
scripts/emcc.sh -m64 -O2 -Wall -Werror --no-entry -Inative/wine/include \
  -sIMPORTED_MEMORY -sSHARED_MEMORY -sALLOW_MEMORY_GROWTH \
  -sINITIAL_MEMORY=4GB -sMAXIMUM_MEMORY=16GB -sGLOBAL_BASE=2GB -Wl,--wrap=sbrk \
  -fwasm-exceptions -sSUPPORT_LONGJMP=wasm \
  -sMODULARIZE -sEXPORT_ES6 -sENVIRONMENT=node,worker \
  -sEXPORTED_FUNCTIONS=_malloc,_emscripten_stack_set_limits,__emscripten_stack_restore \
  --js-library "$src/library.js" \
  -o "$out/boundary.mjs" "$src/boundary.c"
echo "built $out: $(cd "$out" && ls boundary.mjs boundary.wasm guest_a.wasm guest_b.wasm | tr '\n' ' ')"
