// The JS side of the M1 boundary spike (docs/design/abi.md, "Wine boundary").
// Host-neutral: tests/node/wine-boundary.test.mjs runs it in Node,
// tests/browser/wine-boundary-worker.mjs in a Chromium Worker. Both pass the
// outputs of native/spike/boundary/build.sh (target/wine-spike/).
//
// What it does, in the order of the gate items of the brief:
// 1. one shared memory64, one env.table, the lookup table; guest_a.wasm linked;
//    the Emscripten instance instantiated through the instantiateWasm hook with
//    the raw `run` of guest_a, a plain `bnd_translate` that links guest_b.wasm
//    and a Suspending `green_block`; thread_main entered through promising;
// 2. the raw export host_dispatch placed in env.table behind lookup slots
//    (C finds them again with abi.h's lookup layout and hash);
// 3. two green threads that both block in C, each on its own state block and
//    shadow stack (set through the raw stack exports);
// 4. a nested guest call (in guest_b) through C's loop, env.depth unchanged
//    across it (M2: `run` keeps the caller's depth);
// 5. memory.grow in C past 4 GiB, then memory_grown; then the heap's cap at
//    4 GiB (M2: malloc returns NULL, the memory's size is unchanged);
// 6. a JS function between promising and Suspending (expected to fail).

import { Linker, hostFunction } from '../../../runtime/src/link.mjs';
import { createMemory64 } from '../../../runtime/src/memory.mjs';
import {
  PAGE_SIZE, STATE_SIZE, STATE_GPR, STATE_RIP, STATE_RFLAGS, STATE_MXCSR, STATE_FCW, STATE_FTW,
} from '../../../runtime/src/abi.mjs';

// The layout (guest_a.S, guest_b.S and boundary.c use the same values).
export const LAYOUT = {
  lookupHeader: 0x10000n,
  lookupSlots: 0x10100n,
  slotCount: 1024,
  iat: 0x18000,
  results: 0x19000,
  states: 0x20000,
  codeA: 0x10000000n,
  codeB: 0x10100000n,
  thunkBase: 0x7ff00000n,
  negThunk: 0x7ff00100n,
  /** The Emscripten module's INITIAL_MEMORY (build.sh): 4 GiB, the host region's end. */
  initialPages: 65536,
  maximumPages: 262144,
  shadowStackSize: 64 * 1024,
};
const ENTRY = { thread: 0x10000000n, callback: 0x10000100n, grow: 0x10000200n, neg: 0x10000300n };
export const HOST = { add: 1, block: 2, callback: 3, cbReturn: 4, exit: 5, grow: 6, negBlock: 7 };
const RCX = 1, RSP = 4;

const now = () => performance.now();

/**
 * Runs the spike.
 * @param {{factory: Function, emWasm: Uint8Array, guestA: Uint8Array, guestB: Uint8Array,
 *   rounds?: number, log?: (s: string) => void}} opts
 *   `factory`: the default export of target/wine-spike/boundary.mjs (Emscripten's glue).
 * @returns {Promise<object>} what verify() checks, plus timings
 */
