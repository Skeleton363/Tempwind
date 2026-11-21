// hello.exe on Wine in a Chromium Worker (docs/design/wine-unix.md, section 10,
// "Browser (M3)"). The assertions are tests/wine/programs.test.mjs's for hello.
// Needs the Wine inputs (TW_WINE_DIR, TW_WINE_UNIX_DIR, TW_RT_WASM, TW_FP_WASM
// or the repository's target/): tests/wine/prepare.sh, then
// node tests/wine/catalog.mjs --all (it writes prefix.json). Run it with
// `npm run test:browser:wine`.
import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const PE = '/wine/lib/wine/x86_64-windows/';
const AOT = ['ntdll.dll', 'kernel32.dll', 'kernelbase.dll', 'msvcrt.dll'].map((n) => PE + n);

test('hello.exe on Wine in a Worker', async ({ page }) => {
  await page.goto('/tests/browser/index.html');
  page.on('console', (m) => console.log(`[page] ${m.text()}`));
  const r = await page.evaluate(() => new Promise((resolve) => {
    const worker = new Worker('/tests/browser/wine-hello-worker.mjs', { type: 'module' });
    worker.onmessage = (e) => { resolve(e.data); worker.terminate(); };
    worker.onerror = (e) => resolve({ error: `worker error: ${e.message}` });
    worker.postMessage('run');
  }));
  const measured = `boot-to-exit ${r.bootMs?.toFixed(0)} ms; run-time translations ${r.translations}; ` +
    `lazy bytes loaded ${r.lazy?.bytes} in ${r.lazy?.files} files`;
  console.log(`wine-hello: ${measured}`);
  test.info().annotations.push({ type: 'measured', description: measured });

  const lines = (r.stderr ?? '').split('\n').filter((l) => l.startsWith('tempwind:'));
  expect(lines, r.stderr).toEqual([]);
  expect(r.error, `${r.error}\n${r.stderr}`).toBeUndefined();
  expect(r.isolated).toBe(true);
  expect(r.imageFallbacks, 'an image mapped without its AOT module').toEqual([]);
  expect([...r.aot].sort()).toEqual([r.programPath, ...AOT].sort());
  const expected = readFileSync(new URL('../native/hello.expected', import.meta.url), 'utf8').replace(/\n/g, '\r\n');
  expect(r.stdout).toBe(expected);
  expect(r.status).toBe(0);
});
