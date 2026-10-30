// msvcrt.dll for Phase 1 (runtime.md, "msvcrt (M3)"). Observable behavior
// follows msvcrt.dll on the reference runner; the details follow Wine 11's
// dlls/msvcrt (file.c, printf.h, heap.c, exit.c, data.c, string.c, misc.c,
// errno.c, except.c, locale.c, lock.c). Cases the shim doesn't handle stop
// with an error; they never return a made-up success.

import { CrtHeap, HeapError } from './crtheap.mjs';
import { formatPrintf, FormatError } from './printf.mjs';

const hex = (v) => '0x' + BigInt.asUintN(64, BigInt(v)).toString(16);
const enc = new TextEncoder();
const I32 = (v) => Number(BigInt.asIntN(32, BigInt(v)));

/** msvcrt FILE: 48 bytes (_ptr, _cnt, _base, _flag, _file, _charbuf, _bufsiz, _tmpfname). */
export const FILE_SIZE = 48n;
export const FILE_FLAG = 24n;
export const FILE_FILE = 28n;
export const _IOREAD = 0x1;
export const _IOWRT = 0x2;
export const _O_TEXT = 0x4000;
export const EOF = 0xffff_ffff; // (int)-1 as the u32 a function returns in EAX

export const EPERM = 1, ENOENT = 2, EBADF = 9, ENOMEM = 12, EACCES = 13, EEXIST = 17, EINVAL = 22, ERANGE = 34;
/** msvcrt.dll's _sys_errlist (errno 0..42, then "Unknown error"); Wine's own texts differ. */
export const SYS_ERRLIST = [
  'No error', 'Operation not permitted', 'No such file or directory', 'No such process',
  'Interrupted function call', 'Input/output error', 'No such device or address', 'Arg list too long',
  'Exec format error', 'Bad file descriptor', 'No child processes', 'Resource temporarily unavailable',
  'Not enough space', 'Permission denied', 'Bad address', 'Unknown error', 'Resource device',
  'File exists', 'Improper link', 'No such device', 'Not a directory', 'Is a directory',
  'Invalid argument', 'Too many open files in system', 'Too many open files',
  'Inappropriate I/O control operation', 'Unknown error', 'File too large', 'No space left on device',
  'Invalid seek', 'Read-only file system', 'Too many links', 'Broken pipe', 'Domain error',
  'Result too large', 'Unknown error', 'Resource deadlock avoided', 'Unknown error',
  'Filename too long', 'No locks available', 'Function not implemented', 'Directory not empty',
  'Illegal byte sequence', 'Unknown error',
];

export const SIGINT = 2, SIGILL = 4, SIGFPE = 8, SIGSEGV = 11, SIGTERM = 15, SIGBREAK = 21, SIGABRT = 22;
const SIGNALS = [SIGINT, SIGILL, SIGFPE, SIGSEGV, SIGTERM, SIGBREAK, SIGABRT];
export const SIG_DFL = 0n, SIG_IGN = 1n, SIG_ERR = 0xffff_ffff_ffff_ffffn;
/** _TOTAL_LOCKS of msvcrt (Wine's mtdll.h): _STREAM_LOCKS + _IOB_ENTRIES. */
export const TOTAL_LOCKS = 48;
/** The largest request malloc accepts (_HEAP_MAXREQ). */
const HEAP_MAXREQ = 0xffff_ffff_ffff_ffe0n;

/** The process's CRT state, created on first use. */
export function crt(ctx) {
  return ctx.dllState('msvcrt.dll', () => {
    const iob = ctx.alloc(3n * FILE_SIZE);
    for (let i = 0; i < 3; i++) {
      ctx.write32(iob + FILE_SIZE * BigInt(i) + FILE_FILE, i);
      ctx.write32(iob + FILE_SIZE * BigInt(i) + FILE_FLAG, i === 0 ? _IOREAD : _IOWRT);
    }
    const host = ctx.host;
    return {
      iob,
      errno: ctx.alloc(4n),
      heap: new CrtHeap((size) => host.alloc(size, 16n)),
      onexit: [],
      appType: 0,
      userMathErr: 0n,
      newMode: 0,
      signals: new Map(SIGNALS.map((s) => [s, SIG_DFL])),
      locks: new Array(TOTAL_LOCKS).fill(0),
      strerrorBuffer: 0n,
      lconv: 0n,
      mainArgs: null,
    };
  });
}

