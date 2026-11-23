// The JS host of Wine's Unix side (docs/design/wine-unix.md; abi.md v5,
// "Wine boundary"): one Emscripten instance of wine-unix (or of a test-NAME
// harness module) on the process's shared memory64.
//
// What it does:
// - creates the shared memory64: INITIAL_MEMORY = 4 GiB (65536 pages), maximum
//   262144 pages (16 GiB);
// - instantiates the module past Emscripten's glue through the
//   `instantiateWasm` hook: it supplies the imports of abi.md v5's table
//   (`green_block` as a WebAssembly.Suspending, `tw_run`; `green_wake` and
//   `memory_grown` are library functions that call back here) and keeps the
//   raw exports, which take and return BigInts;
// - sets ENV in preRun: wine-unix.md section 8's defaults, then the caller's;
// - makes fds 0-2 FIFOs (S_IFIFO, not a tty) that hand bytes to callbacks;
// - exposes MEMFS (`FS`) and the glue's Module;
// - enters a raw export through WebAssembly.promising (`call`);
// - schedules the green threads (wine-unix.md, section 9): it resumes the
//   threads C woke and starts the ones C spawned (M4), one at a time in the
//   order of the wakes and spawns; when none is runnable it waits
//   tw_next_timeout() milliseconds, runs tw_run_timeouts() and looks again;
//   with nothing runnable and no timeout it stops the waiting threads with a
//   deadlock error. Its direct raw calls run on a 64 KiB stack of its own
//   (M4);
// - settles the process once (wine-unix.md, section 8, "Process exit from
//   another thread"): `call`'s promise settles with the first of the entry's
//   return, a thread's TW_THREAD_PROCESS_EXIT (with tw_exit_status()), or the
//   rejection of any entry; then nothing in the module is resumed or started
//   again.
//
// The boot set and the entry into `boot` are beside it, in boot-set.mjs
// (wine-unix.md, section 8). A test may replace the scheduler with
// `greenBlock`.
// Host-neutral: the caller passes the glue's factory and the compiled module.

import { createMemory64 } from '../memory.mjs';

/** INITIAL_MEMORY of the module (wine-unix.md, section 5): 4 GiB. */
export const INITIAL_PAGES = 65536;
/** The memory's maximum (abi.md v5, "Instances"): 16 GiB. */
export const MAXIMUM_PAGES = 262144;

/** tw_thread_main's value when the process exited on that thread (abi.md v5, "Process exit"). */
export const TW_THREAD_PROCESS_EXIT = 1;
/** The scheduler's own stack (abi.md v5, "Green threads"): 64 KiB. */
export const SCHEDULER_STACK_SIZE = 0x10000;

/** wine-unix.md, section 8: the environment of the Unix side. */
export const DEFAULT_ENV = Object.freeze({
  WINEPREFIX: '/home/runner/wine-prefix',
  HOME: '/home/runner',
  USER: 'runner',
  LANG: 'en_US.UTF-8',
  TZ: 'UTC',
});

// Character-device majors for the three stdio devices (Emscripten's own
// createDevice counts up from 64).
const STDIO_MAJOR = 200;
/** The file-type bits of st_mode, and a FIFO's (POSIX; Emscripten's values). */
export const S_IFMT = 0o170000;
export const S_IFIFO = 0o010000;

/**
 * Instantiates a Unix-side module.
 * @param {object} opts
 * @param {Function} opts.factory   the default export of NAME.mjs (Emscripten's glue)
 * @param {WebAssembly.Module} opts.wasm  NAME.wasm, compiled
 * @param {Record<string,string>} [opts.env]  added to (or replacing) DEFAULT_ENV; WINEDEBUG passes through here
 * @param {(bytes: Uint8Array) => void} [opts.stdout]  bytes written to fd 1
 * @param {(bytes: Uint8Array) => void} [opts.stderr]  bytes written to fd 2 (and the glue's own messages)
 * @param {(max: number) => Uint8Array|null} [opts.stdin]  bytes for fd 0; null = end of file
 * @param {(thread: number) => Promise<bigint>|bigint} [opts.greenBlock]  green_block in place of the scheduler (tests)
 * @param {(thread: number) => void} [opts.onGreenWake]  green_wake with opts.greenBlock
 * @param {(thread: number, top: bigint, bottom: bigint) => void} [opts.onGreenSpawn]  green_spawn with opts.greenBlock
 * @param {(state: bigint) => number} [opts.run]  tw_run: the bridge's raw `run` (M3)
 * @param {Record<string, Function>} [opts.imports]  more `env` imports, replacing the defaults of
 *   M3's imports (abi.md v5, "Host entries of the Unix side"): the bridge's raw exports for
 *   tw_depth_get, tw_depth_set and tw_flags_materialize, and the host's tw_translate,
 *   tw_image_mapped and tw_invalidate. Without them tw_flags_materialize and tw_translate stop
 *   loudly, tw_depth_get and tw_depth_set keep a cell of the host's (0 unless set), and
 *   tw_image_mapped and tw_invalidate do nothing, which is right for a host that links no
 *   translated code (the boot and harness tests).
 * @param {() => void} [opts.onMemoryGrown]  after the module grew the memory
 * @param {WebAssembly.Memory} [opts.memory]  the process's memory, if it exists already
 */
