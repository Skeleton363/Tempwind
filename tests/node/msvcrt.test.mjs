// msvcrt.dll shim (runtime.md, "msvcrt (M3)"), function by function, through
// the shim harness. Expected values follow msvcrt.dll (tests/native/*.expected
// were recorded on Windows) and Wine's dlls/msvcrt.
import test from 'node:test';
import assert from 'node:assert/strict';
import { shimEnv, msvcrt } from './shim-harness.mjs';
import { formatPrintf, FormatError } from '../../runtime/src/win32/printf.mjs';
import { FILE_SIZE } from '../../runtime/src/win32/msvcrt.mjs';

const enc = new TextEncoder();
const dec = new TextDecoder('latin1');
const neg = (v) => BigInt.asUintN(64, BigInt(v));
const i32 = (v) => Number(BigInt.asIntN(32, v));
const SIG_ERR = 0xffff_ffff_ffff_ffffn;

/** formatPrintf over JS values: numbers/BigInts are slots, strings are guest strings. */
function fmt(f, ...vals) {
  const env = shimEnv();
  const slots = vals.map((v) => (typeof v === 'string' ? env.put(v) : BigInt.asUintN(64, BigInt(v))));
  let i = 0;
  const next = () => slots[i++];
  const out = formatPrintf(enc.encode(f), {
    int: () => Number(BigInt.asIntN(32, next())),
    i64: () => BigInt.asIntN(64, next()),
    ptr: () => next(),
    str: (a, max) => { const s = enc.encode(env.cstr(a)); return max < 0 ? s : s.subarray(0, max); },
    wstr: () => { throw new Error('unused'); },
  });
  return dec.decode(out);
}

test('printf: the formats of l2_crt.expected (recorded with msvcrt.dll)', () => {
  assert.equal(fmt('[%d] [%i] [%u] [%x] [%X] [%o]', -42, 42, 4000000000, 48879, 48879, 511), '[-42] [42] [4000000000] [beef] [BEEF] [777]');
  assert.equal(fmt('[%c] [%s] [%5d] [%-5d] [%05d] [%+d]', 0x5a, 'str', 42, 42, 42, 42), '[Z] [str] [   42] [42   ] [00042] [+42]');
  assert.equal(fmt('[%8s] [%-8s] [%.3s] [%8.3s]', 'right', 'left', 'truncate', 'abcdef'), '[   right] [left    ] [tru] [     abc]');
  assert.equal(fmt('[%#x] [%#X] [%#o] [%08x] [%-8x]', 255, 255, 8, 0xbeef, 0xbeef), '[0xff] [0XFF] [010] [0000beef] [beef    ]');
  assert.equal(fmt('[%lld] [%llu] [%llx] [%lld]', -9000000000000000000n, 18000000000000000000n, 0xdeadbeefcafen, -(2n ** 63n)),
    '[-9000000000000000000] [18000000000000000000] [deadbeefcafe] [-9223372036854775808]');
  assert.equal(fmt('[%ld] [%lu] [%hd] [%5.3d]', -123456, 4000000000, neg(-5), 7), '[-123456] [4000000000] [-5] [  007]');
  assert.equal(fmt('[%*d] [%-*d] [%%] [100%%]', 6, 12, 6, 12), '[    12] [12    ] [%] [100%]');
  assert.equal(fmt('[%d%%] [%c%c%c]', 50, 0x61, 0x62, 0x63), '[50%] [abc]');
  assert.equal(fmt('%s=%016llx%016llx', 'x', 1, 0xabcn), 'x=00000000000000010000000000000abc');
});

