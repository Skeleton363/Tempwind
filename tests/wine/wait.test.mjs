// The `wait` checks (docs/design/wine-unix.md, section 10): waits on one
// thread through the in-process server and green.c, scheduled by the JS host
// (runtime/src/wine/unix-host.mjs). The harness is native/wine/tests/wait.c;
// the loud stop (a wait nothing can end) runs alone in a fresh instance.
// Needs native/wine/build.sh's output in target/wine-unix (or TW_WINE_UNIX_DIR)
// and l_intl.nls from tests/wine/prepare.sh's target/wine (or TW_WINE_DIR).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { startHarness, runChecks } from './harness-imports.mjs';

/** A fresh instance of test-wait. */
const start = (env = {}) => startHarness('wait', { env, dosdevices: false });

const CHECKS = ['event_timeout', 'event_signaled', 'timer_fires', 'zero_timeout_polls',
  'delay_not_alertable', 'delay_alertable'];

test('wait: the harness checks pass', async () => {
  await runChecks(await start(), CHECKS);
});

// Alone (section 10): nothing is runnable and no timeout is pending, so the
// scheduler stops the waiting thread, after C named its wait on stderr.
test('wait: wait_forever_stops stops loudly (the deadlock)', async () => {
  const { host, out } = await start({ TW_TEST_ONLY: 'wait_forever_stops' });
  await assert.rejects(host.call('tw_test_main'), (e) => e.message.startsWith('tempwind: green: deadlock: '));
  // C's waiter lists still point into the dead frames: no call may enter.
  await assert.rejects(host.call('tw_test_main'), (e) => e.message.startsWith('tempwind: green: stopped after a deadlock'));
  const lines = out.stderr.split('\n');
  assert.ok(lines.includes('tempwind: green: thread 1 waits on its server wakeup pipe (a server wait) with no deadline'),
    `stderr: ${out.stderr}`);
  assert.ok(lines.some((l) => l.startsWith('tempwind: green: deadlock: ')), `stderr: ${out.stderr}`);
  assert.ok(!/^done /m.test(out.stdout), `stdout: ${out.stdout}`);
});
