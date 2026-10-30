// kernel32.dll shim functions of the M3 programs (runtime.md, "Shim rules"),
// through the shim harness.
import test from 'node:test';
import assert from 'node:assert/strict';
import { shimEnv, kernel32, TEB } from './shim-harness.mjs';

const BASE = 0x20_0000n;
const sec = (name, va, size, characteristics) => ({ name, va: BASE + va, mappedSize: size, characteristics });
/** An image like a MinGW one: headers, .text (RX), .data (RW), .rdata (R), .bss (RW). */
const IMAGE = {
  base: BASE, size: 0x6000n, sizeOfHeaders: 0x400n,
  sections: [
    sec('.text', 0x1000n, 0x1234n, 0x6000_0020),
    sec('.data', 0x3000n, 0x10n, 0xc000_0040),
    sec('.bss', 0x4000n, 0x80n, 0xc000_0080),
    sec('.rdata', 0x5000n, 0x200n, 0x4000_0040),
  ],
};
const neg = (v) => BigInt.asUintN(64, BigInt(v));

test('GetLastError and SetLastError use TEB.LastErrorValue', () => {
  const env = shimEnv();
  env.mem.setU32(TEB + 0x68n, 0x57);
  assert.equal(kernel32(env, 'GetLastError'), 0x57n);
  kernel32(env, 'SetLastError', 5);
  assert.equal(env.mem.u32(TEB + 0x68n), 5);
});

test('critical sections: initialize, recursive enter/leave, delete; misuse stops', () => {
  const env = shimEnv();
  const cs = env.zeros(40);
  env.mem.fill(cs, 40, 0xcc);
  kernel32(env, 'InitializeCriticalSection', cs);
  assert.equal(env.mem.u64(cs), neg(-1), 'DebugInfo');
  assert.equal(env.mem.u32(cs + 8n), 0xffff_ffff, 'LockCount -1');
  assert.deepEqual([env.mem.u32(cs + 12n), env.mem.u64(cs + 16n), env.mem.u64(cs + 24n), env.mem.u64(cs + 32n)], [0, 0n, 0n, 0n]);
  kernel32(env, 'EnterCriticalSection', cs);
  kernel32(env, 'EnterCriticalSection', cs);
  assert.equal(env.mem.u32(cs + 12n), 2, 'RecursionCount');
  assert.equal(env.mem.u64(cs + 16n), 0x1004n, 'OwningThread');
  kernel32(env, 'LeaveCriticalSection', cs);
  kernel32(env, 'LeaveCriticalSection', cs);
  assert.equal(env.mem.u64(cs + 16n), 0n);
  assert.equal(env.mem.u32(cs + 8n), 0xffff_ffff);
  assert.match(env.stop('kernel32.dll', 'LeaveCriticalSection', cs).error, /is not held/);
  kernel32(env, 'DeleteCriticalSection', cs);
  assert.match(env.stop('kernel32.dll', 'EnterCriticalSection', cs).error, /is not initialized/);
  assert.match(env.stop('kernel32.dll', 'InitializeCriticalSection', 0).error, /bad critical section 0x0/);
});

test('TlsGetValue reads the TEB slots and clears the last error', () => {
  const env = shimEnv();
  env.mem.setU64(TEB + 0x1480n + 8n * 3n, 0xabcdn);
  env.mem.setU32(TEB + 0x68n, 99);
  assert.equal(kernel32(env, 'TlsGetValue', 3), 0xabcdn);
  assert.equal(env.mem.u32(TEB + 0x68n), 0);
  assert.equal(kernel32(env, 'TlsGetValue', 100), 0n, 'no expansion slots yet');
  assert.equal(kernel32(env, 'TlsGetValue', 1088), 0n);
  assert.equal(env.mem.u32(TEB + 0x68n), 87, 'ERROR_INVALID_PARAMETER');
});