test('printf: msvcrt details (Wine printf.h)', () => {
  assert.equal(fmt('%d', 0x1_0000_0005n), '5', 'int arguments use the low 32 bits');
  assert.equal(fmt('%I64d %I32d %Iu %zu %jd', -1n, neg(-2), 2n ** 40n, 7, -3n), '-1 -2 1099511627776 7 -3');
  assert.equal(fmt('%hhd %hu', 0x1ff, neg(-1)), '511 65535', 'hh is h in msvcrt');
  assert.equal(fmt('[%.0d] [%.0x] [%#x] [%#o]', 0, 0, 0, 0), '[] [] [0] [0]');
  assert.equal(fmt('[% d] [% d] [%+u] [%+ d]', 5, -5, 5, 5), '[ 5] [-5] [5] [+5]');
  assert.equal(fmt('[%-05d] [%05s] [%#010x] [%-#10x]', -7, 'ab', 0x1f, 0x1f), '[-7   ] [000ab] [0x0000001f] [0x1f      ]');
  assert.equal(fmt('[%05.3d] [%-*d] [%.*s]', 7, -4, 1, 2, 'xyz'), '[00007] [1   ] [xy]');
  assert.equal(fmt('[%p] [%10p]', 0x1234, 0xab), '[0000000000001234] [00000000000000AB]');
  assert.equal(fmt('[%s] [%.2s]', 0, 0), '[(null)] [(n]');
  assert.equal(fmt('[%y] [%', 1), '[y] [', 'an unknown conversion prints its character');
  assert.equal(fmt('%Lx %Nd', 255, 1), 'ff 1', 'L, F and N prefixes are skipped');
  assert.throws(() => fmt('%f', 0), FormatError);
  assert.throws(() => fmt('%n', 0), FormatError);
});

test('stdio: _iob, text mode, fwrite/fputc/fputs/puts/putchar/fprintf/vfprintf/printf', () => {
  const env = shimEnv();
  const iob = msvcrt(env, '__iob_func');
  assert.equal(msvcrt(env, '__iob_func'), iob);
  assert.equal(iob % 16n, 0n);
  assert.deepEqual([0, 1, 2].map((i) => [env.mem.u32(iob + FILE_SIZE * BigInt(i) + 24n), env.mem.u32(iob + FILE_SIZE * BigInt(i) + 28n)]),
    [[1, 0], [2, 1], [2, 2]], '_flag _IOREAD/_IOWRT, _file 0/1/2');
  const stdout = iob + FILE_SIZE, stderr = iob + 2n * FILE_SIZE;
  assert.equal(msvcrt(env, 'fwrite', env.put('ab\ncd'), 1, 5, stdout), 5n);
  assert.equal(msvcrt(env, 'fwrite', env.put('xyzw'), 2, 2, stdout), 2n);
  assert.equal(msvcrt(env, 'fwrite', env.put(''), 0, 9, stdout), 0n);
  assert.equal(msvcrt(env, 'fputc', 0x121, stdout), 0x21n);
  assert.equal(msvcrt(env, 'fputs', env.put('fp\n'), stdout), 0n);
  assert.equal(msvcrt(env, 'puts', env.put('line')), 0n);
  assert.equal(msvcrt(env, 'putchar', 0x0a), 0x0an);
  assert.equal(msvcrt(env, 'fprintf', stderr, env.put('%s=%d\n'), env.put('e'), 7), 4n, 'bytes before LF -> CR LF');
  assert.equal(msvcrt(env, 'fprintf', stdout, env.put('%d %d %d %d\n'), 1, 2, 3, 4), 8n, 'stack arguments');
  const va = env.zeros(16);
  env.mem.setU64(va, 42n);
  env.mem.setU64(va + 8n, env.put('v'));
  assert.equal(msvcrt(env, 'vfprintf', stdout, env.put('[%d %s]'), va), 6n);
  assert.equal(msvcrt(env, 'printf', env.put('%c\n'), 0x70), 2n);
  assert.equal(env.stdout(), 'ab\r\ncdxyzw!fp\r\nline\r\n\r\n1 2 3 4\r\n[42 v]p\r\n');
  assert.equal(env.stderr(), 'e=7\r\n');
  assert.match(env.stop('msvcrt.dll', 'fputc', 0x41, iob).error, /writing to stdin/);
  assert.match(env.stop('msvcrt.dll', 'fputc', 0x41, iob + 8n).error, /is not stdin, stdout or stderr/);
  assert.match(env.stop('msvcrt.dll', 'printf', env.put('%f'), 0).error, /floating-point conversion %f is not supported/);
});

