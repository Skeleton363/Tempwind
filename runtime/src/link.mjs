// The v3 linker (docs/design/abi.md, "Linking a module", "Lookup table"):
// the process-wide funcref table, the `table_base`/`lookup`/`depth` globals,
// the lookup table in guest memory, and module linking. Standard WebAssembly
// APIs only (Node, page, Worker).

import {
  MEMORY_MODULE, MEMORY_NAME, TABLE_NAME, TABLE_BASE_NAME, LOOKUP_NAME, DEPTH_NAME,
  LOOKUP_SLOTS, LOOKUP_MASK, LOOKUP_USED, LOOKUP_SLOT_SIZE,
  BLOCKS_SECTION, BLOCKS_VERSION, BLOCKS_HEADER_SIZE, BLOCKS_ENTRY_SIZE,
  HOST_FUNC_INDEX, lookupHash,
} from './abi.mjs';
import { GuestMemory } from './guest-memory.mjs';

const hex = (v) => '0x' + BigInt(v).toString(16);
const isPow2 = (n) => Number.isInteger(n) && n > 0 && (n & (n - 1)) === 0;

/**
 * Parses an `x64wasm.blocks` section.
 * @param {ArrayBuffer|Uint8Array} section
 * @returns {{chunks: number, blocks: {addr: bigint, chunk: number, entry: number}[]}}
 */
export function parseBlocks(section) {
  const u8 = section instanceof Uint8Array ? section : new Uint8Array(section);
  const dv = new DataView(u8.buffer, u8.byteOffset, u8.byteLength);
  if (u8.length < BLOCKS_HEADER_SIZE) throw new Error(`${BLOCKS_SECTION}: ${u8.length} bytes, shorter than its header`);
  const version = dv.getUint32(0, true);
  const chunks = dv.getUint32(4, true);
  const n = dv.getUint32(8, true);
  const reserved = dv.getUint32(12, true);
  if (version !== BLOCKS_VERSION) throw new Error(`${BLOCKS_SECTION}: version ${version}, expected ${BLOCKS_VERSION}`);
  if (reserved !== 0) throw new Error(`${BLOCKS_SECTION}: reserved field is ${reserved}`);
  const want = BLOCKS_HEADER_SIZE + n * BLOCKS_ENTRY_SIZE;
  if (u8.length !== want) throw new Error(`${BLOCKS_SECTION}: ${u8.length} bytes for ${n} blocks, expected ${want}`);
  const blocks = new Array(n);
  let prev = -1n;
  for (let i = 0; i < n; i++) {
    const o = BLOCKS_HEADER_SIZE + i * BLOCKS_ENTRY_SIZE;
    const addr = dv.getBigUint64(o, true);
    const chunk = dv.getUint32(o + 8, true);
    const entry = dv.getUint32(o + 12, true);
    if (addr === 0n) throw new Error(`${BLOCKS_SECTION}: block ${i} has address 0`);
    if (addr <= prev) throw new Error(`${BLOCKS_SECTION}: block ${i} at ${hex(addr)} is not above ${hex(prev)} (unsorted or duplicate)`);
    if (chunk >= chunks) throw new Error(`${BLOCKS_SECTION}: block ${hex(addr)} names chunk ${chunk} of ${chunks}`);
    blocks[i] = { addr, chunk, entry };
    prev = addr;
  }
  return { chunks, blocks };
}

/** The parsed `x64wasm.blocks` section of a compiled module (exactly one required). */
export function moduleBlocks(module) {
  const secs = WebAssembly.Module.customSections(module, BLOCKS_SECTION);
  if (secs.length !== 1) throw new Error(`module has ${secs.length} ${BLOCKS_SECTION} sections, expected 1`);
  return parseBlocks(secs[0]);
}

