// kernel32.dll: the process, modules and GetProcAddress, memory, time,
// synchronization with one thread, the console and the machine profile
// (runtime.md, "Files, handles and the 7-Zip shim (M5)"). Behaviour follows
// Microsoft's documentation; details are checked against Wine 11
// (kernelbase/{loader,memory,process,sync,console,file}.c, kernel32/file.c,
// ntdll/time.c).

import { INVALID_HANDLE_VALUE, PROCESS_ID, THREAD_ID, OS_VERSION } from '../abi.mjs';
import {
  Win32Error, ERROR_INVALID_PARAMETER, ERROR_INVALID_HANDLE, ERROR_FILE_NOT_FOUND, ERROR_MOD_NOT_FOUND,
  ERROR_INSUFFICIENT_BUFFER, ERROR_TOO_MANY_POSTS, ERROR_INVALID_ADDRESS, ERROR_NOT_ENOUGH_MEMORY,
} from './errors.mjs';
import { addHandle, getHandle, removeHandle, isStdHandle, CURRENT_PROCESS, CURRENT_THREAD } from './handles.mjs';
import { hex, fn, wstr, utf16, copyOut, writeWString, win32 } from './util.mjs';
import { TEB_TLS_SLOTS, TLS_MINIMUM_AVAILABLE } from './kernel32.mjs';

// --- the machine profile -----------------------------------------------------

export const WINDOWS_DIR = 'C:\\Windows';
export const SYSTEM_DIR = 'C:\\Windows\\System32';
/** Physical memory of the profile: 4 GiB. */
export const TOTAL_PHYS = 4n << 30n;
/** The user address space Windows x64 reports (128 TiB minus 64 KiB granules at both ends). */
export const TOTAL_VIRTUAL = 0x7fff_fffe_0000n;
export const PAGE_SIZE = 0x1000;
export const ALLOCATION_GRANULARITY = 0x1_0000;
export const MIN_APP_ADDRESS = 0x1_0000n;
const PROCESSOR_ARCHITECTURE_AMD64 = 9, PROCESSOR_AMD_X8664 = 8664;
/** The CPUID profile's family 6, model 0, stepping 0. */
const PROCESSOR_LEVEL = 6, PROCESSOR_REVISION = 0;
/** IsProcessorFeaturePresent: the features the CPUID profile reports. */
export const PROCESSOR_FEATURES = new Set([
  2, // PF_COMPARE_EXCHANGE_DOUBLE (CMPXCHG8B)
  3, // PF_MMX_INSTRUCTIONS_AVAILABLE
  6, // PF_XMMI_INSTRUCTIONS_AVAILABLE (SSE)
  10, // PF_XMMI64_INSTRUCTIONS_AVAILABLE (SSE2)
  12, // PF_NX_ENABLED
  14, // PF_COMPARE_EXCHANGE128 (CMPXCHG16B)
]);
/** The OEM code page of an en-US Windows. */
export const OEM_CODE_PAGE = 437;
/** The supportedOS GUID of Windows 10 and 11 in an application manifest. */
const WIN10_MANIFEST_GUID = new TextEncoder().encode('8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a');

// --- modules -------------------------------------------------------------------

/**
 * System DLLs the reference Windows has, with fixed pseudo-bases for their
 * module handles. `loaded`: in a 7zz process at startup (its imports and
 * their dependencies).
 */
export const SYSTEM_DLLS = new Map([
  ['kernel32.dll', { base: 0x7ffa_0000_0000n, loaded: true }],
  ['kernelbase.dll', { base: 0x7ffa_0100_0000n, loaded: true }],
  ['ntdll.dll', { base: 0x7ffa_0200_0000n, loaded: true }],
  ['advapi32.dll', { base: 0x7ffa_0300_0000n, loaded: true }],
  ['user32.dll', { base: 0x7ffa_0400_0000n, loaded: true }],
  ['oleaut32.dll', { base: 0x7ffa_0500_0000n, loaded: true }],
  ['msvcrt.dll', { base: 0x7ffa_0600_0000n, loaded: true }],
  ['psapi.dll', { base: 0x7ffa_0700_0000n, loaded: false }],
  ['version.dll', { base: 0x7ffa_0800_0000n, loaded: false }],
  ['shell32.dll', { base: 0x7ffa_0900_0000n, loaded: false }],
]);

const sys = (ctx) => ctx.dllState('tempwind:system', () => ({
  loaded: new Set([...SYSTEM_DLLS].filter(([, d]) => d.loaded).map(([n]) => n)),
  tickStart: Date.now(),
  commandLine: 0n,
  tlsUsed: new Set(),
  vm: null,
  fileApisOem: false,
  ctrlHandlers: [],
  dllDirectoryFlags: 0,
  version: null,
}));

/** The DLL a module name means: a bare name gets ".dll"; a path must be in the system directory. */
function dllName(name) {
  let base = name;
  const i = Math.max(name.lastIndexOf('\\'), name.lastIndexOf('/'));
  if (i >= 0) {
    const dir = name.slice(0, i).toLowerCase();
    if (dir !== SYSTEM_DIR.toLowerCase()) return null;
    base = name.slice(i + 1);
  }
  base = base.toLowerCase();
  if (base.endsWith('.')) base = base.slice(0, -1);
  else if (!base.includes('.')) base += '.dll';
  return base;
}

