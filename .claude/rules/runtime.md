---
paths:
  - "runtime/**"
  - "tests/node/**"
  - "tests/sevenzip/**"
---
# Runtime and shim rules

- The Win32 shim is strict. Never return a made-up success. An unknown case (unimplemented import, `GetProcAddress` of a missing function, exception, thread creation) stops loudly.
- Look things up before implementing a shim function. Sources: Microsoft's documentation, Wine's source (wine-11.0 tarball) and the program's own source. Read them; never copy code. The clean-room list in `CLAUDE.md` applies.
- Contracts: `docs/design/runtime.md` and `docs/design/abi.md`. Change the doc first.
- Tests go through `tests/node/shim-harness.mjs`.
- In Node, compile with the synchronous `new WebAssembly.Module`. `await WebAssembly.compile` never settles under `--no-wasm-lazy-compilation`. Eager TurboFan is `--no-liftoff --no-wasm-lazy-compilation`; measure one process per configuration. `--js-flags` has no effect in Playwright's headless shell.
- Avoid BigInt in hot loops (use 32-bit Numbers).
- Hosts set the FP defaults a real thread starts with (MXCSR); a zeroed state block is not a thread's state.
- Never convert line ends in `tests/sevenzip/expected/` (binary in `.gitattributes`). Expected files come from `--mode wine`; real Windows in CI is the reference. The `tests/native` checks compare as CRLF.
- Do not edit `tests/sevenzip/expected/` by hand.
