// The `virtual` checks (docs/design/wine-unix.md, section 10): Wine's virtual
// memory on the wasm memory, without the server, through the `virtual`
// harness (native/wine/tests/virtual.c). Checks that expect a loud stop run
// alone in a fresh instance (TW_TEST_ONLY).
// Needs native/wine/build.sh's output in target/wine-unix (or TW_WINE_UNIX_DIR).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { createUnixHost } from '../../runtime/src/wine/unix-host.mjs';
import { harnessImports } from './harness-imports.mjs';

const root = new URL('../../', import.meta.url);
const dir = process.env.TW_WINE_UNIX_DIR
  ? pathToFileURL(process.env.TW_WINE_UNIX_DIR.replace(/\/?$/, '/'))
  : new URL('target/wine-unix/', root);

const FOUR_GIB = 4 * 1024 ** 3;

let compiled;
async function load() {
  if (!compiled) {
    const { default: factory } = await import(new URL('test-virtual.mjs', dir));
    const wasm = new WebAssembly.Module(readFileSync(new URL('test-virtual.wasm', dir)));
    compiled = { factory, wasm };
  }
  return compiled;
}

/** A fresh instance of test-virtual with its stdout and stderr as text. */
async function start(opts = {}) {
  const out = { stdout: '', stderr: '', grown: 0 };
  const dec = () => { const d = new TextDecoder(); return (b) => d.decode(b, { stream: true }); };
  const o = dec(), e = dec();
  const harness = harnessImports();
  const host = await createUnixHost({
    ...(await load()),
    stdout: (b) => { out.stdout += o(b); },
    stderr: (b) => { out.stderr += e(b); },
    onMemoryGrown: () => { out.grown++; },
    imports: harness.imports,
    ...opts,
  });
  harness.attach(host);
  return { host, out, harness };
}

// In the harness's order (it runs them in one instance).
const CHECKS = ['init_first_teb', 'kuser_committed', 'reserve_commit_free', 'fixed_high_grows', 'reserve_high_no_growth',
  'zero_on_recommit', 'protect_query', 'write_watch_page_watch', 'guest_regions_only',
  'heap_cap_after_growth', 'file_view_copy', 'file_view_write_back', 'pointer_validation', 'invalidate_code',
  'exec_range'];

test('virtual: the harness checks pass', async () => {
  const { host, out, harness } = await start();
  const ret = await host.call('tw_test_main');
  const lines = out.stdout.trim().split('\n');
  assert.deepEqual(lines.filter((l) => l.startsWith('not ok')), [], out.stdout + out.stderr);
  assert.deepEqual(lines.filter((l) => l.startsWith('ok ')).map((l) => l.slice(3)), CHECKS);
  assert.equal(lines.at(-1), `done ${CHECKS.length}/${CHECKS.length}`);
  assert.equal(ret, 0);
  // fixed_high_grows mapped at 0x1_4000_0000: the backend grew the memory
  // (section 5) and JS heard of it.
  assert.ok(out.grown >= 1, 'memory_grown was called');
  assert.ok(host.memory.buffer.byteLength >= 0x1_4002_0000, `memory: ${host.memory.buffer.byteLength}`);
  // tw_invalidate (section 6): exactly the calls invalidate_code expects, for
  // its PAGE_CODE pages, each while the range still held its marker; none
  // for any other unmap or remap of the whole run.
  const expected = lines.filter((l) => l.startsWith('# invalidate ')).map((l) => {
    const [, , base, size] = l.split(' ');
    return { base: BigInt(base), size: BigInt(size), marked: true };
  });
  assert.equal(expected.length, 2, out.stdout);
  assert.deepEqual(harness.invalidated, expected);
});

test('virtual: init alone leaves the memory at 4 GiB', async () => {
  const { host, out } = await start({ env: { TW_TEST_ONLY: 'init_first_teb' } });
  assert.equal(await host.call('tw_test_main'), 0, out.stdout + out.stderr);
  assert.equal(out.grown, 0);
  assert.equal(host.memory.buffer.byteLength, FOUR_GIB);
});

// Each loud stop alone, in a fresh instance (section 10).
for (const [check, line] of [
  ['second_writable_view', 'tempwind: unsupported: second writable view'],
]) {
  test(`virtual: ${check} stops loudly`, async () => {
    const { host, out } = await start({ env: { TW_TEST_ONLY: check } });
    await assert.rejects(host.call('tw_test_main'));
    assert.ok(out.stderr.split('\n').includes(line), `stderr: ${out.stderr}`);
    assert.ok(!/^done /m.test(out.stdout), `stdout: ${out.stdout}`);
  });
}
