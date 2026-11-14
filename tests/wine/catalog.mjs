#!/usr/bin/env node
// The AOT catalog and the boot-set manifest of a Wine process
// (docs/design/wine-build.md, "Packaging for the Unix side").
//
//   node tests/wine/catalog.mjs [--all] [--dir target/wine] [--cli target/release/x64wasm] [--jobs N]
//   node tests/wine/catalog.mjs --check [--all] [--dir target/wine]
//
// Build: runs `translate-pe FILE --shared --null-check --store-check` per PE
// file of target/wine (or --dir, or TW_WINE_DIR): the closure's lib/ files
// (from rebased/ where they moved), except apisetschema.dll, plus the test
// programs. Writes catalog/catalog.json, catalog/<file>.wasm and
// catalog/<file>.plan.json, and boot-set.json. By default the catalog holds the
// modules the test programs load (closure.json's "roots"); --all holds the
// whole closure. Needs closure.json, rebased/ (pe-rebase) and the programs.
//
// --check: compiles every module catalog.json lists with the synchronous
// `new WebAssembly.Module` (the translation gate); a failure names the file.
// With --all it also fails if the catalog lacks a closure file.
import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cpus } from 'node:os';
import { fileURLToPath } from 'node:url';
import { join, resolve } from 'node:path';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';

export const PROGRAMS = ['hello.exe', 'l2_int.exe', 'l2_crt.exe', 'l2_float.exe'];
export const OPTIONS = ['--shared', '--null-check', '--store-check'];
const MEMFS_LIB = '/wine/lib/wine/x86_64-windows';
const MEMFS_DATA = '/wine/share/wine';
const SKIP = new Set(['apisetschema.dll']);

const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
const hex = (n) => '0x' + n.toString(16);

/** ImageBase of a PE32+ file. */
export function imageBase(bytes, name) {
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const pe = dv.getUint32(0x3c, true);
  if (bytes[0] !== 0x4d || bytes[1] !== 0x5a || dv.getUint32(pe, true) !== 0x4550) throw new Error(`${name}: not a PE file`);
  return dv.getBigUint64(pe + 24 + 24, true);
}

function parseArgs(argv) {
  const a = { all: false, check: false, dir: process.env.TW_WINE_DIR ?? 'target/wine', cli: 'target/release/x64wasm', jobs: Math.max(1, cpus().length) };
  for (let i = 0; i < argv.length; i++) {
    switch (argv[i]) {
      case '--all': a.all = true; break;
      case '--check': a.check = true; break;
      case '--dir': a.dir = argv[++i]; break;
      case '--cli': a.cli = argv[++i]; break;
      case '--jobs': a.jobs = Number(argv[++i]); break;
      default: throw new Error(`usage: catalog.mjs [--all] [--check] [--dir DIR] [--cli PATH] [--jobs N] (got ${argv[i]})`);
    }
  }
  a.dir = resolve(a.dir);
  a.cli = resolve(a.cli);
  return a;
}

const readJson = (p) => JSON.parse(readFileSync(p, 'utf8'));

/** The catalog's entries and the boot set's files, from target/wine. */
export function plan(dir, all) {
  const closure = readJson(join(dir, 'closure.json'));
  const rebasedPath = join(dir, 'rebased/rebase.json');
  const rebased = new Map((existsSync(rebasedPath) ? readJson(rebasedPath).modules : []).map((m) => [m.path, m]));
  const prefixPath = readFileSync(join(dir, 'wine-prefix.path'), 'utf8').trim().replace(/\/+$/, '');
  const lib = closure.modules.filter((m) => m.path.startsWith('lib/') && !SKIP.has(m.name));
  const wanted = all ? lib : lib.filter((m) => m.roots.some((r) => PROGRAMS.includes(r)));
  if (!all && wanted.length === 0) throw new Error('closure.json: no lib/ module is loaded by the test programs; are hello.exe and the L2 programs among its roots?');
  const files = [];
  for (const m of wanted) {
    const mv = rebased.get(m.path);
    files.push({ name: m.name, memfs: `/wine/${m.path}`, source: mv ? `rebased/${m.path}` : `pe/${m.path}`, base: mv ? BigInt(mv.base) : BigInt(m.base) });
  }
  for (const p of PROGRAMS) files.push({ name: p, memfs: `${prefixPath}/drive_c/tempwind/${p}`, source: p, base: null });
  // Everything the closure lists in lib/ goes into the boot set, translated or not.
  const bootPe = lib.concat(closure.modules.filter((m) => SKIP.has(m.name))).map((m) => {
    const mv = rebased.get(m.path);
    return { path: `/wine/${m.path}`, source: mv ? `rebased/${m.path}` : `pe/${m.path}`, lazy: true };
  });
  const bootData = closure.data_files.map((f) => ({ path: `/wine/${f}`, source: `native/opt/wine-11.0/${f}`, lazy: true }));
  const bootPrograms = PROGRAMS.map((p) => ({ path: `${prefixPath}/drive_c/tempwind/${p}`, source: p, lazy: false }));
  return { closure, files, lib, boot: [...bootPe, ...bootData, ...bootPrograms] };
}

