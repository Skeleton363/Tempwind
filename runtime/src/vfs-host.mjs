// `run-pe --dir HOST` (runtime.md, "The file system"): loads a host directory
// into the file system as C:\work and writes C:\work back when the run ends.
// Node only (node:fs); the file system itself (win32/vfs.mjs) is pure JS.
import * as fs from 'node:fs';
import * as path from 'node:path';
import { UNIX_EPOCH_FILETIME, nameKey, FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_DIRECTORY } from './win32/vfs.mjs';

export const WORK_DIR = 'C:\\work';

const toFileTime = (ns) => ns / 100n + UNIX_EPOCH_FILETIME;
/** A FILETIME as the seconds `fs.utimesSync` takes (exact for whole and half seconds and most fractions). */
const toUnixSeconds = (ft) => Number(ft - UNIX_EPOCH_FILETIME) / 1e7;

/**
 * Copies HOST into C:\work: each file keeps its contents and mtime, which
 * becomes all three times; files get FILE_ATTRIBUTE_ARCHIVE and directories
 * FILE_ATTRIBUTE_DIRECTORY. Names must be valid Windows names and unique
 * without case.
 */
export function loadHostDir(vfs, hostDir) {
  const root = vfs.stat(WORK_DIR);
  const load = (dirNode, hostPath) => {
    for (const ent of fs.readdirSync(hostPath, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
      const hp = path.join(hostPath, ent.name);
      if (dirNode.children.has(nameKey(ent.name))) throw new Error(`--dir: ${hp} differs from another name only in case`);
      const st = fs.statSync(hp, { bigint: true });
      const t = toFileTime(st.mtimeNs);
      let n;
      if (ent.isDirectory()) {
        n = vfs.newNode('dir', ent.name, t);
        n.attrs = FILE_ATTRIBUTE_DIRECTORY;
      } else if (ent.isFile()) {
        n = vfs.newNode('file', ent.name, t);
        n.attrs = FILE_ATTRIBUTE_ARCHIVE;
        vfs.setContents(n, new Uint8Array(fs.readFileSync(hp)));
      } else {
        throw new Error(`--dir: ${hp} is neither a file nor a directory`);
      }
      n.parent = dirNode;
      dirNode.children.set(nameKey(ent.name), n);
      n.host = { name: ent.name };
      if (n.isDir) load(n, hp);
      n.dirty = false;
    }
  };
  load(root, hostDir);
  root.dirty = false;
}

/**
 * Writes C:\work back to HOST: deletions, new directories, new and changed
 * files with their times, then the times of directories (last, since writing
 * files changes them on the host).
 */
export function writeBack(vfs, hostDir) {
  const root = vfs.stat(WORK_DIR);
  const dirTimes = [];
  const sync = (dirNode, hostPath) => {
    // Host entries that are gone (or renamed, including a change of case).
    for (const name of fs.readdirSync(hostPath)) {
      const n = dirNode.children.get(nameKey(name));
      if (!n || n.name !== name) fs.rmSync(path.join(hostPath, name), { recursive: true, force: true });
    }
    for (const n of dirNode.sortedChildren()) {
      const hp = path.join(hostPath, n.name);
      if (n.isDir) {
        if (!fs.existsSync(hp)) fs.mkdirSync(hp);
        sync(n, hp);
        if (n.dirty || !n.host) dirTimes.push([hp, n]);
      } else if (n.dirty || !n.host || !fs.existsSync(hp)) {
        fs.writeFileSync(hp, vfs.contents(n));
        fs.utimesSync(hp, toUnixSeconds(n.atime), toUnixSeconds(n.mtime));
      }
    }
  };
  sync(root, hostDir);
  if (root.dirty) dirTimes.push([hostDir, root]);
  for (const [hp, n] of dirTimes) fs.utimesSync(hp, toUnixSeconds(n.atime), toUnixSeconds(n.mtime));
}
