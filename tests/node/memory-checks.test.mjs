// M6 memory checks (translator.md, "Memory checks"; abi.md, "Page-state
// table") through the runtime: code translated by x64wasm-cli with
// --null-check and --store-check, run at .text of the synthetic image.
// Needs target/release/x64wasm, the x64wasm-cli binary (tests/pe/prepare.sh builds it).
import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import {
  STATE_EXIT, STATE_FAULT_ADDR, STATE_FAULT_KIND, EXIT_FAULT, EXIT_UNSUPPORTED, FAULT_READ, FAULT_WRITE,
  MAIN_STATE_ADDR, MEMORY_MODULE, COUNTERS_NAME, PAGE_FLAGS_NAME, PAGE_CODE,
} from '../../runtime/src/abi.mjs';
import { Process, ProcessError } from '../../runtime/src/process.mjs';
import { RuntimeTranslator } from '../../runtime/src/rt.mjs';
import { fakeExe, fakePlan, TEXT, DATA } from './pe-fixture.mjs';

const root = new URL('../../', import.meta.url).pathname;
const CLI = `${root}target/release/x64wasm`;

/** Translates `code` (hex) at .text with `flags`; returns the module bytes. */
function translate(code, flags) {
  assert.ok(existsSync(CLI), `${CLI} is missing. Build it with: tests/pe/prepare.sh`);
  const dir = mkdtempSync(join(tmpdir(), 'tempwind-checks-'));
  try {
    const out = join(dir, 'm.wasm');
    const r = spawnSync(CLI, ['translate', '--base', `0x${TEXT.toString(16)}`, '--hex', code, ...flags, '-o', out]);
    assert.equal(r.status, 0, r.stderr.toString());
    return readFileSync(out);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

const hexBytes = (s) => Uint8Array.from(s.replace(/\s+/g, '').match(/../g), (b) => parseInt(b, 16));

/** A process whose .text holds `code` and whose AOT module is its translation with `flags`. */
async function load(code, flags) {
  const exe = fakeExe();
  exe.set(hexBytes(code), 0x400);
  return Process.load({ exe, plan: await fakePlan(exe), aot: translate(code, flags) });
}

const BOTH = ['--null-check', '--store-check'];
const state = (p) => ({
  exit: p.mem.u32(MAIN_STATE_ADDR + BigInt(STATE_EXIT)),
  kind: p.mem.u32(MAIN_STATE_ADDR + BigInt(STATE_FAULT_KIND)),
  addr: p.mem.u64(MAIN_STATE_ADDR + BigInt(STATE_FAULT_ADDR)),
});

test('a NULL read exits EXIT_FAULT with FAULT_READ and the address, and stops loudly', async () => {
  // mov rax, [8]; ret
  const p = await load('488b042508000000 c3', BOTH);
  await assert.rejects(p.start(), new ProcessError('fault: read at 0x8 (rip 0x140001000)'));
  assert.deepEqual(state(p), { exit: EXIT_FAULT, kind: FAULT_READ, addr: 8n });
});

test('a NULL write exits EXIT_FAULT with FAULT_WRITE and the address', async () => {
  // mov [0xfffc], eax; ret
  const p = await load('890425fcff0000 c3', ['--null-check']);
  await assert.rejects(p.start(), new ProcessError('fault: write at 0xfffc (rip 0x140001000)'));
  assert.deepEqual(state(p), { exit: EXIT_FAULT, kind: FAULT_WRITE, addr: 0xfffcn });
});

test('a store into a code page exits EXIT_UNSUPPORTED at the store; the code pages are PAGE_CODE', async () => {
  // mov [rip + 0], eax (the next instruction, in .text); ret
  const p = await load('890500000000 c3', BOTH);
  const table = p.linker.pageFlagsAddr;
  assert.notEqual(table, null, 'the module imports env.page_flags');
  assert.equal(p.mem.u8get(table + (TEXT >> 12n)), PAGE_CODE, '.text');
  assert.equal(p.mem.u8get(table + (DATA >> 12n)), 0, '.data');
  await assert.rejects(p.start(), /unsupported instruction at 0x140001000 .*store check/);
  assert.equal(state(p).exit, EXIT_UNSUPPORTED);
});

test('imports: env.page_flags only with --store-check, neither global without the options', () => {
  const globals = (flags) => WebAssembly.Module.imports(new WebAssembly.Module(translate('890500000000 c3', flags)))
    .filter((i) => i.module === MEMORY_MODULE && (i.name === COUNTERS_NAME || i.name === PAGE_FLAGS_NAME))
    .map((i) => i.name);
  assert.deepEqual(globals([]), []);
  assert.deepEqual(globals(['--null-check']), []);
  assert.deepEqual(globals(['--store-check']), [PAGE_FLAGS_NAME]);
  assert.deepEqual(globals(['--counters', ...BOTH]), [COUNTERS_NAME, PAGE_FLAGS_NAME]);
});

// Run-time translation with the image's options (runtime.md, "Load plan"; abi.md, "Request").
const RT = `${root}target/wasm32-unknown-unknown/release/x64wasm_rt.wasm`;

/**
 * lea rax, [rip + 3]; call rax; ret / (.text + 10) mov rax, [8]; ret: the read is
 * reachable only through the computed pointer, so it is translated at run time.
 */
async function loadHidden(flags, options) {
  assert.ok(existsSync(RT), `${RT} is missing. Build it with: tests/pe/prepare.sh`);
  const exe = fakeExe();
  exe.set(hexBytes('488d0503000000 ffd0 c3 488b042508000000 c3'), 0x400);
  const plan = await fakePlan(exe, (json) => { if (options) json.options = options; });
  const p = await Process.load({ exe, plan, aot: translate('488d0503000000 ffd0 c3', flags), rt: await RuntimeTranslator.load(readFileSync(RT)) });
  return p;
}

test('run-time-translated code with the plan\'s checks faults on NULL like AOT code', async () => {
  const p = await loadHidden(BOTH, { nullCheck: true, storeCheck: true, counters: false });
  await assert.rejects(p.start(), new ProcessError('fault: read at 0x8 (rip 0x14000100a)'));
  assert.deepEqual(state(p), { exit: EXIT_FAULT, kind: FAULT_READ, addr: 8n });
  assert.equal(p.translations, 1, 'the read was translated at run time');
});

test('without the plan\'s options, run-time-translated code has no checks', async () => {
  const p = await loadHidden([], undefined);
  await p.start();
  assert.equal(p.translations, 1);
});
