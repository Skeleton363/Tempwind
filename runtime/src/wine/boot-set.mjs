// The boot set (docs/design/wine-unix.md, section 8): what MEMFS holds
// before `boot`. Host-neutral: the caller reads the files (from target/wine
// in Node, from the static assets in the browser) and passes their bytes.
//
// - the prefix at its recorded path: its .reg files, the dosdevices
//   symlinks and drive_c's directories (not drive_c's files);
// - Wine's install tree at the constant paths of section 8: the PE files the
//   boot opens under PE_DIR, the data files (closure.json's data_files, the
//   NLS files and wine.inf) under DATA_DIR;
// - the programs, by DOS path on C: (C:\tempwind\hello.exe), in drive_c;
// - overlay.reg, wineboot's per-boot keys (native/wine/prefix/overlay.reg),
//   at OVERLAY_PATH, which the server loads right after user.reg.

/** Where init_paths finds the PE files (section 8). */
export const PE_DIR = '/wine/lib/wine/x86_64-windows';
/** Where init_paths finds NLS and wine.inf (section 8). */
export const DATA_DIR = '/wine/share/wine';
/** Where the server loads the overlay from, right after user.reg (section 8). */
export const OVERLAY_PATH = '/wine/share/tempwind/overlay.reg';
/** The PE files the Unix side opens before LdrInitializeThunk: load_ntdll and load_apiset_dll. */
export const BOOT_DLLS = Object.freeze(['ntdll.dll', 'apisetschema.dll']);

/**
 * Writes the boot set into MEMFS.
 * @param {object} FS  Emscripten's FS (host.FS)
 * @param {object} set
 * @param {string} set.prefixPath  the prefix's recorded path (WINEPREFIX)
 * @param {Record<string, Uint8Array>} set.prefixFiles  top-level prefix files by name (system.reg, user.reg, userdef.reg, ...)
 * @param {string[]} set.prefixDirs  the prefix's directories, relative (drive_c, drive_c/windows, ...)
 * @param {[string, string][]} set.prefixLinks  symlinks [relative path, target] (dosdevices/c:, ...)
 * @param {Record<string, Uint8Array>} set.dlls  PE files by name, for PE_DIR
 * @param {Record<string, Uint8Array>} set.data  data files by path relative to DATA_DIR (nls/locale.nls, wine.inf)
 * @param {Record<string, Uint8Array>} set.programs  programs by DOS path on C: (C:\tempwind\hello.exe)
 * @param {Uint8Array} set.overlay  native/wine/prefix/overlay.reg
 */
export function installBootSet(FS, set) {
  const prefix = set.prefixPath.replace(/\/+$/, '');
  FS.mkdirTree(prefix);
  for (const dir of set.prefixDirs) FS.mkdirTree(`${prefix}/${dir}`);
  for (const [path, target] of set.prefixLinks) {
    FS.mkdirTree(parent(`${prefix}/${path}`));
    FS.symlink(target, `${prefix}/${path}`);
  }
  for (const [name, bytes] of Object.entries(set.prefixFiles)) FS.writeFile(`${prefix}/${name}`, bytes);
  writeTree(FS, PE_DIR, set.dlls);
  writeTree(FS, DATA_DIR, set.data);
  FS.mkdirTree(parent(OVERLAY_PATH));
  FS.writeFile(OVERLAY_PATH, set.overlay);
  for (const [dosPath, bytes] of Object.entries(set.programs)) {
    const m = /^[cC]:\\(.+)$/.exec(dosPath);
    if (!m) throw new Error(`${dosPath}: a program's path must be a DOS path on C:`);
    const path = `${prefix}/drive_c/${m[1].replaceAll('\\', '/')}`;
    FS.mkdirTree(parent(path));
    FS.writeFile(path, bytes);
  }
}

/** boot's flags (abi.md v5, "Host entries of the Unix side"). */
export const BOOT_STOP_AT_ENTRY = 1;

/**
 * Enters `boot` (section 8): writes argv into the host heap through the raw
 * `malloc` (argv[0] /wine/bin/wine, argv[1] the program as a DOS path, then
 * its arguments) and calls boot through `promising`.
 * @param {object} host  a createUnixHost of wine-unix
 * @param {string[]} args  the program's DOS path, then its arguments
 * @param {number} [flags]  BOOT_STOP_AT_ENTRY, M2's only mode
 * @returns {Promise<number>} boot's result: 0 once stopped at the entry
 */
export function boot(host, args, flags = BOOT_STOP_AT_ENTRY) {
  const argv = ['/wine/bin/wine', ...args];
  const encoder = new TextEncoder();
  const malloc = (size) => {
    const p = host.raw.malloc(BigInt(size));
    if (p === 0n) throw new Error(`boot: malloc(${size}) failed`);
    return p;
  };
  const ptrs = argv.map((s) => {
    const bytes = encoder.encode(s + '\0');
    const p = malloc(bytes.length);
    new Uint8Array(host.memory.buffer, Number(p), bytes.length).set(bytes);
    return p;
  });
  const array = malloc(8 * (ptrs.length + 1));
  const view = new BigUint64Array(host.memory.buffer, Number(array), ptrs.length + 1);
  ptrs.forEach((p, i) => { view[i] = p; });
  view[ptrs.length] = 0n;
  return host.call('boot', argv.length, array, flags);
}

/**
 * Records the paths the module opens from now on (every FS.open, which the
 * open syscalls go through), absolute, in order.
 * @returns {{ paths: string[], stop: () => void }}
 */
export function recordOpens(FS) {
  const paths = [];
  const open = FS.open;
  FS.open = function (path, ...rest) {
    const stream = open.call(this, path, ...rest);
    paths.push(typeof path === 'string' ? stream.path ?? path : path);
    return stream;
  };
  return { paths, stop() { FS.open = open; } };
}

function writeTree(FS, base, files) {
  for (const [rel, bytes] of Object.entries(files)) {
    const path = `${base}/${rel}`;
    FS.mkdirTree(parent(path));
    FS.writeFile(path, bytes);
  }
}

function parent(path) {
  return path.slice(0, path.lastIndexOf('/')) || '/';
}
