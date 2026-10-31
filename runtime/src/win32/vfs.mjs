// The in-memory file system of a Phase 1 process (docs/design/runtime.md,
// "Files, handles and the 7-Zip shim (M5)"): one NTFS volume C:, names in
// UTF-16, case-insensitive and case-preserving, with Win32 path rules and
// Win32 error codes. Pure JS, so it runs in Node and in browsers; the host
// directory loader is vfs-host.mjs.
//
// Path rules follow Windows (Microsoft's "Naming Files, Paths, and
// Namespaces"; the details checked against Wine 11 ntdll/path.c:
// RtlGetFullPathName_U and collapse_path). Errors follow NTFS as Win32 reports
// them (Wine 11 kernelbase/file.c and ntdll/unix/file.c for the mapping).

import {
  Win32Error, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION,
  ERROR_FILE_EXISTS, ERROR_INVALID_NAME, ERROR_DIR_NOT_EMPTY, ERROR_ALREADY_EXISTS, ERROR_DIRECTORY,
  ERROR_INVALID_PARAMETER,
} from './errors.mjs';

export const FILE_ATTRIBUTE_READONLY = 0x1;
export const FILE_ATTRIBUTE_HIDDEN = 0x2;
export const FILE_ATTRIBUTE_SYSTEM = 0x4;
export const FILE_ATTRIBUTE_DIRECTORY = 0x10;
export const FILE_ATTRIBUTE_ARCHIVE = 0x20;
export const FILE_ATTRIBUTE_NORMAL = 0x80;
export const FILE_ATTRIBUTE_TEMPORARY = 0x100;
export const FILE_ATTRIBUTE_OFFLINE = 0x1000;
export const FILE_ATTRIBUTE_NOT_CONTENT_INDEXED = 0x2000;
/** The attributes SetFileAttributesW and CreateFileW can set (FILE_ATTRIBUTE_VALID_SET_FLAGS minus the ones NTFS needs special calls for). */
export const SETTABLE_ATTRIBUTES = FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM
  | FILE_ATTRIBUTE_ARCHIVE | FILE_ATTRIBUTE_TEMPORARY | FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED;

// CreateFileW access, share and disposition values (winnt.h, winbase.h).
export const GENERIC_READ = 0x8000_0000, GENERIC_WRITE = 0x4000_0000, GENERIC_EXECUTE = 0x2000_0000, GENERIC_ALL = 0x1000_0000;
export const FILE_READ_DATA = 0x1, FILE_WRITE_DATA = 0x2, FILE_APPEND_DATA = 0x4;
export const FILE_READ_ATTRIBUTES = 0x80, FILE_WRITE_ATTRIBUTES = 0x100, DELETE = 0x1_0000;
export const FILE_SHARE_READ = 0x1, FILE_SHARE_WRITE = 0x2, FILE_SHARE_DELETE = 0x4;
export const CREATE_NEW = 1, CREATE_ALWAYS = 2, OPEN_EXISTING = 3, OPEN_ALWAYS = 4, TRUNCATE_EXISTING = 5;
export const FILE_FLAG_BACKUP_SEMANTICS = 0x0200_0000;

/** FILETIME of 1970-01-01 (100 ns ticks since 1601-01-01). */
export const UNIX_EPOCH_FILETIME = 116_444_736_000_000_000n;
/** The volume's fixed serial number and size (runtime.md: fixed values). */
export const VOLUME_SERIAL = 0x5445_4d50; // "TEMP"
export const VOLUME_TOTAL_BYTES = 256n << 30n;
export const VOLUME_FREE_BYTES = 128n << 30n;
export const CLUSTER_BYTES = 4096n;

/** The host's current time as a FILETIME. */
export const hostNow = () => BigInt(Date.now()) * 10_000n + UNIX_EPOCH_FILETIME;

/** NTFS compares names by their upper-cased UTF-16 code units. */
export function nameKey(name) {
  let k = '';
  for (let i = 0; i < name.length; i++) {
    const c = name[i];
    const u = c.toUpperCase();
    k += u.length === 1 ? u : c;
  }
  return k;
}

