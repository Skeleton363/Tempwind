---
paths:
  - "crates/x64wasm/**"
  - "crates/x64wasm-rt/**"
  - "crates/x64wasm-cli/**"
  - "crates/x64wasm-fp/**"
---
# Translator rules

- `docs/design/abi.md` and `docs/design/translator.md` are the contract. Change a contract in the doc first, in its own commit, before the code.
- Every decoder uses `x64wasm::DECODER_OPTIONS` (`F3 0F BC/BD` decode as `bsf`/`bsr`, as the profile's CPU runs them). The census uses the same decoding.
- `FaultIf` goes before the first side effect of an instruction, so a faulting instruction leaves the state block unchanged.
- Fixtures first: a new instruction form gets L1 fixtures (recorded by `testkit record`) before or with its lowering. A form without fixtures is not done. Scope: `docs/design/instruction-scope.md`.
- Regenerate IR/WAT snapshots with `UPDATE_SNAPSHOTS=1 cargo test -p x64wasm --test snapshots`, then review the diff.
- The soft float in `x64wasm-fp` is integer-only (no host float operations) and checked bit for bit against the CPU with the inline-asm differential tests. Build it for `wasm32-unknown-unknown`.
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`; `cargo clippy -p x64wasm-fp --target wasm32-unknown-unknown -- -D warnings`.
- Run `rustfmt` on your own files only, never `cargo fmt --all`, while another agent works.
- Compare timings at several guest base addresses before calling something a regression (code placement changes wasmtime speed by up to 1.6x).
