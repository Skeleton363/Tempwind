// Helpers shared by the shim modules: wide strings, handles, failures.
import { Win32Error } from './errors.mjs';
import { VfsStop } from './vfs.mjs';

export const hex = (v) => '0x' + BigInt.asUintN(64, BigInt(v)).toString(16);
export const fn = (args, f) => ({ args, fn: f });

/** A guest NUL-terminated UTF-16 string as a JS string. */
export function wstr(ctx, addr, what = 'string') {
  if (addr === 0n) ctx.fail(`${what} is NULL`);
  const units = ctx.readWString(addr);
  let s = '';
  for (let i = 0; i < units.length; i += 4096) s += String.fromCharCode(...units.slice(i, i + 4096));
  return s;
}

/** `count` UTF-16 units at `addr` as a JS string. */
export function wstrN(ctx, addr, count) {
  const b = ctx.read(addr, 2 * count);
  let s = '';
  for (let i = 0; i < count; i++) s += String.fromCharCode(b[2 * i] | (b[2 * i + 1] << 8));
  return s;
}

/** The UTF-16LE bytes of `s`, with a NUL if `nul`. */
export function utf16(s, nul = true) {
  const n = s.length + (nul ? 1 : 0);
  const out = new Uint8Array(2 * n);
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    out[2 * i] = c & 0xff;
    out[2 * i + 1] = c >> 8;
  }
  return out;
}

/** Writes `s` and a NUL at `addr`. */
export function writeWString(ctx, addr, s) { ctx.write(addr, utf16(s)); }

/**
 * The usual "copy a string into a buffer of `size` characters" contract:
 * returns the length without the NUL when it fits, else the size needed
 * with the NUL (and writes nothing).
 */
export function copyOut(ctx, buf, size, s) {
  if (size > s.length) {
    writeWString(ctx, buf, s);
    return s.length;
  }
  return s.length + 1;
}

/** Runs `f`; a Win32Error sets the last error and returns `failure`, a VfsStop stops loudly. */
export function win32(ctx, failure, f) {
  try {
    return f();
  } catch (e) {
    if (e instanceof Win32Error) {
      ctx.setLastError(e.code);
      return failure;
    }
    if (e instanceof VfsStop) ctx.fail(e.message);
    throw e;
  }
}

/** Writes a FILETIME (u64) at `addr` unless `addr` is NULL. */
export function putFileTime(ctx, addr, t) { if (addr !== 0n) ctx.write64(addr, t); }

/** Reads a bigint u64 from the bytes of a little-endian Uint8Array. */
export const u64le = (b, at = 0) => new DataView(b.buffer, b.byteOffset, b.byteLength).getBigUint64(at, true);
