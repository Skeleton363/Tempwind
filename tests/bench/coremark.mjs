#!/usr/bin/env node
// CoreMark benchmark (docs/design/runtime.md, "Benchmarks (M4)"): coremark.exe translated and run on our
// Wine in Node, and natively under wine64, both with `0x0 0x0 0x66 0` (iterations 0: CoreMark calibrates itself
// to a run of at least 10 s, so a valid run prints "Correct operation validated"). Prints one JSON line:
// commit, date, Node version, CPU model, iterations per second translated and native (medians of
// --runs runs, default 3) and their ratio. A run that isn't validated fails the script.
//
//   node tests/bench/coremark.mjs [--exe target/targets/coremark.exe] [--runs 3] [--wine CMD] [--skip-native]
//
// The translated side runs on our Wine (runtime/src/wine/run-wine.mjs), so the JSON line carries "host": "wine"
// (Phase 1's lines, from the shim, have no host). Needs target/wine and target/wine-unix (tests/wine/prepare.sh,
// TW_WINE_DIR and TW_WINE_UNIX_DIR) with coremark.exe in the catalog.
import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { cpus } from 'node:os';
import { parseArgs } from 'node:util';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
const { values } = parseArgs({
  options: {
    exe: { type: 'string', default: `${root}target/targets/coremark.exe` },
    runs: { type: 'string', default: '3' },
    wine: { type: 'string' },
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

// --- translated: on our Wine, through the Wine host (run-wine.mjs). The program is the catalog's coremark.exe
// (target/wine, tests/wine/prepare.sh): the catalog matches an image by its sha256, so --exe must be those bytes.
const translated = [];
for (let i = 0; i < runs; i++) {
  const r = sh(process.execPath, [`${root}runtime/src/wine/run-wine.mjs`, exe, ...ARGS], { env: { ...process.env, TEMPWIND_TRACE: '' } });
  translated.push(parse(r.stdout, `translated run ${i + 1}`));
  process.stderr.write(`translated ${i + 1}/${runs}: ${translated[i]} iterations/s\n`);
}

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
  host: 'wine',
  runs,
  translated_ips: t,
  native_ips: n,
  ratio: n ? Number((t / n).toFixed(4)) : null,
}));
