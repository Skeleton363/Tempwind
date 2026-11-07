// Run every unit of a Wine test executable alone and write the results file
// (docs/design/wine-build.md, "Conformance baselines", schema 1).
//
//   node tests/conformance/run-baseline.mjs --exe PATH --platform windows-2025 --out OUT.json
//        [--launcher wine] [--timeout 120] [--env K=V]... [--trace-dir DIR]
//        [--wine wine-<sha7>] [--commit SHA] [--run URL] [--jobs 1] [--only a,b]
//
// Each unit runs as `[launcher] <exe> <unit>` in a fresh temporary working
// directory with WINETEST_DEBUG=1 (so the summary line prints) and the given
// environment, and is killed (with its process tree) at the time limit.
// --trace-dir keeps stderr of every unit as <exe>__<unit>.log (the +loaddll
// traces the closure is computed from).
import { spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, createWriteStream } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, join, resolve } from 'node:path';
import { parseArgs } from 'node:util';
import { parseList, parseUnit } from './parse.mjs';

const { values: o } = parseArgs({
  options: {
    exe: { type: 'string' }, platform: { type: 'string' }, out: { type: 'string' },
    launcher: { type: 'string', default: '' }, timeout: { type: 'string', default: '120' },
    env: { type: 'string', multiple: true, default: [] }, 'trace-dir': { type: 'string' },
    wine: { type: 'string', default: '' }, commit: { type: 'string', default: process.env.GITHUB_SHA ?? '' },
    run: { type: 'string', default: '' }, jobs: { type: 'string', default: '1' }, only: { type: 'string' },
  },
});
for (const k of ['exe', 'platform', 'out']) if (!o[k]) { console.error(`missing --${k}`); process.exit(2); }

const exe = resolve(o.exe);
const exeName = basename(exe);
const launcher = o.launcher ? o.launcher.split(/\s+/) : [];
const limitMs = Number(o.timeout) * 1000;
const env = { ...process.env, WINETEST_DEBUG: '1' };
for (const kv of o.env) { const i = kv.indexOf('='); env[kv.slice(0, i)] = kv.slice(i + 1); }
const traceDir = o['trace-dir'] ? resolve(o['trace-dir']) : null;
if (traceDir) mkdirSync(traceDir, { recursive: true });

function killTree(child) {
  try {
    if (process.platform === 'win32') spawnSync('taskkill', ['/T', '/F', '/PID', String(child.pid)]);
    else process.kill(-child.pid, 'SIGKILL');
  } catch { /* already gone */ }
}

/** Run a command; resolves {stdout, output (stdout then stderr), exit, timedOut, seconds}. */
function exec(args, cwd, ms, stderrFile) {
  return new Promise((done) => {
    const t0 = performance.now();
    const [cmd, ...rest] = [...launcher, ...args];
    const child = spawn(cmd, rest, { cwd, env, stdio: ['ignore', 'pipe', 'pipe'], detached: process.platform !== 'win32' });
    const chunks = [];
    const err = [];
    const file = stderrFile ? createWriteStream(stderrFile) : null;
    child.stdout.on('data', (d) => chunks.push(d));
    child.stderr.on('data', (d) => { err.push(d); file?.write(d); });
    let timedOut = false;
    const timer = setTimeout(() => { timedOut = true; killTree(child); }, ms);
    const finish = (exit) => {
      clearTimeout(timer);
      file?.end();
      // Leftover children of a finished unit would hold the pipes open; the tree is gone by now.
      done({
        stdout: Buffer.concat(chunks).toString('latin1'),
        output: Buffer.concat([...chunks, ...err]).toString('latin1'),
        exit, timedOut, seconds: (performance.now() - t0) / 1000,
      });
    };
    child.on('error', (e) => { err.push(Buffer.from(String(e))); finish(null); });
    child.on('close', (code) => finish(code));
  });
}

const listed = await exec([exe, '--list'], tmpdir(), 60_000);
// The list is on stdout; under Wine, stderr carries WINEDEBUG traces (parse.mjs, parseList).
const units = parseList(listed.stdout);
if (units.length === 0) {
  console.error(`no units from "${exeName} --list" (exit ${listed.exit}):\n${listed.output.slice(0, 2000)}`);
  process.exit(1);
}
const wanted = o.only ? o.only.split(',') : units;
console.log(`${exeName}: ${units.length} units listed, running ${wanted.length} on ${o.platform}`);

const results = new Array(wanted.length);
let next = 0;
async function worker() {
  while (next < wanted.length) {
    const i = next++;
    const unit = wanted[i];
    const dir = mkdtempSync(join(tmpdir(), 'wt-'));
    const log = traceDir ? join(traceDir, `${exeName}__${unit}.log`) : null;
    const r = await exec([exe, unit], dir, limitMs, log);
    rmSync(dir, { recursive: true, force: true });
    results[i] = parseUnit({ unit, ...r });
    const x = results[i];
    console.log(`${exeName} ${unit}: ${x.status} (${x.executed} executed, ${x.failures} failures, exit ${x.exit}, ${x.seconds}s)`);
  }
}
await Promise.all(Array.from({ length: Math.max(1, Number(o.jobs)) }, worker));

const tally = {};
for (const r of results) tally[r.status] = (tally[r.status] ?? 0) + 1;
console.log(`${exeName} on ${o.platform}:`, JSON.stringify(tally));
writeFileSync(o.out, JSON.stringify({
  schema: 1, platform: o.platform, exe: exeName, wine: o.wine, commit: o.commit, run: o.run, units: results,
}, null, 1) + '\n');
