// wine-unix.yml runs on the files its build and tests read (M3's retro,
// docs/dev/agent-runs.md). Its `push` and `pull_request` path lists are equal
// and cover:
// - every repository file that runtime/src/wine/*.mjs, tests/wine/*.mjs and
//   tests/browser/wine-*.mjs import, transitively (relative static, dynamic
//   and re-export specifiers);
// - the crates tests/wine/prepare.sh builds (`cargo build ... -p NAME`), with
//   their path dependencies ([dependencies], [build-dependencies]);
// - the repository files that tests/wine/prepare.sh, native/wine/build.sh
//   and the workflow itself name outside comments (scripts, locks, sources),
//   but those NOT_READ_BY_THE_JOB lists with its reason.
// M2's and M3's closes each found files missing from the lists by hand.
// TW_WINE_UNIX_YML names another copy of the workflow to check.
import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
const rel = (p) => relative(root, p).split('\\').join('/');
const isFile = (p) => existsSync(p) && statSync(p).isFile();
const WORKFLOW = process.env.TW_WINE_UNIX_YML ?? join(root, '.github/workflows/wine-unix.yml');

/** The `paths` lists under `on:`, by event. */
function triggerPaths(text) {
  const lines = text.split('\n');
  const start = lines.findIndex((l) => /^on:\s*$/.test(l));
  assert.ok(start >= 0, 'the workflow has an `on:` block');
  const lists = {};
  let event = null;
  let inPaths = false;
  for (const line of lines.slice(start + 1)) {
    if (/^\S/.test(line)) break; // the next top-level key
    if (/^\s*(#.*)?$/.test(line)) continue;
    let m;
    if ((m = /^ {2}([a-z_]+):/.exec(line))) { event = m[1]; inPaths = false; continue; }
    if (/^ {4}paths:\s*$/.test(line)) { inPaths = true; lists[event] = []; continue; }
    if (/^ {4}\S/.test(line)) { inPaths = false; continue; }
    if (inPaths && (m = /^ {6}- "([^"]+)"\s*$/.exec(line))) lists[event].push(m[1]);
  }
  return lists;
}

/** Files the scripts name that the job never reads, each with the reason. */
const NOT_READ_BY_THE_JOB = new Map([
  ['tests/targets/wine-unix.lock', 'only prepare.sh --fetch reads it, and the job builds instead'],
]);

const covers = (pattern, path) => (pattern.endsWith('/**') ? path.startsWith(pattern.slice(0, -2)) : path === pattern);

const SPECIFIER = /\b(?:import|export)\s*(?:[^'"();]*?\bfrom\s*)?['"](\.{1,2}\/[^'"]+)['"]|\bimport\(\s*['"](\.{1,2}\/[^'"]+)['"]\s*\)/g;

/** The repository files `entries` import, transitively, the entries included. */
function importClosure(entries) {
  const seen = new Set();
  const queue = [...entries];
  while (queue.length) {
    const file = queue.pop();
    if (seen.has(file)) continue;
    seen.add(file);
    for (const m of readFileSync(file, 'utf8').matchAll(SPECIFIER)) {
      const target = resolve(dirname(file), m[1] ?? m[2]);
      if (!target.startsWith(root) || !isFile(target)) continue;
      if (/\.(mjs|js)$/.test(target)) queue.push(target);
      else seen.add(target);
    }
  }
  return [...seen].map(rel);
}

const files = (dir, test) => readdirSync(join(root, dir)).filter(test).map((f) => join(root, dir, f));

/** A Cargo.toml's [section] lines. */
function section(toml, name) {
  const out = [];
  let inside = false;
  for (const line of toml.split('\n')) {
    const h = /^\[([^\]]+)\]\s*$/.exec(line);
    if (h) { inside = h[1] === name; continue; }
    if (inside) out.push(line);
  }
  return out;
}

/** The crate directories prepare.sh builds, with their path dependencies. */
function crateClosure() {
  const rootToml = readFileSync(join(root, 'Cargo.toml'), 'utf8');
  const members = JSON.parse(/members\s*=\s*(\[[^\]]*\])/.exec(rootToml)[1]);
  const byName = new Map(members.map((dir) => {
    const name = /^name\s*=\s*"([^"]+)"/m.exec(readFileSync(join(root, dir, 'Cargo.toml'), 'utf8'))[1];
    return [name, dir];
  }));
  const workspacePath = new Map();
  for (const line of section(rootToml, 'workspace.dependencies')) {
    const m = /^([A-Za-z0-9_-]+)\s*=\s*\{[^}]*\bpath\s*=\s*"([^"]+)"/.exec(line);
    if (m) workspacePath.set(m[1], m[2]);
  }
  const prepare = readFileSync(join(root, 'tests/wine/prepare.sh'), 'utf8');
  const wanted = [...prepare.matchAll(/^\s*cargo\s+build\b.*$/gm)].flatMap((m) => [...m[0].matchAll(/-p\s+([A-Za-z0-9_-]+)/g)].map((p) => p[1]));
  assert.ok(wanted.length > 0, 'tests/wine/prepare.sh builds crates with cargo build -p');
  const dirs = new Set();
  const queue = wanted.map((name) => {
    assert.ok(byName.has(name), `prepare.sh builds ${name}, which is no workspace member`);
    return byName.get(name);
  });
  while (queue.length) {
    const dir = queue.pop();
    if (dirs.has(dir)) continue;
    dirs.add(dir);
    const toml = readFileSync(join(root, dir, 'Cargo.toml'), 'utf8');
    for (const line of [...section(toml, 'dependencies'), ...section(toml, 'build-dependencies')]) {
      const name = /^([A-Za-z0-9_-]+)/.exec(line)?.[1];
      if (!name) continue;
      const own = /\bpath\s*=\s*"([^"]+)"/.exec(line);
      const dep = own ? rel(resolve(root, dir, own[1])) : /workspace\s*=\s*true/.test(line) ? workspacePath.get(name) : undefined;
      if (dep) queue.push(dep);
    }
  }
  return [...dirs];
}

