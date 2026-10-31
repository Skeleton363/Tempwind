// Guest threads (runtime.md, "Guest threads (M5): cooperative, on one JS
// thread"): real translated x86 code on real JSPI. Every thread runs the same
// small driver at TEXT, which asks TEST.dll!next for a call to make (an import
// thunk, or the driver itself for a nested guest call), makes it, and reports
// RAX to TEST.dll!result. The steps come from JS generator "scripts", keyed by
// the driver's RCX, so each test reads like the guest program it stands for.
// Needs x64wasm_rt.wasm (tests/pe/prepare.sh), which translates the driver.
import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { TEB_ADDR, PEB_ADDR, gprOffset } from '../../runtime/src/abi.mjs';
import { Process, ProcessError } from '../../runtime/src/process.mjs';
import { RuntimeTranslator } from '../../runtime/src/rt.mjs';
import { SHIM } from '../../runtime/src/win32/index.mjs';
import { fakeExe, fakePlan, BASE, TEXT, IDATA, TLSSEC } from './pe-fixture.mjs';

const RT = new URL('../../target/wasm32-unknown-unknown/release/x64wasm_rt.wasm', import.meta.url).pathname;
const h = (v) => '0x' + BigInt(v).toString(16);
const INFINITE = 0xffff_ffff, WAIT_TIMEOUT = 0x102n, STILL_ACTIVE = 259, CREATE_SUSPENDED = 4;
const RCX = 1, RSP = 4;
const ARG_REGS = [1, 2, 8, 9];
const IAT_NEXT = IDATA + 0x40n, IAT_RESULT = IDATA + 0x48n;

const rel32 = (target, next) => {
  const d = Number(BigInt.asIntN(32, target - next));
  return [d & 0xff, (d >> 8) & 0xff, (d >> 16) & 0xff, (d >>> 24) & 0xff];
};
/**
 * The driver (RCX = script id; returns the script's value):
 *   push rbx; sub rsp, 0x40; mov rbx, rcx
 * loop:
 *   mov rcx, rbx; call [next]; test rax, rax; jz done
 *   call rax; mov rdx, rax; mov rcx, rbx; call [result]; jmp loop
 * done:
 *   mov rax, rcx; add rsp, 0x40; pop rbx; ret
 */
const DRIVER = [
  0x53, 0x48, 0x83, 0xec, 0x40, 0x48, 0x89, 0xcb,
  0x48, 0x89, 0xd9, 0xff, 0x15, ...rel32(IAT_NEXT, TEXT + 17n),
  0x48, 0x85, 0xc0, 0x74, 0x10,
  0xff, 0xd0, 0x48, 0x89, 0xc2, 0x48, 0x89, 0xd9, 0xff, 0x15, ...rel32(IAT_RESULT, TEXT + 36n),
  0xeb, 0xe2,
  0x48, 0x89, 0xc8, 0x48, 0x83, 0xc4, 0x40, 0x5b, 0xc3,
];

/** A guest call the script asks for: `dll!name(args)`, or the driver with another script. */
const api = (dll, name, ...args) => ({ dll, name, args });
const k32 = (name, ...args) => api('kernel32.dll', name, ...args);
const crt = (name, ...args) => api('msvcrt.dll', name, ...args);
const guest = (id) => ({ target: TEXT, args: [id] });

/**
 * Loads a process whose threads run `scripts` (Map id -> generator function
 * `(s) => ...`); the main thread runs script 1. `s` has the process (`p`),
 * the thread's `id`, `rdx` at the driver's entry and `depth()`. Each `yield`
 * of api(...) or guest(...) returns RAX as a BigInt.
 */
