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
import { DEFAULT_ENV } from '../../runtime/src/wine/unix-host.mjs';
import { startHarness, runChecks, wineDir } from './harness-imports.mjs';

const PE = 'pe/lib/wine/x86_64-windows/';
const images = {
  'hello.exe': readFileSync(new URL('hello.exe', wineDir)),
  'sas.dll': readFileSync(new URL(`${PE}sas.dll`, wineDir)),
  'softpub.dll': readFileSync(new URL(`${PE}softpub.dll`, wineDir)),
};

/** A fresh instance of test-section, with z: pointing at / and the images in /tempwind. */
const start = (env = {}) => startHarness('section', {
  env,
  imports: true,
  setup(host) {
    host.FS.symlink('/', `${DEFAULT_ENV.WINEPREFIX}/dosdevices/z:`);
    host.FS.mkdirTree('/tempwind');
    for (const [name, bytes] of Object.entries(images)) host.FS.writeFile(`/tempwind/${name}`, bytes);
  },
});

const CHECKS = ['image_exe_preferred_base', 'image_dll_slot', 'file_view_written_back', 'top_down_ceiling',
  'image_mapped'];

test('section: the harness checks pass', async () => {
  const h = await start();
  const lines = await runChecks(h, CHECKS);
  const { out, harness } = h;

  // tw_image_mapped (section 6): the call image_mapped expects, with the
  // fd's path; every image map made one, for a file under /tempwind.
  const expected = lines.filter((l) => l.startsWith('# image_mapped ')).map((l) => {
    const [, , path, base, size] = l.split(' ');
    return { path, base: BigInt(base), size: BigInt(size) };
  });
  assert.equal(expected.length, 1, out.stdout);
  const calls = harness.imageMapped.map(({ path, base, size }) => ({ path, base, size }));
  for (const call of expected) assert.ok(calls.some((c) => c.path === call.path && c.base === call.base && c.size === call.size),
    `no tw_image_mapped ${JSON.stringify(call, (k, v) => typeof v === 'bigint' ? `0x${v.toString(16)}` : v)} in ` +
    JSON.stringify(calls, (k, v) => typeof v === 'bigint' ? `0x${v.toString(16)}` : v));
  assert.ok(harness.imageMapped.every((c) => /^\/tempwind\/[\w.]+\.(exe|dll)$/.test(c.path ?? '')),
    JSON.stringify(harness.imageMapped.map((c) => c.path)));
  assert.deepEqual(harness.invalidated, []); // no PAGE_CODE anywhere: no invalidation
});
