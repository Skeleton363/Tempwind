// M1's boundary gate in a Chromium Worker (docs/design/abi.md, "Wine
// boundary"); the same driver as tests/node/wine-boundary.test.mjs.
import { test, expect } from '@playwright/test';

test('Wine boundary: JSPI through Emscripten wasm64 and translated code, in a Worker', async ({ page }) => {
  await page.goto('/tests/browser/index.html');
  page.on('console', (m) => console.log(`[page] ${m.text()}`));
  const result = await page.evaluate(() => new Promise((resolve) => {
    const worker = new Worker('/tests/browser/wine-boundary-worker.mjs', { type: 'module' });
    worker.onmessage = (e) => { resolve(e.data); worker.terminate(); };
    worker.onerror = (e) => resolve({ error: `worker error: ${e.message}` });
    worker.postMessage('run');
  }));
  console.log('wine-boundary', result.summary ?? result.error);
  expect(result.error).toBeUndefined();
  expect(result.isolated).toBe(true);
  expect(result.problems).toEqual([]);
});
