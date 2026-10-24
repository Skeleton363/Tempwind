// A Phase 1 process (docs/design/runtime.md): guest memory and its address
// map, the mapped image, import thunks and the IAT, PEB/TEB/stack/TLS, the
// linker, the host-call function and the run loop. Standard APIs only, so it
// runs in Node, a page or a Worker; the CLI is run-pe.mjs.

import {
  PAGE_SIZE, STATE_RIP, STATE_RFLAGS, STATE_EXIT, STATE_SIZE, STATE_GS_BASE, STATE_FS_BASE,
  STATE_FAULT_ADDR, STATE_FAULT_KIND, EXIT_LEFT_CODE, EXIT_UNSUPPORTED, EXIT_FAULT, EXIT_HOST,
  FAULT_READ, FAULT_WRITE, FAULT_EXECUTE, SENTINEL_RET, LOOKUP_HEADER_ADDR, MAIN_STATE_ADDR,
  THUNK_BASE, THUNK_SIZE, THUNK_BYTES, HOST_FUNC_INDEX, MIN_LOOKUP_SLOTS, LOOKUP_SLOT_SIZE,
  PEB_ADDR, TEB_ADDR, PROCESS_ID, THREAD_ID, RFLAGS_INIT, GUEST_CALL_FRAME, DLL_PROCESS_ATTACH,
  TEB_STACK_BASE, TEB_STACK_LIMIT, TEB_SELF, TEB_CLIENT_ID_PROCESS, TEB_CLIENT_ID_THREAD,
  TEB_TLS_POINTER, TEB_PEB, PEB_BEING_DEBUGGED, PEB_IMAGE_BASE, PEB_NUMBER_OF_PROCESSORS,
  PEB_OS_MAJOR, PEB_OS_MINOR, PEB_OS_BUILD, PEB_OS_PLATFORM_ID, OS_VERSION, gprOffset,
} from './abi.mjs';
import { createMemory64 } from './memory.mjs';
import { GuestMemory } from './guest-memory.mjs';
import { Linker, moduleBlocks } from './link.mjs';
import { makeHostCall } from './hostcall.mjs';
import { parsePlan, checkPlan, executableSection, hex } from './plan.mjs';
import { findShim, SHIM } from './win32/index.mjs';