/** Ordinal order of upper-cased names (NTFS directory order). */
export const compareKeys = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

const INVALID_CHARS = new Set(['<', '>', ':', '"', '|', '?', '*', '/', '\\']);
/** A valid NTFS file name: no control characters and none of < > : " | ? * / \. */
export function validName(name) {
  if (name.length === 0 || name.length > 255) return false;
  for (const c of name) if (c.charCodeAt(0) < 0x20 || INVALID_CHARS.has(c)) return false;
  return true;
}

const isSep = (c) => c === '\\' || c === '/';
const DOS_DEVICES = /^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(\s*\..*)?$/i;

export class Node {
  constructor(kind, name, time) {
    this.kind = kind;
    this.name = name;
    this.parent = null;
    this.attrs = kind === 'dir' ? FILE_ATTRIBUTE_DIRECTORY : FILE_ATTRIBUTE_ARCHIVE;
    this.ctime = time;
    this.atime = time;
    this.mtime = time;
    /** Directories: upper-cased name -> Node. */
    this.children = kind === 'dir' ? new Map() : null;
    /** Files: contents (capacity may exceed size). */
    this.data = new Uint8Array(0);
    this.size = 0;
    /** Open handles (OpenFile). */
    this.opens = new Set();
    this.id = 0;
    /** Set when the contents, times or attributes change (for the write-back). */
    this.dirty = true;
    /** The host entry this node was loaded from (vfs-host.mjs), if any. */
    this.host = null;
  }

  get isDir() { return this.kind === 'dir'; }

  /** The node's full Win32 path. */
  path() {
    const parts = [];
    for (let n = this; n.parent; n = n.parent) parts.unshift(n.name);
    return 'C:\\' + parts.join('\\');
  }

  /** Children in NTFS order. */
  sortedChildren() {
    return [...this.children.entries()].sort((a, b) => compareKeys(a[0], b[0])).map((e) => e[1]);
  }
}

/** An open file or directory: its node, access, sharing and file pointer. */
export class OpenFile {
  constructor(node, access, share) {
    this.node = node;
    this.access = access;
    this.share = share;
    this.pos = 0n;
    /** SetFileTime with 0xFFFFFFFF_FFFFFFFF, or an explicit time: no automatic updates through this handle. */
    this.keepMtime = false;
    this.keepAtime = false;
    this.closed = false;
  }

  get canRead() { return (this.access & (GENERIC_READ | GENERIC_ALL | FILE_READ_DATA)) !== 0; }
  get canWrite() { return (this.access & (GENERIC_WRITE | GENERIC_ALL | FILE_WRITE_DATA | FILE_APPEND_DATA)) !== 0; }
}

/** The data access an open asks for, for share-mode checks: read, write, delete. */
function dataAccess(access) {
  return {
    read: (access & (GENERIC_READ | GENERIC_ALL | GENERIC_EXECUTE | FILE_READ_DATA)) !== 0,
    write: (access & (GENERIC_WRITE | GENERIC_ALL | FILE_WRITE_DATA | FILE_APPEND_DATA)) !== 0,
    delete: (access & (DELETE | GENERIC_ALL)) !== 0,
  };
}

export class Vfs {
  /** @param {{now?: () => bigint}} [opts] */
  constructor({ now = hostNow } = {}) {
    this.now = now;
    this.nextId = 1;
    const t = now();
    this.root = this.newNode('dir', '', t);
    this.cwd = 'C:\\';
    for (const p of ['C:\\Windows', 'C:\\Windows\\System32', 'C:\\work', 'C:\\tempwind']) this.createDirectory(p);
    this.cwd = 'C:\\work';
    for (const n of this.walk()) n.dirty = false;
  }

  newNode(kind, name, time) {
    const n = new Node(kind, name, time);
    n.id = this.nextId++;
    return n;
  }

