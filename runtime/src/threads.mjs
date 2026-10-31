// Guest threads (docs/design/runtime.md, "Guest threads (M5): cooperative,
// on one JS thread"): the scheduler, waits on handle objects (events,
// semaphores, threads) and locks (critical sections, msvcrt locks).
//
// A thread that cannot go on gets a Promise from `block`/`yield`; the host
// call returns it through a WebAssembly.Suspending host function, which
// suspends the thread's whole wasm stack. The scheduler then resolves the
// Promise of the next thread, which continues once the current one has
// suspended. Only one thread runs at a time and only the running thread
// switches, so the schedule is deterministic.

import { THREAD_ID } from './abi.mjs';

export const WAIT_OBJECT_0 = 0;
export const WAIT_TIMEOUT = 0x102;
export const WAIT_FAILED = 0xffff_ffff;
export const INFINITE = 0xffff_ffff;
/** GetExitCodeThread's value while a thread runs (STATUS_PENDING). */
export const STILL_ACTIVE = 259;

const hex = (v) => '0x' + BigInt.asUintN(64, BigInt(v)).toString(16);

/** One guest thread. `object` is what its handles name in the handle table. */
export class Thread {
  /**
   * @param {{id: number, state: bigint, teb?: bigint, stackBase?: bigint, stackLimit?: bigint, main?: boolean}} opts
   */
  constructor({ id, state, teb = 0n, stackBase = 0n, stackLimit = 0n, main = false }) {
    this.id = id;
    this.state = state;
    this.teb = teb;
    this.stackBase = stackBase;
    this.stackLimit = stackLimit;
    this.main = main;
    /** 'new' (not started), 'ready', 'running', 'blocked' or 'done'. */
    this.status = main ? 'running' : 'new';
    this.suspendCount = 0;
    this.exitCode = STILL_ACTIVE;
    /** What the thread waits for while 'blocked': {objects?, all?, lock?, finite, label, seq}. */
    this.wait = null;
    /** The value its pending host call returns when it resumes. */
    this.result = undefined;
    /** `depth` while it is not running (runtime.md: saved when it suspends). */
    this.depth = 0;
    /** Per-thread values of the shim DLLs (msvcrt's errno, ...). */
    this.locals = new Map();
    this.resume = null;
    this.object = { kind: 'thread', thread: this };
  }

  local(key, init) {
    if (!this.locals.has(key)) this.locals.set(key, init());
    return this.locals.get(key);
  }
}

/** Whether a wait on handle object `obj` is satisfied now (without its side effect). */
export function isSignaled(obj) {
  switch (obj.kind) {
    case 'event': return obj.signaled;
    case 'semaphore': return obj.count > 0;
    case 'thread': return obj.thread.status === 'done';
    default: throw new Error(`waiting on a ${obj.kind} handle`);
  }
}

/** The side effect of a satisfied wait: an auto-reset event resets, a semaphore counts down. */
function consume(obj) {
  if (obj.kind === 'event' && !obj.manual) obj.signaled = false;
  else if (obj.kind === 'semaphore') obj.count--;
}

/** Satisfies a wait if it can: the index of the object (0 for "all"), or -1. */
function acquire(objects, all) {
  if (all) {
    if (!objects.every(isSignaled)) return -1;
    objects.forEach(consume);
    return 0;
  }
  const i = objects.findIndex(isSignaled);
  if (i >= 0) consume(objects[i]);
  return i;
}

/** A lock with an owner, a recursion count and waiters (critical sections, msvcrt locks). */
export class Lock {
  constructor(name) {
    this.name = name;
    this.owner = null;
    this.count = 0;
    this.waiters = [];
  }
}

/** The process's threads: who runs, who waits for what, and the switches between them. */
export class Threads {
  /**
   * @param {{depth?: {value: number} | null, nesting?: () => number, canSuspend?: boolean,
   *   onEnter?: (t: Thread) => void}} [opts]
   *   `depth`: the linker's call-depth global; `nesting`: the depth of calls
   *   from the host into guest code; `canSuspend`: JSPI is available;
   *   `onEnter`: called when a thread starts or resumes running.
   */
  constructor({ depth = null, nesting = () => 0, canSuspend = false, onEnter = () => {} } = {}) {
    this.depth = depth;
    this.nesting = nesting;
    this.canSuspend = canSuspend;
    this.onEnter = onEnter;
    this.threads = [];
    this.byState = new Map();
    /** Runnable threads, in the order they became runnable. */
    this.queue = [];
    this.current = null;
    this.seq = 0;
    this.nextId = THREAD_ID + 4;
    /** Set when the process has ended: nothing switches any more. */
    this.finished = false;
  }

