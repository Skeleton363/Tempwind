// kernel32.dll: files, directories, find handles and the volume
// (runtime.md, "Files, handles and the 7-Zip shim (M5)"). The file system is
// vfs.mjs; the behaviour follows Microsoft's documentation, with the details
// (error codes, last-error rules) checked against Wine 11 kernelbase/file.c
// and kernelbase/volume.c.

import { INVALID_HANDLE_VALUE } from '../abi.mjs';
import {
  Win32Error, ERROR_INVALID_PARAMETER, ERROR_ACCESS_DENIED, ERROR_NEGATIVE_SEEK, ERROR_PATH_NOT_FOUND,
  ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, ERROR_HANDLE_EOF, ERROR_INVALID_FUNCTION, ERROR_DIRECTORY,
  ERROR_INVALID_NAME, ERROR_INSUFFICIENT_BUFFER,
} from './errors.mjs';
import {
  FILE_ATTRIBUTE_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS, VOLUME_SERIAL, VOLUME_TOTAL_BYTES, VOLUME_FREE_BYTES,
  CLUSTER_BYTES, nameKey, FILE_WRITE_ATTRIBUTES, GENERIC_WRITE, GENERIC_ALL,
} from './vfs.mjs';
import { addHandle, getHandle, handleTable } from './handles.mjs';
import { hex, fn, wstr, utf16, copyOut, win32, writeWString } from './util.mjs';

export const INVALID_FILE_ATTRIBUTES = 0xffff_ffff;
export const INVALID_SET_FILE_POINTER = 0xffff_ffff;
const FILE_BEGIN = 0, FILE_CURRENT = 1, FILE_END = 2;

// CreateFileW flags (winbase.h).
const FILE_FLAG_WRITE_THROUGH = 0x8000_0000, FILE_FLAG_OVERLAPPED = 0x4000_0000, FILE_FLAG_NO_BUFFERING = 0x2000_0000;
const FILE_FLAG_RANDOM_ACCESS = 0x1000_0000, FILE_FLAG_SEQUENTIAL_SCAN = 0x0800_0000, FILE_FLAG_DELETE_ON_CLOSE = 0x0400_0000;
const FILE_FLAG_OPEN_REPARSE_POINT = 0x0020_0000;
const SUPPORTED_FLAGS = FILE_FLAG_WRITE_THROUGH | FILE_FLAG_NO_BUFFERING | FILE_FLAG_RANDOM_ACCESS | FILE_FLAG_SEQUENTIAL_SCAN
  | FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | 0xffff;

const MOVEFILE_REPLACE_EXISTING = 0x1, MOVEFILE_COPY_ALLOWED = 0x2, MOVEFILE_WRITE_THROUGH = 0x8;

/** WIN32_FIND_DATAW: attributes, three FILETIMEs, size, reserved, cFileName[260], cAlternateFileName[14]. */
export const WIN32_FIND_DATAW_SIZE = 592;
/** WIN32_FIND_STREAM_DATA: StreamSize, cStreamName[MAX_PATH + 36]. */
export const WIN32_FIND_STREAM_DATA_SIZE = 600;
/** BY_HANDLE_FILE_INFORMATION. */
export const BY_HANDLE_FILE_INFORMATION_SIZE = 52;

const DRIVE_NO_ROOT_DIR = 1, DRIVE_FIXED = 3;
/** GetVolumeInformationW flags of an NTFS volume on Windows 10/11 (case-preserved, Unicode, ACLs, compression, quotas, sparse files, reparse points, object ids, EFS, named streams, read-only support, transactions, open-by-id, USN, hard links, extended attributes, block refcounting off). */
export const NTFS_FLAGS = 0x03e7_02ff;

/** The OpenFile behind a file handle. */
function fileOf(ctx, h, api) {
  const obj = getHandle(ctx, h, api, 'file', 'std');
  return obj;
}

