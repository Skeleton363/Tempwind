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
import { startHarness, runChecks } from './harness-imports.mjs';

/** A fresh instance of test-session. */
const start = (env = {}) => startHarness('session', { env, imports: true });

// In the harness's order (it runs them in one instance).
const CHECKS = ['view_start', 'view_grown_block'];

test('session: the harness checks pass', async () => {
  const h = await start();
  await runChecks(h, CHECKS);
  const { out } = h;
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
