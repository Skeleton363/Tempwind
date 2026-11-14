---
paths:
  - "native/wine/**"
  - "docs/design/wine-unix.md"
  - "docs/design/guest-memory.md"
---
# Wine's Unix side
- A statement about what Wine does cites wine-11.0's file and line where it is written, in a contract or a patch header (`virtual.c` L5740). That covers a limit, a search order, a fallback, a status it returns, or a file it opens. Read that code path before writing the statement. In M2, three of the contract's statements about Wine had no source line, and all three were wrong (`docs/dev/agent-runs.md`, M2's retro).
