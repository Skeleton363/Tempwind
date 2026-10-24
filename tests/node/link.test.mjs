import test from 'node:test';
import assert from 'node:assert/strict';
import { lookupHash, LOOKUP_SLOTS, LOOKUP_MASK, LOOKUP_USED, HOST_FUNC_INDEX } from '../../runtime/src/abi.mjs';
import { Linker, parseBlocks, moduleBlocks, moduleBlockCount, hostFunction, hashIndex } from '../../runtime/src/link.mjs';
import { createMemory64 } from '../../runtime/src/memory.mjs';
import { GuestMemory } from '../../runtime/src/guest-memory.mjs';
import { v3Module, blocksPayload } from './v3-mini.mjs';

const HEADER = 0x1000n;
const SLOTS = 0x2000n;

function mkLinker(slots = 16, { withAlloc = false } = {}) {
  const { memory } = createMemory64({ initialPages: 4 });
  let next = 0x10000n;
  const alloc = withAlloc ? (size, align) => {
    next = (next + align - 1n) / align * align;
    const at = next;
    next += size;
    return at;
  } : undefined;
  const linker = new Linker({ memory, header: HEADER, slotsAddr: SLOTS, slots, alloc });
  return { linker, mem: new GuestMemory(memory), memory };
}

/** Keys (multiples of 16 from `start`) whose first probe is slot `want` under `mask`. */
function keysHashingTo(want, mask, count, start = 0x1000n) {
  const out = [];
  for (let k = start; out.length < count; k += 16n) if (lookupHash(k, mask) === want) out.push(k);
  return out;
}

const slotAt = (mem, slots, i) => {
  const at = slots + BigInt(i) * 16n;
  return { key: mem.u64(at), func: mem.u32(at + 8n), entry: mem.u32(at + 12n) };
};

test('lookupHash matches the abi.rs vectors', () => {
  assert.equal(lookupHash(0x1_4000_1000n, 0xfffn), 0x30en);
  assert.equal(lookupHash(0x8001_0000n, 0xffffn), 0xbd54n);
  assert.equal(lookupHash(0x7fff_ffff_0000n, 0x3ffn), 0xb5n);
});

test('the 32-bit hash equals lookupHash for any key and mask', () => {
  let x = 0x0123_4567_89ab_cdefn;
  const next = () => (x = BigInt.asUintN(64, x * 6364136223846793005n + 1442695040888963407n));
  const masks = [0n, 1n, 0xfffn, 0xffffn, 0x7fff_ffffn, 0xffff_ffffn];
  const keys = [1n, 0xffff_ffff_ffff_ffffn, 0x8000_0000n, 0xffff_ffffn, 0x1_0000_0000n, 0x1_4000_1000n];
  for (let i = 0; i < 2000; i++) keys.push(next());
  for (const k of keys) {
    for (const m of masks) {
      assert.equal(hashIndex(Number(k & 0xffff_ffffn), Number(k >> 32n), Number(m)), Number(lookupHash(k, m)), `key ${k.toString(16)} mask ${m.toString(16)}`);
    }
  }
});

test('header is written and insertion lands on the hashed slot', () => {
  const { linker, mem } = mkLinker(16);
  assert.equal(mem.u64(HEADER + BigInt(LOOKUP_SLOTS)), SLOTS);
  assert.equal(mem.u64(HEADER + BigInt(LOOKUP_MASK)), 15n);
  assert.equal(mem.u64(HEADER + BigInt(LOOKUP_USED)), 0n);
  assert.equal(mem.u64(HEADER + 24n), 0n);
  const key = 0x1_4000_1000n;
  assert.equal(linker.insert(key, 7, 3), true);
  assert.deepEqual(slotAt(mem, SLOTS, Number(lookupHash(key, 15n))), { key, func: 7, entry: 3 });
  assert.equal(mem.u64(HEADER + BigInt(LOOKUP_USED)), 1n);
  assert.deepEqual(linker.lookup(key), { func: 7, entry: 3 });
  assert.equal(linker.lookup(key + 1n), null);
});

test('first insertion wins', () => {
  const { linker, mem } = mkLinker(16);
  assert.equal(linker.insert(0x5000n, 1, 1), true);
  assert.equal(linker.insert(0x5000n, 2, 2), false);
  assert.deepEqual(linker.lookup(0x5000n), { func: 1, entry: 1 });
  assert.equal(linker.used, 1);
  assert.equal(mem.u64(HEADER + BigInt(LOOKUP_USED)), 1n);
});

test('collisions probe linearly', () => {
  const { linker, mem } = mkLinker(16);
  const [a, b, c] = keysHashingTo(5n, 15n, 3);
  linker.insert(a, 1, 10);
  linker.insert(b, 2, 20);
  linker.insert(c, 3, 30);
  assert.equal(slotAt(mem, SLOTS, 5).key, a);
  assert.equal(slotAt(mem, SLOTS, 6).key, b);
  assert.equal(slotAt(mem, SLOTS, 7).key, c);
  assert.deepEqual(linker.lookup(c), { func: 3, entry: 30 });
});

