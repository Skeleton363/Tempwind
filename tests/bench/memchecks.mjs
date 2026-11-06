#!/usr/bin/env node
// Memory-check matrix (docs/design/translator.md, "Memory checks"): coremark.exe and 7zz.exe translated in four
// variants (none, --null-check, --store-check, both) into target/bench/memchecks/<variant>/, then
// tests/bench/coremark.mjs and tests/bench/sevenzip.mjs run on each variant, interleaved: each round runs every
// variant once on each target, and the variant order rotates by one per round (so no variant always runs first).
// Prints one JSON line: median per variant and target, the module sizes and all samples. A run that does not
// validate (the two scripts fail then) fails this script.
//
//   node tests/bench/memchecks.mjs [--rounds 5] [--skip-translate]
//
// Needs target/targets/coremark.exe and 7zz.exe, x64wasm_fp.wasm and x64wasm_rt.wasm, and a release x64wasm-cli.
import { spawnSync } from 'node:child_process';
import { statSync } from 'node:fs';
import { cpus } from 'node:os';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const root = fileURLToPath(new URL('../../', import.meta.url));
const { values } = parseArgs({ options: { rounds: { type: 'string', default: '5' }, 'skip-translate': { type: 'boolean', default: false } } });
const rounds = Number(values.rounds);
if (!Number.isInteger(rounds) || rounds < 1) throw new Error(`--rounds ${values.rounds}: expected a positive integer`);

const VARIANTS = { none: [], null: ['--null-check'], store: ['--store-check'], both: ['--null-check', '--store-check'] };
const names = Object.keys(VARIANTS);
const TARGETS = {
  coremark: { exe: 'target/targets/coremark.exe', script: 'tests/bench/coremark.mjs', key: 'translated_ips' },
  '7z': { exe: 'target/targets/7zz.exe', script: 'tests/bench/sevenzip.mjs', key: 'translated_mips' },
};
const stem = { coremark: 'coremark', '7z': '7zz' };
const dir = (v) => `${root}target/bench/memchecks/${v}`;
const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];

const sizes = {};
for (const v of names) {
  sizes[v] = {};
  for (const [t, cfg] of Object.entries(TARGETS)) {
    if (!values['skip-translate']) {
      const r = spawnSync(`${root}target/release/x64wasm`, ['translate-pe', cfg.exe, '--out-dir', dir(v), '--entries', 'all', ...VARIANTS[v]], { cwd: root, encoding: 'utf8' });
      if (r.status !== 0) throw new Error(`translate-pe ${cfg.exe} (${v}) failed:\n${r.stderr}`);
    }
    sizes[v][t] = statSync(`${dir(v)}/${stem[t]}.wasm`).size;
  }
}

const samples = Object.fromEntries(names.map((v) => [v, { coremark: [], '7z': [] }]));
for (let round = 0; round < rounds; round++) {
  const order = names.map((_, i) => names[(i + round) % names.length]);
  for (const v of order) {
    for (const [t, cfg] of Object.entries(TARGETS)) {
      const r = spawnSync(process.execPath, [cfg.script, '--skip-native', '--runs', '1', '--aot-dir', dir(v)], { cwd: root, encoding: 'utf8', maxBuffer: 1 << 26 });
      if (r.status !== 0) throw new Error(`${cfg.script} (${v}, round ${round}) failed:\n${r.stdout}\n${r.stderr}`);
      samples[v][t].push(JSON.parse(r.stdout.trim().split('\n').pop())[cfg.key]);
      console.error(`round ${round} ${v} ${t}: ${samples[v][t].at(-1)}`);
    }
  }
}

const result = {};
for (const v of names) {
  result[v] = {};
  for (const t of Object.keys(TARGETS)) result[v][t] = { median: median(samples[v][t]), min: Math.min(...samples[v][t]), max: Math.max(...samples[v][t]), wasmBytes: sizes[v][t], samples: samples[v][t] };
}
console.log(JSON.stringify({ date: new Date().toISOString(), node: process.version, cpu: cpus()[0]?.model ?? null, rounds, units: { coremark: 'iterations/s', '7z': 'MIPS' }, variants: result }));
