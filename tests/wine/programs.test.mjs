// The programs (docs/design/wine-unix.md, section 10, "Programs (M3)"):
// hello.exe, l2_int, l2_crt and l2_float, each in a fresh Unix-side instance
// in run mode, with both memory checks on: the catalog's modules are built
// with --shared --null-check --store-check (parseCatalog refuses others) and
// run-time translations use the NULL, store and shared-memory flags
// (WINE_RT_FLAGS).
//
// Until WP5 writes the run loop, each asserts the furthest point that is
// real: boot reaches signal_start_thread's loud stop at tw_run_loop, and
// every image mapped so far (the program and ntdll) was linked from the
// catalog, with no fallback. WP5 replaces those assertions with stdout
// against tests/native/<name>.expected (as CRLF), the exit status against
// <name>.exitcode (0 without one), and no `tempwind:` line.
//
// Needs target/wine-unix (TW_WINE_UNIX_DIR), target/wine prepared by
// tests/wine/prepare.sh (TW_WINE_DIR: the programs, the catalog and
// boot-set.json), and the run-time translator and floating-point helper
// (target/wasm32-unknown-unknown/release, or TW_RT_WASM and TW_FP_WASM).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { loadWineInputs, wineDirs } from '../../runtime/src/wine/node-files.mjs';
import { createWineProcess, WINE_RT_FLAGS } from '../../runtime/src/wine/wine-process.mjs';
import { RT_FLAG_NULL_CHECK, RT_FLAG_STORE_CHECK, RT_FLAG_SHARED_MEMORY } from '../../runtime/src/abi.mjs';

const PROGRAMS = ['hello', 'l2_int', 'l2_crt', 'l2_float'];
const RUN_LOOP_STOP = 'tempwind: unsupported: tw_run_loop (M3 WP5)';
const NTDLL = '/wine/lib/wine/x86_64-windows/ntdll.dll';

const dirs = wineDirs();
const inputs = await loadWineInputs(dirs); // compiled once; each program gets its own instance

test('both memory checks are on in run-time translations', () => {
  const want = RT_FLAG_NULL_CHECK | RT_FLAG_STORE_CHECK | RT_FLAG_SHARED_MEMORY;
  assert.equal(WINE_RT_FLAGS & want, want);
});

for (const name of PROGRAMS) {
  test(`${name}.exe on Wine (run mode)`, async (t) => {
    const file = `${name}.exe`;
    const out = { stdout: '', stderr: '' };
    const dec = () => { const d = new TextDecoder(); return (b) => d.decode(b, { stream: true }); };
    const o = dec(), e = dec();
    const proc = await createWineProcess({
      ...inputs,
      program: { name: file, bytes: new Uint8Array(readFileSync(join(dirs.wine, file))) },
      stdout: (b) => { out.stdout += o(b); },
      stderr: (b) => { out.stderr += e(b); },
    });
    let status, error;
    try {
      status = await proc.run();
    } catch (err) {
      error = err;
    }
    const s = proc.stats;
    t.diagnostic(`${file}: boot ${s.bootMs.toFixed(0)} ms; AOT links ${s.aot.length}; fallbacks: ${s.imageFallbacks.length} images, ` +
      `${s.translations.length} run-time translations; lazy nodes loaded ${s.lazy.files} files, ${s.lazy.bytes} bytes`);

    // The furthest point that is real until WP5's run loop.
    assert.equal(status, undefined, `the process exited with 0x${status?.toString(16)}: ${out.stderr}`);
    assert.ok(error, 'boot neither returned nor stopped');
    const lines = out.stderr.split('\n').filter((l) => l.startsWith('tempwind:'));
    assert.deepEqual(lines, [RUN_LOOP_STOP], out.stderr);
    assert.deepEqual(s.imageFallbacks, [], 'an image mapped without its AOT module');
    assert.deepEqual(s.translations, [], 'code translated at run time');
    assert.deepEqual(s.aot.map((a) => a.path), [proc.programPath, NTDLL]);
  });
}
