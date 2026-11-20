// Node's side of a Wine process's inputs (wine-build.md, "Layout under
// target/wine"): reads target/wine (TW_WINE_DIR) and target/wine-unix
// (TW_WINE_UNIX_DIR), and the run-time translator and floating-point helper
// modules. createWineProcess (wine-process.mjs) takes what this returns.
import { readFileSync, readdirSync, readlinkSync, lstatSync, existsSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const repo = fileURLToPath(new URL('../../../', import.meta.url));

/** The directories, from the environment or the repository's defaults. */
export function wineDirs(env = process.env) {
  return {
    wine: resolve(env.TW_WINE_DIR ?? join(repo, 'target/wine')),
    unix: resolve(env.TW_WINE_UNIX_DIR ?? join(repo, 'target/wine-unix')),
    rt: resolve(env.TW_RT_WASM ?? join(repo, 'target/wasm32-unknown-unknown/release/x64wasm_rt.wasm')),
    fp: resolve(env.TW_FP_WASM ?? join(repo, 'target/wasm32-unknown-unknown/release/x64wasm_fp.wasm')),
    overlay: join(repo, 'native/wine/prefix/overlay.reg'),
  };
}

const need = (path, how) => {
  if (!existsSync(path)) throw new Error(`${path} is missing: ${how}`);
  return path;
};

/** The prefix as made (wine-build.md, "The prefix"): top-level files, directories, symlinks. */
export function readPrefix(wineDir) {
  const dir = need(join(wineDir, 'wine-prefix'), 'run tests/wine/prepare.sh') + '/';
  const path = readFileSync(join(wineDir, 'wine-prefix.path'), 'utf8').trim();
  const files = {}, dirs = [], links = [];
  const walk = (rel) => {
    for (const name of readdirSync(dir + rel)) {
      const p = rel ? `${rel}/${name}` : name;
      const st = lstatSync(dir + p);
      if (st.isSymbolicLink()) links.push([p, readlinkSync(dir + p)]);
      else if (st.isDirectory()) { dirs.push(p); walk(p); }
      else if (!rel) files[name] = readFileSync(dir + p);
    }
  };
  walk('');
  return { path, files, dirs, links };
}

/**
 * A synchronous reader of target/wine by relative path. Files the manifest
 * lists are checked against its size and sha256 when read.
 */
export function nodeReader(wineDir, manifest) {
  const known = new Map((manifest?.files ?? []).map((f) => [f.source, f]));
  return (source) => {
    const bytes = new Uint8Array(readFileSync(join(wineDir, source)));
    const f = known.get(source);
    if (f && createHash('sha256').update(bytes).digest('hex') !== f.sha256) {
      throw new Error(`${join(wineDir, source)}: sha256 differs from boot-set.json's (rerun tests/wine/prepare.sh)`);
    }
    return bytes;
  };
}

/**
 * Everything createWineProcess needs but the program and the callbacks.
 * Compiles the modules once; pass the result to every process of a run.
 */
export async function loadWineInputs(dirs = wineDirs()) {
  const json = (p) => JSON.parse(readFileSync(p, 'utf8'));
  const prep = 'run tests/wine/prepare.sh (or set TW_WINE_DIR)';
  const manifest = json(need(join(dirs.wine, 'boot-set.json'), prep));
  const catalog = json(need(join(dirs.wine, 'catalog/catalog.json'), prep));
  const compile = (p, how) => new WebAssembly.Module(readFileSync(need(p, how)));
  const { default: factory } = await import(pathToFileURL(need(join(dirs.unix, 'wine-unix.mjs'), 'build it with native/wine/build.sh (or set TW_WINE_UNIX_DIR)')).href);
  return {
    factory,
    wasm: compile(join(dirs.unix, 'wine-unix.wasm'), 'build it with native/wine/build.sh'),
    bridge: compile(join(dirs.wine, 'bridge.wasm'), prep),
    rt: compile(dirs.rt, 'cargo build -p x64wasm-rt --release --target wasm32-unknown-unknown (or set TW_RT_WASM)'),
    fp: compile(dirs.fp, 'cargo build -p x64wasm-fp --release --target wasm32-unknown-unknown (or set TW_FP_WASM)'),
    manifest,
    catalog,
    read: nodeReader(dirs.wine, manifest),
    prefix: readPrefix(dirs.wine),
    overlay: readFileSync(dirs.overlay),
    moduleCache: new Map(),
  };
}
