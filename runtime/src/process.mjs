// A Phase 1 process (docs/design/runtime.md): guest memory and its address
// map, the mapped image, import thunks and the IAT, PEB/TEB/stack/TLS, the
// linker, the host-call function and the run loop. Standard APIs only, so it
// runs in Node, a page or a Worker; the CLI is run-pe.mjs.

import {
  PAGE_SIZE, STATE_RIP, STATE_RFLAGS, STATE_EXIT, STATE_SIZE, STATE_GS_BASE, STATE_FS_BASE,
  STATE_FAULT_ADDR, STATE_FAULT_KIND, EXIT_LEFT_CODE, EXIT_UNSUPPORTED, EXIT_FAULT, EXIT_HOST,
  FAULT_READ, FAULT_WRITE, FAULT_EXECUTE, SENTINEL_RET, LOOKUP_HEADER_ADDR, MAIN_STATE_ADDR,
  THUNK_BASE, THUNK_SIZE, THUNK_BYTES, HOST_FUNC_INDEX, MAX_THUNKS, MIN_LOOKUP_SLOTS, LOOKUP_SLOT_SIZE,
  PEB_ADDR, TEB_ADDR, PROCESS_ID, THREAD_ID, RFLAGS_INIT, GUEST_CALL_FRAME, DLL_PROCESS_ATTACH,
  STATE_XMM, STATE_MXCSR, STATE_FCW, STATE_FTW, STATE_FP_END, MXCSR_INIT, FCW_INIT, FTW_EMPTY,
  TEB_STACK_BASE, TEB_STACK_LIMIT, TEB_SELF, TEB_CLIENT_ID_PROCESS, TEB_CLIENT_ID_THREAD,
  TEB_TLS_POINTER, TEB_PEB, PEB_BEING_DEBUGGED, PEB_IMAGE_BASE, PEB_NUMBER_OF_PROCESSORS,
  PEB_OS_MAJOR, PEB_OS_MINOR, PEB_OS_BUILD, PEB_OS_PLATFORM_ID, OS_VERSION, gprOffset,
  DLL_THREAD_ATTACH, DLL_THREAD_DETACH, TEB_SIZE, STACK_ALIGN,
} from './abi.mjs';
import { createMemory64 } from './memory.mjs';
import { GuestMemory } from './guest-memory.mjs';
import { Linker, moduleBlockCount } from './link.mjs';
import { makeHostCall, HostStop } from './hostcall.mjs';
import { parsePlan, checkPlan, executableSection, hex } from './plan.mjs';
import { findShim, SHIM } from './win32/index.mjs';
import { Vfs, hostNow } from './win32/vfs.mjs';
import { Thread, Threads } from './threads.mjs';

const RCX = 1, RDX = 2, RSP = 4, R8 = 8, R9 = 9;
const ARG_REGS = [RCX, RDX, R8, R9];
const EXIT_NAMES = { [EXIT_LEFT_CODE]: 'LEFT_CODE', [EXIT_UNSUPPORTED]: 'UNSUPPORTED', [EXIT_FAULT]: 'FAULT', [EXIT_HOST]: 'HOST' };
const FAULT_NAMES = { [FAULT_READ]: 'read', [FAULT_WRITE]: 'write', [FAULT_EXECUTE]: 'execute' };
const now = () => performance.now();
/** Calls from the host into guest code nest at most this deep (runtime.md). */
export const MAX_GUEST_CALL_NESTING = 16;
/** JSPI (runtime.md, "Guest threads"): Node 24, Chromium 137+. */
const JSPI = typeof WebAssembly.Suspending === 'function' && typeof WebAssembly.promising === 'function';
/** A thread's stack reservation is at least 1 MiB (Windows: RtlCreateUserStack). */
const MIN_STACK_RESERVE = 0x10_0000n;
const roundUp = (v, a) => (v + a - 1n) / a * a;

/** An error stop of the process (exit status 125 in the CLI). */
export class ProcessError extends Error {}

/** A bump allocator over [base, limit) of guest memory (fresh memory is zero). */
export class Bump {
  constructor(name, base, limit) {
    this.name = name;
    this.next = base;
    this.limit = limit;
  }

  alloc(size, align = 16n) {
    const at = (this.next + align - 1n) / align * align;
    if (at + size > this.limit) throw new ProcessError(`${this.name} exhausted: ${hex(size)} bytes at ${hex(at)}, limit ${hex(this.limit)}`);
    this.next = at + size;
    return at;
  }
}

