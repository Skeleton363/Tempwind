// The run-pe CLI as a child process: guest stdout/stderr/exit code
// passthrough, error exits (125), traces.
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync, existsSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { STDOUT_HANDLE, STDERR_HANDLE } from '../../runtime/src/abi.mjs';
import { fakeExe, fakePlan, hostCallAot, hostCallsAot, DATA, IDATA, GREETING, ID } from './pe-fixture.mjs';

const CLI = new URL('../../runtime/src/run-pe.mjs', import.meta.url).pathname;
const dir = mkdtempSync(join(tmpdir(), 'run-pe-test-'));
test.after(() => rmSync(dir, { recursive: true, force: true }));

let n = 0;
async function runPe({ aot, edit, env = {}, args, patch, extra = [] } = {}) {
  const exe = fakeExe();
  patch?.(exe);
  const base = join(dir, `case${n++}`);
  writeFileSync(`${base}.exe`, exe);
  writeFileSync(`${base}.plan.json`, JSON.stringify(await fakePlan(exe, edit)));
  writeFileSync(`${base}.wasm`, aot);
  const r = spawnSync(process.execPath, [CLI, ...(args ?? ['--exe', `${base}.exe`, '--aot', `${base}.wasm`, '--plan', `${base}.plan.json`, ...extra])], {
    env: { ...process.env, TEMPWIND_TRACE: '', ...env },
  });
  return { status: r.status, stdout: r.stdout.toString(), stderr: r.stderr.toString() };
}

const writeFile = (handle) => hostCallAot(ID.WriteFile, { rcx: handle, rdx: DATA, r8: BigInt(GREETING.length), r9: DATA + 0x280n });

test('guest stdout passes through; the entry point returns EAX', async () => {
  const r = await runPe({ aot: writeFile(STDOUT_HANDLE) });
  assert.deepEqual(r, { status: 1, stdout: GREETING, stderr: '' });
});

test('guest stderr passes through', async () => {
  const r = await runPe({ aot: writeFile(STDERR_HANDLE) });
  assert.deepEqual(r, { status: 1, stdout: '', stderr: GREETING });
});

test('ExitProcess sets the exit code; TEMPWIND_TRACE goes to stderr', async () => {
  const r = await runPe({ aot: hostCallAot(ID.ExitProcess, 42n), env: { TEMPWIND_TRACE: 'api,exits' } });
  assert.equal(r.status, 42);
  assert.equal(r.stdout, '');
  assert.equal(r.stderr, '[tempwind] kernel32.dll!ExitProcess(0x2a) -> exit 42\n[tempwind] exit HOST rip=0x80010020\n');
});

for (const [label, opts, err] of [
  ['an unimplemented API', { aot: hostCallAot(ID.MessageBoxA, 0n) }, 'tempwind: unimplemented API user32.dll!MessageBoxA\n'],
  ['a shim error', { aot: hostCallAot(ID.WriteFile, { rcx: 0x44n }) }, 'tempwind: kernel32.dll!WriteFile: lpNumberOfBytesWritten is NULL without lpOverlapped\n'],
  ['a plan for another file', { aot: hostCallAot(ID.ExitProcess, 0n), edit: (j) => { j.sha256 = 'f'.repeat(64); } }, /^tempwind: plan sha256 f{64} does not match the file/],
  ['an unknown trace value', { aot: hostCallAot(ID.ExitProcess, 0n), env: { TEMPWIND_TRACE: 'api,everything' } }, /^tempwind: TEMPWIND_TRACE: unknown value "everything"/],
  ['a bad AOT module', { aot: new Uint8Array([1, 2, 3]) }, /^tempwind: AOT module: /],
  ['a missing option', { aot: new Uint8Array(0), args: ['--exe', 'x.exe'] }, /^tempwind: missing --aot\nusage: run-pe/],
  ['an unknown option', { aot: new Uint8Array(0), args: ['--frobnicate'] }, /^tempwind: Unknown option '--frobnicate'/],
]) {
  test(`${label} exits 125 with "tempwind: <error>"`, async () => {
    const r = await runPe(opts);
    assert.equal(r.status, 125, r.stderr);
    assert.equal(r.stdout, '');
    if (typeof err === 'string') assert.equal(r.stderr, err); else assert.match(r.stderr, err);
  });
}

test('--dir: C:\\work is written back after an error stop', async () => {
  const host = join(dir, 'work-after-error');
  mkdirSync(host);
  const name = 'newdir';
  // CreateDirectoryW(L"newdir", NULL), then an unimplemented API: the run stops with an error.
  const r = await runPe({
    aot: hostCallsAot([[5, { rcx: DATA + 0x100n, rdx: 0n }], [ID.MessageBoxA, 0n]]),
    edit: (j) => { j.imports.push({ dll: 'KERNEL32.dll', functions: [{ name: 'CreateDirectoryW', ordinal: null, iat: '0x' + (IDATA + 48n).toString(16) }] }); },
    patch: (exe) => exe.set(new TextEncoder().encode(name + '\0').reduce((a, c) => [...a, c, 0], []), 0x700),
    extra: ['--dir', host],
  });
  assert.equal(r.status, 125, r.stderr);
  assert.match(r.stderr, /unimplemented API user32.dll!MessageBoxA/);
  assert.ok(existsSync(join(host, name)), 'the directory the guest created is on the host');
});