export async function runBoundary({ factory, emWasm, guestA, guestB, rounds = 10000, log = () => {} }) {
  const out = { timings: {}, events: [], output: [], depths: [] };
  const L = LAYOUT;

  // One shared memory64 for everything (abi.md, "Instances").
  let t = now();
  const { memory, form } = createMemory64({ initialPages: L.initialPages, maximumPages: L.maximumPages, shared: true });
  out.timings.memoryMs = now() - t;
  out.memoryForm = form;
  out.initialBytes = memory.buffer.byteLength;

  const linker = new Linker({ memory, header: L.lookupHeader, slotsAddr: L.lookupSlots, slots: L.slotCount });
  // The table slot of the host functions, filled once the Unix side exists.
  const hostIndex = linker.table.length;
  linker.table.grow(1);
  const modA = new WebAssembly.Module(guestA);
  const modB = new WebAssembly.Module(guestB);
  const linkedA = linker.linkSync(modA);

  // EXIT_LEFT_CODE: the plain import links prebuilt code (a stand-in for the translator).
  const prebuilt = new Map([[L.codeB, modB]]);
  out.translated = [];
  const translate = (rip) => {
    const key = BigInt.asUintN(64, rip);
    const m = prebuilt.get(key);
    if (!m) return 1;
    prebuilt.delete(key);
    linker.linkSync(m);
    out.translated.push('0x' + rip.toString(16));
    return 0;
  };

  // green_block, the only switch point: a Promise that the scheduler below resolves.
  const waiting = new Map();
  const greenBlock = (tid) => {
    if (tid < 0) return Promise.resolve(0n); // bench_main: resume at once
    out.events.push(`block ${tid}`);
    return new Promise((resolve) => waiting.set(tid, resolve));
  };
  const wake = (tid, value) => {
    const resolve = waiting.get(tid);
    if (!resolve) throw new Error(`thread ${tid} is not blocked`);
    waiting.delete(tid);
    out.events.push(`wake ${tid}`);
    resolve(value);
  };

  // The Unix side, past Emscripten's glue: instantiateWasm gets the import
  // object; we replace three imports and keep the raw exports.
  t = now();
  const emModule = new WebAssembly.Module(emWasm);
  out.timings.emCompileMs = now() - t;
  let raw = null;
  const views = { grown: [], glue: [] };
  t = now();
  await factory({
    wasmMemory: memory,
    print: (s) => { out.output.push(s); log(s); },
    printErr: (s) => { out.output.push(s); log(s); },
    instantiateWasm(imports, receive) {
      Object.assign(imports.env, {
        bnd_run: linkedA.run, // raw wasm export: a wasm-to-wasm call
        bnd_translate: translate,
        green_block: new WebAssembly.Suspending(greenBlock),
        // Plain: the value of env.depth, the global every translated module imports.
        bnd_depth: () => {
          const d = linker.depth.value;
          out.depths.push(d);
          return d;
        },
      });
      const instance = new WebAssembly.Instance(emModule, imports);
      raw = instance.exports;
      receive(instance, emModule);
      return {};
    },
    // After C's memory.grow: the test's own views (the runtime's, in M3).
    onMemoryGrown() {
      const dv = linker.view(); // GuestMemory.refresh()
      views.grown.push({ bytes: memory.buffer.byteLength, dv });
    },
    onGlueString(s) { views.glue.push(s); },
  });
  out.timings.emInstantiateMs = now() - t;

  // Host functions: the raw export in env.table, lookup slots for their guest addresses.
  linker.table.set(hostIndex, raw.host_dispatch);
  const dv = () => linker.view();
  for (const id of [HOST.add, HOST.block, HOST.callback, HOST.cbReturn, HOST.exit, HOST.grow]) {
    const addr = L.thunkBase + 16n * BigInt(id);
    linker.insert(addr, hostIndex, id);
    dv().setBigUint64(L.iat + 8 * id, addr, true);
  }
  // The negative test's slot: a JS function in env.table that calls the raw export.
  const jsIndex = linker.addHostFunction((s, e, r) => raw.host_dispatch(s, HOST.block, r));
  linker.insert(L.negThunk, jsIndex, HOST.block);
  dv().setBigUint64(L.iat + 8 * HOST.negBlock, L.negThunk, true);
  // C probes the same slots with abi.h's layout and hash; code B is not linked yet.
  out.lookup = [HOST.add, HOST.block, HOST.callback, HOST.cbReturn, HOST.exit, HOST.grow, HOST.negBlock]
    .map((id) => raw.lookup_entry(L.lookupHeader, id === HOST.negBlock ? L.negThunk : L.thunkBase + 16n * BigInt(id)));
  out.lookup.push(raw.lookup_entry(L.lookupHeader, L.codeB));

  out.longjmp = raw.longjmp_check();

  // A green thread: a state block, a guest stack, and a shadow stack whose
  // pointer and limits JS sets right before the promising entry.
  const threadMain = WebAssembly.promising(raw.thread_main);
  out.stacks = {};
  const setShadowStack = (tid) => {
    const low = raw.malloc(BigInt(L.shadowStackSize));
    if (low === 0n) throw new Error('malloc failed');
    const high = low + BigInt(L.shadowStackSize);
    raw.emscripten_stack_set_limits(high, low); // (base = top, end = bottom)
    raw._emscripten_stack_restore(high);
    out.stacks[tid] = { low: '0x' + low.toString(16), high: '0x' + high.toString(16) };
  };
  const startThread = (tid, entry) => {
    const st = L.states + STATE_SIZE * tid;
    const d = dv();
    for (let o = 0; o < STATE_SIZE; o += 8) d.setBigUint64(st + o, 0n, true);
    const guestStack = BigInt(0x100000 * (tid + 1)) - 8n; // [rsp] = 0: the thread never returns
    d.setBigUint64(Number(guestStack), 0n, true);
    d.setBigUint64(st + STATE_GPR + 8 * RCX, BigInt(tid), true);
    d.setBigUint64(st + STATE_GPR + 8 * RSP, guestStack, true);
    d.setBigUint64(st + STATE_RIP, entry, true);
    d.setBigUint64(st + STATE_RFLAGS, 0x202n, true);
    d.setUint32(st + STATE_MXCSR, 0x1f80, true);
    d.setUint16(st + STATE_FCW, 0x037f, true);
    d.setUint16(st + STATE_FTW, 0xffff, true);
    setShadowStack(tid);
    out.events.push(`start ${tid}`);
    return threadMain(BigInt(st), tid);
  };
  const record = (tid) => {
    const d = dv();
    const r = L.results + 0x40 * tid;
    const at = (o) => d.getBigUint64(r + o, true);
    return { add: at(0), block: at(8), rbx: at(0x10), callback: at(0x18), grow: at(0x20) };
  };

  // Gate 3: thread 1 blocks, thread 2 starts and blocks, thread 1 resumes and
  // ends, then thread 2 resumes and ends.
  const p1 = startThread(1, ENTRY.thread);
  const p2 = startThread(2, ENTRY.thread);
  wake(1, 107n);
  const exit1 = await p1;
  out.events.push(`end 1: ${exit1}`);
  wake(2, 114n);
  const exit2 = await p2;
  out.events.push(`end 2: ${exit2}`);
  out.threads = { 1: { exit: exit1, ...record(1) }, 2: { exit: exit2, ...record(2) } };

  // Gate 5: memory growth from C.
  const oldBytes = memory.buffer.byteLength;
  const exit3 = await startThread(3, ENTRY.grow);
  const grownAt = oldBytes + 0x100;
  out.grow = {
    exit: exit3,
    oldBytes,
    newBytes: memory.buffer.byteLength,
    notified: views.grown.length,
    viewBytes: views.grown[0]?.bytes,
    jsValue: views.grown[0] && grownAt + 8 <= views.grown[0].bytes ? views.grown[0].dv.getBigUint64(grownAt, true) : null,
    glueString: views.glue[0] ?? null,
    guestValue: record(3).grow,
    capCheck: raw.cap_check(),
  };

  // The suspend/resume round trip.
  setShadowStack(0);
  t = now();
  const done = await WebAssembly.promising(raw.bench_main)(rounds);
  const ms = now() - t;
  out.bench = { rounds, done, usPerRoundTrip: (ms * 1000) / rounds };

  // Gate 6: a JS frame between promising and Suspending.
  try {
    const r = await startThread(4, ENTRY.neg);
    out.negative = { threw: false, result: r };
  } catch (e) {
    out.negative = { threw: true, name: e?.constructor?.name ?? typeof e, message: String(e?.message ?? e) };
  }
  out.negative.importRan = out.events.at(-1) === 'block 4';
  out.waitingAfter = [...waiting.keys()];
  return out;
}

