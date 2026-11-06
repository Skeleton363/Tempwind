---
name: milestone-close
description: Close a milestone - run the full verification, review the diff in two stages, update the status note, open the draft PR, then a retro. Use before a milestone's pull request.
---
# Close a milestone

Never merge. The user merges.

## 1. Verify (record the commands and results)
- `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- `cargo clippy -p x64wasm-fp --target wasm32-unknown-unknown -- -D warnings`
- `cargo run -p testkit --release -- record --check`
- `cargo run -p testkit --release -- census FILE... --check` for each target in `target/targets/`
- `cargo run -p testkit --release -- spike --out target/spike`, then `tests/pe/prepare.sh` and `tests/native/prepare.sh`
- `npm run test:node` and `npm run test:browser`
- For M5 and later: the 7-Zip matrix in `translated` mode and `compare.mjs` against `tests/sevenzip/expected`.
Save long logs in the scratchpad; keep the numbers. Fix failures at the root cause; never weaken or skip a test.

## 2. Two-stage review of the milestone diff (`git diff main...HEAD`)
1. Spec compliance: a fresh agent reads the diff against `docs/design/*.md` and `docs/plan.md` and lists deviations, missing pieces and extras, and tests that pin numbers no contract fixes. Read-only; no style comments.
2. Correctness: run the built-in `/code-review medium`. Fix correctness findings; skip style.
Re-run step 1's affected tests after fixes.

## 3. Status note
Update `docs/status/phase-1.md`: header, decisions, what works, test summary, measurements, known issues, next step.

## 4. Draft PR
- Commit by pathspec, `git push`.
- Open a draft PR titled `[Phase 1] <milestone>` with a summary, the test summary and the measurements. No `Co-Authored-By`, `Claude-Session` or "Generated with" lines.
- The description's first line is the merge danger: whether a revert alone undoes the merge (a two-way door) or not (one-way: published releases, recorded data, a contract others build on), and what breaks for whom if it is wrong. Trial 4 in `docs/dev/agent-runs.md`.
- `subscribe_pr_activity` for the PR; keep one safety-net check-in (`send_later`) and wait for PR events instead of polling.

## 5. Retro (once CI is green)
1. List the milestone's avoidable costs, worst first:
   - red CI runs on the PR branch (`actions_list`), with their causes;
   - rework rounds and findings (`node scripts/agent-stats.mjs --milestone Mn`);
   - escalations.
2. Give each one a fix where it would have been caught first. A mechanical mistake gets an automated check (`tools/git-hooks/pre-push`, a CI step, a test). A judgement call gets a rule where its writer or reviewer reads it: an agent's fixed rules, or this skill's review brief.
3. Decide the trials that are due in `docs/dev/agent-runs.md` (kept, reshaped or reverted, with the numbers). Record the counts and the changes there.
4. Commit the retro by pathspec and push. It touches docs and workflow files only.