const setErrno = (ctx, v) => ctx.write32(crt(ctx).errno, v);

function heapCall(ctx, f) {
  try { return f(crt(ctx).heap); } catch (e) {
    if (e instanceof HeapError) ctx.fail(e.message);
    throw e;
  }
}

function malloc(ctx, size) {
  if (size > HEAP_MAXREQ) { setErrno(ctx, ENOMEM); return 0n; }
  return heapCall(ctx, (h) => h.alloc(size));
}

function free(ctx, p) {
  if (p !== 0n) heapCall(ctx, (h) => h.freeBlock(p));
}

/** Copies `len` guest bytes from `src` to `dst`; overlapping ranges are fine. */
function copy(ctx, dst, src, len) {
  if (len === 0n) return;
  const n = Number(len);
  ctx.check(dst, n, 'destination');
  ctx.check(src, n, 'source');
  const u8 = ctx.mem.u8;
  u8.copyWithin(Number(dst), Number(src), Number(src) + n);
}

/** The bytes of a C string, at most `max` of them (max < 0: up to the NUL). */
function cstr(ctx, addr, max = -1) {
  if (max < 0) return ctx.readCString(addr);
  const start = ctx.mem.at(ctx.check(addr, 1, 'string'), 1);
  const u8 = ctx.mem.u8;
  let end = start;
  while (end - start < max && end < u8.length && u8[end] !== 0) end++;
  return u8.slice(start, end);
}

// --- stdio ---------------------------------------------------------------

/** The stream index (0 stdin, 1 stdout, 2 stderr) of a FILE pointer. */
function streamIndex(ctx, file) {
  const { iob } = crt(ctx);
  const off = file - iob;
  if (off >= 0n && off < 3n * FILE_SIZE && off % FILE_SIZE === 0n) return Number(off / FILE_SIZE);
  ctx.fail(`FILE ${hex(file)} is not stdin, stdout or stderr (no other stream is open)`);
}

/** Writes to stdout or stderr, which are in text mode: LF becomes CR LF. */
function writeStream(ctx, index, bytes) {
  if (index === 0) ctx.fail('writing to stdin');
  let lf = 0;
  for (const b of bytes) if (b === 0x0a) lf++;
  let out = bytes;
  if (lf) {
    out = new Uint8Array(bytes.length + lf);
    let j = 0;
    for (const b of bytes) {
      if (b === 0x0a) out[j++] = 0x0d;
      out[j++] = b;
    }
  }
  if (out.length) ctx.output(index === 1 ? 'stdout' : 'stderr', out);
}

/** The printf argument reader over `slot(i)` (the i-th 8-byte variadic slot). */
function printfArgs(ctx, slot) {
  let i = 0;
  const next = () => slot(i++);
  return {
    int: () => I32(next()),
    i64: () => BigInt.asIntN(64, next()),
    ptr: () => next(),
    str: (addr, max) => cstr(ctx, addr, max),
    wstr: (addr, max) => {
      const out = [];
      for (let a = addr; max < 0 || out.length < max; a += 2n) {
        const u = ctx.mem.u16(ctx.check(a, 2, 'string'));
        if (u === 0) break;
        out.push(u);
      }
      return out;
    },
  };
}

function printfTo(ctx, index, fmtAddr, slot) {
  if (fmtAddr === 0n) ctx.fail('format is NULL');
  let bytes;
  try {
    bytes = formatPrintf(ctx.readCString(fmtAddr), printfArgs(ctx, slot));
  } catch (e) {
    if (e instanceof FormatError) ctx.fail(`format "${new TextDecoder().decode(ctx.readCString(fmtAddr))}": ${e.message}`);
    throw e;
  }
  writeStream(ctx, index, bytes);
  return bytes.length;
}

// --- exit ------------------------------------------------------------------

/** Runs the atexit/_onexit handlers in reverse order (Wine: execute_onexit_table). */
function runOnexit(ctx) {
  const c = crt(ctx);
  const table = c.onexit;
  c.onexit = [];
  for (let i = table.length - 1; i >= 0; i--) ctx.call(table[i]);
}

