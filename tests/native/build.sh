#!/usr/bin/env bash
# Build target/native/hello.exe with MinGW-w64 (x86-64) and write its sha256.
set -euo pipefail
cd "$(dirname "$0")/../.."
out=target/native
mkdir -p "$out"
x86_64-w64-mingw32-gcc -O2 -static -Wall -Wextra -o "$out/hello.exe" tests/native/hello.c
(cd "$out" && sha256sum hello.exe > hello.exe.sha256)
echo "built $out/hello.exe"
cat "$out/hello.exe.sha256"
