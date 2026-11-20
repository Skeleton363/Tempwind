// The programs (docs/design/wine-unix.md, section 10, "Programs (M3)"):
// hello.exe, l2_int, l2_crt and l2_float, each in a fresh Unix-side instance
// in run mode, with both memory checks on: the catalog's modules are built
// with --shared --null-check --store-check (parseCatalog refuses others) and
// run-time translations use the NULL, store and shared-memory flags
// (WINE_RT_FLAGS).
//
// Each must match native: stdout equals tests/native/<name>.expected with
// CRLF line ends (msvcrt's text mode), the exit status equals
// <name>.exitcode (0 without one), and no `tempwind:` line appears on stderr.
//
// Needs target/wine-unix (TW_WINE_UNIX_DIR), target/wine prepared by
// tests/wine/prepare.sh (TW_WINE_DIR: the programs, the catalog and
// boot-set.json), and the run-time translator and floating-point helper
// (target/wasm32-unknown-unknown/release, or TW_RT_WASM and TW_FP_WASM).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { loadWineInputs, wineDirs } from '../../runtime/src/wine/node-files.mjs';
import { createWineProcess, WINE_RT_FLAGS } from '../../runtime/src/wine/wine-process.mjs';
import { RT_FLAG_NULL_CHECK, RT_FLAG_STORE_CHECK, RT_FLAG_SHARED_MEMORY } from '../../runtime/src/abi.mjs';

const PROGRAMS = ['hello', 'l2_int', 'l2_crt', 'l2_float'];
const NATIVE = new URL('../native/', import.meta.url);

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

    const lines = out.stderr.split('\n').filter((l) => l.startsWith('tempwind:'));
    assert.deepEqual(lines, [], out.stderr);
    assert.equal(error, undefined, `the process stopped: ${error?.message}\n${out.stderr}`);
    const expected = readFileSync(new URL(`${name}.expected`, NATIVE), 'utf8').replace(/\n/g, '\r\n');
    assert.equal(out.stdout, expected);
    const code = new URL(`${name}.exitcode`, NATIVE);
    const want = existsSync(code) ? Number(readFileSync(code, 'utf8').trim()) : 0;
    assert.equal(status, want, `exit status 0x${status?.toString(16)}`);
  });
}
