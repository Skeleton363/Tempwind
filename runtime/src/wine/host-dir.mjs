// `run-wine.mjs --dir HOST` (docs/design/wine-unix.md, section 8, "Host
// files (M4)"): a host directory loaded as the `files` of a Wine process and
// written back from `collect`'s list. Node only (node:fs). These are Phase
// 1's rules: new and changed files with their mtimes, new directories,
// deletions, and a file replaced by a directory (or the reverse) removed
// first; names invalid on Windows, or equal without case, fail at load.
import * as fs from 'node:fs';
import * as path from 'node:path';

const INVALID_CHARS = /[<>:"|?*\\/\u0000-\u001f]/;
const DOS_DEVICES = /^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(\s*\..*)?$/i;
/** A valid NTFS file name: 1-255 characters, no control characters and none of < > : " | ? * / \, not a DOS device. */
export function validWindowsName(name) {
  return name.length > 0 && name.length <= 255 && !INVALID_CHARS.test(name) && !DOS_DEVICES.test(name);
}
/** Names are equal when equal after per-character upper-casing (NTFS, as Phase 1's VFS). */
const nameKey = (name) => Array.from(name, (c) => {
  const u = c.toUpperCase();
  return u.length === 1 ? u : c;
}).join('');

const mtimeMs = (st) => Number(st.mtimeNs / 1000000n);

/**
 * Reads HOST as a `files` list: `under` itself as a directory, then every file
 * and directory below it, in sorted order.
 * @param {string} hostDir
 * @param {string} under  the path in drive_c ('work' for C:\work)
 * @throws on a name invalid on Windows, two names equal without case, or an entry that is neither a file nor a directory
 */
export function loadHostDir(hostDir, under = 'work') {
  const out = [{ path: under, dir: true, mtime: mtimeMs(fs.statSync(hostDir, { bigint: true })) }];
  const load = (hostPath, rel) => {
    const seen = new Map();
    const ents = fs.readdirSync(hostPath, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
    for (const ent of ents) {
      const hp = path.join(hostPath, ent.name);
      if (!validWindowsName(ent.name)) throw new Error(`${hp} is not a valid Windows file name`);
      const key = nameKey(ent.name);
      if (seen.has(key)) throw new Error(`${hp} differs from ${seen.get(key)} only in case`);
      seen.set(key, ent.name);
      const st = fs.statSync(hp, { bigint: true });
      const r = `${rel}/${ent.name}`;
      if (ent.isDirectory()) {
        out.push({ path: r, dir: true, mtime: mtimeMs(st) });
        load(hp, r);
      } else if (ent.isFile()) {
        out.push({ path: r, bytes: new Uint8Array(fs.readFileSync(hp)), mtime: mtimeMs(st) });
      } else {
        throw new Error(`${hp} is neither a file nor a directory`);
      }
    }
  };
  load(hostDir, under);
  return out;
}

const sameBytes = (a, b) => a.length === b.length && Buffer.compare(a, b) === 0;

/**
 * Writes what `collect` returned back to HOST.
 * @param {string} hostDir
 * @param {string} under  the same path as loadHostDir's
 * @param {object[]} before  loadHostDir's list
 * @param {object[]} after  the process's `collect(under)`: the tree below `under`
 * @param {number} [rootMtime]  the mtime of `under` itself after the run (ms); omitted: left as the host has it
 */
export function writeBackHostDir(hostDir, under, before, after, rootMtime) {
  const old = new Map(before.map((f) => [f.path, f]));
  const now = new Map(after.map((f) => [f.path, f]));
  const dirTimes = [];
  const touched = new Set([hostDir]);
  const hostPath = (rel) => path.join(hostDir, ...rel.slice(under.length + 1).split('/'));
  const parentOf = (hp) => path.dirname(hp);
  const secs = (ms) => ms / 1000;

  // Host entries the guest deleted (or renamed, including a change of case): remove them, deepest not needed (recursive).
  for (const f of before) {
    if (f.path === under) continue;
    if (!now.has(f.path)) {
      const hp = hostPath(f.path);
      if (fs.lstatSync(hp, { throwIfNoEntry: false })) {
        fs.rmSync(hp, { recursive: true, force: true });
        touched.add(parentOf(hp));
      }
    }
  }
  // after is in sorted path order, so a directory comes before its contents.
  for (const f of after) {
    const hp = hostPath(f.path);
    const was = old.get(f.path);
    const hst = fs.lstatSync(hp, { throwIfNoEntry: false });
    if (hst && hst.isDirectory() !== !!f.dir) {
      fs.rmSync(hp, { recursive: true, force: true });
      touched.add(parentOf(hp));
    }
    const kindKept = was && !!was.dir === !!f.dir && fs.lstatSync(hp, { throwIfNoEntry: false });
    if (f.dir) {
      if (!kindKept) {
        fs.mkdirSync(hp, { recursive: true });
        touched.add(parentOf(hp));
        touched.add(hp);
      }
      if (!kindKept || was.mtime !== f.mtime) touched.add(hp);
      dirTimes.push([hp, f.mtime]);
    } else if (!kindKept || was.mtime !== f.mtime || !sameBytes(was.bytes, f.bytes)) {
      fs.writeFileSync(hp, f.bytes);
      fs.utimesSync(hp, secs(f.mtime), secs(f.mtime));
      touched.add(parentOf(hp));
    }
  }
  // Directory times last (writing into a directory changes it), deepest first, only where the run changed something.
  dirTimes.sort((a, b) => b[0].length - a[0].length);
  for (const [hp, mtime] of dirTimes) if (touched.has(hp)) fs.utimesSync(hp, secs(mtime), secs(mtime));
  if (rootMtime !== undefined && touched.has(hostDir)) fs.utimesSync(hostDir, secs(rootMtime), secs(rootMtime));
}
