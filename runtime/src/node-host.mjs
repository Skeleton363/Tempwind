// Usage: node runtime/src/node-host.mjs target/spike
// (Node < 24 needs --experimental-wasm-memory64.)
import { readFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { runManifest } from './run.mjs';

export async function runSpikeDir(dir, opts = {}) {
  const manifest = JSON.parse(await readFile(join(dir, 'manifest.json'), 'utf8'));
  return runManifest(manifest, async (name) => new Uint8Array(await readFile(join(dir, name))), opts);
}

export function formatResults(results) {
  const lines = [];
  for (const r of results) {
    lines.push(`${r.ok ? 'PASS' : 'FAIL'} ${r.name}`);
    for (const d of r.diffs) lines.push(`     ${d}`);
    if (r.error) lines.push(`     error: ${r.error}`);
  }
  const failed = results.filter((r) => !r.ok).length;
  lines.push(`${results.length - failed}/${results.length} cases passed`);
  return lines.join('\n');
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const dir = process.argv[2];
  if (!dir) { console.error('usage: node runtime/src/node-host.mjs <spike-dir>'); process.exit(2); }
  try {
    const results = await runSpikeDir(dir);
    console.log(formatResults(results));
    process.exit(results.length > 0 && results.every((r) => r.ok) ? 0 : 1);
  } catch (e) {
    console.error(`cannot run ${dir}: ${e.message}\nCreate it with: cargo run -p testkit --release -- spike --out ${dir}`);
    process.exit(1);
  }
}
