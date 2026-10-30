// Translator measurements in V8 (docs/design/translator.md, "Measurements").
// First write the modules: cargo run -p testkit --release -- spike --bench --out target/bench/translator
//
//   node tests/bench/translator/bench.mjs [--dir DIR] [--runs N] [--configs a,b]
//       runs every measurement once per V8 configuration (a child process each):
//         default    lazy compilation, Liftoff first, dynamic tier-up to TurboFan
//         eager      --no-wasm-lazy-compilation (Liftoff for every function at compile time)
//         turbofan   --no-liftoff --no-wasm-lazy-compilation (TurboFan for every function at compile time)
//         liftoff    --liftoff-only (never tiers up)
//   node tests/bench/translator/bench.mjs --browser [PATH] [--dir DIR]
//       the recursion-depth probes in headless Chromium, page and Worker
//       (binary: PATH, else $CHROME_PATH, else Playwright's Chromium; V8
//       flags passed with --js-flags did not take effect there)
//
// Each measurement is the best of N runs (default 3). Native times come from
// the manifest (`nativeMs`, measured by testkit on the same machine).
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';
import { cpus, loadavg } from 'node:os';
import { deepest, load, setFp, timeCase } from './core.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../../..');
const args = process.argv.slice(2);
const opt = (name, dflt) => {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] !== undefined && !args[i + 1].startsWith('--') ? args[i + 1] : dflt;
};
const dir = resolve(opt('--dir', join(root, 'target/bench/translator')));
const runs = Number(opt('--runs', 3));

const CONFIGS = {
  default: [],
  eager: ['--no-wasm-lazy-compilation'],
  turbofan: ['--no-liftoff', '--no-wasm-lazy-compilation'],
  liftoff: ['--liftoff-only'],
};

const f1 = (x) => x.toFixed(1).padStart(8);

if (args.includes('--child')) {
  // One configuration: print one JSON object.
  const cases = await load(dir, async (p) => new Uint8Array(readFileSync(p)));
  const fpPath = resolve(dirname(fileURLToPath(import.meta.url)), '../../../target/wasm32-unknown-unknown/release/x64wasm_fp.wasm');
  // Synchronous: an awaited compile never settles under --no-wasm-lazy-compilation.
  setFp(new WebAssembly.Instance(new WebAssembly.Module(readFileSync(fpPath)), {}).exports);
  const out = { regions: [], timed: [], depth: {} };
  for (const [name, c] of Object.entries(cases)) {
    if (!name.startsWith('region_')) continue;
    let best = null;
    for (let i = 0; i < runs; i++) {
      const t = await timeCase(c.spec, c.bytes, 1);
      best = best ? Object.fromEntries(Object.entries(t).map(([k, v]) => [k, Math.min(v, best[k])])) : t;
    }
    out.regions.push({ name, bytes: c.bytes.length, chunks: c.spec.chunks, options: c.spec.options, ...best });
  }
  for (const name of ['sum_loop', 'fib_rec', 'byte_calls', 'dot_product']) {
    const c = cases[name];
    const t = await timeCase(c.spec, c.bytes, Math.max(runs, 5));
    out.timed.push({ name, nativeMs: c.spec.nativeMs, ...t });
  }
  for (const name of ['deep_probe', 'deep_fat']) out.depth[name] = await deepest(cases[name].spec, cases[name].bytes);
  console.log(JSON.stringify(out));
  process.exit(0);
}

if (args.includes('--browser')) {
  await browser();
  process.exit(0);
}

console.log(`node ${process.versions.node} (V8 ${process.versions.v8}), ${cpus().length} cpus, load ${loadavg().map((x) => x.toFixed(2)).join(' ')}`);
const wanted = opt('--configs', Object.keys(CONFIGS).join(',')).split(',');
for (const name of wanted) {
  const flags = CONFIGS[name];
  const r = spawnSync(process.execPath, [...flags, fileURLToPath(import.meta.url), '--child', '--dir', dir, '--runs', String(runs)], {
    encoding: 'utf8', maxBuffer: 1 << 26,
  });
  if (r.status !== 0) {
    console.error(r.stdout, r.stderr);
    process.exit(1);
  }
  const res = JSON.parse(r.stdout.trim().split('\n').pop());
  console.log(`\n== ${name} ${flags.join(' ')} (load ${loadavg()[0].toFixed(2)})`);
  console.log('region (201k insns)    chunks   wasm bytes   compile ms  instantiate ms  first run ms');
  for (const x of res.regions) {
    console.log(`max_function_insns ${String(x.options.maxFunctionInsns).padStart(5)} ${String(x.chunks).padStart(5)} ${String(x.bytes).padStart(12)} ${f1(x.compileMs)}    ${f1(x.instantiateMs)}      ${f1(x.firstRunMs)}`);
  }
  console.log('case          compile ms  first run ms  best run ms  native ms  ratio');
  for (const x of res.timed) {
    console.log(`${x.name.padEnd(12)} ${f1(x.compileMs)}  ${f1(x.firstRunMs)}     ${f1(x.bestRunMs)}   ${f1(x.nativeMs ?? NaN)}  ${(x.bestRunMs / x.nativeMs).toFixed(2)}x`);
  }
  console.log(`deepest recursion without the guard (main thread): deep_rec ${res.depth.deep_probe}, deep_fat ${res.depth.deep_fat}`);
}

async function browser() {
  const { chromium } = await import('@playwright/test');
  const { createServer } = await import('../../browser/server.mjs');
  const path = opt('--browser', process.env.CHROME_PATH || undefined);
  const server = createServer();
  await new Promise((ok) => server.listen(0, '127.0.0.1', ok));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const b = await chromium.launch({ executablePath: path });
  try {
    const page = await b.newPage();
    await page.goto(`${origin}/tests/browser/index.html`);
    const base = '/' + dir.slice(root.length + 1).split('\\').join('/');
    const res = await page.evaluate(async (base) => {
      const core = await import('/tests/bench/translator/core.mjs');
      const read = async (url) => new Uint8Array(await (await fetch(url)).arrayBuffer());
      const inPage = await core.depthProbes(await core.load(base, read));
      const inWorker = await new Promise((ok, fail) => {
        const w = new Worker('/tests/bench/translator/worker.mjs', { type: 'module' });
        w.onmessage = (e) => { w.terminate(); e.data.error ? fail(new Error(e.data.error)) : ok(e.data.depth); };
        w.onerror = (e) => { w.terminate(); fail(new Error(e.message)); };
        w.postMessage({ base });
      });
      return { inPage, inWorker, ua: navigator.userAgent };
    }, base);
    console.log(`${b.version()} (${res.ua})`);
    console.log(`deepest recursion without the guard: page deep_rec ${res.inPage.deep_probe}, deep_fat ${res.inPage.deep_fat}; ` +
      `worker deep_rec ${res.inWorker.deep_probe}, deep_fat ${res.inWorker.deep_fat}`);
  } finally {
    await b.close();
    server.close();
  }
}
