// Static server for browser tests: serves the repo root on 127.0.0.1 with
// cross-origin isolation headers. No dependencies.
//   node tests/browser/server.mjs [port]     (PORT env also works; default 4173)
import http from 'node:http';
import { createReadStream } from 'node:fs';
import { stat } from 'node:fs/promises';
import { resolve, join, extname, sep } from 'node:path';
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

/** Map a request URL to a file path under ROOT, or null if it escapes ROOT. */
export function resolvePath(urlPath) {
  let p;
  try { p = decodeURIComponent(urlPath.split('?')[0].split('#')[0]); } catch { return null; }
  if (p.includes('\0')) return null;
  const full = resolve(join(ROOT, p));
  if (full !== ROOT && !full.startsWith(ROOT + sep)) return null;
  return full;
}

export function createServer() {
  return http.createServer(async (req, res) => {
    const send = (code, body, extra = {}) => {
      res.writeHead(code, { ...HEADERS, 'Content-Type': 'text/plain; charset=utf-8', ...extra });
      res.end(body);
    };
    if (req.method !== 'GET' && req.method !== 'HEAD') return send(405, 'method not allowed');
    let file = resolvePath(req.url ?? '/');
    if (!file) return send(403, 'forbidden');
    try {
      let st = await stat(file);
      if (st.isDirectory()) { file = join(file, 'index.html'); st = await stat(file); }
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
