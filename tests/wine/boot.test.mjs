// The `boot` checks (docs/design/wine-unix.md, section 10): the Unix side
// boots to LdrInitializeThunk without running guest code. The harness is
// native/wine/tests/boot.c; the boot set comes from
// runtime/src/wine/boot-set.mjs.
// Needs native/wine/build.sh's output in target/wine-unix (or TW_WINE_UNIX_DIR)
// and tests/wine/prepare.sh's target/wine (or TW_WINE_DIR): the PE tree, the
// install tree's data files, the prefix, hello.exe and kuser.bin.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, readlinkSync, lstatSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { STATE_RIP, STATE_GS_BASE } from '../../runtime/src/abi.mjs';
import { createUnixHost, DEFAULT_ENV } from '../../runtime/src/wine/unix-host.mjs';
import { installBootSet, recordOpens, boot, BOOT_DLLS, OVERLAY_PATH, PE_DIR } from '../../runtime/src/wine/boot-set.mjs';

const root = new URL('../../', import.meta.url);
const dirUrl = (env, fallback) => (process.env[env]
  ? pathToFileURL(process.env[env].replace(/\/?$/, '/'))
  : new URL(fallback, root));
const unixDir = dirUrl('TW_WINE_UNIX_DIR', 'target/wine-unix/');
const wineDir = dirUrl('TW_WINE_DIR', 'target/wine/');
const read = (rel) => readFileSync(new URL(rel, wineDir));

const PROGRAM = 'C:\\tempwind\\hello.exe';
/** Where the harness reads the KUSER oracle. */
const KUSER_PATH = '/tempwind-test/kuser.bin';

/** The boot set from target/wine (section 8). */
function bootSet() {
  const prefixDir = fileURLToPath(new URL('wine-prefix/', wineDir));
  const prefixFiles = {}, prefixDirs = [], prefixLinks = [];
  const walk = (rel) => {
    for (const name of readdirSync(prefixDir + rel)) {
      const path = rel ? `${rel}/${name}` : name;
      const st = lstatSync(prefixDir + path);
      if (st.isSymbolicLink()) prefixLinks.push([path, readlinkSync(prefixDir + path)]);
      else if (st.isDirectory()) { prefixDirs.push(path); walk(path); }
      else if (!rel) prefixFiles[name] = readFileSync(prefixDir + path);
    }
  };
  walk('');
  const share = 'native/opt/wine-11.0/share/wine/';
  const data = { 'wine.inf': read(`${share}wine.inf`) };
  for (const name of readdirSync(new URL(`${share}nls/`, wineDir))) data[`nls/${name}`] = read(`${share}nls/${name}`);
  const dlls = {};
  for (const name of BOOT_DLLS) dlls[name] = read(`pe/lib/wine/x86_64-windows/${name}`);
  return {
    prefixPath: DEFAULT_ENV.WINEPREFIX, prefixFiles, prefixDirs, prefixLinks, dlls, data,
    programs: { [PROGRAM]: read('hello.exe') },
    overlay: readFileSync(new URL('native/wine/prefix/overlay.reg', root)),
  };
}

const SET = bootSet();

/** A fresh instance of NAME (test-boot or wine-unix) with the boot set in MEMFS. */
async function start(name) {
  const { default: factory } = await import(new URL(`${name}.mjs`, unixDir));
  const wasm = new WebAssembly.Module(readFileSync(new URL(`${name}.wasm`, unixDir)));
  const out = { stdout: '', stderr: '' };
  const dec = () => { const d = new TextDecoder(); return (b) => d.decode(b, { stream: true }); };
  const o = dec(), e = dec();
  const host = await createUnixHost({
    factory, wasm,
    stdout: (b) => { out.stdout += o(b); },
    stderr: (b) => { out.stderr += e(b); },
  });
  installBootSet(host.FS, SET);
  host.FS.mkdirTree(KUSER_PATH.slice(0, KUSER_PATH.lastIndexOf('/')));
  host.FS.writeFile(KUSER_PATH, read('kuser.bin'));
  // The launch directory: the program's own (section 8).
  host.FS.chdir(`${DEFAULT_ENV.WINEPREFIX}/drive_c/tempwind`);
  return { host, out };
}

const CHECKS = ['rip_ldr_initialize_thunk', 'context_and_state', 'images_at_preferred_bases', 'apiset_map',
  'registry_values', 'nls_loaded', 'kuser'];

