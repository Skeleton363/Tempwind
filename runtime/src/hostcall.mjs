// The host-call function (docs/design/abi.md, "Host functions";
// runtime.md, "Host calls"): the JS function behind table slot 0. Translated
// code calls it like a guest function with (state, id, ret); it runs the shim
// function of import `id` and then performs the guest's `ret`.

import {
  STATE_RIP, STATE_EXIT, STATE_GS_BASE, EXIT_HOST, STATUS_RETURNED, STATUS_UNWIND, STATUS_EXIT,
  TEB_LAST_ERROR, NULL_REGION_SIZE, gprOffset,
} from './abi.mjs';

const RAX = 0, RCX = 1, RDX = 2, RSP = 4, R8 = 8, R9 = 9;
const ARG_REGS = [RCX, RDX, R8, R9];
/** Argument i >= 4 is at [RSP + 0x28 + 8 * (i - 4)] at the host function's entry. */
export const STACK_ARGS_OFFSET = 0x28n;
const hex = (v) => '0x' + BigInt.asUintN(64, BigInt(v)).toString(16);

/** Thrown by a shim function through ctx.exit / ctx.fail. */
export class HostStop {
  /** @param {{exitCode: number} | {error: string}} stop */
  constructor(stop) { this.stop = stop; }
}

/**
 * @typedef {{call: (target: bigint, args: bigint[]) => ({rax: bigint} | {exitCode: number} | {error: string}),
 *   alloc: (size: bigint, align: bigint) => bigint, dllState: (dll: string, init: () => object) => object,
 *   dataAddress: (dll: string, name: string) => bigint | undefined, argv: string[],
 *   image: {base: bigint, size: bigint, sizeOfHeaders: bigint, sections: object[]} | null,
 *   vfs?: import('./win32/vfs.mjs').Vfs, procAddress?: (dll: string, name: string) => bigint | undefined,
 *   startTime?: bigint, programPath?: string}} HostServices
 */
const unavailable = (what) => () => { throw new HostStop({ error: `${what} is not available in this context` }); };
const NO_HOST = {
  call: unavailable('calling guest code'),
  alloc: unavailable('the guest heap'),
  dllState: unavailable('shim DLL state'),
  dataAddress: unavailable('data imports'),
  argv: [],
  image: null,
};

/** What a shim function sees: Win64 arguments, guest memory, last error, output, stops. */
export class HostContext {
  /**
   * @param {import('./guest-memory.mjs').GuestMemory} mem
   * @param {bigint} state
   * @param {(stream: string, bytes: Uint8Array) => void} output
   * @param {HostServices} [host] process services (guest calls, guest heap, per-DLL state, data imports)
   */
  constructor(mem, state, output, host = NO_HOST) {
    this.mem = mem;
    this.state = state;
    this.outputSink = output;
    this.host = host;
  }

  reg(i) { return this.mem.u64(this.state + BigInt(gprOffset(i))); }

  /** Argument i (0-based) as a u64 BigInt, by the Win64 convention. */
  arg(i) {
    if (i < 4) return this.reg(ARG_REGS[i]);
    return this.guard(() => this.mem.u64(this.reg(RSP) + STACK_ARGS_OFFSET + 8n * BigInt(i - 4)), `stack argument ${i}`);
  }

  /** Argument i as a 32-bit unsigned Number (DWORD, UINT): the upper half of the register is ignored. */
  arg32(i) { return Number(BigInt.asUintN(32, this.arg(i))); }

  /** Argument i as a 32-bit signed Number (INT, or a DWORD compared with negative constants). */
  argI32(i) { return Number(BigInt.asIntN(32, this.arg(i))); }

  /** Checks a guest pointer range: not in the NULL region, inside memory. */
  check(addr, len, what = 'pointer') {
    addr = BigInt.asUintN(64, BigInt(addr));
    if (addr < NULL_REGION_SIZE || addr + BigInt(len) > BigInt(this.mem.size)) {
      this.fail(`bad ${what} ${hex(addr)} (${len} bytes)`);
    }
    return addr;
  }

