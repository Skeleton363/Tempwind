#!/usr/bin/env bash
# Prepare the inputs of the Node tests: the run-time translator module (x64wasm_rt.wasm).
# target/pe/basic.exe (build.sh) is an input of tests/wine/prepare.sh.
set -euo pipefail
cd "$(dirname "$0")/../.."
cargo build -q -p x64wasm-rt --release --target wasm32-unknown-unknown
