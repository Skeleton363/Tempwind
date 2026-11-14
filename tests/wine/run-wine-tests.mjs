// Launcher: node --test over tests/wine/*.test.mjs (docs/design/wine-unix.md,
// section 10), against target/wine-unix or TW_WINE_UNIX_DIR (passed through
// the environment). Needs Node 24+ and native/wine/build.sh's output.
import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync } from 'node:fs';
import { resolve } from 'node:path';

const major = Number(process.versions.node.split('.')[0]);
if (major < 24) {
  console.error(`Node ${process.versions.node} is too old: the tests need Node 24+ (see .nvmrc).`);
  process.exit(1);
}
const dir = new URL('.', import.meta.url);
const unixDir = process.env.TW_WINE_UNIX_DIR ?? new URL('../../target/wine-unix/', dir).pathname;
if (!existsSync(resolve(unixDir, 'wine-unix.wasm'))) {
  console.error(`${unixDir}: no wine-unix.wasm. Build it with native/wine/build.sh` +
    ' (or set TW_WINE_UNIX_DIR to a built or fetched wine-unix).');
  process.exit(1);
}
const files = readdirSync(dir).filter((f) => f.endsWith('.test.mjs')).sort()
  .map((f) => new URL(f, dir).pathname);
const r = spawnSync(process.execPath, ['--test', ...files], {
  stdio: 'inherit',
  env: { ...process.env, TW_WINE_UNIX_DIR: resolve(unixDir) },
});
process.exit(r.status ?? 1);
