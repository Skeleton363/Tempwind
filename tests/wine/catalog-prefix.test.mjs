// catalog.mjs --check and prefix.json (wine-build.md, "Layout under
// target/wine"): the check fails when prefix.json is missing or differs from a
// fresh walk of wine-prefix/, and passes when it matches. Runs on a small
// synthetic target/wine; needs no Wine inputs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, symlinkSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { prefixListing, checkPrefix } from './catalog.mjs';

const script = fileURLToPath(new URL('./catalog.mjs', import.meta.url));

function fixture() {
  const dir = mkdtempSync(join(tmpdir(), 'prefix-test-'));
  const p = join(dir, 'wine-prefix');
  mkdirSync(join(p, 'drive_c/windows/system32'), { recursive: true });
  mkdirSync(join(p, 'dosdevices'));
  writeFileSync(join(p, 'system.reg'), 'x');
  writeFileSync(join(p, 'drive_c/windows/system32/kernel32.dll'), 'y');
  symlinkSync('../drive_c', join(p, 'dosdevices/c:'));
  writeFileSync(join(dir, 'wine-prefix.path'), '/home/runner/wine-prefix\n');
  mkdirSync(join(dir, 'catalog'));
  writeFileSync(join(dir, 'catalog/catalog.json'), JSON.stringify({ schema: 1, modules: [] }));
  return dir;
}
const run = (dir) => spawnSync(process.execPath, [script, '--check', '--dir', dir], { encoding: 'utf8' });

test("the listing has readPrefix's values", () => {
  const dir = fixture();
  try {
    const l = prefixListing(dir);
    assert.equal(l.schema, 1);
    assert.equal(l.path, '/home/runner/wine-prefix');
    assert.deepEqual(l.files, ['system.reg']);
    assert.ok(l.dirs.includes('drive_c/windows/system32'));
    assert.deepEqual(l.links, [['dosdevices/c:', '../drive_c']]);
    assert.deepEqual(l.installFiles, ['drive_c/windows/system32/kernel32.dll']);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('--check fails without prefix.json, passes with a fresh one, fails when the prefix changed', () => {
  const dir = fixture();
  try {
    let r = run(dir);
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /prefix\.json is missing/);
    writeFileSync(join(dir, 'prefix.json'), JSON.stringify(prefixListing(dir), null, 2) + '\n');
    assert.deepEqual(checkPrefix(dir), []);
    r = run(dir);
    assert.equal(r.status, 0, r.stderr);
    writeFileSync(join(dir, 'wine-prefix/drive_c/windows/new.dll'), 'z');
    r = run(dir);
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /prefix\.json differs/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
