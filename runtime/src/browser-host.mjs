// Browser host: run a spike manifest in the page and/or in a Worker.
import { runManifest } from './run.mjs';

async function fetchJson(url) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`fetch ${url}: HTTP ${r.status}`);
  return r.json();
}

/** @param {string} base e.g. "/target/spike" */
export async function runInPage(base, opts = {}) {
  const manifest = await fetchJson(`${base}/manifest.json`);
  return runManifest(manifest, async (name) => {
    const r = await fetch(`${base}/${name}`);
    if (!r.ok) throw new Error(`fetch ${name}: HTTP ${r.status}`);
    return new Uint8Array(await r.arrayBuffer());
  }, opts);
}

/** Runs the manifest inside a module Worker (src/worker.mjs). */
export function runInWorker(base, opts = {}) {
  return new Promise((resolve, reject) => {
    const w = new Worker(new URL('./worker.mjs', import.meta.url), { type: 'module' });
    w.onmessage = (e) => {
      w.terminate();
      if (e.data.error) reject(new Error(e.data.error)); else resolve(e.data.results);
    };
    w.onerror = (e) => { w.terminate(); reject(new Error(`worker error: ${e.message}`)); };
    w.postMessage({ base: new URL(base, location.href).href, opts });
  });
}
