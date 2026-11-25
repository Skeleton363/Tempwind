#!/usr/bin/env bash
# Prepare the inputs of the Node tests: the floating-point helper module x64wasm_fp.wasm and its test vectors
# (target/fp/vectors.json). The programs come from build.sh; tests/wine/prepare.sh fetches coremark.exe.
set -euo pipefail
cd "$(dirname "$0")/../.."
cargo build -q -p x64wasm-fp --release --target wasm32-unknown-unknown
mkdir -p target/fp
cargo run -q -p x64wasm-fp --release --example vectors -- --out target/fp/vectors.json
