// The `session` checks (docs/design/wine-unix.md, section 10, "The M4
// checks"): the session's shared memory as one memory (section 7). The
// harness is native/wine/tests/session.c; it starts the in-process server,
// whose init_directories sets the session up, so the prefix directory and
// l_intl.nls go into MEMFS first. tw_session_view reads what the server
// wrote, at the start and in a grown block; the loud stops (a view past the
// cap, growth past it, a view before the setup) each run alone in a fresh
// instance (TW_TEST_ONLY).
// Needs native/wine/build.sh's output in target/wine-unix (or TW_WINE_UNIX_DIR)
// and tests/wine/prepare.sh's target/wine (or TW_WINE_DIR) for l_intl.nls.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { createUnixHost, DEFAULT_ENV } from '../../runtime/src/wine/unix-host.mjs';
import { harnessImports } from './harness-imports.mjs';

const root = new URL('../../', import.meta.url);
const dirUrl = (env, fallback) => (process.env[env]
  ? pathToFileURL(process.env[env].replace(/\/?$/, '/'))
  : new URL(fallback, root));
const unixDir = dirUrl('TW_WINE_UNIX_DIR', 'target/wine-unix/');
const wineDir = dirUrl('TW_WINE_DIR', 'target/wine/');

const NLS_DIR = '/wine/share/wine/nls';
const intl = readFileSync(new URL('native/opt/wine-11.0/share/wine/nls/l_intl.nls', wineDir));

const { default: factory } = await import(new URL('test-session.mjs', unixDir));
const wasm = new WebAssembly.Module(readFileSync(new URL('test-session.wasm', unixDir)));

async function start(env = {}) {
  const out = { stdout: '', stderr: '' };
  const dec = () => { const d = new TextDecoder(); return (b) => d.decode(b, { stream: true }); };
  const o = dec(), e = dec();
  const harness = harnessImports();
  const host = await createUnixHost({
    factory, wasm, env,
    stdout: (b) => { out.stdout += o(b); },
    stderr: (b) => { out.stderr += e(b); },
    imports: harness.imports,
  });
  harness.attach(host);
  host.FS.mkdirTree(`${DEFAULT_ENV.WINEPREFIX}/dosdevices`);
  host.FS.mkdirTree(NLS_DIR);
  host.FS.writeFile(`${NLS_DIR}/l_intl.nls`, intl);
  return { host, out };
}

// In the harness's order (it runs them in one instance).
const CHECKS = ['view_start', 'view_grown_block'];

test('session: the harness checks pass', async () => {
  const { host, out } = await start();
  const ret = await host.call('tw_test_main');
  const lines = out.stdout.trim().split('\n');
  assert.deepEqual(lines.filter((l) => l.startsWith('not ok')), [], out.stdout + out.stderr);
  assert.deepEqual(lines.filter((l) => l.startsWith('ok ')).map((l) => l.slice(3)), CHECKS);
  assert.equal(lines.at(-1), `done ${CHECKS.length}/${CHECKS.length}`);
  assert.equal(ret, 0);
  assert.ok(!/^tempwind:/m.test(out.stderr), `stderr: ${out.stderr}`);
});

// Each loud stop alone, in a fresh instance (section 10). The cap is
// TW_SESSION_CAP, 16 MiB (section 7).
for (const [check, line] of [
  ['view_past_cap', 'tempwind: unsupported: session memory past 16 MiB'],
  ['grow_past_cap', 'tempwind: unsupported: session memory past 16 MiB'],
  ['view_before_setup', 'tempwind: unsupported: session memory: a view before the server set the session up'],
]) {
  test(`session: ${check} stops loudly`, async () => {
    const { host, out } = await start({ TW_TEST_ONLY: check });
    await assert.rejects(host.call('tw_test_main'));
    assert.ok(out.stderr.split('\n').includes(line), `stderr: ${out.stderr}`);
    assert.ok(!/^done /m.test(out.stdout), `stdout: ${out.stdout}`);
  });
}
