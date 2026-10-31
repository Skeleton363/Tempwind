// kernel32: modules and GetProcAddress, the command line, time, sync, TLS,
// memory, the console and the machine profile (kernel32-system.mjs), through
// shim-harness.mjs.
import test from 'node:test';
import assert from 'node:assert/strict';
import { shimEnv, kernel32, PROC_BASE } from './shim-harness.mjs';
import { quoteArg, commandLine, timeFields, fieldsToTime, SYSTEM_DLLS } from '../../runtime/src/win32/kernel32-system.mjs';

const K32 = SYSTEM_DLLS.get('kernel32.dll').base;
const image = (extra = []) => ({ base: 0x1_4000_0000n, size: 0x10_0000n, sizeOfHeaders: 0x400n, sections: extra });

test('module handles: the program, loaded system DLLs, LoadLibrary, unknown DLLs', () => {
  const env = shimEnv({ image: image() });
  assert.equal(kernel32(env, 'GetModuleHandleW', 0), 0x1_4000_0000n);
  assert.equal(kernel32(env, 'GetModuleHandleW', env.putW('KERNEL32')), K32, 'no extension: .dll');
  assert.equal(kernel32(env, 'GetModuleHandleA', env.put('kernel32.dll')), K32);
  assert.equal(kernel32(env, 'GetModuleHandleW', env.putW('psapi.dll')), 0n, 'not loaded yet');
  assert.equal(env.lastError(), 126);
  const psapi = kernel32(env, 'LoadLibraryW', env.putW('Psapi.dll'));
  assert.equal(psapi, SYSTEM_DLLS.get('psapi.dll').base);
  assert.equal(kernel32(env, 'GetModuleHandleW', env.putW('psapi.dll')), psapi);
  assert.equal(kernel32(env, 'LoadLibraryExW', env.putW('C:\\Windows\\System32\\version.dll'), 0, 8), SYSTEM_DLLS.get('version.dll').base);
  assert.equal(kernel32(env, 'LoadLibraryW', env.putW('C:\\other\\version.dll')), 0n);
  assert.equal(env.lastError(), 126);
  assert.equal(kernel32(env, 'LoadLibraryW', env.putW('nosuch.dll')), 0n);
  assert.equal(env.lastError(), 126);
  assert.equal(kernel32(env, 'FreeLibrary', psapi), 1n);
  assert.match(env.stop('kernel32.dll', 'FreeLibrary', 0x1234n).error, /unknown module handle/);
});

test('GetProcAddress: implemented functions get an address, others stop loudly', () => {
  const env = shimEnv({ image: image() });
  const a = kernel32(env, 'GetProcAddress', K32, env.put('SetDefaultDllDirectories'));
  assert.equal(a, PROC_BASE);
  assert.equal(kernel32(env, 'GetProcAddress', K32, env.put('SetDefaultDllDirectories')), a, 'one address per function');
  assert.match(env.stop('kernel32.dll', 'GetProcAddress', K32, env.put('SetThreadSelectedCpuSetMasks')).error,
    /kernel32.dll!SetThreadSelectedCpuSetMasks is not implemented/);
  assert.match(env.stop('kernel32.dll', 'GetProcAddress', K32, 5).error, /ordinal/);
  assert.match(env.stop('kernel32.dll', 'GetProcAddress', 0x1_4000_0000n, env.put('x')).error, /own exports/);
  assert.equal(kernel32(env, 'SetDefaultDllDirectories', 0x1000), 1n);
  assert.equal(kernel32(env, 'SetDefaultDllDirectories', 0x1), 0n);
  assert.equal(env.lastError(), 87);
});

test('GetModuleFileNameW: the program path, truncation', () => {
  const env = shimEnv({ image: image() });
  const buf = env.zeros(128);
  assert.equal(kernel32(env, 'GetModuleFileNameW', 0, buf, 64), 20n);
  assert.equal(env.wstr(buf), 'C:\\tempwind\\prog.exe');
  assert.equal(env.lastError(), 0);
  assert.equal(kernel32(env, 'GetModuleFileNameW', 0, buf, 5), 5n);
  assert.equal(env.wstr(buf), 'C:\\t');
  assert.equal(env.lastError(), 122);
  kernel32(env, 'GetModuleFileNameW', K32, buf, 64);
  assert.equal(env.wstr(buf), 'C:\\Windows\\System32\\kernel32.dll');
});

