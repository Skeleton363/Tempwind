#!/bin/bash
# SessionStart hook: start dockerd in Claude Code cloud sessions. The cached
# environment keeps files, not running processes. Runs async so it never
# delays the session; local sessions are left alone.
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi
echo '{"async": true, "asyncTimeout": 120000}'
exec "$CLAUDE_PROJECT_DIR/tools/start-dockerd.sh"