  /** Every node below (and including) `from`, parents before children. */
  *walk(from = this.root) {
    yield from;
    if (from.isDir) for (const c of from.sortedChildren()) yield* this.walk(c);
  }

  // --- paths ---------------------------------------------------------------

  /**
   * GetFullPathNameW: the absolute Win32 path for `name`, or null if it has
   * none (empty, or only spaces). Relative paths resolve against the current
   * directory; `/` becomes `\`; `.` and `..` collapse; the last component
   * loses trailing dots and spaces. `\\?\` paths are returned unchanged.
   */
  fullPathName(name) {
    if (!name.length || /^ *$/.test(name)) return null;
    if (name.startsWith('\\\\?\\')) return name;
    const cwd = this.cwd;
    let prefix, rest, mark;
    if (isSep(name[0]) && isSep(name[1])) {
      // UNC (\\server\share\...) or a device (\\.\x): kept as given, collapsed after the prefix.
      const isDevice = (name[2] === '.' || name[2] === '?') && (name.length === 3 || isSep(name[3]));
      if (isDevice) { prefix = '\\\\' + name[2] + '\\'; rest = name.slice(4); mark = 4; } else {
        prefix = ''; rest = name; mark = 0;
        // Skip \\server\share\ as the mark.
        let i = 2;
        while (i < name.length && !isSep(name[i])) i++;
        if (i < name.length) i++;
        while (i < name.length && !isSep(name[i])) i++;
        mark = i;
      }
    } else if (/^[A-Za-z]:/.test(name)) {
      const drive = name[0].toUpperCase();
      if (isSep(name[2])) { prefix = drive + ':\\'; rest = name.slice(3); } else if (drive === cwd[0].toUpperCase()) {
        prefix = cwd.endsWith('\\') ? cwd : cwd + '\\'; rest = name.slice(2);
      } else { prefix = drive + ':\\'; rest = name.slice(2); }
      mark = 3;
    } else if (isSep(name[0])) {
      prefix = cwd.slice(0, 2) + '\\'; rest = name.slice(1); mark = 3;
    } else {
      prefix = cwd.endsWith('\\') ? cwd : cwd + '\\'; rest = name; mark = 3;
    }
    return collapse(prefix + rest, mark);
  }

  /**
   * Parses a Win32 path into the components below C:\. Throws Win32Error:
   * ERROR_PATH_NOT_FOUND for empty names, other drives and UNC paths,
   * ERROR_INVALID_NAME for malformed `\\?\` paths. Device paths stop loudly.
   * @returns {{comps: string[], trailing: boolean}}
   */
  parse(path) {
    let full;
    if (path.startsWith('\\\\?\\')) {
      full = path.slice(4);
      if (/^UNC\\/i.test(full)) throw new Win32Error(ERROR_PATH_NOT_FOUND, `UNC path ${path}`);
      if (!/^[A-Za-z]:\\/.test(full)) {
        if (/^[A-Za-z]:$/.test(full)) throw new VfsStop(`the volume device ${path} is not supported`);
        throw new Win32Error(ERROR_PATH_NOT_FOUND, `${path}`);
      }
    } else {
      if (isSep(path[0]) && isSep(path[1]) && (path[2] === '.' || path[2] === '?') && (path.length === 3 || isSep(path[3]))) {
        throw new VfsStop(`device path ${path} is not supported`);
      }
      full = this.fullPathName(path);
      if (full === null) throw new Win32Error(ERROR_PATH_NOT_FOUND, `empty path`);
      if (isSep(full[0]) && isSep(full[1])) throw new Win32Error(ERROR_PATH_NOT_FOUND, `UNC path ${path}`);
      const last = full.slice(full.lastIndexOf('\\') + 1);
      if (DOS_DEVICES.test(last)) throw new VfsStop(`DOS device name ${path} is not supported`);
    }
    if (full[0].toUpperCase() !== 'C') throw new Win32Error(ERROR_PATH_NOT_FOUND, `drive ${full[0]}: does not exist`);
    let tail = full.slice(3);
    let trailing = false;
    if (tail.endsWith('\\')) { trailing = true; tail = tail.slice(0, -1); }
    const comps = tail === '' ? [] : tail.split('\\');
    for (const c of comps) {
      if (c === '' || c === '.' || c === '..') throw new Win32Error(ERROR_INVALID_NAME, `bad component in ${path}`);
    }
    return { comps, trailing };
  }

