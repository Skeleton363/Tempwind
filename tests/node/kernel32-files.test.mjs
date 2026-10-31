// kernel32 files, directories, find handles and the volume
// (runtime/src/win32/kernel32-files.mjs), through shim-harness.mjs.
import test from 'node:test';
import assert from 'node:assert/strict';
import { shimEnv, kernel32 } from './shim-harness.mjs';
import { fixupMask, matchMask, WIN32_FIND_DATAW_SIZE, NTFS_FLAGS } from '../../runtime/src/win32/kernel32-files.mjs';
import { VOLUME_SERIAL } from '../../runtime/src/win32/vfs.mjs';

const INVALID = 0xffff_ffff_ffff_ffffn;
const GENERIC_READ = 0x8000_0000, GENERIC_WRITE = 0x4000_0000;
const CREATE_NEW = 1, CREATE_ALWAYS = 2, OPEN_EXISTING = 3;
const BACKUP = 0x0200_0000;

function create(env, name, access = GENERIC_READ | GENERIC_WRITE, disposition = CREATE_ALWAYS, share = 1, flags = 0x80) {
  return kernel32(env, 'CreateFileW', env.putW(name), access, share, 0, disposition, flags, 0);
}

function write(env, h, bytes) {
  const buf = env.put(Uint8Array.from(bytes)), n = env.zeros(4);
  const r = kernel32(env, 'WriteFile', h, buf, bytes.length, n, 0);
  return { r, n: env.mem.u32(n) };
}

function read(env, h, count) {
  const buf = env.zeros(count || 1), n = env.zeros(4);
  const r = kernel32(env, 'ReadFile', h, buf, count, n, 0);
  const got = env.mem.u32(n);
  return { r, bytes: [...env.mem.read(buf, got)] };
}

/** FindFirstFileW/FindNextFileW names (or the last error). */
function find(env, pattern) {
  const data = env.zeros(WIN32_FIND_DATAW_SIZE);
  const h = kernel32(env, 'FindFirstFileW', env.putW(pattern), data);
  if (h === INVALID) return env.lastError();
  const names = [env.wstr(data + 44n)];
  while (kernel32(env, 'FindNextFileW', h, data) === 1n) names.push(env.wstr(data + 44n));
  assert.equal(env.lastError(), 18, 'ERROR_NO_MORE_FILES');
  assert.equal(kernel32(env, 'FindClose', h), 1n);
  return names;
}

test('CreateFileW: handles from 0x100 in steps of 4, last errors of each disposition', () => {
  const env = shimEnv();
  const h = create(env, 'a.txt', GENERIC_WRITE, CREATE_NEW);
  assert.equal(h, 0x100n);
  assert.equal(env.lastError(), 0);
  assert.equal(create(env, 'a.txt', GENERIC_READ, CREATE_NEW), INVALID);
  assert.equal(env.lastError(), 80, 'ERROR_FILE_EXISTS');
  assert.equal(create(env, 'A.TXT', GENERIC_WRITE, OPEN_EXISTING, 3), INVALID);
  assert.equal(env.lastError(), 32, 'the first handle shares only reads');
  assert.equal(kernel32(env, 'CloseHandle', h), 1n);
  const h2 = create(env, 'A.TXT', GENERIC_WRITE, CREATE_ALWAYS);
  assert.equal(h2, 0x104n);
  assert.equal(env.lastError(), 183, 'CREATE_ALWAYS on an existing file');
  assert.equal(create(env, 'missing', GENERIC_READ, OPEN_EXISTING), INVALID);
  assert.equal(env.lastError(), 2);
  assert.equal(create(env, 'nodir\\x', GENERIC_READ, CREATE_NEW), INVALID);
  assert.equal(env.lastError(), 3);
  assert.equal(create(env, 'bad?name', GENERIC_READ, CREATE_NEW), INVALID);
  assert.equal(env.lastError(), 123);
  assert.equal(create(env, '', GENERIC_READ, OPEN_EXISTING), INVALID);
  assert.equal(env.lastError(), 3, 'an empty name');
  assert.equal(create(env, 'x', GENERIC_READ, 9), INVALID);
  assert.equal(env.lastError(), 87);
});