export async function createUnixHost(opts) {
  const { factory, wasm } = opts;
  const memory = opts.memory ??
    createMemory64({ initialPages: INITIAL_PAGES, maximumPages: MAXIMUM_PAGES, shared: true }).memory;
  const env = { ...DEFAULT_ENV, ...(opts.env ?? {}) };
  const sink = (f) => f ?? (() => {});
  const stdout = sink(opts.stdout);
  const stderr = sink(opts.stderr);
  const stdin = opts.stdin ?? (() => null);
  const encoder = new TextEncoder();

  let raw = null;
  const scheduler = opts.greenBlock ? null : createScheduler(() => raw, stderr, encoder);
  const greenBlock = opts.greenBlock ?? scheduler.block;
  const greenWake = scheduler ? scheduler.wake : (thread) => {
    if (!opts.onGreenWake) throw new Error(`green_wake(${thread}): no scheduler in this host`);
    opts.onGreenWake(thread);
  };
  const greenSpawn = scheduler ? scheduler.spawn : (thread, top, bottom) => {
    if (!opts.onGreenSpawn) throw new Error(`green_spawn(${thread}): no scheduler in this host`);
    opts.onGreenSpawn(thread, top, bottom);
  };
  const run = opts.run ?? ((state) => {
    throw new Error(`tw_run(0x${BigInt.asUintN(64, state).toString(16)}): no translated module (M3)`);
  });
  const noBridge = (name) => () => {
    throw new Error(`${name}: no bridge in this host (abi.md v5, "The bridge")`);
  };
  // Without a bridge, env.depth is a cell of this host's: green.c saves and
  // restores it at every switch (abi.md v5, "Green threads"), and a host
  // that links no translated code keeps it at 0.
  let depthCell = 0;
  const m3Imports = {
    tw_depth_get: () => depthCell,
    tw_depth_set: (d) => { depthCell = d; },
    tw_flags_materialize: noBridge('tw_flags_materialize'),
    tw_translate: (rip) => {
      throw new Error(`tw_translate(0x${BigInt.asUintN(64, rip).toString(16)}): no translator in this host`);
    },
    tw_image_mapped: () => {},
    tw_invalidate: () => {},
    ...(opts.imports ?? {}),
  };

  const Module = {
    wasmMemory: memory,
    noFSInit: true, // the stdio devices replace FS.init's (preRun below)
    print: (s) => stderr(encoder.encode(s + '\n')),
    printErr: (s) => stderr(encoder.encode(s + '\n')),
    instantiateWasm(imports, receive) {
      Object.assign(imports.env, {
        green_block: new WebAssembly.Suspending(greenBlock),
        tw_run: run,
        ...m3Imports,
      });
      // Synchronous, as the spike: the caller compiled the module already.
      const instance = new WebAssembly.Instance(wasm, imports);
      raw = instance.exports;
      receive(instance, wasm);
      return {};
    },
    preRun: [() => {
      Object.assign(Module.ENV, env);
      createStdio(Module.FS, { stdin, stdout, stderr });
    }],
    onGreenWake: greenWake,
    onGreenSpawn: greenSpawn,
    onMemoryGrown: () => opts.onMemoryGrown?.(),
  };
  await factory(Module);
  // The scheduler's own stack, from the raw malloc before any entry runs.
  scheduler?.init();

  const promised = new Map();
  return {
    memory,
    /** The raw exports (BigInt pointers); never the glue's wrappers. */
    raw,
    /** Emscripten's Module, after instantiation. */
    Module,
    /** MEMFS and the rest of Emscripten's FS. */
    FS: Module.FS,
    env,
    /**
     * Enters a raw export through WebAssembly.promising (abi.md v5, "Who
     * calls whom"): the process's main thread. With the scheduler the promise
     * is the process's: it settles once, with the first of the entry's
     * return, a thread's process exit (its status from tw_exit_status), or
     * any entry's rejection.
     */
    call(name, ...args) {
      if (!promised.has(name)) {
        if (typeof raw[name] !== 'function') throw new Error(`no raw export ${name}`);
        promised.set(name, WebAssembly.promising(raw[name]));
      }
      if (!scheduler) return promised.get(name)(...args);
      return scheduler.enter(() => promised.get(name)(...args));
    },
  };
}

