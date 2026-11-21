// The JS host of a Wine process (docs/design/wine-unix.md, sections 8 and
// 12; abi.md v5, "Host entries of the Unix side"):
// - stdio as FIFOs: fstat reports S_IFIFO, not a tty;
// - a lazy boot-set node: stat before the first read, the whole file on it;
// - the catalog: a hit links the AOT module and sets PAGE_CODE, a miss is a
//   counted fallback whose code tw_translate translates at run time;
//   tw_invalidate removes it again;
// - the exit code run-wine.mjs gives a Windows status, as native Wine does.
// The catalog checks boot wine-unix to the entry (BOOT_STOP_AT_ENTRY), where
// the program and ntdll are mapped. Needs target/wine-unix (TW_WINE_UNIX_DIR),
// target/wine prepared by tests/wine/prepare.sh (TW_WINE_DIR), and the
// run-time translator and floating-point helper (cargo build -p x64wasm-rt
// -p x64wasm-fp --release --target wasm32-unknown-unknown, or TW_RT_WASM and
// TW_FP_WASM).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { STATE_RIP, PAGE_CODE } from '../../runtime/src/abi.mjs';
import { createUnixHost, S_IFMT, S_IFIFO } from '../../runtime/src/wine/unix-host.mjs';
import { installBootSet, parseBootSetManifest, BOOT_STOP_AT_ENTRY, PE_DIR } from '../../runtime/src/wine/boot-set.mjs';
import { parseCatalog, matchCatalog } from '../../runtime/src/wine/aot-catalog.mjs';
import { loadWineInputs, wineDirs } from '../../runtime/src/wine/node-files.mjs';
import { createWineProcess, unixExitCode } from '../../runtime/src/wine/wine-process.mjs';

const dirs = wineDirs();
const inputs = await loadWineInputs(dirs);
const HELLO = { name: 'hello.exe', bytes: new Uint8Array(readFileSync(join(dirs.wine, 'hello.exe'))) };
const NTDLL = '/wine/lib/wine/x86_64-windows/ntdll.dll';
const HELLO_PATH = '/home/runner/wine-prefix/drive_c/tempwind/hello.exe';

/** A fresh Unix-side instance of test-link, the smallest module, with stdout and stderr as bytes. */
async function smallHost() {
  const { default: factory } = await import(join(dirs.unix, 'test-link.mjs'));
  const wasm = new WebAssembly.Module(readFileSync(join(dirs.unix, 'test-link.wasm')));
  const out = [];
  const host = await createUnixHost({ factory, wasm, stdout: (b) => out.push(...b) });
  return { host, out };
}

test('stdio: fds 0-2 are FIFOs, not ttys, and writes still reach the host', async () => {
  const { host, out } = await smallHost();
  for (const fd of [0, 1, 2]) {
    const stream = host.FS.getStream(fd);
    assert.equal(host.FS.fstat(fd).mode & S_IFMT, S_IFIFO, `fd ${fd}`);
    assert.ok(!stream.tty, `fd ${fd} is a tty`); // isatty: ioctl TCGETS fails with ENOTTY without stream.tty
    assert.throws(() => host.FS.llseek(stream, 0, 0), (e) => e.errno === 70, `fd ${fd} seeks`); // ESPIPE
  }
  host.FS.write(host.FS.getStream(1), new TextEncoder().encode('pipe\n'), 0, 5);
  assert.equal(new TextDecoder().decode(new Uint8Array(out)), 'pipe\n');
});

test('lazy node: stat and seek give the manifest\'s size before the first read, which loads the whole file once', async () => {
  const { host } = await smallHost();
  const bytes = new TextEncoder().encode('lazy file contents\n');
  const reads = [];
  const loaded = installBootSet(host.FS, {
    prefixPath: '/p', prefixFiles: {}, prefixDirs: [], prefixLinks: [], programs: {}, overlay: new Uint8Array(0),
    lazy: [{ path: '/wine/x/a.dll', size: bytes.length, source: 'pe/a.dll' }, { path: '/wine/x/bad', size: 3, source: 'bad' }],
    read: (source) => { reads.push(source); return source === 'bad' ? new Uint8Array(4) : bytes; },
  });
  const st = host.FS.stat('/wine/x/a.dll');
  assert.equal(st.size, bytes.length);
  assert.equal(st.mode & S_IFMT, 0o100000); // a regular file
  const stream = host.FS.open('/wine/x/a.dll', 0);
  assert.equal(host.FS.llseek(stream, 0, 2), bytes.length); // SEEK_END
  assert.deepEqual(reads, []);
  assert.deepEqual(loaded, { files: 0, bytes: 0 });
  const buf = new Uint8Array(4);
  assert.equal(host.FS.read(stream, buf, 0, 4, 5), 4);
  assert.equal(new TextDecoder().decode(buf), 'file');
  host.FS.close(stream);
  assert.deepEqual(host.FS.readFile('/wine/x/a.dll'), bytes);
  assert.deepEqual(reads, ['pe/a.dll']);
  assert.deepEqual(loaded, { files: 1, bytes: bytes.length });
  // A file whose bytes don't match the manifest's size stops loudly.
  assert.throws(() => host.FS.readFile('/wine/x/bad'), /4 bytes, the boot-set manifest says 3/);
});

