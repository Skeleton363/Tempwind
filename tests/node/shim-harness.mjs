// Drives shim functions directly, without wasm: a small guest memory, a
// state block with Win64 arguments, a TEB, a bump guest heap, and guest
// functions written in JS (for callbacks such as qsort's comparator).
import { createMemory64 } from '../../runtime/src/memory.mjs';
import { GuestMemory } from '../../runtime/src/guest-memory.mjs';
import { HostContext, HostStop } from '../../runtime/src/hostcall.mjs';
import { STATE_GS_BASE, gprOffset } from '../../runtime/src/abi.mjs';
import { Bump } from '../../runtime/src/process.mjs';
import { SHIM, findShim } from '../../runtime/src/win32/index.mjs';
import { Vfs } from '../../runtime/src/win32/vfs.mjs';

export const STATE = 0x1_0000n;
export const TEB = 0x1_1000n;
export const RSP0 = 0x3_0000n - 0x38n;
export const HEAP = 0x10_0000n;
const GUEST_FN = 0x5000_0000n;
const ARG_REGS = [1, 2, 8, 9];

/** A FILETIME clock for tests: starts at 2024-01-01 00:00:00 UTC and advances 1 s per reading. */
export function testClock(start = 133_485_408_000_000_000n) {
  let t = start;
  const now = () => { const v = t; t += 10_000_000n; return v; };
  return now;
}

/** The pseudo-address GetProcAddress returns in the harness for `dll!name`. */
export const PROC_BASE = 0x6000_0000n;

export function shimEnv({ argv = ['prog.exe'], image = null, pages = 128, vfs = null, programPath = 'C:\\tempwind\\prog.exe' } = {}) {
  const { memory } = createMemory64({ initialPages: pages });
  const mem = new GuestMemory(memory);
  const bump = new Bump('guest heap', HEAP, BigInt(pages) * 0x1_0000n);
  const out = { stdout: [], stderr: [] };
  const guestFns = [];
  const states = new Map();
  const data = new Map();
  const calls = [];
  const procs = [];
  const fs = vfs ?? new Vfs({ now: testClock() });
  const host = {
    call(target, args) {
      calls.push([target, args]);
      const f = guestFns[Number((target - GUEST_FN) / 16n)];
      if (target < GUEST_FN || !f) return { error: `no guest function at 0x${target.toString(16)}` };
      try {
        const r = f(...args);
        return typeof r === 'object' ? r : { rax: BigInt.asUintN(64, BigInt(r ?? 0)) };
      } catch (e) {
        if (e instanceof HostStop) return e.stop;
        throw e;
      }
    },
    alloc: (size, align) => bump.alloc(size, align),
    dllState: (dll, init) => { if (!states.has(dll)) states.set(dll, init()); return states.get(dll); },
    dataAddress: (dll, name) => data.get(`${dll}!${name}`),
    argv,
    image,
    vfs: fs,
    startTime: 133_485_408_000_000_000n,
    programPath,
    heapFree: () => bump.limit - bump.next,
    maxAppAddress: 0x7fff_fffe_ffffn,
    /** GetProcAddress: a pseudo-address per implemented function (the harness has no thunks). */
    procAddress(dll, name) {
      if (!findShim(dll, name, SHIM)?.fn) return undefined;
      let i = procs.indexOf(`${dll}!${name}`);
      if (i < 0) i = procs.push(`${dll}!${name}`) - 1;
      return PROC_BASE + 16n * BigInt(i);
    },
  };
  mem.setU64(STATE + BigInt(STATE_GS_BASE), TEB);
  const env = {
    mem, out, calls, host, data, vfs: fs, procs,
    /** The calling thread's last error. */
    lastError: () => env.ctx().lastError(),
    setLastError: (v) => env.ctx().setLastError(v),
    /** Reads a NUL-terminated UTF-16 string. */
    wstr(a) {
      let s = '';
      for (let p = a; ; p += 2n) { const c = mem.u16(p); if (!c) return s; s += String.fromCharCode(c); }
    },
    stdout: () => out.stdout.join(''),
    stderr: () => out.stderr.join(''),
    /** Registers a JS guest function and returns its address. */
    guest(f) { guestFns.push(f); return GUEST_FN + 16n * BigInt(guestFns.length - 1); },
    /** Allocates guest bytes (a string gets a NUL). */
    put(v, align = 16n) {
      const bytes = typeof v === 'string' ? new TextEncoder().encode(v + '\0') : v;
      const a = bump.alloc(BigInt(bytes.length || 1), align);
      mem.write(a, bytes);
      return a;
    },
    /** Allocates and writes a NUL-terminated UTF-16 string. */
    putW(s) {
      const a = bump.alloc(BigInt(2 * s.length + 2), 16n);
      for (let i = 0; i < s.length; i++) mem.setU16(a + 2n * BigInt(i), s.charCodeAt(i));
      return a;
    },
    zeros(n) { return bump.alloc(BigInt(n), 16n); },
    cstr(a) {
      const bytes = [];
      for (let p = a; mem.u8get(p); p++) bytes.push(mem.u8get(p));
      return new TextDecoder().decode(new Uint8Array(bytes));
    },
    ctx() { return new HostContext(mem, STATE, (s, b) => out[s].push(new TextDecoder('latin1').decode(b)), host); },
    /**
     * Calls `dll!name` with Win64 arguments; returns RAX's value (BigInt), or
     * undefined. Throws HostStop for a stop.
     */
    call(dll, name, ...args) {
      const impl = findShim(dll, name, SHIM);
      if (!impl?.fn) throw new Error(`no shim function ${dll}!${name}`);
      mem.setU64(STATE + BigInt(gprOffset(4)), RSP0);
      args.forEach((a, i) => {
        const v = BigInt.asUintN(64, BigInt(a));
        if (i < 4) mem.setU64(STATE + BigInt(gprOffset(ARG_REGS[i])), v);
        else mem.setU64(RSP0 + 0x28n + 8n * BigInt(i - 4), v);
      });
      const r = impl.fn(env.ctx());
      return r === undefined ? undefined : BigInt.asUintN(64, BigInt(r));
    },
    /** Like call, but returns the stop ({exitCode} or {error}) the call ends with. */
    stop(dll, name, ...args) {
      try { env.call(dll, name, ...args); } catch (e) { if (e instanceof HostStop) return e.stop; throw e; }
      throw new Error(`${dll}!${name} returned instead of stopping`);
    },
  };
  return env;
}

export const msvcrt = (env, name, ...args) => env.call('msvcrt.dll', name, ...args);
export const kernel32 = (env, name, ...args) => env.call('kernel32.dll', name, ...args);
export const advapi32 = (env, name, ...args) => env.call('advapi32.dll', name, ...args);
export const oleaut32 = (env, name, ...args) => env.call('oleaut32.dll', name, ...args);
export const user32 = (env, name, ...args) => env.call('user32.dll', name, ...args);
