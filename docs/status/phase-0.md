# Phase 0 status: Foundations

Draft PR: https://github.com/Skeleton363/Tempwind/pull/1 ("[Phase 0] Foundations").
Branch: `claude/epic-hypatia-9y2839`.

_This note is being filled in during the session._

## Environment findings (session 1, 2026-10-20)

| Item | Found | Action |
|---|---|---|
| CPU | Intel Xeon @ 2.1 GHz, 4 cores, AVX2/AVX-512/BMI2/ADX/SHA | none |
| Rust | stable 1.97.0 preinstalled; `rust-toolchain.toml` pins `1.97.0`, which rustup installs on first use (static.rust-lang.org reachable, ~30 s) | none |
| Node | 22.22 (`/opt/node22`). memory64 needs `--experimental-wasm-memory64`. Node 24 is the CI target. | setup script: install Node 24 |
| Chrome | `/opt/chrome` is **empty**. Playwright Chromium 141 is at `/opt/pw-browsers/chromium-1194` (`PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers`). `@playwright/test@1.56.1` matches it. | setup script: populate `/opt/chrome`, or CLAUDE.md should point at `/opt/pw-browsers` |
| Docker | `service docker start` fails (`ulimit: error setting limit`). Starting `dockerd` directly works. | setup script: start `dockerd` directly |
| Emscripten | `emscripten/emsdk:6.0.12` pulls from Docker Hub | none |
| MinGW | `x86_64-w64-mingw32-gcc` preinstalled | none |
| ptrace | no Yama LSM; `mmap_min_addr` = 4096 | none |
| Disk | ~30 GB free | none |