test('GetCommandLineW: the program path, then the arguments quoted for CommandLineToArgvW', () => {
  assert.equal(quoteArg('a'), 'a');
  assert.equal(quoteArg(''), '""');
  assert.equal(quoteArg('a b'), '"a b"');
  assert.equal(quoteArg('a"b'), '"a\\"b"');
  assert.equal(quoteArg('a\\ b\\'), '"a\\ b\\\\"');
  assert.equal(quoteArg('a\\\\"b'), '"a\\\\\\\\\\"b"');
  assert.equal(commandLine('C:\\tempwind\\7zz.exe', ['a', 'x y.7z']), 'C:\\tempwind\\7zz.exe a "x y.7z"');
  const env = shimEnv({ argv: ['prog.exe', 'l', '-slt', 'a b'] });
  const p = kernel32(env, 'GetCommandLineW');
  assert.equal(env.wstr(p), 'C:\\tempwind\\prog.exe l -slt "a b"');
  assert.equal(kernel32(env, 'GetCommandLineW'), p, 'the same buffer every time');
});

test('time fields: FILETIME <-> calendar, weekday, leap years', () => {
  assert.deepEqual(timeFields(0n), { year: 1601, month: 1, day: 1, hour: 0, minute: 0, second: 0, ms: 0, weekday: 1 });
  const t = 133_486_382_460_000_000n; // 2024-01-02 03:04:06 UTC, a Tuesday
  assert.deepEqual(timeFields(t), { year: 2024, month: 1, day: 2, hour: 3, minute: 4, second: 6, ms: 0, weekday: 2 });
  assert.equal(fieldsToTime({ year: 2024, month: 1, day: 2, hour: 3, minute: 4, second: 6 }), t);
  assert.deepEqual(timeFields(fieldsToTime({ year: 2000, month: 2, day: 29, hour: 23, minute: 59, second: 59, ms: 999 })),
    { year: 2000, month: 2, day: 29, hour: 23, minute: 59, second: 59, ms: 999, weekday: 2 });
  assert.equal(fieldsToTime({ year: 2100, month: 2, day: 29, hour: 0, minute: 0, second: 0 }), null, '2100 is not a leap year');
  assert.equal(fieldsToTime({ year: 2024, month: 13, day: 1, hour: 0, minute: 0, second: 0 }), null);
  for (let d = 0n; d < 1_000_000n; d += 9973n) {
    const ft = d * 864_000_000_000n + 123_4567n;
    assert.equal(fieldsToTime(timeFields(ft)) + 4567n, ft);
  }
});

test('DOS date and time conversions, invalid inputs included', () => {
  const env = shimEnv();
  const ft = env.zeros(8), d = env.zeros(2), t = env.zeros(2);
  assert.equal(kernel32(env, 'DosDateTimeToFileTime', 0x5822, 0x1883, ft), 1n);
  assert.equal(env.mem.u64(ft), 133_486_382_460_000_000n);
  assert.equal(kernel32(env, 'FileTimeToDosDateTime', ft, d, t), 1n);
  assert.deepEqual([env.mem.u16(d), env.mem.u16(t)], [0x5822, 0x1883]);
  assert.equal(kernel32(env, 'DosDateTimeToFileTime', 0x5800, 0, ft), 0n, 'day 0 of month 0');
  assert.equal(env.lastError(), 87);
  assert.equal(kernel32(env, 'DosDateTimeToFileTime', 0x5822, 0xc000, ft), 0n, 'hour 24');
  env.mem.setU64(ft, 100n);
  assert.equal(kernel32(env, 'FileTimeToDosDateTime', ft, d, t), 0n, 'before 1980');
  assert.equal(env.lastError(), 87);
  env.mem.setU64(ft, fieldsToTime({ year: 2108, month: 1, day: 1, hour: 0, minute: 0, second: 0 }));
  assert.equal(kernel32(env, 'FileTimeToDosDateTime', ft, d, t), 0n, 'after 2107');
  env.mem.setU64(ft, 133_486_382_470_000_000n); // :07, rounds down to :06
  kernel32(env, 'FileTimeToDosDateTime', ft, d, t);
  assert.equal(env.mem.u16(t), 0x1883);
});

