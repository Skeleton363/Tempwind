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
const exitText = readFileSync(`${root}tests/pe/basic.exitcode`, 'utf8');
const EXIT_CODE = /^\d+\n?$/.test(exitText) ? Number(exitText.trim()) : NaN;

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

test('tests/pe/basic.exitcode holds one decimal exit code', () => {
  assert.ok(Number.isInteger(EXIT_CODE), `tests/pe/basic.exitcode is ${JSON.stringify(exitText)}`);
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
    assert.equal(r.status, EXIT_CODE);
  });

  test(`basic.exe in-process (${variant}): output, exit code, run-time translation taken`, async () => {
    const { aot, plan } = inputs(variant);
    const p = await Process.load({
      exe: new Uint8Array(readFileSync(EXE)),
      plan: JSON.parse(readFileSync(plan, 'utf8')),
      aot: readFileSync(aot),
      rt: await RuntimeTranslator.load(readFileSync(RT)),
    });
    assert.equal(await p.start(), EXIT_CODE);
    assert.deepEqual(Buffer.from(p.captured('stdout')), expected);
    assert.equal(p.captured('stderr').length, 0);
    // `hidden` is reachable only through a computed pointer: AOT cannot find it.
    assert.ok(p.translations >= 1, `${p.translations} run-time translations`);
    console.log(`basic.exe (${variant}): ${p.translations} run-time translations, ${p.translateMs.toFixed(1)} ms`);
  });
}

test('basic.exe built with --counters: TEMPWIND_TRACE=counts reports the block and host calls', () => {
  const { aot, plan } = inputs('counters');
  const r = spawnSync(process.execPath, [CLI, '--exe', EXE, '--aot', aot, '--plan', plan, '--rt', RT], {
    env: { ...process.env, TEMPWIND_TRACE: 'counts' },
  });
  assert.equal(r.status, EXIT_CODE);
  assert.ok(r.stdout.equals(expected), 'counters do not change the output');
  const lines = Object.fromEntries(
    r.stderr.toString().split('\n').filter((l) => l.startsWith('[tempwind] ')).map((l) => {
      const [, kind, name, value] = l.split(' ');
      return [`${kind} ${name}`, Number(value)];
    }),
  );
  // rec(1000) and fib(20) (21,891 calls) are direct calls inside the module.
  assert.equal(lines['count calls_direct'], 22927);
  assert.ok(lines['count entries_fast'] > 0 && lines['count entries_generic'] > 0);
  // Since WP3 the structured layout takes loops without `$dispatch`, so dispatches may be 0; the slot is still reported.
  assert.ok(Number.isInteger(lines['count dispatches']) && lines['count unwinds'] > 0);
  // Since M6 (translator.md, "Registers and flags") a boundary moves only what it must: fewer values than a
  // full sync of all 20 at every entry, store and reload, but never none.
  const calls = lines['count calls_direct'] + lines['count calls_probed'];
  const entries = lines['count entries_fast'] + lines['count entries_generic'];
  const leaves = calls + entries + lines['count tail_transfers'] + lines['count unwinds'];
  assert.ok(lines['count values_loaded'] > 0 && lines['count values_loaded'] < 20 * (entries + calls), 'loads: live values only');
  assert.ok(lines['count values_stored'] > 0 && lines['count values_stored'] < 20 * leaves, 'stores: dirty values only');
  assert.ok(lines['count mem_loads'] > 0 && lines['count mem_stores'] > 0);
  assert.deepEqual(
    [lines['hostcalls kernel32.dll!ExitProcess'], lines['hostcalls kernel32.dll!GetStdHandle'], lines['hostcalls kernel32.dll!WriteFile']],
    [1, 22, 22],
  );
});

test('TEMPWIND_TRACE=counts without --counters says so and still counts host calls', () => {
  const { aot, plan } = inputs('all');
  const r = spawnSync(process.execPath, [CLI, '--exe', EXE, '--aot', aot, '--plan', plan, '--rt', RT], {
    env: { ...process.env, TEMPWIND_TRACE: 'counts' },
  });
  const err = r.stderr.toString();
  assert.match(err, /count \(no module was built with --counters\)/);
  assert.match(err, /hostcalls kernel32\.dll!WriteFile 22/);
});
