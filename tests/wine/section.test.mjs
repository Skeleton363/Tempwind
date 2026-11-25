// The `section` checks (docs/design/wine-unix.md, section 10): image
// placement through NtCreateSection and NtMapViewOfSection, at the preferred
// base and in a ranges64 slot when the base is taken, and a writable shared
// file view written back, through the in-process server. The harness is
// native/wine/tests/section.c.
// Needs native/wine/build.sh's output in target/wine-unix (or TW_WINE_UNIX_DIR)
// and tests/wine/prepare.sh's target/wine (or TW_WINE_DIR): l_intl.nls,
// hello.exe and two DLLs of the PE tree.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { createUnixHost, DEFAULT_ENV } from '../../runtime/src/wine/unix-host.mjs';
import { m3Imports } from './harness-imports.mjs';

const root = new URL('../../', import.meta.url);
const dirUrl = (env, fallback) => (process.env[env]
  ? pathToFileURL(process.env[env].replace(/\/?$/, '/'))
  : new URL(fallback, root));
const unixDir = dirUrl('TW_WINE_UNIX_DIR', 'target/wine-unix/');
const wineDir = dirUrl('TW_WINE_DIR', 'target/wine/');

// The server's init_directories reads l_intl.nls from the constant NLS path
// (section 8); init_registry starts from the prefix directory.
const NLS_DIR = '/wine/share/wine/nls';
const intl = readFileSync(new URL('native/opt/wine-11.0/share/wine/nls/l_intl.nls', wineDir));
// The images the harness maps, under /tempwind (z: is /). sas.dll and
// softpub.dll are relocatable DLLs with the same preferred base.
const PE = 'pe/lib/wine/x86_64-windows/';
const images = {
  'hello.exe': readFileSync(new URL('hello.exe', wineDir)),
  'sas.dll': readFileSync(new URL(`${PE}sas.dll`, wineDir)),
  'softpub.dll': readFileSync(new URL(`${PE}softpub.dll`, wineDir)),
};

const { default: factory } = await import(new URL('test-section.mjs', unixDir));
const wasm = new WebAssembly.Module(readFileSync(new URL('test-section.wasm', unixDir)));

async function start(env = {}) {
  const out = { stdout: '', stderr: '' };
  const dec = () => { const d = new TextDecoder(); return (b) => d.decode(b, { stream: true }); };
  const o = dec(), e = dec();
  const m3 = m3Imports();
  const host = await createUnixHost({
    factory, wasm, env,
    stdout: (b) => { out.stdout += o(b); },
    stderr: (b) => { out.stderr += e(b); },
    imports: m3.imports,
  });
  m3.attach(host);
  host.FS.mkdirTree(`${DEFAULT_ENV.WINEPREFIX}/dosdevices`);
  host.FS.symlink('/', `${DEFAULT_ENV.WINEPREFIX}/dosdevices/z:`);
  host.FS.mkdirTree(NLS_DIR);
  host.FS.writeFile(`${NLS_DIR}/l_intl.nls`, intl);
  host.FS.mkdirTree('/tempwind');
  for (const [name, bytes] of Object.entries(images)) host.FS.writeFile(`/tempwind/${name}`, bytes);
  return { host, out, m3 };
}

const CHECKS = ['image_exe_preferred_base', 'image_dll_slot', 'file_view_written_back', 'top_down_ceiling',
  'image_mapped'];

test('section: the harness checks pass', async () => {
  const { host, out, m3 } = await start();
  const ret = await host.call('tw_test_main');
  const lines = out.stdout.trim().split('\n');
  assert.deepEqual(lines.filter((l) => l.startsWith('not ok')), [], out.stdout + out.stderr);
  assert.deepEqual(lines.filter((l) => l.startsWith('ok ')).map((l) => l.slice(3)), CHECKS);
  assert.equal(lines.at(-1), `done ${CHECKS.length}/${CHECKS.length}`);
  assert.equal(ret, 0);

  // tw_image_mapped (section 6): the call image_mapped expects, with the
  // fd's path; every image map made one, for a file under /tempwind.
  const expected = lines.filter((l) => l.startsWith('# image_mapped ')).map((l) => {
    const [, , path, base, size] = l.split(' ');
    return { path, base: BigInt(base), size: BigInt(size) };
  });
  assert.equal(expected.length, 1, out.stdout);
  const calls = m3.imageMapped.map(({ path, base, size }) => ({ path, base, size }));
  for (const call of expected) assert.ok(calls.some((c) => c.path === call.path && c.base === call.base && c.size === call.size),
    `no tw_image_mapped ${JSON.stringify(call, (k, v) => typeof v === 'bigint' ? `0x${v.toString(16)}` : v)} in ` +
    JSON.stringify(calls, (k, v) => typeof v === 'bigint' ? `0x${v.toString(16)}` : v));
  assert.ok(m3.imageMapped.every((c) => /^\/tempwind\/[\w.]+\.(exe|dll)$/.test(c.path ?? '')),
    JSON.stringify(m3.imageMapped.map((c) => c.path)));
  assert.deepEqual(m3.invalidated, []); // no PAGE_CODE anywhere: no invalidation
});