test('FileTimeToSystemTime, local time is UTC, CompareFileTime', () => {
  const env = shimEnv();
  const ft = env.zeros(8), st = env.zeros(16), lt = env.zeros(8);
  env.mem.setU64(ft, 133_486_382_460_000_000n);
  assert.equal(kernel32(env, 'FileTimeToSystemTime', ft, st), 1n);
  assert.deepEqual(Array.from({ length: 8 }, (_, i) => env.mem.u16(st + 2n * BigInt(i))), [2024, 1, 2, 2, 3, 4, 6, 0]);
  env.mem.setU64(ft, 1n << 63n);
  assert.equal(kernel32(env, 'FileTimeToSystemTime', ft, st), 0n);
  assert.equal(env.lastError(), 87);
  env.mem.setU64(ft, 42n);
  kernel32(env, 'FileTimeToLocalFileTime', ft, lt);
  assert.equal(env.mem.u64(lt), 42n);
  kernel32(env, 'LocalFileTimeToFileTime', lt, ft);
  assert.equal(env.mem.u64(ft), 42n);
  env.mem.setU64(lt, 43n);
  assert.equal(kernel32(env, 'CompareFileTime', ft, lt), 0xffff_ffffn);
  assert.equal(kernel32(env, 'CompareFileTime', lt, ft), 1n);
  assert.equal(kernel32(env, 'CompareFileTime', ft, ft), 0n);
});

test('events and semaphores with one thread: signaled waits, timeouts, deadlocks', () => {
  const env = shimEnv();
  const auto = kernel32(env, 'CreateEventW', 0, 0, 1, 0);
  assert.equal(kernel32(env, 'WaitForSingleObject', auto, 0xffff_ffff), 0n);
  assert.equal(kernel32(env, 'WaitForSingleObject', auto, 0), 0x102n, 'auto-reset');
  assert.match(env.stop('kernel32.dll', 'WaitForSingleObject', auto, 0xffff_ffff).error, /deadlock/);
  const manual = kernel32(env, 'CreateEventW', 0, 1, 0, 0);
  kernel32(env, 'SetEvent', manual);
  assert.equal(kernel32(env, 'WaitForSingleObject', manual, 5), 0n);
  assert.equal(kernel32(env, 'WaitForSingleObject', manual, 5), 0n, 'manual reset stays signaled');
  kernel32(env, 'ResetEvent', manual);
  const sem = kernel32(env, 'CreateSemaphoreW', 0, 1, 2, 0);
  const prev = env.zeros(4);
  assert.equal(kernel32(env, 'ReleaseSemaphore', sem, 1, prev), 1n);
  assert.equal(env.mem.u32(prev), 1);
  assert.equal(kernel32(env, 'ReleaseSemaphore', sem, 1, prev), 0n);
  assert.equal(env.lastError(), 298);
  const arr = env.zeros(16);
  env.mem.setU64(arr, manual);
  env.mem.setU64(arr + 8n, sem);
  assert.equal(kernel32(env, 'WaitForMultipleObjects', 2, arr, 0, 0), 1n, 'the first signaled one');
  assert.equal(kernel32(env, 'WaitForMultipleObjects', 2, arr, 1, 0), 0x102n);
  assert.equal(kernel32(env, 'CreateSemaphoreW', 0, 3, 2, 0), 0n);
  assert.equal(env.lastError(), 87);
  assert.equal(kernel32(env, 'OpenEventW', 0x1f0003, 0, env.putW('x')), 0n);
  assert.equal(env.lastError(), 2);
  assert.match(env.stop('kernel32.dll', 'CreateEventW', 0, 0, 0, env.putW('named')).error, /named objects/);
  assert.equal(kernel32(env, 'CloseHandle', sem), 1n);
  assert.match(env.stop('kernel32.dll', 'SetEvent', sem).error, /unknown handle/);
});