const nextPow2 = (n) => { let p = 1; while (p < n) p *= 2; return p; };

export class Process {
  /**
   * Loads a program (runtime.md, "Loading", steps 1-7). Does not run it.
   * @param {{exe: Uint8Array, plan: object, aot: BufferSource | WebAssembly.Module,
   *   rt?: import('./rt.mjs').RuntimeTranslator | null,
   *   trace?: {api?: boolean, translate?: boolean, exits?: boolean}, log?: (line: string) => void,
   *   onOutput?: (stream: 'stdout' | 'stderr', bytes: Uint8Array) => void, shim?: object,
   *   argv?: string[], fp?: WebAssembly.Exports | null}} opts
   *   `argv`: the command line (default: the program name, the plan's `file`).
   *   `fp`: the exports of x64wasm_fp.wasm (loadFp), instantiated once for this process.
   */
  static async load(opts) {
    const p = new Process(opts);
    await p.init(opts);
    return p;
  }

  constructor({ rt = null, trace = {}, log = () => {}, onOutput = () => {}, shim = SHIM, argv = null, fp = null, vfs = null }) {
    this.rt = rt;
    this.fp = fp;
    this.trace = { api: false, translate: false, exits: false, ...trace };
    this.log = log;
    this.onOutput = onOutput;
    this.shim = shim;
    this.output = { stdout: [], stderr: [] };
    this.stop = null;
    this.running = false;
    /** Set once the process has ended (exit code or error stop). */
    this.finished = false;
    /** Depth of calls from host functions into guest code. */
    this.nesting = 0;
    this.argvOption = argv;
    /** Per-process state of the shim DLLs (HostContext.dllState). */
    this.dllStates = new Map();
    /** Data import storage by `dll!name` (DLL lower-cased). */
    this.dataImports = new Map();
    /** The file system (runtime.md, "The file system"). */
    this.vfs = vfs ?? new Vfs();
    this.startTime = hostNow();
    /** GetProcAddress thunks by `dll!name`. */
    this.procThunks = new Map();
    this.translations = 0;
    /** Total milliseconds of run-time translation: translate, compile, link. */
    this.translateMs = 0;
  }