const HOST_FUNC_MODULE = new Uint8Array([
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
  // type section: (i64, i32, i64) -> i32
  0x01, 0x08, 0x01, 0x60, 0x03, 0x7e, 0x7f, 0x7e, 0x01, 0x7f,
  // import section: env.f, func type 0
  0x02, 0x09, 0x01, 0x03, 0x65, 0x6e, 0x76, 0x01, 0x66, 0x00, 0x00,
  // export section: "f" = func 0
  0x07, 0x05, 0x01, 0x01, 0x66, 0x00, 0x00,
]);
let hostFuncModule = null;

/**
 * Wraps `fn(state: bigint, entry: number, ret: bigint) -> number` as a wasm
 * function of the guest-function type, so it can sit in `env.table`.
 */
export function hostFunction(fn) {
  hostFuncModule ??= new WebAssembly.Module(HOST_FUNC_MODULE);
  return new WebAssembly.Instance(hostFuncModule, { env: { f: fn } }).exports.f;
}

/**
 * The table, globals and lookup table of one process (or one test instance).
 */
export class Linker {
  /**
   * @param {{memory: WebAssembly.Memory, header: bigint, slotsAddr: bigint, slots: number,
   *   alloc?: (size: bigint, align: bigint) => bigint}} opts
   *   `header`: guest address of the 32-byte lookup header (8-byte aligned);
   *   `slotsAddr`/`slots`: the first slot array (16-byte aligned, power-of-two count);
   *   `alloc`: zeroed guest memory for larger slot arrays (rehash). Without it, a full table is an error.
   */
  constructor({ memory, header, slotsAddr, slots, alloc }) {
    header = BigInt(header);
    slotsAddr = BigInt(slotsAddr);
    if (header % 8n) throw new Error(`lookup header ${hex(header)} is not 8-byte aligned`);
    if (slotsAddr % 16n) throw new Error(`lookup slots ${hex(slotsAddr)} are not 16-byte aligned`);
    if (!isPow2(slots)) throw new Error(`lookup slot count ${slots} is not a power of two`);
    this.memory = memory;
    this.mem = new GuestMemory(memory);
    this.header = header;
    this.alloc = alloc;
    this.table = new WebAssembly.Table({ element: 'anyfunc', initial: HOST_FUNC_INDEX + 1 });
    this.lookupGlobal = new WebAssembly.Global({ value: 'i64', mutable: false }, header);
    this.depth = new WebAssembly.Global({ value: 'i32', mutable: true }, 0);
    this.used = 0;
    this.keyMirror = new BigUint64Array(64);
    this.sorted = true;
    this.modules = [];
    this.setSlots(slotsAddr, slots);
    this.mem.setU64(header + BigInt(LOOKUP_USED), 0n);
    this.mem.setU64(header + 24n, 0n);
  }

  setSlots(addr, count) {
    this.mem.fill(addr, count * LOOKUP_SLOT_SIZE);
    this.slotsAddr = addr;
    this.mask = BigInt(count - 1);
    this.mem.setU64(this.header + BigInt(LOOKUP_SLOTS), addr);
    this.mem.setU64(this.header + BigInt(LOOKUP_MASK), this.mask);
  }

  get slotCount() { return Number(this.mask) + 1; }

  /** Puts the host-call function (a JS function, see `hostFunction`) into table slot 0. */
  setHostFunction(fn) {
    this.table.set(HOST_FUNC_INDEX, hostFunction(fn));
  }

  /** Probes for `key`: {func, entry} or null. Follows abi.md's probe exactly. */
  lookup(key) {
    key = BigInt.asUintN(64, BigInt(key));
    const mem = this.mem;
    let i = lookupHash(key, this.mask);
    for (;;) {
      const at = this.slotsAddr + i * BigInt(LOOKUP_SLOT_SIZE);
      const k = mem.u64(at);
      if (k === key) return { func: mem.u32(at + 8n), entry: mem.u32(at + 12n) };
      if (k === 0n) return null;
      i = (i + 1n) & this.mask;
    }
  }

