// Host files (docs/design/wine-unix.md, section 8, "Host files (M4)"): the
// `files` a Wine process takes, `collect` after it, and `run-wine.mjs --dir`'s
// write-back rules (Phase 1's: new and changed files with mtimes, new
// directories, deletions, file and directory replacing each other, invalid or
// case-colliding names failing at load). The FS tests use the smallest Unix-
// side module (test-link) for an Emscripten FS; the process test needs
// target/wine prepared by tests/wine/prepare.sh.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, readdirSync, rmSync, statSync, utimesSync, existsSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createUnixHost } from '../../runtime/src/wine/unix-host.mjs';
import { writeHostFiles, collectHostFiles } from '../../runtime/src/wine/host-files.mjs';
import { loadHostDir, writeBackHostDir, validWindowsName } from '../../runtime/src/wine/host-dir.mjs';
import { loadWineInputs, wineDirs } from '../../runtime/src/wine/node-files.mjs';
import { createWineProcess } from '../../runtime/src/wine/wine-process.mjs';

const dirs = wineDirs();
const bytes = (s) => new TextEncoder().encode(s);
const text = (b) => new TextDecoder().decode(b);

async function smallFS() {
  const { default: factory } = await import(join(dirs.unix, 'test-link.mjs'));
  const wasm = new WebAssembly.Module(readFileSync(join(dirs.unix, 'test-link.wasm')));
  return (await createUnixHost({ factory, wasm })).FS;
}

const T0 = Date.UTC(2024, 0, 2, 3, 4, 6); // even seconds: the matrix's trees

test('writeHostFiles then collect: contents, sorted order and mtimes round trip', async () => {
  const FS = await smallFS();
  const files = [
    { path: 'work/b.txt', bytes: bytes('bee'), mtime: T0 + 2000 },
    { path: 'work/sub/deeper/c.bin', bytes: new Uint8Array([0, 1, 255]), mtime: T0 + 4000 },
    { path: 'work/empty', dir: true, mtime: T0 + 6000 },
    { path: 'work/sub', dir: true, mtime: T0 + 8000 },
    { path: 'work/sub/deeper', dir: true, mtime: T0 + 10000 },
    { path: 'work', dir: true, mtime: T0 + 12000 },
    { path: 'work/a.txt', bytes: new Uint8Array(0), mtime: T0 + 14000 },
  ];
  writeHostFiles(FS, '/drive', files);
  const got = collectHostFiles(FS, '/drive', 'work');
  assert.deepEqual(got.map((f) => f.path), ['work/a.txt', 'work/b.txt', 'work/empty', 'work/sub', 'work/sub/deeper', 'work/sub/deeper/c.bin']);
  const by = new Map(files.map((f) => [f.path, f]));
  for (const f of got) {
    const want = by.get(f.path);
    assert.equal(!!f.dir, !!want.dir, f.path);
    assert.equal(f.mtime, want.mtime, `${f.path}: mtime survives other files being written into its directory`);
    if (!f.dir) assert.deepEqual(f.bytes, want.bytes, f.path);
  }
  // The tree below `dir`, not `dir` itself, and nothing for a directory that isn't there.
  assert.deepEqual(collectHostFiles(FS, '/drive', 'work/sub').map((f) => f.path), ['work/sub/deeper', 'work/sub/deeper/c.bin']);
  assert.deepEqual(collectHostFiles(FS, '/drive', 'nothing'), []);
  // Collect copies: a later write doesn't change what it returned.
  FS.writeFile('/drive/work/b.txt', bytes('changed'));
  assert.equal(text(got.find((f) => f.path === 'work/b.txt').bytes), 'bee');
});

test('writeHostFiles refuses overwriting, bad paths and non-numeric mtimes', async () => {
  const FS = await smallFS();
  writeHostFiles(FS, '/drive', [{ path: 'x/y', bytes: bytes('1'), mtime: T0 }]);
  assert.throws(() => writeHostFiles(FS, '/drive', [{ path: 'x/y', bytes: bytes('2'), mtime: T0 }]), /already exists/);
  assert.throws(() => writeHostFiles(FS, '/drive', [{ path: 'x/y', dir: true, mtime: T0 }]), /not a directory/);
  for (const bad of ['/abs', 'a\\b', 'a/../b', 'a//b', '']) {
    assert.throws(() => writeHostFiles(FS, '/drive', [{ path: bad, bytes: bytes(''), mtime: T0 }]), /host file path/, JSON.stringify(bad));
  }
  assert.throws(() => writeHostFiles(FS, '/drive', [{ path: 'z', bytes: bytes(''), mtime: 'now' }]), /mtime/);
  assert.throws(() => writeHostFiles(FS, '/drive', [{ path: 'z', bytes: 'text', mtime: T0 }]), /Uint8Array/);
});

