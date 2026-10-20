// Launcher: node --test over tests/node/*.test.mjs. Needs Node 24+ (memory64
// on by default); the version is pinned in .nvmrc for CI and cloud sessions.
import { spawnSync } from 'node:child_process';
import { readdirSync } from 'node:fs';

const major = Number(process.versions.node.split('.')[0]);
if (major < 24) {
  console.error(`Node ${process.versions.node} is too old: the tests need Node 24+ (see .nvmrc; ` +
    'cloud sessions get it from tools/cloud-setup.sh).');
  process.exit(1);
}
const dir = new URL('.', import.meta.url);
const files = readdirSync(dir).filter((f) => f.endsWith('.test.mjs')).sort()
  .map((f) => new URL(f, dir).pathname);
const r = spawnSync(process.execPath, ['--test', ...files], { stdio: 'inherit' });
process.exit(r.status ?? 1);
