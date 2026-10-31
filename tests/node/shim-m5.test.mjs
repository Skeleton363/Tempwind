// advapi32, oleaut32, user32, the M5 msvcrt functions, the OEM code page,
// and `run-pe --dir` (vfs-host.mjs), through shim-harness.mjs.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, statSync, existsSync, utimesSync, rmSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { shimEnv, kernel32, advapi32, oleaut32, user32, msvcrt, testClock } from './shim-harness.mjs';
import { Vfs, OPEN_EXISTING, CREATE_ALWAYS, GENERIC_WRITE } from '../../runtime/src/win32/vfs.mjs';
import { loadHostDir, writeBack } from '../../runtime/src/vfs-host.mjs';

const HKCU = 0x8000_0001n;

test('registry: empty; opens fail with ERROR_FILE_NOT_FOUND, writes stop', () => {
  const env = shimEnv();
  const out = env.zeros(8);
  env.mem.setU64(out, 0x55n);
  assert.equal(advapi32(env, 'RegOpenKeyExW', HKCU, env.putW('Software\\7-Zip'), 0, 0x20019, out), 2n);
  assert.equal(env.mem.u64(out), 0n);
  assert.equal(advapi32(env, 'RegCloseKey', HKCU), 0n);
  assert.equal(advapi32(env, 'RegEnumKeyExW', HKCU, 0, env.zeros(8), env.zeros(4), 0, 0, 0, 0), 259n);
  assert.match(env.stop('advapi32.dll', 'RegCloseKey', 0x1234n).error, /unknown key/);
  assert.match(env.stop('advapi32.dll', 'RegCreateKeyExW', HKCU, env.putW('x'), 0, 0, 0, 0, 0, out, 0).error, /registry writes/);
  assert.match(env.stop('advapi32.dll', 'RegSetValueExW', HKCU, 0, 0, 1, 0, 0).error, /registry writes/);
});

test('tokens: a normal user; privileges it lacks give ERROR_NOT_ALL_ASSIGNED', () => {
  const env = shimEnv();
  const hp = env.zeros(8);
  assert.equal(advapi32(env, 'OpenProcessToken', -1, 0x20, hp), 1n);
  const tok = env.mem.u64(hp);
  const luid = env.zeros(8);
  assert.equal(advapi32(env, 'LookupPrivilegeValueW', 0, env.putW('SeRestorePrivilege'), luid), 1n);
  assert.equal(env.mem.u64(luid), 18n);
  assert.equal(advapi32(env, 'LookupPrivilegeValueW', 0, env.putW('SeNoSuchPrivilege'), luid), 0n);
  assert.equal(env.lastError(), 1313);
  const tp = env.zeros(16);
  env.mem.setU32(tp, 1);
  env.mem.setU64(tp + 4n, 18n);
  env.mem.setU32(tp + 12n, 2);
  assert.equal(advapi32(env, 'AdjustTokenPrivileges', tok, 0, tp, 0, 0, 0), 1n);
  assert.equal(env.lastError(), 1300);
  env.mem.setU64(tp + 4n, 19n); // SeShutdownPrivilege, held
  assert.equal(advapi32(env, 'AdjustTokenPrivileges', tok, 0, tp, 0, 0, 0), 1n);
  assert.equal(env.lastError(), 0);
  assert.equal(kernel32(env, 'CloseHandle', tok), 1n);
  assert.match(env.stop('advapi32.dll', 'GetFileSecurityW', 0, 0, 0, 0, 0).error, /NT security/);
});

test('SystemFunction036 fills the buffer', () => {
  const env = shimEnv();
  const b = env.zeros(64);
  assert.equal(advapi32(env, 'SystemFunction036', b, 64), 1n);
  assert.ok(env.mem.read(b, 64).some((x) => x !== 0));
});

