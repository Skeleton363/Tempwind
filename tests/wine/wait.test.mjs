// The `wait` checks (docs/design/wine-unix.md, section 10): waits on one
// thread through the in-process server and green.c, scheduled by the JS host
// (runtime/src/wine/unix-host.mjs). The harness is native/wine/tests/wait.c;
// the loud stop (a wait nothing can end) runs alone in a fresh instance.
// Needs native/wine/build.sh's output in target/wine-unix (or TW_WINE_UNIX_DIR)
// and l_intl.nls from tests/wine/prepare.sh's target/wine (or TW_WINE_DIR).
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

const { default: factory } = await import(new URL('test-wait.mjs', unixDir));
const wasm = new WebAssembly.Module(readFileSync(new URL('test-wait.wasm', unixDir)));

/** A fresh instance of test-wait with the prefix and l_intl.nls in MEMFS. */
async function start(env = {}) {
  const out = { stdout: '', stderr: '' };
  const dec = () => { const d = new TextDecoder(); return (b) => d.decode(b, { stream: true }); };
  const o = dec(), e = dec();
  const host = await createUnixHost({
    factory, wasm, env,
    stdout: (b) => { out.stdout += o(b); },
    stderr: (b) => { out.stderr += e(b); },
  });
  host.FS.mkdirTree(DEFAULT_ENV.WINEPREFIX);
  host.FS.mkdirTree(NLS_DIR);
  host.FS.writeFile(`${NLS_DIR}/l_intl.nls`, intl);
  return { host, out };
}

const CHECKS = ['event_timeout', 'event_signaled', 'timer_fires', 'zero_timeout_polls',
  'delay_not_alertable', 'delay_alertable'];

test('wait: the harness checks pass', async () => {
  const { host, out } = await start();
  const ret = await host.call('tw_test_main');
  const lines = out.stdout.trim().split('\n');
  assert.deepEqual(lines.filter((l) => l.startsWith('not ok')), [], out.stdout + out.stderr);
  assert.deepEqual(lines.filter((l) => l.startsWith('ok ')).map((l) => l.slice(3)), CHECKS);
  assert.equal(lines.at(-1), `done ${CHECKS.length}/${CHECKS.length}`);
  assert.equal(ret, 0);
});

// Alone (section 10): nothing is runnable and no timeout is pending, so the
// scheduler stops the waiting thread, after C named its wait on stderr.
test('wait: wait_forever_stops stops loudly (the deadlock)', async () => {
  const { host, out } = await start({ TW_TEST_ONLY: 'wait_forever_stops' });
  await assert.rejects(host.call('tw_test_main'), (e) => e.message.startsWith('tempwind: green: deadlock: '));
  const lines = out.stderr.split('\n');
  assert.ok(lines.includes('tempwind: green: thread 1 waits on its server wakeup pipe (a server wait) with no deadline'),
    `stderr: ${out.stderr}`);
  assert.ok(lines.some((l) => l.startsWith('tempwind: green: deadlock: ')), `stderr: ${out.stderr}`);
  assert.ok(!/^done /m.test(out.stdout), `stdout: ${out.stdout}`);
});
