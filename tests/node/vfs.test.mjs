// The in-memory file system (runtime/src/win32/vfs.mjs): Win32 path rules,
// NTFS names and order, errors, share modes, renames.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  Vfs, VfsStop, nameKey, validName, FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY,
  FILE_ATTRIBUTE_HIDDEN, GENERIC_READ, GENERIC_WRITE, DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE,
  CREATE_NEW, CREATE_ALWAYS, OPEN_EXISTING, OPEN_ALWAYS, TRUNCATE_EXISTING, FILE_FLAG_BACKUP_SEMANTICS,
} from '../../runtime/src/win32/vfs.mjs';
import { Win32Error } from '../../runtime/src/win32/errors.mjs';
import { testClock } from './shim-harness.mjs';

const fresh = () => new Vfs({ now: testClock() });
const code = (f) => {
  try { f(); } catch (e) { if (e instanceof Win32Error) return e.code; throw e; }
  return 0;
};

test('GetFullPathNameW rules: relative, rooted, drive-relative, separators, dots and spaces', () => {
  const v = fresh();
  const cases = [
    ['a', 'C:\\work\\a'], ['a\\b\\', 'C:\\work\\a\\b\\'], ['C:\\a\\.', 'C:\\a'], ['C:\\.', 'C:\\'],
    ['C:\\a\\..', 'C:\\'], ['C:\\a\\b\\..', 'C:\\a'], ['..\\..\\..\\x', 'C:\\x'], ['a..\\b', 'C:\\work\\a.\\b'],
    ['a. ', 'C:\\work\\a'], ['\\x', 'C:\\x'], ['C:x', 'C:\\work\\x'], ['C:\\a//b', 'C:\\a\\b'],
    ['./a/./b/../c', 'C:\\work\\a\\c'], ['tree\\*', 'C:\\work\\tree\\*'], ['..', 'C:\\'], ['D:x', 'D:\\x'],
    ['\\\\?\\C:\\a\\..', '\\\\?\\C:\\a\\..'],
  ];
  for (const [inp, out] of cases) assert.equal(v.fullPathName(inp), out, inp);
  assert.equal(v.fullPathName(''), null);
  assert.equal(v.fullPathName('   '), null);
});

test('names: case-insensitive keys, invalid characters, NTFS order', () => {
  assert.equal(nameKey('aBc.Txt'), 'ABC.TXT');
  assert.equal(nameKey('straße'), 'STRAßE', 'a unit whose upper case is two units stays');
  for (const bad of ['a<b', 'a>b', 'a:b', 'a"b', 'a|b', 'a?b', 'a*b', 'a\x01b', '']) assert.equal(validName(bad), false, bad);
  assert.equal(validName('a b.c'), true);
  const v = fresh();
  for (const n of ['b', 'A', '_x', 'a1', 'Z']) v.addFile(`C:\\work\\${n}`, new Uint8Array(0));
  assert.deepEqual(v.stat('C:\\work').sortedChildren().map((n) => n.name), ['A', 'a1', 'b', 'Z', '_x']);
});

test('the volume: system directories, the work directory as the current one', () => {
  const v = fresh();
  for (const p of ['C:\\Windows', 'c:\\windows\\SYSTEM32', 'C:\\work', 'C:\\']) assert.equal(v.stat(p).attrs, FILE_ATTRIBUTE_DIRECTORY, p);
  assert.equal(v.cwd, 'C:\\work');
  assert.equal(v.stat('.'), v.stat('C:\\work'));
});

test('lookups: file versus path not found, other drives, UNC, bad \\\\?\\ components, devices', () => {
  const v = fresh();
  v.addFile('C:\\work\\f', new Uint8Array(1));
  assert.equal(code(() => v.stat('missing')), 2);
  assert.equal(code(() => v.stat('missing\\x')), 3);
  assert.equal(code(() => v.stat('f\\x')), 3, 'a file on the way');
  assert.equal(code(() => v.stat('D:\\x')), 3);
  assert.equal(code(() => v.stat('\\\\server\\share\\x')), 3);
  assert.equal(code(() => v.stat('\\\\?\\UNC\\server\\share')), 3);
  assert.equal(code(() => v.stat('\\\\?\\C:\\work\\..\\work')), 123);
  assert.equal(code(() => v.stat('a*b')), 123);
  assert.equal(code(() => v.stat('f\\')), 123, 'a file with a trailing backslash');
  assert.equal(v.stat('\\\\?\\C:\\WORK\\F').name, 'f');
  assert.throws(() => v.stat('\\\\.\\PhysicalDrive0'), VfsStop);
  assert.throws(() => v.stat('NUL'), VfsStop);
});

