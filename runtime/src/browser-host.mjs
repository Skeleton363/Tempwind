// Browser host: run a spike manifest in the page and/or in a Worker.
import { runManifest } from './run.mjs';
import { loadFp } from './fp.mjs';

async function fetchJson(url) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`fetch ${url}: HTTP ${r.status}`);
  return r.json();
}

/**
 * opts with `fp` (the helper module's exports) loaded from `opts.fpUrl`, if given.
 * Instance exports cannot cross postMessage, so a Worker loads its own from the URL.
 */
export async function withFp(opts = {}) {
  if (!opts.fpUrl || opts.fp) return opts;
  const r = await fetch(opts.fpUrl);
  if (!r.ok) throw new Error(`fetch ${opts.fpUrl}: HTTP ${r.status}`);
  return { ...opts, fp: await loadFp(new Uint8Array(await r.arrayBuffer())) };
}

/** @param {string} base e.g. "/target/spike" */
export async function runInPage(base, opts = {}) {
  opts = await withFp(opts);
  const manifest = await fetchJson(`${base}/manifest.json`);
  return runManifest(manifest, async (name) => {
    const r = await fetch(`${base}/${name}`);
    if (!r.ok) throw new Error(`fetch ${name}: HTTP ${r.status}`);
    return new Uint8Array(await r.arrayBuffer());
  }, opts);
}

/** Runs the manifest inside a module Worker (src/worker.mjs). */
export function runInWorker(base, opts = {}) {
  return workerCall({ base: new URL(base, location.href).href, opts: absFp(opts) }, 'results');
}

/** Runs one case (wasm bytes + spec) inside a module Worker. */
export function runCaseInWorker(wasm, spec, opts = {}) {
  return workerCall({ wasm, spec, opts: absFp(opts) }, 'result');
}

function workerCall(message, key) {
  return new Promise((resolve, reject) => {
    const w = new Worker(new URL('./worker.mjs', import.meta.url), { type: 'module' });
    w.onmessage = (e) => {
      w.terminate();
      if (e.data.error) reject(new Error(e.data.error)); else resolve(e.data[key]);
    };
    w.onerror = (e) => { w.terminate(); reject(new Error(`worker error: ${e.message}`)); };
    w.postMessage(message);
  });
}

/** Resolves `fpUrl` against the page, so a Worker (other base URL) fetches the same file. */
function absFp(opts) {
  const { fp, ...rest } = opts; // exports are not cloneable
  return opts.fpUrl ? { ...rest, fpUrl: new URL(opts.fpUrl, location.href).href } : rest;
}
