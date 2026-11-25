// Host files of a Wine process (docs/design/wine-unix.md, section 8, "Host
// files (M4)"): a caller's files are written into the prefix's drive_c with
// the boot set, with their mtimes, and read back after the run. Host-neutral:
// it works on Emscripten's FS, so Node and a Worker share it. The Node-only
// side (a host directory in, a host directory out) is host-dir.mjs.
//
// A file is { path, bytes, mtime } and an empty or listed directory is
// { path, dir: true, mtime }: `path` is relative to drive_c with `/`, `mtime`
// is in ms since the epoch. MEMFS keeps milliseconds, and Wine reads the mtime
// as the last write time (get_file_times, unix/file.c).

const normalize = (path) => {
  if (typeof path !== 'string' || path === '' || path.startsWith('/') || path.includes('\\')) {
    throw new Error(`host file path ${JSON.stringify(path)}: must be relative to drive_c, with '/'`);
  }
  const parts = path.split('/');
  if (parts.some((p) => p === '' || p === '.' || p === '..')) {
    throw new Error(`host file path ${JSON.stringify(path)}: empty, '.' or '..' component`);
  }
  return parts.join('/');
};

const exists = (FS, path) => {
  try { FS.lstat(path); return true; } catch { return false; }
};
const isDir = (FS, path) => FS.isDir(FS.lstat(path).mode);

/**
 * Writes the files under `<drivePath>/` (the prefix's drive_c), creating the
 * parents, then sets every mtime (directories last, since writing into one
 * changes it). A path that already exists as something else than a directory
 * stops: the prefix's own files, and the symlinks to the lazy PE files, are
 * never overwritten.
 * @param {object} FS  Emscripten's FS
 * @param {string} drivePath  the MEMFS path of drive_c
 * @param {{path: string, bytes?: Uint8Array, dir?: boolean, mtime: number}[]} files
 */
export function writeHostFiles(FS, drivePath, files) {
  const base = drivePath.replace(/\/+$/, '');
  const times = [];
  for (const f of files) {
    const rel = normalize(f.path);
    const path = `${base}/${rel}`;
    if (typeof f.mtime !== 'number' || !Number.isFinite(f.mtime)) throw new Error(`host file ${rel}: mtime must be a number of ms`);
    if (f.dir) {
      if (exists(FS, path) && !isDir(FS, path)) throw new Error(`host file ${rel}: exists and is not a directory`);
      FS.mkdirTree(path);
    } else {
      if (!(f.bytes instanceof Uint8Array)) throw new Error(`host file ${rel}: bytes must be a Uint8Array`);
      if (exists(FS, path)) throw new Error(`host file ${rel}: already exists in the prefix`);
      const parent = path.slice(0, path.lastIndexOf('/'));
      FS.mkdirTree(parent);
      FS.writeFile(path, f.bytes);
    }
    times.push([path, f.mtime, !!f.dir]);
  }
  // Files first, then directories deepest first.
  times.sort((a, b) => (a[2] === b[2] ? (a[2] ? b[0].length - a[0].length : 0) : a[2] ? 1 : -1));
  for (const [path, mtime] of times) FS.utime(path, mtime, mtime);
}

/**
 * The tree under `<drivePath>/<dir>` as the same list `writeHostFiles` takes:
 * every file and directory below it (not `dir` itself), paths relative to
 * drive_c, in sorted path order, with the bytes (a copy) and mtime of each.
 * Reads MEMFS directly: nothing in the instance runs.
 */
export function collectHostFiles(FS, drivePath, dir) {
  const base = drivePath.replace(/\/+$/, '');
  const rel = normalize(dir);
  const out = [];
  const walk = (path, relPath) => {
    for (const name of FS.readdir(path)) {
      if (name === '.' || name === '..') continue;
      const p = `${path}/${name}`;
      const r = `${relPath}/${name}`;
      const st = FS.lstat(p);
      const mtime = new Date(st.mtime).getTime();
      if (FS.isDir(st.mode)) {
        out.push({ path: r, dir: true, mtime });
        walk(p, r);
      } else if (FS.isFile(st.mode)) {
        out.push({ path: r, bytes: new Uint8Array(FS.readFile(p)), mtime });
      } else {
        throw new Error(`collect: ${r} is neither a file nor a directory`);
      }
    }
  };
  if (!exists(FS, `${base}/${rel}`)) return [];
  walk(`${base}/${rel}`, rel);
  out.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  return out;
}
