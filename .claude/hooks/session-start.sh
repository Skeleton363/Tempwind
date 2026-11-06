#!/bin/bash
# SessionStart hook for Claude Code cloud sessions; local sessions are left
# alone. Commits get the repository owner's identity, unsigned (CLAUDE.md, "How
# we work"); the image's global git config would make them Claude's. The
# pre-push hook in tools/git-hooks keeps WIP heads and unformatted Rust off the
# branches CI runs on. Then dockerd starts: the cached environment keeps
# files, not running processes.
# That part runs async so it never delays the session.
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi
git -C "$CLAUDE_PROJECT_DIR" config user.name "Skeleton363"
git -C "$CLAUDE_PROJECT_DIR" config user.email "46569584+Skeleton363@users.noreply.github.com"
git -C "$CLAUDE_PROJECT_DIR" config commit.gpgsign false
git -C "$CLAUDE_PROJECT_DIR" config core.hooksPath tools/git-hooks
echo '{"async": true, "asyncTimeout": 120000}'
exec "$CLAUDE_PROJECT_DIR/tools/start-dockerd.sh"
