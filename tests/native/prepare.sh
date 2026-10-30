#!/usr/bin/env bash
# Prepare the inputs of the Node tests for the M3/M4 programs (run build.sh first):
#  - AOT translation from all entries of hello, l2_int, l2_crt, l2_float into target/native/<name>/;
#  - coremark.exe in target/targets/ (fetched from the targets release unless already there),
#    translated into target/targets/coremark/;
#  - the floating-point helper module x64wasm_fp.wasm and its test vectors (target/fp/vectors.json).
set -euo pipefail
cd "$(dirname "$0")/../.."
for name in hello l2_int l2_crt l2_float; do
  exe=target/native/$name.exe
  [ -f "$exe" ] || { echo "missing $exe; run tests/native/build.sh" >&2; exit 1; }
  cargo run -q -p x64wasm-cli --release -- translate-pe "$exe" --out-dir "target/native/$name" --entries all
done
if [ ! -f target/targets/coremark.exe ]; then
  scripts/fetch-artifact.sh "$(cat tests/targets/targets.lock)" coremark.exe target/targets
fi
cargo run -q -p x64wasm-cli --release -- translate-pe target/targets/coremark.exe --out-dir target/targets/coremark --entries all
cargo build -q -p x64wasm-fp --release --target wasm32-unknown-unknown
mkdir -p target/fp
cargo run -q -p x64wasm-fp --release --example vectors -- --out target/fp/vectors.json
