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
  const host = await createUnixHost({
    factory, wasm, env,
    stdout: (b) => { out.stdout += o(b); },
    stderr: (b) => { out.stderr += e(b); },
  });
  host.FS.mkdirTree(`${DEFAULT_ENV.WINEPREFIX}/dosdevices`);
  host.FS.symlink('/', `${DEFAULT_ENV.WINEPREFIX}/dosdevices/z:`);
  host.FS.mkdirTree(NLS_DIR);
  host.FS.writeFile(`${NLS_DIR}/l_intl.nls`, intl);
  host.FS.mkdirTree('/tempwind');
  for (const [name, bytes] of Object.entries(images)) host.FS.writeFile(`/tempwind/${name}`, bytes);
  return { host, out };
}

const CHECKS = ['image_exe_preferred_base', 'image_dll_slot', 'file_view_written_back'];

test('section: the harness checks pass', async () => {
  const { host, out } = await start();
  const ret = await host.call('tw_test_main');
  const lines = out.stdout.trim().split('\n');
  assert.deepEqual(lines.filter((l) => l.startsWith('not ok')), [], out.stdout + out.stderr);
  assert.deepEqual(lines.filter((l) => l.startsWith('ok ')).map((l) => l.slice(3)), CHECKS);
  assert.equal(lines.at(-1), `done ${CHECKS.length}/${CHECKS.length}`);
  assert.equal(ret, 0);
});
