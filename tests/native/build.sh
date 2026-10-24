#!/usr/bin/env bash
# Build target/native/{hello,l2_int,l2_crt}.exe with MinGW-w64 (x86-64) and write their sha256.
set -euo pipefail
cd "$(dirname "$0")/../.."
out=target/native
mkdir -p "$out"
x86_64-w64-mingw32-gcc -O2 -static -Wall -Wextra -o "$out/hello.exe" tests/native/hello.c
# The L2 programs: no timestamp in the PE header, so the builds are reproducible.
for name in l2_int l2_crt; do
  x86_64-w64-mingw32-gcc -O2 -static -Wall -Wextra -Wl,--no-insert-timestamp \
    -o "$out/$name.exe" "tests/native/$name.c"
done
(cd "$out" && sha256sum hello.exe l2_int.exe l2_crt.exe > exes.sha256 && sha256sum hello.exe > hello.exe.sha256)
echo "built $out/{hello,l2_int,l2_crt}.exe"
cat "$out/exes.sha256"