  /** Registers a thread. The first one is the main thread and runs. */
  add(t) {
    this.threads.push(t);
    this.byState.set(t.state, t);
    if (t.main) this.current = t;
    return t;
  }

  /** A new thread id: 0x1008, 0x100C, ... (the main thread is 0x1004). */
  newId() {
    const id = this.nextId;
    this.nextId += 4;
    return id;
  }

  /**
   * The thread a host call runs on (by its state block). Where no thread is
   * registered yet (the shim test harness), the caller is the main thread.
   */
  of(ctx) {
    const t = this.byState.get(ctx.state);
    if (t) return t;
    if (this.threads.length) ctx.fail(`no thread runs on state block ${hex(ctx.state)}`);
    return this.add(new Thread({ id: THREAD_ID, state: ctx.state, teb: ctx.teb(), main: true }));
  }

  /** Threads that have not ended. */
  live() { return this.threads.filter((t) => t.status !== 'done'); }

  /** Makes a thread runnable with the value its pending call returns. */
  ready(t, result) {
    t.wait = null;
    t.result = result;
    t.status = 'ready';
    this.queue.push(t);
  }

  /** Starts a new thread once its suspend count is 0 (it runs when the scheduler picks it). */
  start(t) {
    if (t.status === 'new' && t.suspendCount === 0) this.ready(t, undefined);
  }

  /**
   * A Promise that settles when `t` runs again, with the value its pending
   * call returns. Saves `depth` (runtime.md: `depth` belongs to each thread).
   */
  suspend(t) {
    if (this.depth) t.depth = this.depth.value;
    return new Promise((resolve) => { t.resume = resolve; }).then((v) => { this.enter(t); return v; });
  }

  /** `t` runs: it becomes current and gets its `depth` back. */
  enter(t) {
    t.status = 'running';
    this.current = t;
    if (this.depth) this.depth.value = t.depth;
    this.onEnter(t);
  }

  /** Resumes `t` (from the queue): it continues once the running thread has suspended or returned. */
  switchTo(t) {
    if (this.finished) return;
    const resume = t.resume;
    t.resume = null;
    this.current = null;
    resume(t.result);
  }

  /**
   * The next thread to run: the oldest runnable one, else the thread that
   * has waited longest with a finite timeout, which times out. Null if none.
   */
  pick() {
    if (this.queue.length) return this.queue.shift();
    let oldest = null;
    for (const t of this.threads) {
      if (t.status === 'blocked' && t.wait.finite && (!oldest || t.wait.seq < oldest.wait.seq)) oldest = t;
    }
    if (!oldest) return null;
    this.ready(oldest, WAIT_TIMEOUT);
    return this.queue.shift();
  }

  /** The error stop when no thread can run: each live thread and what it waits for. */
  deadlock() {
    const what = this.live().map((t) => {
      if (t.status === 'new') return `thread ${hex(t.id)} is suspended (CREATE_SUSPENDED, never resumed)`;
      if (t.status !== 'blocked') return `thread ${hex(t.id)} is ${t.status}`;
      const owner = t.wait.lock?.owner;
      return `thread ${hex(t.id)} waits in ${t.wait.label}${owner ? `, held by thread ${hex(owner.id)}` : ''}`;
    });
    return `deadlock: no thread can run; ${what.join('; ')}`;
  }

