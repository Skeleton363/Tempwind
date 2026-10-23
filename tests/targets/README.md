# Phase 1 targets

Real programs the translator must run in Phase 1: the 64-bit 7-Zip CLI (`7zz.exe`, the standalone Alone2 bundle) and CoreMark (`coremark.exe`), both built with MinGW-w64 in GitHub Actions. Sessions cannot reach their upstream hosts, so CI mirrors the sources and publishes the builds as releases of this repo; sessions download them with `scripts/fetch-artifact.sh`.

| Path | What |
|---|---|
| `sources.json` | Pinned upstream sources (URL, version or commit, sha256). |
| `targets.lock` | The `targets-<sha7>` release every job and session uses. Updated by the PR that changes the targets. |
| `7zip/build.sh` | Builds `7zz.exe` from the 7-Zip source with MinGW (upstream `makefile.gcc` flags; build-system tweaks only, never source patches). |
| `coremark/build.sh`, `coremark/port/` | Builds `coremark.exe` from the upstream core files plus our MIT Win64 port (QueryPerformanceCounter timer). |
| `smoke.sh` | Native smoke test (Windows runner, and the Linux runner under Wine). |

Releases (see `.github/workflows/targets.yml`):
- `src-7zip-<version>`, `src-coremark-<sha7>`: mirrored upstream sources, published once.
- `targets-<sha7>`: `7zz.exe`, `coremark.exe`, `census.json`, `census.md`, `build-info.json`, plus a `.sha256` per asset.

Decisions and the instruction scope that come from these binaries: `docs/design/instruction-scope.md`, `docs/status/phase-1.md`.