test('open: dispositions, last errors, attributes and times', () => {
  const v = fresh();
  const o = (path, disposition, access = GENERIC_READ | GENERIC_WRITE, share = 0, attrs = 0) => v.open(path, { access, share, disposition, attrs });
  assert.equal(code(() => o('x', OPEN_EXISTING)), 2);
  assert.equal(code(() => o('x', TRUNCATE_EXISTING)), 2);
  const a = o('x', CREATE_NEW);
  assert.equal(a.lastError, 0);
  assert.equal(a.file.node.attrs, FILE_ATTRIBUTE_ARCHIVE);
  v.write(a.file, new Uint8Array([1, 2, 3]));
  v.close(a.file);
  assert.equal(code(() => o('x', CREATE_NEW)), 80, 'ERROR_FILE_EXISTS');
  const b = o('X', OPEN_ALWAYS);
  assert.equal(b.lastError, 183, 'OPEN_ALWAYS on an existing file');
  assert.equal(b.file.node.size, 3);
  v.close(b.file);
  const c = o('x', CREATE_ALWAYS, GENERIC_WRITE, 0, FILE_ATTRIBUTE_HIDDEN);
  assert.equal(c.lastError, 183);
  assert.equal(c.file.node.size, 0, 'truncated');
  assert.equal(c.file.node.attrs, FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_ARCHIVE);
  v.close(c.file);
  assert.equal(code(() => o('x', CREATE_ALWAYS)), 5, 'CREATE_ALWAYS without the hidden attribute');
  assert.equal(code(() => o('x', 0)), 87);
  assert.equal(code(() => o('x', 6)), 87);
  assert.equal(code(() => o('nodir\\x', CREATE_NEW)), 3);
  assert.equal(code(() => o('new\\', CREATE_NEW)), 123);
});

test('open: directories need FILE_FLAG_BACKUP_SEMANTICS and an open disposition', () => {
  const v = fresh();
  v.createDirectory('d');
  assert.equal(code(() => v.open('d', { access: GENERIC_READ, share: 3, disposition: OPEN_EXISTING })), 5);
  assert.equal(code(() => v.open('d', { access: GENERIC_READ, share: 3, disposition: CREATE_ALWAYS, attrs: FILE_FLAG_BACKUP_SEMANTICS })), 5);
  const h = v.open('d\\', { access: GENERIC_WRITE, share: 3, disposition: OPEN_EXISTING, attrs: FILE_FLAG_BACKUP_SEMANTICS });
  assert.equal(h.file.node.isDir, true);
});

test('read-only files refuse writes and overwrites', () => {
  const v = fresh();
  v.addFile('r', new Uint8Array(4), { attrs: FILE_ATTRIBUTE_READONLY });
  assert.equal(code(() => v.open('r', { access: GENERIC_WRITE, share: 0, disposition: OPEN_EXISTING })), 5);
  assert.equal(code(() => v.open('r', { access: GENERIC_READ, share: 0, disposition: CREATE_ALWAYS })), 5);
  assert.equal(code(() => v.deleteFile('r')), 5);
  const ok = v.open('r', { access: GENERIC_READ, share: 0, disposition: OPEN_EXISTING });
  assert.equal(ok.lastError, 0);
});