const RCX = 1, RDX = 2, RSP = 4, R8 = 8, R9 = 9;
const ARG_REGS = [RCX, RDX, R8, R9];
const EXIT_NAMES = { [EXIT_LEFT_CODE]: 'LEFT_CODE', [EXIT_UNSUPPORTED]: 'UNSUPPORTED', [EXIT_FAULT]: 'FAULT', [EXIT_HOST]: 'HOST' };
const FAULT_NAMES = { [FAULT_READ]: 'read', [FAULT_WRITE]: 'write', [FAULT_EXECUTE]: 'execute' };
const now = () => performance.now();

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
   *   onOutput?: (stream: 'stdout' | 'stderr', bytes: Uint8Array) => void, shim?: object}} opts
   */
  static async load(opts) {
    const p = new Process(opts);
    await p.init(opts);
    return p;
  }

  constructor({ rt = null, trace = {}, log = () => {}, onOutput = () => {}, shim = SHIM }) {
    this.rt = rt;
    this.trace = { api: false, translate: false, exits: false, ...trace };
    this.log = log;
    this.onOutput = onOutput;
    this.shim = shim;
    this.output = { stdout: [], stderr: [] };
    this.stop = null;
    this.running = false;
    this.translations = 0;
  }

  async init({ exe, plan, aot }) {
    // 1. The plan against the file.
    this.plan = parsePlan(plan);
    this.map = await checkPlan(this.plan, exe);
    let aotModule, aotBlocks;
    try {
      aotModule = aot instanceof WebAssembly.Module ? aot : await WebAssembly.compile(aot);
      aotBlocks = moduleBlocks(aotModule).blocks.length;
    } catch (e) {
      throw new ProcessError(`AOT module: ${e.message}`);
    }

    // 2. Memory and the image.
    const { memory } = createMemory64({ initialPages: Number(this.map.memoryBytes / BigInt(PAGE_SIZE)) });
    this.memory = memory;
    this.mem = new GuestMemory(memory);
    this.mapImage(exe);
    this.guestHeap = new Bump('guest heap', this.map.guestHeap.base, this.map.guestHeap.limit);
    this.runtimeHeap = new Bump('runtime heap', this.map.runtimeHeap.base, this.map.runtimeHeap.limit);

    // 3. The linker: header, slots for the expected keys, host-call function at slot 0.
    const imports = this.plan.imports.flatMap((d) => d.functions.map((f) => ({ dll: d.dll, ...f })));
    const slots = Math.max(MIN_LOOKUP_SLOTS, nextPow2(2 * (aotBlocks + imports.length)));
    this.linker = new Linker({
      memory, header: LOOKUP_HEADER_ADDR, slots,
      slotsAddr: this.runtimeHeap.alloc(BigInt(slots * LOOKUP_SLOT_SIZE), 16n),
      alloc: (size, align) => this.runtimeHeap.alloc(size, align),
    });
    this.imports = imports.map((f, id) => ({ ...f, thunk: THUNK_BASE + THUNK_SIZE * BigInt(id), impl: findShim(f.dll, f.name, this.shim) }));
    this.linker.setHostFunction(makeHostCall({
      mem: this.mem,
      imports: this.imports,
      output: (stream, bytes) => this.write(stream, bytes),
      onStop: (stop) => { this.stop = stop; },
      trace: this.trace.api ? (line) => this.log(line) : undefined,
    }));

    // 4. Imports: thunks, lookup slots, IAT.
    this.writeImports();

    // 5. The AOT module.
    try {
      this.aot = await this.linker.link(aotModule);
    } catch (e) {
      throw new ProcessError(`linking the AOT module: ${e.message}`);
    }
    this.run = this.aot.run;

    // 6. PEB, TEB, stack (the state block is fresh, zeroed memory).
    this.state = MAIN_STATE_ADDR;
    this.writePebTeb();

    // 7. TLS data (callbacks run in start()).
    if (this.plan.tls) this.setupTls();
  }

  mapImage(exe) {
    const { imageBase, sizeOfHeaders, sections } = this.plan;
    this.mem.write(imageBase, exe.subarray(0, Number(sizeOfHeaders)));
    for (const s of sections) {
      const off = Number(s.fileOffset);
      this.mem.write(s.va, exe.subarray(off, off + Number(s.copySize)));
    }
  }

  writeImports() {
    for (const [id, f] of this.imports.entries()) {
      this.mem.write(f.thunk, THUNK_BYTES);
      this.linker.insert(f.thunk, HOST_FUNC_INDEX, id);
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

  setupTls() {
    const t = this.plan.tls;
    const dataSize = t.endOfRawData - t.startOfRawData;
    const block = this.guestHeap.alloc(dataSize + t.sizeOfZeroFill || 1n, 16n);
    if (dataSize) this.mem.write(block, this.mem.read(t.startOfRawData, Number(dataSize)));
    const array = this.guestHeap.alloc(8n, 16n);
    this.mem.setU64(array, block);
    this.mem.setU64(TEB_ADDR + BigInt(TEB_TLS_POINTER), array);
    this.mem.setU32(t.addressOfIndex, 0);
    this.tlsBlock = block;
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

  /**
   * Sets up a host-initiated guest call (runtime.md, "Start of the main
   * thread"): fresh GPRs, Win64 register arguments, RSP = top - 0x38 with
   * [RSP] = SENTINEL_RET, RFLAGS = 0x202, GS base = TEB, FS base = 0.
   */
  prepareCall(target, args = []) {
    if (args.length > 4) throw new ProcessError('host-initiated guest calls take at most 4 arguments');
    const m = this.mem;
    m.fill(this.state, Number(STATE_SIZE));
    const top = this.map.stackTop & ~0xfn;
    const rsp = top - GUEST_CALL_FRAME;
    args.forEach((a, i) => this.setReg(ARG_REGS[i], BigInt.asUintN(64, BigInt(a))));
    this.setReg(RSP, rsp);
    m.setU64(rsp, SENTINEL_RET);
    m.setU64(this.state + BigInt(STATE_RIP), target);
    m.setU64(this.state + BigInt(STATE_RFLAGS), RFLAGS_INIT);
    m.setU64(this.state + BigInt(STATE_GS_BASE), TEB_ADDR);
    m.setU64(this.state + BigInt(STATE_FS_BASE), 0n);
  }

  /**
   * Runs a host-initiated guest call to its end.
   * @returns {Promise<{returned: true, rax: bigint} | {exitCode: number}>}
   * Throws ProcessError on an error stop.
   */
  async callGuest(target, args = []) {
    if (this.running) throw new ProcessError('nested host-initiated guest calls are not supported');
    this.running = true;
    try {
      this.prepareCall(target, args);
      return await this.loop();
    } finally {
      this.running = false;
    }
  }

  async loop() {
    const m = this.mem;
    for (;;) {
      const reason = this.run(this.state);
      const rip = m.u64(this.state + BigInt(STATE_RIP));
      if (this.trace.exits) this.log(`exit ${EXIT_NAMES[reason] ?? reason} rip=${hex(rip)}`);
      switch (reason) {
        case EXIT_LEFT_CODE:
          if (rip === SENTINEL_RET) return { returned: true, rax: this.reg(0) };
          if (executableSection(this.plan, rip)) { await this.translateAt(rip); continue; }
          throw new ProcessError(`fault: execute at ${hex(rip)}`);
        case EXIT_UNSUPPORTED:
          throw new ProcessError(`unsupported instruction at ${hex(rip)} (bytes ${this.codeBytes(rip)})`);
        case EXIT_HOST: {
          const stop = this.stop;
          this.stop = null;
          if (!stop) throw new ProcessError(`EXIT_HOST at ${hex(rip)} without a recorded stop`);
          if ('exitCode' in stop) return { exitCode: stop.exitCode };
          throw new ProcessError(stop.error);
        }
        case EXIT_FAULT: {
          const addr = m.u64(this.state + BigInt(STATE_FAULT_ADDR));
          const kind = m.u32(this.state + BigInt(STATE_FAULT_KIND));
          throw new ProcessError(`fault: ${FAULT_NAMES[kind] ?? `kind ${kind}`} at ${hex(addr)} (rip ${hex(rip)})`);
        }
        default:
          throw new ProcessError(`run returned unknown exit reason ${reason} (rip ${hex(rip)})`);
      }
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
    if (!this.rt) throw new ProcessError(`no translated code at ${hex(rip)} and no run-time translator loaded`);
    const s = executableSection(this.plan, rip);
    const t0 = now();
    let wasm;
    try {
      wasm = this.rt.translate({ codeBase: s.va, code: this.mem.read(s.va, Number(s.mappedSize)), entry: rip, known: this.linker.sortedKeys() });
    } catch (e) {
      throw new ProcessError(`run-time translation at ${hex(rip)}: ${e.message}`);
    }
    let linked;
    try {
      linked = await this.linker.link(await WebAssembly.compile(wasm));
    } catch (e) {
      throw new ProcessError(`linking the run-time translation at ${hex(rip)}: ${e.message}`);
    }
    this.translations++;
    if (this.trace.translate) {
      this.log(`translate ${hex(rip)}: ${linked.blocks} blocks (${linked.inserted} new), ${wasm.length} wasm bytes, ${(now() - t0).toFixed(1)} ms`);
    }
    if (!this.linker.lookup(rip)) throw new ProcessError(`run-time translation at ${hex(rip)} produced no block for it`);
  }

  /**
   * Runs the program: TLS callbacks (DLL_PROCESS_ATTACH), then the entry
   * point with RCX = PEB. Resolves with the exit code; rejects with a
   * ProcessError on an error stop.
   */
  async start() {
    const { imageBase, entryPoint, tls } = this.plan;
    for (const cb of tls?.callbacks ?? []) {
      const r = await this.callGuest(cb, [imageBase, DLL_PROCESS_ATTACH, 0n]);
      if ('exitCode' in r) return r.exitCode;
    }
    const r = await this.callGuest(entryPoint, [PEB_ADDR]);
    return 'exitCode' in r ? r.exitCode : Number(BigInt.asUintN(32, r.rax));
  }
}