// The green-thread scheduler (wine-unix.md, sections 8 and 9; abi.md v5,
// "Green threads" and "Host entries of the Unix side"). A green thread is a
// promising entry in flight: the main thread is `enter`'s, a spawned one
// tw_thread_main's. One runs at a time; it runs until it blocks in
// green_block or its entry settles. C calls green_wake for a thread it made
// runnable and green_spawn for a thread it made; the scheduler resumes or
// starts them after the current call into the module returns, one at a
// time, in the order of the wakes and spawns (a wake of a thread already
// queued changes nothing). Only when no thread runs and none is queued does
// it ask C for the next timeout. Its direct raw calls (tw_next_timeout,
// tw_run_timeouts, tw_exit_status) run on its own 64 KiB stack, never on the
// stack of a thread that has ended, whose region Wine frees.
function createScheduler(getRaw, stderr, encoder) {
  const blocked = new Map(); // thread -> { resolve, reject }
  const queue = [];          // FIFO of { thread, spawn: { top, bottom } | null }
  const queuedWakes = new Set();
  let running = null;        // the thread that runs (or is being resumed)
  let queued = false;
  let timer = null;
  let stopped = null;        // the error every later call gets
  let process = null;        // { resolve, reject } of enter's promise, until it settles
  let settled = false;
  let stack = null;          // { top, bottom } of the scheduler's stack
  let threadMain = null;     // promising(tw_thread_main)

  const schedule = () => {
    if (queued) return;
    queued = true;
    queueMicrotask(dispatch); // after the suspension or the return completes
  };

  // A direct raw call while no thread runs, on the scheduler's own stack.
  function direct(name) {
    const raw = getRaw();
    raw.emscripten_stack_set_limits(stack.top, stack.bottom);
    raw._emscripten_stack_restore(stack.top);
    return raw[name]();
  }

  function settle(ok, value) {
    if (settled) return;
    settled = true;
    if (timer !== null) { clearTimeout(timer); timer = null; }
    queue.length = 0;
    queuedWakes.clear();
    const p = process;
    process = null;
    if (ok) p.resolve(value); else p.reject(value);
  }

  // An entry settled: its thread no longer runs.
  function ended(thread) {
    if (running === thread) running = null;
    schedule();
  }

  function start(thread, { top, bottom }) {
    const raw = getRaw();
    running = thread;
    threadMain ??= WebAssembly.promising(raw.tw_thread_main);
    let p;
    try {
      raw.emscripten_stack_set_limits(top, bottom);
      raw._emscripten_stack_restore(top);
      p = threadMain(thread);
    } catch (e) {
      ended(thread);
      return settle(false, e);
    }
    p.then((ret) => {
      ended(thread);
      if (ret === TW_THREAD_PROCESS_EXIT) {
        let status;
        try {
          status = direct('tw_exit_status');
        } catch (e) {
          return settle(false, e);
        }
        settle(true, status);
      } else if (ret !== 0) {
        settle(false, new Error(`tempwind: green: tw_thread_main(${thread}) returned ${ret}`));
      }
    }, (e) => {
      ended(thread);
      settle(false, e);
    });
  }

  function dispatch() {
    queued = false;
    if (settled || running !== null) return;
    if (timer !== null) { clearTimeout(timer); timer = null; }
    while (queue.length) {
      const { thread, spawn } = queue.shift();
      if (spawn) return start(thread, spawn);
      queuedWakes.delete(thread);
      const b = blocked.get(thread);
      if (!b) continue; // it ended, or never blocked: nothing to resume
      blocked.delete(thread);
      running = thread;
      b.resolve(0n);
      return;
    }
    if (blocked.size === 0) return; // nothing in flight
    let ms;
    try {
      ms = Number(direct('tw_next_timeout'));
    } catch (e) {
      return stop(e);
    }
    if (ms < 0) {
      const threads = [...blocked.keys()].join(', ');
      const msg = `tempwind: green: deadlock: nothing is runnable and no timeout is pending (waiting: thread ${threads})`;
      stderr(encoder.encode(msg + '\n'));
      return stop(new Error(msg));
    }
    timer = setTimeout(() => {
      timer = null;
      if (settled) return;
      try {
        direct('tw_run_timeouts');
      } catch (e) {
        return stop(e);
      }
      schedule();
    }, ms);
  }

  // Throws err into every blocked thread: its green_block throws, and its
  // promising entry rejects, which settles the process.
  // After it, the module's waiter lists point into frames that are gone, so
  // no call may enter it again: enter and block reject, wake is ignored.
  function stop(err) {
    if (!stopped) stopped = new Error(`tempwind: green: stopped after a deadlock: ${err.message}`);
    queue.length = 0;
    queuedWakes.clear();
    for (const [thread, b] of blocked) {
      blocked.delete(thread);
      b.reject(err);
    }
    if (process) settle(false, err); // no entry was blocked to carry it
  }

  return {
    init() {
      const raw = getRaw();
      const p = raw.malloc(BigInt(SCHEDULER_STACK_SIZE + 16));
      if (p === 0n) throw new Error('tempwind: green: no host memory for the scheduler\'s stack');
      const bottom = (p + 15n) & ~15n;
      stack = { top: bottom + BigInt(SCHEDULER_STACK_SIZE), bottom };
    },
    block(thread) {
      if (stopped) return Promise.reject(stopped);
      return new Promise((resolve, reject) => {
        blocked.set(thread, { resolve, reject });
        if (running === thread) running = null;
        schedule();
      });
    },
    wake(thread) {
      if (stopped || settled || queuedWakes.has(thread)) return;
      queuedWakes.add(thread);
      queue.push({ thread, spawn: null });
      schedule();
    },
    spawn(thread, top, bottom) {
      if (stopped || settled) return;
      queue.push({ thread, spawn: { top, bottom } });
      schedule();
    },
    enter(startEntry) {
      if (stopped) return Promise.reject(stopped);
      if (settled) return Promise.reject(new Error('tempwind: green: the process has ended: an instance runs one process'));
      if (process) return Promise.reject(new Error('tempwind: green: an entry is already in flight'));
      const result = new Promise((resolve, reject) => { process = { resolve, reject }; });
      running = MAIN_THREAD;
      let p;
      try {
        p = startEntry();
      } catch (e) {
        ended(MAIN_THREAD);
        settle(false, e);
        return result;
      }
      p.then((ret) => { ended(MAIN_THREAD); settle(true, ret); },
        (e) => { ended(MAIN_THREAD); settle(false, e); });
      return result;
    },
  };
}

