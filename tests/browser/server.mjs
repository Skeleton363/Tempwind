// Static server for browser tests: serves the repo root on 127.0.0.1 with
// cross-origin isolation headers. No dependencies. TW_WINE_DIR, TW_WINE_UNIX_DIR,
// TW_RT_WASM and TW_FP_WASM remap the Wine inputs (envMounts); every file is
// served only if its real path is inside the directory it is served from.
//   node tests/browser/server.mjs [port]     (PORT env also works; default 4173)
import http from 'node:http';
import { createReadStream } from 'node:fs';
import { stat, realpath } from 'node:fs/promises';
import { resolve, join, extname, sep, posix } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.json': 'application/json',
  '.wasm': 'application/wasm',
  '.css': 'text/css; charset=utf-8',
  '.txt': 'text/plain; charset=utf-8',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
};
const HEADERS = {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Embedder-Policy': 'require-corp',
  'Cross-Origin-Resource-Policy': 'same-origin',
  'Cache-Control': 'no-store',
};
const WASM32 = '/target/wasm32-unknown-unknown/release/';

/**
 * The mounts the environment asks for: with TW_WINE_DIR and TW_WINE_UNIX_DIR
 * set, those directories are served at /target/wine/ and /target/wine-unix/
 * (the inputs Node's wineDirs() reads), and TW_RT_WASM and TW_FP_WASM at the
 * run-time translator's and the floating-point helper's usual URLs. Without
 * them the repository's own target/ is served.
 */
export function envMounts(env = process.env) {
  const mounts = [];
  if (env.TW_WINE_DIR) mounts.push({ url: '/target/wine/', dir: env.TW_WINE_DIR });
  if (env.TW_WINE_UNIX_DIR) mounts.push({ url: '/target/wine-unix/', dir: env.TW_WINE_UNIX_DIR });
  if (env.TW_RT_WASM) mounts.push({ url: WASM32 + 'x64wasm_rt.wasm', file: env.TW_RT_WASM });
  if (env.TW_FP_WASM) mounts.push({ url: WASM32 + 'x64wasm_fp.wasm', file: env.TW_FP_WASM });
  return mounts;
}

/**
 * Map a request URL to the real path of a file to serve: null when it is
 * outside what is served, undefined when it doesn't exist. Containment is by
 * realpath: a file is served only when its real path is inside the real path
 * of the directory it is served from, so a symlink that leaves it
 * (wine-prefix/dosdevices/z: is one, to /) is refused.
 * @param {string} urlPath
 * @param {{root?: string, mounts?: ({url: string, dir: string}|{url: string, file: string})[]}} [cfg]
 */
export async function resolvePath(urlPath, { root = ROOT, mounts = [] } = {}) {
  let p;
  try { p = decodeURIComponent(urlPath.split('?')[0].split('#')[0]); } catch { return null; }
  if (p.includes('\0') || !p.startsWith('/')) return null;
  const norm = posix.normalize(p);
  let base = root, rel = norm.slice(1);
  for (const m of mounts) {
    if (m.file !== undefined && norm === m.url) { base = null; rel = m.file; break; }
    if (m.dir !== undefined && norm.startsWith(m.url)) { base = m.dir; rel = norm.slice(m.url.length); break; }
  }
  try {
    if (base === null) return await realpath(resolve(rel)); // a mounted file, named by the environment
    const realBase = await realpath(resolve(base));
    const real = await realpath(join(realBase, rel));
    return real === realBase || real.startsWith(realBase + sep) ? real : null;
  } catch {
    return undefined;
  }
}

export function createServer(cfg = { mounts: envMounts() }) {
  return http.createServer(async (req, res) => {
    const send = (code, body, extra = {}) => {
      res.writeHead(code, { ...HEADERS, 'Content-Type': 'text/plain; charset=utf-8', ...extra });
      res.end(body);
    };
    if (req.method !== 'GET' && req.method !== 'HEAD') return send(405, 'method not allowed');
    const url = (req.url ?? '/').split(/[?#]/)[0];
    let file = await resolvePath(url, cfg);
    if (file === null) return send(403, 'forbidden');
    if (file === undefined) return send(404, 'not found');
    try {
      let st = await stat(file);
      if (st.isDirectory()) {
        file = await resolvePath(posix.join(url, 'index.html'), cfg);
        if (file === null) return send(403, 'forbidden');
        if (file === undefined) return send(404, 'not found');
        st = await stat(file);
      }
      res.writeHead(200, {
        ...HEADERS,
        'Content-Type': MIME[extname(file)] ?? 'application/octet-stream',
        'Content-Length': st.size,
      });
      if (req.method === 'HEAD') return res.end();
      createReadStream(file).pipe(res);
    } catch {
      send(404, 'not found');
    }
  });
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const port = Number(process.argv[2] ?? process.env.PORT ?? 4173);
  createServer().listen(port, '127.0.0.1', () => console.log(`serving ${ROOT} on http://127.0.0.1:${port}`));
}
