import { test, expect } from '@playwright/test';

test('cross-origin isolated shared memory64 + Atomics wake across a Worker', async ({ page }, testInfo) => {
  await page.goto('/tests/browser/index.html');
  page.on('console', (m) => console.log(`[page] ${m.text()}`));

  const result = await page.evaluate(async () => {
    const { createMemory64 } = await import('/runtime/src/memory.mjs');
    const { threadsModule } = await import('/runtime/src/wasm-mini.mjs');
    const out = { isolated: self.crossOriginIsolated };

    const { memory, form } = createMemory64({ initialPages: 1, maximumPages: 262144, shared: true });
    out.form = form;
    out.isShared = memory.buffer instanceof SharedArrayBuffer;
    const i32 = new Int32Array(memory.buffer);

    const worker = new Worker('/tests/browser/threads-worker.mjs', { type: 'module' });
    const next = (type) => new Promise((res, rej) => {
      const h = (e) => { if (e.data.type === type) { worker.removeEventListener('message', h); res(e.data); } };
      worker.addEventListener('message', h);
      worker.addEventListener('error', (e) => rej(new Error('worker error: ' + e.message)), { once: true });
    });
    const ROUNDS = 300;
    const ready = next('ready');
    worker.postMessage({ memory, bytes: threadsModule(), rounds: ROUNDS });
    await ready;
    await new Promise((r) => setTimeout(r, 100)); // let the worker reach wait32

    // Single wake: store a value, notify, worker reads it through wasm.
    const woke = next('woke');
    const t0 = performance.now();
    Atomics.store(i32, 0, 42);
    out.notified = Atomics.notify(i32, 0, 1); // number of waiters woken
    const w = await woke;
    out.firstWakeMs = performance.now() - t0;
    out.woke = w;

    // Ping-pong round trips with Atomics.waitAsync for the ack.
    const lat = [];
    for (let i = 1; i <= ROUNDS; i++) {
      const t = performance.now();
      Atomics.store(i32, 0, 1000 + i);
      Atomics.notify(i32, 0, 1);
      while (Atomics.load(i32, 1) !== 1000 + i) {
        const { async, value } = Atomics.waitAsync(i32, 1, Atomics.load(i32, 1), 5000);
        if (async) await value;
      }
      lat.push(performance.now() - t);
    }
    const done = await next('done');
    if (done.error) throw new Error('worker: ' + done.error);
    worker.terminate();
    lat.sort((a, b) => a - b);
    out.rtt = { n: lat.length, medianMs: lat[lat.length >> 1], p95Ms: lat[Math.floor(lat.length * 0.95)], minMs: lat[0] };
    return out;
  });

  console.log('threads result', JSON.stringify(result));
  expect(result.isolated).toBe(true);
  expect(result.isShared).toBe(true);
  expect(result.woke.waitResult).toBe(0); // woken, not "not-equal" or timed out
  expect(result.woke.value).toBe(42);
  expect(result.notified).toBe(1);
  const msg = `memory64 form=${result.form}; first wake ${result.firstWakeMs.toFixed(2)} ms; ` +
    `ping-pong RTT median ${result.rtt.medianMs.toFixed(3)} ms, p95 ${result.rtt.p95Ms.toFixed(3)} ms, min ${result.rtt.minMs.toFixed(3)} ms (n=${result.rtt.n})`;
  console.log(msg);
  testInfo.annotations.push({ type: 'measurement', description: msg });
});
