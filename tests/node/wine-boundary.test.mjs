// M1's boundary gate in Node (docs/design/abi.md, "Wine boundary"): the spike
// in native/spike/boundary/, built by its build.sh into target/wine-spike/.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runBoundary, verify } from '../../native/spike/boundary/driver.mjs';

const dir = new URL('../../target/wine-spike/', import.meta.url);

test('Wine boundary: JSPI through Emscripten wasm64 and translated code', async () => {
  const { default: factory } = await import(new URL('boundary.mjs', dir));
  const r = await runBoundary({
    factory,
    emWasm: readFileSync(new URL('boundary.wasm', dir)),
    guestA: readFileSync(new URL('guest_a.wasm', dir)),
    guestB: readFileSync(new URL('guest_b.wasm', dir)),
  });
  console.log('wine-boundary', JSON.stringify({ timings: r.timings, bench: r.bench, negative: r.negative, stacks: r.stacks, events: r.events },
    (_, v) => (typeof v === 'bigint' ? '0x' + v.toString(16) : v)));
  assert.deepEqual(verify(r), []);
});