test('TLS slots: alloc, set, get, free', () => {
  const env = shimEnv();
  const i = kernel32(env, 'TlsAlloc');
  assert.equal(kernel32(env, 'TlsSetValue', i, 0x1234), 1n);
  assert.equal(kernel32(env, 'TlsGetValue', i), 0x1234n);
  assert.notEqual(kernel32(env, 'TlsAlloc'), i);
  assert.equal(kernel32(env, 'TlsFree', i), 1n);
  assert.equal(kernel32(env, 'TlsGetValue', i), 0n);
  assert.equal(kernel32(env, 'TlsFree', i), 0n);
  assert.equal(env.lastError(), 87);
});

test('VirtualAlloc and VirtualFree: 64 KiB granularity, zeroed reuse', () => {
  const env = shimEnv({ pages: 256 });
  const a = kernel32(env, 'VirtualAlloc', 0, 0x1_8000, 0x3000, 4);
  assert.equal(a % 0x1_0000n, 0n);
  env.mem.setU32(a, 0xdead);
  const b = kernel32(env, 'VirtualAlloc', 0, 100, 0x1000, 4);
  assert.equal(b, a + 0x2_0000n);
  assert.equal(kernel32(env, 'VirtualFree', a, 0, 0x8000), 1n);
  assert.equal(kernel32(env, 'VirtualFree', a, 0, 0x8000), 0n);
  assert.equal(env.lastError(), 487);
  const c = kernel32(env, 'VirtualAlloc', 0, 0x1_0000, 0x1000, 4);
  assert.equal(c, a, 'released space is reused');
  assert.equal(env.mem.u32(c), 0, 'and zeroed');
  assert.equal(kernel32(env, 'VirtualFree', b, 1, 0x8000), 0n);
  assert.equal(env.lastError(), 87);
  assert.match(env.stop('kernel32.dll', 'VirtualAlloc', 0, 100, 0x2000, 4).error, /flAllocationType/);
  assert.match(env.stop('kernel32.dll', 'VirtualAlloc', 0, 100, 0x1000, 0x40).error, /flProtect/);
});

test('the console: redirected handles, control handlers', () => {
  const env = shimEnv();
  for (const f of ['GetConsoleMode', 'SetConsoleMode', 'GetConsoleScreenBufferInfo']) {
    assert.equal(kernel32(env, f, 0x7n, env.zeros(32)), 0n, f);
    assert.equal(env.lastError(), 6);
  }
  assert.equal(kernel32(env, 'SetConsoleCtrlHandler', 0x5000_0000n, 1), 1n);
  assert.equal(kernel32(env, 'SetConsoleCtrlHandler', 0x5000_0000n, 0), 1n);
  assert.equal(kernel32(env, 'SetConsoleCtrlHandler', 0x5000_0000n, 0), 0n);
  assert.equal(env.lastError(), 87);
});