test('VirtualQuery and VirtualProtect against the image sections', () => {
  const env = shimEnv({ image: IMAGE });
  const mbi = env.zeros(48);
  const query = (addr) => {
    assert.equal(kernel32(env, 'VirtualQuery', addr, mbi, 48), 48n);
    const m = env.mem;
    return { base: m.u64(mbi), alloc: m.u64(mbi + 8n), allocProtect: m.u32(mbi + 16n), size: m.u64(mbi + 24n), state: m.u32(mbi + 32n), protect: m.u32(mbi + 36n), type: m.u32(mbi + 40n) };
  };
  assert.deepEqual(query(BASE + 0x1500n), { base: BASE + 0x1000n, alloc: BASE, allocProtect: 0x80, size: 0x2000n, state: 0x1000, protect: 0x20, type: 0x100_0000 });
  assert.equal(query(BASE + 0x10n).protect, 0x02, 'headers read-only');
  assert.deepEqual([query(BASE + 0x3008n).size, query(BASE + 0x3008n).protect], [0x2000n, 0x08], '.data and .bss merge into one write-copy region');
  assert.deepEqual([query(BASE + 0x5000n).size, query(BASE + 0x5000n).protect], [0x1000n, 0x02]);
  const old = env.zeros(4);
  assert.equal(kernel32(env, 'VirtualProtect', BASE + 0x5010n, 8, 0x04, old), 1n);
  assert.equal(env.mem.u32(old), 0x02);
  assert.equal(query(BASE + 0x5000n).protect, 0x04);
  assert.equal(kernel32(env, 'VirtualProtect', BASE + 0x5010n, 8, 0x02, old), 1n);
  assert.equal(env.mem.u32(old), 0x04);
  assert.equal(kernel32(env, 'VirtualQuery', BASE, mbi, 47), 0n);
  assert.equal(env.mem.u32(TEB + 0x68n), 24, 'ERROR_BAD_LENGTH');
  assert.match(env.stop('kernel32.dll', 'VirtualQuery', 0x1234_0000, mbi, 48).error, /only the image's pages are supported/);
  assert.match(env.stop('kernel32.dll', 'VirtualProtect', BASE + 0x1000n, 1, 0x04, 0).error, /lpflOldProtect is NULL/);
  assert.match(env.stop('kernel32.dll', 'VirtualProtect', BASE + 0x1000n, 1, 0x104, old).error, /flNewProtect 0x104 is not supported/);
});

test('Sleep returns at once; Sleep(INFINITE) stops', () => {
  const env = shimEnv();
  assert.equal(kernel32(env, 'Sleep', 10), undefined);
  assert.match(env.stop('kernel32.dll', 'Sleep', 0xffff_ffff).error, /INFINITE/);
});

test('QueryPerformanceFrequency and QueryPerformanceCounter: 100 ns ticks, monotonic, NULL stops', () => {
  const env = shimEnv();
  const f = env.zeros(8), c1 = env.zeros(8), c2 = env.zeros(8);
  assert.equal(kernel32(env, 'QueryPerformanceFrequency', f), 1n);
  assert.equal(env.mem.u64(f), 10_000_000n);
  assert.equal(kernel32(env, 'QueryPerformanceCounter', c1), 1n);
  const t0 = process.hrtime.bigint();
  while (process.hrtime.bigint() - t0 < 2_000_000n); // 2 ms
  assert.equal(kernel32(env, 'QueryPerformanceCounter', c2), 1n);
  const a = env.mem.u64(c1), b = env.mem.u64(c2);
  assert.ok(a < 10_000_000n, `first count ${a} is not near the process start`);
  assert.ok(b - a >= 15_000n && b - a < 10_000_000n, `2 ms apart gave ${b - a} ticks`);
  assert.match(env.stop('kernel32.dll', 'QueryPerformanceFrequency', 0).error, /NULL/);
  assert.match(env.stop('kernel32.dll', 'QueryPerformanceCounter', 0).error, /NULL/);
});

test('SetUnhandledExceptionFilter returns the previous filter', () => {
  const env = shimEnv();
  assert.equal(kernel32(env, 'SetUnhandledExceptionFilter', 0x1234), 0n);
  assert.equal(kernel32(env, 'SetUnhandledExceptionFilter', 0x5678), 0x1234n);
});

test('code page 1252: IsDBCSLeadByteEx, MultiByteToWideChar, WideCharToMultiByte', () => {
  const env = shimEnv();
  assert.equal(kernel32(env, 'IsDBCSLeadByteEx', 0, 0x81), 0n);
  assert.equal(kernel32(env, 'IsDBCSLeadByteEx', 1252, 0xe0), 0n);
  assert.match(env.stop('kernel32.dll', 'IsDBCSLeadByteEx', 932, 0x81).error, /code page 932 is not supported/);

  const src = env.put(new Uint8Array([0x41, 0x80, 0x81, 0xe9, 0x9f, 0]));
  assert.equal(kernel32(env, 'MultiByteToWideChar', 0, 0, src, -1, 0, 0), 6n);
  const w = env.zeros(12);
  assert.equal(kernel32(env, 'MultiByteToWideChar', 0, 1, src, -1, w, 6), 6n);
  assert.deepEqual(Array.from({ length: 6 }, (_, i) => env.mem.u16(w + 2n * BigInt(i))), [0x41, 0x20ac, 0x81, 0xe9, 0x178, 0]);
  assert.equal(kernel32(env, 'MultiByteToWideChar', 1252, 0, src, 2, w, 1), 0n);
  assert.equal(env.mem.u32(TEB + 0x68n), 122, 'ERROR_INSUFFICIENT_BUFFER');
  assert.equal(kernel32(env, 'MultiByteToWideChar', 0, 0, src, 0, w, 6), 0n);
  assert.equal(env.mem.u32(TEB + 0x68n), 87);

  const ws = env.putW('A€é');
  assert.equal(kernel32(env, 'WideCharToMultiByte', 0, 0, ws, -1, 0, 0, 0, 0), 4n);
  const out = env.zeros(4);
  assert.equal(kernel32(env, 'WideCharToMultiByte', 0, 0, ws, 3, out, 4, 0, 0), 3n);
  assert.deepEqual([...env.mem.read(out, 3)], [0x41, 0x80, 0xe9]);
  const used = env.zeros(4);
  const omega = env.putW('xΩ');
  assert.equal(kernel32(env, 'WideCharToMultiByte', 0, 0x400, omega, -1, out, 4, 0, used), 3n);
  assert.deepEqual([...env.mem.read(out, 3)], [0x78, 0x3f, 0]);
  assert.equal(env.mem.u32(used), 1);
  assert.match(env.stop('kernel32.dll', 'WideCharToMultiByte', 0, 0, omega, -1, out, 4, 0, 0).error, /U\+03a9 needs a best-fit mapping/);
});
