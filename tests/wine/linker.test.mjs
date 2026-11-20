// The Linker in a Wine process (abi.md v5, "Lookup table"; wine-unix.md,
// section 6, "Invalidation"): an injected table with the reserved slots,
// tombstones, the whole-module rule, PAGE_CODE, and the rehash that drops
// tombstones. Synthetic modules (runtime/src/wasm-mini.mjs) and a plain
// memory; no Unix side.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  LOOKUP_TOMBSTONE, TOMBSTONE_INDEX, TABLE_RESERVED, LOOKUP_USED, PAGE_CODE, lookupHash,
} from '../../runtime/src/abi.mjs';
import { Linker } from '../../runtime/src/link.mjs';
import { createMemory64 } from '../../runtime/src/memory.mjs';
import { GuestMemory } from '../../runtime/src/guest-memory.mjs';
import { v3Module } from '../node/v3-mini.mjs';

const HEADER = 0x1000n;
const SLOTS = 0x2000n;
const PAGE_FLAGS = 0x10_0000n; // one byte per page: pages below 0x2_0000 * 4 KiB here
const RET0 = [0x41, 0x00]; // i32.const 0

function mkLinker(slots = 16) {
  const { memory } = createMemory64({ initialPages: 64 });
  let next = 0x8_0000n;
  const allocs = [];
  const alloc = (size, align) => {
    next = (next + align - 1n) / align * align;
    const at = next;
    next += size;
    allocs.push(at);
    return at;
  };
  const freed = [];
  const table = new WebAssembly.Table({ element: 'anyfunc', initial: TABLE_RESERVED });
  const depth = new WebAssembly.Global({ value: 'i32', mutable: true }, 0);
  const linker = new Linker({
    memory, header: HEADER, slotsAddr: SLOTS, slots, alloc, free: (a) => freed.push(a),
    table, depth, pageFlagsAddr: PAGE_FLAGS,
  });
  return { linker, mem: new GuestMemory(memory), table, depth, freed };
}

/** A module of `chunks` chunks with blocks at `addrs` (block i in chunk i % chunks). */
function mod(addrs, chunks = 1) {
  const bytes = v3Module({
    run: RET0,
    chunks: Array.from({ length: chunks }, () => RET0),
    blocks: addrs.map((addr, i) => ({ addr, chunk: i % chunks, entry: i })),
  });
  return new WebAssembly.Module(bytes);
}

const pageFlag = (mem, addr) => mem.u8get(PAGE_FLAGS + (BigInt(addr) >> 12n));
const slot = (mem, linker, key) => {
  // the slot the probe for key stops at
  const mask = BigInt(linker.slotMask);
  for (let i = lookupHash(BigInt(key), mask); ; i = (i + 1n) & mask) {
    const at = linker.slotsAddr + i * 16n;
    const k = mem.u64(at);
    if (k === BigInt(key) || k === 0n) return { at, key: k, func: mem.u32(at + 8n), entry: mem.u32(at + 12n) };
  }
};

test('the injected table and depth are the ones modules import; chunks start after the reserved slots', () => {
  const { linker, table, depth } = mkLinker();
  assert.equal(linker.table, table);
  assert.equal(linker.depth, depth);
  const l = linker.linkSync(mod([0x40_1000n]));
  assert.equal(l.tableBase, TABLE_RESERVED);
  assert.throws(() => linker.setHostFunction(() => 0), /RUN_INDEX/);
});

test('invalidate: every block of every module overlapping the range goes, whole; others stay', () => {
  const { linker, mem } = mkLinker(64);
  // A: two code pages, blocks on both; B: elsewhere; C: overlaps A's second page.
  const a = linker.linkSync(mod([0x40_1000n, 0x40_1800n, 0x40_2400n], 2), { ranges: [[0x40_1000n, 0x2000n]] });
  const b = linker.linkSync(mod([0x50_0000n, 0x50_0010n]), { ranges: [[0x50_0000n, 0x1000n]] });
  const c = linker.linkSync(mod([0x40_2800n]), { ranges: [[0x40_2000n, 0x1000n]] });
  for (const l of [a, b, c]) for (const [s, n] of l.ranges) linker.setPageFlags(s, n, PAGE_CODE);
  const live = linker.live;

  // One byte of A's first page: A goes whole (its block on the second page
  // too); C, which shares only A's second page, stays.
  const removed = linker.invalidate(0x40_1234n, 1n);
  assert.deepEqual(removed, [a]);
  for (const k of [0x40_1000n, 0x40_1800n, 0x40_2400n]) {
    assert.equal(linker.lookup(k), null, `0x${k.toString(16)} still has a block`);
    const s = slot(mem, linker, k);
    assert.equal(s.key, 0n); // the probe passes the tombstone and ends at an empty slot
  }
  assert.deepEqual(linker.lookup(0x40_2800n), { func: c.tableBase, entry: 0 });
  assert.deepEqual(linker.lookup(0x50_0010n), { func: b.tableBase, entry: 1 });
  assert.equal(linker.live, live - 3);
  assert.equal(linker.used, live, 'tombstones still count in used until the rehash');
  assert.equal(mem.u64(HEADER + BigInt(LOOKUP_USED)), BigInt(live));
  // PAGE_CODE: cleared on A's first page, kept on the page C still covers.
  assert.equal(pageFlag(mem, 0x40_1000n) & PAGE_CODE, 0);
  assert.equal(pageFlag(mem, 0x40_2000n) & PAGE_CODE, PAGE_CODE);
  assert.equal(pageFlag(mem, 0x50_0000n) & PAGE_CODE, PAGE_CODE);
  // The run-time translator's known set no longer lists A's blocks.
  assert.deepEqual([...linker.sortedKeys()], [0x40_2800n, 0x50_0000n, 0x50_0010n]);
  // A range touching nothing linked removes nothing.
  assert.deepEqual(linker.invalidate(0x60_0000n, 0x1000n), []);
});