  async init({ exe, plan, aot }) {
    /** Milliseconds per load phase: check (plan, sha256), compile, memory (create, map), link. */
    this.timings = {};
    let t = now();
    const lap = (name) => { const t1 = now(); this.timings[name] = t1 - t; t = t1; };
    // 1. The plan against the file.
    this.plan = parsePlan(plan);
    this.map = await checkPlan(this.plan, exe);
    this.argv = this.argvOption ?? [this.plan.file];
    /** The program in the file system, read-only: the plan's file name in C:\\tempwind. */
    this.programPath = `C:\\tempwind\\${this.plan.file}`;
    this.vfs.addFile(this.programPath, exe, { attrs: 0x21 }).dirty = false;
    lap('check');
    let aotModule, aotBlocks;
    try {
      aotModule = aot instanceof WebAssembly.Module ? aot : await WebAssembly.compile(aot);
      aotBlocks = moduleBlockCount(aotModule);
    } catch (e) {
      throw new ProcessError(`AOT module: ${e.message}`);
    }
    lap('compile');

    // 2. Memory and the image.
    const { memory } = createMemory64({ initialPages: Number(this.map.memoryBytes / BigInt(PAGE_SIZE)) });
    this.memory = memory;
    this.mem = new GuestMemory(memory);
    this.mapImage(exe);
    this.guestHeap = new Bump('guest heap', this.map.guestHeap.base, this.map.guestHeap.limit);
    this.runtimeHeap = new Bump('runtime heap', this.map.runtimeHeap.base, this.map.runtimeHeap.limit);
    lap('memory');

    // 3. The linker: header, slots for the expected keys, host-call function at slot 0.
    const imports = this.plan.imports.flatMap((d) => d.functions.map((f) => ({ dll: d.dll, ...f })));
    const slots = Math.max(MIN_LOOKUP_SLOTS, nextPow2(2 * (aotBlocks + imports.length)));
    this.linker = new Linker({
      memory, header: LOOKUP_HEADER_ADDR, slots,
      slotsAddr: this.runtimeHeap.alloc(BigInt(slots * LOOKUP_SLOT_SIZE), 16n),
      alloc: (size, align) => this.runtimeHeap.alloc(size, align),
      fp: this.fp,
    });
    this.imports = imports.map((f, id) => ({ ...f, thunk: THUNK_BASE + THUNK_SIZE * BigInt(id), impl: findShim(f.dll, f.name, this.shim) }));
    const hostCall = makeHostCall({
      mem: this.mem,
      imports: this.imports,
      output: (stream, bytes) => this.write(stream, bytes),
      onStop: (stop) => { this.stop = stop; },
      trace: this.trace.api ? (line) => this.log(line) : undefined,
      host: this.services(),
    });
    this.linker.setHostFunction(hostCall);
    // Calls that can block (runtime.md, "Guest threads") go through a second
    // host function wrapped in WebAssembly.Suspending. Inside a call from the
    // host into guest code its slot holds the plain one (callNested).
    this.hostFn = this.linker.table.get(HOST_FUNC_INDEX);
    this.blockingIndex = this.linker.addHostFunction(JSPI ? new WebAssembly.Suspending(hostCall) : hostCall);
    this.blockingFn = this.linker.table.get(this.blockingIndex);
    this.threads = new Threads({
      depth: this.linker.depth,
      nesting: () => this.nesting,
      canSuspend: JSPI,
      onEnter: (t) => { this.state = t.state; },
    });

    // 4. Imports: thunks, lookup slots, IAT.
    this.writeImports();

    // 5. The AOT module.
    try {
      this.aot = await this.linker.link(aotModule);
    } catch (e) {
      throw new ProcessError(`linking the AOT module: ${e.message}`);
    }
    this.run = this.aot.run;
    lap('link');

    // 6. PEB, TEB, stack, and the main thread's FP state (the rest of the
    // state block is fresh, zeroed memory).
    this.state = MAIN_STATE_ADDR;
    this.writePebTeb();
    this.initFpState();
    this.mainThread = this.threads.add(new Thread({
      id: THREAD_ID, state: MAIN_STATE_ADDR, teb: TEB_ADDR, stackBase: this.map.stackTop, stackLimit: this.map.stackBottom, main: true,
    }));

    // 7. TLS data (callbacks run in start()).
    if (this.plan.tls) this.tlsBlock = this.setupTls(TEB_ADDR);
  }

  mapImage(exe) {
    const { imageBase, sizeOfHeaders, sections } = this.plan;
    this.mem.write(imageBase, exe.subarray(0, Number(sizeOfHeaders)));
    for (const s of sections) {
      const off = Number(s.fileOffset);
      this.mem.write(s.va, exe.subarray(off, off + Number(s.copySize)));
    }
  }

  /** What host functions reach through HostContext (hostcall.mjs, HostServices). */
  services() {
    const p = this;
    return {
      call: (target, args) => p.callNested(target, args),
      createThread: (opts) => p.createThread(opts),
      get threads() { return p.threads; },
      alloc: (size, align) => {
        try { return p.guestHeap.alloc(size, align); } catch (e) { throw new HostStop({ error: e.message }); }
      },
      dllState: (dll, init) => {
        if (!p.dllStates.has(dll)) p.dllStates.set(dll, init());
        return p.dllStates.get(dll);
      },
      dataAddress: (dll, name) => p.dataAddress(dll, name),
      procAddress: (dll, name) => p.procAddress(dll, name),
      heapFree: () => p.guestHeap.limit - p.guestHeap.next,
      get maxAppAddress() { return p.map.memoryBytes - 0x1_0001n; },
      get vfs() { return p.vfs; },
      get startTime() { return p.startTime; },
      get programPath() { return p.programPath; },
      get argv() { return p.argv; },
      get image() {
        const { imageBase: base, sizeOfImage: size, sizeOfHeaders, sections } = p.plan;
        return { base, size, sizeOfHeaders, sections };
      },
    };
  }

