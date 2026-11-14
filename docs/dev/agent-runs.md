# Agent runs: cost and quality ledger

We judge workflow decisions (effort levels, caps, agent rules) by data, at almost no token cost: the numbers come from files the harness already writes.

## What is recorded

**`agent-runs.jsonl`** gets one line per agent run, a run's resumes included. The coordinator writes it when the run finishes, with `node scripts/agent-usage.mjs --agent ID --json`. The script sums the usage of every request in the run's transcript (`~/.claude/projects/<project>/<session>/subagents/`). Transcripts live only as long as the container, so the line is written at once.
- **Usage:**
  - `requests`, `input`, `cache_write`, `cache_read` and `output`;
  - `index`: the cost in the model's input-token equivalents (cache writes 1.25, cache reads 0.1, output 5). Compare it within a model.
- **The run's setup:** date, milestone, task, agent, model (the one that served the requests) and effort.
- **Quality:**
  - `rework`: follow-up messages needed before the result passed the coordinator's verification (0 means right the first time);
  - `findings`: valid review findings in the run's own diff, filled in at milestone close-out. A finding counts for the run whose commit introduced the line (`git log -S`). A defect in a contract doc, or in the coordinator's own change, counts for no run; neither do notes.
- **`kind`** (optional): `"diverge"` for a bug hunt run through `/diverge` (trial 5).

**Not a cost measure:** the token count in a completion notice. It is the context size of the run's last request. It fell from 172,549 to 158,912 between two notices of one run, and WP4's 161,269 hid 44.2M cache-read tokens. Cache reads, the context re-read at every request, are most of a run's cost. Until 2026-11-06 the ledger recorded the notice's count; on that day every line was rebuilt from the transcripts (`source: transcript`), keeping the quality fields and the efforts that were recorded.

**The main session** gets a `main` line at each milestone's start and end, from `get_session`: the cumulative `cost_usd`, `output_tokens` and `cache_read_tokens`. The difference between two lines is that stretch's usage.

**Effort** is `unrecorded` before 2026-10-31, when the agent definitions got their efforts.

## Reading it

`node scripts/agent-stats.mjs [--milestone M6]` prints, per agent, model and effort, the runs, the median index and requests, the index per request, rework and findings, then the main session's usage between snapshots.

## How we judge a change
- **Every workflow change is a trial.** It records:
  - the problem we saw, with its evidence;
  - the change;
  - the measure;
  - when it is decided, normally the next milestone close, in its retro;
  - the result: kept, reshaped or reverted.
- **An outside recommendation is a lead** (docs, a skill library, the digest). It becomes a trial only when it addresses a problem we have.
- **The quality guard:** a change that saves tokens but raises rework, review findings, CI failures or escalations is reverted.

## Trials (started 2026-11-06, from the review of mattpocock/skills, `research.md`)