test('fopen: the Phase 1 file system is empty', () => {
  const env = shimEnv();
  const errno = msvcrt(env, '_errno');
  assert.equal(msvcrt(env, 'fopen', env.put('this\\file\\does\\not\\exist.txt'), env.put('rb')), 0n);
  assert.equal(env.mem.u32(errno), 2, 'ENOENT');
  assert.match(env.stop('msvcrt.dll', 'fopen', env.put('out.txt'), env.put('w')).error, /creating or writing files is not supported/);
  assert.match(env.stop('msvcrt.dll', 'fopen', env.put('x'), env.put('q')).error, /mode "q" is not supported/);
});

test('heap: malloc, calloc, realloc, free; 16-byte aligned, deterministic reuse; invalid free stops', () => {
  const env = shimEnv();
  const a = msvcrt(env, 'malloc', 100);
  const b = msvcrt(env, 'malloc', 0);
  assert.equal(a % 16n, 0n);
  assert.ok(b >= a + 112n, 'distinct blocks; size 0 gets a block');
  env.mem.fill(a, 100, 0xee);
  msvcrt(env, 'free', a);
  const c = msvcrt(env, 'calloc', 10, 10);
  assert.equal(c, a, 'first fit reuses the freed block');
  assert.deepEqual([...env.mem.read(c, 100)], new Array(100).fill(0), 'calloc zeroes reused memory');
  const errno = msvcrt(env, '_errno');
  assert.equal(msvcrt(env, 'calloc', 1n << 62n, 8), 0n);
  assert.equal(env.mem.u32(errno), 12, 'ENOMEM');
  // realloc: grow (moves when the next block is taken), shrink in place, NULL, size 0.
  env.mem.write(c, enc.encode('0123456789'));
  const d = msvcrt(env, 'realloc', c, 5000);
  assert.notEqual(d, c);
  assert.equal(env.cstr(d).slice(0, 10), '0123456789');
  assert.equal(msvcrt(env, 'realloc', d, 40), d);
  const e = msvcrt(env, 'realloc', 0, 16);
  assert.ok(e !== 0n);
  assert.equal(msvcrt(env, 'realloc', e, 0), 0n);
  assert.equal(msvcrt(env, 'free', 0), undefined);
  assert.match(env.stop('msvcrt.dll', 'free', e).error, /free of 0x[0-9a-f]+, which is not an allocated block/);
  assert.match(env.stop('msvcrt.dll', 'free', d + 16n).error, /not an allocated block/);
  assert.match(env.stop('msvcrt.dll', 'realloc', d + 16n, 8).error, /realloc of 0x[0-9a-f]+, which is not an allocated block/);
  // Growing into the free block that follows stays in place.
  const f = msvcrt(env, 'malloc', 32), g = msvcrt(env, 'malloc', 32);
  msvcrt(env, 'free', g);
  assert.equal(msvcrt(env, 'realloc', f, 64), f);
});

test('memory and string functions', () => {
  const env = shimEnv();
  const buf = env.put('0123456789abcdef');
  assert.equal(msvcrt(env, 'memmove', buf + 4n, buf, 12), buf + 4n);
  assert.equal(env.cstr(buf), '01230123456789ab');
  env.mem.write(buf, enc.encode('0123456789abcdef'));
  assert.equal(msvcrt(env, 'memcpy', buf, buf + 5n, 11), buf);
  assert.equal(env.cstr(buf).slice(0, 11), '56789abcdef');
  assert.equal(msvcrt(env, 'memset', buf, 0x171, 3), buf);
  assert.equal(env.cstr(buf).slice(0, 4), 'qqq8');
  assert.equal(msvcrt(env, 'memcpy', 0, 0, 0), 0n, 'zero length touches nothing');
  assert.equal(msvcrt(env, 'memcmp', env.put('abcdef'), env.put('abcdeg'), 6), neg(-1));
  assert.equal(msvcrt(env, 'memcmp', env.put('b\xff'), env.put('a'), 1), 1n);
  assert.equal(msvcrt(env, 'memcmp', 0, 0, 0), 0n);
  assert.equal(msvcrt(env, 'strlen', env.put('hello, world')), 12n);
  assert.equal(msvcrt(env, 'strlen', env.put('')), 0n);
  assert.equal(msvcrt(env, 'wcslen', env.putW('wide')), 4n);
  assert.equal(msvcrt(env, 'strcmp', env.put('abc'), env.put('abd')), neg(-1));
  assert.equal(msvcrt(env, 'strcmp', env.put('b'), env.put('a')), 1n);
  assert.equal(msvcrt(env, 'strcmp', env.put('abc'), env.put('abc')), 0n);
  assert.equal(msvcrt(env, 'strncmp', env.put('abcdef'), env.put('abcxyz'), 3), 0n);
  assert.equal(msvcrt(env, 'strncmp', env.put('abcdef'), env.put('abcxyz'), 4), neg(-1));
  assert.equal(msvcrt(env, 'strncmp', env.put('ab'), env.put('abc'), 0), 0n);
  assert.equal(msvcrt(env, 'strncmp', env.put('ab'), env.put('ab'), 100), 0n);
  const t = env.put('find the needle');
  assert.equal(msvcrt(env, 'strchr', t, 0x6e), t + 2n);
  assert.equal(msvcrt(env, 'strchr', t, 0x5a), 0n);
  assert.equal(msvcrt(env, 'strchr', t, 0x100), t + 15n, "(char)c == 0 finds the NUL");
});

