// kernel32.dll, shim v0 (docs/design/runtime.md, "Shim rules"). Each entry:
// `args` (the Win64 argument count, for traces) and `fn(ctx)` returning the
// value for RAX (undefined for none). Unsupported cases stop with ctx.fail.

import {
  STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE,
  STDIN_HANDLE, STDOUT_HANDLE, STDERR_HANDLE, INVALID_HANDLE_VALUE, ERROR_INVALID_HANDLE, THREAD_ID,
} from '../abi.mjs';
import { CP1252_TO_UNICODE, UNICODE_TO_CP1252 } from './cp1252.mjs';

const hex = (v) => '0x' + BigInt.asUintN(64, BigInt(v)).toString(16);

export const ERROR_SUCCESS = 0;
export const ERROR_BAD_LENGTH = 24;
export const ERROR_INVALID_PARAMETER = 87;
export const ERROR_INSUFFICIENT_BUFFER = 122;
export const ERROR_INVALID_FLAGS = 1004;

// Memory protection constants and MEMORY_BASIC_INFORMATION (winnt.h).
export const PAGE_NOACCESS = 0x01, PAGE_READONLY = 0x02, PAGE_READWRITE = 0x04, PAGE_WRITECOPY = 0x08;
export const PAGE_EXECUTE = 0x10, PAGE_EXECUTE_READ = 0x20, PAGE_EXECUTE_READWRITE = 0x40, PAGE_EXECUTE_WRITECOPY = 0x80;
const PROTECTIONS = new Set([0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80]);
export const MEM_COMMIT = 0x1000;
export const MEM_IMAGE = 0x100_0000;
export const MEMORY_BASIC_INFORMATION_SIZE = 48;
const PAGE = 0x1000n;
const IMAGE_SCN_MEM_EXECUTE = 0x2000_0000, IMAGE_SCN_MEM_READ = 0x4000_0000, IMAGE_SCN_MEM_WRITE = 0x8000_0000;

// TEB64 TLS slots (Wine include/winternl.h).
export const TEB_TLS_SLOTS = 0x1480n;
export const TEB_TLS_EXPANSION_SLOTS = 0x1780n;
export const TLS_MINIMUM_AVAILABLE = 64;
export const TLS_EXPANSION_SLOTS = 1024;

// RTL_CRITICAL_SECTION (x64): DebugInfo, LockCount, RecursionCount, OwningThread, LockSemaphore, SpinCount.
export const CRITICAL_SECTION_SIZE = 40;
const CS_DEBUG_INFO = 0n, CS_LOCK_COUNT = 8n, CS_RECURSION_COUNT = 12n, CS_OWNING_THREAD = 16n, CS_LOCK_SEMAPHORE = 24n, CS_SPIN_COUNT = 32n;

const CP_ACP = 0, CP_THREAD_ACP = 3, CP_1252 = 1252;
const MB_PRECOMPOSED = 0x1, MB_ERR_INVALID_CHARS = 0x8;
const WC_NO_BEST_FIT_CHARS = 0x400;

/** Monotonic clock in 100 ns ticks (runtime.md, "kernel32 timing"): process.hrtime in Node, performance.now in browsers. */
const QPC_FREQUENCY = 10_000_000n;
function monotonicTicks() {
  const hr = globalThis.process?.hrtime?.bigint;
  if (hr) return hr.call(globalThis.process) / 100n;
  return BigInt(Math.floor(performance.now() * 10_000));
}

/** Per-process kernel32 state. */
const k32 = (ctx) => ctx.dllState('kernel32.dll', () => ({
  clockStart: monotonicTicks(),
  criticalSections: new Map(),
  pages: null,
  unhandledExceptionFilter: 0n,
}));

/** The code pages the shim converts: the ANSI code page is 1252 (runtime.md, "Locale"). */
function codePage(ctx, cp) {
  if (cp === CP_ACP || cp === CP_THREAD_ACP || cp === CP_1252) return CP_1252;
  ctx.fail(`code page ${cp} is not supported (only the ANSI code page, 1252)`);
}

