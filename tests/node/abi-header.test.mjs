// The C header and the JS constants agree: both come from crates/x64wasm's
// abi.rs (`abi.md`, "The C header"), the header through `x64wasm-cli
// gen-header` and abi.mjs by hand. For every numeric `#define TW_<NAME>` in
// native/wine/include/tempwind/abi.h whose <NAME> runtime/src/abi.mjs
// exports, the values must be equal, whether the export is a Number or a
// BigInt.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import * as abi from '../../runtime/src/abi.mjs';

const HEADER = new URL('../../native/wine/include/tempwind/abi.h', import.meta.url);

/// The numeric object-like macros of the header: TW_NAME -> BigInt. A value
/// is a decimal or hex integer, optionally inside UINT32_C() or UINT64_C().
function numericDefines(text) {
  const out = new Map();
  const define = /^#define\s+TW_([A-Z0-9_]+)\s+(.+?)\s*$/gm;
  const number = /^(?:UINT(?:32|64)_C\((0x[0-9a-fA-F]+|\d+)\)|(0x[0-9a-fA-F]+|\d+))$/;
  for (const [, name, value] of text.matchAll(define)) {
    const m = number.exec(value);
    if (m) out.set(name, BigInt(m[1] ?? m[2]));
  }
  return out;
}

test('abi.h and abi.mjs give the same values', () => {
  const defines = numericDefines(readFileSync(HEADER, 'utf8'));
  // Function-like macros (TW_STATE_GPR(i)) and initializers (TW_CPUID_LEAVES)
  // are not numbers; plain constants are.
  assert.ok(!defines.has('STATE_GPR') && !defines.has('CPUID_LEAVES'));
  assert.equal(defines.get('LOOKUP_TOMBSTONE'), 0xdead_0000_0000_0001n);
  let compared = 0;
  for (const [name, value] of defines) {
    if (!(name in abi)) continue;
    const js = abi[name];
    assert.ok(typeof js === 'number' || typeof js === 'bigint',
      `abi.mjs exports ${name} as a ${typeof js}, the header as a number`);
    if (typeof js === 'number') assert.ok(Number.isSafeInteger(js), `${name} = ${js}`);
    assert.equal(BigInt(js), value, `TW_${name}`);
    compared++;
  }
  // Most of the header's constants are the runtime's too.
  assert.ok(compared >= defines.size / 2, `only ${compared} of ${defines.size} compared`);
});

test('the parser reads the header forms', () => {
  const d = numericDefines([
    '#define TW_A UINT64_C(0x10)',
    '#define TW_B UINT32_C(0x1f80)',
    '#define TW_C 12',
    '#define TW_D 0x27f',
    '#define TW_E(i) (UINT64_C(0x0) + 8 * (uint64_t)(i))',
    '#define TW_F { \\',
  ].join('\n'));
  assert.deepEqual([...d], [['A', 0x10n], ['B', 0x1f80n], ['C', 12n], ['D', 0x27fn]]);
});
