// Whole-image measurements in V8: translate-pe (--entries all) of each PE,
// then per V8 configuration (a child process each): module size, compile time
// (synchronous `new WebAssembly.Module`), instantiate + link time
// (Process.load's link phase: linker, thunks, instantiate, block insertion),
// chunks and blocks. Median of N runs; each compile gets fresh bytes.
//
//   node tests/bench/translator/images.mjs [--runs N] [--configs default,eager,turbofan]
//        [--out DIR] [--no-translate] [EXE ...]
//
// EXEs default to target/targets/7zz.exe and target/targets/coremark.exe
// (scripts/fetch-artifact.sh "$(cat tests/targets/targets.lock)" 7zz.exe target/targets).
// Translations go to DIR (default target/bench/images) as <stem>.wasm and <stem>.plan.json.
import { readFileSync, existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { basename, dirname, join, resolve } from 'node:path';
import { cpus, loadavg } from 'node:os';
import { freshBytes } from './core.mjs';
import { Process } from '../../../runtime/src/process.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../../..');
const args = process.argv.slice(2);
const VALUED = ['--runs', '--configs', '--out', '--child'];
const opt = (name, dflt) => { const i = args.indexOf(name); return i >= 0 ? args[i + 1] : dflt; };
const positional = args.filter((a, i) => !a.startsWith('--') && !VALUED.includes(args[i - 1]));
const runs = Number(opt('--runs', 3));
const out = resolve(opt('--out', join(root, 'target/bench/images')));
const exes = (positional.length ? positional : ['target/targets/7zz.exe', 'target/targets/coremark.exe']).map((p) => resolve(root, p));
const stem = (exe) => basename(exe).replace(/\.exe$/i, '');

const CONFIGS = {
  default: [],
  eager: ['--no-wasm-lazy-compilation'],
  turbofan: ['--no-liftoff', '--no-wasm-lazy-compilation'],
};
const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
const now = () => performance.now();

if (args.includes('--child')) {
  const res = [];
  for (const exe of exes) {
    const wasm = new Uint8Array(readFileSync(join(out, `${stem(exe)}.wasm`)));
    const plan = JSON.parse(readFileSync(join(out, `${stem(exe)}.plan.json`), 'utf8'));
    const bytes = new Uint8Array(readFileSync(exe));
    const compile = [], link = [];
    let info;
    for (let r = 0; r < runs; r++) {
      const fresh = freshBytes(wasm);
      const t0 = now();
      const module = new WebAssembly.Module(fresh);
      compile.push(now() - t0);
      const p = await Process.load({ exe: bytes, plan, aot: module });
      link.push(p.timings.link);
      info = { chunks: p.aot.chunks, blocks: p.aot.blocks };
    }
    res.push({ name: basename(exe), bytes: wasm.length, ...info, compileMs: median(compile), linkMs: median(link) });
  }
  console.log(JSON.stringify(res));
  process.exit(0);
}

if (!args.includes('--no-translate')) {
  for (const exe of exes) {
    if (!existsSync(exe)) { console.error(`missing ${exe}; fetch it with scripts/fetch-artifact.sh "$(cat tests/targets/targets.lock)" ${basename(exe)} target/targets`); process.exit(1); }
    const r = spawnSync('cargo', ['run', '-q', '-p', 'x64wasm-cli', '--release', '--', 'translate-pe', exe, '--out-dir', out, '--entries', 'all'], { cwd: root, encoding: 'utf8' });
    if (r.status !== 0) { console.error(r.stdout, r.stderr); process.exit(1); }
    console.log(r.stdout.trim());
  }
}

console.log(`node ${process.versions.node} (V8 ${process.versions.v8}), ${cpus().length} cpus, load ${loadavg()[0].toFixed(2)}; median of ${runs}`);
const results = {};
for (const name of opt('--configs', Object.keys(CONFIGS).join(',')).split(',')) {
  const r = spawnSync(process.execPath, [...CONFIGS[name], fileURLToPath(import.meta.url), '--child', 'x', '--runs', String(runs), '--out', out, '--no-translate', ...exes], { encoding: 'utf8', maxBuffer: 1 << 26 });
  if (r.status !== 0) { console.error(r.stdout, r.stderr); process.exit(1); }
  results[name] = JSON.parse(r.stdout.trim().split('\n').pop());
}
const names = Object.keys(results);
console.log(`\n| image | wasm bytes | chunks | blocks | ${names.map((n) => `compile ms (${n})`).join(' | ')} | ${names.map((n) => `link ms (${n})`).join(' | ')} |`);
console.log(`|---|---:|---:|---:|${names.map(() => '---:').join('|')}|${names.map(() => '---:').join('|')}|`);
results[names[0]].forEach((x, i) => {
  const per = (k) => names.map((n) => results[n][i][k].toFixed(1)).join(' | ');
  console.log(`| ${x.name} | ${x.bytes} | ${x.chunks} | ${x.blocks} | ${per('compileMs')} | ${per('linkMs')} |`);
});
