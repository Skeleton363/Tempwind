// The programs (docs/design/wine-unix.md, section 10, "Programs (M3)"):
// hello.exe, l2_int, l2_crt and l2_float, basic.exe, l2_threads and l2_suspend, each in a fresh Unix-side instance
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
import { PE_DIR } from '../../runtime/src/wine/boot-set.mjs';
import { RT_FLAG_NULL_CHECK, RT_FLAG_STORE_CHECK, RT_FLAG_SHARED_MEMORY } from '../../runtime/src/abi.mjs';

const NATIVE = new URL('../native/', import.meta.url);
const dll = (...names) => names.map((n) => `${PE_DIR}/${n}`);
/** The modules our boot maps for a program besides itself (section 10; closure.json's list can be longer, see there). */
const CRT_DLLS = dll('ntdll.dll', 'kernel32.dll', 'kernelbase.dll', 'msvcrt.dll');
const BASIC_DLLS = dll('ntdll.dll', 'kernelbase.dll', 'kernel32.dll'); // basic.exe's static closure: no msvcrt
/**
 * The programs. `expected`/`exitcode` come from tests/native (`dir` set: from that directory, as raw bytes, since
 * basic.exe writes with WriteFile and so no CRLF translation happens).
 */
const PROGRAMS = [
  { name: 'hello', dlls: CRT_DLLS },
  { name: 'l2_int', dlls: CRT_DLLS },
  { name: 'l2_crt', dlls: CRT_DLLS },
  { name: 'l2_float', dlls: CRT_DLLS },
  { name: 'l2_threads', dlls: CRT_DLLS }, // green threads (wine-unix.md v3, section 9)
  { name: 'l2_suspend', dlls: CRT_DLLS }, // suspension, contexts and user APCs (sections 9 and 11)
  { name: 'basic', dlls: BASIC_DLLS, expectedDir: new URL('../pe/', import.meta.url), crlf: false },
];

const dirs = wineDirs();
const inputs = await loadWineInputs(dirs); // compiled once; each program gets its own instance

test('both memory checks are on in run-time translations', () => {
  const want = RT_FLAG_NULL_CHECK | RT_FLAG_STORE_CHECK | RT_FLAG_SHARED_MEMORY;
  assert.equal(WINE_RT_FLAGS & want, want);
});

/**
 * Runs target/wine's `file` on its own instance and checks what every run
 * must show; returns its stdout and exit status.
 */
async function runOnWine(t, file, { args, dlls }) {
  const out = { stdout: '', stderr: '' };
  const o = new TextDecoder(), e = new TextDecoder();
  const proc = await createWineProcess({
    ...inputs,
    program: { name: file, bytes: new Uint8Array(readFileSync(join(dirs.wine, file))) },
    args,
    stdout: (b) => { out.stdout += o.decode(b, { stream: true }); },
    stderr: (b) => { out.stderr += e.decode(b, { stream: true }); },
  });
  let status, error;
  try { status = await proc.run(); } catch (err) { error = err; }
  const s = proc.stats;
  t.diagnostic(`${file}: boot ${s.bootMs.toFixed(0)} ms; AOT links ${s.aot.length}; fallbacks: ${s.imageFallbacks.length} images, ` +
    `${s.translations.length} run-time translations; lazy nodes loaded ${s.lazy.files} files, ${s.lazy.bytes} bytes`);
  assert.deepEqual(out.stderr.split('\n').filter((l) => l.startsWith('tempwind:')), [], out.stderr);
  assert.equal(error, undefined, `the process stopped: ${error?.message}\n${out.stderr}`);
  // Every image mapped links its AOT module (wine-build.md, "The catalog"):
  // the program and the DLLs our boot maps for it (the program's list above;
  // apisetschema is data). Run-time translations stay allowed: they cover
  // code the AOT pass didn't find, such as jump-table targets.
  assert.deepEqual(s.imageFallbacks, [], 'an image mapped without its AOT module');
  assert.deepEqual(s.aot.map((a) => a.path).sort(), [proc.programPath, ...dlls].sort());
  return { stdout: out.stdout, status };
}

for (const { name, dlls, expectedDir = NATIVE, crlf = true } of PROGRAMS) {
  test(`${name}.exe on Wine (run mode)`, async (t) => {
    const { stdout, status } = await runOnWine(t, `${name}.exe`, { dlls });
    const text = readFileSync(new URL(`${name}.expected`, expectedDir), 'utf8');
    assert.equal(stdout, crlf ? text.replace(/\n/g, '\r\n') : text);
    const code = new URL(`${name}.exitcode`, expectedDir);
    const want = existsSync(code) ? Number(readFileSync(code, 'utf8').trim()) : 0;
    assert.equal(status, want, `exit status 0x${status?.toString(16)}`);
  });
}

// CoreMark with a small fixed iteration count (section 10). 200 iterations take
// milliseconds, so CoreMark itself complains that it ran under 10 s: that is
// the one `ERROR!` line allowed. A wrong list, matrix or state CRC prints
// another; the seed CRC is the known one for the standard seeds 0x0 0x0 0x66
// (tests/targets/smoke.sh checks the same); the exit status is 0.
test('coremark.exe on Wine (run mode, 200 iterations)', async (t) => {
  const exe = join(dirs.wine, 'coremark.exe');
  assert.ok(existsSync(exe), `${exe} is missing: tests/wine/prepare.sh copies it from the targets release`);
  const { stdout, status } = await runOnWine(t, 'coremark.exe', { args: ['0x0', '0x0', '0x66', '200'], dlls: CRT_DLLS });
  const errors = stdout.split(/\r?\n/).filter((l) => l.includes('ERROR!'));
  assert.deepEqual(errors.map((l) => l.trim()), ['ERROR! Must execute for at least 10 secs for a valid result!'], stdout);
  assert.match(stdout, /seedcrc\s*: 0xe9f5/);
  assert.equal(status, 0, `exit status 0x${status?.toString(16)}`);
});