/** The checks both tests make. Returns a list of problems (empty = pass). */
export function verify(r) {
  const problems = [];
  const want = (name, got, exp) => {
    if (got !== exp) problems.push(`${name}: got ${typeof got === 'bigint' ? '0x' + got.toString(16) : got}, want ${typeof exp === 'bigint' ? '0x' + exp.toString(16) : exp}`);
  };
  want('memory initial bytes', r.initialBytes, LAYOUT.initialPages * PAGE_SIZE);
  want('longjmp_check', r.longjmp, 1);
  // Node 24's V8 (13.6) runs the Suspending's JS function (block 4) before it
  // refuses to suspend the JS frame, leaving thread 4's Promise unresolved;
  // Chromium's refuses before calling it. Both throw the same SuspendError.
  const ran = r.events.at(-1) === 'block 4';
  want('events', r.events.slice(0, ran ? -1 : undefined).join(', '),
    'start 1, block 1, start 2, block 2, wake 1, end 1: 81, wake 2, end 2: 82, start 3, start 4');
  want('translated', r.translated.join(','), '0x' + LAYOUT.codeB.toString(16));
  for (const [tid, wake] of [[1, 107n], [2, 114n]]) {
    const th = r.threads[tid];
    const b = BigInt(tid);
    want(`thread ${tid} exit`, th.exit, 0x50 + tid);
    want(`thread ${tid} add`, th.add, b + 40n);
    want(`thread ${tid} block (0xBAD... = a save-set check failed)`, th.block, wake);
    want(`thread ${tid} rbx`, th.rbx, b * 1000n);
    want(`thread ${tid} callback (0xBADD0000 = depth changed)`, th.callback, 3n * (b + 5n) + 1n);
  }
  // env.depth before and after each nested run (one per thread's callback):
  // `run` keeps the caller's depth (abi.md v5), so equal and nonzero.
  want('depth reads around nested runs', r.depths.length, 4);
  for (let i = 0; i + 1 < r.depths.length; i += 2) {
    const [before, after] = r.depths.slice(i, i + 2);
    if (before === 0 || after !== before) {
      problems.push(`nested run ${i / 2}: depth ${before} before, ${after} after; want equal and nonzero`);
    }
  }
  const g = r.grow;
  const value = 0x600DF00D00000003n;
  want('grow exit', g.exit, 0x60);
  want('grow new size', g.newBytes, g.oldBytes + PAGE_SIZE);
  want('memory_grown calls', g.notified, 1);
  want('view size after memory_grown', g.viewBytes, g.newBytes);
  want('value read by JS', g.jsValue, value);
  want('value read by the glue', g.glueString, 'past the old end');
  want('value read by translated code', g.guestValue, value);
  // boundary.c's cap_check bits: 1 not past 4 GiB, 2 malloc returned memory,
  // 4 errno not ENOMEM, 8 the memory's size changed, 16 the break moved.
  want('heap cap at 4 GiB after growth (cap_check)', g.capCheck, 0);
  want('lookup entries found by C through abi.h', r.lookup.join(','),
    [HOST.add, HOST.block, HOST.callback, HOST.cbReturn, HOST.exit, HOST.grow, HOST.block, -1].join(','));
  want('bench round trips', r.bench.done, r.bench.rounds);
  want('negative test threw', r.negative.threw, true);
  want('negative test error', `${r.negative.name}: ${r.negative.message}`, 'SuspendError: trying to suspend JS frames');
  want('blocked threads left', r.waitingAfter.join(','), ran ? '4' : '');
  return problems;
}