function moduleByHandle(ctx, h, api) {
  if (h === 0n || (ctx.image && h === ctx.image.base)) return null; // the program
  for (const [name, d] of SYSTEM_DLLS) if (d.base === h && sys(ctx).loaded.has(name)) return name;
  ctx.fail(`${api}: unknown module handle ${hex(h)}`);
}

function getModuleHandle(ctx, name) {
  if (name === null) return ctx.image.base;
  const dll = dllName(name);
  const prog = ctx.programPath;
  if (name.toLowerCase() === prog.toLowerCase() || dll === prog.slice(prog.lastIndexOf('\\') + 1).toLowerCase()) return ctx.image.base;
  if (dll && SYSTEM_DLLS.has(dll) && sys(ctx).loaded.has(dll)) return SYSTEM_DLLS.get(dll).base;
  ctx.setLastError(ERROR_MOD_NOT_FOUND);
  return 0n;
}

function loadLibrary(ctx, name) {
  const dll = dllName(name);
  if (dll && SYSTEM_DLLS.has(dll)) {
    sys(ctx).loaded.add(dll);
    return SYSTEM_DLLS.get(dll).base;
  }
  ctx.setLastError(ERROR_MOD_NOT_FOUND);
  return 0n;
}

/** GetVersion's compatibility rule: Windows 10 values only for a manifest that declares Windows 10. */
function osVersion(ctx) {
  const s = sys(ctx);
  if (s.version) return s.version;
  let win10 = false;
  const img = ctx.image;
  if (img) {
    for (const sec of img.sections) {
      if (sec.name !== '.rsrc') continue;
      const bytes = ctx.mem.read(sec.va, Number(sec.mappedSize));
      win10 ||= indexOf(bytes, WIN10_MANIFEST_GUID) >= 0;
    }
  }
  s.version = win10 ? OS_VERSION : { major: 6, minor: 2, build: 9200, platformId: 2 };
  return s.version;
}

function indexOf(hay, needle) {
  outer: for (let i = hay.indexOf(needle[0]); i >= 0 && i <= hay.length - needle.length; i = hay.indexOf(needle[0], i + 1)) {
    for (let j = 1; j < needle.length; j++) if (hay[i + j] !== needle[j]) continue outer;
    return i;
  }
  return -1;
}

// --- the command line ----------------------------------------------------------

