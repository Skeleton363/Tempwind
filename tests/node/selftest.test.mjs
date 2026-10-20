import test from 'node:test';
import assert from 'node:assert/strict';
import { runCase } from '../../runtime/src/run.mjs';
import { selfTestModule } from '../../runtime/src/wasm-mini.mjs';
import { createMemory64, probeMemory64Forms } from '../../runtime/src/memory.mjs';

const mkSpec = (expect) => ({
  name: 'selftest', memoryBytes: '0x10000', state: '0x1000',
  init: { rip: '0x100', rflags: '0x202', regs: { rcx: '0x5' }, mem: [{ addr: '0x2000', hex: 'de ad be ef' }] },
  expect,
});
const good = {
  exit: 1, rip: '0x7fffffff0000',
  regs: { rax: '0x123456789abcdef0', rcx: '0x5' },
  mem: [{ addr: '0x2000', hex: 'deadbeef' }],
};

test('memory64 API form is detected', () => {
  const forms = probeMemory64Forms();
  console.log(`node ${process.versions.node} memory64 forms:`, JSON.stringify(forms));
  const { form } = createMemory64({ initialPages: 1 });
  console.log('createMemory64 used', form);
  assert.ok(Object.values(forms).some(Boolean));
});

test('runCase passes on matching expectations', async () => {
  const r = await runCase(selfTestModule(), mkSpec(good));
  assert.deepEqual(r.diffs, []);
  assert.equal(r.ok, true);
  assert.equal(r.exit, 1);
  assert.equal(r.state.regs.rax, 0x123456789abcdef0n);
});

test('runCase passes with shared memory', async () => {
  const r = await runCase(selfTestModule(), mkSpec(good), { shared: true });
  assert.equal(r.ok, true, r.diffs.join('\n'));
});

for (const [label, expect, needle] of [
  ['register', { ...good, regs: { rax: '0x1' } }, 'rax'],
  ['exit', { ...good, exit: 2 }, 'exit'],
  ['rip', { ...good, rip: '0x42' }, 'rip'],
  ['memory', { ...good, mem: [{ addr: '0x2000', hex: 'deadbeee' }] }, 'mem@'],
  ['rflags', { ...good, rflags: '0x1', rflagsMask: '0x1' }, 'rflags'],
]) {
  test(`runCase reports a ${label} mismatch`, async () => {
    const r = await runCase(selfTestModule(), mkSpec(expect));
    assert.equal(r.ok, false);
    assert.ok(r.diffs.some((d) => d.includes(needle)), r.diffs.join('\n'));
  });
}

test('rflags check honours mask', async () => {
  // Module preserves entry rflags (0x202): bit 1 set, bit 0 clear.
  const ok = await runCase(selfTestModule(), mkSpec({ ...good, rflags: '0x2', rflagsMask: '0x3' }));
  assert.equal(ok.ok, true, ok.diffs.join('\n'));
  const bad = await runCase(selfTestModule(), mkSpec({ ...good, rflags: '0x3', rflagsMask: '0x3' }));
  assert.equal(bad.ok, false);
});
