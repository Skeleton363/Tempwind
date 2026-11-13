// Worker for wine-boundary.spec: runs the M1 boundary spike
// (native/spike/boundary/driver.mjs) on target/wine-spike/'s build.
import { runBoundary, verify } from '/native/spike/boundary/driver.mjs';

const bytes = async (name) => {
  const res = await fetch(`/target/wine-spike/${name}`);
  if (!res.ok) throw new Error(`${name}: HTTP ${res.status} (run native/spike/boundary/build.sh)`);
  return new Uint8Array(await res.arrayBuffer());
};
const json = (v) => JSON.stringify(v, (_, x) => (typeof x === 'bigint' ? '0x' + x.toString(16) : x));

self.onmessage = async () => {
  try {
    const { default: factory } = await import('/target/wine-spike/boundary.mjs');
    const r = await runBoundary({
      factory,
      emWasm: await bytes('boundary.wasm'),
      guestA: await bytes('guest_a.wasm'),
      guestB: await bytes('guest_b.wasm'),
    });
    self.postMessage({
      isolated: self.crossOriginIsolated,
      problems: verify(r),
      summary: json({ userAgent: navigator.userAgent, timings: r.timings, bench: r.bench, negative: r.negative, memoryForm: r.memoryForm }),
    });
  } catch (e) {
    self.postMessage({ error: String(e?.stack ?? e) });
  }
};