  /**
   * A thunk for shim function `dll!name` (GetProcAddress), created on first
   * use like an import's: a host-call thunk with a lookup slot. Undefined if
   * the shim lacks the function.
   */
  procAddress(dll, name) {
    const key = `${dll.toLowerCase()}!${name}`;
    if (this.procThunks.has(key)) return this.procThunks.get(key);
    const impl = findShim(dll, name, this.shim);
    if (!impl?.fn) return undefined;
    const id = this.imports.length;
    if (id >= MAX_THUNKS) throw new HostStop({ error: `more than ${MAX_THUNKS} import thunks` });
    const thunk = THUNK_BASE + THUNK_SIZE * BigInt(id);
    this.imports.push({ dll, name, thunk, impl, iat: 0n });
    this.mem.write(thunk, THUNK_BYTES);
    this.linker.insert(thunk, impl.blocks ? this.blockingIndex : HOST_FUNC_INDEX, id);
    this.procThunks.set(key, thunk);
    return thunk;
  }

  /** The storage of data import `dll!name`, or undefined. */
  dataAddress(dll, name) { return this.dataImports.get(`${dll.toLowerCase()}!${name}`); }

  /** Storage for a data import (runtime.md, "Loading", step 4): `{ data: { size, init } }`. */
  allocDataImport(f) {
    const { size, init } = f.impl.data;
    const label = `${f.dll.toLowerCase()}!${f.name}`;
    const addr = this.guestHeap.alloc(BigInt(size), 16n);
    let v = typeof init === 'function' ? init(this.services()) : init;
    if (v !== undefined) {
      if (typeof v === 'number' || typeof v === 'bigint') {
        let n = BigInt.asUintN(8 * size, BigInt(v));
        const bytes = new Uint8Array(size);
        for (let i = 0; i < size; i++, n >>= 8n) bytes[i] = Number(n & 0xffn);
        v = bytes;
      }
      if (!(v instanceof Uint8Array)) throw new ProcessError(`data import ${label}: init is not bytes or a number`);
      if (v.length > size) throw new ProcessError(`data import ${label}: init has ${v.length} bytes, size is ${size}`);
      this.mem.write(addr, v);
    }
    this.dataImports.set(label, addr);
    return addr;
  }

  writeImports() {
    for (const [id, f] of this.imports.entries()) {
      if (f.impl?.data) {
        this.mem.setU64(f.iat, this.allocDataImport(f));
        continue;
      }
      this.mem.write(f.thunk, THUNK_BYTES);
      this.linker.insert(f.thunk, f.impl?.blocks ? this.blockingIndex : HOST_FUNC_INDEX, id);
      this.mem.setU64(f.iat, f.thunk);
    }
  }

  writePebTeb() {
    const m = this.mem;
    const { stackTop, stackBottom } = this.map;
    m.setU64(TEB_ADDR + BigInt(TEB_STACK_BASE), stackTop);
    m.setU64(TEB_ADDR + BigInt(TEB_STACK_LIMIT), stackBottom);
    m.setU64(TEB_ADDR + BigInt(TEB_SELF), TEB_ADDR);
    m.setU64(TEB_ADDR + BigInt(TEB_CLIENT_ID_PROCESS), PROCESS_ID);
    m.setU64(TEB_ADDR + BigInt(TEB_CLIENT_ID_THREAD), THREAD_ID);
    m.setU64(TEB_ADDR + BigInt(TEB_PEB), PEB_ADDR);
    m.setU8(PEB_ADDR + BigInt(PEB_BEING_DEBUGGED), 0);
    m.setU64(PEB_ADDR + BigInt(PEB_IMAGE_BASE), this.plan.imageBase);
    m.setU32(PEB_ADDR + BigInt(PEB_NUMBER_OF_PROCESSORS), 1);
    m.setU32(PEB_ADDR + BigInt(PEB_OS_MAJOR), OS_VERSION.major);
    m.setU32(PEB_ADDR + BigInt(PEB_OS_MINOR), OS_VERSION.minor);
    m.setU32(PEB_ADDR + BigInt(PEB_OS_BUILD), OS_VERSION.build);
    m.setU32(PEB_ADDR + BigInt(PEB_OS_PLATFORM_ID), OS_VERSION.platformId);
  }

  /** The image's TLS data for the thread of `teb`: a block from the template and the pointer array. Returns the block. */
  setupTls(teb) {
    const t = this.plan.tls;
    const dataSize = t.endOfRawData - t.startOfRawData;
    const block = this.guestHeap.alloc(dataSize + t.sizeOfZeroFill || 1n, 16n);
    if (dataSize) this.mem.write(block, this.mem.read(t.startOfRawData, Number(dataSize)));
    const array = this.guestHeap.alloc(8n, 16n);
    this.mem.setU64(array, block);
    this.mem.setU64(teb + BigInt(TEB_TLS_POINTER), array);
    this.mem.setU32(t.addressOfIndex, 0);
    return block;
  }