function translate(cli, dir, entry, outDir) {
  const tmp = mkdtempSync(join(outDir, '.tmp-'));
  const src = join(dir, entry.source);
  return new Promise((ok, fail) => {
    const p = spawn(cli, ['translate-pe', src, '--out-dir', tmp, ...OPTIONS], { stdio: ['ignore', 'ignore', 'pipe'] });
    let err = '';
    p.stderr.on('data', (d) => { err += d; });
    p.on('error', fail);
    p.on('close', (code) => {
      try {
        if (code !== 0) throw new Error(`${entry.name}: translate-pe exited ${code}: ${err.split('\n').filter((l) => l.trim()).slice(0, 2).join(' | ')}`);
        const stem = entry.name.replace(/\.[^.]*$/, '');
        renameSync(join(tmp, `${stem}.wasm`), join(outDir, `${entry.name}.wasm`));
        renameSync(join(tmp, `${stem}.plan.json`), join(outDir, `${entry.name}.plan.json`));
        ok();
      } catch (e) { fail(e); } finally { rmSync(tmp, { recursive: true, force: true }); }
    });
  });
}

async function pool(items, jobs, fn) {
  let next = 0;
  const run = async () => { while (next < items.length) await fn(items[next++]); };
  await Promise.all(Array.from({ length: Math.min(jobs, items.length) }, run));
}

async function build(a) {
  const { dir } = a;
  const { closure, files, boot } = plan(dir, a.all);
  for (const f of files) if (!existsSync(join(dir, f.source))) throw new Error(`${f.name}: ${f.source} is missing in ${dir} (run tests/wine/prepare.sh)`);
  if (!existsSync(a.cli)) throw new Error(`${a.cli} is missing (cargo build --release -p x64wasm-cli)`);
  const cliSha = sha256(readFileSync(a.cli));
  let translator = 'unknown';
  try { translator = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(); } catch { /* not a checkout */ }
  const outDir = join(dir, 'catalog');
  mkdirSync(outDir, { recursive: true });
  const modules = [];
  let translated = 0;
  const failures = [];
  await pool(files, a.jobs, async (f) => {
    try {
    const bytes = readFileSync(join(dir, f.source));
    const fileSha = sha256(bytes);
    f.sha256 = fileSha;
    f.base ??= imageBase(bytes, f.name);
    // The translation is reused when the file, the options and the translator binary are the same.
    const key = sha256(`${fileSha} ${cliSha} ${OPTIONS.join(' ')}`);
    const keyPath = join(outDir, `${f.name}.key`);
    const wasm = join(outDir, `${f.name}.wasm`);
    const fresh = existsSync(keyPath) && readFileSync(keyPath, 'utf8') === key && existsSync(wasm) && existsSync(join(outDir, `${f.name}.plan.json`));
    if (!fresh) {
      rmSync(keyPath, { force: true });
      await translate(a.cli, dir, f, outDir);
      writeFileSync(keyPath, key);
      translated++;
    }
    f.wasm_bytes = statSync(wasm).size;
    } catch (e) { failures.push(e.message); }
  });
  if (failures.length) {
    console.error(`catalog: ${failures.length} file(s) failed to translate:\n  ${failures.sort().join('\n  ')}`);
    process.exit(1);
  }
  for (const f of files.sort((x, y) => (x.memfs < y.memfs ? -1 : 1))) {
    modules.push({ path: f.memfs, sha256: f.sha256, base: hex(f.base), wasm: `${f.name}.wasm`, plan: `${f.name}.plan.json`, wasm_bytes: f.wasm_bytes });
  }
  writeFileSync(join(outDir, 'catalog.json'), JSON.stringify({ schema: 1, wine: closure.wine, translator, options: OPTIONS, modules }, null, 2) + '\n');
  // The boot-set manifest: the PE files and data files the host loads lazily, and the programs.
  const entries = boot.map((b) => {
    const bytes = readFileSync(join(dir, b.source));
    return { path: b.path, source: b.source, size: bytes.length, sha256: sha256(bytes), lazy: b.lazy };
  }).sort((x, y) => (x.path < y.path ? -1 : 1));
  writeFileSync(join(dir, 'boot-set.json'), JSON.stringify({ schema: 1, wine: closure.wine, files: entries }, null, 2) + '\n');
  console.log(`catalog: ${modules.length} modules (${translated} translated, ${modules.length - translated} reused), boot-set: ${entries.length} files`);
}

function check(a) {
  const { dir } = a;
  const outDir = join(dir, 'catalog');
  const catalog = readJson(join(outDir, 'catalog.json'));
  const failures = [];
  if (a.all) {
    const have = new Set(catalog.modules.map((m) => m.path));
    for (const f of plan(dir, true).files) if (!have.has(f.memfs)) failures.push(`${f.name}: not in catalog.json`);
  }
  let bytes = 0;
  for (const m of catalog.modules) {
    try {
      const wasm = readFileSync(join(outDir, m.wasm));
      new WebAssembly.Module(wasm);
      bytes += wasm.length;
    } catch (e) {
      failures.push(`${m.wasm}: ${e.message}`);
    }
  }
  if (failures.length) {
    console.error(`catalog --check: ${failures.length} failure(s):\n  ${failures.join('\n  ')}`);
    process.exit(1);
  }
  console.log(`catalog --check: ${catalog.modules.length} modules compile in V8 (${(bytes / 1048576).toFixed(1)} MiB of wasm)`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  const a = parseArgs(process.argv.slice(2));
  if (a.check) check(a);
  else await build(a);
}