/** raise(sig) for the signals abort uses: a guest handler runs once (reset to SIG_DFL first). */
function raise(ctx, sig) {
  const c = crt(ctx);
  const handler = c.signals.get(sig);
  if (handler === SIG_DFL) ctx.exit(3);
  if (handler !== SIG_IGN) {
    c.signals.set(sig, SIG_DFL);
    ctx.call(handler, sig);
  }
}

// --- strings to numbers ----------------------------------------------------

const isSpace = (c) => c === 0x20 || (c >= 0x09 && c <= 0x0d);
const lower = (c) => (c >= 0x41 && c <= 0x5a ? c + 0x20 : c);

/**
 * Wine's _strtoi64_l / _strtoui64_l: returns {value (BigInt), end (address),
 * overflow}. `unsigned` saturates at 2^64-1 and negates afterwards.
 */
function parseInteger(ctx, nptr, base, unsigned) {
  if (nptr === 0n) ctx.fail('nptr is NULL');
  if (!(base === 0 || (base >= 2 && base <= 36))) ctx.fail(`invalid base ${base}`);
  const s = ctx.readCString(nptr);
  let i = 0;
  while (i < s.length && isSpace(s[i])) i++;
  let negative = false;
  if (s[i] === 0x2d) { negative = true; i++; } else if (s[i] === 0x2b) i++;
  if ((base === 0 || base === 16) && s[i] === 0x30 && lower(s[i + 1] ?? 0) === 0x78) { base = 16; i += 2; }
  if (base === 0) base = s[i] === 0x30 ? 8 : 10;
  const B = BigInt(base);
  const MAX = unsigned ? 0xffff_ffff_ffff_ffffn : 0x7fff_ffff_ffff_ffffn;
  const MIN = -0x8000_0000_0000_0000n;
  let value = 0n, gotDigit = false, overflow = false;
  for (; i < s.length; i++) {
    const c = lower(s[i]);
    let v;
    if (c >= 0x30 && c <= 0x39) { if (c >= 0x30 + base) break; v = c - 0x30; }
    else { if (c < 0x61 || c >= 0x61 + base - 10) break; v = c - 0x61 + 10; }
    gotDigit = true;
    const V = BigInt(v);
    if (unsigned || !negative) {
      if (value * B + V > MAX) { value = MAX; overflow = true; } else value = value * B + V;
    } else if (value * B - V < MIN) { value = MIN; overflow = true; } else value = value * B - V;
  }
  if (unsigned && negative) value = BigInt.asUintN(64, -value);
  return { value, end: gotDigit ? nptr + BigInt(i) : nptr, overflow };
}

function strtoX(ctx, unsigned, clamp) {
  const nptr = ctx.arg(0), endptr = ctx.arg(1), base = ctx.argI32(2);
  let { value, end, overflow } = parseInteger(ctx, nptr, base, unsigned);
  if (clamp) {
    if (value > clamp.max) { value = clamp.max; overflow = true; } else if (value < clamp.min) { value = clamp.min; overflow = true; }
  }
  if (overflow) setErrno(ctx, ERANGE);
  if (endptr !== 0n) ctx.write64(endptr, end);
  return BigInt.asUintN(64, value);
}

// --- qsort (Wine misc.c: quick_sort and small_sort, identical comparisons to native) ---

