#!/usr/bin/env bash
# Write build-info.json: toolchain versions, flags and source pins of the
# Phase 1 targets build.
#   tests/targets/build-info.sh OUT.json
# Run in the same environment (and after) tests/targets/{7zip,coremark}/build.sh;
# reads target/targets/7zz.make-args written by the 7-Zip build.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
out=${1:?usage: build-info.sh OUT.json}
cc=x86_64-w64-mingw32
pkg() { dpkg-query -W -f='${Version}' "$1" 2>/dev/null || echo "not installed"; }

jq -n \
  --arg commit "${GITHUB_SHA:-$(git -C "$root" rev-parse HEAD)}" \
  --arg run_url "${RUN_URL:-}" \
  --arg gcc "$("$cc-gcc" --version | head -1)" \
  --arg gxx "$("$cc-g++" --version | head -1)" \
  --arg threads "$("$cc-gcc" -v 2>&1 | sed -n 's/^Thread model: //p')" \
  --arg ld "$("$cc-ld" --version | head -1)" \
  --arg windres "$("$cc-windres" --version | head -1)" \
  --arg p_gcc "$(pkg gcc-mingw-w64-x86-64-win32)" \
  --arg p_gxx "$(pkg g++-mingw-w64-x86-64-win32)" \
  --arg p_bin "$(pkg binutils-mingw-w64-x86-64)" \
  --arg p_dev "$(pkg mingw-w64-x86-64-dev)" \
  --arg p_common "$(pkg mingw-w64-common)" \
  --arg z7_flags "$(tr '\n' ' ' < "$root/target/targets/7zz.make-args")" \
  --slurpfile sources "$root/tests/targets/sources.json" \
  '{
     commit: $commit,
     run_url: $run_url,
     compilers: {gcc: $gcc, gxx: $gxx, thread_model: $threads, ld: $ld, windres: $windres},
     packages: {
       "gcc-mingw-w64-x86-64-win32": $p_gcc,
       "g++-mingw-w64-x86-64-win32": $p_gxx,
       "binutils-mingw-w64-x86-64": $p_bin,
       "mingw-w64-x86-64-dev": $p_dev,
       "mingw-w64-common": $p_common
     },
     flags: {
       "7zz.exe": ("make -f CPP/7zip/cmpl_gcc.mak " + $z7_flags + "(upstream -O2 -DNDEBUG -D_REENTRANT -D_FILE_OFFSET_BITS=64 -fPIC; no assembler)"),
       "coremark.exe": "-O2 -static, tests/targets/coremark/port, SEED_ARG, MEM_STATIC"
     },
     sources: {
       "7zip": {version: $sources[0]["7zip"].version, url: $sources[0]["7zip"].url, sha256: $sources[0]["7zip"].sha256},
       coremark: {repo: $sources[0].coremark.repo, commit: $sources[0].coremark.commit, sha256: $sources[0].coremark.sha256}
     }
   }' > "$out"
echo "wrote $out"
