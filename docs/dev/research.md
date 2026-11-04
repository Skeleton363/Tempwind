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
