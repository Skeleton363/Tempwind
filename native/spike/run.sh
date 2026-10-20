#!/usr/bin/env bash
# Emscripten spike (Phase 0, item 6): build the probes natively with gcc
# (x86_64-linux-gnu, the usual target of Wine's x86-64 Unix side) and with
# emcc -sMEMORY64=1 -pthread (wasm64-emscripten) via scripts/emcc.sh, run
# both, and write native/spike/out/report.md.
#
# Exits nonzero if an LP64 core type differs (pointer, long, size_t, ssize_t,
# ptrdiff_t, intptr_t, off_t, time_t, int) or the pthread test fails. Other
# differences are reported as known differences.
#
# Environment: CC (default gcc), EMSDK_IMAGE (see scripts/emcc.sh),
# NODE (host node used for the comparison and the optional host run).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo="$(cd "$here/../.." && pwd -P)"
out="$here/out"
rel="native/spike"          # path of this directory inside the container (/src/...)
emcc_sh="$repo/scripts/emcc.sh"
CC="${CC:-gcc}"
NODE="${NODE:-node}"

rm -rf "$out"
mkdir -p "$out"

step() { printf '\n== %s\n' "$*" >&2; }

# ---------------------------------------------------------------- native
step "native build ($CC)"
"$CC" --version | head -1 >&2
native_flags=(-std=gnu11 -O1 -Wall -Wextra -Werror -pthread)
"$CC" "${native_flags[@]}" "$here/sizes.c" -o "$out/sizes-native"
"$CC" "${native_flags[@]}" "$here/posix.c" -o "$out/posix-native"
"$out/sizes-native" > "$out/sizes-x86_64-linux.json"
"$out/posix-native" > "$out/posix-x86_64-linux.json"
"$here/probe-features.sh" "$out/features-native" "$CC" -std=gnu11 -O1 -pthread > "$out/features-x86_64-linux.json"

# ---------------------------------------------------------------- wasm64
# -sMEMORY64=1 is what the plan names; Emscripten 6 calls it deprecated in
# favour of -m64 (same code), hence -Wno-deprecated.
em_flags=(-std=gnu11 -O1 -Wall -Wextra -Werror -Wno-deprecated
          -sMEMORY64=1 -pthread -sPTHREAD_POOL_SIZE=4 -sEXIT_RUNTIME=1
          -sENVIRONMENT=node,worker)
step "wasm64 build (scripts/emcc.sh, ${EMSDK_IMAGE:-emscripten/emsdk:6.0.12})"
cd "$repo"
"$emcc_sh" --version | head -1 >&2
# .cjs: the repository's package.json sets "type": "module", and Emscripten's
# Node output is CommonJS.
"$emcc_sh" "${em_flags[@]}" "$rel/sizes.c" -o "$rel/out/sizes-wasm64.cjs"
"$emcc_sh" "${em_flags[@]}" -Wno-pthreads-mem-growth -sALLOW_MEMORY_GROWTH=1 -sMAXIMUM_MEMORY=16GB \
  "$rel/posix.c" -o "$rel/out/posix-wasm64.cjs"

step "wasm64 run (image node)"
"$emcc_sh" node --version >&2
"$emcc_sh" node "$rel/out/sizes-wasm64.cjs" > "$out/sizes-wasm64-emscripten.json"
"$emcc_sh" node "$rel/out/posix-wasm64.cjs" > "$out/posix-wasm64-emscripten.json"
"$emcc_sh" bash "$rel/probe-features.sh" "$rel/out/features-wasm64" \
  emcc -std=gnu11 -O1 -Wno-deprecated -sMEMORY64=1 -pthread > "$out/features-wasm64-emscripten.json"

# Host Node is optional: Node 22 (V8 12.4) implements a pre-standard memory64
# JS API and table64 encoding and cannot load Emscripten 6 output; Node 24+ can.
step "wasm64 run (host node, optional)"
if command -v "$NODE" >/dev/null 2>&1; then
  hv="$("$NODE" --version)"
  if "$NODE" "$out/sizes-wasm64.cjs" > "$out/sizes-wasm64-host-node.json" 2> "$out/host-node.err"; then
    echo "$hv" > "$out/host-node.status"
  else
    rm -f "$out/sizes-wasm64-host-node.json"
    err="$(grep -m1 -E 'Error|error' "$out/host-node.err" || head -1 "$out/host-node.err")"
    echo "$hv failed: ${err}" > "$out/host-node.status"
  fi
  cat "$out/host-node.status" >&2
else
  echo "no host node" > "$out/host-node.status"
fi

# ---------------------------------------------------------------- compare
step "compare"
if command -v "$NODE" >/dev/null 2>&1; then
  "$NODE" "$here/compare.mjs" "$out"
else
  "$emcc_sh" node "$rel/compare.mjs" "$rel/out"
fi
