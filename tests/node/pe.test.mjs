// M2 integration: target/pe/basic.exe through the runtime, with AOT from all
// entries and from the entry point only, plus run-time translation. Inputs:
// tests/pe/build.sh, then tests/pe/prepare.sh (translate-pe both variants and
// build x64wasm_rt.wasm).
import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { Process } from '../../runtime/src/process.mjs';
import { RuntimeTranslator } from '../../runtime/src/rt.mjs';

const root = new URL('../../', import.meta.url).pathname;
const CLI = `${root}runtime/src/run-pe.mjs`;
const EXE = `${root}target/pe/basic.exe`;
const RT = `${root}target/wasm32-unknown-unknown/release/x64wasm_rt.wasm`;
const expected = readFileSync(`${root}tests/pe/basic.expected`);
const readme = readFileSync(`${root}tests/pe/README.md`, 'utf8');
const exitMatch = /^Exit code: \*\*(\d+)\*\*/m.exec(readme);

const need = (path, how) => assert.ok(existsSync(path), `${path} is missing. Create it with: ${how}`);
const inputs = (variant) => {
  need(EXE, 'tests/pe/build.sh');
  need(RT, 'tests/pe/prepare.sh');
  const aot = `${root}target/pe/${variant}/basic.wasm`;
  const plan = `${root}target/pe/${variant}/basic.plan.json`;
  need(aot, 'tests/pe/prepare.sh');
  need(plan, 'tests/pe/prepare.sh');
  return { aot, plan };
};

test('tests/pe/README.md documents the exit code', () => {
  assert.ok(exitMatch, 'tests/pe/README.md has no "Exit code: **N**" line');
});

for (const variant of ['all', 'entry']) {
  test(`basic.exe via run-pe, AOT from ${variant === 'all' ? 'all entries' : 'the entry point'}`, () => {
    const { aot, plan } = inputs(variant);
    const r = spawnSync(process.execPath, [CLI, '--exe', EXE, '--aot', aot, '--plan', plan, '--rt', RT], {
      env: { ...process.env, TEMPWIND_TRACE: '' },
    });
    assert.equal(r.stderr.toString(), '');
    assert.equal(r.stdout.toString(), expected.toString());
    assert.ok(r.stdout.equals(expected), 'stdout matches byte for byte');
    assert.equal(r.status, Number(exitMatch[1]));
  });

  test(`basic.exe in-process (${variant}): output, exit code, run-time translation taken`, async () => {
    const { aot, plan } = inputs(variant);
    const p = await Process.load({
      exe: new Uint8Array(readFileSync(EXE)),
      plan: JSON.parse(readFileSync(plan, 'utf8')),
      aot: readFileSync(aot),
      rt: await RuntimeTranslator.load(readFileSync(RT)),
    });
    assert.equal(await p.start(), Number(exitMatch[1]));
    assert.deepEqual(Buffer.from(p.captured('stdout')), expected);
    assert.equal(p.captured('stderr').length, 0);
    // `hidden` is reachable only through a computed pointer: AOT cannot find it.
    assert.ok(p.translations >= 1, `${p.translations} run-time translations`);
  });
}