test('share modes are enforced both ways', () => {
  const v = fresh();
  v.addFile('s', new Uint8Array(1));
  const r = v.open('s', { access: GENERIC_READ, share: FILE_SHARE_READ, disposition: OPEN_EXISTING });
  const r2 = v.open('s', { access: GENERIC_READ, share: FILE_SHARE_READ, disposition: OPEN_EXISTING });
  assert.equal(code(() => v.open('s', { access: GENERIC_WRITE, share: FILE_SHARE_READ, disposition: OPEN_EXISTING })), 32);
  assert.equal(code(() => v.open('s', { access: GENERIC_READ, share: FILE_SHARE_WRITE, disposition: OPEN_EXISTING })), 32,
    'the new open must share the existing read');
  assert.equal(code(() => v.deleteFile('s')), 32);
  v.close(r.file);
  v.close(r2.file);
  const w = v.open('s', { access: GENERIC_WRITE | DELETE, share: FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, disposition: OPEN_EXISTING });
  assert.ok(v.open('s', { access: 0x80, share: 0, disposition: OPEN_EXISTING }), 'attribute-only access does not conflict');
  v.close(w.file);
});

test('reads, writes past the end, resizing, and the last-write time', () => {
  const v = fresh();
  const { file } = v.open('f', { access: GENERIC_READ | GENERIC_WRITE, share: 0, disposition: CREATE_NEW });
  const t0 = file.node.mtime;
  file.pos = 4n;
  v.write(file, new Uint8Array([9, 9]));
  assert.deepEqual([...v.contents(file.node)], [0, 0, 0, 0, 9, 9]);
  assert.ok(file.node.mtime > t0);
  file.pos = 2n;
  assert.deepEqual([...v.read(file, 10)], [0, 0, 9, 9]);
  assert.equal(file.pos, 6n);
  assert.deepEqual([...v.read(file, 10)], []);
  v.resize(file.node, 5);
  file.pos = 0n;
  assert.deepEqual([...v.read(file, 10)], [0, 0, 0, 0, 9]);
  file.keepMtime = true;
  const t1 = file.node.mtime;
  v.write(file, new Uint8Array([1]));
  assert.equal(file.node.mtime, t1, 'no automatic update after SetFileTime');
});

test('directories: create, remove, errors, parent times', () => {
  const v = fresh();
  const work = v.stat('C:\\work');
  const t0 = work.mtime;
  v.createDirectory('d');
  assert.ok(work.mtime > t0, 'a new entry updates the parent');
  assert.equal(code(() => v.createDirectory('D')), 183);
  assert.equal(code(() => v.createDirectory('x\\y')), 3);
  v.addFile('d\\f', new Uint8Array(0));
  assert.equal(code(() => v.removeDirectory('d')), 145);
  assert.equal(code(() => v.removeDirectory('d\\f')), 267);
  assert.equal(code(() => v.removeDirectory('nope')), 2);
  assert.equal(code(() => v.removeDirectory('C:\\work')), 32, 'the current directory');
  assert.equal(code(() => v.deleteFile('d')), 5);
  v.deleteFile('d\\F');
  v.removeDirectory('d');
  assert.equal(code(() => v.stat('d')), 2);
});

test('move: renames, replace, collisions, case-only renames, into itself', () => {
  const v = fresh();
  v.addFile('a', new Uint8Array([1]));
  v.addFile('b', new Uint8Array([2]));
  assert.equal(code(() => v.move('a', 'b', false)), 183);
  v.move('a', 'b', true);
  assert.deepEqual([...v.contents(v.stat('b'))], [1]);
  assert.equal(code(() => v.stat('a')), 2);
  v.move('b', 'B', false);
  assert.equal(v.stat('b').name, 'B');
  v.createDirectory('d');
  v.move('B', 'd\\c', false);
  assert.equal(v.stat('d\\c').size, 1);
  assert.equal(code(() => v.move('d', 'd\\e', false)), 32);
  assert.equal(code(() => v.move('nope', 'x', false)), 2);
  v.move('d', 'e', false);
  assert.equal(v.stat('e\\c').size, 1);
});

test('attributes: settable bits only, directories keep theirs', () => {
  const v = fresh();
  v.addFile('f', new Uint8Array(0));
  v.setAttributes('f', 0x80);
  assert.equal(v.stat('f').attrs, 0);
  v.setAttributes('f', 0xffff_ffff);
  assert.equal(v.stat('f').attrs, 0x3127);
  v.createDirectory('d');
  v.setAttributes('d', FILE_ATTRIBUTE_HIDDEN);
  assert.equal(v.stat('d').attrs, FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_HIDDEN);
});
