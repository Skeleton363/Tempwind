#!/usr/bin/env node
// 7-Zip benchmark (tests/sevenzip/README.md, "7z b"): `7zz.exe b 2 -mmt1 -md18` (two passes, single thread,
// 256 KiB dictionary: about 17 s natively) translated and run in Node, and natively under wine64. Prints one
// JSON line: commit, date, Node version, CPU model, the "Tot:" rating (MIPS, compressing and decompressing
// combined) translated and native (medians of --runs runs, default 3) and their ratio. A run without a
// "Tot:" line, or one that reports an error, fails the script.
//
//   node tests/bench/sevenzip.mjs [--exe target/targets/7zz.exe] [--runs 3] [--wine CMD] [--skip-native] [--aot-dir DIR]
//
// Needs x64wasm_fp.wasm and x64wasm_rt.wasm (cargo build -p x64wasm-fp -p x64wasm-rt --release --target
// wasm32-unknown-unknown) and, unless --aot-dir holds 7zz.wasm and 7zz.plan.json, the translator CLI.
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { cpus, tmpdir } from 'node:os';
import { join } from 'node:path';
import { parseArgs } from 'node:util';

const root = new URL('../../', import.meta.url).pathname;
const { values } = parseArgs({
  options: {
    exe: { type: 'string', default: `${root}target/targets/7zz.exe` },
    runs: { type: 'string', default: '3' },
    wine: { type: 'string' },
    'aot-dir': { type: 'string' },
    'skip-native': { type: 'boolean', default: false },
  },
});
const runs = Number(values.runs);
if (!Number.isInteger(runs) || runs < 1) throw new Error(`--runs ${values.runs}: expected a positive integer`);
const exe = values.exe;
if (!existsSync(exe)) throw new Error(`${exe} is missing: scripts/fetch-artifact.sh "$(cat tests/targets/targets.lock)" 7zz.exe target/targets`);

const ARGS = ['b', '2', '-mmt1', '-md18'];
const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
const sh = (cmd, args, opts = {}) => spawnSync(cmd, args, { maxBuffer: 1 << 26, ...opts });

/** The Tot: rating of a `7zz b` output, or an error. */
function parse(r, what) {
  const text = `${r.stdout ?? ''}`.replace(/\r/g, '');
  if (r.status !== 0) throw new Error(`${what}: exit code ${r.status}:\n${text.slice(-1000)}${r.stderr}`.slice(-2500));
  if (/error/i.test(text)) throw new Error(`${what}: the benchmark reports an error:\n${text.slice(-1500)}`);
  const m = /^Tot:\s+(\d+)\s+(\d+)\s+(\d+)\s*$/m.exec(text);
  if (!m) throw new Error(`${what}: no Tot: line:\n${text.slice(-1500)}`);
  return Number(m[3]);
}

// --- translated
let aotDir = values['aot-dir'];
let tmp = null;
if (!aotDir) {
  tmp = mkdtempSync(join(tmpdir(), 'sevenzip-'));
  aotDir = tmp;
}
if (!existsSync(`${aotDir}/7zz.wasm`)) {
  const r = sh('cargo', ['run', '-q', '-p', 'x64wasm-cli', '--release', '--', 'translate-pe', exe, '--out-dir', aotDir], { cwd: root, encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`translate-pe failed:\n${r.stderr}`);
}
const wasm = `${root}target/wasm32-unknown-unknown/release`;
const translated = [];
for (let i = 0; i < runs; i++) {
  const a = ['--exe', exe, '--aot', `${aotDir}/7zz.wasm`, '--plan', `${aotDir}/7zz.plan.json`, '--rt', `${wasm}/x64wasm_rt.wasm`, '--fp', `${wasm}/x64wasm_fp.wasm`];
  const r = sh(process.execPath, [`${root}runtime/src/run-pe.mjs`, ...a, '--', ...ARGS], { encoding: 'utf8', env: { ...process.env, TEMPWIND_TRACE: '' } });
  translated.push(parse(r, `translated run ${i + 1}`));
  process.stderr.write(`translated ${i + 1}/${runs}: Tot rating ${translated[i]} MIPS\n`);
}
if (tmp) rmSync(tmp, { recursive: true, force: true });

// --- native under wine64
const native = [];
if (!values['skip-native']) {
  const wine = values.wine ?? process.env.WINE ?? (sh('which', ['wine64']).status === 0 ? 'wine64' : '/usr/lib/wine/wine64');
  for (let i = 0; i < runs; i++) {
    const r = sh(wine, [exe, ...ARGS], { encoding: 'utf8', env: { ...process.env, WINEDEBUG: '-all' } });
    native.push(parse(r, `native run ${i + 1}`));
    process.stderr.write(`native ${i + 1}/${runs}: Tot rating ${native[i]} MIPS\n`);
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
  args: ARGS.join(' '),
  translated_mips: t,
  native_mips: n,
  ratio: n ? Number((t / n).toFixed(4)) : null,
}));
