import test from 'node:test';
import assert from 'node:assert/strict';
import {
  LOOKUP_HEADER_ADDR, LOOKUP_SLOTS, LOOKUP_MASK, THUNK_BASE, PEB_ADDR, TEB_ADDR, SENTINEL_RET,
  STATE_RIP, STATE_RFLAGS, STATE_XMM, STATE_MXCSR, STATE_FCW, STATE_FSW, STATE_FTW, STATE_EXIT, STATE_GS_BASE, STATE_FAULT_ADDR, STATE_FAULT_KIND,
  EXIT_LEFT_CODE, EXIT_UNSUPPORTED, EXIT_FAULT, FAULT_WRITE, MAIN_STATE_ADDR, gprOffset,
} from '../../runtime/src/abi.mjs';
import { Process, ProcessError } from '../../runtime/src/process.mjs';
import { fakeExe, fakePlan, hostCallAot, trivialAot, BASE, TEXT, DATA, IDATA, TLSSEC, TLS_PLAN, ID } from './pe-fixture.mjs';
import { v3Module } from './v3-mini.mjs';

const exe = fakeExe();
const load = async ({ edit, aot = trivialAot(), ...opts } = {}) =>
  Process.load({ exe, plan: await fakePlan(exe, edit), aot, ...opts });
const RAX = 0, RCX = 1, RDX = 2, RSP = 4, R8 = 8;

test('maps headers and sections; bss stays zero', async () => {
  const p = await load();
  assert.deepEqual(p.mem.read(BASE, 0x400), exe.subarray(0, 0x400));
  assert.equal(p.mem.u8get(BASE + 0x400n), 0, 'past sizeOfHeaders');
  assert.deepEqual(p.mem.read(TEXT, 0x100), exe.subarray(0x400, 0x500));
  assert.equal(p.mem.u8get(TEXT + 0x100n), 0, 'past virtualSize');
  assert.deepEqual(p.mem.read(DATA, 0x200), exe.subarray(0x600, 0x800));
  assert.equal(p.mem.u8get(DATA + 0x200n), 0, 'bss');
  assert.equal(p.memory.buffer.byteLength, 0x1_4001_0000);
});

test('import thunks, lookup slots and the IAT', async () => {
  const p = await load();
  for (let id = 0; id < 5; id++) {
    const thunk = THUNK_BASE + 16n * BigInt(id);
    assert.deepEqual([...p.mem.read(thunk, 16)], [0x0f, 0x0b, ...new Array(14).fill(0xcc)]);
    assert.deepEqual(p.linker.lookup(thunk), { func: 0, entry: id });
  }
  assert.equal(p.mem.u64(IDATA), THUNK_BASE);
  assert.equal(p.mem.u64(IDATA + 16n), THUNK_BASE + 32n);
  assert.equal(p.mem.u64(IDATA + 40n), THUNK_BASE + 64n);
  assert.equal(p.imports[ID.ordinal5].name, '#5');
  assert.equal(p.imports[ID.ordinal5].impl, undefined);
  assert.ok(p.imports[ID.WriteFile].impl, 'KERNEL32.dll matches kernel32.dll');
  assert.equal(p.imports[ID.MessageBoxA].impl, undefined);
  // The lookup table: header at 0x8000_0000, at least 4096 slots in the runtime heap.
  assert.equal(p.mem.u64(LOOKUP_HEADER_ADDR + BigInt(LOOKUP_MASK)), 4095n);
  assert.ok(p.mem.u64(LOOKUP_HEADER_ADDR + BigInt(LOOKUP_SLOTS)) >= 0x8010_0000n);
  // Table slot 1 is the blocking host function (runtime.md, "Guest threads"); the AOT module follows.
  assert.equal(p.blockingIndex, 1);
  assert.deepEqual(p.linker.lookup(TEXT), { func: 2, entry: 0 }, 'AOT block');
});

test('GetProcAddress thunks: created on demand after the imports, one per function', async () => {
  const p = await load();
  const n = p.imports.length;
  const a = p.procAddress('KERNEL32.dll', 'SetDefaultDllDirectories');
  assert.equal(a, THUNK_BASE + 16n * BigInt(n));
  assert.deepEqual([...p.mem.read(a, 16)], [0x0f, 0x0b, ...new Array(14).fill(0xcc)]);
  assert.deepEqual(p.linker.lookup(a), { func: 0, entry: n });
  assert.equal(p.imports[n].name, 'SetDefaultDllDirectories');
  assert.equal(p.procAddress('kernel32.dll', 'SetDefaultDllDirectories'), a);
  assert.equal(p.procAddress('kernel32.dll', 'NoSuchFunction'), undefined);
  assert.equal(p.imports.length, n + 1);
});

