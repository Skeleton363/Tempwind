// user32.dll (runtime.md, "The rest"): the two string functions 7-Zip
// imports. Details follow Microsoft's documentation and Wine 11 dlls/user32
// (lstr.c).

import { fn } from './util.mjs';

/** Upper-cases one UTF-16 code unit (a code unit whose upper case isn't a single unit stays). */
export function upperUnit(c) {
  const u = String.fromCharCode(c).toUpperCase();
  return u.length === 1 ? u.charCodeAt(0) : c;
}

export default {
  // LPWSTR CharUpperW(LPWSTR lpsz): a string in place, or one character in the low word.
  CharUpperW: fn(1, (ctx) => {
    const p = ctx.arg(0);
    if (p >> 16n === 0n) return BigInt(upperUnit(Number(p)));
    for (let a = p; ; a += 2n) {
      const c = ctx.mem.u16(ctx.check(a, 2, 'string'));
      if (c === 0) break;
      ctx.mem.setU16(a, upperUnit(c));
    }
    return p;
  }),
  // LPSTR CharPrevExA(WORD CodePage, LPCSTR lpStart, LPCSTR lpCurrentChar, DWORD dwFlags): the ANSI
  // (1252) and OEM (437) code pages have no lead bytes, so the previous character is one byte back.
  CharPrevExA: fn(4, (ctx) => {
    const cp = ctx.arg32(0) & 0xffff, start = ctx.arg(1), cur = ctx.arg(2);
    if (![0, 1, 3, 437, 1252].includes(cp)) ctx.fail(`code page ${cp} is not supported`);
    if (ctx.arg32(3) !== 0) ctx.fail('dwFlags must be 0');
    return cur > start ? cur - 1n : start;
  }),
};