test('BSTRs: Windows layout, NULL handling, lengths', () => {
  const env = shimEnv();
  const b = oleaut32(env, 'SysAllocString', env.putW('héllo'));
  assert.equal(b % 8n, 0n);
  assert.equal(env.mem.u32(b - 4n), 10, 'byte length prefix');
  assert.equal(env.wstr(b), 'héllo');
  assert.equal(oleaut32(env, 'SysStringLen', b), 5n);
  assert.equal(oleaut32(env, 'SysAllocString', 0), 0n);
  assert.equal(oleaut32(env, 'SysStringLen', 0), 0n);
  const u = oleaut32(env, 'SysAllocStringLen', 0, 3);
  assert.equal(env.mem.u16(u + 6n), 0, 'terminated');
  assert.equal(oleaut32(env, 'SysStringLen', u), 3n);
  const c = oleaut32(env, 'SysAllocStringLen', env.putW('abcdef'), 2);
  assert.equal(env.wstr(c), 'ab');
  oleaut32(env, 'SysFreeString', b);
  oleaut32(env, 'SysFreeString', 0);
  assert.match(env.stop('oleaut32.dll', 'SysFreeString', b).error, /not a live BSTR/);
});

test('VariantClear and VariantCopy: BSTR copies, simple types, unsupported types stop', () => {
  const env = shimEnv();
  const src = env.zeros(24), dst = env.zeros(24);
  env.mem.setU16(src, 8);
  env.mem.setU64(src + 8n, oleaut32(env, 'SysAllocString', env.putW('x.txt')));
  assert.equal(oleaut32(env, 'VariantCopy', dst, src), 0n);
  const copy = env.mem.u64(dst + 8n);
  assert.notEqual(copy, env.mem.u64(src + 8n));
  assert.equal(env.wstr(copy), 'x.txt');
  assert.equal(oleaut32(env, 'VariantClear', dst), 0n);
  assert.equal(env.mem.u16(dst), 0);
  assert.match(env.stop('oleaut32.dll', 'SysFreeString', copy).error, /not a live BSTR/, 'VariantClear freed it');
  env.mem.setU16(src, 21);
  env.mem.setU64(src + 8n, 7n);
  assert.equal(oleaut32(env, 'VariantCopy', dst, src), 0n);
  assert.equal(env.mem.u64(dst + 8n), 7n);
  env.mem.setU16(src, 12);
  assert.match(env.stop('oleaut32.dll', 'VariantCopy', dst, src).error, /type 12/);
});

test('CharUpperW (string or character) and CharPrevExA (ANSI and OEM code pages)', () => {
  const env = shimEnv();
  const s = env.putW('abc-é');
  assert.equal(user32(env, 'CharUpperW', s), s);
  assert.equal(env.wstr(s), 'ABC-É');
  assert.equal(user32(env, 'CharUpperW', 0x61), 0x41n);
  const p = env.put('abc');
  assert.equal(user32(env, 'CharPrevExA', 1, p, p + 2n, 0), p + 1n, 'CP_OEMCP');
  assert.equal(user32(env, 'CharPrevExA', 0, p, p, 0), p);
  assert.match(env.stop('user32.dll', 'CharPrevExA', 932, p, p + 1n, 0).error, /code page 932/);
});

test('the OEM code page (437) in WideCharToMultiByte and MultiByteToWideChar', () => {
  const env = shimEnv();
  const w = env.putW('Ä½─');
  const out = env.zeros(8);
  assert.equal(kernel32(env, 'WideCharToMultiByte', 1, 0, w, 3, out, 8, 0, 0), 3n);
  assert.deepEqual([...env.mem.read(out, 3)], [0x8e, 0xab, 0xc4]);
  const back = env.zeros(8);
  assert.equal(kernel32(env, 'MultiByteToWideChar', 437, 0, out, 3, back, 4), 3n);
  assert.equal(env.wstr(back), 'Ä½─');
  assert.match(env.stop('kernel32.dll', 'MultiByteToWideChar', 65001, 0, out, 3, back, 4).error, /code page 65001/);
});