/**
 * The repository files a script or workflow names outside its comment lines;
 * with `after`, only in the lines after the first one it matches.
 */
function namedFiles(file, after = null) {
  let lines = readFileSync(file, 'utf8').split('\n');
  if (after) lines = lines.slice(lines.findIndex((l) => after.test(l)) + 1);
  const text = lines.filter((l) => !/^\s*#/.test(l)).join('\n');
  const names = text.match(/(?<![\w./-])(?:scripts|tools|tests|native|crates)\/[A-Za-z0-9_./-]*[A-Za-z0-9_-]/g) ?? [];
  return [...new Set(names)].filter((n) => isFile(join(root, n)));
}

test('wine-unix.yml: the push and pull_request path lists are the same', () => {
  const lists = triggerPaths(readFileSync(WORKFLOW, 'utf8'));
  assert.ok(lists.push?.length, 'on.push has paths');
  assert.deepEqual(lists.pull_request, lists.push);
});

test('wine-unix.yml runs on every file the Wine build and tests read', () => {
  const patterns = triggerPaths(readFileSync(WORKFLOW, 'utf8')).push ?? [];
  const needed = new Map(); // path -> why
  const entries = [
    ...files('runtime/src/wine', (f) => f.endsWith('.mjs')),
    ...files('tests/wine', (f) => f.endsWith('.mjs')),
    ...files('tests/browser', (f) => /^wine-.*\.mjs$/.test(f)),
  ];
  const need = (p, why) => { if (!needed.has(p)) needed.set(p, why); };
  for (const p of importClosure(entries)) need(p, 'imported by the Wine host or its tests');
  for (const dir of crateClosure()) need(`${dir}/Cargo.toml`, `crate ${dir}, built by tests/wine/prepare.sh`);
  const named = [
    ['tests/wine/prepare.sh', namedFiles(join(root, 'tests/wine/prepare.sh'))],
    ['native/wine/build.sh', namedFiles(join(root, 'native/wine/build.sh'))],
    [`${rel(WORKFLOW)}'s jobs`, namedFiles(WORKFLOW, /^jobs:/)],
  ];
  for (const [src, list] of named) {
    for (const p of list) if (!NOT_READ_BY_THE_JOB.has(p)) need(p, `named in ${src}`);
  }
  assert.ok(needed.has('runtime/src/link.mjs'), 'the import walk reaches runtime/src/link.mjs');
  assert.ok(needed.has('crates/x64wasm/Cargo.toml'), 'the crate walk reaches x64wasm through x64wasm-cli');
  assert.ok(needed.has('scripts/emcc.sh'), 'the scripts\' named files include scripts/emcc.sh');
  const missing = [...needed].filter(([p]) => !patterns.some((pat) => covers(pat, p))).map(([p, why]) => `${p} (${why})`);
  assert.deepEqual(missing, [], `add these to both path lists of ${rel(WORKFLOW)}`);
});