function qsortImpl(ctx, base, nmemb, size, compar) {
  const cmp = (a, b) => I32(ctx.call(compar, a, b));
  const X = (i) => base + size * i;
  const swap = (a, b) => {
    const n = Number(size);
    const x = ctx.read(a, n);
    ctx.write(a, ctx.read(b, n));
    ctx.write(b, x);
  };
  const smallSort = (b0, n) => {
    for (let e = n; e > 1n; e--) {
      let max = b0, p = b0;
      for (let i = 1n; i < e; i++) {
        p = b0 + i * size;
        if (cmp(p, max) > 0) max = p;
      }
      if (p !== max) swap(p, max);
    }
  };
  const stack = [[0n, nmemb - 1n]];
  while (stack.length) {
    const [beg, end] = stack.pop();
    if (end - beg < 8n) { smallSort(X(beg), end - beg + 1n); continue; }
    let lo = beg, hi = end;
    let med = lo + (hi - lo + 1n) / 2n;
    if (cmp(X(lo), X(med)) > 0) swap(X(lo), X(med));
    if (cmp(X(lo), X(hi)) > 0) swap(X(lo), X(hi));
    if (cmp(X(med), X(hi)) > 0) swap(X(med), X(hi));
    lo++;
    hi--;
    for (;;) {
      while (lo <= hi) {
        if (lo !== med && cmp(X(lo), X(med)) > 0) break;
        lo++;
      }
      while (med !== hi) {
        if (cmp(X(hi), X(med)) <= 0) break;
        hi--;
      }
      if (hi < lo) break;
      swap(X(lo), X(hi));
      if (hi === med) med = lo;
      lo++;
      hi--;
    }
    while (hi > beg) {
      if (hi !== med && cmp(X(hi), X(med)) !== 0) break;
      hi--;
    }
    // Wine pushes the larger part first, so the smaller one is sorted next.
    if (hi - beg >= end - lo) { stack.push([beg, hi], [lo, end]); } else { stack.push([lo, end], [beg, hi]); }
  }
}

// --- the shim --------------------------------------------------------------

const fn = (args, f) => ({ args, fn: f });

