// Typed access to guest memory (a memory64 WebAssembly.Memory, identity
// mapped: guest virtual address = offset). Addresses are BigInt or Number;
// every access is bounds-checked against the current memory size.

const MAX_SAFE = BigInt(Number.MAX_SAFE_INTEGER);
const hex = (v) => '0x' + BigInt(v).toString(16);

export class GuestMemory {
  /** @param {WebAssembly.Memory} memory */
  constructor(memory) {
    this.memory = memory;
    this.buffer = null;
    this.refresh();
  }

  /** Re-creates the views after the buffer changed (grow detaches it). */
  refresh() {
    const buf = this.memory.buffer;
    if (buf === this.buffer) return;
    this.buffer = buf;
    this.dv = new DataView(buf);
    this.u8 = new Uint8Array(buf);
  }

  get size() { this.refresh(); return this.buffer.byteLength; }

  /** Offset for `len` bytes at `addr`, or throws. */
  at(addr, len) {
    this.refresh();
    const a = BigInt(addr);
    if (a < 0n || a > MAX_SAFE || a + BigInt(len) > BigInt(this.buffer.byteLength)) {
      throw new RangeError(`guest access out of memory: ${hex(a)}+${len}`);
    }
    return Number(a);
  }

  u8get(addr) { return this.u8[this.at(addr, 1)]; }
  u16(addr) { return this.dv.getUint16(this.at(addr, 2), true); }
  u32(addr) { return this.dv.getUint32(this.at(addr, 4), true); }
  u64(addr) { return this.dv.getBigUint64(this.at(addr, 8), true); }
  setU8(addr, v) { this.u8[this.at(addr, 1)] = v; }
  setU16(addr, v) { this.dv.setUint16(this.at(addr, 2), v, true); }
  setU32(addr, v) { this.dv.setUint32(this.at(addr, 4), Number(BigInt.asUintN(32, BigInt(v))), true); }
  setU64(addr, v) { this.dv.setBigUint64(this.at(addr, 8), BigInt.asUintN(64, BigInt(v)), true); }

  /** A copy of `len` bytes at `addr`. */
  read(addr, len) {
    const o = this.at(addr, len);
    return this.u8.slice(o, o + len);
  }

  write(addr, bytes) { this.u8.set(bytes, this.at(addr, bytes.length)); }

  fill(addr, len, value = 0) {
    const o = this.at(addr, len);
    this.u8.fill(value, o, o + len);
  }
}
