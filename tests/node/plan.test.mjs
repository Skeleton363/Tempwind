import test from 'node:test';
import assert from 'node:assert/strict';
import { parsePlan, checkPlan, addressMap, sha256 } from '../../runtime/src/plan.mjs';
import { fakeExe, fakePlan, BASE, TLS_PLAN } from './pe-fixture.mjs';

const exe = fakeExe();
const check = async (edit, bytes = exe) => checkPlan(parsePlan(await fakePlan(exe, edit)), bytes);

test('sha256 is the SHA-256 of the bytes', async () => {
  assert.equal(await sha256(new TextEncoder().encode('abc')), 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
});

test('a valid plan gives the Phase 1 address map', async () => {
  const map = await check();
  assert.equal(map.imageBase, BASE);
  assert.equal(map.stackTop, 0x7ff0_0000n);
  assert.equal(map.stackBottom, 0x7ff0_0000n - 0x20_0000n);
  assert.deepEqual(map.guestHeap, { base: 0x1000_0000n, limit: map.stackBottom });
  assert.deepEqual(map.runtimeHeap, { base: 0x8010_0000n, limit: 0x1_0000_0000n });
  assert.equal(map.memoryBytes, 0x1_4001_0000n, 'image end rounded up to 64 KiB');
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

test('stack reserve rounds up to 64 KiB', () => {
  const m = addressMap({ imageBase: BASE, sizeOfImage: 0x5000n, stackReserve: 0x1_2345n });
  assert.equal(m.stackBottom, 0x7ff0_0000n - 0x2_0000n);
});

test('an image between the heap start and the stack bounds the guest heap', () => {
  const m = addressMap({ imageBase: 0x2000_0000n, sizeOfImage: 0x5000n, stackReserve: 0x10_0000n });
  assert.equal(m.guestHeap.limit, 0x2000_0000n);
  assert.equal(m.memoryBytes, 0x1_0000_0000n, 'the host region always ends at 4 GiB');
  const low = addressMap({ imageBase: 0x40_0000n, sizeOfImage: 0x5000n, stackReserve: 0x10_0000n });
  assert.equal(low.guestHeap.limit, low.stackBottom);
});

for (const [label, edit, re] of [
  ['sha256', (j) => { j.sha256 = '0'.repeat(64); }, /does not match the file/],
  ['machine', (j) => { j.machine = '0x14c'; }, /not x86-64/],
  ['version', (j) => { j.version = 2; }, /version 2/],
  ['u64 field', (j) => { j.imageBase = 0x140000000; }, /imageBase must be/],
  ['NULL region', (j) => { j.imageBase = '0x0'; }, /NULL region/],
  ['heap start', (j) => { j.imageBase = '0xfff0000'; j.sizeOfImage = '0x20000'; }, /guest heap's start/],
  ['stack', (j) => { j.imageBase = '0x7fe00000'; }, /the stack/],
  ['PEB', (j) => { j.imageBase = '0x7ffd0000'; }, /the PEB/],
  ['KUSER', (j) => { j.imageBase = '0x7ffe0000'; }, /KUSER_SHARED_DATA/],
  ['host region', (j) => { j.imageBase = '0x80000000'; }, /the host region/],
  ['16 GiB', (j) => { j.imageBase = '0x3ffff0000'; j.sizeOfImage = '0x20000'; }, /above 16 GiB/],
  ['alignment', (j) => { j.imageBase = '0x140001000'; }, /64 KiB aligned/],
  ['stack size', (j) => { j.stackReserve = '0x70000000'; }, /does not fit/],
  ['section in image', (j) => { j.sections[1].virtualSize = '0x4000'; }, /section .data .* outside the image/],
  ['section in file', (j) => { j.sections[3].fileOffset = '0xbf0'; }, /file range .* outside the file/],
  ['entry point', (j) => { j.entryPoint = '0x140002000'; }, /entry point .* not in an executable section/],
  ['IAT', (j) => { j.imports[0].functions[0].iat = '0x140003004'; }, /IAT slot/],
  ['import name and ordinal', (j) => { j.imports[0].functions[0].ordinal = 3; }, /exactly one of/],
  ['TLS template', (j) => { j.tls = { ...TLS_PLAN, endOfRawData: '0x150000000' }; }, /TLS template/],
  ['TLS callback', (j) => { j.tls = { ...TLS_PLAN, callbacks: ['0x140002000'] }; }, /TLS callback/],
  ['thunk count', (j) => {
    j.imports[0].functions = Array.from({ length: 4097 }, () => ({ name: 'X', ordinal: null, iat: '0x140003000' }));
  }, /more than the 4096 thunks/],
]) {
  test(`checkPlan rejects a bad plan: ${label}`, async () => {
    await assert.rejects(check(edit), re);
  });
}