/** Page protections of the image: headers read-only, sections by their characteristics, as Windows maps an image. */
function imagePages(ctx) {
  const state = k32(ctx);
  if (state.pages) return state.pages;
  const img = ctx.image;
  if (!img) ctx.fail('no image is loaded');
  const count = Number((img.size + PAGE - 1n) / PAGE);
  const pages = new Array(count).fill(0);
  const set = (va, size, protect) => {
    const first = Number((va - img.base) / PAGE);
    const last = Number((va - img.base + size + PAGE - 1n) / PAGE);
    for (let i = first; i < last && i < count; i++) pages[i] = protect;
  };
  set(img.base, img.sizeOfHeaders, PAGE_READONLY);
  for (const s of img.sections) {
    const x = (s.characteristics & IMAGE_SCN_MEM_EXECUTE) !== 0;
    const w = (s.characteristics & IMAGE_SCN_MEM_WRITE) !== 0;
    const r = (s.characteristics & IMAGE_SCN_MEM_READ) !== 0;
    let protect;
    if (x) protect = w ? PAGE_EXECUTE_WRITECOPY : r ? PAGE_EXECUTE_READ : PAGE_EXECUTE;
    else protect = w ? PAGE_WRITECOPY : r ? PAGE_READONLY : PAGE_NOACCESS;
    set(s.va, s.mappedSize, protect);
  }
  state.pages = { base: img.base, protect: pages };
  return state.pages;
}

/** Index of the image page holding `addr`, or a stop for addresses outside the image. */
function pageIndex(ctx, pages, addr, api) {
  const i = addr >= pages.base ? Number((addr - pages.base) / PAGE) : -1;
  if (i < 0 || i >= pages.protect.length) ctx.fail(`${api} at ${hex(addr)}: only the image's pages are supported`);
  if (pages.protect[i] === 0) ctx.fail(`${api} at ${hex(addr)}: the page is not part of a section`);
  return i;
}

function criticalSection(ctx, cs, api) {
  ctx.check(cs, CRITICAL_SECTION_SIZE, 'critical section');
  const held = k32(ctx).criticalSections.get(cs);
  if (held === undefined) ctx.fail(`${api}: critical section ${hex(cs)} is not initialized`);
  return held;
}