test('the file system: the program is C:\\tempwind\\<file>, read-only; C:\\work is empty', async () => {
  const p = await load();
  const node = p.vfs.stat(p.programPath);
  assert.equal(p.programPath, `C:\\tempwind\\${p.plan.file}`);
  assert.equal(node.attrs, 0x21);
  assert.equal(node.size, exe.length);
  assert.equal(p.vfs.cwd, 'C:\\work');
  assert.equal(p.vfs.stat('C:\\work').children.size, 0);
});

test('PEB and TEB fields', async () => {
  const p = await load();
  const m = p.mem;
  assert.equal(m.u64(TEB_ADDR + 0x08n), 0x7ff0_0000n);
  assert.equal(m.u64(TEB_ADDR + 0x10n), 0x7ff0_0000n - 0x20_0000n);
  assert.equal(m.u64(TEB_ADDR + 0x30n), TEB_ADDR);
  assert.equal(m.u64(TEB_ADDR + 0x40n), 0x1000n);
  assert.equal(m.u64(TEB_ADDR + 0x48n), 0x1004n);
  assert.equal(m.u64(TEB_ADDR + 0x58n), 0n, 'no TLS');
  assert.equal(m.u64(TEB_ADDR + 0x60n), PEB_ADDR);
  assert.equal(m.u32(TEB_ADDR + 0x68n), 0);
  assert.equal(m.u8get(PEB_ADDR + 2n), 0);
  assert.equal(m.u64(PEB_ADDR + 0x10n), BASE);
  assert.equal(m.u32(PEB_ADDR + 0xb8n), 1);
  assert.deepEqual([0x118n, 0x11cn, 0x120n, 0x124n].map((o) => m.u32(PEB_ADDR + o)), [10, 0, 26100, 2]);
});

test('TLS: block, pointer array, index', async () => {
  const p = await load({ edit: (j) => { j.tls = TLS_PLAN; } });
  const arr = p.mem.u64(TEB_ADDR + 0x58n);
  assert.ok(arr >= 0x1000_0000n && arr < 0x7ff0_0000n, 'in the guest heap');
  const block = p.mem.u64(arr);
  assert.equal(block % 16n, 0n);
  assert.deepEqual(p.mem.read(block, 0x10), exe.subarray(0xa00, 0xa10));
  assert.deepEqual([...p.mem.read(block + 0x10n, 0x20)], new Array(0x20).fill(0));
  assert.equal(p.mem.u32(TLSSEC + 0x20n), 0);
});

test('a host-initiated guest call starts as after a call, with 32 bytes of shadow space', async () => {
  const p = await load();
  p.mem.setU64(MAIN_STATE_ADDR + 0x300n, 0x1234n);
  p.prepareCall(TEXT, [PEB_ADDR]);
  const rsp = p.reg(RSP);
  assert.equal(rsp, 0x7ff0_0000n - 0x38n);
  assert.equal(rsp % 16n, 8n);
  assert.equal(p.mem.u64(rsp), SENTINEL_RET);
  assert.equal(p.reg(RCX), PEB_ADDR);
  assert.equal(p.reg(RDX), 0n);
  assert.equal(p.mem.u64(MAIN_STATE_ADDR + BigInt(STATE_RIP)), TEXT);
  assert.equal(p.mem.u64(MAIN_STATE_ADDR + BigInt(STATE_RFLAGS)), 0x202n);
  assert.equal(p.mem.u64(MAIN_STATE_ADDR + BigInt(STATE_GS_BASE)), TEB_ADDR);
  assert.equal(p.mem.u64(MAIN_STATE_ADDR + 0x300n), 0n, 'state block zeroed');
});

