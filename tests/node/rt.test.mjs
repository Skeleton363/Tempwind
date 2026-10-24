// The request/result layout of runtime/src/rt.mjs, against stand-in exports
// over a real WebAssembly.Memory (the real x64wasm_rt.wasm is exercised by
// tests/node/pe.test.mjs).
import test from 'node:test';
import assert from 'node:assert/strict';
import { RuntimeTranslator } from '../../runtime/src/rt.mjs';

function standIn(respond) {
  const memory = new WebAssembly.Memory({ initial: 1 });
  let next = 1024;
  const allocs = [];
  const frees = [];
  const seen = {};
  const x = {
    memory,
    rt_alloc(size) {
      if (next + size > memory.buffer.byteLength) memory.grow(1); // views must be retaken
      const p = next;
      next = (next + size + 15) & ~15;
      allocs.push([p, size]);
      return p;
    },
    rt_free(ptr, size) { frees.push([ptr, size]); },
    rt_translate(req) {
      const dv = new DataView(memory.buffer);
      const codePtr = dv.getUint32(req + 8, true);
      const knownPtr = dv.getUint32(req + 24, true);
      seen.codeBase = dv.getBigUint64(req, true);
      seen.code = [...new Uint8Array(memory.buffer, codePtr, dv.getUint32(req + 12, true))];
      seen.entry = dv.getBigUint64(req + 16, true);
      seen.known = Array.from({ length: dv.getUint32(req + 28, true) }, (_, i) => dv.getBigUint64(knownPtr + 8 * i, true));
      seen.flags = [dv.getUint32(req + 32, true), dv.getUint32(req + 36, true)];
      return respond(memory, x.rt_alloc);
    },
  };
  return { x, allocs, frees, seen };
}

const result = (memory, alloc, status, bytes, asMsg) => {
  const data = alloc(bytes.length);
  new Uint8Array(memory.buffer).set(bytes, data);
  const r = alloc(20);
  const dv = new DataView(memory.buffer);
  dv.setUint32(r, status, true);
  dv.setUint32(r + (asMsg ? 12 : 4), data, true);
  dv.setUint32(r + (asMsg ? 16 : 8), bytes.length, true);
  return r;
};

test('the request carries code, entry and the sorted known addresses; the result is copied', () => {
  const s = standIn((m, a) => result(m, a, 0, [0, 0x61, 0x73, 0x6d], false));
  const rt = new RuntimeTranslator(s.x);
  const wasm = rt.translate({ codeBase: 0x1_4000_1000n, code: new Uint8Array([0x48, 0x01, 0xd8, 0xc3]), entry: 0x1_4000_1002n, known: new BigUint64Array([0x8001_0000n, 0x1_4000_1000n]) });
  assert.deepEqual([...wasm], [0, 0x61, 0x73, 0x6d]);
  assert.notEqual(wasm.buffer, s.x.memory.buffer);
  assert.deepEqual(s.seen, { codeBase: 0x1_4000_1000n, code: [0x48, 0x01, 0xd8, 0xc3], entry: 0x1_4000_1002n, known: [0x8001_0000n, 0x1_4000_1000n], flags: [0, 0] });
  // The three request buffers are freed with the sizes they were allocated with.
  assert.deepEqual(s.frees, s.allocs.slice(0, 3));
});

test('an error result becomes an Error with the message', () => {
  const s = standIn((m, a) => result(m, a, 1, [...new TextEncoder().encode('entry 0x5 outside the code')], true));
  const rt = new RuntimeTranslator(s.x);
  assert.throws(() => rt.translate({ codeBase: 0x1000n, code: new Uint8Array(4), entry: 0x5n, known: new BigUint64Array(0) }),
    /^Error: run-time translator: entry 0x5 outside the code$/);
  assert.equal(s.frees.length, 3, 'freed on error too');
  assert.ok(s.frees.every(([, n]) => n >= 16), 'empty buffers are still real allocations');
});

test('bad results are errors', () => {
  const bad = new RuntimeTranslator(standIn((m, a) => result(m, a, 7, [], false)).x);
  assert.throws(() => bad.translate({ codeBase: 0n, code: new Uint8Array(1), entry: 0n, known: new BigUint64Array(0) }), /unknown status 7/);
  const nul = new RuntimeTranslator(standIn(() => 0).x);
  assert.throws(() => nul.translate({ codeBase: 0n, code: new Uint8Array(1), entry: 0n, known: new BigUint64Array(0) }), /bad result pointer 0/);
});

test('load rejects a translator with imports or missing exports', async () => {
  // (module (import "env" "f" (func)))
  const withImport = new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0, 1, 4, 1, 0x60, 0, 0, 2, 9, 1, 3, 0x65, 0x6e, 0x76, 1, 0x66, 0, 0]);
  await assert.rejects(RuntimeTranslator.load(withImport), /imports env.f; it must import nothing/);
  await assert.rejects(RuntimeTranslator.load(new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0])), /lacks export "memory"/);
});
