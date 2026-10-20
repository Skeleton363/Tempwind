// Worker for threads.spec: blocks in wasm memory.atomic.wait32 on a shared memory64.

self.onmessage = (e) => {
  const { memory, bytes, rounds } = e.data;
  const mod = new WebAssembly.Module(bytes);
  const { wait, load } = new WebAssembly.Instance(mod, { env: { memory } }).exports;
  const i32 = new Int32Array(memory.buffer);

  // Phase 1: single wake. i32[0] starts at 0; block until main changes it.
  self.postMessage({ type: 'ready' });
  const r = wait(0n, 0, 10_000_000_000n); // 10 s in ns; 0 = woken, 1 = not-equal, 2 = timeout
  self.postMessage({ type: 'woke', waitResult: r, value: load(0n), tid: 'worker' });

  // Phase 2: ping-pong. Main bumps i32[0] to i; worker echoes it to i32[1].
  let last = load(0n);
  for (let i = 0; i < rounds; i++) {
    while (load(0n) === last) wait(0n, last, 10_000_000_000n);
    last = load(0n);
    Atomics.store(i32, 1, last);
    Atomics.notify(i32, 1);
  }
  self.postMessage({ type: 'done' });
};
