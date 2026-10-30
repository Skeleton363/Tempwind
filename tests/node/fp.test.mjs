// The wasm build of the soft float (x64wasm_fp.wasm, abi.md "Floating-point
// helper module") against vectors computed by the native Rust API, which
// crates/x64wasm-fp's differential tests check against the CPU. Each case
// calls the export, then status(), and compares both.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';

const root = new URL('../../', import.meta.url);
const WASM = new URL('target/wasm32-unknown-unknown/release/x64wasm_fp.wasm', root);
const VECTORS = new URL('target/fp/vectors.json', root);
const MAKE = {
  [WASM]: 'cargo build -p x64wasm-fp --release --target wasm32-unknown-unknown',
  [VECTORS]: 'cargo run -p x64wasm-fp --release --example vectors -- --out target/fp/vectors.json',
};

// Parameter and result types of every export ('i' = i32, 'I' = i64), as in
// the abi.md tables.
const SIG = {
  f32_add: ['iii', 'i'], f32_sub: ['iii', 'i'], f32_mul: ['iii', 'i'],
  f32_div: ['iii', 'i'], f32_min: ['iii', 'i'], f32_max: ['iii', 'i'],
  f32_sqrt: ['ii', 'i'],
  f64_add: ['IIi', 'I'], f64_sub: ['IIi', 'I'], f64_mul: ['IIi', 'I'],
  f64_div: ['IIi', 'I'], f64_min: ['IIi', 'I'], f64_max: ['IIi', 'I'],
  f64_sqrt: ['Ii', 'I'],
  f32_cmp: ['iiii', 'i'], f64_cmp: ['IIii', 'i'],
  f32_comi: ['iiii', 'i'], f64_comi: ['IIii', 'i'],
  f32_to_f64: ['ii', 'I'], f64_to_f32: ['Ii', 'i'],
  i32_to_f32: ['ii', 'i'], i64_to_f32: ['Ii', 'i'], i64_to_f64: ['Ii', 'I'],
  f32_to_i32: ['iii', 'i'], f64_to_i32: ['Iii', 'i'],
  f32_to_i64: ['iii', 'I'], f64_to_i64: ['Iii', 'I'],
  f80_add: ['IiIiii', 'I'], f80_sub: ['IiIiii', 'I'], f80_subr: ['IiIiii', 'I'],
  f80_mul: ['IiIiii', 'I'], f80_div: ['IiIiii', 'I'], f80_divr: ['IiIiii', 'I'],
  f80_sqrt: ['Iii', 'I'], f80_round: ['Iii', 'I'],
  f80_scale: ['IiIii', 'I'], f80_prem: ['IiIii', 'I'], f80_prem1: ['IiIii', 'I'],
  f80_load: ['Iii', 'I'], f80_store: ['Iiii', 'I'],
  f80_cmp: ['IiIiiii', 'i'],
};

function need(url) {
  if (!existsSync(url)) {
    assert.fail(`${url.pathname} is missing; make it with: ${MAKE[url]}`);
  }
  return readFileSync(url);
}

const arg = (t, hex) => (t === 'I' ? BigInt.asIntN(64, BigInt(hex)) : Number(BigInt.asIntN(32, BigInt(hex))));
const bits = (t, v) => (t === 'I' ? BigInt.asUintN(64, v) : BigInt(v >>> 0));

test('x64wasm_fp.wasm exports exactly the abi.md functions', async () => {
  const { instance } = await WebAssembly.instantiate(need(WASM));
  const fns = Object.keys(instance.exports).filter((k) => typeof instance.exports[k] === 'function');
  assert.deepEqual(fns.sort(), [...Object.keys(SIG), 'status'].sort());
  assert.ok(instance.exports.memory instanceof WebAssembly.Memory);
  for (const [name, [params]] of Object.entries(SIG)) {
    assert.equal(instance.exports[name].length, params.length, `${name} arity`);
  }
});

test('x64wasm_fp.wasm matches the native vectors', async () => {
  const { instance } = await WebAssembly.instantiate(need(WASM));
  const x = instance.exports;
  const vectors = JSON.parse(need(VECTORS).toString());
  assert.ok(vectors.length > 0);
  const seen = new Set();
  const bad = [];
  for (const v of vectors) {
    const [params, ret] = SIG[v.fn];
    assert.ok(params, `unknown function ${v.fn}`);
    seen.add(v.fn);
    const r = x[v.fn](...v.args.map((a, i) => arg(params[i], a)));
    const s = x.status();
    const got = { result: bits(ret, r), status: BigInt(s >>> 0) };
    const want = { result: BigInt(v.result), status: BigInt(v.status) };
    if (got.result !== want.result || got.status !== want.status) {
      if (bad.push(`${v.fn}(${v.args.join(', ')}): wasm ${got.result.toString(16)}/${got.status.toString(16)}, ` +
        `native ${want.result.toString(16)}/${want.status.toString(16)}`) >= 10) break;
    }
  }
  assert.deepEqual(bad, []);
  assert.deepEqual([...seen].sort(), Object.keys(SIG).sort(), 'every export has vectors');
});