/** Quotes one argument so that CommandLineToArgvW gives it back. */
export function quoteArg(a) {
  if (a.length && !/[ \t"]/.test(a)) return a;
  let out = '"';
  let slashes = 0;
  for (const c of a) {
    if (c === '\\') { slashes++; continue; }
    if (c === '"') { out += '\\'.repeat(2 * slashes + 1) + '"'; slashes = 0; continue; }
    out += '\\'.repeat(slashes) + c;
    slashes = 0;
  }
  return out + '\\'.repeat(2 * slashes) + '"';
}

/** The command line Windows would hold: the program path, then the arguments. */
export const commandLine = (programPath, args) => [programPath, ...args].map(quoteArg).join(' ');

// --- time ----------------------------------------------------------------------

const TICKS_PER_DAY = 864_000_000_000n;
const daysInMonth = (y, m) => [31, (y % 4 === 0 && (y % 100 !== 0 || y % 400 === 0)) ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m - 1];

/** RtlTimeToTimeFields: a FILETIME (< 2^63) as calendar fields. */
export function timeFields(t) {
  const days = t / TICKS_PER_DAY;
  let rem = t % TICKS_PER_DAY;
  const ms = Number(rem / 10_000n) % 1000;
  const secs = Number(rem / 10_000_000n);
  const hour = Math.floor(secs / 3600), minute = Math.floor(secs / 60) % 60, second = secs % 60;
  // 1601-01-01 was a Monday.
  const weekday = Number((days + 1n) % 7n);
  // Civil date from days since 1601-01-01 (proleptic Gregorian, 400-year cycles of 146097 days).
  let d = days;
  let year = 1601n + 400n * (d / 146097n);
  d %= 146097n;
  let c = d / 36524n; if (c === 4n) c = 3n; year += 100n * c; d -= 36524n * c;
  let q = d / 1461n; year += 4n * q; d -= 1461n * q;
  let y = d / 365n; if (y === 4n) y = 3n; year += y; d -= 365n * y;
  const yr = Number(year);
  let day = Number(d) + 1, month = 1;
  while (day > daysInMonth(yr, month)) { day -= daysInMonth(yr, month); month++; }
  rem = 0n;
  return { year: yr, month, day, hour, minute, second, ms, weekday };
}

/** RtlTimeFieldsToTime: null for invalid fields. */
export function fieldsToTime({ year, month, day, hour, minute, second, ms = 0 }) {
  if (month < 1 || month > 12 || day < 1 || year < 1601 || day > daysInMonth(year, month)) return null;
  if (hour > 23 || minute > 59 || second > 59 || ms > 999 || hour < 0 || minute < 0 || second < 0 || ms < 0) return null;
  let days = 0n;
  const y = BigInt(year - 1601);
  days += y * 365n + y / 4n - y / 100n + y / 400n;
  for (let m = 1; m < month; m++) days += BigInt(daysInMonth(year, m));
  days += BigInt(day - 1);
  return days * TICKS_PER_DAY + BigInt(((hour * 60 + minute) * 60 + second) * 1000 + ms) * 10_000n;
}

/** Host CPU time of the process in 100 ns ticks (user, kernel). */
function cpuTimes() {
  const u = globalThis.process?.cpuUsage?.();
  if (!u) return { user: 0n, kernel: 0n };
  return { user: BigInt(u.user) * 10n, kernel: BigInt(u.system) * 10n };
}

// --- virtual memory --------------------------------------------------------------

const MEM_COMMIT = 0x1000, MEM_RESERVE = 0x2000, MEM_DECOMMIT = 0x4000, MEM_RELEASE = 0x8000;
const MEM_TOP_DOWN = 0x10_0000;
const PAGE_READWRITE = 0x04;
const GRANULE = BigInt(ALLOCATION_GRANULARITY);

/** VirtualAlloc regions in the guest heap: 64 KiB granularity, released regions reused first-fit. */
function vm(ctx) {
  const s = sys(ctx);
  s.vm ??= { regions: new Map(), free: [] };
  return s.vm;
}

function vmAlloc(ctx, size) {
  const v = vm(ctx);
  const need = (size + GRANULE - 1n) / GRANULE * GRANULE;
  const i = v.free.findIndex((r) => r.size >= need);
  let base;
  if (i >= 0) {
    const r = v.free[i];
    base = r.base;
    if (r.size === need) v.free.splice(i, 1); else { r.base += need; r.size -= need; }
    ctx.mem.fill(base, Number(need));
  } else {
    base = ctx.alloc(need, GRANULE);
  }
  v.regions.set(base, need);
  return base;
}

function vmFree(ctx, base) {
  const v = vm(ctx);
  const size = v.regions.get(base);
  v.regions.delete(base);
  v.free.push({ base, size });
  v.free.sort((a, b) => (a.base < b.base ? -1 : 1));
  // Merge neighbours.
  for (let i = 0; i + 1 < v.free.length;) {
    const a = v.free[i], b = v.free[i + 1];
    if (a.base + a.size === b.base) { a.size += b.size; v.free.splice(i + 1, 1); } else i++;
  }
}

// --- synchronization -------------------------------------------------------------

const WAIT_OBJECT_0 = 0, WAIT_TIMEOUT = 0x102;
const INFINITE = 0xffff_ffff;

/** Whether a wait on `obj` is satisfied now; `consume` applies its side effect. */
function signaled(ctx, obj, api) {
  if (obj.kind === 'event') return obj.signaled;
  if (obj.kind === 'semaphore') return obj.count > 0;
  ctx.fail(`${api}: waiting on a ${obj.kind} handle is not supported`);
}

function consume(obj) {
  if (obj.kind === 'event' && !obj.manual) obj.signaled = false;
  if (obj.kind === 'semaphore') obj.count--;
}

function checkUnnamed(ctx, sa, nameP, api) {
  if (sa !== 0n) ctx.fail(`${api}: security attributes are not supported`);
  if (nameP !== 0n) ctx.fail(`${api}: named objects (${wstr(ctx, nameP)}) are not supported`);
}

// --- FormatMessage -----------------------------------------------------------------

/** System message texts (English, as Windows prints them) for the errors the shim sets. */
export const SYSTEM_MESSAGES = new Map([
  [0, 'The operation completed successfully.'],
  [1, 'Incorrect function.'],
  [2, 'The system cannot find the file specified.'],
  [3, 'The system cannot find the path specified.'],
  [5, 'Access is denied.'],
  [6, 'The handle is invalid.'],
  [8, 'Not enough memory resources are available to process this command.'],
  [18, 'There are no more files.'],
  [32, 'The process cannot access the file because it is being used by another process.'],
  [38, 'Reached the end of the file.'],
  [80, 'The file exists.'],
  [87, 'The parameter is incorrect.'],
  [123, 'The filename, directory name, or volume label syntax is incorrect.'],
  [145, 'The directory is not empty.'],
  [183, 'Cannot create a file when that file already exists.'],
  [267, 'The directory name is invalid.'],
]);

const FORMAT_MESSAGE_ALLOCATE_BUFFER = 0x100, FORMAT_MESSAGE_IGNORE_INSERTS = 0x200;
const FORMAT_MESSAGE_FROM_SYSTEM = 0x1000, FORMAT_MESSAGE_MAX_WIDTH_MASK = 0xff;

export default {
  // --- handles and the process ---
  // BOOL CloseHandle(HANDLE hObject)
  CloseHandle: fn(1, (ctx) => {
    const h = BigInt.asUintN(64, ctx.arg(0));
    if (isStdHandle(h)) ctx.fail(`CloseHandle on standard handle ${hex(h)} is not supported`);
    if (h === CURRENT_PROCESS || h === CURRENT_THREAD) return 1;
    const obj = getHandle(ctx, h, 'hObject');
    if (obj.kind === 'find') ctx.fail(`CloseHandle on find handle ${hex(h)} (FindClose closes it)`);
    removeHandle(ctx, h, 'hObject');
    if (obj.kind === 'file') ctx.vfs.close(obj.file);
    return 1;
  }),
  GetCurrentProcess: fn(0, () => CURRENT_PROCESS),
  GetCurrentProcessId: fn(0, () => PROCESS_ID),
  GetCurrentThreadId: fn(0, () => THREAD_ID),
  GetCurrentThread: fn(0, () => CURRENT_THREAD),
  // DWORD GetThreadId(HANDLE Thread)
  GetThreadId: fn(1, (ctx) => {
    if (BigInt.asUintN(64, ctx.arg(0)) !== CURRENT_THREAD) ctx.fail(`GetThreadId(${hex(ctx.arg(0))}): only the current thread exists`);
    return THREAD_ID;
  }),

  // LPWSTR GetCommandLineW(void)
  GetCommandLineW: fn(0, (ctx) => {
    const s = sys(ctx);
    if (!s.commandLine) {
      const line = commandLine(ctx.programPath, ctx.argv.slice(1));
      s.commandLine = ctx.alloc(BigInt(2 * line.length + 2));
      writeWString(ctx, s.commandLine, line);
    }
    return s.commandLine;
  }),

  // BOOL GetProcessTimes(HANDLE, LPFILETIME lpCreationTime, LPFILETIME lpExitTime, LPFILETIME lpKernelTime, LPFILETIME lpUserTime)
  GetProcessTimes: fn(5, (ctx) => {
    if (BigInt.asUintN(64, ctx.arg(0)) !== CURRENT_PROCESS) ctx.fail('GetProcessTimes: only the current process is supported');
    const { user, kernel } = cpuTimes();
    ctx.write64(ctx.arg(1), ctx.startTime);
    ctx.write64(ctx.arg(2), 0n);
    ctx.write64(ctx.arg(3), kernel);
    ctx.write64(ctx.arg(4), user);
    return 1;
  }),

  // --- modules ---
  // DWORD GetModuleFileNameW(HMODULE hModule, LPWSTR lpFilename, DWORD nSize)
  GetModuleFileNameW: fn(3, (ctx) => {
    const h = ctx.arg(0), buf = ctx.arg(1), size = ctx.arg32(2);
    const dll = moduleByHandle(ctx, h, 'GetModuleFileNameW');
    const name = dll === null ? ctx.programPath : `${SYSTEM_DIR}\\${dll}`;
    if (size === 0) ctx.fail('nSize is 0');
    if (size > name.length) {
      writeWString(ctx, buf, name);
      ctx.setLastError(0);
      return name.length;
    }
    // Truncated and terminated; the result is nSize (Windows Vista and later).
    writeWString(ctx, buf, name.slice(0, size - 1));
    ctx.setLastError(ERROR_INSUFFICIENT_BUFFER);
    return size;
  }),

  // HMODULE GetModuleHandleW(LPCWSTR lpModuleName)
  GetModuleHandleW: fn(1, (ctx) => {
    const p = ctx.arg(0);
    return getModuleHandle(ctx, p === 0n ? null : wstr(ctx, p));
  }),
  // HMODULE GetModuleHandleA(LPCSTR lpModuleName)
  GetModuleHandleA: fn(1, (ctx) => {
    const p = ctx.arg(0);
    return getModuleHandle(ctx, p === 0n ? null : new TextDecoder('latin1').decode(ctx.readCString(p)));
  }),
  // HMODULE LoadLibraryW(LPCWSTR lpLibFileName)
  LoadLibraryW: fn(1, (ctx) => loadLibrary(ctx, wstr(ctx, ctx.arg(0), 'lpLibFileName'))),
  // HMODULE LoadLibraryExW(LPCWSTR lpLibFileName, HANDLE hFile, DWORD dwFlags)
  LoadLibraryExW: fn(3, (ctx) => {
    if (ctx.arg(1) !== 0n) ctx.fail('hFile must be NULL');
    const flags = ctx.arg32(2);
    const LOAD_WITH_ALTERED_SEARCH_PATH = 0x8, LOAD_LIBRARY_SEARCH_SYSTEM32 = 0x800;
    if (flags & ~(LOAD_WITH_ALTERED_SEARCH_PATH | LOAD_LIBRARY_SEARCH_SYSTEM32)) ctx.fail(`dwFlags ${hex(flags)} is not supported`);
    return loadLibrary(ctx, wstr(ctx, ctx.arg(0), 'lpLibFileName'));
  }),
  // BOOL FreeLibrary(HMODULE hLibModule): system DLLs stay loaded.
  FreeLibrary: fn(1, (ctx) => {
    const dll = moduleByHandle(ctx, ctx.arg(0), 'FreeLibrary');
    if (dll === null) ctx.fail('FreeLibrary on the program');
    return 1;
  }),
  // FARPROC GetProcAddress(HMODULE hModule, LPCSTR lpProcName)
  GetProcAddress: fn(2, (ctx) => {
    const h = ctx.arg(0), p = ctx.arg(1);
    if (p < 0x1_0000n) ctx.fail(`import by ordinal ${p} is not supported`);
    const name = new TextDecoder('latin1').decode(ctx.readCString(p));
    const dll = moduleByHandle(ctx, h, 'GetProcAddress');
    if (dll === null) ctx.fail(`GetProcAddress(${name}) on the program's own exports is not supported`);
    const addr = ctx.procAddress(dll, name);
    if (addr === undefined) ctx.fail(`${dll}!${name} is not implemented (GetProcAddress)`);
    return addr;
  }),
  // BOOL SetDefaultDllDirectories(DWORD DirectoryFlags)
  SetDefaultDllDirectories: fn(1, (ctx) => {
    const flags = ctx.arg32(0);
    const valid = 0x100 | 0x200 | 0x400 | 0x800 | 0x1000; // LOAD_LIBRARY_SEARCH_*
    if (!flags || (flags & ~valid)) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    sys(ctx).dllDirectoryFlags = flags;
    return 1;
  }),

  // --- the machine profile ---
  // void GetSystemInfo(LPSYSTEM_INFO lpSystemInfo)
  GetSystemInfo: fn(1, (ctx) => {
    const b = new Uint8Array(48);
    const dv = new DataView(b.buffer);
    dv.setUint16(0, PROCESSOR_ARCHITECTURE_AMD64, true);
    dv.setUint32(4, PAGE_SIZE, true);
    dv.setBigUint64(8, MIN_APP_ADDRESS, true);
    dv.setBigUint64(16, ctx.host.maxAppAddress ?? 0x7fff_fffe_ffffn, true);
    dv.setBigUint64(24, 1n, true); // active processor mask
    dv.setUint32(32, 1, true); // processors
    dv.setUint32(36, PROCESSOR_AMD_X8664, true);
    dv.setUint32(40, ALLOCATION_GRANULARITY, true);
    dv.setUint16(44, PROCESSOR_LEVEL, true);
    dv.setUint16(46, PROCESSOR_REVISION, true);
    ctx.write(ctx.arg(0), b);
  }),
  // WORD GetActiveProcessorGroupCount(void)
  GetActiveProcessorGroupCount: fn(0, () => 1),
  // DWORD GetActiveProcessorCount(WORD GroupNumber): group 0 or ALL_PROCESSOR_GROUPS (0xFFFF).
  GetActiveProcessorCount: fn(1, (ctx) => {
    const g = ctx.arg32(0) & 0xffff;
    if (g === 0 || g === 0xffff) return 1;
    ctx.setLastError(ERROR_INVALID_PARAMETER);
    return 0;
  }),
  // BOOL GetProcessAffinityMask(HANDLE, PDWORD_PTR lpProcessAffinityMask, PDWORD_PTR lpSystemAffinityMask)
  GetProcessAffinityMask: fn(3, (ctx) => {
    if (BigInt.asUintN(64, ctx.arg(0)) !== CURRENT_PROCESS) ctx.fail('only the current process is supported');
    ctx.write64(ctx.arg(1), 1n);
    ctx.write64(ctx.arg(2), 1n);
    return 1;
  }),
  // BOOL SetProcessAffinityMask(HANDLE, DWORD_PTR)
  SetProcessAffinityMask: fn(2, (ctx) => {
    if (BigInt.asUintN(64, ctx.arg(0)) !== CURRENT_PROCESS) ctx.fail('only the current process is supported');
    if (ctx.arg(1) !== 1n) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    return 1;
  }),
  // DWORD_PTR SetThreadAffinityMask(HANDLE, DWORD_PTR): returns the previous mask.
  SetThreadAffinityMask: fn(2, (ctx) => {
    if (BigInt.asUintN(64, ctx.arg(0)) !== CURRENT_THREAD) ctx.fail('only the current thread is supported');
    if (ctx.arg(1) !== 1n) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    return 1;
  }),
  // BOOL GlobalMemoryStatusEx(LPMEMORYSTATUSEX)
  GlobalMemoryStatusEx: fn(1, (ctx) => {
    const p = ctx.arg(0);
    if (ctx.read32(p) !== 64) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    const free = ctx.host.heapFree ? ctx.host.heapFree() : TOTAL_PHYS / 2n;
    const avail = free < TOTAL_PHYS ? free : TOTAL_PHYS;
    const b = new Uint8Array(64);
    const dv = new DataView(b.buffer);
    dv.setUint32(0, 64, true);
    dv.setUint32(4, Number((100n * (TOTAL_PHYS - avail)) / TOTAL_PHYS), true);
    dv.setBigUint64(8, TOTAL_PHYS, true);
    dv.setBigUint64(16, avail, true);
    dv.setBigUint64(24, TOTAL_PHYS, true);
    dv.setBigUint64(32, avail, true);
    dv.setBigUint64(40, TOTAL_VIRTUAL, true);
    dv.setBigUint64(48, avail, true);
    ctx.write(p, b);
    return 1;
  }),
  // BOOL IsProcessorFeaturePresent(DWORD ProcessorFeature)
  IsProcessorFeaturePresent: fn(1, (ctx) => (PROCESSOR_FEATURES.has(ctx.arg32(0)) ? 1 : 0)),
  // SIZE_T GetLargePageMinimum(void)
  GetLargePageMinimum: fn(0, () => 0),
  // DWORD GetVersion(void)
  GetVersion: fn(0, (ctx) => {
    const v = osVersion(ctx);
    return ((v.build & 0x7fff) << 16 | v.minor << 8 | v.major) >>> 0;
  }),
  // UINT GetOEMCP(void)
  GetOEMCP: fn(0, () => OEM_CODE_PAGE),
  // void SetFileApisToOEM(void): only the ANSI file APIs change, and the shim has none.
  SetFileApisToOEM: fn(0, (ctx) => { sys(ctx).fileApisOem = true; }),
  // UINT GetWindowsDirectoryW(LPWSTR lpBuffer, UINT uSize)
  GetWindowsDirectoryW: fn(2, (ctx) => copyOut(ctx, ctx.arg(0), ctx.arg32(1), WINDOWS_DIR)),
  // UINT GetSystemDirectoryW(LPWSTR lpBuffer, UINT uSize)
  GetSystemDirectoryW: fn(2, (ctx) => copyOut(ctx, ctx.arg(0), ctx.arg32(1), SYSTEM_DIR)),
  // DWORD GetTempPathW(DWORD nBufferLength, LPWSTR lpBuffer): TMP, TEMP, USERPROFILE, then the Windows directory.
  GetTempPathW: fn(2, (ctx) => copyOut(ctx, ctx.arg(1), ctx.arg32(0), WINDOWS_DIR + '\\')),

  // --- time ---
  // void GetSystemTimeAsFileTime(LPFILETIME)
  GetSystemTimeAsFileTime: fn(1, (ctx) => { ctx.write64(ctx.arg(0), ctx.vfs.now()); }),
  // DWORD GetTickCount(void): milliseconds since the process started.
  GetTickCount: fn(0, (ctx) => (Date.now() - sys(ctx).tickStart) >>> 0),
  // BOOL FileTimeToLocalFileTime(const FILETIME *, LPFILETIME): UTC, so the identity.
  FileTimeToLocalFileTime: fn(2, (ctx) => { ctx.write64(ctx.arg(1), ctx.read64(ctx.arg(0))); return 1; }),
  // BOOL LocalFileTimeToFileTime(const FILETIME *, LPFILETIME)
  LocalFileTimeToFileTime: fn(2, (ctx) => { ctx.write64(ctx.arg(1), ctx.read64(ctx.arg(0))); return 1; }),
  // BOOL FileTimeToSystemTime(const FILETIME *, LPSYSTEMTIME)
  FileTimeToSystemTime: fn(2, (ctx) => {
    const t = ctx.read64(ctx.arg(0));
    if (t >= 1n << 63n) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    const f = timeFields(t);
    const b = new Uint8Array(16);
    const dv = new DataView(b.buffer);
    [f.year, f.month, f.weekday, f.day, f.hour, f.minute, f.second, f.ms].forEach((v, i) => dv.setUint16(2 * i, v, true));
    ctx.write(ctx.arg(1), b);
    return 1;
  }),
  // BOOL DosDateTimeToFileTime(WORD wFatDate, WORD wFatTime, LPFILETIME)
  DosDateTimeToFileTime: fn(3, (ctx) => {
    const d = ctx.arg32(0) & 0xffff, t = ctx.arg32(1) & 0xffff;
    const ft = fieldsToTime({
      year: (d >> 9) + 1980, month: (d >> 5) & 0xf, day: d & 0x1f,
      hour: t >> 11, minute: (t >> 5) & 0x3f, second: (t & 0x1f) * 2,
    });
    if (ft === null) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    ctx.write64(ctx.arg(2), ft);
    return 1;
  }),
  // BOOL FileTimeToDosDateTime(const FILETIME *, LPWORD lpFatDate, LPWORD lpFatTime): 1980 to 2107.
  FileTimeToDosDateTime: fn(3, (ctx) => {
    const dP = ctx.arg(1), tP = ctx.arg(2);
    if (dP === 0n || tP === 0n) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    const t = ctx.read64(ctx.arg(0));
    if (t >= 1n << 63n) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    const f = timeFields(t);
    if (f.year < 1980 || f.year > 2107) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    ctx.mem.setU16(ctx.check(dP, 2), ((f.year - 1980) << 9) | (f.month << 5) | f.day);
    ctx.mem.setU16(ctx.check(tP, 2), (f.hour << 11) | (f.minute << 5) | (f.second >> 1));
    return 1;
  }),
  // LONG CompareFileTime(const FILETIME *, const FILETIME *)
  CompareFileTime: fn(2, (ctx) => {
    const a = ctx.read64(ctx.arg(0)), b = ctx.read64(ctx.arg(1));
    return a < b ? 0xffff_ffff : a > b ? 1 : 0;
  }),

  // --- synchronization ---
  // HANDLE CreateEventW(LPSECURITY_ATTRIBUTES, BOOL bManualReset, BOOL bInitialState, LPCWSTR lpName)
  CreateEventW: fn(4, (ctx) => {
    checkUnnamed(ctx, ctx.arg(0), ctx.arg(3), 'CreateEventW');
    ctx.setLastError(0);
    return addHandle(ctx, { kind: 'event', manual: ctx.arg32(1) !== 0, signaled: ctx.arg32(2) !== 0 });
  }),
  // BOOL SetEvent(HANDLE)
  SetEvent: fn(1, (ctx) => { getHandle(ctx, ctx.arg(0), 'hEvent', 'event').signaled = true; return 1; }),
  // BOOL ResetEvent(HANDLE)
  ResetEvent: fn(1, (ctx) => { getHandle(ctx, ctx.arg(0), 'hEvent', 'event').signaled = false; return 1; }),
  // HANDLE OpenEventW(DWORD, BOOL, LPCWSTR lpName): there are no named objects.
  OpenEventW: fn(3, (ctx) => { wstr(ctx, ctx.arg(2), 'lpName'); ctx.setLastError(ERROR_FILE_NOT_FOUND); return 0; }),
  // HANDLE OpenFileMappingW(DWORD, BOOL, LPCWSTR lpName)
  OpenFileMappingW: fn(3, (ctx) => { wstr(ctx, ctx.arg(2), 'lpName'); ctx.setLastError(ERROR_FILE_NOT_FOUND); return 0; }),
  // HANDLE CreateSemaphoreW(LPSECURITY_ATTRIBUTES, LONG lInitialCount, LONG lMaximumCount, LPCWSTR lpName)
  CreateSemaphoreW: fn(4, (ctx) => {
    checkUnnamed(ctx, ctx.arg(0), ctx.arg(3), 'CreateSemaphoreW');
    const initial = ctx.argI32(1), max = ctx.argI32(2);
    if (max <= 0 || initial < 0 || initial > max) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    ctx.setLastError(0);
    return addHandle(ctx, { kind: 'semaphore', count: initial, max });
  }),
  // BOOL ReleaseSemaphore(HANDLE, LONG lReleaseCount, LPLONG lpPreviousCount)
  ReleaseSemaphore: fn(3, (ctx) => {
    const s = getHandle(ctx, ctx.arg(0), 'hSemaphore', 'semaphore');
    const n = ctx.argI32(1), prevP = ctx.arg(2);
    if (n <= 0) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    if (s.count + n > s.max) { ctx.setLastError(ERROR_TOO_MANY_POSTS); return 0; }
    if (prevP !== 0n) ctx.write32(prevP, s.count);
    s.count += n;
    return 1;
  }),
  // DWORD WaitForSingleObject(HANDLE, DWORD dwMilliseconds)
  WaitForSingleObject: fn(2, (ctx) => {
    const obj = getHandle(ctx, ctx.arg(0), 'hHandle', 'event', 'semaphore');
    const ms = ctx.arg32(1);
    if (signaled(ctx, obj, 'WaitForSingleObject')) { consume(obj); return WAIT_OBJECT_0; }
    if (ms === 0) return WAIT_TIMEOUT;
    ctx.fail(`deadlock: waiting ${ms === INFINITE ? 'forever' : `${ms} ms`} on ${hex(ctx.arg(0))}, which only another thread could signal`);
  }),
  // DWORD WaitForMultipleObjects(DWORD nCount, const HANDLE *lpHandles, BOOL bWaitAll, DWORD dwMilliseconds)
  WaitForMultipleObjects: fn(4, (ctx) => {
    const n = ctx.arg32(0), arr = ctx.arg(1), all = ctx.arg32(2) !== 0, ms = ctx.arg32(3);
    if (n === 0 || n > 64) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0xffff_ffff; }
    const objs = Array.from({ length: n }, (_, i) => getHandle(ctx, ctx.read64(arr + 8n * BigInt(i)), 'lpHandles[i]', 'event', 'semaphore'));
    const ready = objs.map((o) => signaled(ctx, o, 'WaitForMultipleObjects'));
    if (all ? ready.every(Boolean) : ready.some(Boolean)) {
      if (all) { objs.forEach(consume); return WAIT_OBJECT_0; }
      const i = ready.indexOf(true);
      consume(objs[i]);
      return WAIT_OBJECT_0 + i;
    }
    if (ms === 0) return WAIT_TIMEOUT;
    ctx.fail(`deadlock: waiting on ${n} objects, which only another thread could signal`);
  }),
  // --- thread-local storage ---
  // DWORD TlsAlloc(void)
  TlsAlloc: fn(0, (ctx) => {
    const used = sys(ctx).tlsUsed;
    for (let i = 0; i < TLS_MINIMUM_AVAILABLE; i++) {
      if (!used.has(i)) {
        used.add(i);
        ctx.write64(ctx.teb() + TEB_TLS_SLOTS + 8n * BigInt(i), 0n);
        return i;
      }
    }
    ctx.fail('TlsAlloc: more than 64 TLS slots are not supported');
  }),
  // BOOL TlsFree(DWORD dwTlsIndex)
  TlsFree: fn(1, (ctx) => {
    const i = ctx.arg32(0);
    const used = sys(ctx).tlsUsed;
    if (!used.has(i)) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    used.delete(i);
    ctx.write64(ctx.teb() + TEB_TLS_SLOTS + 8n * BigInt(i), 0n);
    return 1;
  }),
  // BOOL TlsSetValue(DWORD dwTlsIndex, LPVOID lpTlsValue)
  TlsSetValue: fn(2, (ctx) => {
    const i = ctx.arg32(0);
    if (i >= TLS_MINIMUM_AVAILABLE) {
      if (i < TLS_MINIMUM_AVAILABLE + 1024) ctx.fail(`TlsSetValue: expansion slot ${i} is not supported`);
      ctx.setLastError(ERROR_INVALID_PARAMETER);
      return 0;
    }
    ctx.write64(ctx.teb() + TEB_TLS_SLOTS + 8n * BigInt(i), ctx.arg(1));
    return 1;
  }),

  // --- memory ---
  // LPVOID VirtualAlloc(LPVOID lpAddress, SIZE_T dwSize, DWORD flAllocationType, DWORD flProtect)
  VirtualAlloc: fn(4, (ctx) => {
    const addr = ctx.arg(0), size = ctx.arg(1), type = ctx.arg32(2), protect = ctx.arg32(3);
    if (addr !== 0n) ctx.fail(`VirtualAlloc at a given address ${hex(addr)} is not supported`);
    if (protect !== PAGE_READWRITE) ctx.fail(`flProtect ${hex(protect)} is not supported`);
    // With lpAddress NULL, MEM_COMMIT alone reserves and commits too.
    const t = type & ~MEM_TOP_DOWN;
    if (t !== MEM_COMMIT && t !== (MEM_COMMIT | MEM_RESERVE)) ctx.fail(`flAllocationType ${hex(type)} is not supported (only MEM_COMMIT, with or without MEM_RESERVE)`);
    if (size === 0n) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0n; }
    try {
      return vmAlloc(ctx, size);
    } catch (e) {
      if (e?.stop?.error?.includes('exhausted')) { ctx.setLastError(ERROR_NOT_ENOUGH_MEMORY); return 0n; }
      throw e;
    }
  }),
  // BOOL VirtualFree(LPVOID lpAddress, SIZE_T dwSize, DWORD dwFreeType)
  VirtualFree: fn(3, (ctx) => {
    const addr = ctx.arg(0), size = ctx.arg(1), type = ctx.arg32(2);
    if (type === MEM_DECOMMIT) ctx.fail('MEM_DECOMMIT is not supported');
    if (type !== MEM_RELEASE) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    if (size !== 0n) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    if (!vm(ctx).regions.has(addr)) { ctx.setLastError(ERROR_INVALID_ADDRESS); return 0; }
    vmFree(ctx, addr);
    return 1;
  }),
  // HLOCAL LocalFree(HLOCAL hMem): blocks from FormatMessageW's FORMAT_MESSAGE_ALLOCATE_BUFFER.
  LocalFree: fn(1, (ctx) => {
    const p = ctx.arg(0);
    if (p === 0n) return 0n;
    const s = sys(ctx);
    s.localBlocks ??= new Set();
    if (!s.localBlocks.has(p)) ctx.fail(`LocalFree(${hex(p)}): not a block the shim allocated`);
    s.localBlocks.delete(p);
    return 0n;
  }),

  // DWORD FormatMessageW(DWORD dwFlags, LPCVOID lpSource, DWORD dwMessageId, DWORD dwLanguageId,
  //   LPWSTR lpBuffer, DWORD nSize, va_list *Arguments)
  FormatMessageW: fn(7, (ctx) => {
    const flags = ctx.arg32(0), id = ctx.arg32(2), lang = ctx.arg32(3), bufP = ctx.arg(4), size = ctx.arg32(5);
    const known = FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_IGNORE_INSERTS | FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_MAX_WIDTH_MASK;
    if (!(flags & FORMAT_MESSAGE_FROM_SYSTEM) || (flags & ~known)) ctx.fail(`dwFlags ${hex(flags)} is not supported`);
    if (!(flags & FORMAT_MESSAGE_IGNORE_INSERTS)) ctx.fail('messages with inserts are not supported');
    if (flags & FORMAT_MESSAGE_MAX_WIDTH_MASK) ctx.fail('FORMAT_MESSAGE_MAX_WIDTH_MASK is not supported');
    if (lang !== 0) ctx.fail(`dwLanguageId ${hex(lang)} is not supported`);
    const text = SYSTEM_MESSAGES.get(id);
    if (text === undefined) ctx.fail(`no message text for error ${id}`);
    const msg = text + '\r\n';
    if (flags & FORMAT_MESSAGE_ALLOCATE_BUFFER) {
      const p = ctx.alloc(BigInt(2 * msg.length + 2));
      writeWString(ctx, p, msg);
      const s = sys(ctx);
      s.localBlocks ??= new Set();
      s.localBlocks.add(p);
      ctx.write64(bufP, p);
      return msg.length;
    }
    if (size <= msg.length) { ctx.setLastError(ERROR_INSUFFICIENT_BUFFER); return 0; }
    writeWString(ctx, bufP, msg);
    return msg.length;
  }),

  // int lstrlenW(LPCWSTR)
  lstrlenW: fn(1, (ctx) => (ctx.arg(0) === 0n ? 0 : ctx.readWString(ctx.arg(0)).length)),

  // --- the console: stdout and stderr are redirected ---
  // BOOL GetConsoleMode(HANDLE, LPDWORD)
  GetConsoleMode: fn(2, (ctx) => {
    getHandle(ctx, ctx.arg(0), 'hConsoleHandle', 'std', 'file');
    ctx.setLastError(ERROR_INVALID_HANDLE);
    return 0;
  }),
  // BOOL SetConsoleMode(HANDLE, DWORD)
  SetConsoleMode: fn(2, (ctx) => {
    getHandle(ctx, ctx.arg(0), 'hConsoleHandle', 'std', 'file');
    ctx.setLastError(ERROR_INVALID_HANDLE);
    return 0;
  }),
  // BOOL GetConsoleScreenBufferInfo(HANDLE, PCONSOLE_SCREEN_BUFFER_INFO)
  GetConsoleScreenBufferInfo: fn(2, (ctx) => {
    getHandle(ctx, ctx.arg(0), 'hConsoleOutput', 'std', 'file');
    ctx.setLastError(ERROR_INVALID_HANDLE);
    return 0;
  }),
  // BOOL SetConsoleCtrlHandler(PHANDLER_ROUTINE, BOOL Add): recorded, never called.
  SetConsoleCtrlHandler: fn(2, (ctx) => {
    const h = ctx.arg(0), add = ctx.arg32(1) !== 0;
    const list = sys(ctx).ctrlHandlers;
    if (h === 0n) return 1;
    if (add) list.push(h);
    else {
      const i = list.lastIndexOf(h);
      if (i < 0) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
      list.splice(i, 1);
    }
    return 1;
  }),
};

export { utf16, win32, Win32Error, INVALID_HANDLE_VALUE };
