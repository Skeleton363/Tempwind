#!/usr/bin/env node
// The 7-Zip command matrix (tests/sevenzip/README.md).
//
//   node tests/sevenzip/matrix.mjs --mode native|wine|translated --out OUT [--exe 7zz.exe]
//        [--filter SUBSTRING] [--jobs N] [--wine CMD] [--aot-dir DIR] [--skip-translate] [--quiet]
//
// Steps come from steps.json and are grouped: a group is the steps of one archive (a, t, l, x), run in order in
// its own work directory OUT/work/<group> that holds a fresh copy of the input tree (tree.mjs). Groups are
// independent and run --jobs at a time. Every step is one process, with relative paths in the work directory.
// Results go to OUT/results/<step id>/: stdout, stderr (raw bytes), exitcode, every file the step created in
// the work directory (the archives), and manifest.txt for a step with `extract` (path, size, SHA-256 and
// mtime of the extracted tree, which is then deleted). A `volatile` step (encryption: random salt and IV)
// records only its exit code and manifest. `sameAsTree` steps must extract exactly the input tree. `noMtime` leaves the mtimes out of the manifest
// (bz2 and xz store none, so the extracted file is stamped with the host clock).
// Compare OUT with expected/ using compare.mjs.
import { spawn, spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { cpus } from 'node:os';
import { join, resolve } from 'node:path';
import { parseArgs } from 'node:util';
import { fileURLToPath } from 'node:url';
import { manifest, writeTree } from './tree.mjs';

const here = fileURLToPath(new URL('.', import.meta.url));
const root = resolve(here, '../..');
const { values } = parseArgs({
  options: {
    mode: { type: 'string' },
    out: { type: 'string' },
    exe: { type: 'string', default: join(root, 'target/targets/7zz.exe') },
    filter: { type: 'string' },
    jobs: { type: 'string', default: '1' },
    wine: { type: 'string' },
    'aot-dir': { type: 'string', default: join(root, 'target/sevenzip') },
    'skip-translate': { type: 'boolean', default: false },
    quiet: { type: 'boolean', default: false },
  },
});
const mode = values.mode;
if (!['native', 'wine', 'translated'].includes(mode) || !values.out) {
  console.error('usage: node tests/sevenzip/matrix.mjs --mode native|wine|translated --out OUT [--exe 7zz.exe] [--filter S] [--jobs N]');
  process.exit(2);
}
const exe = resolve(values.exe);
if (!existsSync(exe)) throw new Error(`${exe} is missing: scripts/fetch-artifact.sh "$(cat tests/targets/targets.lock)" 7zz.exe target/targets`);
const jobs = Number(values.jobs);
if (!Number.isInteger(jobs) || jobs < 1) throw new Error(`--jobs ${values.jobs}: expected a positive integer`);
const out = resolve(values.out);
const resultsDir = join(out, 'results');
const workRoot = join(out, 'work');
rmSync(out, { recursive: true, force: true });
mkdirSync(resultsDir, { recursive: true });

const spec = JSON.parse(readFileSync(join(here, 'steps.json'), 'utf8'));
for (const s of spec.steps) {
  if (!s.volatile && !(s.args.includes('-mmt1') && s.args.includes('-bsp0'))) throw new Error(`step ${s.id}: -mmt1 and -bsp0 are required`);
}
const steps = spec.steps.filter((s) => !values.filter || s.id.includes(values.filter) || s.group.includes(values.filter));
if (steps.length === 0) throw new Error(`--filter ${values.filter}: no step matches`);
const groups = new Map();
for (const s of steps) groups.set(s.group, [...(groups.get(s.group) ?? []), s]);

// The environment is the same in every mode: UTC (zip's DOS times are local time), no Wine noise.
const env = { ...process.env, TZ: 'UTC', WINEDEBUG: '-all', TEMPWIND_TRACE: '' };
let wine = null;
if (mode === 'wine') {
  wine = values.wine ?? process.env.WINE ?? (spawnSync('which', ['wine64']).status === 0 ? 'wine64' : '/usr/lib/wine/wine64');
  // Wine creates its prefix on the first run and says so on stderr: do that before the steps.
  const w = spawnSync(wine, [exe, 'i'], { env, timeout: 300000 });
  if (w.status !== 0) throw new Error(`wine warm-up failed (${w.status}): ${w.stderr}`);
}
if (mode === 'translated') {
  const aot = resolve(values['aot-dir']);
  if (!values['skip-translate']) {
    const r = spawnSync('cargo', ['run', '-q', '-p', 'x64wasm-cli', '--release', '--', 'translate-pe', exe, '--out-dir', aot], { cwd: root, encoding: 'utf8' });
    if (r.status !== 0) throw new Error(`translate-pe failed:\n${r.stderr}`);
  }
  values['aot-dir'] = aot;
}
const wasmDir = join(root, 'target/wasm32-unknown-unknown/release');

/** Command and arguments of one step in the work directory. */
function command(args, work) {
  if (mode === 'native') return [exe, args];
  if (mode === 'wine') return [wine, [exe, ...args]];
  const a = values['aot-dir'];
  return [process.execPath, [join(root, 'runtime/src/run-pe.mjs'), '--exe', exe, '--aot', join(a, '7zz.wasm'), '--plan', join(a, '7zz.plan.json'),
    '--rt', join(wasmDir, 'x64wasm_rt.wasm'), '--fp', join(wasmDir, 'x64wasm_fp.wasm'), '--dir', work, '--', ...args]];
}

function run(cmd, args, cwd) {
  return new Promise((done) => {
    const p = spawn(cmd, args, { cwd, env, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
    const o = [], e = [];
    p.stdout.on('data', (d) => o.push(d));
    p.stderr.on('data', (d) => e.push(d));
    const timer = setTimeout(() => p.kill('SIGKILL'), 900000);
    p.on('error', (err) => { clearTimeout(timer); done({ code: -1, stdout: Buffer.alloc(0), stderr: Buffer.from(String(err)) }); });
    p.on('close', (code, sig) => { clearTimeout(timer); done({ code: code ?? (sig ? -2 : -1), stdout: Buffer.concat(o), stderr: Buffer.concat(e) }); });
  });
}

const topFiles = (dir) => new Set(readdirSync(dir).filter((n) => statSync(join(dir, n)).isFile()));
const failures = [];

async function runGroup(group, list) {
  const work = join(workRoot, group);
  mkdirSync(work, { recursive: true });
  writeTree(join(work, 'tree'));
  const treeManifest = manifest(join(work, 'tree'));
  for (const s of list) {
    const t0 = Date.now();
    const before = topFiles(work);
    const [cmd, args] = command(s.args, work);
    const r = await run(cmd, args, work);
    const dir = join(resultsDir, s.id);
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, 'exitcode'), `${r.code}\n`);
    if (!s.volatile) {
      writeFileSync(join(dir, 'stdout'), r.stdout);
      writeFileSync(join(dir, 'stderr'), r.stderr);
      for (const n of topFiles(work)) if (!before.has(n)) copyFileSync(join(work, n), join(dir, n));
    }
    let note = '';
    if (s.extract) {
      const xdir = join(work, s.extract);
      if (existsSync(xdir)) {
        writeFileSync(join(dir, 'manifest.txt'), manifest(xdir, { mtime: !s.noMtime }));
        if (s.sameAsTree && existsSync(join(work, s.sameAsTree))) {
          const got = manifest(join(work, s.sameAsTree));
          if (got !== treeManifest) { note = 'extracted tree differs from the input tree'; failures.push(`${s.id}: ${note}\n--- input\n${treeManifest}--- extracted\n${got}`); }
        } else if (s.sameAsTree) { note = `${s.sameAsTree} is missing`; failures.push(`${s.id}: ${note}`); }
        rmSync(xdir, { recursive: true, force: true });
      } else { note = `${s.extract} was not created`; failures.push(`${s.id}: ${note}`); }
    }
    if (r.code !== 0) failures.push(`${s.id}: exit code ${r.code}\n${r.stdout.toString('latin1').slice(-600)}${r.stderr.toString('latin1').slice(-600)}`);
    if (!values.quiet) console.error(`${r.code === 0 && !note ? 'ok  ' : 'FAIL'} ${s.id} (${Date.now() - t0} ms)${note ? ' ' + note : ''}`);
  }
}

const queue = [...groups.entries()];
const t0 = Date.now();
await Promise.all(Array.from({ length: Math.min(jobs, queue.length) }, async () => {
  for (let g = queue.shift(); g; g = queue.shift()) await runGroup(g[0], g[1]);
}));
rmSync(workRoot, { recursive: true, force: true });
console.error(`${mode}: ${steps.length} steps in ${groups.size} groups, ${((Date.now() - t0) / 1000).toFixed(1)} s, ${failures.length} problems`);
if (failures.length) {
  for (const f of failures.slice(0, 5)) console.error(`problem: ${f}`);
  process.exit(1);
}