test('the main thread FP state starts as on Windows and carries over between host-initiated calls', async () => {
  const p = await load();
  const at = (o) => MAIN_STATE_ADDR + BigInt(o);
  p.prepareCall(TEXT, [PEB_ADDR]);
  assert.equal(p.mem.u32(at(STATE_MXCSR)), 0x1f80, 'all SSE exceptions masked, round to nearest');
  assert.equal(p.mem.u16(at(STATE_FCW)), 0x027f, 'the Windows x64 x87 control word');
  assert.equal(p.mem.u16(at(STATE_FSW)), 0);
  assert.equal(p.mem.u16(at(STATE_FTW)), 0xffff, 'every x87 register empty');
  assert.equal(p.mem.u64(at(STATE_XMM)), 0n);
  // A TLS callback's FP changes reach the entry point, as on one Windows thread.
  p.mem.setU32(at(STATE_MXCSR), 0x9fc0);
  p.mem.setU64(at(STATE_XMM + 0x10), 0x4000_0000_0000_0000n);
  p.mem.setU16(at(STATE_FCW), 0x037f);
  p.prepareCall(TEXT, [PEB_ADDR]);
  assert.equal(p.mem.u32(at(STATE_MXCSR)), 0x9fc0);
  assert.equal(p.mem.u64(at(STATE_XMM + 0x10)), 0x4000_0000_0000_0000n);
  assert.equal(p.mem.u16(at(STATE_FCW)), 0x037f);
});

/** Replaces `run` with a JS script: each call gets (process, state) and returns an exit reason. */
const scripted = (p, steps) => {
  const calls = [];
  p.run = (state) => {
    calls.push(p.mem.u64(state + BigInt(STATE_RIP)));
    const step = steps.shift();
    if (!step) throw new Error('run called too often');
    const reason = step(p, state);
    p.mem.setU32(state + BigInt(STATE_EXIT), reason);
    return reason;
  };
  return calls;
};
const setRip = (p, v) => p.mem.setU64(MAIN_STATE_ADDR + BigInt(STATE_RIP), v);

test('the entry point returning to the sentinel exits with EAX', async () => {
  const p = await load();
  scripted(p, [(q) => { q.setReg(RAX, 0xdead_0000_0000_002an); setRip(q, SENTINEL_RET); return EXIT_LEFT_CODE; }]);
  assert.equal(await p.start(), 42);
});

test('EXIT_LEFT_CODE outside executable code is an execute fault', async () => {
  const p = await load();
  scripted(p, [(q) => { setRip(q, DATA); return EXIT_LEFT_CODE; }]);
  await assert.rejects(p.start(), new ProcessError(`fault: execute at 0x${DATA.toString(16)}`));
});

test('EXIT_LEFT_CODE in code without a run-time translator stops', async () => {
  const p = await load();
  scripted(p, [(q) => { setRip(q, TEXT + 0x40n); return EXIT_LEFT_CODE; }]);
  await assert.rejects(p.start(), /no translated code at 0x140001040 and no run-time translator/);
});

test('EXIT_UNSUPPORTED names the address and the bytes', async () => {
  const p = await load();
  scripted(p, [(q) => { setRip(q, TEXT + 0xf8n); return EXIT_UNSUPPORTED; }]);
  const want = Array.from(exe.subarray(0x4f8, 0x500), (b) => b.toString(16).padStart(2, '0')).join(' ');
  await assert.rejects(p.start(), new ProcessError(`unsupported instruction at 0x1400010f8 (bytes ${want})`));
});

test('EXIT_FAULT names the kind and the address', async () => {
  const p = await load();
  scripted(p, [(q, s) => {
    q.mem.setU64(s + BigInt(STATE_FAULT_ADDR), 0x10n);
    q.mem.setU32(s + BigInt(STATE_FAULT_KIND), FAULT_WRITE);
    setRip(q, TEXT);
    return EXIT_FAULT;
  }]);
  await assert.rejects(p.start(), /fault: write at 0x10 \(rip 0x140001000\)/);
});

test('an unknown exit reason stops', async () => {
  const p = await load();
  scripted(p, [() => 9]);
  await assert.rejects(p.start(), /unknown exit reason 9/);
});

