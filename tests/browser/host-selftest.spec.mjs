import { test, expect } from '@playwright/test';

// Runtime host self-test in the page and in a Worker, with a hand-made
// memory64 module. Does not depend on the translator.
test('runCase works in the page and in a Worker (hand-made module)', async ({ page }) => {
  await page.goto('/tests/browser/index.html');
  const r = await page.evaluate(async () => {
    const { runCase } = await import('/runtime/src/run.mjs');
    const { runCaseInWorker } = await import('/runtime/src/browser-host.mjs');
    const { selfTestModule } = await import('/runtime/src/wasm-mini.mjs');
    const mk = (expect) => ({
      name: 'selftest', memoryBytes: '0x10000', state: '0x1000',
      init: { rip: '0x100', rflags: '0x202', regs: { rcx: '0x5' }, mem: [{ addr: '0x2000', hex: 'deadbeef' }] },
      expect,
    });
    const good = { exit: 1, rip: '0x7fffffff0000', regs: { rax: '0x123456789abcdef0', rcx: '0x5' }, mem: [{ addr: '0x2000', hex: 'deadbeef' }] };
    const bad = { ...good, regs: { rax: '0x1' } };
    const strip = (x) => ({ ok: x.ok, diffs: x.diffs, form: x.memoryForm });
    const wasm = selfTestModule();
    return {
      pageGood: strip(await runCase(wasm, mk(good))),
      pageBad: strip(await runCase(wasm, mk(bad))),
      workerGood: strip(await runCaseInWorker(wasm, mk(good))),
      workerBad: strip(await runCaseInWorker(wasm, mk(bad))),
    };
  });
  expect(r.pageGood.diffs).toEqual([]);
  expect(r.pageGood.ok).toBe(true);
  expect(r.workerGood.ok).toBe(true);
  expect(r.pageBad.ok).toBe(false);
  expect(r.pageBad.diffs.join()).toContain('rax');
  expect(r.workerBad.ok).toBe(false);
  console.log('memory64 form in Chromium:', r.pageGood.form);
});
