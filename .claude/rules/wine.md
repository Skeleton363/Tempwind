---
paths:
  - "native/wine/**"
  - "docs/design/wine-unix.md"
  - "docs/design/guest-memory.md"
---
# Wine's Unix side
- A statement about what Wine does cites wine-11.0's file and line where it is written, in a contract or a patch header (`virtual.c` L5740). That covers a limit, a search order, a fallback, a status it returns, or a file it opens. Read that code path before writing the statement. In M2, three of the contract's statements about Wine had no source line, and all three were wrong (`docs/dev/agent-runs.md`, M2's retro).
- A hook or change inside a Wine function lists that function's callers in wine-11.0 (grep) and says what it does on each caller's path. M3's exit hook went into `abort_process`, whose other caller, `abort_thread` (`unix/thread.c` L1477), ends the process on Wine's errors, so the hook turned them into normal exits. The spec review caught it at the close (`docs/dev/agent-runs.md`, M3's retro).