test('run-time translation: request, link, continue', async () => {
  const requests = [];
  const target = TEXT + 0x40n;
  const rt = {
    translate(req) {
      requests.push(req);
      return v3Module({ run: [0x41, 1], chunks: [[0x41, 0]], blocks: [{ addr: TEXT, chunk: 0, entry: 1 }, { addr: target, chunk: 0, entry: 0 }] });
    },
  };
  const lines = [];
  const p = await load({ rt, trace: { translate: true, exits: true }, log: (l) => lines.push(l) });
  const calls = scripted(p, [
    (q) => { setRip(q, target); return EXIT_LEFT_CODE; },
    (q) => { q.setReg(RAX, 3n); setRip(q, SENTINEL_RET); return EXIT_LEFT_CODE; },
  ]);
  assert.equal(await p.start(), 3);
  assert.deepEqual(calls, [TEXT, target]);
  assert.equal(requests.length, 1);
  const r = requests[0];
  assert.equal(r.codeBase, TEXT);
  assert.equal(r.entry, target);
  assert.deepEqual(r.code, exe.subarray(0x400, 0x500));
  const thunks = Array.from({ length: 5 }, (_, i) => THUNK_BASE + 16n * BigInt(i));
  assert.deepEqual([...r.known], [...thunks, TEXT], 'every key, sorted');
  assert.deepEqual(p.linker.lookup(target), { func: 3, entry: 0 });
  assert.deepEqual(p.linker.lookup(TEXT), { func: 2, entry: 0 }, 'the AOT block wins');
  assert.equal(p.translations, 1);
  assert.match(lines.join('\n'), /^exit LEFT_CODE rip=0x140001040$/m);
  assert.match(lines.join('\n'), /^translate 0x140001040: 2 blocks \(1 new\), \d+ wasm bytes, [\d.]+ ms$/m);
});

test('run-time translation errors stop the process', async () => {
  const p = await load({ rt: { translate() { throw new Error('run-time translator: decode error'); } } });
  scripted(p, [(q) => { setRip(q, TEXT + 0x40n); return EXIT_LEFT_CODE; }]);
  await assert.rejects(p.start(), /run-time translation at 0x140001040: run-time translator: decode error/);
});

test('a translation without a block for the address stops', async () => {
  const rt = { translate: () => v3Module({ run: [0x41, 1], chunks: [[0x41, 0]], blocks: [{ addr: TEXT + 0x50n, chunk: 0, entry: 0 }] }) };
  const p = await load({ rt });
  scripted(p, [(q) => { setRip(q, TEXT + 0x40n); return EXIT_LEFT_CODE; }]);
  await assert.rejects(p.start(), /produced no block for it/);
});

test('TLS callbacks run first with (imageBase, DLL_PROCESS_ATTACH, 0)', async () => {
  const p = await load({ edit: (j) => { j.tls = TLS_PLAN; } });
  const seen = [];
  const calls = scripted(p, [
    (q) => { seen.push([q.reg(RCX), q.reg(RDX), q.reg(R8)]); setRip(q, SENTINEL_RET); return EXIT_LEFT_CODE; },
    (q) => { seen.push([q.reg(RCX)]); q.setReg(RAX, 0n); setRip(q, SENTINEL_RET); return EXIT_LEFT_CODE; },
  ]);
  assert.equal(await p.start(), 0);
  assert.deepEqual(calls, [TEXT + 0x80n, TEXT]);
  assert.deepEqual(seen, [[BASE, 1n, 0n], [PEB_ADDR]]);
});

// The real wiring: an AOT module whose `run` calls table slot 0.
test('wasm path: ExitProcess through the host-call function', async () => {
  const lines = [];
  const p = await load({ aot: hostCallAot(ID.ExitProcess, 0xffff_ffff_0000_0007n), trace: { api: true, exits: true }, log: (l) => lines.push(l) });
  assert.equal(await p.start(), 7);
  assert.equal(p.mem.u64(MAIN_STATE_ADDR + BigInt(STATE_RIP)), THUNK_BASE + 32n, 'rip = the thunk');
  assert.deepEqual(lines, ['kernel32.dll!ExitProcess(0xffffffff00000007) -> exit 7', 'exit HOST rip=0x80010020']);
});

test('wasm path: a host function returns into the guest (ret to an unexpected address unwinds)', async () => {
  const p = await load({ aot: hostCallAot(ID.GetStdHandle, 0xfffffff5n) });
  // ret = NO_RET, [RSP] = SENTINEL_RET: STATUS_UNWIND with rip = SENTINEL_RET, then the run's EXIT_LEFT_CODE.
  assert.equal(await p.start(), 7, 'EAX = the stdout handle');
  assert.equal(p.reg(RSP), 0x7ff0_0000n - 0x30n, 'the ret popped the return address');
});

test('wasm path: an unimplemented API is an error stop', async () => {
  const p = await load({ aot: hostCallAot(ID.MessageBoxA, 0n) });
  await assert.rejects(p.start(), new ProcessError('unimplemented API user32.dll!MessageBoxA'));
  const q = await load({ aot: hostCallAot(ID.ordinal5, 0n) });
  await assert.rejects(q.start(), new ProcessError('unimplemented API kernel32.dll!#5'));
});
