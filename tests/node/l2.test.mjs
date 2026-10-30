// M3 L2 programs (runtime.md, "M3 L2 programs"): target/native/l2_int.exe and
// l2_crt.exe translated (AOT from all entries, plus run-time translation)
// and run in Node through the msvcrt and kernel32 shim. Their raw stdout is
// msvcrt's text mode, so it must equal the .expected text with LF -> CR LF;
// the exit code must equal the .exitcode file. Inputs: tests/native/build.sh,
// tests/native/prepare.sh, and the run-time translator (tests/pe/prepare.sh).
import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

const root = new URL('../../', import.meta.url).pathname;
const CLI = `${root}runtime/src/run-pe.mjs`;
const RT = `${root}target/wasm32-unknown-unknown/release/x64wasm_rt.wasm`;
const need = (path, how) => assert.ok(existsSync(path), `${path} is missing. Create it with: ${how}`);

for (const name of ['l2_int', 'l2_crt']) {
  test(`${name}.exe translated: raw stdout (text mode) and exit code match the native run`, () => {
    const exe = `${root}target/native/${name}.exe`;
    const aot = `${root}target/native/${name}/${name}.wasm`;
    const plan = `${root}target/native/${name}/${name}.plan.json`;
    need(exe, 'tests/native/build.sh');
    need(aot, 'tests/native/prepare.sh');
    need(plan, 'tests/native/prepare.sh');
    need(RT, 'tests/pe/prepare.sh');
    const expected = Buffer.from(readFileSync(`${root}tests/native/${name}.expected`, 'latin1').replace(/\n/g, '\r\n'), 'latin1');
    const exitText = readFileSync(`${root}tests/native/${name}.exitcode`, 'utf8');
    assert.match(exitText, /^\d+\n?$/, `tests/native/${name}.exitcode holds one decimal exit code`);
    const r = spawnSync(process.execPath, [CLI, '--exe', exe, '--aot', aot, '--plan', plan, '--rt', RT], {
      env: { ...process.env, TEMPWIND_TRACE: '' },
      maxBuffer: 1 << 24,
    });
    assert.equal(r.stderr.toString(), '', 'no runtime error and nothing on stderr');
    assert.equal(r.stdout.toString('latin1'), expected.toString('latin1'));
    assert.ok(r.stdout.equals(expected), 'stdout matches byte for byte');
    assert.equal(r.status, Number(exitText.trim()));
  });
}