  /**
   * Finds a path's parent directory and node. Throws ERROR_PATH_NOT_FOUND
   * when a directory on the way is missing or a file, ERROR_INVALID_NAME
   * for an invalid component.
   * @returns {{parent: Node | null, name: string, node: Node | null, trailing: boolean}}
   */
  locate(path) {
    const { comps, trailing } = this.parse(path);
    let dir = this.root;
    for (let i = 0; i < comps.length - 1; i++) {
      if (!validName(comps[i])) throw new Win32Error(ERROR_INVALID_NAME, `invalid name ${comps[i]}`);
      const child = dir.children.get(nameKey(comps[i]));
      if (!child || !child.isDir) throw new Win32Error(ERROR_PATH_NOT_FOUND, `${path}: no directory ${comps[i]}`);
      dir = child;
    }
    if (comps.length === 0) return { parent: null, name: '', node: this.root, trailing };
    const name = comps[comps.length - 1];
    if (!validName(name)) throw new Win32Error(ERROR_INVALID_NAME, `invalid name ${name}`);
    return { parent: dir, name, node: dir.children.get(nameKey(name)) ?? null, trailing };
  }

  /** The node at `path`; ERROR_FILE_NOT_FOUND if only the last component is missing. */
  stat(path) {
    const r = this.locate(path);
    if (!r.node) throw new Win32Error(ERROR_FILE_NOT_FOUND, `${path} not found`);
    if (r.trailing && !r.node.isDir) throw new Win32Error(ERROR_INVALID_NAME, `${path}: a file with a trailing backslash`);
    return r.node;
  }

  /** The directory at `path` (ERROR_DIRECTORY for a file). */
  directory(path) {
    const n = this.stat(path);
    if (!n.isDir) throw new Win32Error(ERROR_DIRECTORY, `${path} is not a directory`);
    return n;
  }

  // --- changes ---------------------------------------------------------------

  touchDir(dir) {
    dir.mtime = this.now();
    dir.dirty = true;
  }

  link(dir, node) {
    node.parent = dir;
    dir.children.set(nameKey(node.name), node);
    this.touchDir(dir);
  }

  unlink(node) {
    const dir = node.parent;
    dir.children.delete(nameKey(node.name));
    node.parent = null;
    this.touchDir(dir);
  }

  /** CreateDirectoryW. */
  createDirectory(path) {
    const r = this.locate(path);
    if (r.node) throw new Win32Error(ERROR_ALREADY_EXISTS, `${path} exists`);
    const d = this.newNode('dir', r.name, this.now());
    this.link(r.parent, d);
    return d;
  }

  /** RemoveDirectoryW. */
  removeDirectory(path) {
    const r = this.locate(path);
    const n = r.node;
    if (!n) throw new Win32Error(ERROR_FILE_NOT_FOUND, `${path} not found`);
    if (!n.isDir) throw new Win32Error(ERROR_DIRECTORY, `${path} is not a directory`);
    if (n === this.root) throw new Win32Error(ERROR_ACCESS_DENIED, 'the root directory');
    // The open for DELETE comes first (the current directory is held open without FILE_SHARE_DELETE).
    if (this.isCwdOrAbove(n) || n.opens.size) throw new Win32Error(ERROR_SHARING_VIOLATION, `${path} is in use`);
    if (n.attrs & FILE_ATTRIBUTE_READONLY) throw new Win32Error(ERROR_ACCESS_DENIED, `${path} is read-only`);
    if (n.children.size) throw new Win32Error(ERROR_DIR_NOT_EMPTY, `${path} is not empty`);
    this.unlink(n);
  }

