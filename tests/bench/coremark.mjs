#!/usr/bin/env node
// CoreMark benchmark (docs/design/runtime.md, "Benchmarks (M4)"): coremark.exe translated and run in
// Node, and natively under wine64, both with `0x0 0x0 0x66 0` (iterations 0: CoreMark calibrates itself
// to a run of at least 10 s, so a valid run prints "Correct operation validated"). Prints one JSON line:
// commit, date, Node version, CPU model, iterations per second translated and native (medians of
// --runs runs, default 3) and their ratio. A run that isn't validated fails the script.
//
//   node tests/bench/coremark.mjs [--exe target/targets/coremark.exe] [--runs 3] [--wine CMD] [--skip-native]
//
// Needs x64wasm_fp.wasm (tests/native/prepare.sh) and, unless --aot-dir exists, the translator CLI.
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { cpus, tmpdir } from 'node:os';
import { join } from 'node:path';
import { parseArgs } from 'node:util';

const root = new URL('../../', import.meta.url).pathname;
const { values } = parseArgs({
  options: {
    exe: { type: 'string', default: `${root}target/targets/coremark.exe` },
    runs: { type: 'string', default: '3' },
    wine: { type: 'string' },
    'aot-dir': { type: 'string' },
    'skip-native': { type: 'boolean', default: false },
  },
});
const runs = Number(values.runs);
if (!Number.isInteger(runs) || runs < 1) throw new Error(`--runs ${values.runs}: expected a positive integer`);
const exe = values.exe;
if (!existsSync(exe)) throw new Error(`${exe} is missing: scripts/fetch-artifact.sh "$(cat tests/targets/targets.lock)" coremark.exe target/targets`);

const ARGS = ['0x0', '0x0', '0x66', '0'];
const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
const sh = (cmd, args, opts = {}) => spawnSync(cmd, args, { maxBuffer: 1 << 26, ...opts });

/** Iterations/Sec of a validated CoreMark output, or an error. */
function parse(out, what) {
  const text = out.toString('latin1');
  if (!text.includes('Correct operation validated')) throw new Error(`${what}: run not validated:\n${text.slice(-1500)}`);
  const m = /Iterations\/Sec\s*:\s*([0-9.]+)/.exec(text);
  if (!m) throw new Error(`${what}: no Iterations/Sec line:\n${text.slice(-1500)}`);
  return Number(m[1]);
}

// --- translated
let aotDir = values['aot-dir'];
let tmp = null;
if (!aotDir) {
  tmp = mkdtempSync(join(tmpdir(), 'coremark-'));
  aotDir = tmp;
  const r = sh('cargo', ['run', '-q', '-p', 'x64wasm-cli', '--release', '--', 'translate-pe', exe, '--out-dir', aotDir, '--entries', 'all'], { cwd: root });
  if (r.status !== 0) throw new Error(`translate-pe failed:\n${r.stderr}`);
}
const stem = exe.replace(/^.*\//, '').replace(/\.exe$/, '');
const fp = `${root}target/wasm32-unknown-unknown/release/x64wasm_fp.wasm`;
const rt = `${root}target/wasm32-unknown-unknown/release/x64wasm_rt.wasm`;
const translated = [];
for (let i = 0; i < runs; i++) {
  const a = ['--exe', exe, '--aot', `${aotDir}/${stem}.wasm`, '--plan', `${aotDir}/${stem}.plan.json`, '--fp', fp];
  if (existsSync(rt)) a.push('--rt', rt);
  const r = sh(process.execPath, [`${root}runtime/src/run-pe.mjs`, ...a, '--', ...ARGS], { env: { ...process.env, TEMPWIND_TRACE: '' } });
  translated.push(parse(r.stdout, `translated run ${i + 1}`));
  process.stderr.write(`translated ${i + 1}/${runs}: ${translated[i]} iterations/s\n`);
}
if (tmp) rmSync(tmp, { recursive: true, force: true });

// --- native under wine64
let native = [];
if (!values['skip-native']) {
  const wine = values.wine ?? process.env.WINE ?? (sh('which', ['wine64']).status === 0 ? 'wine64' : '/usr/lib/wine/wine64');
  for (let i = 0; i < runs; i++) {
    const r = sh(wine, [exe, ...ARGS], { env: { ...process.env, WINEDEBUG: '-all' } });
    native.push(parse(r.stdout, `native run ${i + 1}`));
    process.stderr.write(`native ${i + 1}/${runs}: ${native[i]} iterations/s\n`);
  }
}

const git = sh('git', ['rev-parse', '--short=7', 'HEAD'], { cwd: root });
const t = median(translated), n = native.length ? median(native) : null;
console.log(JSON.stringify({
  commit: git.status === 0 ? git.stdout.toString().trim() : null,
  date: new Date().toISOString(),
  node: process.version,
  cpu: cpus()[0]?.model ?? null,
  runs,
  translated_ips: t,
  native_ips: n,
  ratio: n ? Number((t / n).toFixed(4)) : null,
}));