async function load(scripts, { tls = false } = {}) {
  assert.ok(existsSync(RT), `${RT} is missing. Create it with: tests/pe/prepare.sh`);
  const exe = fakeExe();
  exe.set(DRIVER, 0x400);
  const plan = await fakePlan(exe, (j) => {
    j.imports.push({ dll: 'TEST.dll', functions: [
      { name: 'next', ordinal: null, iat: h(IAT_NEXT) },
      { name: 'result', ordinal: null, iat: h(IAT_RESULT) },
    ] });
    if (tls) {
      j.tls = {
        startOfRawData: h(TLSSEC), endOfRawData: h(TLSSEC + 0x10n), addressOfIndex: h(TLSSEC + 0x20n),
        sizeOfZeroFill: '0x10', callbacks: [h(TEXT)],
      };
    }
  });
  const rt = await RuntimeTranslator.load(readFileSync(RT));
  // The AOT module: the run-time translator's translation of the driver.
  const aot = rt.translate({ codeBase: TEXT, code: exe.subarray(0x400, 0x500), entry: TEXT, known: new BigUint64Array(0) });
  let p;
  const runs = new Map();
  const shim = {
    ...SHIM,
    'test.dll': {
      next: {
        args: 1,
        fn(ctx) {
          const id = ctx.arg(0);
          const key = `${ctx.state}:${id}`;
          let run = runs.get(key);
          if (!run) {
            // The entry point's RCX is the PEB: the main thread runs script 1.
            const script = scripts.get(id === PEB_ADDR ? 1n : id);
            if (!script) ctx.fail(`no script ${id}`);
            const s = { p, id, rdx: ctx.reg(2), depth: () => p.linker.depth.value, thread: () => p.threads.current };
            run = { gen: script(s), last: undefined };
            runs.set(key, run);
          }
          const r = run.gen.next(run.last);
          if (r.done) {
            runs.delete(key);
            ctx.mem.setU64(ctx.state + BigInt(gprOffset(RCX)), BigInt.asUintN(64, BigInt(r.value ?? 0)));
            return 0n;
          }
          let { target, args } = r.value;
          if (target === undefined) {
            target = p.procAddress(r.value.dll, r.value.name);
            if (!target) ctx.fail(`no shim function ${r.value.dll}!${r.value.name}`);
          }
          // Stack arguments go above the driver's shadow space: [driver RSP + 0x20 + 8 (i - 4)].
          const rsp = ctx.reg(RSP) + 8n;
          args.forEach((a, i) => {
            const v = BigInt.asUintN(64, BigInt(a));
            if (i < 4) ctx.mem.setU64(ctx.state + BigInt(gprOffset(ARG_REGS[i])), v);
            else ctx.write64(rsp + 0x20n + 8n * BigInt(i - 4), v);
          });
          return target;
        },
      },
      result: {
        args: 2,
        fn(ctx) { runs.get(`${ctx.state}:${ctx.arg(0)}`).last = ctx.arg(1); return 0n; },
      },
    },
  };
  p = await Process.load({ exe, plan, aot, rt, shim });
  return p;
}

const id32 = (p) => p.guestHeap.alloc(8n);
const start = (fnId, flags = 0, idP = 0n) => crt('_beginthreadex', 0, 0, TEXT, fnId, flags, idP);

test('threads start in order, end with their return value or _endthreadex; CREATE_SUSPENDED waits for ResumeThread', async () => {
  const log = [];
  let p;
  const scripts = new Map([
    [1n, function* (s) {
      p = s.p;
      const idP = id32(p);
      const h1 = yield start(101, 0, idP);
      assert.equal(p.mem.u32(idP), 0x1008);
      assert.equal(yield k32('GetThreadId', h1), 0x1008n);
      const h2 = yield start(102, CREATE_SUSPENDED, idP);
      assert.equal(p.mem.u32(idP), 0x100c);
      log.push('main: created');
      const code = id32(p);
      assert.equal(yield k32('GetExitCodeThread', h1, code), 1n);
      assert.equal(p.mem.u32(code), STILL_ACTIVE);
      assert.equal(yield k32('WaitForSingleObject', h1, INFINITE), 0n);
      yield k32('GetExitCodeThread', h1, code);
      assert.equal(p.mem.u32(code), 7);
      log.push('main: thread 1 ended');
      assert.equal(yield k32('ResumeThread', h2), 1n, 'previous suspend count');
      assert.equal(yield k32('ResumeThread', h2), 0n);
      assert.equal(yield k32('WaitForSingleObject', h2, INFINITE), 0n);
      yield k32('GetExitCodeThread', h2, code);
      assert.equal(p.mem.u32(code), 9);
      assert.equal(yield k32('CloseHandle', h1), 1n);
      assert.equal(yield k32('GetCurrentThreadId'), 0x1004n);
      return 42;
    }],
    [101n, function* () { log.push(`thread 0x${(yield k32('GetCurrentThreadId')).toString(16)}`); return 0xdead_0000_0007n; }],
    [102n, function* () { log.push(`thread 0x${(yield k32('GetCurrentThreadId')).toString(16)}`); yield crt('_endthreadex', 9); throw new Error('not reached'); }],
  ]);
  const proc = await load(scripts);
  assert.equal(await proc.start(), 42, 'the main thread returns the exit code');
  assert.deepEqual(log, ['main: created', 'thread 0x1008', 'main: thread 1 ended', 'thread 0x100c']);
});