export default {
  // Data imports (runtime.md, "Loading", step 4).
  _fmode: { data: { size: 4, init: _O_TEXT } },
  _commode: { data: { size: 4, init: 0 } },
  __initenv: { data: { size: 8, init: 0 } },

  // --- startup and exit ---
  __set_app_type: fn(1, (ctx) => { crt(ctx).appType = ctx.argI32(0); }),
  __setusermatherr: fn(1, (ctx) => { crt(ctx).userMathErr = ctx.arg(0); }),

  // int __getmainargs(int *argc, char ***argv, char ***envp, int expand_wildcards, _startupinfo *info)
  __getmainargs: fn(5, (ctx) => {
    const c = crt(ctx);
    const [argcP, argvP, envpP] = [ctx.arg(0), ctx.arg(1), ctx.arg(2)];
    const expand = ctx.argI32(3), info = ctx.arg(4);
    if (expand && ctx.argv.some((a) => /[*?]/.test(a))) ctx.fail('wildcard expansion of the command line is not supported');
    if (!c.mainArgs) {
      const ptrs = ctx.argv.map((a) => {
        const bytes = enc.encode(a);
        const p = ctx.alloc(BigInt(bytes.length + 1), 1n);
        ctx.write(p, bytes);
        return p;
      });
      const argv = ctx.alloc(8n * BigInt(ptrs.length + 1), 8n);
      ptrs.forEach((p, i) => ctx.write64(argv + 8n * BigInt(i), p));
      const envp = ctx.alloc(8n, 8n); // the empty environment: { NULL }
      c.mainArgs = { argc: ptrs.length, argv, envp };
    }
    ctx.write32(argcP, c.mainArgs.argc);
    ctx.write64(argvP, c.mainArgs.argv);
    ctx.write64(envpP, c.mainArgs.envp);
    if (info !== 0n) {
      const mode = I32(ctx.read32(info));
      if (mode !== 0 && mode !== 1) ctx.fail(`_startupinfo.newmode ${mode} is not 0 or 1`);
      c.newMode = mode;
    }
    return 0;
  }),

  // void _initterm(_PVFV *start, _PVFV *end): calls each non-null entry.
  _initterm: fn(2, (ctx) => {
    const start = ctx.arg(0), end = ctx.arg(1);
    for (let p = start; p < end; p += 8n) {
      const f = ctx.read64(p);
      if (f !== 0n) ctx.call(f);
    }
  }),

  _onexit: fn(1, (ctx) => {
    const f = ctx.arg(0);
    if (f === 0n) return 0n;
    crt(ctx).onexit.push(f);
    return f;
  }),
  atexit: fn(1, (ctx) => {
    const f = ctx.arg(0);
    if (f !== 0n) crt(ctx).onexit.push(f);
    return 0; // _onexit(f) == f, also for NULL (Wine)
  }),
  _cexit: fn(0, (ctx) => { runOnexit(ctx); }),
  exit: fn(1, (ctx) => {
    const code = ctx.arg32(0);
    runOnexit(ctx);
    ctx.exit(code);
  }),
  _exit: fn(1, (ctx) => { ctx.exit(ctx.arg32(0)); }),

  // _amsg_exit(n): "runtime error R60<n>" on the console (stderr in Phase 1), then _exit(255).
  _amsg_exit: fn(1, (ctx) => {
    const n = ctx.argI32(0);
    if (crt(ctx).appType === 2) ctx.fail(`runtime error R60${n} in a GUI application (message boxes are not supported)`);
    writeStream(ctx, 2, enc.encode(`\nruntime error R60${n}\n`));
    ctx.exit(255);
  }),

  // abort: raise(SIGABRT), then _exit(3) if that returns.
  abort: fn(0, (ctx) => {
    raise(ctx, SIGABRT);
    ctx.exit(3);
  }),

  // __sighandler_t signal(int sig, __sighandler_t func)
  signal: fn(2, (ctx) => {
    const sig = ctx.argI32(0), func = ctx.arg(1);
    if (func === SIG_ERR) return SIG_ERR;
    const c = crt(ctx);
    if (!c.signals.has(sig)) return SIG_ERR;
    const old = c.signals.get(sig);
    c.signals.set(sig, func);
    return old;
  }),

  // Exceptions do not exist in Phase 1; reaching the handler means something faulted.
  __C_specific_handler: fn(4, (ctx) => { ctx.fail('structured exception handling is not supported'); }),

  // --- locks (no-ops with one thread; they still check their arguments) ---
  _lock: fn(1, (ctx) => {
    const n = ctx.argI32(0);
    if (n < 0 || n >= TOTAL_LOCKS) ctx.fail(`lock number ${n} is out of range`);
    crt(ctx).locks[n]++;
  }),
  _unlock: fn(1, (ctx) => {
    const n = ctx.argI32(0);
    if (n < 0 || n >= TOTAL_LOCKS) ctx.fail(`lock number ${n} is out of range`);
    const c = crt(ctx);
    if (c.locks[n] === 0) ctx.fail(`lock ${n} is not held`);
    c.locks[n]--;
  }),

  // --- locale: the C locale ---
  ___lc_codepage_func: fn(0, () => 0),
  ___mb_cur_max_func: fn(0, () => 1),
  localeconv: fn(0, (ctx) => {
    const c = crt(ctx);
    if (!c.lconv) {
      // struct lconv: 10 char* (decimal_point "." and nine ""), then 8 chars of CHAR_MAX.
      const dot = ctx.alloc(2n, 1n);
      ctx.write(dot, enc.encode('.\0'));
      const empty = ctx.alloc(1n, 1n);
      c.lconv = ctx.alloc(88n, 8n);
      ctx.write64(c.lconv, dot);
      for (let i = 1n; i < 10n; i++) ctx.write64(c.lconv + 8n * i, empty);
      ctx.write(c.lconv + 80n, new Uint8Array(8).fill(127));
    }
    return c.lconv;
  }),

  // --- errno ---
  _errno: fn(0, (ctx) => crt(ctx).errno),
  strerror: fn(1, (ctx) => {
    let n = ctx.argI32(0);
    if (n < 0 || n >= SYS_ERRLIST.length) n = SYS_ERRLIST.length - 1;
    const c = crt(ctx);
    if (!c.strerrorBuffer) c.strerrorBuffer = malloc(ctx, 256n);
    ctx.write(c.strerrorBuffer, enc.encode(SYS_ERRLIST[n] + '\0'));
    return c.strerrorBuffer;
  }),

  // --- heap ---
  malloc: fn(1, (ctx) => malloc(ctx, ctx.arg(0))),
  calloc: fn(2, (ctx) => {
    const count = ctx.arg(0), size = ctx.arg(1);
    if (size !== 0n && count > HEAP_MAXREQ / size) { setErrno(ctx, ENOMEM); return 0n; }
    const total = count * size;
    const p = malloc(ctx, total);
    if (p !== 0n && total) ctx.mem.fill(ctx.check(p, Number(total)), Number(total));
    return p;
  }),
  realloc: fn(2, (ctx) => {
    const p = ctx.arg(0), size = ctx.arg(1);
    if (p === 0n) return malloc(ctx, size);
    if (size === 0n) { free(ctx, p); return 0n; }
    if (size > HEAP_MAXREQ) { setErrno(ctx, ENOMEM); return 0n; }
    const old = heapCall(ctx, (h) => h.sizeOf(p));
    if (old === undefined) ctx.fail(`realloc of ${hex(p)}, which is not an allocated block`);
    if (heapCall(ctx, (h) => h.resizeInPlace(p, size))) return p;
    const q = malloc(ctx, size);
    copy(ctx, q, p, old < size ? old : size);
    free(ctx, p);
    return q;
  }),
  free: fn(1, (ctx) => { free(ctx, ctx.arg(0)); }),

  // --- memory and strings ---
  memcpy: fn(3, (ctx) => { copy(ctx, ctx.arg(0), ctx.arg(1), ctx.arg(2)); return ctx.arg(0); }),
  memmove: fn(3, (ctx) => { copy(ctx, ctx.arg(0), ctx.arg(1), ctx.arg(2)); return ctx.arg(0); }),
  memset: fn(3, (ctx) => {
    const dst = ctx.arg(0), n = ctx.arg(2);
    if (n) ctx.mem.fill(ctx.check(dst, Number(n)), Number(n), ctx.arg32(1) & 0xff);
    return dst;
  }),
  memcmp: fn(3, (ctx) => {
    const n = Number(ctx.arg(2));
    if (!n) return 0;
    const a = ctx.read(ctx.arg(0), n), b = ctx.read(ctx.arg(1), n);
    for (let i = 0; i < n; i++) if (a[i] !== b[i]) return a[i] > b[i] ? 1 : -1;
    return 0;
  }),
  strlen: fn(1, (ctx) => ctx.readCString(ctx.arg(0)).length),
  wcslen: fn(1, (ctx) => ctx.readWString(ctx.arg(0)).length),
  strcmp: fn(2, (ctx) => {
    const a = ctx.readCString(ctx.arg(0)), b = ctx.readCString(ctx.arg(1));
    for (let i = 0; ; i++) {
      const x = a[i] ?? 0, y = b[i] ?? 0;
      if (x !== y || x === 0) return x > y ? 1 : x < y ? -1 : 0;
    }
  }),
  strncmp: fn(3, (ctx) => {
    const len = ctx.arg(2);
    if (!len) return 0;
    const max = len > 0x10_0000n ? -1 : Number(len);
    const a = cstr(ctx, ctx.arg(0), max), b = cstr(ctx, ctx.arg(1), max);
    // while (--len && *s1 && *s1 == *s2) advance; then compare (64-bit: -1, 0, 1).
    let i = 0;
    for (let left = len - 1n; left && (a[i] ?? 0) && a[i] === b[i]; left--) i++;
    const x = a[i] ?? 0, y = b[i] ?? 0;
    return x > y ? 1 : x < y ? -1 : 0;
  }),
  strchr: fn(2, (ctx) => {
    const s = ctx.arg(0), c = ctx.arg32(1) & 0xff;
    const bytes = ctx.readCString(s);
    if (c === 0) return s + BigInt(bytes.length);
    const i = bytes.indexOf(c);
    return i < 0 ? 0n : s + BigInt(i);
  }),

  // --- numbers ---
  strtol: fn(3, (ctx) => strtoX(ctx, false, { max: 0x7fff_ffffn, min: -0x8000_0000n }) & 0xffff_ffffn),
  _strtoi64: fn(3, (ctx) => strtoX(ctx, false, null)),
  _strtoui64: fn(3, (ctx) => strtoX(ctx, true, null)),
  // msvcrt.dll's own atoi: optional sign, decimal digits, int wrap-around, no errno.
  atoi: fn(1, (ctx) => {
    const p = ctx.arg(0);
    if (p === 0n) return 0;
    const s = ctx.readCString(p);
    let i = 0;
    while (i < s.length && isSpace(s[i])) i++;
    let minus = false;
    if (s[i] === 0x2b) i++; else if (s[i] === 0x2d) { minus = true; i++; }
    let v = 0;
    for (; i < s.length && s[i] >= 0x30 && s[i] <= 0x39; i++) v = (Math.imul(v, 10) + s[i] - 0x30) | 0;
    return (minus ? -v | 0 : v) >>> 0;
  }),
  // div_t div(int, int): {quot, rem} in RAX (low half quot).
  div: fn(2, (ctx) => {
    const a = ctx.argI32(0), b = ctx.argI32(1);
    if (b === 0) ctx.fail('integer division by zero');
    if (a === -0x8000_0000 && b === -1) ctx.fail('integer overflow in division');
    const q = Math.trunc(a / b), r = a - q * b;
    return (BigInt(r >>> 0) << 32n) | BigInt(q >>> 0);
  }),

  qsort: fn(4, (ctx) => {
    const base = ctx.arg(0), nmemb = ctx.arg(1), size = ctx.arg(2), compar = ctx.arg(3);
    if (base === 0n && nmemb !== 0n) ctx.fail('base is NULL');
    if (size === 0n) ctx.fail('size is 0');
    if (compar === 0n) ctx.fail('compare is NULL');
    if (nmemb < 2n) return;
    ctx.check(base, Number(nmemb * size), 'array');
    qsortImpl(ctx, base, nmemb, size, compar);
  }),

  // --- stdio ---
  __iob_func: fn(0, (ctx) => crt(ctx).iob),

  // FILE *fopen(const char *path, const char *mode): Phase 1 has an empty file system.
  fopen: fn(2, (ctx) => {
    const path = ctx.arg(0), modeP = ctx.arg(1);
    if (path === 0n || modeP === 0n) ctx.fail('path or mode is NULL');
    const name = new TextDecoder().decode(ctx.readCString(path));
    const mode = new TextDecoder().decode(ctx.readCString(modeP));
    const m = /^ *([rwa])(\+?)([btcnDTRS+]*)$/.exec(mode);
    if (!m) ctx.fail(`mode "${mode}" is not supported`);
    if (m[1] !== 'r' || m[2] || m[3].includes('+')) ctx.fail(`fopen("${name}", "${mode}"): creating or writing files is not supported`);
    if (!name) ctx.fail('fopen with an empty path');
    setErrno(ctx, ENOENT); // no file exists
    return 0n;
  }),

  fwrite: fn(4, (ctx) => {
    const ptr = ctx.arg(0), size = ctx.arg(1), nmemb = ctx.arg(2), file = ctx.arg(3);
    const index = streamIndex(ctx, file);
    const total = size * nmemb;
    if (total === 0n) return 0n;
    if (total > 0x7fff_ffffn) ctx.fail(`writing ${total} bytes at once is not supported`);
    writeStream(ctx, index, ctx.read(ptr, Number(total)));
    return nmemb;
  }),
  fputc: fn(2, (ctx) => {
    const c = ctx.arg32(0) & 0xff;
    writeStream(ctx, streamIndex(ctx, ctx.arg(1)), new Uint8Array([c]));
    return c;
  }),
  putchar: fn(1, (ctx) => {
    const c = ctx.arg32(0) & 0xff;
    writeStream(ctx, 1, new Uint8Array([c]));
    return c;
  }),
  fputs: fn(2, (ctx) => {
    const s = ctx.readCString(ctx.arg(0));
    writeStream(ctx, streamIndex(ctx, ctx.arg(1)), s);
    return 0;
  }),
  puts: fn(1, (ctx) => {
    const s = ctx.readCString(ctx.arg(0));
    writeStream(ctx, 1, s);
    writeStream(ctx, 1, new Uint8Array([0x0a]));
    return 0;
  }),
  printf: fn(1, (ctx) => printfTo(ctx, 1, ctx.arg(0), (i) => ctx.arg(1 + i))),
  fprintf: fn(2, (ctx) => printfTo(ctx, streamIndex(ctx, ctx.arg(0)), ctx.arg(1), (i) => ctx.arg(2 + i))),
  vprintf: fn(2, (ctx) => {
    const va = ctx.arg(1);
    return printfTo(ctx, 1, ctx.arg(0), (i) => ctx.read64(va + 8n * BigInt(i)));
  }),
  vfprintf: fn(3, (ctx) => {
    const va = ctx.arg(2);
    return printfTo(ctx, streamIndex(ctx, ctx.arg(0)), ctx.arg(1), (i) => ctx.read64(va + 8n * BigInt(i)));
  }),
};