/** Validates the parts of a CreateFileW call the shim doesn't model. */
function checkCreateArgs(ctx, sa, flags, template) {
  if (sa !== 0n) ctx.fail(`lpSecurityAttributes ${hex(sa)} is not supported`);
  if (template !== 0n) ctx.fail(`hTemplateFile ${hex(template)} is not supported`);
  if (flags & FILE_FLAG_OVERLAPPED) ctx.fail('FILE_FLAG_OVERLAPPED is not supported');
  if (flags & FILE_FLAG_DELETE_ON_CLOSE) ctx.fail('FILE_FLAG_DELETE_ON_CLOSE is not supported');
  if (flags & ~SUPPORTED_FLAGS) ctx.fail(`dwFlagsAndAttributes ${hex(flags)} is not supported`);
}

/** Converts a find mask into the NT form (FindFirstFileExW's fixup: `?` -> `>`, `.?`/`.*` -> `"`, final `*.` -> `<`). */
export function fixupMask(mask) {
  if (!mask.length) return mask;
  const a = Array.from(mask);
  const noExt = a[a.length - 1] === '.';
  let len = a.length;
  while (len && (a[len - 1] === '.' || a[len - 1] === ' ')) len--;
  for (let i = 0; i < len; i++) {
    if (a[i] === '.' && (a[i + 1] === '*' || a[i + 1] === '?')) a[i] = '"';
    else if (a[i] === '?') a[i] = '>';
  }
  a.length = len;
  if (noExt && len && a[len - 1] === '*') a[len - 1] = '<';
  return a.join('');
}

const isWild = (c) => c === '*' || c === '?' || c === '<' || c === '>' || c === '"';
const up = (c) => nameKey(c);

/** NT file-name matching with the DOS wildcards `*`, `?`, `<`, `>` and `"`, case-insensitive. */
export function matchMask(name, mask) {
  if (name === '..') name = '.';
  return matchPart(name, 0, mask, 0);
}

function matchPart(name, n, mask, m) {
  while (n < name.length && m < mask.length) {
    const mc = mask[m];
    if (mc === '*') {
      m++;
      while (m < mask.length && mask[m] === '*') m++;
      if (m === mask.length) return true;
      while (n < name.length) {
        const c = mask[m] === '"' ? '.' : mask[m];
        if (!isWild(c)) while (n < name.length && up(name[n]) !== up(c)) n++;
        if (matchPart(name, n, mask, m)) return true;
        n++;
      }
      break;
    } else if (mc === '<') {
      m++;
      let hadDot = false;
      while (n < name.length) {
        let next = n;
        while (next < name.length && name[next] !== '.') next++;
        if (next === name.length && hadDot) break;
        if (next < name.length) { hadDot = true; next++; }
        if (m < mask.length) {
          while (n < next) {
            const c = mask[m] === '"' ? '.' : mask[m];
            if (!isWild(c)) while (n < next && up(name[n]) !== up(c)) n++;
            if (matchPart(name, n, mask, m)) return true;
            n++;
          }
        }
        n = next;
      }
      break;
    } else if (mc === '?') {
      m++; n++;
    } else if (mc === '>') {
      m++;
      if (name[n] === '.') {
        while (m < mask.length && mask[m] === '>') m++;
        if (m === mask.length) n++;
      } else n++;
    } else {
      const c = mc === '"' ? '.' : mc;
      if (up(c) !== up(name[n])) return false;
      m++; n++;
    }
  }
  while (m < mask.length && (mask[m] === '*' || mask[m] === '<' || mask[m] === '"' || mask[m] === '>')) m++;
  return n === name.length && m === mask.length;
}

/** The WIN32_FIND_DATAW bytes for directory entry `name` describing `node`. */
function findData(name, node) {
  const b = new Uint8Array(WIN32_FIND_DATAW_SIZE);
  const dv = new DataView(b.buffer);
  dv.setUint32(0, node.attrs, true);
  dv.setBigUint64(4, node.ctime, true);
  dv.setBigUint64(12, node.atime, true);
  dv.setBigUint64(20, node.mtime, true);
  const size = node.isDir ? 0 : node.size;
  dv.setUint32(28, Math.floor(size / 2 ** 32), true);
  dv.setUint32(32, size >>> 0, true);
  b.set(utf16(name.slice(0, 259)), 44);
  return b;
}