function hostTree() {
  const root = mkdtempSync(join(tmpdir(), 'tw-hostdir-'));
  mkdirSync(join(root, 'sub'));
  writeFileSync(join(root, 'keep.txt'), 'keep');
  writeFileSync(join(root, 'change.txt'), 'old');
  writeFileSync(join(root, 'gone.txt'), 'gone');
  writeFileSync(join(root, 'sub', 'inner.txt'), 'inner');
  writeFileSync(join(root, 'becomes-dir'), 'was a file');
  mkdirSync(join(root, 'becomes-file'));
  writeFileSync(join(root, 'becomes-file', 'x'), 'x');
  for (const [p, t] of [['keep.txt', 2], ['change.txt', 4], ['gone.txt', 6], ['sub/inner.txt', 8], ['becomes-dir', 10], ['becomes-file/x', 12], ['becomes-file', 14], ['sub', 16], ['.', 18]]) {
    utimesSync(join(root, p), (T0 + t * 1000) / 1000, (T0 + t * 1000) / 1000);
  }
  return root;
}
const mt = (p) => statSync(p).mtimeMs;

test('loadHostDir: the tree, in sorted order, with the host mtimes', () => {
  const root = hostTree();
  try {
    const list = loadHostDir(root, 'work');
    assert.deepEqual(list.map((f) => f.path), ['work', 'work/becomes-dir', 'work/becomes-file', 'work/becomes-file/x', 'work/change.txt',
      'work/gone.txt', 'work/keep.txt', 'work/sub', 'work/sub/inner.txt']);
    assert.equal(list[0].dir, true);
    assert.equal(list.find((f) => f.path === 'work/keep.txt').mtime, T0 + 2000);
    assert.equal(text(list.find((f) => f.path === 'work/sub/inner.txt').bytes), 'inner');
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('loadHostDir: invalid names, case collisions and other entries fail, naming the file', () => {
  const bad = (setup, re) => {
    const root = mkdtempSync(join(tmpdir(), 'tw-hostdir-'));
    try {
      setup(root);
      assert.throws(() => loadHostDir(root), re);
    } finally { rmSync(root, { recursive: true, force: true }); }
  };
  bad((r) => writeFileSync(join(r, 'a:b'), ''), /a:b is not a valid Windows file name/);
  bad((r) => writeFileSync(join(r, 'what?'), ''), /what\? is not a valid/);
  bad((r) => writeFileSync(join(r, 'NUL.txt'), ''), /NUL\.txt is not a valid/);
  bad((r) => { mkdirSync(join(r, 'd')); writeFileSync(join(r, 'd', 'x\ty'), ''); }, /x\ty is not a valid/);
  bad((r) => { writeFileSync(join(r, 'File.txt'), ''); writeFileSync(join(r, 'file.TXT'), ''); }, /differs from .* only in case/);
  bad((r) => symlinkSync('keep', join(r, 'link')), /link is neither a file nor a directory/);
  assert.ok(validWindowsName('ok name.txt'));
  assert.ok(!validWindowsName(''));
  assert.ok(!validWindowsName('a'.repeat(256)));
});

test('writeBackHostDir: new and changed files with mtimes, new directories, deletions, replacements', () => {
  const root = hostTree();
  try {
    const before = loadHostDir(root, 'work');
    const keepMtime = mt(join(root, 'keep.txt'));
    // What the run left: keep.txt untouched, change.txt rewritten, gone.txt deleted, a new tree, and the two replacements.
    const by = new Map(before.map((f) => [f.path, f]));
    const after = [
      { ...by.get('work/becomes-dir'), bytes: undefined, dir: true, mtime: T0 + 30000 },
      { path: 'work/becomes-dir/child', bytes: bytes('child'), mtime: T0 + 32000 },
      { path: 'work/becomes-file', bytes: bytes('now a file'), mtime: T0 + 34000 },
      { path: 'work/change.txt', bytes: bytes('new contents'), mtime: T0 + 36000 },
      by.get('work/keep.txt'),
      { path: 'work/new', dir: true, mtime: T0 + 38000 },
      { path: 'work/new/file.bin', bytes: new Uint8Array([9, 8, 7]), mtime: T0 + 40000 },
      { path: 'work/new/sub', dir: true, mtime: T0 + 42000 },
      by.get('work/sub'),
      by.get('work/sub/inner.txt'),
    ];
    writeBackHostDir(root, 'work', before, after, T0 + 50000);
    assert.deepEqual(readdirSync(root).sort(), ['becomes-dir', 'becomes-file', 'change.txt', 'keep.txt', 'new', 'sub']);
    assert.equal(readFileSync(join(root, 'change.txt'), 'utf8'), 'new contents');
    assert.equal(mt(join(root, 'change.txt')), T0 + 36000);
    assert.equal(mt(join(root, 'keep.txt')), keepMtime, 'an unchanged file is not touched');
    assert.equal(readFileSync(join(root, 'becomes-file'), 'utf8'), 'now a file');
    assert.equal(statSync(join(root, 'becomes-dir')).isDirectory(), true);
    assert.equal(readFileSync(join(root, 'becomes-dir', 'child'), 'utf8'), 'child');
    assert.equal(mt(join(root, 'becomes-dir')), T0 + 30000);
    assert.deepEqual([...readFileSync(join(root, 'new', 'file.bin'))], [9, 8, 7]);
    assert.equal(mt(join(root, 'new', 'file.bin')), T0 + 40000);
    assert.equal(mt(join(root, 'new')), T0 + 38000, 'a new directory gets its time after its files are written');
    assert.equal(statSync(join(root, 'new', 'sub')).isDirectory(), true);
    assert.equal(mt(join(root, 'sub')), T0 + 16000, 'an untouched directory keeps its time');
    assert.equal(mt(root), T0 + 50000);
    assert.ok(!existsSync(join(root, 'gone.txt')));
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('writeBackHostDir: a case-only rename replaces the host entry; no change leaves the tree alone', () => {
  const root = hostTree();
  try {
    const before = loadHostDir(root, 'work');
    const by = new Map(before.map((f) => [f.path, f]));
    const rootBefore = mt(root);
    writeBackHostDir(root, 'work', before, before.slice(1));
    assert.equal(mt(root), rootBefore);
    assert.equal(mt(join(root, 'sub')), T0 + 16000);
    // keep.txt becomes Keep.TXT (same bytes): the host's old name goes, the new one appears.
    const after = before.slice(1).filter((f) => f.path !== 'work/keep.txt');
    after.push({ ...by.get('work/keep.txt'), path: 'work/Keep.TXT' });
    after.sort((a, b) => (a.path < b.path ? -1 : 1));
    writeBackHostDir(root, 'work', before, after);
    assert.ok(readdirSync(root).includes('Keep.TXT') && !readdirSync(root).includes('keep.txt'));
    assert.equal(readFileSync(join(root, 'Keep.TXT'), 'utf8'), 'keep');
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('a Wine process takes files before boot and collects them after', async () => {
  const inputs = await loadWineInputs(dirs);
  const hello = { name: 'hello.exe', bytes: new Uint8Array(readFileSync(join(dirs.wine, 'hello.exe'))) };
  const files = [
    { path: 'work', dir: true, mtime: T0 },
    { path: 'work/in.txt', bytes: bytes('hello files'), mtime: T0 + 2000 },
    { path: 'work/dir', dir: true, mtime: T0 + 4000 },
  ];
  const out = [];
  const proc = await createWineProcess({ ...inputs, program: hello, cwd: 'C:\\work', files, stdout: (b) => out.push(...b) });
  assert.equal(proc.host.FS.cwd(), `${proc.driveC}/work`);
  assert.equal(await proc.run(), 0);
  assert.ok(out.length > 0, 'hello printed');
  const got = proc.collect('work');
  assert.deepEqual(got.map((f) => [f.path, !!f.dir, f.mtime]), [['work/dir', true, T0 + 4000], ['work/in.txt', false, T0 + 2000]]);
  assert.equal(text(got[1].bytes), 'hello files');
  // The program is still where it was, and a file may not shadow the prefix's own.
  assert.ok(proc.collect('tempwind').some((f) => f.path === 'tempwind/hello.exe'));
  await assert.rejects(createWineProcess({ ...inputs, program: hello, files: [{ path: 'windows/system32/kernel32.dll', bytes: bytes('x'), mtime: T0 }] }), /already exists/);
});