test('CreateFileW: unsupported arguments stop loudly', () => {
  const env = shimEnv();
  const name = env.putW('x');
  assert.match(env.stop('kernel32.dll', 'CreateFileW', name, GENERIC_READ, 0, 0, CREATE_NEW, 0x4000_0080, 0).error, /OVERLAPPED/);
  assert.match(env.stop('kernel32.dll', 'CreateFileW', name, GENERIC_READ, 0, 0, CREATE_NEW, 0x0400_0080, 0).error, /DELETE_ON_CLOSE/);
  assert.match(env.stop('kernel32.dll', 'CreateFileW', name, GENERIC_READ, 0, env.zeros(24), CREATE_NEW, 0x80, 0).error, /lpSecurityAttributes/);
  assert.match(env.stop('kernel32.dll', 'CreateFileW', env.putW('\\\\.\\C:'), GENERIC_READ, 3, 0, OPEN_EXISTING, 0, 0).error, /device path/);
});

test('ReadFile, WriteFile, SetFilePointer, GetFileSize, SetEndOfFile', () => {
  const env = shimEnv();
  const h = create(env, 'f.bin');
  assert.deepEqual(write(env, h, [1, 2, 3, 4, 5]), { r: 1n, n: 5 });
  const high = env.zeros(4);
  assert.equal(kernel32(env, 'GetFileSize', h, high), 5n);
  assert.equal(env.mem.u32(high), 0);
  assert.equal(kernel32(env, 'SetFilePointer', h, 1, 0, 0), 1n);
  assert.deepEqual(read(env, h, 3), { r: 1n, bytes: [2, 3, 4] });
  assert.equal(kernel32(env, 'SetFilePointer', h, -2, 0, 1), 2n, 'FILE_CURRENT with a negative distance');
  assert.equal(kernel32(env, 'SetFilePointer', h, 0, 0, 2), 5n, 'FILE_END');
  assert.deepEqual(read(env, h, 4), { r: 1n, bytes: [] }, 'at the end: 0 bytes, TRUE');
  assert.equal(kernel32(env, 'SetFilePointer', h, -6, 0, 2), 0xffff_ffffn);
  assert.equal(env.lastError(), 131, 'ERROR_NEGATIVE_SEEK');
  assert.equal(kernel32(env, 'SetFilePointer', h, 0, 0, 3), 0xffff_ffffn);
  assert.equal(env.lastError(), 87);
  // A 64-bit move: high part 1, low part 0.
  env.mem.setU32(high, 1);
  env.setLastError(99);
  assert.equal(kernel32(env, 'SetFilePointer', h, 0, high, 0), 0n);
  assert.equal(env.mem.u32(high), 1);
  // Low part 0xFFFFFFFF on success sets the last error to 0.
  assert.equal(kernel32(env, 'SetFilePointer', h, -1, 0, 0), 0xffff_ffffn, 'without lpDistanceToMoveHigh, -1 is negative');
  assert.equal(env.lastError(), 131);
  env.mem.setU32(high, 0);
  assert.equal(kernel32(env, 'SetFilePointer', h, 0xffff_ffff, high, 0), 0xffff_ffffn);
  assert.equal(env.lastError(), 0);
  kernel32(env, 'SetFilePointer', h, 2, 0, 0);
  assert.equal(kernel32(env, 'SetEndOfFile', h), 1n);
  assert.equal(kernel32(env, 'GetFileSize', h, 0), 2n);
  assert.deepEqual([...env.vfs.contents(env.vfs.stat('f.bin'))], [1, 2]);
});

test('access rights: reading a write-only handle and writing a read-only one fail with ERROR_ACCESS_DENIED', () => {
  const env = shimEnv();
  const w = create(env, 'f', GENERIC_WRITE, CREATE_NEW, 3);
  assert.equal(read(env, w, 1).r, 0n);
  assert.equal(env.lastError(), 5);
  const r = create(env, 'f', GENERIC_READ, OPEN_EXISTING, 3);
  assert.equal(write(env, r, [1]).r, 0n);
  assert.equal(env.lastError(), 5);
  assert.equal(kernel32(env, 'SetEndOfFile', r), 0n);
  assert.equal(env.lastError(), 5);
});