test('msvcrt: getenv, descriptors, stdin at its end, strstr, wcsstr, wcscmp, strtoul', () => {
  const env = shimEnv();
  assert.equal(msvcrt(env, 'getenv', env.put('PATH')), 0n);
  const iob = msvcrt(env, '__iob_func');
  assert.equal(msvcrt(env, '_fileno', iob + 48n), 1n);
  assert.equal(msvcrt(env, '_get_osfhandle', 2), 0xbn);
  assert.equal(msvcrt(env, '_isatty', 1), 0n);
  assert.equal(msvcrt(env, 'fflush', iob + 48n), 0n);
  assert.equal(msvcrt(env, 'fflush', 0), 0n);
  assert.equal(msvcrt(env, 'feof', iob), 0n);
  assert.equal(msvcrt(env, 'fgetc', iob), 0xffff_ffffn);
  assert.equal(msvcrt(env, 'feof', iob), 0x10n);
  assert.equal(msvcrt(env, 'ferror', iob), 0n);
  const hay = env.put('archive.tar.gz');
  assert.equal(msvcrt(env, 'strstr', hay, env.put('.tar')), hay + 7n);
  assert.equal(msvcrt(env, 'strstr', hay, env.put('zip')), 0n);
  assert.equal(msvcrt(env, 'strstr', hay, env.put('')), hay);
  const wh = env.putW('a:b:c');
  assert.equal(msvcrt(env, 'wcsstr', wh, env.putW(':c')), wh + 6n);
  assert.equal(msvcrt(env, 'wcscmp', env.putW('ab'), env.putW('ac')), 0xffff_ffff_ffff_ffffn);
  assert.equal(msvcrt(env, 'wcscmp', env.putW('ab'), env.putW('ab')), 0n);
  const end = env.zeros(8);
  const num = env.put(' 0x1fz');
  assert.equal(msvcrt(env, 'strtoul', num, end, 0), 0x1fn);
  assert.equal(env.mem.u64(end), num + 5n, 'stops at the z');
  assert.equal(msvcrt(env, 'strtoul', env.put('-5'), 0, 10), 0xffff_fffbn, 'negated, no ERANGE');
  assert.equal(msvcrt(env, 'strtoul', env.put('4294967296'), 0, 10), 0xffff_ffffn);
  assert.equal(env.mem.u32(msvcrt(env, '_errno')), 34);
  assert.match(env.stop('msvcrt.dll', '_isatty', 5).error, /descriptors 0-2/);
});

test('--dir: a host directory loads into C:\\work and writes back', () => {
  const host = mkdtempSync(join(tmpdir(), 'tempwind-vfs-'));
  try {
    mkdirSync(join(host, 'tree', 'sub'), { recursive: true });
    writeFileSync(join(host, 'tree', 'a.txt'), 'hello');
    writeFileSync(join(host, 'old.bin'), 'x');
    utimesSync(join(host, 'tree', 'a.txt'), 1704164646, 1704164646);
    const vfs = new Vfs({ now: testClock() });
    loadHostDir(vfs, host);
    const a = vfs.stat('tree\\A.TXT');
    assert.equal(a.attrs, 0x20);
    assert.equal(a.mtime, 133_486_382_460_000_000n);
    assert.equal(a.ctime, a.mtime);
    assert.equal(vfs.stat('tree\\sub').attrs, 0x10);
    // Changes: a new file, a changed file, a new directory, a deletion.
    const { file } = vfs.open('new.txt', { access: GENERIC_WRITE, share: 0, disposition: CREATE_ALWAYS });
    vfs.write(file, new TextEncoder().encode('new'));
    file.node.mtime = 133_486_382_480_000_000n;
    vfs.close(file);
    const t = vfs.open('tree\\a.txt', { access: GENERIC_WRITE, share: 0, disposition: OPEN_EXISTING }).file;
    vfs.write(t, new TextEncoder().encode('J'));
    vfs.createDirectory('out');
    vfs.deleteFile('old.bin');
    writeBack(vfs, host);
    assert.equal(readFileSync(join(host, 'new.txt'), 'utf8'), 'new');
    assert.equal(statSync(join(host, 'new.txt')).mtimeMs, 1704164648000);
    assert.equal(readFileSync(join(host, 'tree', 'a.txt'), 'utf8'), 'Jello');
    assert.ok(existsSync(join(host, 'out')));
    assert.ok(!existsSync(join(host, 'old.bin')));
    assert.deepEqual(readdirSync(join(host, 'tree')).sort(), ['a.txt', 'sub']);
  } finally {
    rmSync(host, { recursive: true, force: true });
  }
});