/** FindFirstFileW: the entries of the search, in NTFS order. Throws Win32Error. */
function findEntries(vfs, pattern) {
  // The mask is the last component; the rest names the directory.
  let dirPath, mask;
  if (pattern.startsWith('\\\\?\\')) {
    const i = pattern.lastIndexOf('\\');
    dirPath = pattern.slice(0, i + 1);
    mask = pattern.slice(i + 1);
  } else {
    const full = vfs.fullPathName(pattern);
    if (full === null) throw new Win32Error(ERROR_PATH_NOT_FOUND, 'empty pattern');
    const i = full.lastIndexOf('\\');
    dirPath = full.slice(0, i + 1);
    mask = full.slice(i + 1);
  }
  if (!mask) throw new Win32Error(ERROR_FILE_NOT_FOUND, `${pattern}: no file name`);
  const wild = /[*?<>]/.test(mask);
  if (wild) {
    // The wildcard mask is the original last component (PathFindFileNameW), trailing dots included.
    const j = Math.max(pattern.lastIndexOf('\\'), pattern.lastIndexOf('/'));
    mask = fixupMask(pattern.slice(j + 1));
  }
  let dir;
  try {
    dir = vfs.stat(dirPath);
  } catch (e) {
    if (e instanceof Win32Error && e.code === ERROR_FILE_NOT_FOUND) throw new Win32Error(ERROR_PATH_NOT_FOUND, e.message);
    throw e;
  }
  if (!dir.isDir) throw new Win32Error(ERROR_DIRECTORY, `${dirPath} is not a directory`);
  const all = [];
  if (dir !== vfs.root) all.push(['.', dir], ['..', dir.parent]);
  for (const c of dir.sortedChildren()) all.push([c.name, c]);
  const out = all.filter(([name]) => (wild ? matchMask(name, mask) : nameKey(name) === nameKey(mask)));
  if (!out.length) throw new Win32Error(ERROR_FILE_NOT_FOUND, `${pattern}: no match`);
  return out.map(([name, node]) => findData(name, node));
}

/** GetDiskFreeSpace(Ex)W's root: must name an existing directory on C:. */
function volumeDirectory(ctx, addr) {
  const vfs = ctx.vfs;
  const path = addr === 0n ? vfs.cwd : wstr(ctx, addr, 'lpRootPathName');
  const n = vfs.stat(path);
  if (!n.isDir) throw new Win32Error(ERROR_DIRECTORY, `${path} is not a directory`);
  return n;
}