  read(addr, len) { return this.mem.read(this.check(addr, len), len); }
  write(addr, bytes) { this.mem.write(this.check(addr, bytes.length), bytes); }
  read32(addr) { return this.mem.u32(this.check(addr, 4)); }
  write32(addr, v) { this.mem.setU32(this.check(addr, 4), v); }
  read64(addr) { return this.mem.u64(this.check(addr, 8)); }
  write64(addr, v) { this.mem.setU64(this.check(addr, 8), v); }

  /** Sets the calling thread's last error (TEB.LastErrorValue; TEB = GS base). */
  setLastError(code) {
    const teb = this.mem.u64(this.state + BigInt(STATE_GS_BASE));
    if (teb === 0n) this.fail('SetLastError without a TEB (GS base is 0)');
    this.mem.setU32(teb + BigInt(TEB_LAST_ERROR), code);
  }

  /** The calling thread's last error (TEB.LastErrorValue). */
  lastError() {
    const teb = this.mem.u64(this.state + BigInt(STATE_GS_BASE));
    if (teb === 0n) this.fail('GetLastError without a TEB (GS base is 0)');
    return this.mem.u32(teb + BigInt(TEB_LAST_ERROR));
  }

  /** The TEB of the calling thread (GS base). */
  teb() {
    const teb = this.mem.u64(this.state + BigInt(STATE_GS_BASE));
    if (teb === 0n) this.fail('no TEB (GS base is 0)');
    return teb;
  }

  /** The loaded image: {base, size, sizeOfHeaders, sections: [{name, va, mappedSize, characteristics}]}. */
  get image() { return this.host.image; }

  /** Appends to the process's captured stdout or stderr. */
  output(stream, bytes) { this.outputSink(stream, bytes); }

  /**
   * Calls guest code at `target` with Win64 arguments (runtime.md, "Calls
   * from the host into guest code") and returns its RAX as a u64 BigInt. A
   * stop inside the call (exit or error) stops the whole process.
   */
  call(target, ...args) {
    const r = this.host.call(BigInt.asUintN(64, BigInt(target)), args.map((a) => BigInt.asUintN(64, BigInt(a))));
    if ('exitCode' in r) throw new HostStop({ exitCode: r.exitCode });
    if ('error' in r) this.fail(r.error);
    return r.rax;
  }

  /** Allocates `size` bytes of the guest heap (process lifetime; fresh memory is zero). */
  alloc(size, align = 16n) { return this.host.alloc(BigInt(size), BigInt(align)); }

  /** Per-process state of a shim DLL: `init()` creates it on first use. */
  dllState(dll, init) { return this.host.dllState(dll.toLowerCase(), init); }

  /** The storage address of data import `dll!name`, or undefined if the program doesn't import it. */
  dataAddress(dll, name) { return this.host.dataAddress(dll.toLowerCase(), name); }

  /** The program's command line as argv strings (argv[0] = program name). */
  get argv() { return this.host.argv; }

  /** The process's file system (win32/vfs.mjs). */
  get vfs() {
    if (!this.host.vfs) this.fail('the file system is not available in this context');
    return this.host.vfs;
  }

  /**
   * A callable address for shim function `dll!name` (GetProcAddress), or
   * undefined if the shim lacks it.
   */
  procAddress(dll, name) {
    if (!this.host.procAddress) this.fail('GetProcAddress thunks are not available in this context');
    return this.host.procAddress(dll.toLowerCase(), name);
  }

  /** The FILETIME the process started at. */
  get startTime() { return this.host.startTime ?? 0n; }

  /** The program's path in the file system (GetModuleFileNameW(NULL)). */
  get programPath() { return this.host.programPath ?? 'C:\\tempwind\\program.exe'; }

  /** Reads a NUL-terminated byte string (at most `max` bytes before the NUL). */
  readCString(addr, max = 1 << 20) {
    const start = this.mem.at(this.check(addr, 1, 'string'), 1);
    const u8 = this.mem.u8;
    const nul = u8.indexOf(0, start);
    if (nul < 0) this.fail(`string at ${hex(addr)} runs past the end of memory`);
    if (nul - start > max) this.fail(`string at ${hex(addr)} longer than ${max} bytes`);
    return u8.slice(start, nul);
  }

