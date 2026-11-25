#!/usr/bin/env bash
# Prepare the inputs of the Node tests (the programs themselves come from build.sh):
#  - coremark.exe in target/targets/ (fetched from the targets release unless already there);
#  - the floating-point helper module x64wasm_fp.wasm and its test vectors (target/fp/vectors.json).
set -euo pipefail
cd "$(dirname "$0")/../.."
if [ ! -f target/targets/coremark.exe ]; then
  scripts/fetch-artifact.sh "$(cat tests/targets/targets.lock)" coremark.exe target/targets
fi
cargo build -q -p x64wasm-fp --release --target wasm32-unknown-unknown
mkdir -p target/fp
cargo run -q -p x64wasm-fp --release --example vectors -- --out target/fp/vectors.json
