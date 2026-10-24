// Hand-built modules with the v3 import shape (docs/design/abi.md), for
// linker and process tests that run before the v3 emitter exists. Not a
// translator: chunk bodies are raw wasm instruction bytes given by the test.
import { uleb } from '../../runtime/src/wasm-mini.mjs';

const str = (s) => { const b = [...new TextEncoder().encode(s)]; return [...uleb(b.length), ...b]; };
const vec = (items) => [...uleb(items.length), ...items.flat()];
const section = (id, body) => [id, ...uleb(body.length), ...body];

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
 *   locals?: {run?: number[], chunk?: number[]}, blocksSections?: number}} m
 */
export function v3Module({ run, chunks, blocks, locals = {}, blocksSections = 1 }) {
  const types = [
    [0x60, 1, 0x7e, 1, 0x7f],
    [0x60, 3, 0x7e, 0x7f, 0x7e, 1, 0x7f],
  ];
  const imports = [
    [...str('env'), ...str('memory'), 0x02, 0x04, 1],
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