export default {
  // HANDLE CreateFileW(LPCWSTR lpFileName, DWORD dwDesiredAccess, DWORD dwShareMode,
  //   LPSECURITY_ATTRIBUTES lpSecurityAttributes, DWORD dwCreationDisposition,
  //   DWORD dwFlagsAndAttributes, HANDLE hTemplateFile)
  CreateFileW: fn(7, (ctx) => {
    const nameP = ctx.arg(0), access = ctx.arg32(1), share = ctx.arg32(2), sa = ctx.arg(3);
    const disposition = ctx.arg32(4), flags = ctx.arg32(5), template = ctx.arg(6);
    checkCreateArgs(ctx, sa, flags, template);
    return win32(ctx, INVALID_HANDLE_VALUE, () => {
      if (nameP === 0n) throw new Win32Error(ERROR_PATH_NOT_FOUND, 'lpFileName is NULL');
      const name = wstr(ctx, nameP);
      if (!name) throw new Win32Error(ERROR_PATH_NOT_FOUND, 'empty name');
      const { file, lastError } = ctx.vfs.open(name, { access, share, disposition, attrs: flags });
      ctx.setLastError(lastError);
      return addHandle(ctx, { kind: 'file', file, name });
    });
  }),

  // BOOL ReadFile(HANDLE, LPVOID, DWORD, LPDWORD lpNumberOfBytesRead, LPOVERLAPPED)
  ReadFile: fn(5, (ctx) => {
    const h = ctx.arg(0), buf = ctx.arg(1), count = ctx.arg32(2), readP = ctx.arg(3), ov = ctx.arg(4);
    if (ov !== 0n) ctx.fail('overlapped I/O is not supported');
    if (readP === 0n) ctx.fail('lpNumberOfBytesRead is NULL without lpOverlapped');
    const obj = fileOf(ctx, h, 'hFile');
    if (obj.kind === 'std') {
      if (obj.stream !== 'stdin') ctx.fail(`hFile ${hex(h)}: not a readable handle (${obj.stream})`);
      ctx.write32(readP, 0); // stdin is empty
      return 1;
    }
    const f = obj.file;
    ctx.write32(readP, 0);
    if (f.node.isDir) { ctx.setLastError(ERROR_INVALID_FUNCTION); return 0; }
    if (!f.canRead) { ctx.setLastError(ERROR_ACCESS_DENIED); return 0; }
    if (count) ctx.check(buf, count, 'lpBuffer');
    const bytes = ctx.vfs.read(f, count);
    if (bytes.length) ctx.write(buf, bytes);
    ctx.write32(readP, bytes.length);
    return 1;
  }),

  // BOOL WriteFile(HANDLE, LPCVOID, DWORD, LPDWORD lpNumberOfBytesWritten, LPOVERLAPPED)
  WriteFile: fn(5, (ctx) => {
    const h = ctx.arg(0), buf = ctx.arg(1), count = ctx.arg32(2), written = ctx.arg(3), ov = ctx.arg(4);
    if (ov !== 0n) ctx.fail(`lpOverlapped ${hex(ov)}: overlapped I/O is not supported`);
    // Microsoft: lpNumberOfBytesWritten can be NULL only when lpOverlapped is not NULL.
    if (written === 0n) ctx.fail('lpNumberOfBytesWritten is NULL without lpOverlapped');
    const obj = fileOf(ctx, h, 'hFile');
    if (obj.kind === 'std' && obj.stream === 'stdin') ctx.fail(`hFile ${hex(h)}: not a writable handle (stdin)`);
    const bytes = count ? ctx.read(buf, count) : new Uint8Array(0);
    if (obj.kind === 'std') {
      ctx.write32(written, count);
      ctx.output(obj.stream, bytes);
      return 1;
    }
    const f = obj.file;
    ctx.write32(written, 0);
    if (f.node.isDir) { ctx.setLastError(ERROR_INVALID_FUNCTION); return 0; }
    if (!f.canWrite) { ctx.setLastError(ERROR_ACCESS_DENIED); return 0; }
    ctx.vfs.write(f, bytes);
    ctx.write32(written, count);
    return 1;
  }),

  // DWORD SetFilePointer(HANDLE, LONG lDistanceToMove, PLONG lpDistanceToMoveHigh, DWORD dwMoveMethod)
  SetFilePointer: fn(4, (ctx) => {
    const h = ctx.arg(0), low = ctx.arg32(1), highP = ctx.arg(2), method = ctx.arg32(3);
    const obj = getHandle(ctx, h, 'hFile', 'file');
    const f = obj.file;
    const dist = highP === 0n ? BigInt(ctx.argI32(1)) : (BigInt(ctx.read32(highP) | 0) << 32n) | BigInt(low);
    let pos;
    if (method === FILE_BEGIN) pos = dist;
    else if (method === FILE_CURRENT) pos = f.pos + dist;
    else if (method === FILE_END) pos = BigInt(f.node.size) + dist;
    else { ctx.setLastError(ERROR_INVALID_PARAMETER); return INVALID_SET_FILE_POINTER; }
    if (pos < 0n) { ctx.setLastError(ERROR_NEGATIVE_SEEK); return INVALID_SET_FILE_POINTER; }
    if (pos >= 1n << 53n) ctx.fail(`file position ${pos} is too large`);
    f.pos = pos;
    if (highP !== 0n) ctx.write32(highP, Number(pos >> 32n));
    const lowPart = Number(pos & 0xffff_ffffn);
    if (lowPart === INVALID_SET_FILE_POINTER) ctx.setLastError(0);
    return lowPart;
  }),

  // DWORD GetFileSize(HANDLE hFile, LPDWORD lpFileSizeHigh)
  GetFileSize: fn(2, (ctx) => {
    const obj = getHandle(ctx, ctx.arg(0), 'hFile', 'file');
    const highP = ctx.arg(1);
    const n = obj.file.node;
    const size = n.isDir ? 0 : n.size;
    if (highP !== 0n) ctx.write32(highP, Math.floor(size / 2 ** 32));
    const low = size >>> 0;
    if (low === 0xffff_ffff) ctx.setLastError(0);
    return low;
  }),

  // BOOL SetEndOfFile(HANDLE hFile): the file ends at the file pointer.
  SetEndOfFile: fn(1, (ctx) => {
    const f = getHandle(ctx, ctx.arg(0), 'hFile', 'file').file;
    if (f.node.isDir) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    if (!f.canWrite) { ctx.setLastError(ERROR_ACCESS_DENIED); return 0; }
    if (f.pos >= 1n << 32n) ctx.fail(`a file of ${f.pos} bytes is too large`);
    const vfs = ctx.vfs;
    vfs.resize(f.node, Number(f.pos));
    if (!f.keepMtime) f.node.mtime = vfs.now();
    return 1;
  }),

  // BOOL SetFileTime(HANDLE, const FILETIME *lpCreationTime, const FILETIME *lpLastAccessTime, const FILETIME *lpLastWriteTime)
  SetFileTime: fn(4, (ctx) => {
    const f = getHandle(ctx, ctx.arg(0), 'hFile', 'file').file;
    if (!(f.access & (FILE_WRITE_ATTRIBUTES | GENERIC_WRITE | GENERIC_ALL))) { ctx.setLastError(ERROR_ACCESS_DENIED); return 0; }
    const times = [ctx.arg(1), ctx.arg(2), ctx.arg(3)].map((p) => (p === 0n ? 0n : ctx.read64(p)));
    const n = f.node;
    const KEEP = 0xffff_ffff_ffff_ffffn;
    for (const t of times) {
      if (t === 0xffff_fffe_ffff_ffffn) ctx.fail('SetFileTime with 0xFFFFFFFE_FFFFFFFF (resume updates) is not supported');
      if (t !== 0n && t !== KEEP && t >= 1n << 63n) { ctx.setLastError(ERROR_INVALID_PARAMETER); return 0; }
    }
    const [c, a, m] = times;
    if (c !== 0n && c !== KEEP) n.ctime = c;
    if (a === KEEP) f.keepAtime = true; else if (a !== 0n) { n.atime = a; f.keepAtime = true; }
    if (m === KEEP) f.keepMtime = true; else if (m !== 0n) { n.mtime = m; f.keepMtime = true; }
    n.dirty = true;
    return 1;
  }),

  // BOOL GetFileInformationByHandle(HANDLE, LPBY_HANDLE_FILE_INFORMATION)
  GetFileInformationByHandle: fn(2, (ctx) => {
    const n = getHandle(ctx, ctx.arg(0), 'hFile', 'file').file.node;
    const out = ctx.arg(1);
    const b = new Uint8Array(BY_HANDLE_FILE_INFORMATION_SIZE);
    const dv = new DataView(b.buffer);
    const size = n.isDir ? 0 : n.size;
    dv.setUint32(0, n.attrs, true);
    dv.setBigUint64(4, n.ctime, true);
    dv.setBigUint64(12, n.atime, true);
    dv.setBigUint64(20, n.mtime, true);
    dv.setUint32(28, VOLUME_SERIAL, true);
    dv.setUint32(32, Math.floor(size / 2 ** 32), true);
    dv.setUint32(36, size >>> 0, true);
    dv.setUint32(40, 1, true); // links
    dv.setUint32(44, 0, true); // file index, high
    dv.setUint32(48, n.id, true); // file index, low
    ctx.write(out, b);
    return 1;
  }),

  // DWORD GetFileAttributesW(LPCWSTR lpFileName)
  GetFileAttributesW: fn(1, (ctx) => win32(ctx, INVALID_FILE_ATTRIBUTES, () => {
    const p = ctx.arg(0);
    if (p === 0n) ctx.fail('lpFileName is NULL');
    return ctx.vfs.stat(wstr(ctx, p)).attrs;
  })),

  // BOOL SetFileAttributesW(LPCWSTR lpFileName, DWORD dwFileAttributes)
  SetFileAttributesW: fn(2, (ctx) => win32(ctx, 0, () => {
    ctx.vfs.setAttributes(wstr(ctx, ctx.arg(0), 'lpFileName'), ctx.arg32(1));
    return 1;
  })),

  // BOOL DeleteFileW(LPCWSTR lpFileName)
  DeleteFileW: fn(1, (ctx) => win32(ctx, 0, () => {
    ctx.vfs.deleteFile(wstr(ctx, ctx.arg(0), 'lpFileName'));
    return 1;
  })),

  // BOOL CreateDirectoryW(LPCWSTR lpPathName, LPSECURITY_ATTRIBUTES)
  CreateDirectoryW: fn(2, (ctx) => {
    if (ctx.arg(1) !== 0n) ctx.fail('lpSecurityAttributes is not supported');
    return win32(ctx, 0, () => {
      const path = wstr(ctx, ctx.arg(0), 'lpPathName');
      if (!path) throw new Win32Error(ERROR_PATH_NOT_FOUND, 'empty path');
      ctx.vfs.createDirectory(path);
      return 1;
    });
  }),

  // BOOL RemoveDirectoryW(LPCWSTR lpPathName)
  RemoveDirectoryW: fn(1, (ctx) => win32(ctx, 0, () => {
    ctx.vfs.removeDirectory(wstr(ctx, ctx.arg(0), 'lpPathName'));
    return 1;
  })),

  // BOOL MoveFileW(LPCWSTR lpExistingFileName, LPCWSTR lpNewFileName)
  MoveFileW: fn(2, (ctx) => win32(ctx, 0, () => {
    ctx.vfs.move(wstr(ctx, ctx.arg(0), 'lpExistingFileName'), wstr(ctx, ctx.arg(1), 'lpNewFileName'), false);
    return 1;
  })),

  // BOOL MoveFileWithProgressW(LPCWSTR, LPCWSTR, LPPROGRESS_ROUTINE, LPVOID, DWORD dwFlags)
  MoveFileWithProgressW: fn(5, (ctx) => {
    const flags = ctx.arg32(4);
    if (ctx.arg(2) !== 0n) ctx.fail('a progress routine is not supported');
    if (flags & ~(MOVEFILE_REPLACE_EXISTING | MOVEFILE_COPY_ALLOWED | MOVEFILE_WRITE_THROUGH)) ctx.fail(`dwFlags ${hex(flags)} is not supported`);
    if (ctx.arg(1) === 0n) ctx.fail('lpNewFileName is NULL (delete on reboot) is not supported');
    return win32(ctx, 0, () => {
      ctx.vfs.move(wstr(ctx, ctx.arg(0), 'lpExistingFileName'), wstr(ctx, ctx.arg(1), 'lpNewFileName'), (flags & MOVEFILE_REPLACE_EXISTING) !== 0);
      return 1;
    });
  }),

  // HANDLE FindFirstFileW(LPCWSTR lpFileName, LPWIN32_FIND_DATAW lpFindFileData)
  FindFirstFileW: fn(2, (ctx) => {
    const out = ctx.arg(1);
    ctx.check(out, WIN32_FIND_DATAW_SIZE, 'lpFindFileData');
    return win32(ctx, INVALID_HANDLE_VALUE, () => {
      const pattern = wstr(ctx, ctx.arg(0), 'lpFileName');
      const entries = findEntries(ctx.vfs, pattern);
      ctx.write(out, entries[0]);
      return addHandle(ctx, { kind: 'find', entries, next: 1 });
    });
  }),

  // BOOL FindNextFileW(HANDLE hFindFile, LPWIN32_FIND_DATAW lpFindFileData)
  FindNextFileW: fn(2, (ctx) => {
    const f = getHandle(ctx, ctx.arg(0), 'hFindFile', 'find');
    if (!f.entries) ctx.fail('FindNextFileW on a stream search handle');
    if (f.next >= f.entries.length) { ctx.setLastError(ERROR_NO_MORE_FILES); return 0; }
    ctx.write(ctx.arg(1), f.entries[f.next++]);
    return 1;
  }),

  // BOOL FindClose(HANDLE hFindFile)
  FindClose: fn(1, (ctx) => {
    getHandle(ctx, ctx.arg(0), 'hFindFile', 'find');
    handleTable(ctx).map.delete(BigInt.asUintN(64, ctx.arg(0)));
    return 1;
  }),

  // HANDLE FindFirstStreamW(LPCWSTR lpFileName, STREAM_INFO_LEVELS InfoLevel, LPVOID lpFindStreamData, DWORD dwFlags):
  // no alternate data streams, so a file has only ::$DATA and a directory none.
  FindFirstStreamW: fn(4, (ctx) => {
    if (ctx.arg32(1) !== 0) { ctx.setLastError(ERROR_INVALID_PARAMETER); return INVALID_HANDLE_VALUE; }
    if (ctx.arg32(3) !== 0) { ctx.setLastError(ERROR_INVALID_PARAMETER); return INVALID_HANDLE_VALUE; }
    const out = ctx.arg(2);
    ctx.check(out, WIN32_FIND_STREAM_DATA_SIZE, 'lpFindStreamData');
    return win32(ctx, INVALID_HANDLE_VALUE, () => {
      const n = ctx.vfs.stat(wstr(ctx, ctx.arg(0), 'lpFileName'));
      if (n.isDir) throw new Win32Error(ERROR_HANDLE_EOF, 'a directory has no streams');
      const b = new Uint8Array(WIN32_FIND_STREAM_DATA_SIZE);
      new DataView(b.buffer).setBigUint64(0, BigInt(n.size), true);
      b.set(utf16('::$DATA'), 8);
      ctx.write(out, b);
      return addHandle(ctx, { kind: 'find', entries: null, streams: true });
    });
  }),

  // BOOL FindNextStreamW(HANDLE hFindStream, LPVOID lpFindStreamData)
  FindNextStreamW: fn(2, (ctx) => {
    const f = getHandle(ctx, ctx.arg(0), 'hFindStream', 'find');
    if (!f.streams) ctx.fail('FindNextStreamW on a file search handle');
    ctx.setLastError(ERROR_HANDLE_EOF);
    return 0;
  }),

  // DWORD GetCurrentDirectoryW(DWORD nBufferLength, LPWSTR lpBuffer)
  GetCurrentDirectoryW: fn(2, (ctx) => {
    const size = ctx.arg32(0), buf = ctx.arg(1);
    const cwd = ctx.vfs.cwd;
    if (size > cwd.length && buf === 0n) ctx.fail('lpBuffer is NULL');
    return copyOut(ctx, buf, size, cwd);
  }),

  // BOOL SetCurrentDirectoryW(LPCWSTR lpPathName)
  SetCurrentDirectoryW: fn(1, (ctx) => win32(ctx, 0, () => {
    const vfs = ctx.vfs;
    const path = wstr(ctx, ctx.arg(0), 'lpPathName');
    if (path.startsWith('\\\\?\\')) ctx.fail('SetCurrentDirectoryW with a \\\\?\\ path is not supported');
    const n = vfs.stat(path);
    if (!n.isDir) throw new Win32Error(ERROR_DIRECTORY, `${path} is not a directory`);
    let full = vfs.fullPathName(path);
    if (full.length > 3 && full.endsWith('\\')) full = full.slice(0, -1);
    vfs.cwd = full;
    return 1;
  })),

  // BOOL GetDiskFreeSpaceExW(LPCWSTR lpDirectoryName, PULARGE_INTEGER lpFreeBytesAvailableToCaller,
  //   PULARGE_INTEGER lpTotalNumberOfBytes, PULARGE_INTEGER lpTotalNumberOfFreeBytes)
  GetDiskFreeSpaceExW: fn(4, (ctx) => win32(ctx, 0, () => {
    volumeDirectory(ctx, ctx.arg(0));
    const [avail, total, free] = [ctx.arg(1), ctx.arg(2), ctx.arg(3)];
    if (avail !== 0n) ctx.write64(avail, VOLUME_FREE_BYTES);
    if (total !== 0n) ctx.write64(total, VOLUME_TOTAL_BYTES);
    if (free !== 0n) ctx.write64(free, VOLUME_FREE_BYTES);
    return 1;
  })),

  // BOOL GetDiskFreeSpaceW(LPCWSTR lpRootPathName, LPDWORD lpSectorsPerCluster, LPDWORD lpBytesPerSector,
  //   LPDWORD lpNumberOfFreeClusters, LPDWORD lpTotalNumberOfClusters)
  GetDiskFreeSpaceW: fn(5, (ctx) => win32(ctx, 0, () => {
    volumeDirectory(ctx, ctx.arg(0));
    const clamp = (n) => (n > 0xffff_ffffn ? 0xffff_ffff : Number(n));
    const vals = [8, 512, clamp(VOLUME_FREE_BYTES / CLUSTER_BYTES), clamp(VOLUME_TOTAL_BYTES / CLUSTER_BYTES)];
    vals.forEach((v, i) => { const p = ctx.arg(1 + i); if (p !== 0n) ctx.write32(p, v); });
    return 1;
  })),

  // UINT GetDriveTypeW(LPCWSTR lpRootPathName)
  GetDriveTypeW: fn(1, (ctx) => {
    const p = ctx.arg(0);
    if (p === 0n) return DRIVE_FIXED; // the current directory's drive
    const root = wstr(ctx, p);
    return /^[Cc]:\\?$/.test(root) ? DRIVE_FIXED : DRIVE_NO_ROOT_DIR;
  }),

  // DWORD GetLogicalDriveStringsW(DWORD nBufferLength, LPWSTR lpBuffer): "C:\" and a final NUL.
  GetLogicalDriveStringsW: fn(2, (ctx) => {
    const size = ctx.arg32(0), buf = ctx.arg(1);
    if (size < 5) return 5;
    ctx.write(buf, utf16('C:\\\0'));
    return 4;
  }),

  // BOOL GetVolumeInformationW(LPCWSTR lpRootPathName, LPWSTR lpVolumeNameBuffer, DWORD nVolumeNameSize,
  //   LPDWORD lpVolumeSerialNumber, LPDWORD lpMaximumComponentLength, LPDWORD lpFileSystemFlags,
  //   LPWSTR lpFileSystemNameBuffer, DWORD nFileSystemNameSize)
  GetVolumeInformationW: fn(8, (ctx) => win32(ctx, 0, () => {
    const rootP = ctx.arg(0);
    const root = rootP === 0n ? 'C:\\' : wstr(ctx, rootP);
    if (!/^[Cc]:\\$/.test(root)) {
      if (/^[A-Za-z]:\\$/.test(root)) throw new Win32Error(ERROR_PATH_NOT_FOUND, `no drive ${root}`);
      ctx.fail(`GetVolumeInformationW(${root}): only a drive root is supported`);
    }
    const [nameBuf, nameSize, serialP, maxCompP, flagsP, fsBuf, fsSize] = [ctx.arg(1), ctx.arg32(2), ctx.arg(3), ctx.arg(4), ctx.arg(5), ctx.arg(6), ctx.arg32(7)];
    const putStr = (buf, size, s) => {
      if (buf === 0n) return;
      if (size <= s.length) throw new Win32Error(ERROR_INSUFFICIENT_BUFFER, 'buffer too small');
      writeWString(ctx, buf, s);
    };
    putStr(nameBuf, nameSize, '');
    putStr(fsBuf, fsSize, 'NTFS');
    if (serialP !== 0n) ctx.write32(serialP, VOLUME_SERIAL);
    if (maxCompP !== 0n) ctx.write32(maxCompP, 255);
    if (flagsP !== 0n) ctx.write32(flagsP, NTFS_FLAGS);
    return 1;
  })),
};

export { FILE_ATTRIBUTE_DIRECTORY, ERROR_INVALID_NAME };