| Change | Applied as | Baseline | Measure | Decided at |
|---|---|---|---|---|
| 1. Prune the always-loaded files: pointers over content, nothing a hook or deny rule already enforces | `CLAUDE.md`: commands behind `commands.md`, deny-rule duplicates out, the stale layout fixed (`x64wasm-fp` was missing). Agent files: duplicate lines out. | `CLAUDE.md` 965 words, builder 297, deep-worker 299. Now 912, 347 and 344, after trial 2's two rules. A builder run loads 1,259 words, against 1,262 before. | The token effect is at most about 1 % of a run, too small for the ledger, so this is judged on quality. No rework from a command that moved behind a pointer. | M6 close |
| 2. A retro as `milestone-close` step 5: a mechanical mistake gets an automated check, a judgement call a rule where its writer or reviewer reads it | The first retro, from M6's three failures, applied now:<br>• `tools/git-hooks/pre-push`, installed by the SessionStart hook, refuses a WIP head and unformatted Rust on `main` and `claude/*`. Tested on `8b6ef85` and `4c02685`: both refused.<br>• The WIP-to-side-branch rule is in the agents' fixed rules and `CLAUDE.md`.<br>• The test-contract rule is in the fixed rules and the spec-review brief. | M6: 2 avoidable red CI runs (the WIP head `8b6ef85`, the fmt break `4c02685`) and 1 rework (the `% 20` assertion) | Avoidable red CI runs and rework rounds per milestone | Kept at Phase 2 M2's close (retro below) |
| 3. A hand-off in `/pause`: the resume path works for a fresh session, and the report ends with a starter message | `/pause` steps 5 and 7 | Main session: 818M cache-read tokens over M1-M5 in one session | `main` lines of a milestone that starts in a fresh session (M2 of Phase 2, from M1's hand-off) against Phase 1's per-milestone stretches | Kept at Phase 2 M2's close (retro below) |
| 4. A merge-danger line opening each PR description: one-way or two-way door, and the blast radius | `milestone-close` step 4 | none (no merge has gone wrong) | The owner's verdict at the next merge: useful or noise | Reverted at PR #12's merge (2026-11-14): the owner's verdict was noise |
| 5. Gates in `/diverge`: one red command, a minimal input, ranked falsifiable hypotheses, one change at a time, `[dbg]`-tagged debug output | `/diverge` steps 3 and 4. Ledger lines of bug hunts get `"kind": "diverge"`. | none (no recent bug hunt went wrong) | Rework and escalations on bug-hunt runs | After three bug hunts |

Trials 4 and 5 are cheap; if they show nothing, they are removed.

## Trial 6 (started 2026-11-13, the owner's change)

| Change | Applied as | Baseline | Measure | Decided at |
|---|---|---|---|---|
| 6. Context checkpoints: the coordinator reads `context_usage` from `get_session` at each checkpoint (plan approved, contracts pushed, each work package merged, before `/milestone-close`, after its reviews). Above 25 % (35 % until M2's close), and always between planning and execution, it asks the owner to run `/compact` with a ready focus line, once the state is in files (the plan, the contracts, the status note, the agents' tracks). Agents read the sources; the coordinator reads diffs and the lines it cites. | One line in `CLAUDE.md`'s "Working efficiently" | M2's planning stretch: 483,687 of 1,000,000 tokens of context before any work, $15.42, 34.2M cache reads, 256k output (`main` line of 2026-11-13). Auto-compaction would have started at 901,600. Compacted at the approval to 98,077. | `context_usage` at each checkpoint; the number of compactions; M2's cache reads per coordinator request, after against before; rework traced to lost context, which must stay 0 | Reshaped at M2's close (retro below); decided at M3's close |

**Problem:** every request re-reads the whole context, so a session that fills half its window while planning pays that half again at every later step. **Lead for M3, not adopted yet:** delegate the research pass's source reading to a subagent, so the coordinator's context holds findings, not sources.

## Trial 7 (started 2026-11-14, the owner's change)

| Change | Applied as | Baseline | Measure | Decided at |
|---|---|---|---|---|
| 7. A branch list at each close: the report names the GitHub branches that can be deleted, each with its evidence, and gives the owner one delete command | `milestone-close` step 6, item 4 | At M2's close, 25 branches besides `main` and the PR branch:<br>• the work tracks of M1 (6), M6 (7) and M2 (7);<br>• 4 old session branches and `bench-data`.<br><br>None had been deleted, and a session's delete gets HTTP 403. 23 could go. `bench-data` and Phase 1's branch, which has its own commits, stay. | Branches left after the owner's cleanup at M3's close, other than `main`, `bench-data` and open PRs: 0 | M3's close |

**Problem:** sessions push a work track per agent and can't delete branches, so every milestone left its tracks behind.

## Trials (started 2026-11-05, decided at the M6 close-out)

| Change | Where | Measure | Kept if |
|---|---|---|---|
| At most 3 subagents at once, none spawning its own | `.claude/settings.json` `env` | "limit reached" refusals | it never blocks planned work |
| Subagents that name no effort (Plan, general-purpose reviewers) run at `high`, not the session's `max` | `CLAUDE_CODE_SUBAGENT_EFFORT` | `index` per review or plan, against the baseline at `max` (Plan 2.06M, the M5 spec review 2.57M) | tokens fall, and the close-out reviews still find the real defects (none escapes to CI or later) |
| deep-worker at `medium` for M6's WP3a and WP4; `high` for WP2 and WP3b | the Agent tool's `effort` per spawn | `index` per run and per request, rework, findings | `medium` costs at least 25 % less (`index`) with no more rework or findings; it then becomes the default for Phase 2 |
| Read a file once; wait on backgrounded commands | builder and deep-worker fixed rules | `index` per request, against the baseline | no measured harm |

**Decision rule:** samples are small, so only large effects count: 20-25 % or more in tokens. Any rise in rework, findings or escalations ("retry one tier up") counts against a change.

**Not adopted:** `maxEffortLevel`. It would also cap the main session's effort, which the owner sets.

**The largest lever, adopted (the owner's decision, 2026-11-06).**
- **The problem:** the main session re-reads its whole context at every step: 818M cache-read tokens over M1-M5, about 400k per step at that context size.
- **The change:** from Phase 2 a session covers one milestone (`CLAUDE.md`). Each milestone starts small, with the status note as the hand-off.
- **The measure:** the `main` lines (trial 3).
- **Ledger labels:** from Phase 2 the ledger names milestones `P2-Mn`, so that they don't mix with Phase 1's `Mn`.

## Retro: M6 (2026-11-06)

**Avoidable costs, worst first:**
- **The ledger measured the wrong thing for five milestones.** It recorded a completion notice's token count, which is only the last request's context. WP4's notice said 161k; its transcript holds 44.2M cache-read tokens. Fixed: every line is rebuilt from the transcripts (`scripts/agent-usage.mjs`), and the trial measures moved to `index`.
- **2 red CI runs,** both before the pre-push hook: the WIP head `8b6ef85` and the fmt break `4c02685`. After `785d5f9`, which installed the hook, every PR head was green.
- **3 rework rounds:**
  - WP2's `% 20` assertion, which pinned the full sync;
  - WP4's inline check exits, which grew 7zz.wasm 87 %. The brief hadn't asked for shared exits, so that gap was the coordinator's;
  - WP5's mislabelled "M5 ratio" column.
- **One push against the brief:** WP4b pushed to the PR branch. It was green, but agents' fixed rules now allow a PR-branch push only when the brief says so.
- **The spec review's 11 items** included a regression of the coordinator's own: a test that still rejected a request-flag bit that had become valid. `cargo test` caught it before any push, so the existing checks did their job and no new one is needed.
- **The code review's 8 candidates** (`/code-review medium`, 0.81M), each checked before acting:
  - 4 real, now fixed. One is a semantic error WP4's test had pinned: a read-modify-write that faults on the NULL page reported a read. This machine's CPU settled it in one run, through the page-fault error code: x86 reports such a fault as a write.
  - 2 real gaps that Phase 1 can't reach, recorded as Phase 2 prerequisites.
  - 2 not acted on, with their reasons in the ledger.
- **A lead, not a rule yet:** fault semantics come from the CPU, just as instruction results do. L1 records results but not fault kinds. Windows exceptions (Phase 4) are the point to record them natively.

**Changes from this retro:**
- **Mechanical, now automated:** the pre-push hook (refuses a WIP head and unformatted Rust), and the ledger script.
- **Judgement, now rules:**
  - tests assert contracts (the fixed rules and the spec-review brief);
  - PR-branch pushes only when the brief says so (the fixed rules);
  - speedups are measured within one run against the reference build (`phase-1.md`, "Lessons").

**Trials decided:**

| Trial | Result | Verdict |
|---|---|---|
| At most 3 subagents, none spawning its own | never blocked planned work | kept |
| Subagents without a set effort at `high` | the M6 spec review cost 1.06M against M5's 2.57M at `max`, and found two real blockers | kept |
| deep-worker `medium` (WP3a, WP4) against `high` (WP2, WP3b) | median 3.29M against 3.53M (−7 %); rework 1 against 0; findings 4 against 0 (added 2026-11-13: the close-out had left them empty, and they point the same way) | not adopted; `high` stays |
| Read a file once; wait on backgrounded commands | builder index per request 18-27k, against 25k before; no rework traced to it | kept |
| 1. The prune of the always-loaded files | no rework traced to content that moved behind a pointer | kept |
| 4. The merge-danger line | the owner's verdict at the M6 merge | open |
| 2, 3, 5 | not due | running |

## Retro: Phase 2 M1 (2026-11-13)

**Avoidable costs, worst first:**
- **4 of the 6 `wine` runs on the PR branch failed or published wrong data.** Each cost a 14-60 min round trip:
  - run 42306869519, red: `basic.exe` exits 42 by design, and the step's shell was `bash -e`;
  - runs 42353561868 and 42355029562: the unit list was parsed from stdout and stderr together, so under Wine the `+loaddll` lines became 176 bogus units per exe. The first run was also red from a transient 403 at publish, which `release-publish.sh` now retries;
  - run 42356976933, red: `| head` closed a pipe early under `pipefail`.

  The three script bugs are of one kind: shell steps whose first run was in CI. Two more of that kind, `find | sort` on a missing directory and a quote inside a `${VAR:?}` message, were caught by a local dry run with stub tools before any push. So was the code review's `.sha256` fix.
- **Contract drift: 7 of the spec review's 9 items.** WP1 built `wine.yml` and `closure.rs` differently from `wine-build.md` and didn't update the contract:
  - the baselines' wrapper;
  - `closure.json`'s extra fields;
  - where `data_files` come from;
  - the note's field;
  - the prefix path;
  - `+loaddll` during the baseline;
  - the status rules;
  - plus one count in `instruction-scope.md`.

  The releases are immutable, so the contract now documents the published data.
- **1 rework round:** WP2's popfq dropped TF and AC silently, and the review sent it back. Unmodelled behaviour should stop loudly, as unsupported paths do elsewhere.
- **The code review's 5 findings,** all latent:
  - 4 fixed with tests. Two of them were errors read as something else: the closure skipped a PE image whose tables didn't parse, and the delay IAT was read as names;
  - 1 recorded as a known issue.
- **2 branch reconciliations:** the coordinator committed WIP on the PR branch while an agent pushed there. Since then, the coordinator's WIP goes to a side branch through a temporary index, and HEAD doesn't move.
- **Escalations:** none.

**Changes from this retro:**
- **Workflow scripts (a rule where the writer reads it):** the builder's fixed rules now say that a changed workflow step or CI script runs locally before the push:
  - under the job's shell, with stub tools on a recorded input;
  - with the three failure patterns named.

  No static check catches these bugs: shellcheck and actionlint don't model designed exit codes, SIGPIPE or mixed streams.
- **Contract drift:** the builder's fixed rules now say that code which must differ from its contract changes the doc first, in its own commit, and the report says so.
- **Silent drops:** `.claude/rules/translator.md` now says that behaviour the translator doesn't model exits with `EXIT_UNSUPPORTED` before any side effect, and is never dropped.
- **Coordinator WIP:** the temporary-index snapshot is in `docs/dev/commands.md`, and `CLAUDE.md`'s WIP line points to it.

**Trials:**

| Trial | Result | Verdict |
|---|---|---|
| 2. The retro as step 5 | M1: 3 red runs and 1 run with wrong data, all in the new `wine.yml`; 1 rework. The pre-push hook kept every PR head lint-clean. | due at M2's close |
| 3. The hand-off | M2 is the first milestone to start fresh, from this hand-off. The baseline is this session, which held planning and M1: $91.93, 239.0M cache reads, 913k output (`main` line of 2026-11-13). | due at M2's close |
| 4. The merge-danger line | no verdict was recorded at the M6 merge (PR #9); asked again at PR #10's merge | open |
| 5. `/diverge` gates | no bug hunt in M1 | running |

## Retro: Phase 2 M2 (2026-11-14)

**CI, rework and escalations:**
- **No red runs.** All green:
  - `wine-unix` ran 10 times: #1 on WP5's track, #2-#8 on the PR branch, and #9-#10 skipped on the PR's events;
  - `targets` ran twice;
  - `ci` #71-#72 and `bench` #32-#33 ran on the PR.

  M1 had 3 red runs and 1 run with wrong data, all in its new workflow. M2's new `wine-unix.yml` was green from its first run, because WP5 first ran its steps locally under the job's shell (M1's rule).
- **Rework rounds:** 0.
- **Escalations:** none.
- **Findings:** 23 from the two reviews (`agent-stats.mjs --milestone P2-M2`).

**Avoidable costs, worst first:**
- **The coordinator's context: 112.8M cache reads in 424 requests, 266k a request.** The figures come from the session's transcript, per compaction segment, up to 13:48 UTC, when `get_session` last updated. The session read 282.7M in all, so agents read the other 170M.
  - Planning: 88 requests at 313k (max 508k), 27.5M, before the first `/compact`.
  - The next two segments: 205k and 210k a request (93 and 64 requests).
  - **The fourth segment ran 155 requests with no compaction,** from WP2 part 1's merge to the PR: 320k a request, max 555k, 49.6M. That is 44 % of the coordinator's reads.
    - Its checkpoints read 32.3, 17.6, 24.8 and 29.2 %, all under 35 %, while the requests carried 323-555k.
    - `context_usage` leaves out about 75k and lags. Once it fell from 32.3 % to 17.6 % with no compaction.
    - The close's restart read 35.9 % and asked for a compaction, but the `/compact` ran 45 requests later, after the PR.

    Split at its middle, the segment would have read about 16M less.
- **Three stops, 20.7 hours from stop to resume:**
  - the 5-hour limit at 14:57 UTC (WP0 and WP1, 10 minutes in) and at 00:27 (both close agents);
  - the owner's pause at 19:08.

  Every resume by `SendMessage` kept the agent's context, so nothing was redone, and the ledger records each segment (`--since`). The only lever is reading less, since cache reads spend the window.
- **Contracts: 3 wrong statements about Wine and 6 drift items.**
  - **Three statements about Wine were wrong, and none cited a source line:**
    - 64-bit Wine keeps `user_space_limit` for every EXE (`3840819`);
    - the image fallback's search would have grown the memory to 16 GiB (`3840819`);
    - a query at or above 16 GiB returns `STATUS_INVALID_PARAMETER` (`virtual.c` L5740, `fb2133b`).

    WP2 found the first two and the spec review the third. Each contract changed before any code relied on it.
  - **6 of the spec review's 14 items were drift:** the code differed from `wine-unix.md`, and the doc hadn't changed (`fb2133b`). M1's drift rule is only in the builder's fixed rules, and M2's code was written by deep-workers.
- **The close's inputs:**
  - The verification found `target/pe`, `target/native` and `target/wine-spike` missing in the main checkout, and built them after one failed test run.
  - It stalled at 2.9 GB of free disk until two merged worktrees' `target/` directories (12.3 GB) were deleted.
  - The skill's step 1 had no step for the Wine Unix side; the brief added one.
- **Six latent bugs:** the code review found them, and a builder fixed them with tests (`e46fced`). One more finding became a contract rule (`08a95a6`). One bug came from a constant written twice: the x87 tag word was 0xffff in JS's `initFpState` but 0 in C's thread start.
- **Ownership:**
  - **The `section` checks were assigned twice,** because the plan and the status note each had a work-package table. This was caught before any work started (`bbf1eff`).
  - **The overlay hook** sat in WP3's server range, but WP4 needed it. WP4 proposed the diff and the coordinator committed it as `0202`, as the one-owner rule intends.
- **A refused force-push:**
  - WP4 tried to rewrite its pushed track so its contract commit came first, and the deny rules refused it.
  - The coordinator merged WP4's local branch instead, which had the same tree.
  - The coordinator's own resume message had asked WP2 for a `--force-with-lease` on its track.

**Changes from this retro:**
- **Trial 6 reshaped** (`CLAUDE.md`):
  - ask for `/compact` above 25 % instead of 35 %, which is about where the compacted segments peaked (323-325k a request);
  - add a checkpoint after the close's reviews.
- **Statements about Wine** (`.claude/rules/wine.md`, new; it loads with `native/wine/**` and the Wine contracts): a statement about what Wine does cites wine-11.0's file and line where it is written.
- **Drift and history** (both agents' fixed rules):
  - the builder's drift rule now binds deep-workers too;
  - a contract change is committed before the code that needs it, since pushed history is never rewritten.
- **The close's step 1** (`milestone-close`):
  - a preflight for free disk and a fresh checkout's inputs;
  - the Wine Unix side's prepare, build and `test:wine`.
- **One work-package table** (`CLAUDE.md`): once the plan is approved, only the status note's table is kept current.
- **For M3** (its "Next step"):
  - one definition of the initial FP state for C and JS;
  - trial 6's lead: give the research pass's source reading to a subagent. Planning took 27.5M of the coordinator's 112.8M.

**Trials:**

| Trial | Result | Verdict |
|---|---|---|
| 2. The retro as step 5 | M2: 0 red CI runs (M1: 3 red and 1 with wrong data; M6: 2) and 0 rework rounds (M1 and M6: 1 each). M1's workflow-script rule held: `wine-unix.yml` was green from its first run. Its drift rule didn't reach the deep-workers, which left 6 drift items. | **Kept** as a standing step |
| 3. The hand-off | M2 started fresh from M1's starter message and status note, and planned from the docs alone. Totals for the whole session:<br>• M2: $120.33, 282.7M cache reads, 1.50M output (the coordinator's share 112.8M)<br>• M1 with the phase's planning: $91.93, 239.0M<br>• Phase 1's M6: $112.63, 281.9M<br><br>A fresh start removes the context carried between milestones. It doesn't bound the context inside one: the coordinator's reached 555k in M2. | **Kept.** Growth inside a milestone is trial 6's. |
| 4. The merge-danger line | no verdict recorded at PR #10's or #11's merge | open; asked at PR #12's merge |
| 5. `/diverge` gates | no bug hunt in M2 | running |
| 6. Context checkpoints | 4 compactions, each asked for by the coordinator and run by the owner; no auto-compaction.<br>• The coordinator's context per request: 313k before the first compaction, then 205k, 210k, 320k and 134k so far.<br>• Rework from lost context: 0. The double-assigned `section` checks came from two copies of the table, not from a compaction.<br>• The miss: the fourth segment (above). | **Reshaped:** 25 %, plus a checkpoint after the close's reviews. Decided at M3's close: the execution segments' mean context per coordinator request at most 220k, and rework from lost context 0. |

**After the merge (2026-11-14):** trial 4 is reverted. The owner's verdict on PR #12's merge-danger line was "noise", so `milestone-close` step 4 no longer asks for it.