test('invalidate: a tombstone is key LOOKUP_TOMBSTONE, func TOMBSTONE_INDEX, entry 0, and an insert reuses it', () => {
  const { linker, mem } = mkLinker(16);
  const mask = BigInt(linker.slotMask);
  // Two keys with the same first slot: the second sits one further along.
  const keys = [];
  for (let k = 0x1000n; keys.length < 3; k += 16n) if (lookupHash(k, mask) === 5n) keys.push(k);
  const a = linker.linkSync(mod([keys[0]]), { ranges: [[keys[0], 1n]] });
  linker.linkSync(mod([keys[1]]), { ranges: [[keys[1], 1n]] });
  linker.invalidate(keys[0], 1n);
  const at = linker.slotsAddr + 5n * 16n;
  assert.equal(mem.u64(at), LOOKUP_TOMBSTONE);
  assert.equal(mem.u32(at + 8n), TOMBSTONE_INDEX);
  assert.equal(mem.u32(at + 12n), 0);
  // The probe passes over it: keys[1], one slot further, is still found.
  assert.notEqual(linker.lookup(keys[1]), null);
  // A new key on the same probe path takes the tombstone's slot; used doesn't grow.
  const used = linker.used;
  assert.equal(linker.insert(keys[2], 7, 9), true);
  assert.equal(mem.u64(at), keys[2]);
  assert.equal(linker.used, used);
  // The removed key may come back.
  assert.equal(linker.insert(keys[0], a.tableBase, 0), true);
  assert.deepEqual(linker.lookup(keys[0]), { func: a.tableBase, entry: 0 });
});

test('invalidate: a slot another module translated first is not the removed module\'s to take', () => {
  const { linker } = mkLinker(16);
  const first = linker.linkSync(mod([0x40_1000n]), { ranges: [[0x40_1000n, 0x10n]] });
  // second also has a block at 0x40_1000 (first wins), and one of its own
  const second = linker.linkSync(mod([0x40_1000n, 0x70_0000n]), { ranges: [[0x70_0000n, 0x10n]] });
  assert.equal(second.inserted, 1);
  linker.invalidate(0x70_0000n, 1n);
  assert.deepEqual(linker.lookup(0x40_1000n), { func: first.tableBase, entry: 0 });
  assert.equal(linker.lookup(0x70_0000n), null);
});

test('rehash: growth after tombstones keeps every live key, drops the tombstones and frees the old slots', () => {
  const { linker, mem, freed } = mkLinker(16);
  // 6 modules of one block each, at distinct pages; remove 4 of them.
  const addrs = Array.from({ length: 6 }, (_, i) => 0x40_0000n + BigInt(i) * 0x1000n);
  const mods = addrs.map((a) => linker.linkSync(mod([a]), { ranges: [[a, 0x10n]] }));
  for (const a of addrs.slice(0, 4)) linker.invalidate(a, 1n);
  assert.equal(linker.used, 6);
  assert.equal(linker.live, 2);
  // Insert until the rehash: an insert that reuses a tombstone leaves `used`
  // alone; one into an empty slot that would make 2 * used > slots rehashes.
  // With few live keys it stays at 16 slots, in a new array, without tombstones.
  const oldSlots = linker.slotsAddr;
  const more = [];
  while (linker.slotsAddr === oldSlots) {
    assert.ok(more.length < 8, 'no rehash');
    const usedBefore = linker.used;
    const a = 0x90_0000n + BigInt(more.length) * 0x1000n;
    more.push(a);
    linker.linkSync(mod([a]), { ranges: [[a, 0x10n]] });
    if (linker.slotsAddr === oldSlots) assert.ok(2 * linker.used <= 16);
    else assert.equal(usedBefore, 8, 'the rehash came before the table was half full');
  }
  assert.deepEqual(freed, [oldSlots]);
  const live = 2 + more.length;
  assert.equal(linker.slotCount, 16);
  assert.equal(linker.used, live);
  assert.equal(linker.live, live);
  assert.equal(mem.u64(HEADER + BigInt(LOOKUP_USED)), BigInt(live));
  for (let i = 0; i < 16; i++) assert.notEqual(mem.u64(linker.slotsAddr + BigInt(i) * 16n), LOOKUP_TOMBSTONE);
  for (const [i, a] of addrs.entries()) {
    assert.deepEqual(linker.lookup(a), i < 4 ? null : { func: mods[i].tableBase, entry: 0 });
  }
  for (const a of more) assert.notEqual(linker.lookup(a), null);
  assert.deepEqual([...linker.sortedKeys()], [addrs[4], addrs[5], ...more]);
  // Many live keys: the rehash doubles.
  for (let i = 0; i < 8; i++) linker.insert(0xa0_0000n + BigInt(i) * 16n, 3, i);
  assert.equal(linker.slotCount, 32);
  assert.equal(linker.live, live + 8);
});

test('a Linker without an injected table cannot remove keys (its slot 1 is no tombstone)', () => {
  const { memory } = createMemory64({ initialPages: 4 });
  const linker = new Linker({ memory, header: HEADER, slotsAddr: SLOTS, slots: 16 });
  linker.linkSync(mod([0x40_1000n]));
  assert.throws(() => linker.invalidate(0x40_1000n, 1n), /tombstone/);
});
