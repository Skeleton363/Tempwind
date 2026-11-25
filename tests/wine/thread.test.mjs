// The `thread` checks (docs/design/wine-unix.md v3, section 10, "The M4
// checks"): green threads made by Wine's own NtCreateThreadEx, scheduled by
// the JS host (runtime/src/wine/unix-host.mjs, section 9), their ends,
// process exit on a thread other than the main one (section 8) and SIGQUIT's
// kill (section 7). The harness is native/wine/tests/thread.c; it starts the
// in-process server, so the prefix directory and l_intl.nls go into MEMFS
// first. The checks that end the process run alone in a fresh instance
// (TW_TEST_ONLY), and the test asserts the status the process settled with.
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

const { default: factory } = await import(new URL('test-thread.mjs', unixDir));
const wasm = new WebAssembly.Module(readFileSync(new URL('test-thread.wasm', unixDir)));

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

// The main thread of the process-exit checks sleeps 60 s; the status must
// arrive well before that deadline (section 10).
const SLEEP_MS = 60000;
const WELL_BEFORE_MS = 10000;

const lines = (s) => s.split('\n').filter((l) => l);

test('thread: the harness checks pass', async () => {
  const { host, out } = await start();
  const ret = await host.call('tw_test_main');
  const got = lines(out.stdout);
  const checks = ['switch_two_threads', 'join_and_exit_order', 'sigquit_kills_sleeper'];
  assert.deepEqual(got.filter((l) => l.startsWith('not ok')), [], out.stdout + out.stderr);
  assert.deepEqual(got.filter((l) => l.startsWith('ok ')).map((l) => l.slice(3)), checks);
  assert.equal(got.at(-1), `done ${checks.length}/${checks.length}`);
  assert.equal(ret, 0);
  assert.ok(!/^tempwind:/m.test(out.stderr), `stderr: ${out.stderr}`);
});

// Each ends the process on a thread other than the main one: the process
// settles with the status from tw_exit_status (TW_THREAD_PROCESS_EXIT), the
// thread's report lines are all `ok`, and no `tempwind:` line appears (an
// exit with other threads alive never reaches process_sigkill, section 7).
for (const [check, status, reports] of [
  // the main thread sleeps 60 s; its SIGQUIT stays pending, never handled
  ['process_exit_from_thread', 0x1234, ['main_sleeps', 'main_sigquit_pending']],
  // the main thread's exit parked it; the other thread's exit settles the run
  ['main_exit_parks', 0x4321, ['main_parked', 'main_handle_signaled']],
  // the main thread asleep and a third thread in a server wait
  ['exit_with_two_alive', 0x5678, ['two_threads_alive']],
]) {
  test(`thread: ${check} settles the process with its status`, async () => {
    const { host, out } = await start({ TW_TEST_ONLY: check });
    const t0 = performance.now();
    const ret = await host.call('tw_test_main');
    const ms = performance.now() - t0;
    assert.equal(ret, status, out.stdout + out.stderr);
    assert.ok(ms < WELL_BEFORE_MS, `settled after ${ms} ms, not well before the ${SLEEP_MS} ms sleep`);
    assert.deepEqual(lines(out.stdout), reports.map((r) => `ok ${r}`), out.stdout + out.stderr);
    assert.ok(!/^tempwind:/m.test(out.stderr), `stderr: ${out.stderr}`);
    // the instance runs one process: nothing enters it again
    await assert.rejects(host.call('tw_test_main'), /tempwind: green: the process has ended/);
  });
}
