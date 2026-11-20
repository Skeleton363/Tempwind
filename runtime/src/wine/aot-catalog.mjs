// The AOT catalog of a Wine process (docs/design/wine-build.md, "The
// catalog"; abi.md v5, the import tw_image_mapped): one translated module per
// PE file, keyed by its MEMFS path, its sha256 and the base it was translated
// at. tests/wine/catalog.mjs writes catalog.json. Host-neutral.

/** The options every module of a Wine process is built with (translator.md, "Wine processes"). */
export const CATALOG_OPTIONS = Object.freeze(['--shared', '--null-check', '--store-check']);

const SHA256 = /^[0-9a-f]{64}$/;

/**
 * Validates catalog.json (schema 1).
 * @param {object} json
 * @returns {{wine: string, translator: string, modules: Map<string, {path: string, sha256: string, base: bigint, wasm: string, plan: string}>}}
 */
export function parseCatalog(json) {
  if (json?.schema !== 1) throw new Error(`catalog.json: schema ${JSON.stringify(json?.schema)}, expected 1`);
  const options = json.options ?? [];
  for (const o of CATALOG_OPTIONS) {
    if (!options.includes(o)) throw new Error(`catalog.json: built without ${o} (options ${options.join(' ')})`);
  }
  if (!Array.isArray(json.modules)) throw new Error('catalog.json: modules must be an array');
  const modules = new Map();
  json.modules.forEach((m, i) => {
    const w = `catalog.json: modules[${i}]`;
    if (typeof m.path !== 'string' || !m.path.startsWith('/')) throw new Error(`${w}: path must be an absolute MEMFS path`);
    if (typeof m.sha256 !== 'string' || !SHA256.test(m.sha256)) throw new Error(`${w} (${m.path}): sha256 must be 64 lower-case hex digits`);
    if (typeof m.base !== 'string' || !/^0x[0-9a-fA-F]+$/.test(m.base)) throw new Error(`${w} (${m.path}): base must be a hex string`);
    if (typeof m.wasm !== 'string' || typeof m.plan !== 'string') throw new Error(`${w} (${m.path}): wasm and plan must be file names`);
    if (modules.has(m.path)) throw new Error(`${w}: ${m.path} is listed twice`);
    modules.set(m.path, { path: m.path, sha256: m.sha256, base: BigInt(m.base), wasm: m.wasm, plan: m.plan });
  });
  return { wine: String(json.wine ?? ''), translator: String(json.translator ?? ''), modules };
}

/**
 * Finds the module for an image mapped from `path` at `base`, whose file has
 * `sha256` (null if unknown). A hit needs all three to match.
 * @returns {{entry: object} | {reason: string}}
 */
export function matchCatalog(catalog, path, sha256, base) {
  const entry = path === null ? undefined : catalog?.modules.get(path);
  if (!entry) return { reason: path === null ? 'the image file has no path' : 'not in the catalog' };
  if (sha256 === null) return { reason: 'the file\'s sha256 is unknown (not in the boot-set manifest)' };
  if (sha256 !== entry.sha256) return { reason: `sha256 ${sha256.slice(0, 12)}… differs from the catalog's ${entry.sha256.slice(0, 12)}…` };
  if (BigInt(base) !== entry.base) return { reason: `mapped at 0x${BigInt(base).toString(16)}, translated at 0x${entry.base.toString(16)}` };
  return { entry };
}
