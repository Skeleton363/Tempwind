// The msvcrt heap of a Phase 1 process (runtime.md, "msvcrt (M3)"): a
// deterministic first-fit allocator over arenas taken from the guest heap,
// 16-byte aligned. Block metadata lives in JS, so the guest sees only its
// own bytes; an invalid free or realloc is an error.

const ALIGN = 16n;
/** Arenas are taken from the guest heap in pieces of at least this size. */
export const ARENA_SIZE = 0x10_0000n;
const roundUp = (n) => (n + ALIGN - 1n) / ALIGN * ALIGN;
const hex = (v) => '0x' + BigInt(v).toString(16);

export class HeapError extends Error {}

export class CrtHeap {
  /** @param {(size: bigint) => bigint} grow allocates a fresh, zeroed, 16-byte-aligned arena */
  constructor(grow) {
    this.grow = grow;
    /** Allocated blocks: address -> rounded size. */
    this.blocks = new Map();
    /** Free blocks, sorted by address and coalesced: {addr, size}. */
    this.free = [];
    this.arena = { next: 0n, end: 0n };
  }

  /** Bytes usable at `addr` (the requested size rounded up to 16), or undefined. */
  sizeOf(addr) { return this.blocks.get(addr); }

  /** A block of at least `size` bytes (size 0 gets a unique block). */
  alloc(size) {
    const need = roundUp(size > 0n ? size : 1n);
    const i = this.free.findIndex((b) => b.size >= need);
    let addr;
    if (i >= 0) {
      const b = this.free[i];
      addr = b.addr;
      if (b.size === need) this.free.splice(i, 1);
      else { b.addr += need; b.size -= need; }
    } else {
      if (this.arena.end - this.arena.next < need) {
        if (this.arena.end > this.arena.next) this.release(this.arena.next, this.arena.end - this.arena.next);
        const size = need > ARENA_SIZE ? need : ARENA_SIZE;
        const base = this.grow(size);
        this.arena = { next: base, end: base + size };
      }
      addr = this.arena.next;
      this.arena.next += need;
    }
    this.blocks.set(addr, need);
    return addr;
  }

  /** Frees a block that alloc returned. */
  release(addr, size) {
    let i = this.free.findIndex((b) => b.addr > addr);
    if (i < 0) i = this.free.length;
    this.free.splice(i, 0, { addr, size });
    const next = this.free[i + 1];
    if (next && addr + size === next.addr) { this.free[i].size += next.size; this.free.splice(i + 1, 1); }
    const prev = this.free[i - 1];
    if (prev && prev.addr + prev.size === addr) { prev.size += this.free[i].size; this.free.splice(i, 1); }
  }

  freeBlock(addr) {
    const size = this.blocks.get(addr);
    if (size === undefined) throw new HeapError(`free of ${hex(addr)}, which is not an allocated block`);
    this.blocks.delete(addr);
    this.release(addr, size);
  }

  /**
   * Resizes a block in place when it can (shrinking, or growing into the
   * free block or arena space right after it); returns true if it did.
   */
  resizeInPlace(addr, size) {
    const old = this.blocks.get(addr);
    if (old === undefined) throw new HeapError(`realloc of ${hex(addr)}, which is not an allocated block`);
    const need = roundUp(size > 0n ? size : 1n);
    if (need <= old) {
      if (need < old) { this.blocks.set(addr, need); this.release(addr + need, old - need); }
      return true;
    }
    const end = addr + old;
    const extra = need - old;
    const i = this.free.findIndex((b) => b.addr === end);
    if (i >= 0 && this.free[i].size >= extra) {
      const b = this.free[i];
      if (b.size === extra) this.free.splice(i, 1);
      else { b.addr += extra; b.size -= extra; }
      this.blocks.set(addr, need);
      return true;
    }
    if (this.arena.next === end && this.arena.end - end >= extra) {
      this.arena.next += extra;
      this.blocks.set(addr, need);
      return true;
    }
    return false;
  }
}
