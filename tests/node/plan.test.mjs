import test from 'node:test';
import assert from 'node:assert/strict';
import { parsePlan, sha256 } from '../../runtime/src/plan.mjs';
import { requestFlags } from '../../runtime/src/rt.mjs';
import * as abi from '../../runtime/src/abi.mjs';
import { fakeExe, fakePlan, BASE, TEXT, TLSSEC, TLS_PLAN } from './pe-fixture.mjs';

const exe = fakeExe();

test('sha256 is the SHA-256 of the bytes', async () => {
  assert.equal(await sha256(new TextEncoder().encode('abc')), 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
});

test('parsePlan converts u64 strings and names ordinal imports', async () => {
  const p = parsePlan(await fakePlan(exe));
  assert.equal(p.entryPoint, BASE + 0x1000n);
  assert.equal(p.imports[0].functions[3].name, '#5');
  assert.equal(p.sections[0].executable, true);
  assert.equal(p.sections[1].copySize, 0x200n);
  assert.equal(p.sections[1].mappedSize, 0x300n);
});

test('a zero virtualSize maps the raw size', async () => {
  const p = parsePlan(await fakePlan(exe, (j) => { j.sections[0].virtualSize = '0x0'; }));
  assert.equal(p.sections[0].mappedSize, 0x200n);
});

test('parsePlan converts the TLS directory', async () => {
  const p = parsePlan(await fakePlan(exe, (j) => { j.tls = TLS_PLAN; }));
  assert.deepEqual(p.tls, {
    startOfRawData: TLSSEC, endOfRawData: TLSSEC + 0x10n, addressOfIndex: TLSSEC + 0x20n,
    sizeOfZeroFill: 0x20n, callbacks: [TEXT + 0x80n],
  });
});

for (const [label, edit, re] of [
  ['version', (j) => { j.version = 2; }, /version 2/],
  ['u64 field', (j) => { j.imageBase = 0x140000000; }, /imageBase must be/],
  ['import name and ordinal', (j) => { j.imports[0].functions[0].ordinal = 3; }, /exactly one of/],
]) {
  test(`parsePlan rejects a bad plan: ${label}`, async () => {
    await assert.rejects(async () => parsePlan(await fakePlan(exe, edit)), re);
  });
}

test('options: sharedMemory reads as given, and false in a plan without it', async () => {
  const read = async (options) => parsePlan(await fakePlan(exe, (j) => { if (options) j.options = options; })).options;
  assert.equal((await read(undefined)).sharedMemory, false, 'no options');
  assert.equal((await read({ nullCheck: true })).sharedMemory, false, 'an older plan');
  assert.equal((await read({ sharedMemory: true })).sharedMemory, true);
  await assert.rejects(read({ sharedMemory: 1 }), /sharedMemory must be a boolean/);
});

test('the plan options map to the run-time translator\'s request flags', () => {
  const none = { nullCheck: false, storeCheck: false, counters: false, syncAll: false, structured: true, sharedMemory: false };
  assert.equal(requestFlags(none), 0);
  const one = [
    ['nullCheck', true, abi.RT_FLAG_NULL_CHECK],
    ['storeCheck', true, abi.RT_FLAG_STORE_CHECK],
    ['counters', true, abi.RT_FLAG_COUNTERS],
    ['syncAll', true, abi.RT_FLAG_SYNC_ALL],
    ['structured', false, abi.RT_FLAG_REFERENCE_LAYOUT],
    ['sharedMemory', true, abi.RT_FLAG_SHARED_MEMORY],
  ];
  for (const [key, value, bit] of one) assert.equal(requestFlags({ ...none, [key]: value }), bit, key);
});
