---
paths:
  - "crates/testkit/**"
---
# Instruction oracle rules

- Fixtures in `crates/testkit/fixtures/` are written only by `cargo run -p testkit --release -- record`. Never edit them by hand (a deny rule enforces it).
- `record --check` must reproduce on any CPU vendor. Put implementation-dependent results (bits the SDM leaves undefined or open) under `undefined`, recorded as the input's values; or mask or vendor-gate them before recording.
- The format is `docs/design/fixtures.md`. Change the doc before the format.
- Scope of forms: `docs/design/instruction-scope.md`. `census --check` must stay green.
- Replay time is wasm compile time (about 95 %); don't add per-case work that scales with case count.
- Never read fixtures or `census.json` whole; use `replay` summaries, Grep, or ranges.
