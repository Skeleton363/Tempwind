#!/usr/bin/env bash
# Prepare the inputs of `npm run test:wine` (docs/design/wine-unix.md, section 10).
#
#   tests/wine/prepare.sh [--fetch]
#
# 1. Fetches the assets of tests/targets/wine.lock into target/wine: the PE
#    tree, the prefix and the native tree (with share/wine), unpacked there.
# 2. Builds target/wine/hello.exe (tests/native/hello.c) and
#    target/wine/kuser-dump.exe (tests/wine/kuser-dump.c) with MinGW-w64.
# 3. Runs kuser-dump.exe under the native Wine 11 with the prefix and TZ=UTC
#    to write target/wine/kuser.bin.
# 4. Builds the Unix side into target/wine-unix with native/wine/build.sh, or,
#    with --fetch, downloads the files of the release tests/targets/wine-unix.lock
#    names (the tag on one line, like wine.lock).
#
# Needs curl, zstd, tar, x86_64-w64-mingw32-gcc; Docker for build.sh.
set -euo pipefail
cd "$(dirname "$0")/../.."

fetch=0
case "${1:-}" in
  "") ;;
  --fetch) fetch=1 ;;
  *) echo "usage: $0 [--fetch]" >&2; exit 2 ;;
esac

out=target/wine
mkdir -p "$out"
out=$(cd "$out" && pwd -P)
tag=$(cat tests/targets/wine.lock)

# 1. The Wine release assets, unpacked once per tag.
if [ "$(cat "$out/.tag" 2>/dev/null || true)" != "$tag" ]; then
  rm -rf "$out/native" "$out/pe" "$out/wine-prefix"
  for asset in wine-pe-x86_64.tar.zst wine-native.tar.zst wine-prefix.tar.zst; do
    scripts/fetch-artifact.sh "$tag" "$asset" "$out"
  done
  mkdir -p "$out/pe" "$out/native"
  tar -C "$out/pe" -I zstd -xf "$out/wine-pe-x86_64.tar.zst"
  tar -C "$out/native" -I zstd -xf "$out/wine-native.tar.zst"
  # Without --no-same-owner Wine refuses a prefix that isn't owned by you.
  tar -C "$out" --no-same-owner -I zstd -xf "$out/wine-prefix.tar.zst"
  echo "$tag" > "$out/.tag"
fi
wine_bin="$out/native/opt/wine-11.0/bin"
[ -x "$wine_bin/wine" ] || { echo "prepare.sh: $wine_bin/wine missing after unpacking $tag" >&2; exit 1; }

# 2. The Windows programs.
x86_64-w64-mingw32-gcc -O2 -static -Wall -Wextra -o "$out/hello.exe" tests/native/hello.c
x86_64-w64-mingw32-gcc -O2 -static -Wall -Wextra -o "$out/kuser-dump.exe" tests/wine/kuser-dump.c

# 3. KUSER_SHARED_DATA as native Wine 11 lays it out.
rm -f "$out/kuser.bin"
rc=0
WINEPREFIX="$out/wine-prefix" WINEDEBUG=-all TZ=UTC \
  "$wine_bin/wine" "$out/kuser-dump.exe" "$out/kuser.bin" || rc=$?
"$wine_bin/wineserver" -k 2>/dev/null || true
if [ "$rc" != 0 ] || [ "$(stat -c %s "$out/kuser.bin" 2>/dev/null || echo 0)" != 4096 ]; then
  echo "prepare.sh: kuser-dump.exe under native Wine failed (exit $rc) or wrote no 4096-byte kuser.bin" >&2
  exit 1
fi
echo "wrote $out/kuser.bin"

# 4. The Unix side.
if [ "$fetch" = 1 ]; then
  utag=$(cat tests/targets/wine-unix.lock)
  [ -n "$utag" ] || { echo "prepare.sh: tests/targets/wine-unix.lock is empty; build without --fetch" >&2; exit 1; }
  rm -rf target/wine-unix
  mkdir -p target/wine-unix
  files="wine-unix.mjs wine-unix.wasm build-info.json"
  for c in native/wine/tests/*.c; do
    n=$(basename "$c" .c)
    files="$files test-$n.mjs test-$n.wasm"
  done
  for f in $files; do
    scripts/fetch-artifact.sh "$utag" "$f" target/wine-unix >/dev/null
  done
else
  native/wine/build.sh
fi
echo "ready: $out, target/wine-unix"
