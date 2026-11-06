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
  - `findings`: valid review findings in the run's own diff, filled in at milestone close-out.
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
| 2. A retro as `milestone-close` step 5: a mechanical mistake gets an automated check, a judgement call a rule where its writer or reviewer reads it | The first retro, from M6's three failures, applied now:<br>• `tools/git-hooks/pre-push`, installed by the SessionStart hook, refuses a WIP head and unformatted Rust on `main` and `claude/*`. Tested on `8b6ef85` and `4c02685`: both refused.<br>• The WIP-to-side-branch rule is in the agents' fixed rules and `CLAUDE.md`.<br>• The test-contract rule is in the fixed rules and the spec-review brief. | M6: 2 avoidable red CI runs (the WIP head `8b6ef85`, the fmt break `4c02685`) and 1 rework (the `% 20` assertion) | Avoidable red CI runs and rework rounds per milestone | Phase 2's second milestone |
| 3. A hand-off in `/pause`: the resume path works for a fresh session, and the report ends with a starter message | `/pause` steps 5 and 7 | Main session: 818M cache-read tokens over M1-M5 in one session | `main` lines for a milestone that starts in a fresh session | Phase 2's first milestone, fresh by the rule "a session covers a phase" |
| 4. A merge-danger line opening each PR description: one-way or two-way door, and the blast radius | `milestone-close` step 4 | none (no merge has gone wrong) | The owner's verdict at the next merge: useful or noise | M6 merge |
| 5. Gates in `/diverge`: one red command, a minimal input, ranked falsifiable hypotheses, one change at a time, `[dbg]`-tagged debug output | `/diverge` steps 3 and 4. Ledger lines of bug hunts get `"kind": "diverge"`. | none (no recent bug hunt went wrong) | Rework and escalations on bug-hunt runs | After three bug hunts |

Trials 4 and 5 are cheap; if they show nothing, they are removed.

## Trials (started 2026-11-05, decided at the M6 close-out)

| Change | Where | Measure | Kept if |
|---|---|---|---|
| At most 3 subagents at once, none spawning its own | `.claude/settings.json` `env` | "limit reached" refusals | it never blocks planned work |
| Subagents that name no effort (Plan, general-purpose reviewers) run at `high`, not the session's `max` | `CLAUDE_CODE_SUBAGENT_EFFORT` | `index` per review or plan, against the baseline at `max` (Plan 2.06M, the M5 spec review 2.57M) | tokens fall, and the close-out reviews still find the real defects (none escapes to CI or later) |
| deep-worker at `medium` for M6's WP3a and WP4; `high` for WP2 and WP3b | the Agent tool's `effort` per spawn | `index` per run and per request, rework, findings | `medium` costs at least 25 % less (`index`) with no more rework or findings; it then becomes the default for Phase 2 |
| Read a file once; wait on backgrounded commands | builder and deep-worker fixed rules | `index` per request, against the baseline | no measured harm |

**Decision rule:** samples are small, so only large effects count: 20-25 % or more in tokens. Any rise in rework, findings or escalations ("retry one tier up") counts against a change.

**Not adopted:** `maxEffortLevel`. It would also cap the main session's effort, which the owner sets.

**The largest lever is not a trial.** The main session re-reads its whole context at every step: 818M cache-read tokens over M1-M5, about 400k per step at today's context size. A fresh session per milestone, with the status note as the hand-off, would start each milestone small. Adopting it is the owner's decision, since `CLAUDE.md` says a session covers a phase. The `main` lines would measure it.

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
| deep-worker `medium` (WP3a, WP4) against `high` (WP2, WP3b) | median 3.29M against 3.53M (−7 %); rework 1 against 0 | not adopted; `high` stays |
| Read a file once; wait on backgrounded commands | builder index per request 18-27k, against 25k before; no rework traced to it | kept |
| 1. The prune of the always-loaded files | no rework traced to content that moved behind a pointer | kept |
| 4. The merge-danger line | the owner's verdict at the M6 merge | open |
| 2, 3, 5 | not due | running |