test('strtol, _strtoi64, _strtoui64, atoi, div; errno', () => {
  const env = shimEnv();
  const errno = msvcrt(env, '_errno');
  const end = env.zeros(8);
  const strtol = (s, base) => {
    const p = env.put(s);
    env.mem.setU32(errno, 0);
    const v = msvcrt(env, 'strtol', p, end, base);
    return [i32(v), env.mem.u32(errno), Number(env.mem.u64(end) - p)];
  };
  assert.deepEqual(strtol('99999999999999999999', 10), [2147483647, 34, 20]);
  assert.deepEqual(strtol('-0x7f xyz', 16), [-127, 0, 5]);
  assert.deepEqual(strtol('  -2147483649', 10), [-2147483648, 34, 13]);
  assert.deepEqual(strtol('0777', 0), [511, 0, 4]);
  assert.deepEqual(strtol('0x', 0), [0, 0, 0], 'no digits: end = nptr');
  assert.deepEqual(strtol('zz', 36), [1295, 0, 2]);
  assert.deepEqual(strtol('+12a', 10), [12, 0, 3]);
  assert.match(env.stop('msvcrt.dll', 'strtol', env.put('1'), 0, 1).error, /invalid base 1/);
  env.mem.setU32(errno, 0);
  assert.equal(msvcrt(env, '_strtoui64', env.put('18446744073709551616'), 0, 10), 0xffff_ffff_ffff_ffffn);
  assert.equal(env.mem.u32(errno), 34);
  env.mem.setU32(errno, 0);
  assert.equal(msvcrt(env, '_strtoui64', env.put('-1'), 0, 10), 0xffff_ffff_ffff_ffffn);
  assert.equal(msvcrt(env, '_strtoi64', env.put('-9223372036854775808'), 0, 10), 1n << 63n);
  assert.equal(env.mem.u32(errno), 0);
  assert.deepEqual(['  123abc', '-77', 'zzz', '+5', '4294967297'].map((s) => i32(msvcrt(env, 'atoi', env.put(s)))), [123, -77, 0, 5, 1]);
  assert.equal(msvcrt(env, 'atoi', 0), 0n);
  const d = msvcrt(env, 'div', neg(-17), 5);
  assert.deepEqual([i32(d & 0xffff_ffffn), i32(d >> 32n)], [-3, -2]);
  assert.match(env.stop('msvcrt.dll', 'div', 1, 0).error, /division by zero/);
});

test('strerror: msvcrt.dll texts in a per-thread buffer', () => {
  const env = shimEnv();
  const s = (n) => env.cstr(msvcrt(env, 'strerror', n));
  assert.deepEqual([17, 13, 2, 22].map(s), ['File exists', 'Permission denied', 'No such file or directory', 'Invalid argument']);
  assert.deepEqual([0, 12, 33, 34, 42, 43, 99, -1].map(s),
    ['No error', 'Not enough space', 'Domain error', 'Result too large', 'Illegal byte sequence', 'Unknown error', 'Unknown error', 'Unknown error']);
  assert.equal(msvcrt(env, 'strerror', 1), msvcrt(env, 'strerror', 2), 'one buffer');
});

