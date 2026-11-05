#!/usr/bin/env node
// Where the time goes (docs/status/phase-1.md, "WP1 baseline"): CoreMark or `7z b 2 -mmt1 -md18` translated
// with default options (no counters) and run under `node --cpu-prof`. Sums self time per function from
// the .cpuprofile and prints the top N with their share of the samples, then the totals for wasm, JS
// (the shim and runtime) and other (GC, the program/idle pseudo-nodes, native builtins).
//
//   node tests/bench/profile.mjs --target coremark|7z [--top 20] [--exe PATH] [--aot-dir DIR] [--keep]
//
// Needs target/targets/<exe>, x64wasm_fp.wasm and x64wasm_rt.wasm (as coremark.mjs and sevenzip.mjs).
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const root = fileURLToPath(new URL('../../', import.meta.url));
const TARGETS = {
  coremark: { exe: `${root}target/targets/coremark.exe`, args: ['0x0', '0x0', '0x66', '0'] },
  '7z': { exe: `${root}target/targets/7zz.exe`, args: ['b', '2', '-mmt1', '-md18'] },
};

/**
 * Self time per function of a .cpuprofile, with a class for each: 'wasm', 'js' or 'other'.
 * Time between two samples goes to the sample that ends it, as DevTools does.
 * @returns {{rows: {name: string, kind: string, samples: number, ms: number}[], total: number}}
 */
export function selfTimes(profile) {
  const nodes = new Map(profile.nodes.map((n) => [n.id, n]));
  const classify = (cf) => {
    if (cf.url.startsWith('wasm://')) return 'wasm';
    if (cf.url.startsWith('file://') || cf.url.startsWith('/') || cf.url.startsWith('node:')) return 'js';
    return 'other';
  };
  const rows = new Map();
  for (let i = 0; i < profile.samples.length; i++) {
    const cf = nodes.get(profile.samples[i]).callFrame;
    const kind = classify(cf);
    const fn = cf.functionName || '(anonymous)';
    const where = kind === 'js' ? ` ${cf.url.replace(/^file:\/\/.*\/(?=runtime\/)/, '').replace(/^.*\/(?=[^/]+$)/, '')}:${cf.lineNumber + 1}` : '';
    const name = `${fn}${where}`;
    const row = rows.get(name) ?? { name, kind, samples: 0, ms: 0 };
    row.samples++;
    row.ms += (profile.timeDeltas[i] ?? 0) / 1000;
    rows.set(name, row);
  }
  return { rows: [...rows.values()].sort((a, b) => b.samples - a.samples), total: profile.samples.length };
}

export function report({ rows, total }, top) {
  const pct = (n) => `${((100 * n) / total).toFixed(1)}%`.padStart(6);
  const lines = [`top ${top} by self time (${total} samples)`];
  for (const r of rows.slice(0, top)) lines.push(`${pct(r.samples)} ${String(r.ms.toFixed(0)).padStart(7)} ms  ${r.kind.padEnd(5)} ${r.name}`);
  const sums = { wasm: 0, js: 0, other: 0 };
  for (const r of rows) sums[r.kind] += r.samples;
  lines.push(`totals: wasm ${pct(sums.wasm).trim()}, JS ${pct(sums.js).trim()}, other ${pct(sums.other).trim()}`);
  return lines.join('\n');
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const { values } = parseArgs({
    options: {
      target: { type: 'string' }, top: { type: 'string', default: '20' }, exe: { type: 'string' },
      'aot-dir': { type: 'string' }, keep: { type: 'boolean', default: false },
    },
  });
  const t = TARGETS[values.target];
  if (!t) throw new Error('--target coremark|7z');
  const top = Number(values.top);
  if (!Number.isInteger(top) || top < 1) throw new Error(`--top ${values.top}: expected a positive integer`);
  const exe = values.exe ?? t.exe;
  if (!existsSync(exe)) throw new Error(`${exe} is missing: scripts/fetch-artifact.sh "$(cat tests/targets/targets.lock)" ${exe.replace(/^.*\//, '')} target/targets`);
  const tmp = mkdtempSync(join(tmpdir(), 'profile-'));
  try {
    let aotDir = values['aot-dir'];
    const stem = exe.replace(/^.*\//, '').replace(/\.exe$/, '');
    if (!aotDir) {
      aotDir = join(tmp, 'aot');
      const r = spawnSync('cargo', ['run', '-q', '-p', 'x64wasm-cli', '--release', '--', 'translate-pe', exe, '--out-dir', aotDir, '--entries', 'all'], { cwd: root, encoding: 'utf8' });
      if (r.status !== 0) throw new Error(`translate-pe failed:\n${r.stderr}`);
    }
    const wasm = `${root}target/wasm32-unknown-unknown/release`;
    const profDir = join(tmp, 'prof');
    const a = ['--cpu-prof', `--cpu-prof-dir=${profDir}`, `${root}runtime/src/run-pe.mjs`, '--exe', exe, '--aot', `${aotDir}/${stem}.wasm`,
      '--plan', `${aotDir}/${stem}.plan.json`, '--rt', `${wasm}/x64wasm_rt.wasm`, '--fp', `${wasm}/x64wasm_fp.wasm`, '--', ...t.args];
    const r = spawnSync(process.execPath, a, { maxBuffer: 1 << 26, encoding: 'utf8', env: { ...process.env, TEMPWIND_TRACE: '' } });
    process.stderr.write(`run exited ${r.status}\n`);
    const files = existsSync(profDir) ? readdirSync(profDir).filter((f) => f.endsWith('.cpuprofile')) : [];
    if (files.length !== 1) throw new Error(`expected one .cpuprofile in ${profDir}, found ${files.length}\n${r.stderr.slice(-1000)}`);
    console.log(report(selfTimes(JSON.parse(readFileSync(join(profDir, files[0]), 'utf8'))), top));
  } finally {
    if (!values.keep) rmSync(tmp, { recursive: true, force: true });
    else process.stderr.write(`kept ${tmp}\n`);
  }
}
