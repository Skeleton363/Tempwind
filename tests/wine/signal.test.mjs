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
import { startHarness, runChecks } from './harness-imports.mjs';

/** A fresh instance of test-signal. */
const start = (env = {}) => startHarness('signal', { env });

test('signal: the harness checks pass', async () => {
  const checks = ['suspend_other', 'self_suspend', 'context_other', 'mask_blocks_sigusr1',
    'suspended_start', 'apc_in_alertable_wait'];
  const h = await start();
  await runChecks(h, checks);
  const { out } = h;
  assert.ok(!/^tempwind:/m.test(out.stderr), `stderr: ${out.stderr}`);
});