// The files of the boot set that the boot opens, in the order of their first
// open (section 10): NLS (init_environment, then the server's
// init_directories), the prefix's registry and the overlay (init_registry),
// the program (load_main_exe), ntdll and the API set schema (load_ntdll,
// load_apiset_dll). Besides these the boot opens only the prefix directory
// and the server's own directory and temporary mapping files under /tmp.
const P = DEFAULT_ENV.WINEPREFIX;
const BOOT_OPENS = [
  '/wine/share/wine/nls/locale.nls',
  '/wine/share/wine/nls/l_intl.nls',
  `${P}/system.reg`,
  `${P}/userdef.reg`,
  `${P}/user.reg`,
  OVERLAY_PATH,
  `${P}/drive_c/tempwind/hello.exe`,
  `${PE_DIR}/ntdll.dll`,
  `${PE_DIR}/apisetschema.dll`,
];
const SERVER_FILE = /^\/tmp\/\.wine-0\/server-[0-9a-f]+-[0-9a-f]+(\/tmpmap-[0-9a-f]+)?$/;

test('boot: the harness checks pass, and the boot opens only its boot set', async () => {
  const { host, out } = await start('test-boot');
  const opens = recordOpens(host.FS);
  const ret = await host.call('tw_test_main');
  opens.stop();
  const lines = out.stdout.trim().split('\n');
  assert.deepEqual(lines.filter((l) => l.startsWith('not ok')), [], out.stdout + out.stderr);
  assert.deepEqual(lines.filter((l) => l.startsWith('ok ')).map((l) => l.slice(3)), CHECKS);
  assert.equal(lines.at(-1), `done ${CHECKS.length}/${CHECKS.length}`);
  assert.equal(ret, 0);

  const opened = [...new Set(opens.paths)].filter((p) => p !== KUSER_PATH); // the harness's oracle
  assert.deepEqual(opened.filter((p) => BOOT_OPENS.includes(p)), BOOT_OPENS);
  const others = opened.filter((p) => !BOOT_OPENS.includes(p) && p !== P && !SERVER_FILE.test(p));
  assert.deepEqual(others, [], `opened outside the boot set: ${others.join(', ')}`);
});

// The real module, entered as the JS host enters it (section 8): argv from
// the raw malloc, boot through promising, the state block from tw_main_state.
test('boot: wine-unix stops at LdrInitializeThunk (measured)', async (t) => {
  const { host, out } = await start('wine-unix');
  const t0 = performance.now();
  const ret = await boot(host, [PROGRAM]);
  const ms = performance.now() - t0;
  assert.equal(ret, 0, out.stderr);
  const mem = () => new DataView(host.memory.buffer);
  const state = Number(host.raw.tw_main_state());
  // LdrInitializeThunk from ntdll's export table, at ntdll's preferred base
  // (the PE optional header's ImageBase).
  const dll = SET.dlls['ntdll.dll'];
  const file = new DataView(dll.buffer, dll.byteOffset, dll.byteLength);
  const base = Number(file.getBigUint64(file.getUint32(0x3c, true) + 0x18 + 0x18, true));
  assert.equal(mem().getBigUint64(state + STATE_RIP, true), BigInt(base + exportRva(mem(), base, 'LdrInitializeThunk')));
  assert.notEqual(mem().getBigUint64(state + STATE_GS_BASE, true), 0n); // GS base: the TEB
  const pages = host.memory.buffer.byteLength / 65536;
  t.diagnostic(`boot to LdrInitializeThunk: ${ms.toFixed(1)} ms; memory after boot: ${pages} pages of 64 KiB`);
});

/** A named export's RVA, read from an image in memory at base. */
function exportRva(mem, base, name) {
  const nt = base + mem.getUint32(base + 0x3c, true);
  const dir = base + mem.getUint32(nt + 0x18 + 0x70, true); // the export directory's RVA
  const count = mem.getUint32(dir + 0x18, true);
  const functions = base + mem.getUint32(dir + 0x1c, true);
  const names = base + mem.getUint32(dir + 0x20, true);
  const ordinals = base + mem.getUint32(dir + 0x24, true);
  for (let i = 0; i < count; i++) {
    let p = base + mem.getUint32(names + 4 * i, true), s = '';
    for (let c; (c = mem.getUint8(p)); p++) s += String.fromCharCode(c);
    if (s === name) return mem.getUint32(functions + 4 * mem.getUint16(ordinals + 2 * i, true), true);
  }
  throw new Error(`no export ${name}`);
}
