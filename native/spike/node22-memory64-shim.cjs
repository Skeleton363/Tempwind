// Preload (`node --require ./node22-memory64-shim.cjs app.cjs`) that lets
// Emscripten 6 memory64 output run on Node 22.
//
// Node 22's V8 (12.4) implements an older memory64 JS API: descriptors use
// `index: 'i64'` and Number sizes, and Memory#grow / Table#get / Table#set /
// Table#grow take Numbers. Emscripten 6 emits the final API (`address: 'i64'`
// and BigInt sizes and indices). This shim translates between them. It is a
// no-op on engines that already accept the final API (Node 24+, Chrome 133+).
'use strict';

const W = globalThis.WebAssembly;

function acceptsFinalApi() {
  try {
    new W.Memory({ initial: 1n, maximum: 1n, address: 'i64' });
    return true;
  } catch {
    return false;
  }
}

if (!acceptsFinalApi()) {
  const num = (v) => (typeof v === 'bigint' ? Number(v) : v);
  const fixDescriptor = (d) => {
    if (!d || typeof d !== 'object') return d;
    const out = { ...d };
    if (out.address !== undefined) {
      out.index = out.address;
      delete out.address;
    }
    for (const k of ['initial', 'maximum', 'minimum']) {
      if (k in out) out[k] = num(out[k]);
    }
    return out;
  };

  const OrigMemory = W.Memory;
  const Memory = function Memory(desc) {
    return Reflect.construct(OrigMemory, [fixDescriptor(desc)], new.target || Memory);
  };
  Memory.prototype = OrigMemory.prototype;
  Object.setPrototypeOf(Memory, OrigMemory);
  const origGrow = OrigMemory.prototype.grow;
  OrigMemory.prototype.grow = function grow(delta) {
    const r = origGrow.call(this, num(delta));
    return typeof delta === 'bigint' ? BigInt(r) : r;
  };
  W.Memory = Memory;

  const OrigTable = W.Table;
  const Table = function Table(desc, value) {
    const args = arguments.length > 1 ? [fixDescriptor(desc), value] : [fixDescriptor(desc)];
    return Reflect.construct(OrigTable, args, new.target || Table);
  };
  Table.prototype = OrigTable.prototype;
  Object.setPrototypeOf(Table, OrigTable);
  const tp = OrigTable.prototype;
  const origGet = tp.get;
  const origSet = tp.set;
  const origTGrow = tp.grow;
  tp.get = function get(i) {
    return origGet.call(this, num(i));
  };
  tp.set = function set(i, ...rest) {
    return origSet.call(this, num(i), ...rest);
  };
  tp.grow = function grow(delta, ...rest) {
    const r = origTGrow.call(this, num(delta), ...rest);
    return typeof delta === 'bigint' ? BigInt(r) : r;
  };
  W.Table = Table;
}
