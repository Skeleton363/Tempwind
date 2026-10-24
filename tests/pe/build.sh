#!/usr/bin/env bash
# Build target/pe/basic.exe with MinGW-w64 (x86-64) and write its sha256.
set -euo pipefail
cd "$(dirname "$0")/../.."
out=target/pe
mkdir -p "$out"
x86_64-w64-mingw32-gcc -nostdlib -Wl,-e,start -Wl,--subsystem,console \
  -Wl,--dynamicbase -Wl,--no-insert-timestamp -Wl,-T,tests/pe/basic.ld \
  -o "$out/basic.exe" tests/pe/basic.S -lkernel32
(cd "$out" && sha256sum basic.exe > basic.exe.sha256)
echo "built $out/basic.exe"
cat "$out/basic.exe.sha256"
