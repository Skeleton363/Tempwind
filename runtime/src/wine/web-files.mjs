// The browser's side of a Wine process's inputs (docs/design/wine-unix.md,
// section 8, "The browser host (M3)"): the counterpart of node-files.mjs's
// loadWineInputs, by URL from static assets, in a Worker. createWineProcess
// (wine-process.mjs) takes what this returns.
//
// Before `boot` it fetches (asynchronously): the modules, boot-set.json,
// catalog/catalog.json, prefix.json, the prefix's top-level files and the
// overlay. The lazy files and each catalog module's .wasm and .plan.json are
// read on first use with one synchronous XHR (`xhrReader`), and a module is
// compiled with the synchronous `new WebAssembly.Module`, which a Worker
// allows at any size. There is no sha256 check against the manifest here;
// Node checks on the first read.
import { xhrReader } from './lazy-node.mjs';

const dirUrl = (u) => {
  const s = new URL(u, globalThis.location?.href ?? undefined).href;
  return s.endsWith('/') ? s : s + '/';
};

async function get(url, what) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${what}: ${url}: HTTP ${res.status}`);
  return res;
}
const getJson = async (url, what) => (await get(url, what)).json();
const getBytes = async (url, what) => new Uint8Array(await (await get(url, what)).arrayBuffer());
const compile = async (url, what) => WebAssembly.compile(await getBytes(url, what));

/**
 * @param {object} urls  base URLs (absolute, or relative to the Worker's own URL)
 * @param {string} urls.wine  target/wine (the release, catalog, boot-set.json, prefix.json), a directory
 * @param {string} urls.unix  target/wine-unix (wine-unix.mjs and .wasm), a directory
 * @param {string} urls.rt  x64wasm_rt.wasm
 * @param {string} urls.fp  x64wasm_fp.wasm
 * @param {string} urls.overlay  native/wine/prefix/overlay.reg
 * @param {object} [extra]
 * @param {{path: string, bytes?: Uint8Array, dir?: boolean, mtime: number}[]} [extra.files]  host files
 *   (host-files.mjs), e.g. received from the page by postMessage, passed through to createWineProcess;
 *   `proc.collect(dir)` returns the tree after the run, for the Worker to post back
 * @returns the same shape as loadWineInputs: factory, wasm, bridge, rt, fp,
 *   manifest, catalog, read, prefix, overlay and moduleCache, and `files`
 */
export async function loadWebWineInputs(urls, extra = {}) {
  const wine = dirUrl(urls.wine);
  const unix = dirUrl(urls.unix);
  const prep = 'run tests/wine/prepare.sh and tests/wine/catalog.mjs --all';
  const [manifest, catalog, listing] = await Promise.all([
    getJson(wine + 'boot-set.json', `boot-set.json (${prep})`),
    getJson(wine + 'catalog/catalog.json', `catalog.json (${prep})`),
    getJson(wine + 'prefix.json', `prefix.json (${prep})`),
  ]);
  if (listing.schema !== 1) throw new Error(`prefix.json: schema ${listing.schema}, expected 1`);
  const [mod, wasm, bridge, rt, fp, overlay, ...files] = await Promise.all([
    import(/* @vite-ignore */ unix + 'wine-unix.mjs'),
    compile(unix + 'wine-unix.wasm', 'wine-unix.wasm (native/wine/build.sh)'),
    compile(wine + 'bridge.wasm', `bridge.wasm (${prep})`),
    compile(urls.rt, 'x64wasm_rt.wasm'),
    compile(urls.fp, 'x64wasm_fp.wasm'),
    getBytes(urls.overlay, 'overlay.reg'),
    ...listing.files.map((name) => getBytes(`${wine}wine-prefix/${name}`, `prefix file ${name}`)),
  ]);
  return {
    factory: mod.default,
    wasm,
    bridge,
    rt,
    fp,
    manifest,
    catalog,
    read: xhrReader(wine),
    prefix: {
      path: listing.path,
      files: Object.fromEntries(listing.files.map((name, i) => [name, files[i]])),
      dirs: listing.dirs,
      links: listing.links,
      installFiles: listing.installFiles,
    },
    overlay,
    files: extra.files ?? [],
    moduleCache: new Map(),
  };
}
