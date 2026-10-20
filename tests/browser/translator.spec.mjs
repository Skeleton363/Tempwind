import { test, expect } from '@playwright/test';
import { existsSync } from 'node:fs';

const MISSING =
  'target/spike/manifest.json is missing. Create it with: cargo run -p testkit --release -- spike --out target/spike';

test('translator spike cases pass in the page and in a Worker', async ({ page }, testInfo) => {
  expect(existsSync(new URL('../../target/spike/manifest.json', import.meta.url)), MISSING).toBe(true);
  await page.goto('/tests/browser/index.html');
  const res = await page.evaluate(async () => {
    const host = await import('/runtime/src/browser-host.mjs');
    const t0 = performance.now();
    const inPage = await host.runInPage('/target/spike');
    const t1 = performance.now();
    const inWorker = await host.runInWorker('/target/spike');
    const t2 = performance.now();
    return { inPage, inWorker, pageMs: t1 - t0, workerMs: t2 - t1 };
  });
  for (const where of ['inPage', 'inWorker']) {
    expect(res[where].length, `${where}: manifest has cases`).toBeGreaterThan(0);
    const failed = res[where].filter((r) => !r.ok);
    expect(failed, `${where} failures`).toEqual([]);
  }
  const msg = `${res.inPage.length} cases; page ${res.pageMs.toFixed(0)} ms, worker ${res.workerMs.toFixed(0)} ms`;
  console.log(msg);
  testInfo.annotations.push({ type: 'measurement', description: msg });
});
