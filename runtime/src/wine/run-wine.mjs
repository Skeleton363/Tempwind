#!/usr/bin/env node
// Runs a Windows program on Wine in Node (docs/design/wine-unix.md, section 12):
//
//   node runtime/src/wine/run-wine.mjs [--dir HOST] [--cwd C:\DIR] [--env NAME=VALUE]... [--stats] FILE.exe [ARGS...]
//
// --dir HOST loads the host directory HOST into C:\work, which becomes the cwd
// (unless --cwd says otherwise), and writes C:\work back to HOST when the run
// settles, whatever its outcome (host-dir.mjs; Phase 1's rules). Invalid or
// case-colliding host names fail at start with exit status 2.
//
// The program is written to C:\tempwind\<name>.exe and runs in a fresh Wine
// process with both memory checks on. Its stdout and stderr are this
// process's (pipes to Windows: section 8, "stdio, as FIFOs"). The exit status
// is native Wine's for the Windows status (unixExitCode: a nonzero status
// with a zero low byte gives 1, otherwise its low 8 bits); --stats prints
// the full status, the AOT links, the fallbacks and the bytes the lazy nodes
// loaded on stderr. Inputs: TW_WINE_DIR (target/wine, prepared by
// tests/wine/prepare.sh), TW_WINE_UNIX_DIR (target/wine-unix), and the
// run-time translator and floating-point helper (TW_RT_WASM, TW_FP_WASM;
// default target/wasm32-unknown-unknown/release).
import { readFileSync } from 'node:fs';
import { basename } from 'node:path';
import { loadWineInputs } from './node-files.mjs';
import { loadHostDir, writeBackHostDir } from './host-dir.mjs';
import { createWineProcess, unixExitCode } from './wine-process.mjs';

const usage = 'usage: run-wine.mjs [--dir HOST] [--cwd C:\\DIR] [--env NAME=VALUE]... [--stats] FILE.exe [ARGS...]';
const argv = process.argv.slice(2);
const env = {};
let cwd, hostDir, stats = false;
while (argv.length && argv[0].startsWith('--')) {
  const a = argv.shift();
  if (a === '--cwd') cwd = argv.shift();
  else if (a === '--dir') hostDir = argv.shift();
  else if (a === '--stats') stats = true;
  else if (a === '--env') {
    const kv = argv.shift() ?? '';
    const i = kv.indexOf('=');
    if (i <= 0) { console.error(usage); process.exit(2); }
    env[kv.slice(0, i)] = kv.slice(i + 1);
  } else if (a === '--') break;
  else { console.error(usage); process.exit(2); }
}
const file = argv.shift();
if (!file) { console.error(usage); process.exit(2); }

const WORK = 'work'; // C:\work in drive_c
let files, hostTree;
if (hostDir !== undefined) {
  try { hostTree = loadHostDir(hostDir, WORK); } catch (e) {
    console.error(`run-wine: cannot load --dir ${hostDir}: ${e.message}`);
    process.exit(2);
  }
  files = hostTree;
  cwd ??= 'C:\\work';
}
const inputs = await loadWineInputs();
const proc = await createWineProcess({
  ...inputs,
  program: { name: basename(file), bytes: new Uint8Array(readFileSync(file)) },
  args: argv, cwd, env, files,
  stdout: (b) => process.stdout.write(b),
  stderr: (b) => process.stderr.write(b),
});
let status;
try {
  status = await proc.run();
} catch (e) {
  console.error(`run-wine: the process stopped: ${e.message}`);
  process.exitCode = 1;
}
if (hostDir !== undefined) {
  // Whatever the outcome: C:\work goes back to HOST.
  try {
    const st = proc.host.FS.stat(`${proc.driveC}/${WORK}`);
    writeBackHostDir(hostDir, WORK, hostTree, proc.collect(WORK), new Date(st.mtime).getTime());
  } catch (e) {
    console.error(`run-wine: cannot write C:\\work back to ${hostDir}: ${e.message}`);
    process.exitCode = 1;
  }
}
if (stats) {
  const s = proc.stats;
  const hex = (v) => '0x' + BigInt(v).toString(16);
  console.error(`run-wine: status ${status === undefined ? '(none)' : hex(status)}, boot ${s.bootMs.toFixed(0)} ms`);
  console.error(`run-wine: AOT links ${s.aot.length}: ${s.aot.map((a) => basename(a.path)).join(', ')}`);
  console.error(`run-wine: fallbacks: ${s.imageFallbacks.length} images, ${s.translations.length} run-time translations`);
  for (const f of s.imageFallbacks) console.error(`run-wine:   ${f.path} at ${hex(f.base)}: ${f.reason}`);
  console.error(`run-wine: lazy nodes loaded ${s.lazy.files} files, ${s.lazy.bytes} bytes`);
}
if (status !== undefined) process.exitCode = unixExitCode(status);
