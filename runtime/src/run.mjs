// runCase: run one spike-manifest case (docs/design/abi.md). Works in Node,
// a browser page and a worker: it uses only standard WebAssembly APIs.

import {
  GPR_NAMES, STATE_GPR, STATE_RIP, STATE_RFLAGS, STATE_EXIT, STATE_SIZE,
  MEMORY_MODULE, MEMORY_NAME, RUN_EXPORT,
} from './abi.mjs';
import { createMemory64, pagesFor } from './memory.mjs';

const hex = (v) => '0x' + BigInt.asUintN(64, v).toString(16);
const big = (s) => BigInt(s);

/** Parse "0000ffff ff7f0000" to bytes. */
export function hexToBytes(h) {
  const s = h.replace(/\s+/g, '');
  if (s.length % 2) throw new Error(`odd-length hex: ${h}`);
  const out = new Uint8Array(s.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(s.substr(i * 2, 2), 16);
  return out;
}
export function bytesToHex(b) {
  return Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
}

/**
 * @param {BufferSource} wasmBytes
 * @param {object} spec a manifest case (name, memoryBytes, state, init, expect)
 * @param {{shared?: boolean, maximumPages?: number}} [opts]
 * @returns {Promise<{ok: boolean, diffs: string[], exit: number, state: object, memoryForm: string}>}
 */
export async function runCase(wasmBytes, spec, opts = {}) {
  const shared = !!opts.shared;
  const { memory, form } = createMemory64({
    initialPages: Math.max(1, pagesFor(big(spec.memoryBytes))),
    maximumPages: shared ? opts.maximumPages ?? 262144 : opts.maximumPages,
    shared,
  });
  const buf = memory.buffer;
  const bytes = new Uint8Array(buf);
  const dv = new DataView(buf);
  const stateAddr = Number(big(spec.state));
  const init = spec.init ?? {};
  const expect = spec.expect ?? {};

  for (const m of init.mem ?? []) bytes.set(hexToBytes(m.hex), Number(big(m.addr)));
  // State block: zeroed (fresh memory), then registers.
  const regs = init.regs ?? {};
  for (const [name, v] of Object.entries(regs)) {
    const i = GPR_NAMES.indexOf(name);
    if (i < 0) throw new Error(`unknown register ${name}`);
    dv.setBigUint64(stateAddr + STATE_GPR + 8 * i, BigInt.asUintN(64, big(v)), true);
  }
  if (init.rip !== undefined) dv.setBigUint64(stateAddr + STATE_RIP, big(init.rip), true);
  if (init.rflags !== undefined) dv.setBigUint64(stateAddr + STATE_RFLAGS, big(init.rflags), true);

  const module = await WebAssembly.compile(wasmBytes);
  const instance = await WebAssembly.instantiate(module, {
    [MEMORY_MODULE]: { [MEMORY_NAME]: memory },
  });
  const run = instance.exports[RUN_EXPORT];
  if (typeof run !== 'function') throw new Error(`module lacks export "${RUN_EXPORT}"`);
  const exit = run(BigInt(stateAddr));

  // Read back from the (possibly regrown) buffer.
  const dv2 = new DataView(memory.buffer);
  const bytes2 = new Uint8Array(memory.buffer);
  const state = { regs: {}, rip: 0n, rflags: 0n, exit: 0 };
  GPR_NAMES.forEach((n, i) => {
    state.regs[n] = dv2.getBigUint64(stateAddr + STATE_GPR + 8 * i, true);
  });
  state.rip = dv2.getBigUint64(stateAddr + STATE_RIP, true);
  state.rflags = dv2.getBigUint64(stateAddr + STATE_RFLAGS, true);
  state.exit = dv2.getUint32(stateAddr + STATE_EXIT, true);

  const diffs = [];
  const exitReturned = Number(exit);
  if (expect.exit !== undefined && exitReturned !== expect.exit) {
    diffs.push(`exit: got ${exitReturned}, expected ${expect.exit}`);
  }
  if (state.exit !== exitReturned) {
    diffs.push(`state.exit (${state.exit}) != returned exit (${exitReturned})`);
  }
  if (expect.rip !== undefined && state.rip !== big(expect.rip)) {
    diffs.push(`rip: got ${hex(state.rip)}, expected ${expect.rip}`);
  }
  for (const [name, v] of Object.entries(expect.regs ?? {})) {
    if (!(name in state.regs)) { diffs.push(`unknown register ${name}`); continue; }
    const want = BigInt.asUintN(64, big(v));
    if (state.regs[name] !== want) {
      diffs.push(`${name}: got ${hex(state.regs[name])}, expected ${hex(want)}`);
    }
  }
  if (expect.rflags !== undefined) {
    const mask = expect.rflagsMask !== undefined ? big(expect.rflagsMask) : 0xffff_ffff_ffff_ffffn;
    if ((state.rflags & mask) !== (big(expect.rflags) & mask)) {
      diffs.push(`rflags: got ${hex(state.rflags & mask)}, expected ${hex(big(expect.rflags) & mask)} (mask ${hex(mask)})`);
    }
  }
  for (const m of expect.mem ?? []) {
    const want = hexToBytes(m.hex);
    const at = Number(big(m.addr));
    const got = bytes2.subarray(at, at + want.length);
    const bad = want.findIndex((b, i) => got[i] !== b);
    if (bad >= 0) {
      diffs.push(`mem@${hex(BigInt(at))}: byte +${bad} got ${bytesToHex(got.subarray(0, 32))}, expected ${bytesToHex(want.subarray(0, 32))}`);
    }
  }
  void bytes; void dv;
  return { ok: diffs.length === 0, diffs, exit: exitReturned, state, memoryForm: form };
}

/**
 * Run all cases of a manifest. `loadWasm(name) -> Promise<Uint8Array>`.
 * @returns {Promise<{name: string, ok: boolean, diffs: string[], error?: string}[]>}
 */
export async function runManifest(manifest, loadWasm, opts = {}) {
  const results = [];
  for (const c of manifest.cases) {
    try {
      const r = await runCase(await loadWasm(c.wasm), c, opts);
      results.push({ name: c.name, ok: r.ok, diffs: r.diffs });
    } catch (e) {
      results.push({ name: c.name, ok: false, diffs: [], error: String(e?.stack ?? e) });
    }
  }
  return results;
}