test('the machine profile: system info, affinity, memory, features, directories', () => {
  const env = shimEnv();
  const si = env.zeros(48);
  kernel32(env, 'GetSystemInfo', si);
  const dv = new DataView(env.mem.read(si, 48).buffer);
  assert.deepEqual([dv.getUint16(0, true), dv.getUint32(4, true), dv.getBigUint64(24, true), dv.getUint32(32, true),
    dv.getUint32(36, true), dv.getUint32(40, true), dv.getUint16(44, true)], [9, 4096, 1n, 1, 8664, 65536, 6]);
  const [pm, sm] = [env.zeros(8), env.zeros(8)];
  assert.equal(kernel32(env, 'GetProcessAffinityMask', -1, pm, sm), 1n);
  assert.deepEqual([env.mem.u64(pm), env.mem.u64(sm)], [1n, 1n]);
  assert.equal(kernel32(env, 'SetProcessAffinityMask', -1, 3), 0n);
  assert.equal(env.lastError(), 87);
  assert.equal(kernel32(env, 'SetProcessAffinityMask', -1, 1), 1n);
  assert.equal(kernel32(env, 'GetActiveProcessorGroupCount'), 1n);
  assert.equal(kernel32(env, 'GetActiveProcessorCount', 0xffff), 1n);
  assert.equal(kernel32(env, 'GetActiveProcessorCount', 1), 0n);
  const ms = env.zeros(64);
  assert.equal(kernel32(env, 'GlobalMemoryStatusEx', ms), 0n, 'dwLength unset');
  env.mem.setU32(ms, 64);
  assert.equal(kernel32(env, 'GlobalMemoryStatusEx', ms), 1n);
  assert.equal(env.mem.u64(ms + 8n), 4n << 30n);
  assert.equal(env.mem.u64(ms + 16n), env.host.heapFree());
  assert.deepEqual([2, 3, 6, 10, 12, 14].map((f) => kernel32(env, 'IsProcessorFeaturePresent', f)), [1n, 1n, 1n, 1n, 1n, 1n]);
  assert.deepEqual([0, 7, 8, 9, 13, 17, 28, 36, 39].map((f) => kernel32(env, 'IsProcessorFeaturePresent', f)), Array(9).fill(0n));
  assert.equal(kernel32(env, 'GetLargePageMinimum'), 0n);
  assert.equal(kernel32(env, 'GetOEMCP'), 437n);
  const buf = env.zeros(64);
  assert.equal(kernel32(env, 'GetWindowsDirectoryW', buf, 32), 10n);
  assert.equal(env.wstr(buf), 'C:\\Windows');
  assert.equal(kernel32(env, 'GetSystemDirectoryW', buf, 5), 20n);
  assert.equal(kernel32(env, 'GetTempPathW', 32, buf), 11n);
  assert.equal(env.wstr(buf), 'C:\\Windows\\');
});

test('GetVersion follows the manifest: Windows 10 values only when it declares Windows 10', () => {
  const plain = shimEnv({ image: image() });
  assert.equal(kernel32(plain, 'GetVersion'), BigInt((9200 << 16) | (2 << 8) | 6));
  const env = shimEnv({ image: image([{ name: '.rsrc', va: 0x10_0000n, mappedSize: 0x100n, characteristics: 0x4000_0040 }]) });
  env.mem.write(0x10_0010n, new TextEncoder().encode('<supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>'));
  assert.equal(kernel32(env, 'GetVersion'), BigInt((26100 << 16) | 10));
});

test('FormatMessageW: system texts, allocated or in a buffer; LocalFree', () => {
  const env = shimEnv();
  const pp = env.zeros(8);
  assert.equal(kernel32(env, 'FormatMessageW', 0x1300, 0, 2, 0, pp, 0, 0), 44n);
  assert.equal(env.wstr(env.mem.u64(pp)), 'The system cannot find the file specified.\r\n');
  assert.equal(kernel32(env, 'LocalFree', env.mem.u64(pp)), 0n);
  assert.match(env.stop('kernel32.dll', 'LocalFree', env.mem.u64(pp)).error, /not a block/);
  const buf = env.zeros(64);
  assert.equal(kernel32(env, 'FormatMessageW', 0x1200, 0, 5, 0, buf, 32, 0), 19n);
  assert.equal(kernel32(env, 'FormatMessageW', 0x1200, 0, 5, 0, buf, 19, 0), 0n);
  assert.equal(env.lastError(), 122);
  assert.match(env.stop('kernel32.dll', 'FormatMessageW', 0x1200, 0, 99999, 0, buf, 32, 0).error, /no message text/);
});

test('process and thread identity, process times', () => {
  const env = shimEnv();
  assert.equal(kernel32(env, 'GetCurrentProcess'), 0xffff_ffff_ffff_ffffn);
  assert.equal(kernel32(env, 'GetCurrentProcessId'), 0x1000n);
  assert.equal(kernel32(env, 'GetCurrentThreadId'), 0x1004n);
  assert.equal(kernel32(env, 'GetThreadId', -2), 0x1004n);
  const t = Array.from({ length: 4 }, () => env.zeros(8));
  assert.equal(kernel32(env, 'GetProcessTimes', -1, ...t), 1n);
  assert.equal(env.mem.u64(t[0]), env.host.startTime);
  assert.equal(env.mem.u64(t[1]), 0n);
  assert.equal(kernel32(env, 'lstrlenW', env.putW('abc')), 3n);
  assert.equal(kernel32(env, 'lstrlenW', 0), 0n);
});
