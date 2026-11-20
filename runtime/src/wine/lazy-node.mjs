// Lazy boot-set nodes (docs/design/wine-unix.md, section 8, "Lazy nodes"):
// a MEMFS file whose size comes from the boot-set manifest, so a stat works
// before the first read, and whose bytes are loaded whole on the first
// access to its contents (a read, a write, a truncate). Wine stats a file
// before it reads it (unix/file.c L2970, server/fd.c L1999, server/mapping.c
// L1087 and L1158), which Emscripten's FS.createLazyFile can't serve in Node
// (docs/dev/research.md, 2026-11-14).
//
// How: MEMFS keeps a file's bytes in `node.contents` and its size in
// `node.usedBytes`; getattr and llseek read only `usedBytes`. The node gets
// accessors for both: `usedBytes` answers the manifest's size, and the first
// access to `contents` loads the file and turns both back into the plain
// properties MEMFS expects. Host-neutral: the caller passes the loader
// (fs in Node, `xhrReader` in a Worker).

/**
 * Creates a lazy file at `path` (its parent directories too).
 * @param {object} FS  Emscripten's FS
 * @param {string} path  absolute MEMFS path
 * @param {number} size  the file's size from the manifest
 * @param {() => Uint8Array} load  the whole file; called once, on the first access to its bytes
 * @returns {object} the node
 */
export function createLazyFile(FS, path, size, load) {
  const dir = path.slice(0, path.lastIndexOf('/')) || '/';
  FS.mkdirTree(dir);
  const node = FS.create(path, 0o644);
  const materialize = () => {
    const bytes = load();
    if (!(bytes instanceof Uint8Array)) throw new Error(`lazy file ${path}: the loader returned no bytes`);
    if (bytes.length !== size) {
      throw new Error(`lazy file ${path}: ${bytes.length} bytes, the boot-set manifest says ${size}`);
    }
    Object.defineProperty(node, 'contents', { value: bytes, writable: true, configurable: true, enumerable: true });
    Object.defineProperty(node, 'usedBytes', { value: bytes.length, writable: true, configurable: true, enumerable: true });
    return bytes;
  };
  Object.defineProperty(node, 'usedBytes', {
    configurable: true, enumerable: true,
    get: () => size,
    set(v) { materialize(); node.usedBytes = v; },
  });
  Object.defineProperty(node, 'contents', {
    configurable: true, enumerable: true,
    get: () => materialize(),
    set(v) { materialize(); node.contents = v; },
  });
  return node;
}

/**
 * A loader for a Worker: one synchronous XHR per file, relative to `base`
 * (the static assets' `target/wine`).
 * @param {string|URL} base
 * @returns {(source: string) => Uint8Array}
 */
export function xhrReader(base) {
  return (source) => {
    const url = new URL(source, base).href;
    const xhr = new XMLHttpRequest();
    xhr.open('GET', url, false);
    xhr.responseType = 'arraybuffer';
    xhr.send(null);
    if (xhr.status !== 200) throw new Error(`${url}: HTTP ${xhr.status}`);
    return new Uint8Array(xhr.response);
  };
}
