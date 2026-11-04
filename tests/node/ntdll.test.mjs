// ntdll: RtlGetVersion (ntdll.mjs), reached through GetProcAddress, through
// shim-harness.mjs.
import test from 'node:test';
import assert from 'node:assert/strict';
import { shimEnv } from './shim-harness.mjs';

const ntdll = (env, name, ...args) => env.call('ntdll.dll', name, ...args);

test('RtlGetVersion: the real version; the EX fields only for the EX size', () => {
  const env = shimEnv();
  const ex = env.zeros(284);
  env.mem.setU32(ex, 284);
  env.mem.fill(ex + 20n, 264, 0xaa);
  assert.equal(ntdll(env, 'RtlGetVersion', ex), 0n, 'STATUS_SUCCESS');
  assert.deepEqual([4n, 8n, 12n, 16n].map((o) => env.mem.u32(ex + o)), [10, 0, 26100, 2]);
  assert.equal(env.mem.u16(ex + 20n), 0, 'empty szCSDVersion');
  assert.deepEqual([env.mem.u16(ex + 276n), env.mem.u16(ex + 278n), env.mem.u16(ex + 280n)], [0, 0, 0x100]);
  assert.deepEqual([env.mem.u8get(ex + 282n), env.mem.u8get(ex + 283n)], [1, 0], 'VER_NT_WORKSTATION');

  const base = env.zeros(284);
  env.mem.setU32(base, 276);
  env.mem.fill(base + 276n, 8, 0xaa);
  assert.equal(ntdll(env, 'RtlGetVersion', base), 0n);
  assert.equal(env.mem.u32(base + 12n), 26100);
  assert.equal(env.mem.u16(base + 276n), 0xaaaa, 'past a plain RTL_OSVERSIONINFOW: untouched');

  const bad = env.zeros(284);
  env.mem.setU32(bad, 100);
  assert.match(env.stop('ntdll.dll', 'RtlGetVersion', bad).error, /dwOSVersionInfoSize 100/);
});