  isCwdOrAbove(n) {
    let cwd;
    try { cwd = this.stat(this.cwd); } catch { return false; }
    for (let c = cwd; c; c = c.parent) if (c === n) return true;
    return false;
  }

  /** DeleteFileW. */
  deleteFile(path) {
    const r = this.locate(path);
    const n = r.node;
    if (!n) throw new Win32Error(ERROR_FILE_NOT_FOUND, `${path} not found`);
    if (n.isDir) throw new Win32Error(ERROR_ACCESS_DENIED, `${path} is a directory`);
    if (r.trailing) throw new Win32Error(ERROR_INVALID_NAME, `${path}: a file with a trailing backslash`);
    if (n.attrs & FILE_ATTRIBUTE_READONLY) throw new Win32Error(ERROR_ACCESS_DENIED, `${path} is read-only`);
    for (const o of n.opens) {
      if (!(o.share & FILE_SHARE_DELETE)) throw new Win32Error(ERROR_SHARING_VIOLATION, `${path} is open`);
    }
    if (n.opens.size) throw new VfsStop(`DeleteFileW(${path}) on a file with open handles is not supported`);
    this.unlink(n);
  }

  /** MoveFileWithProgressW without a callback: a rename on the volume. */
  move(from, to, replace) {
    const src = this.locate(from);
    if (!src.node) throw new Win32Error(ERROR_FILE_NOT_FOUND, `${from} not found`);
    const n = src.node;
    if (n === this.root) throw new Win32Error(ERROR_ACCESS_DENIED, 'the root directory');
    for (const o of n.opens) if (!(o.share & FILE_SHARE_DELETE)) throw new Win32Error(ERROR_SHARING_VIOLATION, `${from} is open`);
    if (n.isDir && this.isCwdOrAbove(n)) throw new Win32Error(ERROR_SHARING_VIOLATION, `${from} is in use`);
    const dst = this.locate(to);
    for (let p = dst.parent; p; p = p.parent) {
      if (p === n) throw new Win32Error(ERROR_SHARING_VIOLATION, `${to} is inside ${from}`);
    }
    if (dst.node && dst.node !== n) {
      if (!replace) throw new Win32Error(ERROR_ALREADY_EXISTS, `${to} exists`);
      if (dst.node.isDir || n.isDir) throw new Win32Error(ERROR_ACCESS_DENIED, `${to} is a directory`);
      if (dst.node.attrs & FILE_ATTRIBUTE_READONLY) throw new Win32Error(ERROR_ACCESS_DENIED, `${to} is read-only`);
      if (dst.node.opens.size) throw new Win32Error(ERROR_ACCESS_DENIED, `${to} is open`);
      this.unlink(dst.node);
    }
    if (n.opens.size) throw new VfsStop(`renaming ${from} with open handles is not supported`);
    this.unlink(n);
    n.name = dst.name;
    this.link(dst.parent, n);
    n.dirty = true;
  }

  /** SetFileAttributesW. */
  setAttributes(path, attrs) {
    const n = this.stat(path);
    if (n === this.root) throw new Win32Error(ERROR_ACCESS_DENIED, 'the root directory');
    const base = n.isDir ? FILE_ATTRIBUTE_DIRECTORY : 0;
    n.attrs = base | (attrs & SETTABLE_ATTRIBUTES);
    n.dirty = true;
  }