test('qsort calls the guest comparator with Wine/native comparisons', () => {
  const env = shimEnv();
  const n = 64;
  const vals = [];
  let s = 12345;
  for (let i = 0; i < n; i++) { s = (Math.imul(s, 1103515245) + 12345) >>> 0; vals.push(((s >>> 8) % 100000) * 64 + i); }
  const base = env.zeros(4 * n);
  vals.forEach((v, i) => env.mem.setU32(base + 4n * BigInt(i), v));
  let compares = 0;
  const cmp = env.guest((x, y) => {
    compares++;
    const a = i32(BigInt(env.mem.u32(x))), b = i32(BigInt(env.mem.u32(y)));
    return BigInt.asUintN(64, BigInt((a > b) - (a < b)));
  });
  assert.equal(msvcrt(env, 'qsort', base, n, 4, cmp), undefined);
  const sorted = Array.from({ length: n }, (_, i) => env.mem.u32(base + 4n * BigInt(i)));
  assert.deepEqual(sorted, [...vals].sort((a, b) => a - b));
  assert.deepEqual([sorted[0], sorted[32], sorted[63]], [15072, 2429545, 6383352], 'l2_crt.expected');
  assert.ok(compares > 0 && env.calls.every(([t]) => t === cmp));
  // Records of 12 bytes, descending, through the same algorithm.
  const r = env.zeros(120);
  for (let i = 0; i < 10; i++) env.mem.setU32(r + 12n * BigInt(i), ((i * 7 + 3) % 10) * 100 + i);
  const desc = env.guest((x, y) => { const a = env.mem.u32(x), b = env.mem.u32(y); return BigInt.asUintN(64, BigInt((b > a) - (b < a))); });
  msvcrt(env, 'qsort', r, 10, 12, desc);
  assert.deepEqual(Array.from({ length: 10 }, (_, i) => env.mem.u32(r + 12n * BigInt(i))), [908, 805, 702, 609, 506, 403, 300, 207, 104, 1]);
  assert.equal(msvcrt(env, 'qsort', 0, 0, 4, cmp), undefined);
  assert.match(env.stop('msvcrt.dll', 'qsort', base, 2, 0, cmp).error, /size is 0/);
});

test('startup: __getmainargs, __set_app_type, __setusermatherr, _initterm, data imports', () => {
  const env = shimEnv({ argv: ['l2.exe', 'one'] });
  const [argc, argv, envp, info] = [env.zeros(4), env.zeros(8), env.zeros(8), env.zeros(4)];
  assert.equal(msvcrt(env, '__getmainargs', argc, argv, envp, 0, info), 0n);
  assert.equal(env.mem.u32(argc), 2);
  const av = env.mem.u64(argv);
  assert.deepEqual([env.cstr(env.mem.u64(av)), env.cstr(env.mem.u64(av + 8n)), env.mem.u64(av + 16n)], ['l2.exe', 'one', 0n]);
  assert.equal(env.mem.u64(env.mem.u64(envp)), 0n, 'empty environment');
  env.mem.setU32(info, 3);
  assert.match(env.stop('msvcrt.dll', '__getmainargs', argc, argv, envp, 0, info).error, /newmode 3/);
  assert.equal(msvcrt(env, '__set_app_type', 1), undefined);
  assert.equal(msvcrt(env, '__setusermatherr', 0x1234), undefined);
  const order = [];
  const f1 = env.guest(() => { order.push(1); }), f2 = env.guest(() => { order.push(2); });
  const table = env.zeros(32);
  [f1, 0n, f2].forEach((f, i) => env.mem.setU64(table + 8n * BigInt(i), f));
  env.mem.setU64(table + 24n, f1);
  assert.equal(msvcrt(env, '_initterm', table, table + 24n), undefined);
  assert.deepEqual(order, [1, 2], 'non-null entries in [start, end)');
});

