// The v3 linker (docs/design/abi.md, "Linking a module", "Lookup table"):
// the process-wide funcref table, the `table_base`/`lookup`/`depth` globals,
// the lookup table in guest memory, and module linking. Standard WebAssembly
// APIs only (Node, page, Worker).

import {
  MEMORY_MODULE, MEMORY_NAME, TABLE_NAME, TABLE_BASE_NAME, LOOKUP_NAME, DEPTH_NAME,
  LOOKUP_SLOTS, LOOKUP_MASK, LOOKUP_USED, LOOKUP_SLOT_SIZE,
  BLOCKS_SECTION, BLOCKS_VERSION, BLOCKS_HEADER_SIZE, BLOCKS_ENTRY_SIZE,
  HOST_FUNC_INDEX, LOOKUP_HASH_MUL,
} from './abi.mjs';
import { GuestMemory } from './guest-memory.mjs';
import { FP_NAMESPACE, fpImports } from './fp.mjs';

const hex = (v) => '0x' + BigInt(v).toString(16);
const isPow2 = (n) => Number.isInteger(n) && n > 0 && (n & (n - 1)) === 0;

// Hot paths use 32-bit Numbers, not BigInt: a key is (lo, hi) u32 halves,
// guest addresses of the lookup table are Numbers (< 2^53).
const MUL_LO = Number(LOOKUP_HASH_MUL & 0xffff_ffffn);
const MUL_HI = Number(LOOKUP_HASH_MUL >> 32n);
const LITTLE = new Uint8Array(new Uint32Array([1]).buffer)[0] === 1;

/** High 32 bits of the 64-bit product of two u32s. */
function mulHi32(x, y) {
  const x0 = x & 0xffff, x1 = x >>> 16, y0 = y & 0xffff, y1 = y >>> 16;
  const p01 = x0 * y1, p10 = x1 * y0;
  const mid = ((x0 * y0) >>> 16) + (p01 & 0xffff) + (p10 & 0xffff);
  return (x1 * y1 + (p01 >>> 16) + (p10 >>> 16) + (mid >>> 16)) >>> 0;
}

/**
 * `lookupHash` (abi.md) for the key (hi << 32 | lo) with a mask below 2^32:
 * bits 32..63 of key * LOOKUP_HASH_MUL mod 2^64, masked.
 */
export function hashIndex(lo, hi, mask) {
  return ((mulHi32(lo, MUL_LO) + Math.imul(hi, MUL_LO) + Math.imul(lo, MUL_HI)) & mask) >>> 0;
}

const splitKey = (key) => {
  key = BigInt(key);
  if (key <= 0n || key >= 1n << 64n) throw new Error(`lookup key ${hex(key)} is not a nonzero u64`);
  return [Number(key & 0xffff_ffffn), Number(key >> 32n)];
};
const joinKey = (lo, hi) => (BigInt(hi) << 32n) | BigInt(lo);

/**
 * Validates an `x64wasm.blocks` section without materializing it.
 * @returns {{chunks: number, count: number, dv: DataView}} entry i at BLOCKS_HEADER_SIZE + 16 i of dv
 */
export function readBlocks(section) {
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
  let prevLo = -1, prevHi = -1;
  for (let i = 0, o = BLOCKS_HEADER_SIZE; i < n; i++, o += BLOCKS_ENTRY_SIZE) {
    const lo = dv.getUint32(o, true);
    const hi = dv.getUint32(o + 4, true);
    const chunk = dv.getUint32(o + 8, true);
    if ((lo | hi) === 0) throw new Error(`${BLOCKS_SECTION}: block ${i} has address 0`);
    if (hi < prevHi || (hi === prevHi && lo <= prevLo)) {
      throw new Error(`${BLOCKS_SECTION}: block ${i} at ${hex(joinKey(lo, hi))} is not above ${hex(joinKey(prevLo, prevHi))} (unsorted or duplicate)`);
    }
    if (chunk >= chunks) throw new Error(`${BLOCKS_SECTION}: block ${hex(joinKey(lo, hi))} names chunk ${chunk} of ${chunks}`);
    prevLo = lo;
    prevHi = hi;
  }
  return { chunks, count: n, dv };
}

