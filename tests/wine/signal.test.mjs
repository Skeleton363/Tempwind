// The `signal` checks (docs/design/wine-unix.md v3, section 10, "The M4
// checks"): SIGUSR1 delivered at the wait (section 9, "Signals"), suspension
// and another thread's context through the in-process server (section 11),
// the per-thread mask, a suspended start, and a user APC in an alertable
// wait. The harness is native/wine/tests/signal.c; it starts the in-process
// server, so the prefix directory and l_intl.nls go into MEMFS first.
// Needs native/wine/build.sh's output in target/wine-unix (or TW_WINE_UNIX_DIR)
// and tests/wine/prepare.sh's target/wine (or TW_WINE_DIR) for l_intl.nls.
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

const NLS_DIR = '/wine/share/wine/nls';
const intl = readFileSync(new URL('native/opt/wine-11.0/share/wine/nls/l_intl.nls', wineDir));

const { default: factory } = await import(new URL('test-signal.mjs', unixDir));
const wasm = new WebAssembly.Module(readFileSync(new URL('test-signal.wasm', unixDir)));

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
  host.FS.mkdirTree(NLS_DIR);
  host.FS.writeFile(`${NLS_DIR}/l_intl.nls`, intl);
  return { host, out };
}

const lines = (s) => s.split('\n').filter((l) => l);

test('signal: the harness checks pass', async () => {
  const { host, out } = await start();
  const ret = await host.call('tw_test_main');
  const got = lines(out.stdout);
  const checks = ['suspend_other', 'self_suspend', 'context_other', 'mask_blocks_sigusr1',
    'suspended_start', 'apc_in_alertable_wait'];
  assert.deepEqual(got.filter((l) => l.startsWith('not ok')), [], out.stdout + out.stderr);
  assert.deepEqual(got.filter((l) => l.startsWith('ok ')).map((l) => l.slice(3)), checks);
  assert.equal(got.at(-1), `done ${checks.length}/${checks.length}`);
  assert.equal(ret, 0);
  assert.ok(!/^tempwind:/m.test(out.stderr), `stderr: ${out.stderr}`);
});
