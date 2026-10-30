// Calls from the host into guest code (runtime.md, "Calls from the host into
// guest code") and data imports (runtime.md, "Loading", step 4), through a
// hand-made AOT module whose `run` jumps into a test shim function.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  SENTINEL_RET, STATE_RIP, STATE_EXIT, STATE_SIZE, MAIN_STATE_ADDR, EXIT_LEFT_CODE, THUNK_BASE,
} from '../../runtime/src/abi.mjs';
import { Process, ProcessError } from '../../runtime/src/process.mjs';
import { SHIM } from '../../runtime/src/win32/index.mjs';
import { fakeExe, fakePlan, BASE, TEXT, IDATA, TLS_PLAN } from './pe-fixture.mjs';
import { v3Module } from './v3-mini.mjs';

const RAX = 0, RCX = 1, RSP = 4;
const h = (v) => '0x' + BigInt(v).toString(16);
const exe = fakeExe();
/** Import ids: the five of fakePlan, then TEST.dll's. */
const NEST = 5, COUNTER = 6, BYTES = 7, LATE = 8, MISSING_DATA = 9;
const G = TEXT + 0x10n;

const sleb64 = (v) => {
  const out = [];
  let n = BigInt.asIntN(64, BigInt(v));
  for (;;) {
    const b = Number(n & 0x7fn);
    n >>= 7n;
    if ((n === 0n && !(b & 0x40)) || (n === -1n && (b & 0x40))) { out.push(b); return out; }
    out.push(b | 0x80);
  }
};

/**
 * `run`: at the entry point, RCX = `first`; then, wherever RIP is, it jumps
 * to import `id` (table slot 0 with ret = NO_RET, as `jmp [iat]` would).
 * STATUS_EXIT passes the exit reason up; otherwise (the host function's ret
 * unwound to [RSP]) it returns EXIT_LEFT_CODE with the popped RIP.
 */
const jumpAot = (id, first) => v3Module({
  run: [
    0x20, 0, 0x29, 3, 0x80, 0x01, 0x42, ...sleb64(TEXT), 0x51, // state.rip == TEXT
    0x04, 0x40, 0x20, 0, 0x42, ...sleb64(first), 0x37, 3, 8, 0x0b, // then state.rcx = first
    0x20, 0, 0x41, id, 0x42, 0x7f, 0x41, 0, 0x11, 1, 0, // table[0](state, id, NO_RET)
    0x41, 2, 0x46,
    0x04, 0x7f, 0x20, 0, 0x28, 2, 0x90, 0x01,
    0x05, 0x20, 0, 0x41, 1, 0x36, 2, 0x90, 0x01, 0x41, 1, 0x0b,
  ],
  chunks: [[0x41, 0]],
  blocks: [{ addr: TEXT, chunk: 0, entry: 0 }],
});

const editPlan = (j) => {
  j.imports.push({ dll: 'TEST.dll', functions: [
    { name: 'nest', ordinal: null, iat: h(IDATA + 0x40n) },
    { name: 'counter', ordinal: null, iat: h(IDATA + 0x48n) },
    { name: 'bytes', ordinal: null, iat: h(IDATA + 0x50n) },
    { name: 'late', ordinal: null, iat: h(IDATA + 0x58n) },
    { name: 'missingData', ordinal: null, iat: h(IDATA + 0x60n) },
  ] });
};

const testShim = (nest) => ({
  ...SHIM,
  'test.dll': {
    nest: { args: 1, fn: nest },
    counter: { data: { size: 8, init: 0x1122_3344_5566_7788n } },
    bytes: { data: { size: 3, init: new Uint8Array([1, 2, 3]) } },
    late: { data: { size: 16, init: (ctx) => ctx.argv.length } },
  },
});

const load = async (nest, { first = 2, edit = () => {}, ...opts } = {}) => Process.load({
  exe, plan: await fakePlan(exe, (j) => { editPlan(j); edit(j); }), aot: jumpAot(NEST, first), shim: testShim(nest), ...opts,
});

test('a host function calls guest code, nested twice; state and depth are restored', async () => {
  const seen = [];
  let p;
  const nest = (ctx) => {
    const n = ctx.argI32(0);
    const rsp = ctx.reg(RSP);
    seen.push({ n, rsp, ret: ctx.read64(rsp), depth: p.linker.depth.value, args: [ctx.arg(1), ctx.arg(4), ctx.arg(5)] });
    if (n === 0) return 100n;
    p.linker.depth.value = 7 + n;
    const before = p.mem.read(MAIN_STATE_ADDR, Number(STATE_SIZE));
    const r = ctx.call(G, n - 1, 0xaa, 0, 0, 0x5000 + n, 0x6000 + n);
    assert.deepEqual(p.mem.read(MAIN_STATE_ADDR, Number(STATE_SIZE)), before, 'state block restored');
    assert.equal(p.linker.depth.value, 7 + n, 'depth restored');
    return r + BigInt(n);
  };
  p = await load(nest);
  assert.equal(await p.start(), 103);
  assert.deepEqual(seen.map((s) => s.n), [2, 1, 0]);
  const top = 0x7ff0_0000n - 0x38n;
  assert.equal(seen[0].rsp, top);
  for (let i = 1; i < 3; i++) {
    const { rsp, ret, args } = seen[i];
    assert.equal(rsp % 16n, 8n, 'as after a call');
    assert.equal(ret, SENTINEL_RET);
    assert.ok(rsp + 0x28n + 16n <= (seen[i - 1].rsp & ~0xfn), 'shadow space and stack arguments below the caller');
    assert.deepEqual(args, [0xaan, BigInt(0x5000 + 3 - i), BigInt(0x6000 + 3 - i)]);
  }
  assert.equal(p.reg(RAX), 103n);
});