  /**
   * Blocks the calling thread until `wait` is satisfied: a Promise of the
   * call's value, or the value itself if a finite wait times out at once.
   * Stops on a deadlock, inside a call from the host into guest code, and
   * without JSPI.
   */
  block(ctx, t, wait) {
    if (this.nesting() > 0) {
      ctx.fail(`${wait.label} would block inside a call from the host into guest code, which cannot suspend (runtime.md, "Guest threads")`);
    }
    t.wait = { ...wait, seq: this.seq++ };
    t.status = 'blocked';
    const next = this.pick();
    if (next === t) {
      t.status = 'running';
      return t.result;
    }
    if (!next) {
      const message = this.deadlock();
      t.wait = null;
      t.status = 'running';
      ctx.fail(message);
    }
    if (!this.canSuspend) ctx.fail(`${wait.label} has to switch threads, which needs JSPI (WebAssembly.Suspending)`);
    if (wait.lock) wait.lock.waiters.push(t);
    const p = this.suspend(t);
    this.switchTo(next);
    return p;
  }

  /**
   * WaitForSingleObject/WaitForMultipleObjects on handle objects: the result
   * (WAIT_OBJECT_0 + i or WAIT_TIMEOUT), or a Promise of it.
   */
  waitFor(ctx, objects, all, ms, label) {
    const i = acquire(objects, all);
    if (i >= 0) return WAIT_OBJECT_0 + i;
    if (ms === 0) return WAIT_TIMEOUT;
    return this.block(ctx, this.of(ctx), { objects, all, finite: ms !== INFINITE, label });
  }

  /** Blocks forever (Sleep(INFINITE)): only the end of the process ends it. */
  blockForever(ctx, label) {
    return this.block(ctx, this.of(ctx), { finite: false, label });
  }

  /**
   * Gives other threads a turn (Sleep, SwitchToThread). With `timePasses`
   * (Sleep with a nonzero time) and no runnable thread, the longest finite
   * wait times out. Returns whether it switched, or a Promise of `value`.
   */
  yield(ctx, timePasses, value) {
    if (this.nesting() > 0 || !this.canSuspend) return false;
    const t = this.of(ctx);
    let next = this.queue.shift();
    if (!next && timePasses) next = this.pick();
    if (!next) return false;
    this.ready(t, value);
    const p = this.suspend(t);
    this.switchTo(next);
    return p;
  }

  /** Signaled objects may satisfy waits: those threads become runnable (in the order they started waiting). */
  signal() {
    const waiting = this.threads.filter((t) => t.status === 'blocked' && t.wait.objects).sort((a, b) => a.wait.seq - b.wait.seq);
    for (const t of waiting) {
      const i = acquire(t.wait.objects, t.wait.all);
      if (i >= 0) this.ready(t, WAIT_OBJECT_0 + i);
    }
  }

  /** Enters a lock: undefined at once (free or recursive), or a Promise that settles when the lock is handed over. */
  lock(ctx, lock, label) {
    const t = this.of(ctx);
    if (lock.owner === null) { lock.owner = t; lock.count = 1; return undefined; }
    if (lock.owner === t) { lock.count++; return undefined; }
    return this.block(ctx, t, { lock, finite: false, label });
  }

  /** Enters a lock if no other thread owns it. */
  tryLock(ctx, lock) {
    const t = this.of(ctx);
    if (lock.owner !== null && lock.owner !== t) return false;
    lock.owner = t;
    lock.count++;
    return true;
  }

  /** Leaves a lock; at count 0 it passes to the first waiter, which becomes runnable. */
  unlock(ctx, lock) {
    const t = this.of(ctx);
    if (lock.owner !== t) ctx.fail(`${lock.name} is not held by thread ${hex(t.id)}${lock.owner ? ` (thread ${hex(lock.owner.id)} holds it)` : ''}`);
    if (--lock.count > 0) return;
    lock.owner = null;
    const next = lock.waiters.shift();
    if (next) {
      lock.owner = next;
      lock.count = 1;
      this.ready(next, undefined);
    }
  }

  /**
   * Thread `t` has ended with `code`: its handle is signaled and the next
   * thread runs. Returns an error message if no thread can run.
   */
  exit(t, code) {
    t.status = 'done';
    t.exitCode = code >>> 0;
    t.wait = null;
    this.byState.delete(t.state);
    if (this.current === t) this.current = null;
    this.signal();
    const next = this.pick();
    if (next) { this.switchTo(next); return null; }
    return this.live().length ? this.deadlock() : null;
  }
}

/** The threads of the process a host call runs in (a single-thread table in the shim test harness). */
export function threadsOf(ctx) {
  return ctx.host.threads ?? ctx.dllState('tempwind:threads', () => new Threads());
}
