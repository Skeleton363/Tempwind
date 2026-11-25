#!/usr/bin/env bash
# Build target/native/{hello,l2_int,l2_crt,l2_float,l2_threads,l2_suspend}.exe with MinGW-w64 (x86-64) and write their sha256.
set -euo pipefail
cd "$(dirname "$0")/../.."
out=target/native
mkdir -p "$out"
# No timestamp in the PE header, so the builds are reproducible: load plans and
# the AOT catalog are keyed by sha256, and tests/wine/prepare.sh rebuilds.
x86_64-w64-mingw32-gcc -O2 -static -Wall -Wextra -Wl,--no-insert-timestamp \
  -o "$out/hello.exe" tests/native/hello.c
for name in l2_int l2_crt l2_threads l2_suspend; do
  x86_64-w64-mingw32-gcc -O2 -static -Wall -Wextra -Wl,--no-insert-timestamp \
    -o "$out/$name.exe" "tests/native/$name.c"
done
# l2_float: -fno-math-errno so sqrt inlines, -ftree-vectorize for packed SSE; no msvcrt math imports.
x86_64-w64-mingw32-gcc -O2 -ftree-vectorize -fno-math-errno -static -Wall -Wextra -Wl,--no-insert-timestamp \
  -o "$out/l2_float.exe" tests/native/l2_float.c
(cd "$out" && sha256sum hello.exe l2_int.exe l2_crt.exe l2_float.exe l2_threads.exe l2_suspend.exe > exes.sha256 && sha256sum hello.exe > hello.exe.sha256)
echo "built $out/{hello,l2_int,l2_crt,l2_float,l2_threads,l2_suspend}.exe"
cat "$out/exes.sha256"
