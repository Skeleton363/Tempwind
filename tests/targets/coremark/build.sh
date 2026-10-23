#!/usr/bin/env bash
# Build coremark.exe (CoreMark with the Win64 port in port/) with MinGW-w64
# cross-compiling from Linux.
#   tests/targets/coremark/build.sh SOURCE.tar.gz [OUTDIR]
# SOURCE is the mirrored src-coremark-<sha7> asset (sha256 checked against
# tests/targets/sources.json); OUTDIR defaults to target/targets.
#
# The five upstream core files are compiled together with port/core_portme.c;
# the upstream Makefile is not used (its default target would run the binary).
# Single-threaded; -static keeps the exe free of MinGW runtime DLLs;
# --no-insert-timestamp zeroes the PE timestamp so rebuilds are comparable.
# Seeds and iterations come from the command line, for example
#   coremark.exe 0x0 0x0 0x66 2000
set -euo pipefail

root=$(cd "$(dirname "$0")/../../.." && pwd)
[ $# -ge 1 ] && [ $# -le 2 ] || { echo "usage: $0 SOURCE.tar.gz [OUTDIR]" >&2; exit 2; }
tarball=$(realpath "$1")
out=${2:-$root/target/targets}
mkdir -p "$out"
out=$(realpath "$out")

want=$(jq -r '.coremark.sha256' "$root/tests/targets/sources.json")
have=$(sha256sum "$tarball" | awk '{print $1}')
if [ "$want" != "$have" ]; then
  echo "coremark/build.sh: sha256 of $tarball is $have, sources.json pins $want" >&2
  exit 1
fi

work="$root/target/targets/coremark-build"
rm -rf "$work"
mkdir -p "$work"
tar -C "$work" -xzf "$tarball"
src=$(echo "$work"/coremark-*)

CC=${CC:-x86_64-w64-mingw32-gcc}
flags="-O2"
"$CC" $flags -Wall -Wno-unused-value \
  "-DFLAGS_STR=\"$flags -static\"" \
  -I"$root/tests/targets/coremark/port" -I"$src" \
  "$src"/core_list_join.c "$src"/core_main.c "$src"/core_matrix.c \
  "$src"/core_state.c "$src"/core_util.c \
  "$root/tests/targets/coremark/port/core_portme.c" \
  -static -Wl,--no-insert-timestamp -o "$out/coremark.exe"
echo "built $out/coremark.exe"
sha256sum "$out/coremark.exe"
