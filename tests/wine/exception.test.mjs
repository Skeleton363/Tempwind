// The `exception` checks (docs/design/wine-unix.md v4, section 10, "The M5
// checks"): the raise path of section 11, "Exceptions", and the run loop's
// records of section 12, on synthetic state blocks with no guest code. The
// harness is native/wine/tests/exception.c; it starts the in-process server,
// so the prefix directory and l_intl.nls go into MEMFS first.
// - tw_run here returns the exit reason the state block holds and leaves
//   EXIT_HOST there (abi.md, "CPU state block": the field holds the value run
//   returns), so the harness's run loop acts on one exit and then returns.
// - The loud stops run alone in a fresh instance (TW_TEST_ONLY), and so does
//   the second chance, which ends the process.
// Needs native/wine/build.sh's output in target/wine-unix (or TW_WINE_UNIX_DIR)
// and tests/wine/prepare.sh's target/wine (or TW_WINE_DIR) for l_intl.nls.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { EXIT_HOST, STATE_EXIT } from '../../runtime/src/abi.mjs';
import { startHarness, runChecks } from './harness-imports.mjs';

/** A fresh instance of test-exception, with the one-exit tw_run. */
async function start(env = {}) {
  let host = null;
  const run = (state) => {
    const at = Number(BigInt.asUintN(64, state)) + STATE_EXIT;
    const dv = new DataView(host.memory.buffer);
    const exit = dv.getUint32(at, true);
    dv.setUint32(at, EXIT_HOST, true);
    return exit;
  };
  const h = await startHarness('exception', { env, imports: true, run });
  host = h.host;
  return h;
}

// In the harness's order (it runs them in one instance).
const CHECKS = ['layout', 'context', 'fault_records', 'execute_records', 'first_chance', 'stubs'];

test('exception: the harness checks pass', async () => {
  const h = await start();
  await runChecks(h, CHECKS);
  assert.ok(!/^tempwind:/m.test(h.out.stderr), `stderr: ${h.out.stderr}`);
});

// The loud stops that stay (section 12), each alone in a fresh instance.
for (const [check, line] of [
  ['code_write_writable_stops', /^tempwind: unsupported: run EXIT_FAULT rip=0x180003456 fault=0x[0-9a-f]+ kind=code-write .*: a store-check fault on a page Wine has writable$/m],
  ['unknown_kind_stops', /^tempwind: unsupported: run EXIT_FAULT rip=0x180003456 fault=0 kind=unknown .*: a fault kind the run loop doesn't know$/m],
  ['syscall_dispatcher_stops', /^tempwind: unsupported: run EXIT_LEFT_CODE rip=0x7ffe1100 .*: no slot at a dispatcher's address$/m],
  ['unix_call_dispatcher_stops', /^tempwind: unsupported: run EXIT_LEFT_CODE rip=0x7ffe1110 .*: no slot at a dispatcher's address$/m],
]) {
  test(`exception: ${check} stops loudly`, async () => {
    const { host, out } = await start({ TW_TEST_ONLY: check });
    await assert.rejects(host.call('tw_test_main'));
    assert.match(out.stderr, line);
    assert.ok(!/^done /m.test(out.stdout), `stdout: ${out.stdout}`);
  });
}

// The second chance ends the process with the exception code as its status
// (unix/thread.c L1618-1626), after Wine's ERR line and with no tempwind:
// line. It runs on a second thread, so the status arrives through
// TW_THREAD_PROCESS_EXIT.
test('exception: the second chance ends the process with the exception code', async () => {
  const { host, out } = await start({ TW_TEST_ONLY: 'second_chance' });
  const ret = await host.call('tw_test_main');
  assert.equal(ret >>> 0, 0xc0000005, out.stdout + out.stderr);
  assert.match(out.stderr, /err:seh:NtRaiseException Unhandled exception code c0000005 flags 0 addr 0x180003456$/m);
  assert.ok(!/^tempwind:/m.test(out.stderr), `stderr: ${out.stderr}`);
  assert.ok(!/^(?:not )?ok |^done /m.test(out.stdout), `stdout: ${out.stdout}`);
});
