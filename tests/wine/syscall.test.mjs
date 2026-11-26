// The `syscall` checks (docs/design/wine-unix.md, section 10, "The M3
// checks"): the syscall and unix-call dispatchers and the context functions
// of section 11, on synthetic state blocks, with no guest code. The harness
// is native/wine/tests/syscall.c; it starts the in-process server, so the
// prefix directory and l_intl.nls go into MEMFS first. Checks that expect a
// loud stop or an _exit run alone in a fresh instance (TW_TEST_ONLY), and so
// does the +syscall trace, with WINEDEBUG.
// Needs native/wine/build.sh's output in target/wine-unix (or TW_WINE_UNIX_DIR)
// and tests/wine/prepare.sh's target/wine (or TW_WINE_DIR) for l_intl.nls.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { startHarness, runChecks } from './harness-imports.mjs';

/** A fresh instance of test-syscall. */
const start = (env = {}) => startHarness('syscall', { env, imports: true });

// In the harness's order (it runs them in one instance). Then
// table 1 before and after win32u's init registers it, and that init as a
// unix call to win32u's table (section 10, "The M4 checks").
const CHECKS = ['dispatcher_page', 'stack_arguments', 'variant_entries', 'bad_id', 'nt_continue',
  'context_round_trip', 'unix_call', 'kuser_clock', 'syscall_trace',
  'table1_before_init', 'win32u_unix_call_init', 'table1_after_init'];

test('syscall: the harness checks pass', async () => {
  const h = await start();
  await runChecks(h, CHECKS);
  const { out } = h;
  assert.ok(!/SysCall|SysRet/.test(out.stderr), 'no +syscall lines without WINEDEBUG');
});

// Wine's +syscall format (unix/loader.c L636-664): "SysCall  NAME(ARGS)" and
// "SysRet   NAME() retval=STATUS", after the debug header (the thread id).
test('syscall: +syscall lines in Wine\'s format', async () => {
  const { host, out } = await start({ WINEDEBUG: '+syscall', TW_TEST_ONLY: 'syscall_trace' });
  assert.equal(await host.call('tw_test_main'), 0, out.stdout + out.stderr);
  assert.match(out.stderr, /^(?:[0-9a-f]{4}:)*SysCall  NtClose\(0000dead\)$/m, out.stderr);
  assert.match(out.stderr, /^(?:[0-9a-f]{4}:)*SysRet   NtClose\(\) retval=c0000008$/m, out.stderr);
});

// Each loud stop alone, in a fresh instance (section 10).
for (const [check, line] of [
  ['stub_stops', 'tempwind: unsupported: syscall NtTraceEvent'],
  ['unix_call_bad_code', 'tempwind: unsupported: tw_unix_call_dispatcher: a code past ntdll\'s unix calls'],
  // Process exit's hook in NtTerminateProcess (section 8): outside boot it stops.
  ['terminate_reaches_exit_hook', 'tempwind: unsupported: tw_process_exit outside boot'],
  // A table-1 stub once table 1 is registered, and a code past
  // win32u's one unix call.
  ['table1_stub_stops', 'tempwind: unsupported: win32u syscall NtBindCompositionSurface'],
  ['win32u_unix_call_bad_code', 'tempwind: unsupported: tw_unix_call_dispatcher: a code past the unix library\'s calls'],
]) {
  test(`syscall: ${check} stops loudly`, async () => {
    const { host, out } = await start({ TW_TEST_ONLY: check });
    await assert.rejects(host.call('tw_test_main'));
    assert.ok(out.stderr.split('\n').includes(line), `stderr: ${out.stderr}`);
    assert.ok(!/^done /m.test(out.stdout), `stdout: ${out.stdout}`);
  });
}

// abort_thread of the last thread is Wine's end for errors (section 8):
// abort_process keeps its _exit, so boot's caller sees a rejection with
// Emscripten's ExitStatus, not the exit hook. Emscripten's Node quit_ also
// sets process.exitCode, which this test restores.
test('syscall: abort_thread ends in _exit with its status, not in the exit hook', async () => {
  const saved = process.exitCode;
  try {
    const { host, out } = await start({ TW_TEST_ONLY: 'abort_thread_exits' });
    await assert.rejects(host.call('tw_test_main'), (err) => {
      assert.equal(err?.name, 'ExitStatus', `${err?.stack ?? err}\n${out.stderr}`);
      assert.equal(err.status, 1);
      return true;
    });
    assert.ok(!/^tempwind:/m.test(out.stderr), `stderr: ${out.stderr}`);
    assert.ok(!/^done /m.test(out.stdout), `stdout: ${out.stdout}`);
  } finally {
    process.exitCode = saved;
  }
});
