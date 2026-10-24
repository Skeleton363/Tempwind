// A synthetic "PE file" and its load plan for process tests. The runtime never
// parses PE files (translate-pe writes the plan), so the bytes only need to be
// distinguishable: a pattern, hashed into the plan.
import { sha256 } from '../../runtime/src/plan.mjs';
import { v3Module } from './v3-mini.mjs';

export const BASE = 0x1_4000_0000n;
export const TEXT = BASE + 0x1000n;
export const DATA = BASE + 0x2000n;
export const IDATA = BASE + 0x3000n;
export const TLSSEC = BASE + 0x4000n;
/** Text at the start of .data (DATA). */
export const GREETING = 'hello from the guest\n';
const h = (v) => '0x' + BigInt(v).toString(16);

export function fakeExe() {
  const exe = new Uint8Array(0xc00);
  for (let i = 0; i < exe.length; i++) exe[i] = (i * 7 + 1) & 0xff;
  exe.set(new TextEncoder().encode(GREETING), 0x600);
  return exe;
}

/** The plan for fakeExe(); `edit(json)` may change it before hashing is applied. */
export async function fakePlan(exe, edit = () => {}) {
  const json = {
    version: 1,
    file: 'fake.exe',
    sha256: await sha256(exe),
    machine: '0x8664',
    imageBase: h(BASE),
    sizeOfImage: '0x5000',
    sizeOfHeaders: '0x400',
    entryPoint: h(TEXT),
    subsystem: 3,
    stackReserve: '0x200000',
    stackCommit: '0x1000',
    sections: [
      { name: '.text', va: h(TEXT), virtualSize: '0x100', fileOffset: '0x400', fileSize: '0x200', characteristics: '0x60000020' },
      { name: '.data', va: h(DATA), virtualSize: '0x300', fileOffset: '0x600', fileSize: '0x200', characteristics: '0xc0000040' },
      { name: '.idata', va: h(IDATA), virtualSize: '0x100', fileOffset: '0x800', fileSize: '0x200', characteristics: '0xc0000040' },
      { name: '.tls', va: h(TLSSEC), virtualSize: '0x40', fileOffset: '0xa00', fileSize: '0x200', characteristics: '0xc0000040' },
    ],
    imports: [
      { dll: 'KERNEL32.dll', functions: [
        { name: 'GetStdHandle', ordinal: null, iat: h(IDATA) },
        { name: 'WriteFile', ordinal: null, iat: h(IDATA + 8n) },
        { name: 'ExitProcess', ordinal: null, iat: h(IDATA + 16n) },
        { name: null, ordinal: 5, iat: h(IDATA + 24n) },
      ] },
      { dll: 'USER32.dll', functions: [{ name: 'MessageBoxA', ordinal: null, iat: h(IDATA + 40n) }] },
    ],
    tls: null,
    hasRelocations: true,
  };
  edit(json);
  return json;
}

export const TLS_PLAN = {
  startOfRawData: h(TLSSEC), endOfRawData: h(TLSSEC + 0x10n), addressOfIndex: h(TLSSEC + 0x20n),
  sizeOfZeroFill: '0x20', callbacks: [h(TEXT + 0x80n)],
};

/** Import ids follow plan order: GetStdHandle 0, WriteFile 1, ExitProcess 2, #5 3, MessageBoxA 4. */
export const ID = { GetStdHandle: 0, WriteFile: 1, ExitProcess: 2, ordinal5: 3, MessageBoxA: 4 };

const i64Const = (v) => {
  const out = [0x42];
  let n = BigInt.asIntN(64, BigInt(v));
  for (;;) {
    const b = Number(n & 0x7fn);
    n >>= 7n;
    if ((n === 0n && !(b & 0x40)) || (n === -1n && (b & 0x40))) { out.push(b); return out; }
    out.push(b | 0x80);
  }
};

/**
 * An AOT-shaped module whose `run` sets registers (`regs`: {rcx, rdx, r8,
 * r9}, or a BigInt for RCX alone), calls the host function (table slot 0)
 * for import `id` with ret = NO_RET, then returns the exit reason on
 * STATUS_EXIT, else stores and returns EXIT_LEFT_CODE (rip is whatever the
 * host call left, e.g. the popped return address). Its one block sits at
 * the entry point.
 */
export function hostCallAot(id, regs) {
  if (typeof regs === 'bigint') regs = { rcx: regs };
  const offsets = { rcx: 8, rdx: 16, r8: 64, r9: 72 };
  const run = [
    ...Object.entries(regs).flatMap(([r, v]) => [0x20, 0, ...i64Const(v), 0x37, 3, offsets[r]]),
    0x20, 0, 0x41, id, ...i64Const(-1n), 0x41, 0, 0x11, 1, 0, // call_indirect (type 1) table[0](state, id, NO_RET)
    0x41, 2, 0x46, // status == STATUS_EXIT
    0x04, 0x7f,
    0x20, 0, 0x28, 2, 0x90, 0x01, // state.exit
    0x05,
    0x20, 0, 0x41, 1, 0x36, 2, 0x90, 0x01, 0x41, 1, // state.exit = 1; 1
    0x0b,
  ];
  return v3Module({ run, chunks: [[0x41, 0]], blocks: [{ addr: TEXT, chunk: 0, entry: 0 }] });
}

/** An AOT-shaped module with one block at the entry point; `run` is replaced by tests. */
export const trivialAot = () => v3Module({ run: [0x41, 1], chunks: [[0x41, 0]], blocks: [{ addr: TEXT, chunk: 0, entry: 0 }] });
