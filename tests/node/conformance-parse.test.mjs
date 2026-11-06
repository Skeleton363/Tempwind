// tests/conformance/parse.mjs on sample outputs in the format of
// include/wine/test.h (docs/design/wine-build.md, "Parsing").
import test from 'node:test';
import assert from 'node:assert/strict';
import { parseUnit, parseList } from '../conformance/parse.mjs';

const run = (output, exit, extra = {}) => parseUnit({ unit: 'file', output, exit, seconds: 1.5, ...extra });

test('ok: a summary line, no failures, exit 0', () => {
  const r = run('0042:file:0.123s noise\n0042:file: 1234 tests executed (3 marked as todo, 1 as flaky, 0 failures), 12 skipped.\n', 0);
  assert.deepEqual(r, { unit: 'file', status: 'ok', executed: 1234, todo: 3, flaky: 1, failures: 0, skipped: 12, exit: 0, seconds: 1.5 });
});

test('failed: failures in the summary', () => {
  const r = run('00a1:file:0.5s 90 tests executed (0 marked as todo, 0 as flaky, 4 failures), 0 skipped.\r\n', 4);
  assert.equal(r.status, 'failed');
  assert.equal(r.failures, 4);
  assert.equal(r.exit, 4);
});

test('failure in the singular', () => {
  const r = run('00a1:file: 10 tests executed (0 marked as todo, 0 as flaky, 1 failure), 0 skipped.\n', 1);
  assert.equal(r.status, 'failed');
  assert.equal(r.failures, 1);
});

test('crashed: no summary line', () => {
  const r = run('Test failed: something\nwine: Unhandled page fault\n', 1);
  assert.equal(r.status, 'crashed');
  assert.equal(r.executed, 0);
});

test('crashed: an exception exit code, even with a summary line', () => {
  const r = run('0042:file: 5 tests executed (0 marked as todo, 0 as flaky, 0 failures), 0 skipped.\n', 0xc0000005);
  assert.equal(r.status, 'crashed');
  assert.equal(r.exit, 0xc0000005);
  // Node reports a negative number for it on some hosts.
  assert.equal(run('', -1073741819).status, 'crashed');
});

test('crashed: killed by a signal', () => {
  const r = run('', null);
  assert.equal(r.status, 'crashed');
  assert.equal(r.exit, -1);
});

test('child summaries are summed with the parent', () => {
  const out = [
    '0100:file: 10 tests executed (1 marked as todo, 0 as flaky, 0 failures), 2 skipped.',
    '0101:file:1.2s 5 tests executed (0 marked as todo, 2 as flaky, 3 failures), 1 skipped.',
    '0102:file: 7 tests executed (1 marked as todo, 0 as flaky, 1 failure), 0 skipped.',
  ].join('\n');
  const r = run(out, 4);
  assert.deepEqual([r.executed, r.todo, r.flaky, r.failures, r.skipped], [22, 2, 2, 4, 3]);
  assert.equal(r.status, 'failed');
});

test('timeout and unsupported', () => {
  assert.equal(run('', null, { timedOut: true }).status, 'timeout');
  assert.equal(run('tempwind: unsupported: NtFoo\n', 1).status, 'unsupported');
});

test('a clean summary with a non-zero exit code is failed', () => {
  assert.equal(run('0001:file: 1 tests executed (0 marked as todo, 0 as flaky, 0 failures), 0 skipped.\n', 2).status, 'failed');
});

test('--list output', () => {
  assert.deepEqual(parseList('Valid test names:\n    alpha\n    beta\n\n'), ['alpha', 'beta']);
  assert.deepEqual(parseList('nothing\n'), []);
});