  write(stream, bytes) {
    this.output[stream].push(bytes);
    this.onOutput(stream, bytes);
  }

  /** Captured output of a stream, as bytes. */
  captured(stream) {
    const chunks = this.output[stream];
    const out = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0));
    let at = 0;
    for (const c of chunks) { out.set(c, at); at += c.length; }
    return out;
  }

  reg(i) { return this.mem.u64(this.state + BigInt(gprOffset(i))); }
  setReg(i, v) { this.mem.setU64(this.state + BigInt(gprOffset(i)), v); }

  /** The FP state of a new thread, as on Windows x64 (runtime.md, "Start of the main thread"). */
  initFpState(state = this.state) {
    const m = this.mem;
    m.setU32(state + BigInt(STATE_MXCSR), MXCSR_INIT);
    m.setU16(state + BigInt(STATE_FCW), FCW_INIT);
    m.setU16(state + BigInt(STATE_FTW), FTW_EMPTY);
  }

  /**
   * Sets up a host-initiated guest call on thread `t` (runtime.md, "Start of
   * the main thread"): fresh GPRs, Win64 register arguments, RSP = top - 0x38
   * with [RSP] = SENTINEL_RET, RFLAGS = 0x202, GS base = TEB, FS base = 0.
   * The FP state (XMM, MXCSR, x87) is the thread's and carries over, as from
   * a TLS callback to the entry point on Windows.
   */
  prepareCall(target, args = [], t = this.mainThread) {
    if (args.length > 4) throw new ProcessError('host-initiated guest calls take at most 4 arguments');
    const m = this.mem;
    const state = t.state;
    const fpAt = state + BigInt(STATE_XMM);
    const fp = m.read(fpAt, STATE_FP_END - STATE_XMM);
    m.fill(state, Number(STATE_SIZE));
    m.write(fpAt, fp);
    const top = t.stackBase & ~0xfn;
    const rsp = top - GUEST_CALL_FRAME;
    const reg = (i, v) => m.setU64(state + BigInt(gprOffset(i)), v);
    args.forEach((a, i) => reg(ARG_REGS[i], BigInt.asUintN(64, BigInt(a))));
    reg(RSP, rsp);
    m.setU64(rsp, SENTINEL_RET);
    m.setU64(state + BigInt(STATE_RIP), target);
    m.setU64(state + BigInt(STATE_RFLAGS), RFLAGS_INIT);
    m.setU64(state + BigInt(STATE_GS_BASE), t.teb);
    m.setU64(state + BigInt(STATE_FS_BASE), 0n);
  }

  /**
   * Runs a host-initiated guest call on thread `t` to its end.
   * @returns {Promise<{returned: true, rax: bigint} | {exitCode: number} | {threadExit: number}>}
   * Throws ProcessError on an error stop.
   */
  async callOn(t, target, args = []) {
    this.prepareCall(target, args, t);
    return await this.loop(t);
  }

  /**
   * `run` for a thread's top level: through WebAssembly.promising, so a
   * blocking host call can suspend the thread (runtime.md, "Guest threads").
   * A `run` that isn't a wasm function (tests) is called directly.
   */
  runTop(state) {
    if (this.promisingFor !== this.run) {
      this.promisingFor = this.run;
      try { this.promisingRun = JSPI ? WebAssembly.promising(this.run) : this.run; } catch { this.promisingRun = this.run; }
    }
    return this.promisingRun(state);
  }

  /**
   * The run loop (runtime.md, "Run loop") of thread `t`. Asynchronous: the
   * thread suspends in blocking calls, and a page can compile run-time
   * translations off the main thread.
   */
  async loop(t = this.mainThread) {
    for (;;) {
      const reason = await this.runTop(t.state);
      const r = this.onExit(reason, t.state);
      if (r.translate === undefined) return r;
      await this.translateAt(r.translate);
    }
  }

  /** The run loop of a call from a host function: host functions are synchronous. */
  loopSync() {
    for (;;) {
      const r = this.onExit(this.run(this.state));
      if (r.translate === undefined) return r;
      this.translateAtSync(r.translate);
    }
  }

  /**
   * What an exit of `run` means: the guest call returned, the process
   * exited, or code at `translate` needs translating. Throws ProcessError on
   * an error stop.
   * @returns {{returned: true, rax: bigint} | {exitCode: number} | {threadExit: number} | {translate: bigint}}
   */
  onExit(reason, state = this.state) {
    const m = this.mem;
    const rip = m.u64(state + BigInt(STATE_RIP));
    if (this.trace.exits) this.log(`exit ${EXIT_NAMES[reason] ?? reason} rip=${hex(rip)}`);
    switch (reason) {
      case EXIT_LEFT_CODE:
        if (rip === SENTINEL_RET) return { returned: true, rax: m.u64(state + BigInt(gprOffset(0))) };
        if (executableSection(this.plan, rip)) return { translate: rip };
        throw new ProcessError(`fault: execute at ${hex(rip)}`);
      case EXIT_UNSUPPORTED:
        throw new ProcessError(`unsupported instruction at ${hex(rip)} (bytes ${this.codeBytes(rip)})`);
      case EXIT_HOST: {
        const stop = this.stop;
        this.stop = null;
        if (!stop) throw new ProcessError(`EXIT_HOST at ${hex(rip)} without a recorded stop`);
        if ('exitCode' in stop) return { exitCode: stop.exitCode };
        if ('threadExit' in stop) return { threadExit: stop.threadExit };
        throw new ProcessError(stop.error);
      }
      case EXIT_FAULT: {
        const addr = m.u64(state + BigInt(STATE_FAULT_ADDR));
        const kind = m.u32(state + BigInt(STATE_FAULT_KIND));
        throw new ProcessError(`fault: ${FAULT_NAMES[kind] ?? `kind ${kind}`} at ${hex(addr)} (rip ${hex(rip)})`);
      }
      default:
        throw new ProcessError(`run returned unknown exit reason ${reason} (rip ${hex(rip)})`);
    }
  }

  /**
   * A call from a host function into guest code (runtime.md, "Calls from
   * the host into guest code"): saves the state block and `depth`, builds a
   * Win64 call frame below the guest's RSP (16-byte aligned, shadow space,
   * stack arguments, SENTINEL_RET), runs to the sentinel and restores both.
   * @param {bigint} target
   * @param {bigint[]} args
   * @returns {{rax: bigint} | {exitCode: number} | {error: string}}
   */
  callNested(target, args) {
    if (!this.running) return { error: 'a guest call from the host outside a running guest call' };
    if (this.nesting >= MAX_GUEST_CALL_NESTING) return { error: `guest calls nested deeper than ${MAX_GUEST_CALL_NESTING}` };
    const m = this.mem;
    const saved = m.read(this.state, Number(STATE_SIZE));
    const depth = this.linker.depth.value;
    const stackLimit = this.threads.current?.stackLimit ?? this.map.stackBottom;
    // Synchronous code can't suspend: blocking calls get the plain host function.
    if (this.nesting++ === 0) this.linker.table.set(this.blockingIndex, this.hostFn);
    try {
      const stackArgs = args.slice(4);
      const top = this.reg(RSP) & ~0xfn;
      const rsp = ((top - 0x20n - 8n * BigInt(stackArgs.length)) & ~0xfn) - 8n;
      if (rsp < stackLimit) return { error: `stack overflow in a guest call to ${hex(target)} (rsp ${hex(rsp)})` };
      m.setU64(rsp, SENTINEL_RET);
      stackArgs.forEach((a, i) => m.setU64(rsp + 0x28n + 8n * BigInt(i), a));
      args.slice(0, 4).forEach((a, i) => this.setReg(ARG_REGS[i], a));
      this.setReg(RSP, rsp);
      m.setU64(this.state + BigInt(STATE_RIP), target);
      m.setU64(this.state + BigInt(STATE_RFLAGS), RFLAGS_INIT);
      if (this.trace.api) this.log(`call ${hex(target)}(${args.map(hex).join(', ')})`);
      const r = this.loopSync();
      if ('threadExit' in r) return { error: 'a thread exit inside a call from the host into guest code is not supported' };
      return 'exitCode' in r ? r : { rax: r.rax };
    } catch (e) {
      if (e instanceof ProcessError) return { error: e.message };
      throw e;
    } finally {
      m.write(this.state, saved);
      this.linker.depth.value = depth;
      if (--this.nesting === 0) this.linker.table.set(this.blockingIndex, this.blockingFn);
    }
  }

  /** Up to 15 bytes at `rip`, within its executable section, as hex. */
  codeBytes(rip) {
    const s = executableSection(this.plan, rip);
    if (!s) return 'outside the image';
    const end = s.va + s.mappedSize;
    const n = Number(end - rip < 15n ? end - rip : 15n);
    return Array.from(this.mem.read(rip, n), (b) => b.toString(16).padStart(2, '0')).join(' ');
  }

  /** Run-time translation (runtime.md, "Run-time translation") of the code at `rip`. */
  async translateAt(rip) {
    const t0 = now();
    const wasm = this.translateRequest(rip);
    let linked;
    try {
      linked = await this.linker.link(await WebAssembly.compile(wasm));
    } catch (e) {
      throw new ProcessError(`linking the run-time translation at ${hex(rip)}: ${e.message}`);
    }
    this.translated(rip, wasm, linked, t0);
  }

  /** translateAt for a guest call from a host function: synchronous compile and link. */
  translateAtSync(rip) {
    const t0 = now();
    const wasm = this.translateRequest(rip);
    let linked;
    try {
      linked = this.linker.linkSync(new WebAssembly.Module(wasm));
    } catch (e) {
      throw new ProcessError(`linking the run-time translation at ${hex(rip)}: ${e.message}`);
    }
    this.translated(rip, wasm, linked, t0);
  }

  translateRequest(rip) {
    if (!this.rt) throw new ProcessError(`no translated code at ${hex(rip)} and no run-time translator loaded`);
    const s = executableSection(this.plan, rip);
    try {
      return this.rt.translate({ codeBase: s.va, code: this.mem.read(s.va, Number(s.mappedSize)), entry: rip, known: this.linker.sortedKeys() });
    } catch (e) {
      throw new ProcessError(`run-time translation at ${hex(rip)}: ${e.message}`);
    }
  }

  translated(rip, wasm, linked, t0) {
    this.translations++;
    this.translateMs += now() - t0;
    if (this.trace.translate) {
      this.log(`translate ${hex(rip)}: ${linked.blocks} blocks (${linked.inserted} new), ${wasm.length} wasm bytes, ${(now() - t0).toFixed(1)} ms`);
    }
    if (!this.linker.lookup(rip)) throw new ProcessError(`run-time translation at ${hex(rip)} produced no block for it`);
  }

  /**
   * Runs the program: TLS callbacks (DLL_PROCESS_ATTACH), then the entry
   * point with RCX = PEB, on the main thread. Resolves with the exit code
   * when the main thread returns or any thread exits the process; rejects
   * with a ProcessError on an error stop in any thread.
   */
  start() {
    if (this.running) return Promise.reject(new ProcessError('the process is already running'));
    this.running = true;
    this.outcome = new Promise((resolve, reject) => { this.settle = { resolve, reject }; });
    this.guard(this.runMain());
    return this.outcome;
  }

  async runMain() {
    const { imageBase, entryPoint, tls } = this.plan;
    const t = this.mainThread;
    for (const cb of tls?.callbacks ?? []) {
      const r = await this.callOn(t, cb, [imageBase, DLL_PROCESS_ATTACH, 0n]);
      if (!('returned' in r)) return this.ended(r, t);
    }
    const r = await this.callOn(t, entryPoint, [PEB_ADDR]);
    if (!('returned' in r)) return this.ended(r, t);
    this.finish({ exitCode: Number(BigInt.asUintN(32, r.rax)) });
  }

  /** A guest call that didn't return: the process exits, or (not on the main thread) the thread ends. */
  ended(r, t) {
    if ('exitCode' in r) { this.finish(r); return true; }
    if (t.main) throw new ProcessError('ExitThread or _endthreadex on the main thread is not supported');
    return false;
  }

  /** Ends the process once: {exitCode} or {error}. Other threads never run again. */
  finish(outcome) {
    if (this.finished) return;
    this.finished = true;
    this.threads.finished = true;
    this.running = false;
    if ('exitCode' in outcome) this.settle.resolve(outcome.exitCode);
    else this.settle.reject(outcome.error instanceof ProcessError ? outcome.error : new ProcessError(String(outcome.error?.stack ?? outcome.error)));
  }

  /** Turns a failure of a thread's loop into the process's error stop. */
  guard(promise) {
    promise.catch((e) => this.finish({ error: e }));
  }

  /**
   * A new guest thread (runtime.md, "Guest threads"): state block, TEB,
   * stack and TLS data; it starts when the scheduler picks it, unless
   * `suspended` (CREATE_SUSPENDED).
   * @param {{start: bigint, arg: bigint, stack: bigint, reservation: boolean, suspended: boolean}} opts
   */
  createThread({ start, arg, stack = 0n, reservation = false, suspended = false }) {
    // Windows' rule (RtlCreateUserStack): the reservation, or the image's;
    // at least the commit size and 1 MiB; 64 KiB granularity.
    let reserve = reservation && stack ? stack : this.plan.stackReserve;
    if (!reservation && stack > reserve) reserve = stack;
    if (reserve < MIN_STACK_RESERVE) reserve = MIN_STACK_RESERVE;
    reserve = roundUp(reserve, STACK_ALIGN);
    const slot = this.threadSlot(reserve);
    const t = new Thread({
      id: this.threads.newId(), state: slot.state, teb: slot.teb, stackBase: slot.stackLimit + reserve, stackLimit: slot.stackLimit,
    });
    t.slot = slot;
    const m = this.mem;
    m.fill(slot.teb, Number(TEB_SIZE));
    m.fill(slot.state, Number(STATE_SIZE));
    m.setU64(slot.teb + BigInt(TEB_STACK_BASE), t.stackBase);
    m.setU64(slot.teb + BigInt(TEB_STACK_LIMIT), t.stackLimit);
    m.setU64(slot.teb + BigInt(TEB_SELF), slot.teb);
    m.setU64(slot.teb + BigInt(TEB_CLIENT_ID_PROCESS), PROCESS_ID);
    m.setU64(slot.teb + BigInt(TEB_CLIENT_ID_THREAD), BigInt(t.id));
    m.setU64(slot.teb + BigInt(TEB_PEB), PEB_ADDR);
    if (this.plan.tls) slot.tls = this.setupTls(slot.teb);
    this.initFpState(slot.state);
    t.suspendCount = suspended ? 1 : 0;
    this.threads.add(t);
    const gate = this.threads.suspend(t);
    t.depth = 0;
    this.guard(this.runThread(t, gate, start, arg));
    this.threads.start(t);
    return t;
  }

  /** A state block, TEB and stack for a new thread: those of an ended thread with the same stack size, or new ones. */
  threadSlot(reserve) {
    this.freeSlots ??= [];
    const i = this.freeSlots.findIndex((s) => s.reserve === reserve);
    if (i >= 0) return this.freeSlots.splice(i, 1)[0];
    const alloc = (size, align) => {
      try { return this.guestHeap.alloc(size, align); } catch (e) { throw new HostStop({ error: `a new thread: ${e.message}` }); }
    };
    let state;
    try { state = this.runtimeHeap.alloc(BigInt(STATE_SIZE), 64n); } catch (e) { throw new HostStop({ error: `a new thread: ${e.message}` }); }
    return { reserve, state, teb: alloc(TEB_SIZE, 0x1000n), stackLimit: alloc(reserve, STACK_ALIGN) };
  }

  /**
   * Thread `t` from its first turn to its end: TLS callbacks with
   * DLL_THREAD_ATTACH, the start routine, TLS callbacks with
   * DLL_THREAD_DETACH; then its handle is signaled and the next thread runs.
   */
  async runThread(t, gate, start, arg) {
    await gate;
    const { imageBase, tls } = this.plan;
    const callbacks = tls?.callbacks ?? [];
    let code = null;
    for (const cb of callbacks) {
      const r = await this.callOn(t, cb, [imageBase, DLL_THREAD_ATTACH, 0n]);
      if (this.ended(r, t)) return;
      if ('threadExit' in r) { code = r.threadExit; break; }
    }
    if (code === null) {
      const r = await this.callOn(t, start, [arg]);
      if (this.ended(r, t)) return;
      code = 'threadExit' in r ? r.threadExit : Number(BigInt.asUintN(32, r.rax));
    }
    for (const cb of callbacks) {
      const r = await this.callOn(t, cb, [imageBase, DLL_THREAD_DETACH, 0n]);
      if (this.ended(r, t)) return;
      if ('threadExit' in r) break;
    }
    this.freeSlots.push(t.slot);
    const deadlock = this.threads.exit(t, code);
    if (deadlock) throw new ProcessError(deadlock);
  }
}
