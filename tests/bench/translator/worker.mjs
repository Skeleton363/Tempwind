// Worker entry for bench.mjs --browser: {base} -> {depth} | {error}.
import { depthProbes, load } from './core.mjs';

const read = async (url) => {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`fetch ${url}: HTTP ${r.status}`);
  return new Uint8Array(await r.arrayBuffer());
};

self.onmessage = async (e) => {
  try {
    self.postMessage({ depth: await depthProbes(await load(e.data.base, read)) });
  } catch (err) {
    self.postMessage({ error: String(err?.stack ?? err) });
  }
};
