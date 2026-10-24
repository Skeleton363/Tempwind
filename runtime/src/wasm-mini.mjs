// Tiny byte-level wasm encoder for hand-made test modules (memory64, threads).
// Not a general assembler: just enough to build the self-test and spike modules.

export const uleb = (n) => {
  n = BigInt(n);
  const out = [];
  do {
    let b = Number(n & 0x7fn);
    n >>= 7n;
    if (n) b |= 0x80;
    out.push(b);
  } while (n);
  return out;
};
export const sleb = (n) => {
  n = BigInt(n);
  const out = [];
  for (;;) {
    const b = Number(n & 0x7fn);
    n >>= 7n;
    if ((n === 0n && !(b & 0x40)) || (n === -1n && (b & 0x40))) { out.push(b); return out; }
    out.push(b | 0x80);
  }
};
const str = (s) => { const b = [...new TextEncoder().encode(s)]; return [...uleb(b.length), ...b]; };
const vec = (items) => [...uleb(items.length), ...items.flat()];
const section = (id, body) => [id, ...uleb(body.length), ...body];

export const T = { i32: 0x7f, i64: 0x7e };

/** Memory limits for a memory64 import. */
function limits64({ min, max, shared }) {
  const flags = 0x04 | (max !== undefined ? 1 : 0) | (shared ? 2 : 0);
  return [flags, ...uleb(min), ...(max !== undefined ? uleb(max) : [])];
}

/**
 * @param {{memory:{min:number,max?:number,shared?:boolean},
 *   funcs:{name:string, params:number[], results:number[], locals?:number[], body:number[]}[]}} m
 * Imports env.memory (memory64) and exports every func by name.
 */
export function buildModule({ memory, funcs }) {
  const types = funcs.map((f) => [0x60, ...vec(f.params.map((p) => [p])), ...vec(f.results.map((p) => [p]))]);
  const bytes = [
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    ...section(1, vec(types)),
    ...section(2, vec([[...str('env'), ...str('memory'), 0x02, ...limits64(memory)]])),
    ...section(3, vec(funcs.map((_, i) => uleb(i)))),
    ...section(7, vec(funcs.map((f, i) => [...str(f.name), 0x00, ...uleb(i)]))),
    ...section(10, vec(funcs.map((f) => {
      const locals = (f.locals ?? []).map((t) => [1, t]);
      const body = [...vec(locals), ...f.body, 0x0b];
      return [...uleb(body.length), ...body];
    }))),
  ];
  return new Uint8Array(bytes);
}

// ABI v3 module shape (docs/design/abi.md): imports env.memory, env.table,
// env.table_base, env.lookup, env.depth; exports run; chunk i at table_base + i;
// an `x64wasm.blocks` section. Chunk bodies are raw instruction bytes.

/** The bytes of an `x64wasm.blocks` section payload. */
export function blocksPayload(chunks, blocks, { version = 1, reserved = 0 } = {}) {
  const out = new Uint8Array(16 + 16 * blocks.length);
  const dv = new DataView(out.buffer);
  dv.setUint32(0, version, true);
  dv.setUint32(4, chunks, true);
  dv.setUint32(8, blocks.length, true);
  dv.setUint32(12, reserved, true);
  blocks.forEach((b, i) => {
    dv.setBigUint64(16 + 16 * i, BigInt(b.addr), true);
    dv.setUint32(24 + 16 * i, b.chunk, true);
    dv.setUint32(28 + 16 * i, b.entry, true);
  });
  return out;
}

/**
 * A module importing env.memory/table/table_base/lookup/depth, exporting
 * run, with `chunks` chunk functions (params: state i64, entry i32, ret i64;
 * result i32) placed at table_base + i.
 * @param {{run: number[], chunks: number[][], blocks: {addr, chunk, entry}[],
 *   locals?: {run?: number[], chunk?: number[]}, blocksSections?: number,
 *   memory?: {min: number, max?: number, shared?: boolean}}} m
 */
