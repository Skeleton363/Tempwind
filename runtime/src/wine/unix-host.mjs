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
// - makes fds 0-2 non-tty character devices that hand bytes to callbacks;
// - exposes MEMFS (`FS`) and the glue's Module;
// - enters a raw export through WebAssembly.promising (`call`).
//
// Hooks for later packages: the scheduler (WP3) passes `greenBlock` and
// `onGreenWake`; the boot set (WP4) writes into `host.FS` before `call('boot')`.
// Host-neutral: the caller passes the glue's factory and the compiled module.

import { createMemory64 } from '../memory.mjs';

/** INITIAL_MEMORY of the module (wine-unix.md, section 5): 4 GiB. */
export const INITIAL_PAGES = 65536;
/** The memory's maximum (abi.md v5, "Instances"): 16 GiB. */
export const MAXIMUM_PAGES = 262144;

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

/**
 * Instantiates a Unix-side module.
 * @param {object} opts
 * @param {Function} opts.factory   the default export of NAME.mjs (Emscripten's glue)
 * @param {WebAssembly.Module} opts.wasm  NAME.wasm, compiled
 * @param {Record<string,string>} [opts.env]  added to (or replacing) DEFAULT_ENV; WINEDEBUG passes through here
 * @param {(bytes: Uint8Array) => void} [opts.stdout]  bytes written to fd 1
 * @param {(bytes: Uint8Array) => void} [opts.stderr]  bytes written to fd 2 (and the glue's own messages)
 * @param {(max: number) => Uint8Array|null} [opts.stdin]  bytes for fd 0; null = end of file
 * @param {(thread: number) => Promise<bigint>|bigint} [opts.greenBlock]  green_block (the scheduler, WP3)
 * @param {(thread: number) => void} [opts.onGreenWake]  green_wake
 * @param {(state: bigint) => number} [opts.run]  tw_run: a translated module's raw `run` (M3)
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

  const greenBlock = opts.greenBlock ?? ((thread) => {
    throw new Error(`green_block(${thread}): no scheduler in this host (WP3)`);
  });
  const run = opts.run ?? ((state) => {
    throw new Error(`tw_run(0x${BigInt.asUintN(64, state).toString(16)}): no translated module (M3)`);
  });

  let raw = null;
  const Module = {
    wasmMemory: memory,
    noFSInit: true, // the stdio devices replace FS.init's (preRun below)
    print: (s) => stderr(encoder.encode(s + '\n')),
    printErr: (s) => stderr(encoder.encode(s + '\n')),
    instantiateWasm(imports, receive) {
      Object.assign(imports.env, {
        green_block: new WebAssembly.Suspending(greenBlock),
        tw_run: run,
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
    onGreenWake: (thread) => {
      if (!opts.onGreenWake) throw new Error(`green_wake(${thread}): no scheduler in this host (WP3)`);
      opts.onGreenWake(thread);
    },
    onMemoryGrown: () => opts.onMemoryGrown?.(),
  };
  await factory(Module);

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
    /** Enters a raw export through WebAssembly.promising (abi.md v5, "Who calls whom"). */
    call(name, ...args) {
      if (!promised.has(name)) {
        if (typeof raw[name] !== 'function') throw new Error(`no raw export ${name}`);
        promised.set(name, WebAssembly.promising(raw[name]));
      }
      return promised.get(name)(...args);
    },
  };
}

// fds 0-2: non-tty character devices (wine-unix.md, section 8). No tty, so
// isatty() is false, libc buffers stdout fully, and Wine passes them to the
// server as plain handles.
function createStdio(FS, { stdin, stdout, stderr }) {
  const device = (minor, path, ops, flags) => {
    const dev = FS.makedev(STDIO_MAJOR, minor);
    FS.registerDevice(dev, {
      open(stream) { stream.seekable = false; },
      close() {},
      read(stream, buffer, offset, length) {
        if (!ops.read) throw new FS.ErrnoError(28 /* EINVAL */);
        const bytes = ops.read(length);
        if (!bytes || bytes.length === 0) return 0;
        const n = Math.min(length, bytes.length);
        buffer.set(bytes.subarray(0, n), offset);
        return n;
      },
      write(stream, buffer, offset, length) {
        if (!ops.write) throw new FS.ErrnoError(28 /* EINVAL */);
        if (length > 0) ops.write(buffer.slice(offset, offset + length));
        return length;
      },
    });
    FS.mkdev(path, 0o666, dev);
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
