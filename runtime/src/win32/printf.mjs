// msvcrt's printf formatting (narrow), after Wine 11 dlls/msvcrt/printf.h
// (pf_printf, pf_fill, pf_integer_conv) with msvcrt.dll's options: legacy
// wide specifiers, legacy msvcrt compatibility, no positional parameters,
// no invalid-parameter handler for unknown conversions.
//
// Floating-point conversions (aAeEfFgG), %n and wide characters above 0xFF
// are not supported yet: they throw FormatError, which the caller turns
// into an error stop.

export class FormatError extends Error {}

const LEN_SHORT = 1, LEN_LONG = 2;
const isDigit = (c) => c >= 0x30 && c <= 0x39;
const ch = (s) => s.charCodeAt(0);

/**
 * Formats `fmt` (bytes, without the NUL) and returns the output bytes.
 * @param {Uint8Array} fmt
 * @param {{int: () => number, i64: () => bigint, ptr: () => bigint,
 *   str: (addr: bigint, max: number) => Uint8Array, wstr: (addr: bigint, max: number) => number[]}} args
 *   `int`: the next argument as a signed 32-bit int; `i64`: as a signed 64-bit
 *   BigInt; `ptr`: as a u64 BigInt. `str`/`wstr` read at most `max` units
 *   (max < 0: up to the NUL) of a narrow or wide string.
 * @returns {Uint8Array}
 */
export function formatPrintf(fmt, args) {
  const out = [];
  const puts = (bytes) => { for (const b of bytes) out.push(b); };
  let p = 0;
  const at = (i) => (i < fmt.length ? fmt[i] : 0);
  while (p < fmt.length) {
    if (fmt[p] !== 0x25) { out.push(fmt[p++]); continue; }
    p++;
    if (at(p) === 0x25) { out.push(0x25); p++; continue; }

    const f = { sign: 0, leftAlign: false, padZero: false, alternate: false, fieldLength: 0, precision: -1,
      integerLength: 0, integerDouble: false, integerNative: false, wideString: false, format: 0 };
    for (;;) {
      const c = at(p);
      if (c === ch('+') || c === ch(' ')) { if (f.sign !== ch('+')) f.sign = c; }
      else if (c === ch('-')) f.leftAlign = true;
      else if (c === ch('0')) f.padZero = true;
      else if (c === ch('#')) f.alternate = true;
      else break;
      p++;
    }
    if (at(p) === ch('*')) {
      p++;
      f.fieldLength = args.int();
      if (f.fieldLength < 0) { f.leftAlign = true; f.fieldLength = -f.fieldLength; }
    }
    while (isDigit(at(p))) f.fieldLength = (f.fieldLength * 10 + at(p++) - 0x30) | 0;
    if (at(p) === ch('.')) {
      f.precision = 0;
      p++;
      if (at(p) === ch('*')) { p++; f.precision = args.int(); }
      else while (isDigit(at(p))) f.precision = (f.precision * 10 + at(p++) - 0x30) | 0;
    }
    for (;;) {
      const c = at(p);
      if (c === ch('l') && at(p + 1) === ch('l')) { f.integerDouble = true; p++; }
      else if (c === ch('l')) f.integerLength = LEN_LONG;
      else if (c === ch('h')) f.integerLength = LEN_SHORT;
      else if (c === ch('I')) {
        if (at(p + 1) === ch('6') && at(p + 2) === ch('4')) { f.integerDouble = true; p += 2; }
        else if (at(p + 1) === ch('3') && at(p + 2) === ch('2')) p += 2;
        else if (at(p + 1) && 'diouxX'.includes(String.fromCharCode(at(p + 1)))) f.integerNative = true;
        else break;
      } else if (c === ch('w')) f.wideString = true;
      else if ((c === ch('z') || c === ch('t')) && at(p + 1) && 'diouxX'.includes(String.fromCharCode(at(p + 1)))) f.integerNative = true;
      else if (c === ch('j')) f.integerDouble = true;
      else if (c !== ch('L') && c !== ch('F') && c !== ch('N')) break;
      p++;
    }
    f.format = at(p);
    const fc = String.fromCharCode(f.format);

    if (fc === 's' || fc === 'S') {
      handleString(puts, args.ptr(), -1, f, args);
    } else if (fc === 'c' || fc === 'C') {
      const c = args.int();
      handleString(puts, { char: c }, 1, f, args);
    } else if (fc === 'p') {
      f.format = ch('X');
      f.padZero = true;
      const precision = f.precision;
      f.precision = 16;
      const buf = integerConv(f, args.ptr(), false);
      f.padZero = false;
      f.precision = precision;
      formatStr(puts, buf, f);
    } else if (fc === 'n') {
      throw new FormatError('the %n conversion is disabled in msvcrt');
    } else if (f.format && 'diouxX'.includes(fc)) {
      if ((fc === 'x' || fc === 'X') && f.padZero && f.alternate && !f.leftAlign && f.precision < f.fieldLength - 2) {
        f.precision = f.fieldLength - 2;
      }
      const signed = fc === 'd' || fc === 'i';
      let v;
      if (f.integerDouble || f.integerNative) v = signed ? args.i64() : BigInt.asUintN(64, args.i64());
      else {
        let n = args.int();
        if (f.integerLength === LEN_SHORT) n = signed ? (n << 16) >> 16 : n & 0xffff;
        else if (!signed) n >>>= 0;
        v = BigInt(n);
      }
      formatStr(puts, integerConv(f, v, signed), f);
    } else if (f.format && 'aAeEfFgG'.includes(fc)) {
      throw new FormatError(`floating-point conversion %${fc} is not supported yet`);
    } else {
      // Unknown conversion: msvcrt skips the specification; the character itself is output next.
      continue;
    }
    p++;
  }
  return new Uint8Array(out);
}