test('stdin is empty and stdout is captured; unknown handles stop', () => {
  const env = shimEnv();
  assert.deepEqual(read(env, 0x3n, 8), { r: 1n, bytes: [] });
  assert.deepEqual(write(env, 0x7n, [0x68, 0x69]), { r: 1n, n: 2 });
  assert.equal(env.stdout(), 'hi');
  assert.match(env.stop('kernel32.dll', 'ReadFile', 0x200n, env.zeros(4), 1, env.zeros(4), 0).error, /hFile 0x200: unknown handle/);
  assert.match(env.stop('kernel32.dll', 'ReadFile', 0x7n, env.zeros(4), 1, env.zeros(4), 0).error, /not a readable handle/);
  assert.match(env.stop('kernel32.dll', 'ReadFile', 0x3n, env.zeros(4), 1, 0, 0).error, /lpNumberOfBytesRead is NULL/);
  assert.match(env.stop('kernel32.dll', 'CloseHandle', 0x7n).error, /standard handle/);
});

test('SetFileTime and GetFileInformationByHandle', () => {
  const env = shimEnv();
  const h = create(env, 'f');
  write(env, h, [1, 2, 3]);
  const ft = env.zeros(8);
  env.mem.setU64(ft, 133_486_382_460_000_000n);
  assert.equal(kernel32(env, 'SetFileTime', h, 0, 0, ft), 1n);
  write(env, h, [4]);
  const info = env.zeros(52);
  assert.equal(kernel32(env, 'GetFileInformationByHandle', h, info), 1n);
  const dv = new DataView(env.mem.read(info, 52).buffer);
  assert.equal(dv.getUint32(0, true), 0x20);
  assert.equal(dv.getBigUint64(20, true), 133_486_382_460_000_000n, 'writes after SetFileTime keep the time');
  assert.equal(dv.getUint32(28, true), VOLUME_SERIAL);
  assert.equal(dv.getUint32(36, true), 4);
  assert.equal(dv.getUint32(40, true), 1, 'one link');
  // 0 means "unchanged"; a read-only handle may not set times.
  env.mem.setU64(ft, 0n);
  assert.equal(kernel32(env, 'SetFileTime', h, ft, ft, ft), 1n);
  assert.equal(env.vfs.stat('f').mtime, 133_486_382_460_000_000n);
  const r = create(env, 'f', GENERIC_READ, OPEN_EXISTING, 3);
  assert.equal(kernel32(env, 'SetFileTime', r, 0, 0, ft), 0n);
  assert.equal(env.lastError(), 5);
});

test('attributes, delete, directories and moves', () => {
  const env = shimEnv();
  assert.equal(kernel32(env, 'CreateDirectoryW', env.putW('d'), 0), 1n);
  assert.equal(kernel32(env, 'CreateDirectoryW', env.putW('D'), 0), 0n);
  assert.equal(env.lastError(), 183);
  assert.equal(kernel32(env, 'GetFileAttributesW', env.putW('d')), 0x10n);
  assert.equal(kernel32(env, 'GetFileAttributesW', env.putW('nope')), 0xffff_ffffn);
  assert.equal(env.lastError(), 2);
  assert.equal(kernel32(env, 'GetFileAttributesW', env.putW('nope\\x')), 0xffff_ffffn);
  assert.equal(env.lastError(), 3);
  kernel32(env, 'CloseHandle', create(env, 'd\\f'));
  assert.equal(kernel32(env, 'SetFileAttributesW', env.putW('d\\f'), 1), 1n);
  assert.equal(kernel32(env, 'DeleteFileW', env.putW('d\\f')), 0n);
  assert.equal(env.lastError(), 5, 'read-only');
  assert.equal(kernel32(env, 'RemoveDirectoryW', env.putW('d')), 0n);
  assert.equal(env.lastError(), 145);
  assert.equal(kernel32(env, 'SetFileAttributesW', env.putW('d\\f'), 0x80), 1n);
  assert.equal(kernel32(env, 'MoveFileW', env.putW('d\\f'), env.putW('g')), 1n);
  assert.equal(kernel32(env, 'DeleteFileW', env.putW('d\\f')), 0n);
  assert.equal(env.lastError(), 2);
  kernel32(env, 'CloseHandle', create(env, 'h'));
  assert.equal(kernel32(env, 'MoveFileWithProgressW', env.putW('g'), env.putW('h'), 0, 0, 2), 0n);
  assert.equal(env.lastError(), 183);
  assert.equal(kernel32(env, 'MoveFileWithProgressW', env.putW('g'), env.putW('h'), 0, 0, 3), 1n);
  assert.equal(kernel32(env, 'DeleteFileW', env.putW('h')), 1n);
  assert.equal(kernel32(env, 'RemoveDirectoryW', env.putW('d')), 1n);
  assert.match(env.stop('kernel32.dll', 'MoveFileWithProgressW', env.putW('a'), env.putW('b'), 0, 0, 4).error, /dwFlags 0x4/);
});

