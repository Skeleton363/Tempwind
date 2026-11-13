---
name: pause
description: Stop work cleanly on a clean tree with everything committed and the resume path written down. Run only by the user as /pause.
disable-model-invocation: true
---
# Pause the session

1. Stop running background agents with TaskStop. Their transcripts survive; a later message resumes them.
2. `git status --short`. Group the uncommitted paths by the agent or track that owns them.
3. Commit each group as a pathspec WIP checkpoint: `git commit -m "WIP: <track>: <state>" -- <paths>`. Never `git add -A`. Identity: `Skeleton363 <46569584+Skeleton363@users.noreply.github.com>` (check `git config user.email`); no attribution lines.
4. Cancel pending check-ins this session created (`delete_trigger` with each trigger id; `list_triggers` finds them).
5. In the current phase's note, `docs/status/phase-N.md` (the highest N there), add or refresh a section `## Mn progress (paused YYYY-MM-DD)` with:
   - what is committed (commit ids, tests that pass);
   - what is in progress (agent, files, state);
   - how to resume, written for a fresh session without this conversation: per track, its side branch and head, where its brief is (the decisions section), what is left, the first command. "Message agent X" is only a shortcut for the same session. Point at artifacts instead of repeating them.
6. Commit that file by pathspec, then `git push` (plain).
7. Report in a few lines: commits pushed, what is in progress, the resume step. End with a starter message of at most 3 lines for a fresh session, pointing at that section (trial 3 in `docs/dev/agent-runs.md`).