/** The main green thread's id (green_pthread.c). */
const MAIN_THREAD = 1;

// fds 0-2: FIFOs (wine-unix.md, section 8, "stdio, as FIFOs"). Each is a
// device node from mkdev whose mode is then set to S_IFIFO: MEMFS refuses
// FIFO nodes, but I/O keeps going to the device's callbacks by rdev
// (research.md, 2026-11-14). So fstat reports S_IFIFO and Windows sees pipes:
// get_device_info reports FILE_DEVICE_NAMED_PIPE (unix/file.c L7215-7218),
// GetFileType FILE_TYPE_PIPE (kernelbase/file.c L3297-3309), and msvcrt
// marks the stream WX_PIPE (msvcrt/file.c L755-766), as a native run whose
// output a pipe captures. No tty, so isatty() is false, and Wine passes them
// to the server as plain handles (env.c L1363-1370).
function createStdio(FS, { stdin, stdout, stderr }) {
  const device = (minor, path, ops, flags) => {
    let pending = null; // what ops.read returned beyond the last read's length
    const dev = FS.makedev(STDIO_MAJOR, minor);
    FS.registerDevice(dev, {
      open(stream) { stream.seekable = false; },
      close() {},
      read(stream, buffer, offset, length) {
        if (!ops.read) throw new FS.ErrnoError(28 /* EINVAL */);
        const bytes = pending ?? ops.read(length);
        pending = null;
        if (!bytes || bytes.length === 0) return 0;
        const n = Math.min(length, bytes.length);
        buffer.set(bytes.subarray(0, n), offset);
        if (n < bytes.length) pending = bytes.subarray(n);
        return n;
      },
      write(stream, buffer, offset, length) {
        if (!ops.write) throw new FS.ErrnoError(28 /* EINVAL */);
        if (length > 0) ops.write(buffer.slice(offset, offset + length));
        return length;
      },
    });
    const node = FS.mkdev(path, 0o666, dev);
    node.mode = S_IFIFO | 0o666;
    return FS.open(path, flags);
  };
  const fds = [
    device(0, '/dev/stdin', { read: stdin }, 0 /* O_RDONLY */),
    device(1, '/dev/stdout', { write: stdout }, 1 /* O_WRONLY */),
    device(2, '/dev/stderr', { write: stderr }, 1 /* O_WRONLY */),
  ].map((s) => s.fd);
  if (fds.join() !== '0,1,2') throw new Error(`stdio devices opened as fds ${fds}, expected 0,1,2`);
  FS.initialized = true; // what FS.init would have set (FS.quit flushes the streams)
}