export default {
  // HANDLE GetStdHandle(DWORD nStdHandle)
  GetStdHandle: {
    args: 1,
    fn(ctx) {
      switch (ctx.argI32(0)) {
        case STD_INPUT_HANDLE: return STDIN_HANDLE;
        case STD_OUTPUT_HANDLE: return STDOUT_HANDLE;
        case STD_ERROR_HANDLE: return STDERR_HANDLE;
        default:
          // Documented failure (as in Wine 11 kernelbase): INVALID_HANDLE_VALUE, ERROR_INVALID_HANDLE.
          ctx.setLastError(ERROR_INVALID_HANDLE);
          return INVALID_HANDLE_VALUE;
      }
    },
  },

  // BOOL WriteFile(HANDLE hFile, LPCVOID lpBuffer, DWORD nNumberOfBytesToWrite,
  //                LPDWORD lpNumberOfBytesWritten, LPOVERLAPPED lpOverlapped)
  WriteFile: {
    args: 5,
    fn(ctx) {
      const handle = ctx.arg(0);
      const buffer = ctx.arg(1);
      const count = ctx.arg32(2);
      const written = ctx.arg(3);
      const overlapped = ctx.arg(4);
      if (overlapped !== 0n) ctx.fail(`lpOverlapped ${hex(overlapped)}: overlapped I/O is not supported`);
      // Microsoft: lpNumberOfBytesWritten can be NULL only when lpOverlapped is not NULL.
      if (written === 0n) ctx.fail('lpNumberOfBytesWritten is NULL without lpOverlapped');
      let stream;
      if (handle === STDOUT_HANDLE) stream = 'stdout';
      else if (handle === STDERR_HANDLE) stream = 'stderr';
      else ctx.fail(`hFile ${hex(handle)}: not a writable handle (only the standard output and error handles are)`);
      const bytes = count ? ctx.read(buffer, count) : new Uint8Array(0);
      ctx.write32(written, count);
      ctx.output(stream, bytes);
      return 1n;
    },
  },

  // DWORD GetLastError(void)
  GetLastError: { args: 0, fn: (ctx) => ctx.lastError() },

  // void SetLastError(DWORD dwErrCode)
  SetLastError: { args: 1, fn(ctx) { ctx.setLastError(ctx.arg32(0)); } },

  // void InitializeCriticalSection(LPCRITICAL_SECTION): one thread, so locks only count.
  InitializeCriticalSection: {
    args: 1,
    fn(ctx) {
      const cs = ctx.check(ctx.arg(0), CRITICAL_SECTION_SIZE, 'critical section');
      ctx.write64(cs + CS_DEBUG_INFO, -1n);
      ctx.write32(cs + CS_LOCK_COUNT, -1);
      ctx.write32(cs + CS_RECURSION_COUNT, 0);
      ctx.write64(cs + CS_OWNING_THREAD, 0n);
      ctx.write64(cs + CS_LOCK_SEMAPHORE, 0n);
      ctx.write64(cs + CS_SPIN_COUNT, 0n);
      k32(ctx).criticalSections.set(cs, 0);
    },
  },

  // void EnterCriticalSection(LPCRITICAL_SECTION)
  EnterCriticalSection: {
    args: 1,
    fn(ctx) {
      const cs = ctx.arg(0);
      const n = criticalSection(ctx, cs, 'EnterCriticalSection') + 1;
      k32(ctx).criticalSections.set(cs, n);
      ctx.write32(cs + CS_LOCK_COUNT, -2);
      ctx.write32(cs + CS_RECURSION_COUNT, n);
      ctx.write64(cs + CS_OWNING_THREAD, BigInt(THREAD_ID));
    },
  },

  // void LeaveCriticalSection(LPCRITICAL_SECTION)
  LeaveCriticalSection: {
    args: 1,
    fn(ctx) {
      const cs = ctx.arg(0);
      const n = criticalSection(ctx, cs, 'LeaveCriticalSection') - 1;
      if (n < 0) ctx.fail(`LeaveCriticalSection: critical section ${hex(cs)} is not held`);
      k32(ctx).criticalSections.set(cs, n);
      ctx.write32(cs + CS_RECURSION_COUNT, n);
      if (n === 0) {
        ctx.write32(cs + CS_LOCK_COUNT, -1);
        ctx.write64(cs + CS_OWNING_THREAD, 0n);
      }
    },
  },

  // void DeleteCriticalSection(LPCRITICAL_SECTION)
  DeleteCriticalSection: {
    args: 1,
    fn(ctx) {
      const cs = ctx.arg(0);
      criticalSection(ctx, cs, 'DeleteCriticalSection');
      k32(ctx).criticalSections.delete(cs);
      ctx.write64(cs + CS_DEBUG_INFO, 0n);
      ctx.write32(cs + CS_LOCK_COUNT, -1);
      ctx.write32(cs + CS_RECURSION_COUNT, 0);
      ctx.write64(cs + CS_OWNING_THREAD, 0n);
      ctx.write64(cs + CS_LOCK_SEMAPHORE, 0n);
    },
  },

  // LPVOID TlsGetValue(DWORD dwTlsIndex): the TEB's slots; minimal validation, as documented.
  TlsGetValue: {
    args: 1,
    fn(ctx) {
      const index = ctx.arg32(0);
      const teb = ctx.teb();
      let value;
      if (index < TLS_MINIMUM_AVAILABLE) value = ctx.read64(teb + TEB_TLS_SLOTS + 8n * BigInt(index));
      else if (index < TLS_MINIMUM_AVAILABLE + TLS_EXPANSION_SLOTS) {
        const expansion = ctx.read64(teb + TEB_TLS_EXPANSION_SLOTS);
        value = expansion === 0n ? 0n : ctx.read64(expansion + 8n * BigInt(index - TLS_MINIMUM_AVAILABLE));
      } else {
        ctx.setLastError(ERROR_INVALID_PARAMETER);
        return 0n;
      }
      ctx.setLastError(ERROR_SUCCESS);
      return value;
    },
  },

  // SIZE_T VirtualQuery(LPCVOID lpAddress, PMEMORY_BASIC_INFORMATION lpBuffer, SIZE_T dwLength)
  VirtualQuery: {
    args: 3,
    fn(ctx) {
      const addr = ctx.arg(0), buf = ctx.arg(1), len = ctx.arg(2);
      if (len < BigInt(MEMORY_BASIC_INFORMATION_SIZE)) { ctx.setLastError(ERROR_BAD_LENGTH); return 0n; }
      const pages = imagePages(ctx);
      const i = pageIndex(ctx, pages, addr, 'VirtualQuery');
      const protect = pages.protect[i];
      let j = i;
      while (j < pages.protect.length && pages.protect[j] === protect) j++;
      const mbi = new DataView(new ArrayBuffer(MEMORY_BASIC_INFORMATION_SIZE));
      mbi.setBigUint64(0, pages.base + PAGE * BigInt(i), true); // BaseAddress
      mbi.setBigUint64(8, pages.base, true); // AllocationBase
      mbi.setUint32(16, PAGE_EXECUTE_WRITECOPY, true); // AllocationProtect (an image)
      mbi.setBigUint64(24, PAGE * BigInt(j - i), true); // RegionSize
      mbi.setUint32(32, MEM_COMMIT, true); // State
      mbi.setUint32(36, protect, true); // Protect
      mbi.setUint32(40, MEM_IMAGE, true); // Type
      ctx.write(buf, new Uint8Array(mbi.buffer));
      return BigInt(MEMORY_BASIC_INFORMATION_SIZE);
    },
  },

  // BOOL VirtualProtect(LPVOID lpAddress, SIZE_T dwSize, DWORD flNewProtect, PDWORD lpflOldProtect)
  VirtualProtect: {
    args: 4,
    fn(ctx) {
      const addr = ctx.arg(0), size = ctx.arg(1), protect = ctx.arg32(2), oldP = ctx.arg(3);
      if (!PROTECTIONS.has(protect)) ctx.fail(`flNewProtect ${hex(protect)} is not supported`);
      if (oldP === 0n) ctx.fail('lpflOldProtect is NULL');
      if (size === 0n) ctx.fail('dwSize is 0');
      const pages = imagePages(ctx);
      const first = pageIndex(ctx, pages, addr, 'VirtualProtect');
      const last = pageIndex(ctx, pages, addr + size - 1n, 'VirtualProtect');
      ctx.write32(oldP, pages.protect[first]);
      for (let i = first; i <= last; i++) pages.protect[i] = protect;
      return 1;
    },
  },

  // BOOL QueryPerformanceFrequency(LARGE_INTEGER *lpFrequency): 100 ns ticks, as on current Windows.
  QueryPerformanceFrequency: {
    args: 1,
    fn(ctx) {
      const out = ctx.arg(0);
      if (out === 0n) ctx.fail('lpFrequency is NULL');
      ctx.write64(out, QPC_FREQUENCY);
      return 1;
    },
  },

  // BOOL QueryPerformanceCounter(LARGE_INTEGER *lpPerformanceCount): 100 ns ticks since the process started.
  QueryPerformanceCounter: {
    args: 1,
    fn(ctx) {
      const out = ctx.arg(0);
      if (out === 0n) ctx.fail('lpPerformanceCount is NULL');
      const start = k32(ctx).clockStart; // read first: the state may be created by this call
      ctx.write64(out, monotonicTicks() - start);
      return 1;
    },
  },

  // void Sleep(DWORD dwMilliseconds): one thread and no clocks, so nothing to wait for.
  Sleep: {
    args: 1,
    fn(ctx) {
      if (ctx.arg32(0) === 0xffff_ffff) ctx.fail('Sleep(INFINITE) with one thread never returns');
    },
  },

  // LPTOP_LEVEL_EXCEPTION_FILTER SetUnhandledExceptionFilter(LPTOP_LEVEL_EXCEPTION_FILTER)
  SetUnhandledExceptionFilter: {
    args: 1,
    fn(ctx) {
      const state = k32(ctx);
      const old = state.unhandledExceptionFilter;
      state.unhandledExceptionFilter = ctx.arg(0);
      return old;
    },
  },

  // BOOL IsDBCSLeadByteEx(UINT CodePage, BYTE TestChar): code page 1252 has no lead bytes.
  IsDBCSLeadByteEx: {
    args: 2,
    fn(ctx) {
      codePage(ctx, ctx.arg32(0));
      return 0;
    },
  },

  // int MultiByteToWideChar(UINT CodePage, DWORD dwFlags, LPCCH lpMultiByteStr, int cbMultiByte,
  //                         LPWSTR lpWideCharStr, int cchWideChar)
  MultiByteToWideChar: {
    args: 6,
    fn(ctx) {
      const cp = ctx.arg32(0), flags = ctx.arg32(1), src = ctx.arg(2), srcLen = ctx.argI32(3);
      const dst = ctx.arg(4), dstLen = ctx.argI32(5);
      if (src === 0n || srcLen === 0 || (dst === 0n && dstLen) || dstLen < 0) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
      codePage(ctx, cp);
      if (flags & ~(MB_PRECOMPOSED | MB_ERR_INVALID_CHARS)) ctx.fail(`dwFlags ${hex(flags)} is not supported`);
      const bytes = srcLen < 0 ? [...ctx.readCString(src), 0] : ctx.read(src, srcLen);
      if (dstLen === 0) return bytes.length;
      if (dstLen < bytes.length) { ctx.setLastError(ERROR_INSUFFICIENT_BUFFER); return 0; }
      const out = new Uint8Array(2 * bytes.length);
      const dv = new DataView(out.buffer);
      Array.from(bytes).forEach((b, i) => dv.setUint16(2 * i, CP1252_TO_UNICODE[b], true));
      ctx.write(dst, out);
      return bytes.length;
    },
  },

  // int WideCharToMultiByte(UINT CodePage, DWORD dwFlags, LPCWCH lpWideCharStr, int cchWideChar,
  //                         LPSTR lpMultiByteStr, int cbMultiByte, LPCCH lpDefaultChar, LPBOOL lpUsedDefaultChar)
  WideCharToMultiByte: {
    args: 8,
    fn(ctx) {
      const cp = ctx.arg32(0), flags = ctx.arg32(1), src = ctx.arg(2), srcLen = ctx.argI32(3);
      const dst = ctx.arg(4), dstLen = ctx.argI32(5), defChar = ctx.arg(6), used = ctx.arg(7);
      if (src === 0n || srcLen === 0 || (dst === 0n && dstLen) || dstLen < 0) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
      codePage(ctx, cp);
      if (flags & ~WC_NO_BEST_FIT_CHARS) ctx.fail(`dwFlags ${hex(flags)} is not supported`);
      let units;
      if (srcLen < 0) units = [...ctx.readWString(src), 0];
      else {
        const raw = ctx.read(src, 2 * srcLen);
        units = Array.from({ length: srcLen }, (_, i) => raw[2 * i] | (raw[2 * i + 1] << 8));
      }
      const def = defChar === 0n ? 0x3f : ctx.read(defChar, 1)[0];
      let usedDefault = false;
      const out = Uint8Array.from(units, (u) => {
        const b = UNICODE_TO_CP1252.get(u);
        if (b !== undefined) return b;
        if (!(flags & WC_NO_BEST_FIT_CHARS)) {
          ctx.fail(`U+${u.toString(16).padStart(4, '0')} needs a best-fit mapping, which is not supported`);
        }
        usedDefault = true;
        return def;
      });
      if (used !== 0n) ctx.write32(used, usedDefault ? 1 : 0);
      if (dstLen === 0) return out.length;
      if (dstLen < out.length) { ctx.setLastError(ERROR_INSUFFICIENT_BUFFER); return 0; }
      ctx.write(dst, out);
      return out.length;
    },
  },

  // DECLSPEC_NORETURN void ExitProcess(UINT uExitCode)
  ExitProcess: {
    args: 1,
    fn(ctx) {
      ctx.exit(ctx.arg32(0));
    },
  },
};