test('install copies: drive_c\'s copies of closure PE files are symlinks to the PE dir\'s lazy nodes', async () => {
  const { host } = await smallHost();
  const reads = [];
  const prefixPath = inputs.prefix.path.replace(/\/+$/, '');
  const lazy = parseBootSetManifest(inputs.manifest).filter((f) => f.lazy);
  installBootSet(host.FS, {
    prefixPath, prefixFiles: inputs.prefix.files, prefixDirs: inputs.prefix.dirs, prefixLinks: inputs.prefix.links,
    installFiles: inputs.prefix.installFiles, programs: {}, overlay: new Uint8Array(0),
    lazy, read: (source) => { reads.push(source); return inputs.read(source); },
  });
  // Where the links are (wine-unix.md, section 8, "the install copies"): the
  // counts of wine-4c6ec0d's prefix and closure.
  const drive = `${prefixPath}/drive_c/`;
  const byDir = {};
  const walk = (dir) => {
    for (const name of host.FS.readdir(dir)) {
      if (name === '.' || name === '..') continue;
      const p = `${dir}/${name}`;
      const st = host.FS.lstat(p);
      if (host.FS.isLink(st.mode)) {
        const target = host.FS.readlink(p);
        assert.equal(target, `${PE_DIR}/${name}`, p);
        const rel = dir.slice(drive.length);
        const key = rel.startsWith('windows/winsxs') ? 'windows/winsxs' : rel;
        byDir[key] = (byDir[key] ?? 0) + 1;
      } else if (host.FS.isDir(st.mode)) walk(p);
    }
  };
  walk(drive.slice(0, -1));
  assert.deepEqual(byDir, { 'windows/system32': 66, 'windows/system32/drivers': 8, windows: 1, 'windows/winsxs': 3 });
  // kernel32.dll's link reaches the PE dir's node: a stat before its first read, and the path an fd gets.
  const k32 = `${drive}windows/system32/kernel32.dll`;
  const node = lazy.find((f) => f.path === `${PE_DIR}/kernel32.dll`);
  assert.equal(host.FS.stat(k32).size, node.size);
  const stream = host.FS.open(k32, 0);
  assert.equal(stream.path, node.path); // tw_image_mapped's catalog path
  host.FS.close(stream);
  assert.deepEqual(reads, []);
});

test('catalog: a module matches by path, sha256 and base only', () => {
  const sha = 'a'.repeat(64);
  const cat = parseCatalog({ schema: 1, options: ['--shared', '--null-check', '--store-check'],
    modules: [{ path: '/wine/a.dll', sha256: sha, base: '0x180000000', wasm: 'a.dll.wasm', plan: 'a.dll.plan.json' }] });
  assert.equal(matchCatalog(cat, '/wine/a.dll', sha, 0x1_8000_0000n).entry.wasm, 'a.dll.wasm');
  assert.match(matchCatalog(cat, '/wine/b.dll', sha, 0x1_8000_0000n).reason, /not in the catalog/);
  assert.match(matchCatalog(cat, '/wine/a.dll', 'b'.repeat(64), 0x1_8000_0000n).reason, /sha256/);
  assert.match(matchCatalog(cat, '/wine/a.dll', null, 0x1_8000_0000n).reason, /sha256 is unknown/);
  assert.match(matchCatalog(cat, '/wine/a.dll', sha, 0x1_7000_0000n).reason, /mapped at 0x170000000, translated at 0x180000000/);
  assert.throws(() => parseCatalog({ schema: 1, options: ['--shared'], modules: [] }), /--null-check/);
});