test('events: auto-reset wakes one waiter and resets, manual-reset wakes all and stays signaled', async () => {
  const log = [];
  let auto, manual;
  const waiter = (name, ev) => function* () { assert.equal(yield k32('WaitForSingleObject', ev(), INFINITE), 0n); log.push(name); };
  const scripts = new Map([
    [1n, function* () {
      auto = yield k32('CreateEventW', 0, 0, 0, 0);
      manual = yield k32('CreateEventW', 0, 1, 0, 0);
      for (const id of [11, 12, 13, 14]) yield start(id);
      yield k32('Sleep', 0); // all four run and block
      log.push('main: set auto');
      yield k32('SetEvent', auto);
      assert.equal(yield k32('WaitForSingleObject', auto, 0), WAIT_TIMEOUT, 'the waiter took the signal');
      yield k32('Sleep', 0);
      log.push('main: set auto again');
      yield k32('SetEvent', auto);
      yield k32('Sleep', 0);
      log.push('main: set manual');
      yield k32('SetEvent', manual);
      yield k32('Sleep', 0);
      assert.equal(yield k32('WaitForSingleObject', manual, 0), 0n, 'still signaled');
      yield k32('ResetEvent', manual);
      assert.equal(yield k32('WaitForSingleObject', manual, 0), WAIT_TIMEOUT);
      return 0;
    }],
    [11n, waiter('a1', () => auto)],
    [12n, waiter('a2', () => auto)],
    [13n, waiter('m1', () => manual)],
    [14n, waiter('m2', () => manual)],
  ]);
  assert.equal(await (await load(scripts)).start(), 0);
  assert.deepEqual(log, ['main: set auto', 'a1', 'main: set auto again', 'a2', 'main: set manual', 'm1', 'm2']);
});

test('semaphores: a release of n wakes the first n waiters', async () => {
  const log = [];
  let sem;
  const waiter = (name) => function* () { yield k32('WaitForSingleObject', sem, INFINITE); log.push(name); };
  const scripts = new Map([
    [1n, function* (s) {
      sem = yield k32('CreateSemaphoreW', 0, 0, 3, 0);
      for (const id of [21, 22, 23]) yield start(id);
      yield k32('Sleep', 0);
      const prev = id32(s.p);
      assert.equal(yield k32('ReleaseSemaphore', sem, 2, prev), 1n);
      assert.equal(s.p.mem.u32(prev), 0);
      yield k32('Sleep', 0);
      log.push('main');
      yield k32('ReleaseSemaphore', sem, 2, 0);
      yield k32('Sleep', 0);
      assert.equal(yield k32('WaitForSingleObject', sem, 0), 0n, 'one count left');
      assert.equal(yield k32('WaitForSingleObject', sem, 0), WAIT_TIMEOUT);
      return 0;
    }],
    [21n, waiter('s1')], [22n, waiter('s2')], [23n, waiter('s3')],
  ]);
  assert.equal(await (await load(scripts)).start(), 0);
  assert.deepEqual(log, ['s1', 's2', 'main', 's3']);
});

