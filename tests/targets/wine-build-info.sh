#!/usr/bin/env bash
# Write build-info.json of the Wine build (docs/design/wine-build.md): the
# tarball's pins, toolchain package versions, configure flags and the run URL.
#   WINE_CONFIGURE_FLAGS="…" tests/targets/wine-build-info.sh OUT.json
# Run in the environment of .github/workflows/wine.yml's build job.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
out=${1:?usage: wine-build-info.sh OUT.json}
cc=x86_64-w64-mingw32
pkg() { dpkg-query -W -f='${Version}' "$1" 2>/dev/null || echo "not installed"; }

jq -n \
  --arg commit "${SHA:-${GITHUB_SHA:-$(git -C "$root" rev-parse HEAD)}}" \
  --arg run_url "${RUN_URL:-}" \
  --arg flags "${WINE_CONFIGURE_FLAGS:?set WINE_CONFIGURE_FLAGS}" \
  --arg gcc "$(gcc --version | head -1)" \
  --arg mingw_gcc "$("$cc-gcc" --version | head -1)" \
  --arg mingw_ld "$("$cc-ld" --version | head -1)" \
  --arg bison "$(bison --version | head -1)" \
  --arg flex "$(flex --version | head -1)" \
  --arg ccache "$(ccache --version | head -1)" \
  --arg p_gcc "$(pkg gcc)" \
  --arg p_mingw_gcc "$(pkg gcc-mingw-w64-x86-64-win32)" \
  --arg p_mingw_bin "$(pkg binutils-mingw-w64-x86-64)" \
  --arg p_mingw_dev "$(pkg mingw-w64-x86-64-dev)" \
  --arg p_mingw_common "$(pkg mingw-w64-common)" \
  --arg p_bison "$(pkg bison)" \
  --arg p_flex "$(pkg flex)" \
  --slurpfile sources "$root/tests/targets/sources.json" \
  '{
     commit: $commit,
     run_url: $run_url,
     source: {version: $sources[0].wine.version, url: $sources[0].wine.url,
              sha256: $sources[0].wine.sha256, sha512: $sources[0].wine.sha512},
     compilers: {gcc: $gcc, mingw_gcc: $mingw_gcc, mingw_ld: $mingw_ld, bison: $bison, flex: $flex, ccache: $ccache},
     packages: {
       gcc: $p_gcc,
       "gcc-mingw-w64-x86-64-win32": $p_mingw_gcc,
       "binutils-mingw-w64-x86-64": $p_mingw_bin,
       "mingw-w64-x86-64-dev": $p_mingw_dev,
       "mingw-w64-common": $p_mingw_common,
       bison: $p_bison,
       flex: $p_flex
     },
     configure_flags: $flags,
     install: "make install DESTDIR=… prefix=/opt/wine-11.0",
     pe_strip: "x86_64-w64-mingw32-strip --strip-debug (COFF symbols kept) on the PE modules and test executables of wine-pe-x86_64.tar.zst"
   }' > "$out"
echo "wrote $out"
