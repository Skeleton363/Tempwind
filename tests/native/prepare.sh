#!/usr/bin/env bash
# Prepare the inputs of the L2 Node tests (run build.sh first): AOT translation
# from all entries of each L2 program into target/native/<name>/.
set -euo pipefail
cd "$(dirname "$0")/../.."
for name in hello l2_int l2_crt l2_float; do
  exe=target/native/$name.exe
  [ -f "$exe" ] || { echo "missing $exe; run tests/native/build.sh" >&2; exit 1; }
  cargo run -q -p x64wasm-cli --release -- translate-pe "$exe" --out-dir "target/native/$name" --entries all
done