test('a guest call inside a host call translates at run time synchronously', async () => {
  const target = TEXT + 0x40n;
  const rt = { translate: () => v3Module({ run: [0x41, 1], chunks: [[0x41, 0]], blocks: [{ addr: target, chunk: 0, entry: 0 }] }) };
  let inner;
  const p = await load((ctx) => { inner = ctx.call(target, 5); return 9n; }, { rt });
  const host = p.linker.table.get(0);
  const steps = [
    (s) => { // the entry point jumps to TEST.dll!nest
      p.mem.setU64(p.state + 0x80n, TEXT);
      return host(s, NEST, -1n) === 2 ? 4 : 1;
    },
    () => EXIT_LEFT_CODE, // the nested call reaches untranslated code
    () => { assert.equal(p.reg(RCX), 5n); p.setReg(RAX, 77n); p.mem.setU64(p.state + BigInt(STATE_RIP), SENTINEL_RET); return EXIT_LEFT_CODE; },
  ];
  p.run = (s) => { const r = steps.shift()(s); p.mem.setU32(s + BigInt(STATE_EXIT), r); return r; };
  assert.equal(await p.start(), 9);
  assert.equal(inner, 77n);
  assert.equal(p.translations, 1);
  assert.deepEqual(p.linker.lookup(target), { func: 2, entry: 0 });
});

test('an exit inside a nested guest call stops the whole process', async () => {
  const p = await load((ctx) => {
    const n = ctx.argI32(0);
    if (n === 0) ctx.exit(5);
    ctx.call(G, n - 1);
    throw new Error('not reached');
  });
  assert.equal(await p.start(), 5);
});

test('an error inside a nested guest call stops the whole process', async () => {
  const p = await load((ctx) => {
    const n = ctx.argI32(0);
    if (n === 0) ctx.fail('boom');
    return ctx.call(G, n - 1);
  });
  await assert.rejects(p.start(), new ProcessError('test.dll!nest: test.dll!nest: test.dll!nest: boom'));
});

test('guest calls nest at most 16 deep', async () => {
  let deepest = 0;
  const p = await load((ctx) => { deepest = Math.max(deepest, ctx.argI32(0)); return ctx.call(G, ctx.argI32(0) + 1); }, { first: 0 });
  await assert.rejects(p.start(), /guest calls nested deeper than 16/);
  assert.equal(deepest, 16);
});

test('TLS callbacks are host-initiated guest calls that may call the host back', async () => {
  const seen = [];
  const p = await load((ctx) => { seen.push(ctx.arg(0)); return 0n; }, { edit: (j) => { j.tls = TLS_PLAN; } });
  // The TLS callback (TEXT + 0x80) jumps to nest with RCX = imageBase; then the entry point with RCX = 2.
  assert.equal(await p.start(), 0);
  assert.deepEqual(seen, [BASE, 2n]);
});

test('data imports: storage in the guest heap, the IAT points to it, no thunk', async () => {
  const p = await load(() => 0n);
  const counter = p.mem.u64(IDATA + 0x48n);
  const bytes = p.mem.u64(IDATA + 0x50n);
  const late = p.mem.u64(IDATA + 0x58n);
  for (const a of [counter, bytes, late]) {
    assert.ok(a >= 0x1000_0000n && a < 0x7ff0_0000n, 'in the guest heap');
    assert.equal(a % 16n, 0n);
  }
  assert.equal(p.mem.u64(counter), 0x1122_3344_5566_7788n);
  assert.deepEqual([...p.mem.read(bytes, 3)], [1, 2, 3]);
  assert.equal(p.mem.u64(late), 1n, 'init(ctx): argv = [program name]');
  assert.equal(p.mem.u64(late + 8n), 0n);
  assert.equal(p.linker.lookup(THUNK_BASE + 16n * BigInt(COUNTER)), null, 'no lookup slot');
  assert.equal(p.dataAddress('TEST.DLL', 'counter'), counter);
  assert.equal(p.dataAddress('test.dll', 'nest'), undefined, 'a function import');
  // A data import the shim lacks is an ordinary unimplemented function: thunk and IAT.
  assert.equal(p.mem.u64(IDATA + 0x60n), THUNK_BASE + 16n * BigInt(MISSING_DATA));
  assert.equal(BYTES + 1, LATE);
});

test('a data import whose init does not fit its size is a load error', async () => {
  const shim = testShim(() => 0n);
  shim['test.dll'].bytes = { data: { size: 2, init: new Uint8Array([1, 2, 3]) } };
  await assert.rejects(Process.load({ exe, plan: await fakePlan(exe, editPlan), aot: jumpAot(NEST, 0), shim }),
    /data import test\.dll!bytes: init has 3 bytes, size is 2/);
});
