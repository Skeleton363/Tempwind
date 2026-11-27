// A synthetic "PE file" and its load plan for the plan tests. The runtime never
// parses PE files (translate-pe writes the plan), so the bytes only need to be
// distinguishable: a pattern, hashed into the plan.
import { sha256 } from '../../runtime/src/plan.mjs';

export const BASE = 0x1_4000_0000n;
export const TEXT = BASE + 0x1000n;
const DATA = BASE + 0x2000n;
const IDATA = BASE + 0x3000n;
export const TLSSEC = BASE + 0x4000n;
const h = (v) => '0x' + BigInt(v).toString(16);

export function fakeExe() {
  const exe = new Uint8Array(0xc00);
  for (let i = 0; i < exe.length; i++) exe[i] = (i * 7 + 1) & 0xff;
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
