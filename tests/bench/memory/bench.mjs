// Node runner for the guest-memory micro-benchmark.
//
//   node tests/bench/memory/bench.mjs [--json FILE] [--reps N] [--variants a,b] [--no-growth]
//
// Node 22 needs no flag for memory64 defined inside a module (V8 12.4 has it
// on by default); `--experimental-wasm-memory64` is accepted and harmless.
// See README.md for the browser runner and recorded results.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
vm.runInThisContext(readFileSync(join(here, 'core.js'), 'utf8'), { filename: 'core.js' });
const { runAll, growthTest } = globalThis.MemBench;

const args = process.argv.slice(2);
const opt = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const jsonOut = opt('--json');
const reps = Number(opt('--reps') ?? 5);
const variants = opt('--variants')?.split(',');

const env = {
  runtime: `node ${process.version}`,
  v8: process.versions.v8,
  execArgv: process.execArgv.join(' '),
  arch: process.arch,
};
console.log(`# ${env.runtime} (V8 ${env.v8}) ${env.execArgv}`);

const results = runAll({ reps, variants, log: (m) => console.error(m) });
const base = results.find((r) => r.variant === 'm64');
console.log('\n| variant | seq ns/access | rand ns/access | chase ns/access | seq vs m64 | rand vs m64 | chase vs m64 |');
console.log('|---|---|---|---|---|---|---|');
for (const r of results) {
  const rel = (k) => (base ? `${(r[k] / base[k]).toFixed(2)}x` : '');
  console.log(`| ${r.variant} | ${r.seq.toFixed(3)} | ${r.rand.toFixed(2)} | ${r.chase.toFixed(2)} | ${rel('seq')} | ${rel('rand')} | ${rel('chase')} |`);
}
const checks = new Set(results.map((r) => JSON.stringify(r.checks)));
if (checks.size !== 1) {
  console.error('result checksums differ between variants:', results.map((r) => [r.variant, r.checks]));
  process.exitCode = 1;
} else {
  console.log(`\nAll variants computed identical results (${[...checks][0]}).`);
}

let growth = null;
if (!args.includes('--no-growth')) {
  const rss = () => process.memoryUsage().rss;
  growth = [growthTest({ shared: true, rss }), growthTest({ shared: false, rss })];
  for (const g of growth) {
    console.log(`\nGrowth, ${g.shared ? 'shared' : 'non-shared'} memory64, max ${g.maxPages} pages, ${g.api ?? ''}: create ${g.createMs?.toFixed(2)} ms${g.error ? `, error: ${g.error}` : ''}`);
    for (const row of g.rows) {
      console.log(row.error ? `  grow to ${row.gib} GiB: ${row.error}` : `  grow to ${row.gib} GiB: ${row.growMs.toFixed(2)} ms, RSS ${row.rssMiB.toFixed(0)} MiB`);
    }
    if (g.touch256MiBMs !== undefined) console.log(`  touch 256 MiB: ${g.touch256MiBMs.toFixed(1)} ms, RSS ${g.rssAfterTouchMiB.toFixed(0)} MiB`);
  }
}

if (jsonOut) writeFileSync(jsonOut, JSON.stringify({ env, results, growth }, null, 2) + '\n');
