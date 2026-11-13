// The `link` check (docs/design/wine-unix.md, section 10): the Unix side
// compiles and instantiates in Node with the 4 GiB memory, its imports are a
// subset of native/wine/imports.txt, a raw export runs; then the `link`
// harness (native/wine/tests/link.c), with its loud stops run alone.
// Needs native/wine/build.sh's output in target/wine-unix (or TW_WINE_UNIX_DIR).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { createUnixHost, INITIAL_PAGES } from '../../runtime/src/wine/unix-host.mjs';

const root = new URL('../../', import.meta.url);
const dir = process.env.TW_WINE_UNIX_DIR
  ? pathToFileURL(process.env.TW_WINE_UNIX_DIR.replace(/\/?$/, '/'))
  : new URL('target/wine-unix/', root);

const HOST_REGION = [0x8000_0000n, 0x1_0000_0000n];
const FOUR_GIB = 4 * 1024 ** 3;

// abi.md v5, "Host entries of the Unix side": the raw exports.
const HOST_ENTRIES = ['malloc', 'free', 'tw_page_flags', 'tw_main_state', 'tw_next_timeout',
  'tw_run_timeouts', 'emscripten_stack_set_limits', '_emscripten_stack_restore'];

const listed = new Set(readFileSync(new URL('native/wine/imports.txt', root), 'utf8').split('\n')
  .map((l) => l.replace(/#.*/, '').trim()).filter(Boolean).map((l) => l.split(/\s+/)[0]));

const modules = new Map();
async function load(name) {
  if (!modules.has(name)) {
    const { default: factory } = await import(new URL(`${name}.mjs`, dir));
    const wasm = new WebAssembly.Module(readFileSync(new URL(`${name}.wasm`, dir)));
    modules.set(name, { factory, wasm });
  }
  return modules.get(name);
}

/** A fresh instance of `name` with its stdout and stderr as text. */
async function start(name, opts = {}) {
  const out = { stdout: '', stderr: '', grown: 0, blocked: [] };
  const dec = () => { const d = new TextDecoder(); return (b) => d.decode(b, { stream: true }); };
  const o = dec(), e = dec();
  const host = await createUnixHost({
    ...(await load(name)),
    stdout: (b) => { out.stdout += o(b); },
    stderr: (b) => { out.stderr += e(b); },
    onMemoryGrown: () => { out.grown++; },
    // The scheduler's stand-in: resume a blocked thread with its id, later.
    greenBlock: (thread) => { out.blocked.push(thread); return new Promise((r) => setTimeout(() => r(BigInt(thread)), 1)); },
    ...opts,
  });
  return { host, out };
}

test('link: wine-unix instantiates with 4 GiB, imports listed, raw malloc/free', async () => {
  const { wasm } = await load('wine-unix');
  for (const name of ['wine-unix', 'test-link']) {
    const imports = WebAssembly.Module.imports((await load(name)).wasm).map((i) => `${i.module}.${i.name}`);
    assert.deepEqual(imports.filter((i) => !listed.has(i)), [], `${name}: imports not in imports.txt`);
  }
  const mem = WebAssembly.Module.imports(wasm).find((i) => i.module === 'env' && i.name === 'memory');
  assert.equal(mem?.kind, 'memory');

  const { host } = await start('wine-unix');
  assert.equal(host.memory.buffer.byteLength, FOUR_GIB);
  assert.equal(host.memory.buffer.byteLength / 65536, INITIAL_PAGES);
  for (const e of ['boot', ...HOST_ENTRIES]) assert.equal(typeof host.raw[e], 'function', `raw export ${e}`);

  const p = host.raw.malloc(4096n);
  assert.equal(typeof p, 'bigint');
  assert.ok(p >= HOST_REGION[0] && p + 4096n <= HOST_REGION[1], `malloc in the host region: 0x${p.toString(16)}`);
  new DataView(host.memory.buffer).setBigUint64(Number(p), 0x1122334455667788n, true);
  host.raw.free(p);
  assert.equal(host.raw.malloc(4096n), p, 'free gave the block back');

  // wine-unix.md, section 8: ENV and the stdio devices.
  assert.equal(host.Module.ENV.WINEPREFIX, '/home/runner/wine-prefix');
  assert.equal(host.Module.ENV.TZ, 'UTC');
  for (const fd of [0, 1, 2]) {
    const s = host.FS.getStream(fd);
    assert.ok(host.FS.isChrdev(s.node.mode), `fd ${fd} is a character device`);
    assert.ok(!s.tty, `fd ${fd} is not a tty`);
  }
});

const CHECKS = ['malloc_host_region', 'heap_cap', 'grow_to_then_heap_cap', 'mutex_recursive',
  'mutex_normal_errorcheck', 'once_keys_self', 'cond_without_waiters', 'green_switch_resumes'];

test('link: the harness checks pass', async () => {
  const { host, out } = await start('test-link');
  const ret = await host.call('tw_test_main');
  const lines = out.stdout.trim().split('\n');
  assert.deepEqual(lines.filter((l) => l.startsWith('not ok')), [], out.stdout + out.stderr);
  assert.deepEqual(lines.filter((l) => l.startsWith('ok ')).map((l) => l.slice(3)), CHECKS);
  assert.equal(lines.at(-1), `done ${CHECKS.length}/${CHECKS.length}`);
  assert.equal(ret, 0);
  assert.equal(out.grown, 1, 'grow_to called memory_grown once');
  assert.equal(host.memory.buffer.byteLength, FOUR_GIB + 65536);
  assert.deepEqual(out.blocked, [1], 'green_switch blocked thread 1 once');
});

// Each loud stop alone, in a fresh instance (section 10).
for (const [check, line] of [
  ['pthread_create_stops', 'tempwind: unsupported: pthread_create'],
  ['suspend_holding_mutex', 'tempwind: green: thread 1 suspends while holding 1 mutexes'],
]) {
  test(`link: ${check} stops loudly`, async () => {
    const { host, out } = await start('test-link', { env: { TW_TEST_ONLY: check } });
    await assert.rejects(host.call('tw_test_main'));
    assert.ok(out.stderr.split('\n').includes(line), `stderr: ${out.stderr}`);
    assert.ok(!/^done /m.test(out.stdout), `stdout: ${out.stdout}`);
    assert.deepEqual(out.blocked, [], 'never reached green_block');
  });
}
