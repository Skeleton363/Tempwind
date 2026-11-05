# Research log

Each milestone's planning starts with a short research pass over official and community sources for its domain and our workflow. Log one dated entry per pass. Each item gives: the source (with link), the finding, why it matters here, and the decision with the reason.

## 2026-10-31: Claude Code workflow

### Official

Sources:
- https://code.claude.com/docs/en/best-practices
- https://code.claude.com/docs/en/skills
- https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/claude-4-best-practices

| Finding | Relevance | Decision |
|---|---|---|
| Keep CLAUDE.md short | Ours had grown to about 1,500 words, mostly commands | Trimmed to about 875 words; commands moved to `docs/dev/commands.md`. Adopted |
| Path-scoped rules (`.claude/rules/` with `paths:`) | Area guidance only matters in that area | `translator.md`, `oracle.md`, `runtime.md`. Adopted |
| Procedures as skills | We ran pause, milestone close and divergence hunts by hand | `/pause`, `/milestone-close`, `/diverge`. Adopted |
| Never-rules as deny rules or hooks | Prose rules get forgotten | Deny rules in `.claude/settings.json` (fixtures, `playwright install`, `service docker start`, force push). Adopted |
| Fixed agent instructions live in agent definitions | Every brief repeated the same rules | Added to `deep-worker.md` and `builder.md`. Adopted |
| Anti-overengineering and anti-test-fitting guidance | Matches "no scope creep" and "general solutions" | In the agent rules. Adopted |
| Review by a fresh agent against the spec, skipping style | Catches spec drift the author misses | Two-stage review in `/milestone-close`. Adopted |
| State files and git as memory for long tasks | Sessions don't share memory | Already practice (`docs/status/`, WIP commits); kept |
| Just-in-time context; subagents return condensed summaries | Keeps the main context small | Already policy ("Working efficiently"); kept |

Also adopted: `ARCHITECTURE.md` (below).

### Community

| Source | Finding | Decision |
|---|---|---|
| https://github.com/obra/superpowers | Brainstorm, plan, a fresh subagent per task with two-stage review; systematic debugging; verification before done | Adopted as tailored skills (`/milestone-close`, `/diverge`), not installed |
| https://github.com/hesreallyhim/awesome-claude-code | Curated index of skills, hooks and workflows | The index to watch at each milestone's research pass |
| Practitioner cost reports | Subagents cost 2-3x, agent teams 4-15x; set small models, keep context-heavy work in the main loop, use read-only reviewers | Kept as policy ("Delegation and effort") |
| https://matklad.github.io/2021/02/06/ARCHITECTURE.md.html | A short map (overview, codemap, invariants, cross-cutting concerns) orients newcomers | `ARCHITECTURE.md` adopted |

### Not adopted yet

- TypeScript checking for `runtime/` (`// @ts-check` plus `tsc`): later, at M6 close-out, when the runtime's shape settles.
- More network domains: not needed; the allowed list covers current work.

## 2026-11-04: Claude Code workflow digest

Proposals only. Items 1-2 rest on undated Claude 5 guides: new to this log (its last entry cites the Claude 4 guide), maybe older than the window (2026-10-31 to 11-04). Item 3 is from the window.

