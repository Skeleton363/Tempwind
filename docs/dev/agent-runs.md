# Agent runs: cost and quality ledger

We judge workflow decisions (effort levels, caps, agent rules) by data, at almost no token cost. Every number here is one the harness already reports.

## What is recorded

**`agent-runs.jsonl`** gets one line per finished agent run. The coordinator writes it from the run's completion notice.
- **From the notice:**
  - `tokens`: the harness's subagent token count;
  - `tool_uses`;
  - `minutes`.
- **The run's setup:** date, milestone, task, agent, model and effort.
- **`resume`:** true when the line continues an earlier run. Notices report running totals, so a resume's line records the increment.
- **Quality:**
  - `rework`: follow-up messages needed before the result passed the coordinator's verification (0 means right the first time);
  - `findings`: valid review findings in the run's own diff, filled in at milestone close-out.

**The main session** gets a `main` line at each milestone's start and end, from `get_session`: the cumulative `cost_usd`, `output_tokens` and `cache_read_tokens`. The difference between two lines is that stretch's usage.

**The baseline** comes from the session history for M1 to M5 (34 runs, `source: backfill`). Effort before 2026-10-31 was not recorded.

## Reading it

`node scripts/agent-stats.mjs [--milestone M6]` prints, per agent and effort, the runs, median tokens, tokens per tool use, rework and findings, then the main session's usage between snapshots.

## Trials (started 2026-11-05, decided at the M6 close-out)

| Change | Where | Measure | Kept if |
|---|---|---|---|
| At most 3 subagents at once, none spawning its own | `.claude/settings.json` `env` | "limit reached" refusals | it never blocks planned work |
| Subagents that name no effort (Plan, general-purpose reviewers) run at `high`, not the session's `max` | `CLAUDE_CODE_SUBAGENT_EFFORT` | tokens per review or plan, against the baseline (306k and 314k at `max`) | tokens fall, and the close-out reviews still find the real defects (none escapes to CI or later) |
| deep-worker at `medium` for M6's WP3a and WP4; `high` for WP2 and WP3b | the Agent tool's `effort` per spawn | tokens per tool use, rework, findings | `medium` uses at least 25 % fewer tokens with no more rework or findings; it then becomes the default for Phase 2 |
| Read a file once; wait on backgrounded commands | builder and deep-worker fixed rules | tokens per tool use, against the baseline | no measured harm |

**Decision rule:** samples are small, so only large effects count: 20-25 % or more in tokens. Any rise in rework, findings or escalations ("retry one tier up") counts against a change.

**Not adopted:** `maxEffortLevel`. It would also cap the main session's effort, which the owner sets.

**The largest lever is not a trial.** The main session re-reads its whole context at every step: 818M cache-read tokens over M1-M5, about 400k per step at today's context size. A fresh session per milestone, with the status note as the hand-off, would start each milestone small. Adopting it is the owner's decision, since `CLAUDE.md` says a session covers a phase. The `main` lines would measure it.
