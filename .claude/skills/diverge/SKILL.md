---
name: diverge
description: Find where a translated run departs from native - first differing API call, return value or output byte - and fix the root cause. Use when a translated program's output or exit code differs from native.
---
# Find the divergence

Gates (trial 5 in `docs/dev/agent-runs.md`): one command that goes red, a minimal input, written hypotheses, one change at a time, tagged debug output.

1. Run both with identical input and switches.
   - Native: `wine64` (`/usr/lib/wine/wine64`) or the committed expected files.
   - Translated: `node runtime/src/run-pe.mjs --exe FILE.exe --aot FILE.wasm --plan FILE.plan.json` (`--dir` for files).
2. Trace the translated run: `TEMPWIND_TRACE=api,translate,exits`, stderr to a scratchpad file. Never read the log whole; grep it.
3. Find the first difference: API call, return value or output byte. For 7-Zip, `node tests/sevenzip/compare.mjs OUT tests/sevenzip/expected` names the first byte.
   - Gate: save one command that goes red on this difference (`repro.sh` in the scratchpad) and re-run it after every change.
   - Shrink the input (fewer files, smaller switches, a shorter run) while it stays red.
4. Narrow it to the guest function (the exit and translate lines give addresses), then to the instruction. Translate that function with `x64wasm-cli ir|wat` to read the lowering.
   - Before each narrowing step, write up to 3 hypotheses ranked by likelihood. Give each the observation that would rule it out, and test the cheapest first.
   - Change one thing at a time. Temporary debug output carries a `[dbg]` tag; `grep -rn '\[dbg\]'` must be empty before a commit.
5. Reproduce it as an L1 case (via `testkit record`, never by hand) or a failing Node or Rust test, before any fix.
6. Fix the root cause in the translator, shim or runtime. No special cases for the test.
7. Run the failing test, then the area's suite.

Report a translator bug with the instruction bytes, the register and flag state before it, the expected and actual results.
If the shim lacks an API, implement it strictly (see `.claude/rules/runtime.md`); never stub a success.