test('probing wraps around the slot array', () => {
  const { linker, mem } = mkLinker(16);
  const [a, b] = keysHashingTo(15n, 15n, 2);
  linker.insert(a, 1, 1);
  linker.insert(b, 2, 2);
  assert.equal(slotAt(mem, SLOTS, 15).key, a);
  assert.equal(slotAt(mem, SLOTS, 0).key, b);
  assert.deepEqual(linker.lookup(b), { func: 2, entry: 2 });
});

test('rehash keeps 2 * used <= slots and every key', () => {
  const { linker, mem } = mkLinker(4, { withAlloc: true });
  const keys = Array.from({ length: 40 }, (_, i) => 0x1_4000_0000n + BigInt(i) * 0x30n);
  keys.forEach((k, i) => {
    assert.equal(linker.insert(k, i, i + 100), true);
    const slots = Number(mem.u64(HEADER + BigInt(LOOKUP_MASK))) + 1;
    assert.ok(2 * (i + 1) <= slots, `${i + 1} keys in ${slots} slots`);
  });
  assert.equal(mem.u64(HEADER + BigInt(LOOKUP_MASK)), 127n);
  assert.notEqual(mem.u64(HEADER + BigInt(LOOKUP_SLOTS)), SLOTS);
  assert.equal(mem.u64(HEADER + BigInt(LOOKUP_USED)), 40n);
  keys.forEach((k, i) => assert.deepEqual(linker.lookup(k), { func: i, entry: i + 100 }));
  // The new array holds exactly the keys, each where the abi.md probe finds it.
  const slots = mem.u64(HEADER + BigInt(LOOKUP_SLOTS));
  let n = 0;
  for (let i = 0; i < 128; i++) if (slotAt(mem, slots, i).key !== 0n) n++;
  assert.equal(n, 40);
});

test('a full table without an allocator is an error', () => {
  const { linker } = mkLinker(4);
  linker.insert(0x10n, 0, 0);
  linker.insert(0x20n, 0, 0);
  assert.throws(() => linker.insert(0x30n, 0, 0), /lookup table full/);
});

test('key 0 and non-u64 keys are rejected', () => {
  const { linker } = mkLinker(16);
  assert.throws(() => linker.insert(0n, 0, 0), /nonzero u64/);
  assert.throws(() => linker.insert(1n << 64n, 0, 0), /nonzero u64/);
});

test('constructor checks alignment and slot count', () => {
  const { memory } = createMemory64({ initialPages: 1 });
  assert.throws(() => new Linker({ memory, header: 0x1004n, slotsAddr: SLOTS, slots: 16 }), /8-byte/);
  assert.throws(() => new Linker({ memory, header: HEADER, slotsAddr: 0x2008n, slots: 16 }), /16-byte/);
  assert.throws(() => new Linker({ memory, header: HEADER, slotsAddr: SLOTS, slots: 12 }), /power of two/);
});

test('sortedKeys mirrors the table in ascending order', () => {
  const { linker } = mkLinker(4, { withAlloc: true });
  const keys = [0x9000n, 0x1000n, 0x5000n, 0x1_4000_0000n, 0x3000n];
  for (const k of keys) linker.insert(k, 0, 0);
  linker.insert(0x5000n, 1, 1);
  assert.deepEqual([...linker.sortedKeys()], [...keys].sort((a, b) => (a < b ? -1 : 1)));
  linker.insert(0x2000n, 0, 0);
  assert.deepEqual([...linker.sortedKeys()], [0x1000n, 0x2000n, 0x3000n, 0x5000n, 0x9000n, 0x1_4000_0000n]);
});

test('parseBlocks reads a valid section', () => {
  const p = blocksPayload(2, [
    { addr: 0x140001000n, chunk: 0, entry: 0 },
    { addr: 0x140001010n, chunk: 0, entry: 1 },
    { addr: 0x140002000n, chunk: 1, entry: 0 },
  ]);
  const r = parseBlocks(p);
  assert.equal(r.chunks, 2);
  assert.deepEqual(r.blocks[2], { addr: 0x140002000n, chunk: 1, entry: 0 });
  assert.deepEqual(parseBlocks(p.buffer).blocks.length, 3);
});