/** pf_integer_conv: digits of `x` with precision zeros and the alternate prefix; updates f.sign and f.precision. */
function integerConv(f, x, signed) {
  const fc = String.fromCharCode(f.format);
  const base = fc === 'o' ? 8n : fc === 'x' || fc === 'X' ? 16n : 10n;
  const digits = fc === 'X' ? '0123456789ABCDEFX' : '0123456789abcdefx';
  if (signed && x < 0n) { x = -x; f.sign = ch('-'); }
  x = BigInt.asUintN(64, x);
  const buf = [];
  if (x === 0n) {
    f.alternate = false;
    if (f.precision) buf.push('0');
  } else {
    while (x !== 0n) { buf.push(digits[Number(x % base)]); x /= base; }
  }
  for (let k = f.precision - buf.length; k > 0; k--) buf.push('0');
  if (f.alternate) {
    if (base === 16n) buf.push(digits[16], '0');
    else if (base === 8n && buf[buf.length - 1] !== '0') buf.push('0');
  }
  f.precision = buf.length;
  return new TextEncoder().encode(buf.reverse().join(''));
}

/** pf_fill: sign, alignment, zero and field padding, before (left) or after the text. */
function fill(puts, len, f, left) {
  if (f.sign && !'diaAeEfFgG'.includes(String.fromCharCode(f.format))) f.sign = 0;
  if (left && f.sign) {
    f.fieldLength--;
    if (f.padZero) puts([f.sign]);
  }
  if ((!left && f.leftAlign) || (left && !f.leftAlign)) {
    const c = left && f.padZero ? ch('0') : ch(' ');
    for (let i = 0; i < f.fieldLength - len; i++) puts([c]);
  }
  if (left && f.sign && !f.padZero) puts([f.sign]);
}

/** pf_output_format_str: `str` truncated to the precision, filled to the field length. */
function formatStr(puts, str, f) {
  let len = str.length;
  if (f.precision >= 0 && f.precision < len) len = f.precision;
  fill(puts, len, f, true);
  puts(str.subarray(0, len));
  fill(puts, len, f, false);
}

/** pf_handle_string for the narrow printf of msvcrt.dll (legacy wide specifiers). */
function handleString(puts, src, len, f, args) {
  if (src === 0n) { formatStr(puts, new TextEncoder().encode('(null)'), f); return; }
  let wide;
  if (f.wideString || f.integerLength === LEN_LONG) wide = true;
  else if (f.integerLength === LEN_SHORT) wide = false;
  else wide = f.format === ch('S') || f.format === ch('C');
  const max = f.precision >= 0 ? f.precision : -1;
  let bytes;
  if (typeof src === 'object') {
    // %c: the int's low byte, or its low 16 bits as a wide character.
    if (!wide) bytes = new Uint8Array([src.char & 0xff]);
    else bytes = narrowFromWide([src.char & 0xffff]);
  } else if (!wide) {
    bytes = args.str(src, max);
  } else {
    bytes = narrowFromWide(args.wstr(src, max));
  }
  formatStr(puts, bytes, f);
}

/** Wide to narrow in the C locale: code units up to 0xFF map to themselves. */
function narrowFromWide(units) {
  for (const u of units) {
    if (u > 0xff) throw new FormatError(`wide character U+${u.toString(16).padStart(4, '0')} has no C-locale narrow form`);
  }
  return new Uint8Array(units);
}
