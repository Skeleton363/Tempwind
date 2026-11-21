// The browser tests' static server (tests/browser/server.mjs) serves a file
// only when its real path is inside the directory it is served from:
// target/wine/wine-prefix/dosdevices/z: is a symlink to /, and must not be
// reachable (wine-unix.md, section 8). Also the mounts of TW_WINE_DIR and the
// others. Needs no Wine inputs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, symlinkSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createServer, envMounts, resolvePath } from '../browser/server.mjs';

function fixture() {
  const tmp = mkdtempSync(join(tmpdir(), 'server-test-'));
  const wine = join(tmp, 'wine');
  mkdirSync(join(wine, 'wine-prefix/dosdevices'), { recursive: true });
  mkdirSync(join(wine, 'wine-prefix/drive_c'));
  writeFileSync(join(wine, 'boot-set.json'), '{"ok":1}');
  writeFileSync(join(wine, 'wine-prefix/drive_c/inside.txt'), 'inside');
  symlinkSync('/', join(wine, 'wine-prefix/dosdevices/z:'));
  symlinkSync('../drive_c', join(wine, 'wine-prefix/dosdevices/c:'));
  writeFileSync(join(tmp, 'secret.txt'), 'secret');
  symlinkSync(join(tmp, 'secret.txt'), join(wine, 'leak.txt'));
  writeFileSync(join(tmp, 'rt.wasm'), '\0asm');
  return { tmp, wine };
}

async function serve(cfg) {
  const server = createServer(cfg);
  await new Promise((ok) => server.listen(0, '127.0.0.1', ok));
  const origin = `http://127.0.0.1:${server.address().port}`;
  return { origin, close: () => new Promise((ok) => server.close(ok)) };
}

test('a symlink that leaves the served directory is refused; a normal file is served', async () => {
  const { tmp, wine } = fixture();
  const s = await serve({ mounts: envMounts({ TW_WINE_DIR: wine }) });
  try {
    let r = await fetch(`${s.origin}/target/wine/boot-set.json`);
    assert.equal(r.status, 200);
    assert.equal(await r.text(), '{"ok":1}');
    // A link that stays inside is served.
    r = await fetch(`${s.origin}/target/wine/wine-prefix/dosdevices/c:/inside.txt`);
    assert.equal(r.status, 200);
    assert.equal(await r.text(), 'inside');
    // dosdevices/z: is a link to /: nothing under it is served, nor the link itself.
    for (const p of ['wine-prefix/dosdevices/z:/etc/passwd', 'wine-prefix/dosdevices/z:', 'leak.txt']) {
      r = await fetch(`${s.origin}/target/wine/${p}`);
      assert.equal(r.status, 403, p);
      assert.doesNotMatch(await r.text(), /secret|root:/);
    }
    r = await fetch(`${s.origin}/target/wine/../secret.txt`);
    assert.notEqual(r.status, 200);
    r = await fetch(`${s.origin}/target/wine/missing`);
    assert.equal(r.status, 404);
  } finally { await s.close(); rmSync(tmp, { recursive: true, force: true }); }
});

test('the repository root is served under the same rule, and TW_* files are mounted', async () => {
  const { tmp, wine } = fixture();
  try {
    assert.ok((await resolvePath('/tests/browser/server.mjs')).endsWith('tests/browser/server.mjs'));
    assert.ok(!(await resolvePath('/../../etc/passwd'))); // clamped under the root: not found, never /etc/passwd
    const m = envMounts({ TW_WINE_DIR: wine, TW_WINE_UNIX_DIR: wine, TW_RT_WASM: join(tmp, 'rt.wasm') });
    assert.equal(m.length, 3);
    const s = await serve({ mounts: m });
    try {
      const r = await fetch(`${s.origin}/target/wasm32-unknown-unknown/release/x64wasm_rt.wasm`);
      assert.equal(r.status, 200);
      assert.equal(r.headers.get('content-type'), 'application/wasm');
      assert.equal((await fetch(`${s.origin}/target/wine-unix/boot-set.json`)).status, 200);
    } finally { await s.close(); }
  } finally { rmSync(tmp, { recursive: true, force: true }); }
});