test('FindFirstFileW: NTFS order, dots, masks, errors', () => {
  const env = shimEnv();
  for (const n of ['b.txt', 'A.c', 'noext', 'c.txt.bak']) kernel32(env, 'CloseHandle', create(env, n));
  kernel32(env, 'CreateDirectoryW', env.putW('sub'), 0);
  assert.deepEqual(find(env, '*'), ['.', '..', 'A.c', 'b.txt', 'c.txt.bak', 'noext', 'sub']);
  assert.deepEqual(find(env, '*.*'), ['.', '..', 'A.c', 'b.txt', 'c.txt.bak', 'noext', 'sub']);
  assert.deepEqual(find(env, '*.txt'), ['b.txt']);
  assert.deepEqual(find(env, '*.'), ['.', '..', 'noext', 'sub'], 'names without an extension');
  assert.deepEqual(find(env, '?.c'), ['A.c']);
  assert.deepEqual(find(env, 'B.TXT'), ['b.txt'], 'an exact name');
  assert.deepEqual(find(env, 'sub\\*'), ['.', '..']);
  assert.equal(find(env, 'zz*'), 2);
  assert.equal(find(env, 'nodir\\*'), 3);
  assert.equal(find(env, 'b.txt\\*'), 267, 'a file as the directory');
  assert.equal(find(env, 'sub\\'), 2, 'no file name');
  assert.deepEqual(find(env, 'C:\\*').slice(0, 2), ['tempwind', 'Windows'], 'no dots in the root');
  const data = env.zeros(WIN32_FIND_DATAW_SIZE);
  const h = kernel32(env, 'FindFirstFileW', env.putW('sub'), data);
  assert.equal(env.mem.u32(data), 0x10);
  assert.match(env.stop('kernel32.dll', 'CloseHandle', h).error, /FindClose/);
});

test('find masks follow FindFirstFileExW\'s fixup and NT matching', () => {
  assert.equal(fixupMask('*.*'), '*"*');
  assert.equal(fixupMask('*.'), '<');
  assert.equal(fixupMask('a?b'), 'a>b');
  assert.equal(fixupMask('x.. '), 'x');
  assert.ok(matchMask('noext', '*"*'));
  assert.ok(matchMask('a.b.c', '*"c'));
  assert.ok(!matchMask('abc', '>'));
  assert.ok(matchMask('ab', 'ab>>'), '> matches nothing at the end');
  assert.ok(matchMask('..', '<'), '.. matches as .');
  assert.ok(matchMask('ABC.TXT', '*.txt'), 'case-insensitive');
});

test('FindFirstStreamW: a file has ::$DATA, a directory none', () => {
  const env = shimEnv();
  const h = create(env, 'f');
  write(env, h, [1, 2, 3]);
  const data = env.zeros(600);
  const s = kernel32(env, 'FindFirstStreamW', env.putW('f'), 0, data, 0);
  assert.notEqual(s, INVALID);
  assert.equal(env.mem.u64(data), 3n);
  assert.equal(env.wstr(data + 8n), '::$DATA');
  assert.equal(kernel32(env, 'FindNextStreamW', s, data), 0n);
  assert.equal(env.lastError(), 38);
  assert.equal(kernel32(env, 'FindClose', s), 1n);
  assert.equal(kernel32(env, 'FindFirstStreamW', env.putW('C:\\work'), 0, data, 0), INVALID);
  assert.equal(env.lastError(), 38);
  assert.equal(kernel32(env, 'FindFirstStreamW', env.putW('nope'), 0, data, 0), INVALID);
  assert.equal(env.lastError(), 2);
});

