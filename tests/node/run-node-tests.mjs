// Launcher: node --test over tests/node/*.test.mjs, adding
// --experimental-wasm-memory64 on Node < 24.
import { spawnSync } from 'node:child_process';
import { readdirSync } from 'node:fs';

const dir = new URL('.', import.meta.url);
const files = readdirSync(dir).filter((f) => f.endsWith('.test.mjs')).sort()
  .map((f) => new URL(f, dir).pathname);
const major = Number(process.versions.node.split('.')[0]);
const args = [];
if (major < 24) args.push('--experimental-wasm-memory64');
args.push('--test', ...files);
const r = spawnSync(process.execPath, args, { stdio: 'inherit' });
process.exit(r.status ?? 1);
