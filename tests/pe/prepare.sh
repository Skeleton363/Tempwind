#!/usr/bin/env bash
# Prepare the inputs of the Node tests for target/pe/basic.exe (run build.sh first):
# AOT translations from all entries and from the entry point only, and the
# run-time translator module.
set -euo pipefail
cd "$(dirname "$0")/../.."
exe=target/pe/basic.exe
[ -f "$exe" ] || { echo "missing $exe; run tests/pe/build.sh" >&2; exit 1; }
cargo run -q -p x64wasm-cli --release -- translate-pe "$exe" --out-dir target/pe/all --entries all
cargo run -q -p x64wasm-cli --release -- translate-pe "$exe" --out-dir target/pe/entry --entries entry
cargo build -q -p x64wasm-rt --release --target wasm32-unknown-unknown