test('critical sections: recursion, contention, hand-over to the waiter, TryEnter', async () => {
  const log = [];
  let cs, p;
  const fields = () => [p.mem.u32(cs + 8n) | 0, p.mem.u32(cs + 12n), Number(p.mem.u64(cs + 16n))];
  const scripts = new Map([
    [1n, function* (s) {
      p = s.p;
      cs = p.guestHeap.alloc(40n);
      yield k32('InitializeCriticalSection', cs);
      yield k32('EnterCriticalSection', cs);
      yield k32('EnterCriticalSection', cs);
      assert.deepEqual(fields(), [-2, 2, 0x1004]);
      const t = yield start(31);
      yield k32('Sleep', 0); // the thread fails TryEnter, then blocks in Enter
      assert.deepEqual(fields(), [-6, 2, 0x1004], 'LockCount counts the waiter');
      yield k32('LeaveCriticalSection', cs);
      assert.deepEqual(fields(), [-6, 1, 0x1004], 'still held once');
      log.push('main: leave');
      yield k32('LeaveCriticalSection', cs);
      assert.deepEqual(fields(), [-2, 1, 0x1008], 'handed to the waiter at once');
      assert.equal(yield k32('TryEnterCriticalSection', cs), 0n, 'owned by the other thread');
      yield k32('WaitForSingleObject', t, INFINITE);
      assert.equal(yield k32('TryEnterCriticalSection', cs), 1n);
      assert.deepEqual(fields(), [-2, 1, 0x1004]);
      yield k32('LeaveCriticalSection', cs);
      yield k32('DeleteCriticalSection', cs);
      return 0;
    }],
    [31n, function* () {
      assert.equal(yield k32('TryEnterCriticalSection', cs), 0n);
      log.push('thread: enter');
      yield k32('EnterCriticalSection', cs);
      log.push('thread: owns');
      yield k32('LeaveCriticalSection', cs);
    }],
  ]);
  assert.equal(await (await load(scripts)).start(), 0);
  assert.deepEqual(log, ['thread: enter', 'main: leave', 'thread: owns']);
});

test('WaitForMultipleObjects: any returns the first signaled index, all waits for every object', async () => {
  let e1, e2, sem;
  const log = [];
  const scripts = new Map([
    [1n, function* (s) {
      e1 = yield k32('CreateEventW', 0, 0, 0, 0);
      e2 = yield k32('CreateEventW', 0, 0, 0, 0);
      sem = yield k32('CreateSemaphoreW', 0, 0, 1, 0);
      const arr = s.p.guestHeap.alloc(24n);
      [e1, e2, sem].forEach((x, i) => s.p.mem.setU64(arr + 8n * BigInt(i), x));
      yield start(41);
      assert.equal(yield k32('WaitForMultipleObjects', 3, arr, 0, INFINITE), 1n, 'e2');
      log.push('main: any');
      assert.equal(yield k32('WaitForMultipleObjects', 3, arr, 1, INFINITE), 0n);
      log.push('main: all');
      assert.equal(yield k32('WaitForMultipleObjects', 3, arr, 0, 0), WAIT_TIMEOUT, 'all three were consumed');
      return 0;
    }],
    [41n, function* () {
      yield k32('SetEvent', e2);
      yield k32('Sleep', 0);
      yield k32('SetEvent', e1);
      log.push('thread: e1');
      yield k32('Sleep', 0);
      yield k32('SetEvent', e2);
      log.push('thread: e2');
      yield k32('Sleep', 0);
      yield k32('ReleaseSemaphore', sem, 1, 0);
      log.push('thread: sem');
    }],
  ]);
  assert.equal(await (await load(scripts)).start(), 0);
  assert.deepEqual(log, ['main: any', 'thread: e1', 'thread: e2', 'thread: sem', 'main: all']);
});

