// The run-time translator x64wasm_rt.wasm (docs/design/abi.md, "Run-time
// translator"): marshals a request into its memory and reads the result.

import {
  RT_REQUEST_SIZE, RT_REQ_CODE_BASE, RT_REQ_CODE_PTR, RT_REQ_CODE_LEN, RT_REQ_ENTRY,
  RT_REQ_KNOWN_PTR, RT_REQ_KNOWN_LEN, RT_REQ_FLAGS, RT_RES_STATUS, RT_RES_WASM_PTR,
  RT_RES_WASM_LEN, RT_RES_MSG_PTR, RT_RES_MSG_LEN, RT_RESULT_SIZE,
} from './abi.mjs';

const EXPORTS = ['memory', 'rt_alloc', 'rt_free', 'rt_translate'];
// Buffers are never requested empty: a zero-length request still gets a valid pointer.
const MIN_ALLOC = 16;

export class RuntimeTranslator {
  /** @param {{memory: WebAssembly.Memory, rt_alloc: Function, rt_free: Function, rt_translate: Function}} exports */
  constructor(exports) {
    for (const name of EXPORTS) {
      if (!(name in exports)) throw new Error(`run-time translator lacks export "${name}"`);
    }
    this.x = exports;
  }

  /** Instantiates x64wasm_rt.wasm (bytes or a compiled module); it must import nothing. */
  static async load(source) {
    const module = source instanceof WebAssembly.Module ? source : await WebAssembly.compile(source);
    const imports = WebAssembly.Module.imports(module);
    if (imports.length) {
      throw new Error(`run-time translator imports ${imports.map((i) => `${i.module}.${i.name}`).join(', ')}; it must import nothing`);
    }
    const instance = await WebAssembly.instantiate(module, {});
    return new RuntimeTranslator(instance.exports);
  }

  alloc(size) {
    const n = Math.max(size, MIN_ALLOC);
    const ptr = this.x.rt_alloc(n) >>> 0;
    if (ptr === 0 || ptr % 16) throw new Error(`run-time translator: rt_alloc(${n}) returned ${ptr}`);
    return { ptr, size: n };
  }

  /**
   * Translates what is reachable from `entry` in [codeBase, codeBase + code.length).
   * @param {{codeBase: bigint, code: Uint8Array, entry: bigint, known: BigUint64Array}} req
   * @returns {Uint8Array} a v3 module (a copy)
   */
  translate({ codeBase, code, entry, known }) {
    const bufs = [];
    try {
      const codeBuf = this.alloc(code.length); bufs.push(codeBuf);
      const knownBuf = this.alloc(known.length * 8); bufs.push(knownBuf);
      const reqBuf = this.alloc(RT_REQUEST_SIZE); bufs.push(reqBuf);
      // rt_alloc may grow the memory: take the views afterwards.
      let buf = this.x.memory.buffer;
      new Uint8Array(buf).set(code, codeBuf.ptr);
      const kdv = new DataView(buf, knownBuf.ptr, known.length * 8);
      known.forEach((k, i) => kdv.setBigUint64(8 * i, k, true));
      const dv = new DataView(buf, reqBuf.ptr, RT_REQUEST_SIZE);
      dv.setBigUint64(RT_REQ_CODE_BASE, BigInt(codeBase), true);
      dv.setUint32(RT_REQ_CODE_PTR, codeBuf.ptr, true);
      dv.setUint32(RT_REQ_CODE_LEN, code.length, true);
      dv.setBigUint64(RT_REQ_ENTRY, BigInt(entry), true);
      dv.setUint32(RT_REQ_KNOWN_PTR, knownBuf.ptr, true);
      dv.setUint32(RT_REQ_KNOWN_LEN, known.length, true);
      dv.setUint32(RT_REQ_FLAGS, 0, true);
      dv.setUint32(RT_REQ_FLAGS + 4, 0, true);

      const res = this.x.rt_translate(reqBuf.ptr) >>> 0;
      buf = this.x.memory.buffer;
      if (res === 0 || res + RT_RESULT_SIZE > buf.byteLength) throw new Error(`run-time translator: bad result pointer ${res}`);
      const r = new DataView(buf, res, RT_RESULT_SIZE);
      const status = r.getUint32(RT_RES_STATUS, true);
      const slice = (ptrOff, lenOff) => {
        const ptr = r.getUint32(ptrOff, true);
        const len = r.getUint32(lenOff, true);
        if (ptr + len > buf.byteLength) throw new Error(`run-time translator: result range ${ptr}+${len} is outside its memory`);
        return new Uint8Array(buf, ptr, len).slice();
      };
      if (status === 0) return slice(RT_RES_WASM_PTR, RT_RES_WASM_LEN);
      if (status === 1) throw new Error(`run-time translator: ${new TextDecoder().decode(slice(RT_RES_MSG_PTR, RT_RES_MSG_LEN))}`);
      throw new Error(`run-time translator: unknown status ${status}`);
    } finally {
      for (const b of bufs) this.x.rt_free(b.ptr, b.size);
    }
  }
}
