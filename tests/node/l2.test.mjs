// M3 and M4 programs (runtime.md, "M3 L2 programs", "M4 programs"): target/native/hello.exe,
// l2_int.exe, l2_crt.exe and l2_float.exe, and CoreMark, translated (AOT from all entries, plus run-time translation)
// and run in Node through the msvcrt and kernel32 shim. Their raw stdout is
// msvcrt's text mode, so it must equal the .expected text with LF -> CR LF;
// the exit code must equal the .exitcode file. Inputs: tests/native/build.sh,
// tests/native/prepare.sh (which also makes coremark.exe, x64wasm_fp.wasm), and the run-time
// translator (tests/pe/prepare.sh).
import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

const root = new URL('../../', import.meta.url).pathname;
const CLI = `${root}runtime/src/run-pe.mjs`;
const RT = `${root}target/wasm32-unknown-unknown/release/x64wasm_rt.wasm`;
const FP = `${root}target/wasm32-unknown-unknown/release/x64wasm_fp.wasm`;
const need = (path, how) => assert.ok(existsSync(path), `${path} is missing. Create it with: ${how}`);

for (const name of ['hello', 'l2_int', 'l2_crt', 'l2_float']) {
  test(`${name}.exe translated: raw stdout (text mode) and exit code match the native run`, () => {
    const exe = `${root}target/native/${name}.exe`;
    const aot = `${root}target/native/${name}/${name}.wasm`;
    const plan = `${root}target/native/${name}/${name}.plan.json`;
    need(exe, 'tests/native/build.sh');
    need(aot, 'tests/native/prepare.sh');
    need(plan, 'tests/native/prepare.sh');
    need(RT, 'tests/pe/prepare.sh');
    need(FP, 'tests/native/prepare.sh');
    const expected = Buffer.from(readFileSync(`${root}tests/native/${name}.expected`, 'latin1').replace(/\n/g, '\r\n'), 'latin1');
    // hello.exe has no .exitcode file: it exits 0.
    const exitFile = `${root}tests/native/${name}.exitcode`;
    const exitText = existsSync(exitFile) ? readFileSync(exitFile, 'utf8') : '0\n';
    assert.match(exitText, /^\d+\n?$/, `tests/native/${name}.exitcode holds one decimal exit code`);
    const r = spawnSync(process.execPath, [CLI, '--exe', exe, '--aot', aot, '--plan', plan, '--rt', RT, '--fp', FP], {
      env: { ...process.env, TEMPWIND_TRACE: '' },
      maxBuffer: 1 << 24,
    });
    assert.equal(r.stderr.toString(), '', 'no runtime error and nothing on stderr');
    assert.equal(r.stdout.toString('latin1'), expected.toString('latin1'));
    assert.ok(r.stdout.equals(expected), 'stdout matches byte for byte');
    assert.equal(r.status, Number(exitText.trim()));
  });
}

test('coremark.exe translated: run type, the four CRCs and only the 10 second complaint (tests/targets/smoke.sh)', () => {
  const exe = `${root}target/targets/coremark.exe`;
  const aot = `${root}target/targets/coremark/coremark.wasm`;
  const plan = `${root}target/targets/coremark/coremark.plan.json`;
  need(exe, 'scripts/fetch-artifact.sh "$(cat tests/targets/targets.lock)" coremark.exe target/targets');
  need(aot, 'tests/native/prepare.sh');
  need(plan, 'tests/native/prepare.sh');
  need(RT, 'tests/pe/prepare.sh');
  need(FP, 'tests/native/prepare.sh');
  const r = spawnSync(process.execPath, [CLI, '--exe', exe, '--aot', aot, '--plan', plan, '--rt', RT, '--fp', FP,
    '--', '0x0', '0x0', '0x66', '200'], { env: { ...process.env, TEMPWIND_TRACE: '' }, maxBuffer: 1 << 24 });
  const out = r.stdout.toString('latin1');
  assert.equal(r.stderr.toString(), '', 'no runtime error and nothing on stderr');
  for (const line of ['2K performance run parameters for coremark', 'seedcrc          : 0xe9f5',
    '[0]crclist       : 0xe714', '[0]crcmatrix     : 0x1fd7', '[0]crcstate      : 0x8e3a']) {
    assert.ok(out.includes(line), `output has "${line}":\n${out}`);
  }
  assert.ok(!/ERROR! .* crc/.test(out), 'no wrong crc');
  assert.equal(out.split('\n').filter((l) => l.includes('ERROR')).length, 1, 'exactly one ERROR line');
  assert.ok(out.includes('ERROR! Must execute for at least 10 secs'), 'the 10 second complaint');
});
