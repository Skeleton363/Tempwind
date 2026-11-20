// The `syscall` checks (docs/design/wine-unix.md, section 10, "The M3
// checks"): the syscall and unix-call dispatchers and the context functions
// of section 11, on synthetic state blocks, with no guest code. The harness
// is native/wine/tests/syscall.c; it starts the in-process server, so the
// prefix directory and l_intl.nls go into MEMFS first. Checks that expect a
// loud stop run alone in a fresh instance (TW_TEST_ONLY), and so does the
// +syscall trace, with WINEDEBUG.
// Needs native/wine/build.sh's output in target/wine-unix (or TW_WINE_UNIX_DIR)
// and tests/wine/prepare.sh's target/wine (or TW_WINE_DIR) for l_intl.nls.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { createUnixHost, DEFAULT_ENV } from '../../runtime/src/wine/unix-host.mjs';
import { m3Imports } from './m3-imports.mjs';

const root = new URL('../../', import.meta.url);
const dirUrl = (env, fallback) => (process.env[env]
  ? pathToFileURL(process.env[env].replace(/\/?$/, '/'))
  : new URL(fallback, root));
const unixDir = dirUrl('TW_WINE_UNIX_DIR', 'target/wine-unix/');
const wineDir = dirUrl('TW_WINE_DIR', 'target/wine/');

const NLS_DIR = '/wine/share/wine/nls';
const intl = readFileSync(new URL('native/opt/wine-11.0/share/wine/nls/l_intl.nls', wineDir));

const { default: factory } = await import(new URL('test-syscall.mjs', unixDir));
const wasm = new WebAssembly.Module(readFileSync(new URL('test-syscall.wasm', unixDir)));

async function start(env = {}) {
  const out = { stdout: '', stderr: '' };
  const dec = () => { const d = new TextDecoder(); return (b) => d.decode(b, { stream: true }); };
  const o = dec(), e = dec();
  const m3 = m3Imports();
  const host = await createUnixHost({
    factory, wasm, env,
    stdout: (b) => { out.stdout += o(b); },
    stderr: (b) => { out.stderr += e(b); },
    imports: m3.imports,
  });
  m3.attach(host);
  host.FS.mkdirTree(`${DEFAULT_ENV.WINEPREFIX}/dosdevices`);
  host.FS.mkdirTree(NLS_DIR);
  host.FS.writeFile(`${NLS_DIR}/l_intl.nls`, intl);
  return { host, out };
}

// In the harness's order (it runs them in one instance).
const CHECKS = ['dispatcher_page', 'stack_arguments', 'variant_entries', 'bad_id', 'nt_continue',
  'context_round_trip', 'unix_call', 'kuser_clock', 'syscall_trace'];

test('syscall: the harness checks pass', async () => {
  const { host, out } = await start();
  const ret = await host.call('tw_test_main');
  const lines = out.stdout.trim().split('\n');
  assert.deepEqual(lines.filter((l) => l.startsWith('not ok')), [], out.stdout + out.stderr);
  assert.deepEqual(lines.filter((l) => l.startsWith('ok ')).map((l) => l.slice(3)), CHECKS);
  assert.equal(lines.at(-1), `done ${CHECKS.length}/${CHECKS.length}`);
  assert.equal(ret, 0);
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
]) {
  test(`syscall: ${check} stops loudly`, async () => {
    const { host, out } = await start({ TW_TEST_ONLY: check });
    await assert.rejects(host.call('tw_test_main'));
    assert.ok(out.stderr.split('\n').includes(line), `stderr: ${out.stderr}`);
    assert.ok(!/^done /m.test(out.stdout), `stdout: ${out.stdout}`);
  });
}