/** A wine-unix process booted to the entry with `catalog`; its stderr as text. */
async function bootToEntry(catalog) {
  let stderr = '';
  const dec = new TextDecoder();
  const proc = await createWineProcess({ ...inputs, catalog, program: HELLO, stderr: (b) => { stderr += dec.decode(b, { stream: true }); } });
  assert.equal(await proc.run(BOOT_STOP_AT_ENTRY), 0, stderr);
  return { proc, stderr: () => stderr };
}

const pageCode = (proc, addr) => new Uint8Array(proc.memory.buffer)[Number(proc.linker.pageFlagsAddr + (addr >> 12n))] & PAGE_CODE;

test('catalog hit: the program and ntdll, mapped by the boot, are linked from the catalog with PAGE_CODE on their code', async () => {
  const { proc } = await bootToEntry(inputs.catalog);
  const { stats } = proc;
  assert.deepEqual(stats.aot.map((a) => a.path), [HELLO_PATH, NTDLL]);
  assert.deepEqual(stats.imageFallbacks, []);
  assert.deepEqual(stats.translations, []);
  const rip = new DataView(proc.memory.buffer).getBigUint64(Number(proc.host.raw.tw_main_state()) + STATE_RIP, true);
  assert.notEqual(proc.linker.lookup(rip), null, 'LdrInitializeThunk has a block');
  assert.equal(pageCode(proc, rip), PAGE_CODE);
  assert.equal(pageCode(proc, 0x1_7000_0000n), 0, 'ntdll\'s headers are not code');
  assert.ok(stats.lazy.files >= 2 && stats.lazy.bytes > 0, 'the lazy nodes loaded ntdll and the NLS files');
});

test('catalog fallback: an image whose base differs is counted, tw_translate translates its code at run time, and tw_invalidate removes it', async () => {
  const json = structuredClone(inputs.catalog);
  const nt = json.modules.find((m) => m.path === NTDLL);
  nt.base = '0x' + (BigInt(nt.base) + 0x1_0000n).toString(16);
  const { proc, stderr } = await bootToEntry(json);
  const { stats, linker } = proc;
  assert.deepEqual(stats.aot.map((a) => a.path), [HELLO_PATH]);
  assert.equal(stats.imageFallbacks.length, 1);
  assert.equal(stats.imageFallbacks[0].path, NTDLL);
  assert.match(stats.imageFallbacks[0].reason, /mapped at 0x170000000, translated at 0x170010000/);
  const rip = new DataView(proc.memory.buffer).getBigUint64(Number(proc.host.raw.tw_main_state()) + STATE_RIP, true);
  assert.equal(linker.lookup(rip), null);

  // The run loop's EXIT_LEFT_CODE: ntdll's .text is the executable view.
  const plan = JSON.parse(readFileSync(join(dirs.wine, 'catalog', nt.plan), 'utf8'));
  const text = plan.sections.find((s) => s.name === '.text');
  const base = BigInt(text.va), size = (BigInt(text.virtualSize) + 0xfffn) & ~0xfffn;
  assert.equal(proc.imports.tw_translate(rip, base, size), 0, stderr());
  assert.equal(stats.translations.length, 1);
  assert.notEqual(linker.lookup(rip), null);
  assert.equal(pageCode(proc, base), PAGE_CODE);
  // A rip outside any code it can translate fails with the loud line, not an exception.
  assert.equal(proc.imports.tw_translate(rip, rip, 1n), 1);
  assert.match(stderr(), /tempwind: translate: 0x170[0-9a-f]+ /);

  proc.imports.tw_invalidate(base + 0x10n, 1n);
  assert.equal(stats.invalidated, 1);
  assert.equal(linker.lookup(rip), null);
  assert.equal(pageCode(proc, base), 0);
  assert.equal(pageCode(proc, size - 1n + base), 0);
  // The program's module is elsewhere and stays.
  assert.equal(pageCode(proc, 0x1_4000_1000n), PAGE_CODE);
});

// The exit code of native Wine's process for a Windows status:
// get_unix_exit_code (dlls/ntdll/unix/thread.c L89-94) turns a nonzero status
// with a zero low byte into 1, and _exit keeps the low 8 bits.
test('unixExitCode: a Windows status gives native Wine\'s exit code', () => {
  for (const [status, code] of [[0, 0], [0x25, 0x25], [0x100, 1], [0xc0000135, 0x35], [0xc0000100, 1]]) {
    assert.equal(unixExitCode(status), code, `status 0x${status.toString(16)}`);
  }
});