test('timeouts: 0 polls; a finite wait times out when no other thread can run; Sleep lets time pass', async () => {
  let ev;
  const log = [];
  const scripts = new Map([
    [1n, function* () {
      ev = yield k32('CreateEventW', 0, 0, 0, 0);
      assert.equal(yield k32('WaitForSingleObject', ev, 0), WAIT_TIMEOUT);
      assert.equal(yield k32('WaitForSingleObject', ev, 50), WAIT_TIMEOUT, 'no other thread: at once');
      const t = yield start(51);
      assert.equal(yield k32('WaitForSingleObject', ev, 50), 0n, 'the thread signals in its turn');
      log.push('main: signaled');
      assert.equal(yield k32('WaitForSingleObject', ev, 50), WAIT_TIMEOUT, 'the thread ended without signaling');
      log.push('main: timed out');
      const u = yield start(52);
      yield k32('Sleep', 0);
      yield k32('Sleep', 0); // only a yield: the thread's finite wait goes on
      log.push('main: sleep 10');
      yield k32('Sleep', 10); // time passes: the thread times out
      yield k32('WaitForSingleObject', u, INFINITE);
      yield k32('CloseHandle', t);
      return 0;
    }],
    [51n, function* () { log.push('thread: set'); yield k32('SetEvent', ev); yield k32('Sleep', 0); log.push('thread: end'); }],
    [52n, function* () {
      assert.equal(yield k32('WaitForSingleObject', ev, 1000), WAIT_TIMEOUT);
      log.push('thread: timed out');
    }],
  ]);
  assert.equal(await (await load(scripts)).start(), 0);
  assert.deepEqual(log, ['thread: set', 'main: signaled', 'thread: end', 'main: timed out', 'main: sleep 10', 'thread: timed out']);
});

test('deadlock: when every thread waits forever, the run stops naming each wait', async () => {
  let e1, e2, cs;
  const scripts = new Map([
    [1n, function* (s) {
      e1 = yield k32('CreateEventW', 0, 0, 0, 0);
      e2 = yield k32('CreateEventW', 0, 0, 0, 0);
      cs = s.p.guestHeap.alloc(40n);
      yield k32('InitializeCriticalSection', cs);
      yield k32('EnterCriticalSection', cs);
      yield start(61);
      yield start(62);
      yield start(63, CREATE_SUSPENDED);
      yield k32('WaitForSingleObject', e1, INFINITE);
    }],
    [61n, function* (s) {
      const arr = s.p.guestHeap.alloc(8n);
      s.p.mem.setU64(arr, e2);
      yield k32('WaitForMultipleObjects', 1, arr, 0, INFINITE);
    }],
    [62n, function* () { yield k32('EnterCriticalSection', cs); }],
    [63n, function* () {}],
  ]);
  const p = await load(scripts);
  await assert.rejects(p.start(), (e) => {
    assert.ok(e instanceof ProcessError);
    // The thread that blocked last finds the deadlock.
    assert.match(e.message, /^kernel32.dll!EnterCriticalSection: deadlock: no thread can run; /);
    assert.match(e.message, new RegExp(`thread 0x1004 waits in WaitForSingleObject\\(${h(e1)}, INFINITE\\)`));
    assert.match(e.message, new RegExp(`thread 0x1008 waits in WaitForMultipleObjects\\(1, \\[${h(e2)}\\], any, INFINITE\\)`));
    assert.match(e.message, new RegExp(`thread 0x100c waits in EnterCriticalSection\\(${h(cs)}\\), held by thread 0x1004`));
    assert.match(e.message, /thread 0x1010 is suspended/);
    return true;
  });
  // Sleep(INFINITE) on the only thread is a deadlock too.
  const q = await load(new Map([[1n, function* () { yield k32('Sleep', INFINITE); }]]));
  await assert.rejects(q.start(), /kernel32.dll!Sleep: deadlock: no thread can run; thread 0x1004 waits in Sleep\(INFINITE\)$/);
});

