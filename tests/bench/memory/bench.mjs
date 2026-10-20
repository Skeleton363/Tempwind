// Node runner for the guest-memory micro-benchmark.
//
//   node tests/bench/memory/bench.mjs [--json FILE] [--runs N] [--reps N] [--variants a,b] [--no-growth]
//   node tests/bench/memory/bench.mjs --browser [auto|PATH] ...   (headless Chromium via --dump-dom)
//
// Node 22 needs no flag for memory64 defined inside a module (V8 12.4 has it
// on by default); `--experimental-wasm-memory64` is accepted and harmless.
// See README.md for the browser runner and recorded results.
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, join } from 'node:path';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
vm.runInThisContext(readFileSync(join(here, 'core.js'), 'utf8'), { filename: 'core.js' });
const { runAll, growthTest, countMemories } = globalThis.MemBench;

const args = process.argv.slice(2);
const opt = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const jsonOut = opt('--json');
const reps = Number(opt('--reps') ?? 5);
const variants = opt('--variants')?.split(',');

const runs = Number(opt('--runs') ?? 1);

// Median of each cell over several complete runs (the machine may be shared).
function mergeRuns(all) {
  const med = (xs) => [...xs].sort((a, b) => a - b)[xs.length >> 1];
  return all[0].map((r, i) => {
    const out = { ...r, runs: all.length };
    for (const k of ['hot', 'seq', 'rand', 'chase', 'store']) {
      out[k] = med(all.map((run) => run[i][k]));
      out[`${k}All`] = all.map((run) => run[i][k]);
    }
    return out;
  });
}

let env, results, growth = null, memories = null;
const browser = args.includes('--browser')
  ? (opt('--browser') && !opt('--browser').startsWith('--') ? opt('--browser') : 'auto')
  : undefined;
if (browser !== undefined) {
  const all = [];
  for (let k = 0; k < runs; k++) {
    console.error(`browser run ${k + 1}/${runs}`);
    const d = runInBrowser(browser, k > 0);
    env = d.env;
    if (!growth && d.growth) {
      growth = d.growth;
      memories = d.memories;
    }
    all.push(d.results);
  }
  results = mergeRuns(all);
} else {
  env = {
    runtime: `node ${process.version}`,
    v8: process.versions.v8,
    execArgv: process.execArgv.join(' '),
    arch: process.arch,
  };
  const all = [];
  for (let k = 0; k < runs; k++) all.push(runAll({ reps, variants, log: (m) => console.error(`run ${k + 1}/${runs}: ${m}`) }));
  results = mergeRuns(all);
  if (!args.includes('--no-growth')) {
    const rss = () => process.memoryUsage().rss;
    growth = [growthTest({ shared: true, rss }), growthTest({ shared: false, rss })];
    memories = countMemories({ limit: 64 });
  }
}
report(env, results, growth, memories);
if (jsonOut) writeFileSync(jsonOut, JSON.stringify({ env, results, growth, memories }, null, 2) + '\n');

// Run bench.html in headless Chromium with --dump-dom and parse its JSON.
function runInBrowser(path, noGrowth) {
  const candidates = [path, process.env.CHROME, '/opt/pw-browsers/chromium-1194/chrome-linux/chrome',
    '/opt/chrome/chrome', '/opt/chrome/chrome-linux64/chrome'].filter((p) => p && p !== 'auto');
  const chrome = candidates.find((p) => existsSync(p));
  if (!chrome) throw new Error(`no Chromium found (tried ${candidates.join(', ')}); pass --browser PATH or set CHROME`);
  const q = new URLSearchParams({ reps: String(reps) });
  if (variants) q.set('variants', variants.join(','));
  if (noGrowth || args.includes('--no-growth')) q.set('growth', '0');
  const url = `${pathToFileURL(join(here, 'bench.html')).href}?${q}`;
  const profile = mkdtempSync(join(tmpdir(), 'membench-chrome-'));
  try {
    const r = spawnSync(chrome, ['--headless', '--no-sandbox', '--disable-gpu', '--no-first-run',
      `--user-data-dir=${profile}`, '--dump-dom', url], { encoding: 'utf8', maxBuffer: 64 << 20, timeout: 900000 });
    const m = /BENCH-JSON-BEGIN\n([\s\S]*?)\nBENCH-JSON-END/.exec(r.stdout || '');
    if (!m) throw new Error(`no results from ${chrome}: status ${r.status} ${r.error ?? ''}\n${(r.stderr || '').slice(-2000)}`);
    const data = JSON.parse(m[1].replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&'));
    if (data.error) throw new Error(`benchmark failed in browser: ${data.error}`);
    data.env.runtime = `${data.env.runtime} (${chrome})`;
    return data;
  } finally {
    rmSync(profile, { recursive: true, force: true });
  }
}

function report(env, results, growth, mc) {
  console.log(`# ${env.runtime}${env.v8 ? ` (V8 ${env.v8})` : ''} ${env.execArgv ?? ''}`);
  console.log(`ns per access, median of ${results[0]?.runs ?? 1} run(s) x ${reps} repetitions (hot/seq/store 32 Mi accesses, rand 8 Mi, chase 4 Mi).`);
  const base = results.find((r) => r.variant === 'm64');
  const K = ['hot', 'seq', 'rand', 'chase', 'store'];
  const digits = { hot: 2, seq: 2, rand: 1, chase: 0, store: 2 };
  console.log(`\n| variant | ${K.join(' | ')} | ${K.map((k) => `${k} vs m64`).join(' | ')} |`);
  console.log(`|---|${K.map(() => '---|').join('')}${K.map(() => '---|').join('')}`);
  for (const r of results) {
    const rel = (k) => (base ? `${(r[k] / base[k]).toFixed(2)}x` : '');
    console.log(`| ${r.variant} | ${K.map((k) => r[k].toFixed(digits[k])).join(' | ')} | ${K.map(rel).join(' | ')} |`);
  }
  const checks = new Set(results.map((r) => JSON.stringify(r.checks)));
  if (checks.size !== 1) {
    console.error('result checksums differ between variants:', results.map((r) => [r.variant, r.checks]));
    process.exitCode = 1;
  } else {
    console.log(`\nAll variants computed identical results (${[...checks][0]}).`);
  }
  for (const g of growth || []) {
    console.log(`\nGrowth, ${g.shared ? 'shared' : 'non-shared'} memory64, max ${g.maxPages} pages, ${g.api ?? ''}: create ${g.createMs?.toFixed(2)} ms${g.error ? `, error: ${g.error}` : ''}`);
    const mib = (x) => (typeof x === 'number' && !Number.isNaN(x) ? `, RSS ${x.toFixed(0)} MiB` : '');
    for (const row of g.rows) {
      console.log(row.error ? `  grow to ${row.gib} GiB: ${row.error}` : `  grow to ${row.gib} GiB: ${row.growMs.toFixed(2)} ms${mib(row.rssMiB)}`);
    }
    if (g.touch256MiBMs !== undefined) console.log(`  touch 256 MiB (one byte per 4 KiB): ${g.touch256MiBMs.toFixed(1)} ms${mib(g.rssAfterTouchMiB)}`);
  }
  if (mc) {
    console.log(`\nMemories with a 16 GiB maximum held at once (${mc.shared ? 'shared' : 'non-shared'}, 1 GiB each): ${mc.created} of ${mc.limit}${mc.error ? ` (stopped: ${mc.error})` : ''}`);
    console.log(`maximum 262145 pages: ${mc.maximum262145}`);
  }
}