  /** Reads a NUL-terminated UTF-16 string as code units (at most `max` units before the NUL). */
  readWString(addr, max = 1 << 20) {
    const out = [];
    for (let a = this.check(addr, 2, 'string'); ; a += 2n) {
      const u = this.mem.u16(this.check(a, 2, 'string'));
      if (u === 0) return out;
      if (out.length >= max) this.fail(`string at ${hex(addr)} longer than ${max} units`);
      out.push(u);
    }
  }

  exit(code) { throw new HostStop({ exitCode: code >>> 0 }); }
  fail(message) { throw new HostStop({ error: message }); }

  guard(f, what) {
    try { return f(); } catch (e) {
      if (e instanceof RangeError) this.fail(`${what}: ${e.message}`);
      throw e;
    }
  }
}

/**
 * Builds the host-call function.
 * @param {{mem: import('./guest-memory.mjs').GuestMemory,
 *   imports: {dll: string, name: string, thunk: bigint, impl?: {args: number, fn: Function}}[],
 *   output: (stream: string, bytes: Uint8Array) => void,
 *   onStop: (stop: {exitCode: number} | {error: string}) => void,
 *   trace?: (line: string) => void, host?: HostServices}} opts
 * @returns {(state: bigint, id: number, ret: bigint) => number}
 */
export function makeHostCall({ mem, imports, output, onStop, trace, host }) {
  return (stateArg, id, retArg) => {
    // i64 parameters reach JS as signed BigInts.
    const state = BigInt.asUintN(64, stateArg);
    const ret = BigInt.asUintN(64, retArg);
    const imp = imports[id];
    const label = imp ? `${imp.dll.toLowerCase()}!${imp.name}` : `import #${id}`;
    const ctx = new HostContext(mem, state, output, host);
    const stopWith = (stop, args) => {
      if (trace) trace(`${label}(${args}) -> ${'exitCode' in stop ? `exit ${stop.exitCode}` : `error: ${stop.error}`}`);
      onStop(stop);
      mem.setU64(state + BigInt(STATE_RIP), imp ? imp.thunk : 0n);
      mem.setU32(state + BigInt(STATE_EXIT), EXIT_HOST);
      return STATUS_EXIT;
    };
    if (!imp) return stopWith({ error: `host call with unknown import id ${id}` }, '?');
    if (!imp.impl || !imp.impl.fn) {
      // The trace names the caller: the return address at [RSP].
      let caller = '?';
      try { caller = `return address ${hex(mem.u64(mem.u64(state + BigInt(gprOffset(RSP)))))}`; } catch { /* unreadable stack */ }
      return stopWith({ error: `unimplemented API ${label}` }, caller);
    }
    let args = '';
    try {
      if (trace) args = Array.from({ length: imp.impl.args }, (_, i) => hex(ctx.arg(i))).join(', ');
      const value = imp.impl.fn(ctx);
      if (value !== undefined) mem.setU64(state + BigInt(gprOffset(RAX)), BigInt.asUintN(64, BigInt(value)));
      if (trace) trace(`${label}(${args})${value === undefined ? '' : ` = ${hex(value)}`}`);
      // The guest's ret: t = [RSP], RSP += 8.
      const rsp = mem.u64(state + BigInt(gprOffset(RSP)));
      const t = ctx.guard(() => mem.u64(rsp), 'return address');
      mem.setU64(state + BigInt(gprOffset(RSP)), rsp + 8n);
      if (t === ret) return STATUS_RETURNED;
      mem.setU64(state + BigInt(STATE_RIP), t);
      return STATUS_UNWIND;
    } catch (e) {
      if (e instanceof HostStop) {
        return stopWith('error' in e.stop ? { error: `${label}: ${e.stop.error}` } : e.stop, args);
      }
      return stopWith({ error: `${label}: internal error: ${e?.stack ?? e}` }, args);
    }
  };
}