export function v3Module({ run, chunks, blocks, locals = {}, blocksSections = 1, memory = { min: 1 } }) {
  const types = [
    [0x60, 1, 0x7e, 1, 0x7f],
    [0x60, 3, 0x7e, 0x7f, 0x7e, 1, 0x7f],
  ];
  const imports = [
    [...str('env'), ...str('memory'), 0x02, ...limits64(memory)],
    [...str('env'), ...str('table'), 0x01, 0x70, 0x00, 0],
    [...str('env'), ...str('table_base'), 0x03, 0x7f, 0],
    [...str('env'), ...str('lookup'), 0x03, 0x7e, 0],
    [...str('env'), ...str('depth'), 0x03, 0x7f, 1],
  ];
  const funcs = [0, ...chunks.map(() => 1)];
  const elem = chunks.length
    ? [section(9, vec([[0x00, 0x23, 0x00, 0x0b, ...vec(chunks.map((_, i) => uleb(1 + i)))]]))]
    : [];
  const body = (code, loc = []) => {
    const b = [...vec(loc.map((t) => [1, t])), ...code, 0x0b];
    return [...uleb(b.length), ...b];
  };
  const custom = [];
  for (let i = 0; i < blocksSections; i++) {
    const payload = [...str('x64wasm.blocks'), ...blocksPayload(chunks.length, blocks)];
    custom.push(section(0, payload));
  }
  return new Uint8Array([
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    ...section(1, vec(types)),
    ...section(2, vec(imports)),
    ...section(3, vec(funcs.map((t) => uleb(t)))),
    ...section(7, vec([[...str('run'), 0x00, 0]])),
    ...elem.flat(),
    ...section(10, vec([body(run, locals.run), ...chunks.map((c) => body(c, locals.chunk))])),
    ...custom.flat(),
  ]);
}

// Opcodes (memory ops take an i64 address under memory64).
export const op = {
  localGet: (i) => [0x20, ...uleb(i)],
  i32Const: (v) => [0x41, ...sleb(v)],
  i64Const: (v) => [0x42, ...sleb(v)],
  i64Store: (offset = 0) => [0x37, 3, ...uleb(offset)],
  i32Store: (offset = 0) => [0x36, 2, ...uleb(offset)],
  i32Load: (offset = 0) => [0x28, 2, ...uleb(offset)],
  i32AtomicLoad: (offset = 0) => [0xfe, 0x10, 2, ...uleb(offset)],
  memoryAtomicWait32: (offset = 0) => [0xfe, 0x01, 2, ...uleb(offset)],
  memoryAtomicNotify: (offset = 0) => [0xfe, 0x00, 2, ...uleb(offset)],
};

/**
 * Self-test module with the ABI v3 shape (no chunks, no blocks): run(state)
 * sets RAX=0x1234_5678_9abc_def0, stores rip=SENTINEL, stores exit=1 and returns 1.
 */
export function selfTestModule({ shared = false } = {}) {
  const body = [
    ...op.localGet(0), ...op.i64Const(0x123456789abcdef0n), ...op.i64Store(0x000),
    ...op.localGet(0), ...op.i64Const(0x7fffffff0000n), ...op.i64Store(0x080),
    ...op.localGet(0), ...op.i32Const(1), ...op.i32Store(0x090),
    ...op.i32Const(1),
  ];
  return v3Module({
    memory: shared ? { min: 1, max: 262144, shared: true } : { min: 1 },
    run: body, chunks: [], blocks: [],
  });
}

/**
 * Threads module over a shared memory64: wait(addr, expected, timeoutNs) -> i32
 * (0 ok, 1 not-equal, 2 timed-out), load(addr) -> i32 atomic load,
 * notify(addr, count) -> i32.
 */
export function threadsModule() {
  return buildModule({
    memory: { min: 1, max: 262144, shared: true },
    funcs: [
      { name: 'wait', params: [T.i64, T.i32, T.i64], results: [T.i32],
        body: [...op.localGet(0), ...op.localGet(1), ...op.localGet(2), ...op.memoryAtomicWait32()] },
      { name: 'load', params: [T.i64], results: [T.i32],
        body: [...op.localGet(0), ...op.i32AtomicLoad()] },
      { name: 'notify', params: [T.i64, T.i32], results: [T.i32],
        body: [...op.localGet(0), ...op.localGet(1), ...op.memoryAtomicNotify()] },
    ],
  });
}