  /**
   * CreateFileW on the volume (no devices). Returns the OpenFile and the
   * last error a success sets (0 or ERROR_ALREADY_EXISTS).
   * @returns {{file: OpenFile, lastError: number}}
   */
  open(path, { access, share, disposition, attrs = 0 }) {
    if (disposition < CREATE_NEW || disposition > TRUNCATE_EXISTING) throw new Win32Error(ERROR_INVALID_PARAMETER, `disposition ${disposition}`);
    const r = this.locate(path);
    let n = r.node;
    const want = dataAccess(access);
    let lastError = 0;
    if (n) {
      if (n.isDir) {
        if (!(attrs & FILE_FLAG_BACKUP_SEMANTICS) || (disposition !== OPEN_EXISTING && disposition !== OPEN_ALWAYS)) {
          throw new Win32Error(ERROR_ACCESS_DENIED, `${path} is a directory`);
        }
      } else {
        if (r.trailing) throw new Win32Error(ERROR_INVALID_NAME, `${path}: a file with a trailing backslash`);
        if (disposition === CREATE_NEW) throw new Win32Error(ERROR_FILE_EXISTS, `${path} exists`);
        const overwrite = disposition === CREATE_ALWAYS || disposition === TRUNCATE_EXISTING;
        if ((n.attrs & FILE_ATTRIBUTE_READONLY) && (want.write || want.delete || overwrite)) {
          throw new Win32Error(ERROR_ACCESS_DENIED, `${path} is read-only`);
        }
        if (disposition === CREATE_ALWAYS) {
          const hs = FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM;
          if ((n.attrs & hs) & ~(attrs & hs)) throw new Win32Error(ERROR_ACCESS_DENIED, `${path} is hidden or system`);
        }
        if (disposition === TRUNCATE_EXISTING && !want.write) throw new Win32Error(ERROR_INVALID_PARAMETER, 'TRUNCATE_EXISTING without write access');
      }
      this.checkSharing(n, access, share, path);
      if (!n.isDir) {
        if (disposition === CREATE_ALWAYS || disposition === TRUNCATE_EXISTING) {
          n.size = 0;
          n.data = new Uint8Array(0);
          n.mtime = this.now();
          if (disposition === CREATE_ALWAYS) n.attrs = (attrs & SETTABLE_ATTRIBUTES) | FILE_ATTRIBUTE_ARCHIVE;
          else n.attrs |= FILE_ATTRIBUTE_ARCHIVE;
          n.dirty = true;
        }
        if (disposition === CREATE_ALWAYS || disposition === OPEN_ALWAYS) lastError = ERROR_ALREADY_EXISTS;
      } else if (disposition === OPEN_ALWAYS) lastError = ERROR_ALREADY_EXISTS;
    } else {
      if (disposition === OPEN_EXISTING || disposition === TRUNCATE_EXISTING) throw new Win32Error(ERROR_FILE_NOT_FOUND, `${path} not found`);
      if (r.trailing || !r.parent) throw new Win32Error(ERROR_INVALID_NAME, `${path}: cannot create a file with a trailing backslash`);
      n = this.newNode('file', r.name, this.now());
      n.attrs = (attrs & SETTABLE_ATTRIBUTES) | FILE_ATTRIBUTE_ARCHIVE;
      this.link(r.parent, n);
    }
    const f = new OpenFile(n, access, share);
    n.opens.add(f);
    return { file: f, lastError };
  }

  /** Share modes: each side's access must be allowed by the other's share mode. */
  checkSharing(n, access, share, path) {
    const want = dataAccess(access);
    // An open without read, write or delete access takes no part in sharing.
    if (!want.read && !want.write && !want.delete) return;
    for (const o of n.opens) {
      const have = dataAccess(o.access);
      const conflict = (want.read && !(o.share & FILE_SHARE_READ)) || (want.write && !(o.share & FILE_SHARE_WRITE))
        || (want.delete && !(o.share & FILE_SHARE_DELETE)) || (have.read && !(share & FILE_SHARE_READ))
        || (have.write && !(share & FILE_SHARE_WRITE)) || (have.delete && !(share & FILE_SHARE_DELETE));
      if (conflict) throw new Win32Error(ERROR_SHARING_VIOLATION, `${path} is open with an incompatible share mode`);
    }
  }

  close(f) {
    if (f.closed) return;
    f.closed = true;
    f.node.opens.delete(f);
  }