### 1. Hard caps instead of prose
- Source: [Opus 5 guide](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/prompting-claude-opus-5#controlling-subagent-spawning), [changelog 2.1.316](https://code.claude.com/docs/en/changelog), [week 37](https://code.claude.com/docs/en/whats-new/2026-w37.md), [HN thread](https://news.ycombinator.com/item?id=50284255).
- Says: Opus 5 "delegates to subagents more readily"; Claude Code can cap subagent concurrency and depth; `CLAUDE_CODE_SUBAGENT_EFFORT` sets effort for agents naming none; `maxEffortLevel` caps effort.
- Why: our limits are prose ("never max effort"). The HN thread (anecdotal, unchecked) says background-agent summaries take a large share of spend.
- Proposal: `.claude/settings.json` `env`: `CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS=3`, `CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH=1`, `CLAUDE_CODE_SUBAGENT_EFFORT=medium`; plus `"maxEffortLevel": "xhigh"`. Confirm semantics on the [SDK page](https://code.claude.com/docs/en/agent-sdk/subagents#cap-subagent-depth-concurrency-and-spend) first (not read).
- Benefit/cost: a ceiling on fan-out, the fastest way to burn plan usage; four lines, overridable per session.

### 2. Effort sweep for deep-worker
- Source: [Effort](https://platform.claude.com/docs/en/build-with-claude/effort), [Opus 5.5 guide](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/prompting-claude-opus-5-5#calibrate-effort).
- Says: Opus 5.5 defaults to `medium` and "matches or exceeds" Opus 5 at `high` on coding, with fewer tokens; sweep, and reserve `xhigh`/`max` for measured gains. Sonnet 5.5 advises `medium` for well-specified agentic coding; Haiku 5.5 warns `low` can skip searches in long prompts.
- Why: `deep-worker` (`opus`, `high`) is our costliest agent; the `builder` and `Explore` pins already fit.
- Proposal: pilot `effort: medium` in `.claude/agents/deep-worker.md` on one known-hard `/diverge` case; compare `/usage` tokens and test results with `high`. Only if `opus` resolves to 5.5 (check `/model`).
- Benefit/cost: fewer thinking tokens per turn; two runs; "retry one tier up" bounds the risk.

### 3. Two harness changes
- Source: [changelog 2.1.317 and 2.1.318](https://code.claude.com/docs/en/changelog).
- Says: re-reading an unchanged file now returns it whole (the stub is gone); background subagents move shell commands to the background after 3 minutes.
- Why: re-reads cost full price; cargo and 7-Zip suites may be backgrounded.
- Proposal: add to the fixed rules in `.claude/agents/builder.md` and `deep-worker.md`: "Read a file once; re-read only after a change or compaction. Commands past 3 minutes go to the background: wait; never re-run or report done meanwhile."
- Benefit/cost: fewer duplicate reads and runs; about 30 words per agent.

**Checked, nothing new:** engineering blog (newest post 2026-10-13, unrelated); awesome-claude-code mirrors; Reddit (blocked). Keep `/milestone-close`: a fresh-agent diff review is the writer-verifier pattern the Opus 5 guide calls effective.

### Decisions (2026-11-05; the owner asked the coordinator to decide, with a cheap way to measure)
- **Item 1:**
  - Adopted: at most 3 subagents at once, and no nested spawning.
  - Adopted, at `high` rather than the proposed `medium`: the default effort for subagents that name none (Plan, general-purpose reviewers). They do judgment work, and `high` is still well below the session's `max`.
  - Not adopted: `maxEffortLevel`, since it would also cap the main session.
  - The docs confirmed the semantics: the SDK page "Cap subagent depth, concurrency, and spend"; the changelog for 2.1.316 and 2.1.317.
- **Item 2:** a pilot in M6. deep-worker runs at `medium` for WP3a and WP4, and at `high` for WP2 and WP3b. Decided at the close-out.
- **Item 3:** adopted in the builder and deep-worker fixed rules (changelog 2.1.317 and 2.1.318).
- **Measurement:** `docs/dev/agent-runs.md` and `docs/dev/agent-runs.jsonl`, a ledger of the token figures the harness already reports, plus rework and review findings as the quality signals.

## 2026-11-04: M6, translator speed

The M6 planning pass, with 4 searches. Several hosts are blocked here (github.com pages of other repos, spidermonkey.dev), so some items rest on search summaries.

### Sources
- **v86's design notes** ("how it works"; search summary). Its wasm JIT emits deep `br_table`s with locals live across the whole function, which browsers handle poorly, and memory use climbs. Our backward branches go through a function-wide `br_table` in the same way. Decision: WP3, loop structuring.
- **QEMU TCG** (`tcg/README`; search summary).
  - Liveness runs per basic block and removes dead condition-code computations.
  - Globals are stored before a helper call unless the helper is marked as not reading or writing them, and reloaded only if it may write them.

  Decision: WP2 stores only dirty values and reloads only live ones, with MOD summaries for direct callees.
- **SpiderMonkey, "Is Memory64 actually worth using?"** (spidermonkey.dev, 2025-01-15; search summary).
  - Memory64 can't use guard pages there, so every access pays a bounds check.
  - V8 enables memory64 trap handling by default since 13.6 (`tests/bench/memory/README.md`), so Node and Chrome don't pay it.

  Decision: it matters once Firefox is a target. M6 measures only our own NULL and store checks.
- **LLVM's WebAssembly backend** (CFGSort, CFGStackify, FixIrreducibleControlFlow) **and Binaryen's Relooper** (search summaries). These are the standard ways to turn a CFG into wasm loops and blocks; irreducible flow keeps a dispatch block. Decision: WP3 follows CFGSort and CFGStackify, with a selector local for entries into a loop's interior.

### Not adopted
- **XMM caching in locals:** neither benchmark is floating point. Revisit if profiles show XMM traffic.
- **USE summaries for callees:** they would only save the caller's stores, and they would need cold-path stores when a callee unwinds.