/**
 * Parses an `x64wasm.blocks` section.
 * @param {ArrayBuffer|Uint8Array} section
 * @returns {{chunks: number, blocks: {addr: bigint, chunk: number, entry: number}[]}}
 */
export function parseBlocks(section) {
  const { chunks, count, dv } = readBlocks(section);
  const blocks = new Array(count);
  for (let i = 0, o = BLOCKS_HEADER_SIZE; i < count; i++, o += BLOCKS_ENTRY_SIZE) {
    blocks[i] = { addr: dv.getBigUint64(o, true), chunk: dv.getUint32(o + 8, true), entry: dv.getUint32(o + 12, true) };
  }
  return { chunks, blocks };
}

function blocksSection(module) {
  const secs = WebAssembly.Module.customSections(module, BLOCKS_SECTION);
  if (secs.length !== 1) throw new Error(`module has ${secs.length} ${BLOCKS_SECTION} sections, expected 1`);
  return secs[0];
}

/** The parsed `x64wasm.blocks` section of a compiled module (exactly one required). */
export function moduleBlocks(module) {
  return parseBlocks(blocksSection(module));
}

/** The number of blocks a compiled module declares (validated). */
export function moduleBlockCount(module) {
  return readBlocks(blocksSection(module)).count;
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
   *   alloc?: (size: bigint, align: bigint) => bigint, fp?: WebAssembly.Exports | null}} opts
   *   `header`: guest address of the 32-byte lookup header (8-byte aligned);
   *   `slotsAddr`/`slots`: the first slot array (16-byte aligned, power-of-two count, at most 2^32);
   *   `alloc`: zeroed guest memory for larger slot arrays (rehash). Without it, a full table is an error.
   *   `fp`: the exports of x64wasm_fp.wasm (loadFp), passed as the `fp` namespace to every module; one per process.
   */
  constructor({ memory, header, slotsAddr, slots, alloc, fp = null }) {
    header = BigInt(header);
    slotsAddr = BigInt(slotsAddr);
    if (header % 8n) throw new Error(`lookup header ${hex(header)} is not 8-byte aligned`);
    if (slotsAddr % 16n) throw new Error(`lookup slots ${hex(slotsAddr)} are not 16-byte aligned`);
    if (!isPow2(slots) || slots > 2 ** 32) throw new Error(`lookup slot count ${slots} is not a power of two up to 2^32`);
    this.memory = memory;
    this.mem = new GuestMemory(memory);
    this.header = header;
    this.alloc = alloc;
    this.fp = fp;
    this.table = new WebAssembly.Table({ element: 'anyfunc', initial: HOST_FUNC_INDEX + 1 });
    this.lookupGlobal = new WebAssembly.Global({ value: 'i64', mutable: false }, header);
    this.depth = new WebAssembly.Global({ value: 'i32', mutable: true }, 0);
    this.used = 0;
    // Mirror of the keys as a BigUint64Array's buffer, written through u32 words.
    this.mirror = new Uint32Array(128);
    this.sorted = true;
    this.lastLo = 0;
    this.lastHi = 0;
    this.modules = [];
    this.setSlots(slotsAddr, slots);
    this.mem.setU64(header + BigInt(LOOKUP_USED), 0n);
    this.mem.setU64(header + 24n, 0n);
  }

  setSlots(addr, count) {
    this.mem.fill(addr, count * LOOKUP_SLOT_SIZE);
    this.slotsAddr = addr;
    this.slotsAt = Number(addr);
    this.slotMask = count - 1;
    this.mem.setU64(this.header + BigInt(LOOKUP_SLOTS), addr);
    this.mem.setU64(this.header + BigInt(LOOKUP_MASK), BigInt(count - 1));
  }

  get slotCount() { return this.slotMask + 1; }
  get mask() { return BigInt(this.slotMask); }

  /** A DataView of the current memory buffer (refreshed after a grow). */
  view() {
    this.mem.refresh();
    return this.mem.dv;
  }

  /** Byte offset of the slot holding (lo, hi), or of the empty slot where its probe ends. */
  probe(dv, lo, hi) {
    const mask = this.slotMask, base = this.slotsAt;
    let i = hashIndex(lo, hi, mask);
    for (;;) {
      const at = base + i * LOOKUP_SLOT_SIZE;
      const klo = dv.getUint32(at, true), khi = dv.getUint32(at + 4, true);
      if ((klo === lo && khi === hi) || (klo | khi) === 0) return at;
      i = (i + 1) & mask;
    }
  }

  /** Puts the host-call function (a JS function, see `hostFunction`) into table slot 0. */
  setHostFunction(fn) {
    this.table.set(HOST_FUNC_INDEX, hostFunction(fn));
  }

  /**
   * Adds another host function (a JS function or a WebAssembly.Suspending)
   * in a new table slot and returns its index. Call it before linking modules.
   */
  addHostFunction(fn) {
    const index = this.table.length;
    this.table.grow(1, hostFunction(fn));
    return index;
  }

  /** Probes for `key`: {func, entry} or null. Follows abi.md's probe exactly. */
  lookup(key) {
    key = BigInt.asUintN(64, BigInt(key));
    if (key === 0n) return null; // never a key (abi.md)
    const [lo, hi] = splitKey(key);
    const dv = this.view();
    const at = this.probe(dv, lo, hi);
    if ((dv.getUint32(at, true) | dv.getUint32(at + 4, true)) === 0) return null;
    return { func: dv.getUint32(at + 8, true), entry: dv.getUint32(at + 12, true) };
  }

  /** Inserts a slot unless `key` is present (first wins). Returns whether it inserted. */
  insert(key, func, entry) {
    const [lo, hi] = splitKey(key);
    const ok = this.insertWords(this.view(), lo, hi, func, entry);
    this.storeUsed();
    return ok;
  }

  /** insert() for a key given as nonzero u32 halves; the caller stores `used` afterwards. */
  insertWords(dv, lo, hi, func, entry) {
    let at = this.probe(dv, lo, hi);
    if ((dv.getUint32(at, true) | dv.getUint32(at + 4, true)) !== 0) return false;
    if (2 * (this.used + 1) > this.slotCount) {
      this.grow(this.used + 1);
      dv = this.view();
      at = this.probe(dv, lo, hi);
    }
    dv.setUint32(at + 8, func, true);
    dv.setUint32(at + 12, entry, true);
    dv.setUint32(at, lo, true);
    dv.setUint32(at + 4, hi, true);
    this.mirrorPush(lo, hi);
    return true;
  }

  mirrorPush(lo, hi) {
    const n = this.used;
    if (2 * n + 2 > this.mirror.length) {
      const bigger = new Uint32Array(this.mirror.length * 2);
      bigger.set(this.mirror);
      this.mirror = bigger;
    }
    this.mirror[2 * n + (LITTLE ? 0 : 1)] = lo;
    this.mirror[2 * n + (LITTLE ? 1 : 0)] = hi;
    if (this.sorted && n > 0 && (hi < this.lastHi || (hi === this.lastHi && lo < this.lastLo))) this.sorted = false;
    this.lastLo = lo;
    this.lastHi = hi;
    this.used = n + 1;
  }

  storeUsed() {
    this.mem.setU64(this.header + BigInt(LOOKUP_USED), BigInt(this.used));
  }

  /** A new slot array with `2 * want <= slots`; reinserts every slot. */
  grow(want) {
    if (!this.alloc) throw new Error(`lookup table full (${this.used} of ${this.slotCount} slots) and no allocator to grow it`);
    let count = this.slotCount;
    while (2 * want > count) count *= 2;
    if (count > 2 ** 32) throw new Error(`lookup table would need ${count} slots`);
    const old = this.mem.read(this.slotsAddr, this.slotCount * LOOKUP_SLOT_SIZE);
    const addr = BigInt(this.alloc(BigInt(count * LOOKUP_SLOT_SIZE), 16n));
    if (addr % 16n) throw new Error(`lookup slots ${hex(addr)} are not 16-byte aligned`);
    this.setSlots(addr, count);
    const odv = new DataView(old.buffer);
    const dv = this.view();
    for (let o = 0; o < old.length; o += LOOKUP_SLOT_SIZE) {
      const lo = odv.getUint32(o, true), hi = odv.getUint32(o + 4, true);
      if ((lo | hi) === 0) continue;
      const at = this.probe(dv, lo, hi);
      dv.setUint32(at + 8, odv.getUint32(o + 8, true), true);
      dv.setUint32(at + 12, odv.getUint32(o + 12, true), true);
      dv.setUint32(at, lo, true);
      dv.setUint32(at + 4, hi, true);
    }
  }

  /** Every key in the lookup table, sorted ascending (the run-time translator's `known`). */
  sortedKeys() {
    const keys = new BigUint64Array(this.mirror.buffer, 0, this.used);
    if (!this.sorted) { keys.sort(); this.sorted = true; }
    // Keep lastLo/lastHi = the largest key, so later sortedness checks stay right.
    if (this.used) { this.lastLo = this.mirror[2 * this.used - 2 + (LITTLE ? 0 : 1)]; this.lastHi = this.mirror[2 * this.used - 2 + (LITTLE ? 1 : 0)]; }
    return keys;
  }

  /**
   * Links a translated module: grows the table, instantiates, inserts its blocks.
   * @param {WebAssembly.Module} module
   * @returns {Promise<{instance: WebAssembly.Instance, run: Function, tableBase: number,
   *   chunks: number, blocks: number, inserted: number}>}
   */
  async link(module) {
    const tableBase = this.reserve(module);
    return this.insertBlocks(module, tableBase, await WebAssembly.instantiate(module, this.imports(tableBase)));
  }

  /** link() for callers that cannot wait (a guest call from a host function). */
  linkSync(module) {
    const tableBase = this.reserve(module);
    return this.insertBlocks(module, tableBase, new WebAssembly.Instance(module, this.imports(tableBase)));
  }

  /** A module that imports `fp` needs the helper module (v4). */
  checkFp(module) {
    if (this.fp) return;
    const names = fpImports(module);
    if (names.length) {
      throw new Error(`the module imports ${names.length} floating-point helper(s) (fp.${names[0]}, ...) but no floating-point helper module is loaded; pass --fp x64wasm_fp.wasm (cargo build -p x64wasm-fp --release --target wasm32-unknown-unknown)`);
    }
  }

  /** Grows the table for the module's chunks; returns its table base. */
  reserve(module) {
    this.checkFp(module);
    const { chunks } = readBlocks(blocksSection(module));
    const tableBase = this.table.length;
    this.table.grow(chunks);
    return tableBase;
  }

  insertBlocks(module, tableBase, instance) {
    const { chunks, count, dv: bdv } = readBlocks(blocksSection(module));
    const run = instance.exports.run;
    if (typeof run !== 'function') throw new Error('module lacks export "run"');
    let inserted = 0;
    let dv = this.view();
    for (let i = 0, o = BLOCKS_HEADER_SIZE; i < count; i++, o += BLOCKS_ENTRY_SIZE) {
      const before = this.slotsAt;
      if (this.insertWords(dv, bdv.getUint32(o, true), bdv.getUint32(o + 4, true), tableBase + bdv.getUint32(o + 8, true), bdv.getUint32(o + 12, true))) inserted++;
      if (this.slotsAt !== before) dv = this.view();
    }
    this.storeUsed();
    const linked = { instance, run, tableBase, chunks, blocks: count, inserted };
    this.modules.push(linked);
    return linked;
  }

  /** The import object for a module whose chunk 0 sits at `tableBase`. */
  imports(tableBase) {
    const imports = {
      [MEMORY_MODULE]: {
        [MEMORY_NAME]: this.memory,
        [TABLE_NAME]: this.table,
        [TABLE_BASE_NAME]: new WebAssembly.Global({ value: 'i32', mutable: false }, tableBase),
        [LOOKUP_NAME]: this.lookupGlobal,
        [DEPTH_NAME]: this.depth,
      },
    };
    if (this.fp) imports[FP_NAMESPACE] = this.fp;
    return imports;
  }
}