  /** Reads up to `len` bytes at the file pointer. */
  read(f, len) {
    const n = f.node;
    const pos = f.pos;
    if (pos >= BigInt(n.size)) return new Uint8Array(0);
    const start = Number(pos);
    const end = Math.min(n.size, start + len);
    f.pos = BigInt(end);
    return n.data.slice(start, end);
  }

  /** Writes at the file pointer, extending the file (and zero-filling a gap). */
  write(f, bytes) {
    const n = f.node;
    const start = Number(f.pos);
    const end = start + bytes.length;
    if (bytes.length) {
      this.resize(n, Math.max(n.size, end));
      n.data.set(bytes, start);
      f.pos = BigInt(end);
    }
    if (!f.keepMtime) n.mtime = this.now();
    n.attrs |= FILE_ATTRIBUTE_ARCHIVE;
    n.dirty = true;
  }

  /** Sets a file's size (SetEndOfFile, writes). */
  resize(n, size) {
    if (size > n.data.length) {
      const grown = new Uint8Array(Math.max(size, Math.min(2 * n.data.length, n.data.length + (64 << 20))));
      grown.set(n.data.subarray(0, n.size));
      n.data = grown;
    } else if (size < n.size) {
      n.data.fill(0, size, n.size);
    }
    n.size = size;
    n.dirty = true;
  }

  /** Replaces a file's contents (loading from the host). */
  setContents(n, bytes) {
    n.data = bytes;
    n.size = bytes.length;
  }

  /** The contents of a file, exactly `size` bytes. */
  contents(n) { return n.data.subarray(0, n.size); }

  /** Adds a file with given contents and times (loader, tests). */
  addFile(path, bytes, { time = this.now(), attrs = FILE_ATTRIBUTE_ARCHIVE } = {}) {
    const r = this.locate(path);
    if (r.node) throw new Win32Error(ERROR_ALREADY_EXISTS, `${path} exists`);
    const n = this.newNode('file', r.name, time);
    n.attrs = attrs;
    this.setContents(n, bytes);
    this.link(r.parent, n);
    return n;
  }
}

/** A case the file system doesn't model: the shim stops loudly. */
export class VfsStop extends Error {}

/**
 * Windows' collapse_path rules from `mark` on: `/` becomes `\`, duplicate
 * backslashes go, `.` and `..` components collapse (never above the mark),
 * an intermediate component loses one trailing dot, and the path loses its
 * trailing dots and spaces.
 */
function collapse(path, mark) {
  const a = Array.from(path.replace(/\//g, '\\'));
  // Duplicate backslashes, from max(1, mark) on.
  let w = Math.max(1, mark);
  for (let r = w; r < a.length; r++) if (a[r] !== '\\' || a[w - 1] !== '\\') a[w++] = a[r];
  a.length = w;
  let p = mark;
  const del = (at, n) => a.splice(at, n);
  while (p < a.length) {
    if (a[p] === '.') {
      const c1 = a[p + 1];
      if (c1 === '\\') { del(p, 2); continue; }
      if (c1 === undefined) { if (p > mark) p--; a.length = p; continue; }
      if (c1 === '.') {
        const c2 = a[p + 2];
        if (c2 === '\\') {
          const next = p + 3;
          let q = p;
          if (q > mark) { q--; while (q > mark && a[q - 1] !== '\\') q--; }
          del(q, next - q);
          p = q;
          continue;
        }
        if (c2 === undefined) {
          let q = p;
          if (q > mark) { q--; while (q > mark && a[q - 1] !== '\\') q--; if (q > mark) q--; }
          a.length = q;
          p = q;
          continue;
        }
      }
    }
    while (p < a.length && a[p] !== '\\') p++;
    if (a[p] === '\\') {
      if (p > mark && a[p - 1] === '.') del(p - 1, 1);
      else p++;
    }
  }
  p = a.length;
  while (p > mark && (a[p - 1] === ' ' || a[p - 1] === '.')) p--;
  a.length = p;
  return a.join('');
}

export const _collapse = collapse;
