// memory64 creation with feature detection of the JS API form.
// The final spec uses {address:'i64'} with BigInt sizes; older engines used
// {index:'i64'} and/or Number sizes. We try each form and report which worked.

import { PAGE_SIZE } from './abi.mjs';

// Old V8 (Node 22) ignores the unknown key {address:'i64'} and silently makes a
// memory32. So every candidate is verified by importing it into a tiny
// memory64 module (import-only, min 0, same shared-ness).
function isMemory64(memory, shared) {
  // magic, version, import section: env.memory, kind 2 (memory), flags, min 0, [max 262144]
  const flags = 0x04 | (shared ? 3 : 0);
  const lim = shared ? [flags, 0, 0x80, 0x80, 0x10] : [flags, 0];
  const imp = [1, 3, 0x65, 0x6e, 0x76, 6, 0x6d, 0x65, 0x6d, 0x6f, 0x72, 0x79, 2, ...lim];
  const bytes = new Uint8Array([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0, 2, imp.length, ...imp]);
  try {
    new WebAssembly.Instance(new WebAssembly.Module(bytes), { env: { memory } });
    return true;
  } catch {
    return false;
  }
}

const FORMS = [
  { key: 'address', big: true },
  { key: 'address', big: false },
  { key: 'index', big: true },
  { key: 'index', big: false },
];

/**
 * @param {{initialPages: number, maximumPages?: number, shared?: boolean}} opts
 * @returns {{memory: WebAssembly.Memory, form: string}} form e.g. "address:i64/BigInt"
 */
export function createMemory64({ initialPages, maximumPages, shared = false }) {
  if (shared && maximumPages === undefined) {
    throw new Error('shared memory requires maximumPages');
  }
  const errors = [];
  for (const { key, big } of FORMS) {
    const n = (v) => (big ? BigInt(v) : v);
    const desc = { [key]: 'i64', initial: n(initialPages) };
    if (maximumPages !== undefined) desc.maximum = n(maximumPages);
    if (shared) desc.shared = true;
    try {
      const memory = new WebAssembly.Memory(desc);
      if (!isMemory64(memory, shared)) throw new Error('constructed a memory32 (key ignored)');
      return { memory, form: `${key}:i64/${big ? 'BigInt' : 'Number'}` };
    } catch (e) {
      errors.push(`${key}/${big ? 'BigInt' : 'Number'}: ${e.message}`);
    }
  }
  throw new Error(`memory64 not supported: ${errors.join(' | ')}`);
}

/** Pages needed to hold `bytes`. */
export const pagesFor = (bytes) => Math.ceil(Number(bytes) / PAGE_SIZE);

/** Detect which forms the engine accepts (for status reporting). */
export function probeMemory64Forms() {
  const out = {};
  for (const { key, big } of FORMS) {
    const n = (v) => (big ? BigInt(v) : v);
    try {
      const memory = new WebAssembly.Memory({ [key]: 'i64', initial: n(1) });
      if (!isMemory64(memory, false)) throw new Error('memory32');
      out[`${key}/${big ? 'BigInt' : 'Number'}`] = true;
    } catch (e) {
      out[`${key}/${big ? 'BigInt' : 'Number'}`] = false;
    }
  }
  return out;
}