for (const [label, bytes, re] of [
  ['short', new Uint8Array(8), /shorter than its header/],
  ['version', blocksPayload(1, [], { version: 2 }), /version 2/],
  ['reserved', blocksPayload(1, [], { reserved: 1 }), /reserved/],
  ['length', blocksPayload(1, [{ addr: 0x10n, chunk: 0, entry: 0 }]).subarray(0, 24), /expected 32/],
  ['unsorted', blocksPayload(1, [{ addr: 0x20n, chunk: 0, entry: 0 }, { addr: 0x10n, chunk: 0, entry: 1 }]), /unsorted/],
  ['duplicate', blocksPayload(1, [{ addr: 0x20n, chunk: 0, entry: 0 }, { addr: 0x20n, chunk: 0, entry: 1 }]), /duplicate/],
  ['chunk', blocksPayload(1, [{ addr: 0x20n, chunk: 1, entry: 0 }]), /chunk 1 of 1/],
  ['address 0', blocksPayload(1, [{ addr: 0n, chunk: 0, entry: 0 }]), /address 0/],
]) {
  test(`parseBlocks rejects a bad section: ${label}`, () => assert.throws(() => parseBlocks(bytes), re));
}

test('moduleBlocks requires exactly one section', () => {
  const none = new WebAssembly.Module(v3Module({ run: [0x41, 1], chunks: [], blocks: [], blocksSections: 0 }));
  assert.throws(() => moduleBlocks(none), /0 x64wasm.blocks sections/);
  const two = new WebAssembly.Module(v3Module({ run: [0x41, 1], chunks: [], blocks: [], blocksSections: 2 }));
  assert.throws(() => moduleBlocks(two), /2 x64wasm.blocks sections/);
  const one = new WebAssembly.Module(v3Module({ run: [0x41, 1], chunks: [[0x41, 0]], blocks: [{ addr: 0x10n, chunk: 0, entry: 0 }, { addr: 0x20n, chunk: 0, entry: 1 }] }));
  assert.equal(moduleBlockCount(one), 2);
});

test('blocks above 4 GiB, and keys differing only in the high word', async () => {
  const { linker } = mkLinker(16);
  const lo = 0x1000n;
  for (const k of [lo, (1n << 32n) | lo, (2n << 32n) | lo, 0xffff_ffff_0000_1000n]) {
    assert.equal(linker.insert(k, 1, Number(k >> 32n) & 0xff), true);
  }
  assert.deepEqual(linker.lookup((2n << 32n) | lo), { func: 1, entry: 2 });
  assert.deepEqual(linker.lookup(0xffff_ffff_0000_1000n), { func: 1, entry: 0xff });
  assert.equal(linker.lookup(3n << 32n | lo), null);
  assert.deepEqual([...linker.sortedKeys()], [lo, (1n << 32n) | lo, (2n << 32n) | lo, 0xffff_ffff_0000_1000n]);
});

test('link grows the table, places chunks at table_base and inserts blocks', async () => {
  const { linker } = mkLinker(16);
  const chunk = (v) => [0x41, ...[v]]; // i32.const v (v < 64)
  const a = await linker.link(new WebAssembly.Module(v3Module({
    run: [0x41, 1], chunks: [chunk(10), chunk(11)],
    blocks: [{ addr: 0x1000n, chunk: 0, entry: 0 }, { addr: 0x1010n, chunk: 0, entry: 1 }, { addr: 0x2000n, chunk: 1, entry: 0 }],
  })));
  assert.equal(a.tableBase, HOST_FUNC_INDEX + 1);
  assert.equal(linker.table.length, 3);
  assert.equal(a.inserted, 3);
  assert.equal(linker.table.get(a.tableBase + 1)(0n, 0, 0n), 11);
  assert.deepEqual(linker.lookup(0x1010n), { func: a.tableBase, entry: 1 });
  assert.equal(a.run(0n), 1);

  const b = await linker.link(new WebAssembly.Module(v3Module({
    run: [0x41, 2], chunks: [chunk(20)],
    blocks: [{ addr: 0x2000n, chunk: 0, entry: 5 }, { addr: 0x3000n, chunk: 0, entry: 0 }],
  })));
  assert.equal(b.tableBase, 3);
  assert.equal(b.inserted, 1);
  assert.deepEqual(linker.lookup(0x2000n), { func: a.tableBase + 1, entry: 0 }, 'first translation wins');
  assert.deepEqual(linker.lookup(0x3000n), { func: 3, entry: 0 });
  assert.equal(linker.table.get(3)(0n, 0, 0n), 20);
  assert.equal(linker.depth.value, 0);
  assert.equal(linker.lookupGlobal.value, HEADER);
});

test('a host function sits in table slot 0 with the guest-function type', () => {
  const { linker } = mkLinker(16);
  const seen = [];
  linker.setHostFunction((state, entry, ret) => { seen.push([state, entry, ret]); return 2; });
  assert.equal(linker.table.get(HOST_FUNC_INDEX)(0x8000_1000n, 7, -1n), 2);
  assert.deepEqual(seen, [[0x8000_1000n, 7, -1n]]);
  // The wrapper has exactly the (i64, i32, i64) -> i32 type: an i64 must be a BigInt.
  assert.throws(() => hostFunction(() => 0)(1, 0, 0n), TypeError);
});
