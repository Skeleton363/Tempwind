// Code page 1252 (Windows Latin 1), the ANSI code page of the reference
// runner. Bytes 0x80-0x9F map as in Windows' table (c_1252.nls, Wine's
// c_1252.c): the five unassigned bytes map to the C1 controls U+0081,
// U+008D, U+008F, U+0090 and U+009D. Every other byte maps to itself.

const HIGH = [
  0x20ac, 0x0081, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008d, 0x017d, 0x008f,
  0x0090, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, 0x009d, 0x017e, 0x0178,
];

/** Byte -> UTF-16 code unit. */
export const CP1252_TO_UNICODE = Uint16Array.from({ length: 256 }, (_, b) => (b >= 0x80 && b < 0xa0 ? HIGH[b - 0x80] : b));

/** UTF-16 code unit -> byte, for the units code page 1252 represents exactly. */
export const UNICODE_TO_CP1252 = new Map(Array.from(CP1252_TO_UNICODE, (u, b) => [u, b]));