test('exit: handlers in reverse order once, then the exit code; _cexit; _amsg_exit; abort and signal', () => {
  const env = shimEnv();
  const order = [];
  const h = (n) => env.guest(() => { order.push(n); });
  const [a, b, c] = [h(1), h(2), h(3)];
  assert.equal(msvcrt(env, '_onexit', a), a);
  assert.equal(msvcrt(env, 'atexit', b), 0n);
  assert.equal(msvcrt(env, '_onexit', 0), 0n);
  assert.equal(msvcrt(env, 'atexit', 0), 0n);
  assert.equal(msvcrt(env, '_onexit', c), c);
  assert.deepEqual(env.stop('msvcrt.dll', 'exit', 37), { exitCode: 37 });
  assert.deepEqual(order, [3, 2, 1]);
  assert.deepEqual(env.stop('msvcrt.dll', 'exit', 0), { exitCode: 0 });
  assert.deepEqual(order, [3, 2, 1], 'the table is emptied before it runs');
  msvcrt(env, '_onexit', a);
  assert.equal(msvcrt(env, '_cexit'), undefined);
  assert.deepEqual(order, [3, 2, 1, 1]);
  // A handler that exits stops the process with its code.
  msvcrt(env, '_onexit', env.guest(() => ({ exitCode: 9 })));
  assert.deepEqual(env.stop('msvcrt.dll', 'exit', 1), { exitCode: 9 });

  assert.deepEqual(env.stop('msvcrt.dll', '_amsg_exit', 8), { exitCode: 255 });
  assert.equal(env.stderr(), '\r\nruntime error R608\r\n');
  assert.deepEqual(env.stop('msvcrt.dll', 'abort'), { exitCode: 3 });
  const seen = [];
  const handler = env.guest((sig) => { seen.push(sig); });
  assert.equal(msvcrt(env, 'signal', 22, handler), 0n, 'SIG_DFL before');
  assert.equal(msvcrt(env, 'signal', 22, handler), handler);
  assert.equal(msvcrt(env, 'signal', 99, handler), SIG_ERR);
  assert.equal(msvcrt(env, 'signal', 22, SIG_ERR), SIG_ERR);
  assert.deepEqual(env.stop('msvcrt.dll', 'abort'), { exitCode: 3 });
  assert.deepEqual(seen, [22n]);
  assert.equal(msvcrt(env, 'signal', 22, 1), 0n, 'reset to SIG_DFL by raise');
  assert.deepEqual(env.stop('msvcrt.dll', 'abort'), { exitCode: 3 }, 'SIG_IGN: raise returns, then _exit(3)');
  assert.match(env.stop('msvcrt.dll', '__C_specific_handler', 0, 0, 0, 0).error, /structured exception handling/);
});

test('locks, locale and errno', () => {
  const env = shimEnv();
  assert.equal(msvcrt(env, '_lock', 13), undefined);
  assert.equal(msvcrt(env, '_lock', 13), undefined);
  msvcrt(env, '_unlock', 13);
  msvcrt(env, '_unlock', 13);
  assert.match(env.stop('msvcrt.dll', '_unlock', 13).error, /lock 13 is not held/);
  assert.match(env.stop('msvcrt.dll', '_lock', 48).error, /out of range/);
  assert.equal(msvcrt(env, '___lc_codepage_func'), 0n);
  assert.equal(msvcrt(env, '___mb_cur_max_func'), 1n);
  const lc = msvcrt(env, 'localeconv');
  assert.equal(msvcrt(env, 'localeconv'), lc);
  assert.equal(env.cstr(env.mem.u64(lc)), '.');
  for (let i = 1n; i < 10n; i++) assert.equal(env.cstr(env.mem.u64(lc + 8n * i)), '');
  assert.deepEqual([...env.mem.read(lc + 80n, 8)], new Array(8).fill(127));
  const e = msvcrt(env, '_errno');
  assert.equal(e % 4n, 0n);
  assert.equal(msvcrt(env, '_errno'), e);
  assert.equal(env.mem.u32(e), 0);
});

test('data imports of msvcrt: _fmode, _commode, __initenv', async () => {
  const { SHIM } = await import('../../runtime/src/win32/index.mjs');
  const m = SHIM['msvcrt.dll'];
  assert.deepEqual(m._fmode.data, { size: 4, init: 0x4000 });
  assert.deepEqual(m._commode.data, { size: 4, init: 0 });
  assert.deepEqual(m.__initenv.data, { size: 8, init: 0 });
});