test('per-thread state: TLS slots, the static TLS block, last error, errno, depth; TLS callbacks attach and detach', async () => {
  const seen = { callbacks: [] };
  let slot, p;
  /** The calling thread's static TLS block (TEB.ThreadLocalStoragePointer[0]). */
  const tlsBlock = () => p.mem.u64(p.mem.u64(p.threads.current.teb + 0x58n));
  const scripts = new Map([
    [1n, function* (s) {
      p = s.p;
      slot = yield k32('TlsAlloc');
      yield k32('TlsSetValue', slot, 0x111);
      yield k32('SetLastError', 5);
      const errno = yield crt('_errno');
      p.mem.setU32(errno, 33);
      p.mem.setU8(tlsBlock(), 0x5a);
      const t = yield start(71);
      // Three guest calls deep, then let the thread run.
      assert.equal(yield guest(72), 0n);
      assert.equal(yield k32('TlsGetValue', slot), 0x111n);
      assert.equal(yield k32('GetLastError'), 0n, 'TlsGetValue cleared it');
      assert.equal(p.mem.u32(errno), 33);
      assert.equal(p.mem.u8get(tlsBlock()), 0x5a);
      yield k32('WaitForSingleObject', t, INFINITE);
      seen.mainErrno = errno;
      return 0;
    }],
    [72n, function* (s) { return yield guest(73); }],
    [73n, function* (s) {
      const before = s.depth();
      seen.mainDepth = before;
      yield k32('SetLastError', 5);
      yield k32('Sleep', 0); // the thread runs to its own Sleep
      assert.equal(s.depth(), before, 'depth restored when the main thread resumes');
      assert.equal(yield k32('GetLastError'), 5n);
      yield k32('Sleep', 0); // the thread ends
      return 0;
    }],
    [71n, function* (s) {
      seen.threadDepth = s.depth();
      assert.equal(yield k32('TlsGetValue', slot), 0n, 'a new thread starts with empty slots');
      yield k32('TlsSetValue', slot, 0x222);
      yield k32('SetLastError', 6);
      const errno = yield crt('_errno');
      seen.threadErrno = errno;
      p.mem.setU32(errno, 44);
      assert.equal(p.mem.u8get(tlsBlock()), p.mem.u8get(TLSSEC), 'its own copy of the TLS template');
      assert.notEqual(p.mem.u8get(TLSSEC), 0x5a);
      assert.notEqual(p.threads.current.teb, TEB_ADDR);
      yield k32('Sleep', 0);
      assert.equal(s.depth(), seen.threadDepth);
      assert.equal(yield k32('GetLastError'), 6n);
      assert.equal(yield k32('TlsGetValue', slot), 0x222n);
      assert.equal(p.mem.u32(errno), 44);
    }],
    // The TLS callback (RCX = image base): records the reason and the thread.
    [BASE, function* (s) { seen.callbacks.push([Number(s.rdx), Number(yield k32('GetCurrentThreadId'))]); return 1; }],
  ]);
  const proc = await load(scripts, { tls: true });
  assert.equal(await proc.start(), 0);
  assert.notEqual(seen.mainErrno, seen.threadErrno, 'errno belongs to each thread');
  assert.ok(seen.mainDepth > seen.threadDepth, `depth ${seen.mainDepth} three calls deep, ${seen.threadDepth} at a thread's start`);
  assert.deepEqual(seen.callbacks, [[1, 0x1004], [2, 0x1008], [3, 0x1008]], 'process attach, then thread attach and detach on the thread');
});

test('a wait that would block inside a call from the host into guest code stops; one that need not, runs', async () => {
  let cs, ev;
  const scripts = new Map([
    [1n, function* (s) {
      cs = s.p.guestHeap.alloc(40n);
      ev = yield k32('CreateEventW', 0, 1, 1, 0);
      yield k32('InitializeCriticalSection', cs);
      const base = s.p.guestHeap.alloc(16n);
      scripts.set(base, comparator);
      scripts.set(base + 8n, comparator);
      // Uncontended: the comparator enters and leaves, waits on a signaled event, yields.
      yield crt('qsort', base, 2, 8, TEXT);
      yield start(81);
      yield k32('Sleep', 0); // the thread takes the critical section and waits
      yield crt('qsort', base, 2, 8, TEXT);
      throw new Error('not reached');
    }],
    [81n, function* () { yield k32('EnterCriticalSection', cs); yield k32('Sleep', INFINITE); }],
  ]);
  const comparator = function* () {
    yield k32('EnterCriticalSection', cs);
    yield k32('LeaveCriticalSection', cs);
    assert.equal(yield k32('WaitForSingleObject', ev, INFINITE), 0n);
    yield k32('Sleep', 0);
    return 0;
  };
  const p = await load(scripts);
  await assert.rejects(p.start(), new RegExp(`kernel32.dll!EnterCriticalSection: EnterCriticalSection\\(${h(cs)}\\) would block inside a call from the host into guest code`));
});