test('the current directory', () => {
  const env = shimEnv();
  const buf = env.zeros(64);
  assert.equal(kernel32(env, 'GetCurrentDirectoryW', 32, buf), 7n);
  assert.equal(env.wstr(buf), 'C:\\work');
  assert.equal(kernel32(env, 'GetCurrentDirectoryW', 7, buf), 8n, 'too small: the size with the NUL');
  kernel32(env, 'CreateDirectoryW', env.putW('Sub'), 0);
  assert.equal(kernel32(env, 'SetCurrentDirectoryW', env.putW('sub\\')), 1n);
  kernel32(env, 'GetCurrentDirectoryW', 32, buf);
  assert.equal(env.wstr(buf), 'C:\\work\\sub', 'as given, without the trailing backslash');
  assert.equal(kernel32(env, 'SetCurrentDirectoryW', env.putW('nope')), 0n);
  assert.equal(env.lastError(), 2);
  assert.equal(kernel32(env, 'SetCurrentDirectoryW', env.putW('..')), 1n);
  assert.equal(env.vfs.cwd, 'C:\\work');
});

test('the volume: drives, free space, volume information', () => {
  const env = shimEnv();
  const buf = env.zeros(64);
  assert.equal(kernel32(env, 'GetLogicalDriveStringsW', 4, buf), 5n);
  assert.equal(kernel32(env, 'GetLogicalDriveStringsW', 32, buf), 4n);
  assert.equal(env.wstr(buf), 'C:\\');
  assert.equal(kernel32(env, 'GetDriveTypeW', env.putW('C:\\')), 3n);
  assert.equal(kernel32(env, 'GetDriveTypeW', env.putW('D:\\')), 1n);
  const [a, b, c] = [env.zeros(8), env.zeros(8), env.zeros(8)];
  assert.equal(kernel32(env, 'GetDiskFreeSpaceExW', env.putW('C:\\work\\'), a, b, c), 1n);
  assert.equal(env.mem.u64(a), 128n << 30n);
  assert.equal(env.mem.u64(b), 256n << 30n);
  assert.equal(kernel32(env, 'GetDiskFreeSpaceExW', env.putW('C:\\nope\\'), a, b, c), 0n);
  assert.equal(env.lastError(), 2);
  const w = Array.from({ length: 4 }, () => env.zeros(4));
  assert.equal(kernel32(env, 'GetDiskFreeSpaceW', env.putW('C:\\'), ...w), 1n);
  assert.deepEqual(w.map((p) => env.mem.u32(p)), [8, 512, 0x200_0000, 0x400_0000]);
  const [serial, maxc, flags, fs] = [env.zeros(4), env.zeros(4), env.zeros(4), env.zeros(32)];
  assert.equal(kernel32(env, 'GetVolumeInformationW', env.putW('C:\\'), buf, 32, serial, maxc, flags, fs, 16), 1n);
  assert.equal(env.wstr(fs), 'NTFS');
  assert.equal(env.mem.u32(serial), VOLUME_SERIAL);
  assert.equal(env.mem.u32(maxc), 255);
  assert.equal(env.mem.u32(flags), NTFS_FLAGS);
  assert.equal(kernel32(env, 'GetVolumeInformationW', env.putW('C:\\'), 0, 0, 0, 0, 0, fs, 4), 0n);
  assert.equal(env.lastError(), 122);
});

test('directory handles: FILE_FLAG_BACKUP_SEMANTICS, and SetFileTime on them', () => {
  const env = shimEnv();
  kernel32(env, 'CreateDirectoryW', env.putW('d'), 0);
  assert.equal(create(env, 'd', GENERIC_WRITE, OPEN_EXISTING, 3, 0), INVALID);
  assert.equal(env.lastError(), 5);
  const h = create(env, 'd', GENERIC_WRITE, OPEN_EXISTING, 3, BACKUP);
  const ft = env.zeros(8);
  env.mem.setU64(ft, 133_485_408_000_000_000n);
  assert.equal(kernel32(env, 'SetFileTime', h, 0, 0, ft), 1n);
  assert.equal(env.vfs.stat('d').mtime, 133_485_408_000_000_000n);
  assert.equal(read(env, h, 1).r, 0n);
  assert.equal(env.lastError(), 1, 'reading a directory');
});