  /** Inserts a slot unless `key` is present (first wins). Returns whether it inserted. */
  insert(key, func, entry) {
    key = BigInt(key);
    if (key <= 0n || key >= 1n << 64n) throw new Error(`lookup key ${hex(key)} is not a nonzero u64`);
    if (this.lookup(key)) return false;
    if (2 * (this.used + 1) > this.slotCount) this.grow(this.used + 1);
    this.place(key, func, entry);
    this.used++;
    this.mem.setU64(this.header + BigInt(LOOKUP_USED), BigInt(this.used));
    if (this.used > this.keyMirror.length) {
      const bigger = new BigUint64Array(this.keyMirror.length * 2);
      bigger.set(this.keyMirror);
      this.keyMirror = bigger;
    }
    if (this.sorted && this.used > 1 && this.keyMirror[this.used - 2] > key) this.sorted = false;
    this.keyMirror[this.used - 1] = key;
    return true;
  }

  /** Writes a slot for a key known to be absent, at its first free probe position. */
  place(key, func, entry) {
    let i = lookupHash(key, this.mask);
    for (;;) {
      const at = this.slotsAddr + i * BigInt(LOOKUP_SLOT_SIZE);
      if (this.mem.u64(at) === 0n) {
        this.mem.setU32(at + 8n, func);
        this.mem.setU32(at + 12n, entry);
        this.mem.setU64(at, key);
        return;
      }
      i = (i + 1n) & this.mask;
    }
  }

  /** A new slot array with `2 * want <= slots`; reinserts every slot. */
  grow(want) {
    if (!this.alloc) throw new Error(`lookup table full (${this.used} of ${this.slotCount} slots) and no allocator to grow it`);
    let count = this.slotCount;
    while (2 * want > count) count *= 2;
    const old = [];
    for (let i = 0; i < this.slotCount; i++) {
      const at = this.slotsAddr + BigInt(i * LOOKUP_SLOT_SIZE);
      const k = this.mem.u64(at);
      if (k !== 0n) old.push([k, this.mem.u32(at + 8n), this.mem.u32(at + 12n)]);
    }
    const addr = BigInt(this.alloc(BigInt(count * LOOKUP_SLOT_SIZE), 16n));
    if (addr % 16n) throw new Error(`lookup slots ${hex(addr)} are not 16-byte aligned`);
    this.setSlots(addr, count);
    for (const [k, f, e] of old) this.place(k, f, e);
  }

  /** Every key in the lookup table, sorted ascending (the run-time translator's `known`). */
  sortedKeys() {
    const keys = this.keyMirror.subarray(0, this.used);
    if (!this.sorted) { keys.sort(); this.sorted = true; }
    return keys;
  }

  /**
   * Links a translated module: grows the table, instantiates, inserts its blocks.
   * @param {WebAssembly.Module} module
   * @returns {Promise<{instance: WebAssembly.Instance, run: Function, tableBase: number,
   *   chunks: number, blocks: number, inserted: number}>}
   */
  async link(module) {
    const { chunks, blocks } = moduleBlocks(module);
    const tableBase = this.table.length;
    this.table.grow(chunks);
    const instance = await WebAssembly.instantiate(module, this.imports(tableBase));
    let inserted = 0;
    for (const b of blocks) if (this.insert(b.addr, tableBase + b.chunk, b.entry)) inserted++;
    const run = instance.exports.run;
    if (typeof run !== 'function') throw new Error('module lacks export "run"');
    const linked = { instance, run, tableBase, chunks, blocks: blocks.length, inserted };
    this.modules.push(linked);
    return linked;
  }

  /** The import object for a module whose chunk 0 sits at `tableBase`. */
  imports(tableBase) {
    return {
      [MEMORY_MODULE]: {
        [MEMORY_NAME]: this.memory,
        [TABLE_NAME]: this.table,
        [TABLE_BASE_NAME]: new WebAssembly.Global({ value: 'i32', mutable: false }, tableBase),
        [LOOKUP_NAME]: this.lookupGlobal,
        [DEPTH_NAME]: this.depth,
      },
    };
  }
}
