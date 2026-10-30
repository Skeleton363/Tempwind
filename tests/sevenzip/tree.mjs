#!/usr/bin/env node
// Input tree of the 7-Zip command matrix (tests/sevenzip/README.md, "Input tree"):
//   node tests/sevenzip/tree.mjs DIR
// Writes DIR (created if absent) with ASCII names, contents from a seeded PRNG or fixed text, and fixed
// mtimes on even seconds. The same bytes and times on every OS. Also exports the helpers matrix.mjs uses.
import { createHash } from 'node:crypto';
import { mkdirSync, readdirSync, readFileSync, statSync, utimesSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** mulberry32: a 32-bit seeded PRNG; next() returns an integer in [0, 2^32). */
function prng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return (t ^ (t >>> 14)) >>> 0;
  };
}

function randomBytes(n, seed) {
  const next = prng(seed);
  const b = Buffer.alloc(n);
  for (let i = 0; i < n; i++) b[i] = next() >>> 24;
  return b;
}

const WORDS = ['alpha', 'bravo', 'charlie', 'delta', 'echo', 'foxtrot', 'golf', 'hotel', 'india', 'juliet', 'kilo', 'lima',
  'mike', 'november', 'oscar', 'papa', 'quebec', 'romeo', 'sierra', 'tango', 'uniform', 'victor', 'whiskey', 'xray', 'yankee', 'zulu'];

/** Prose-like text of about `size` bytes, LF line ends. */
function text(size, seed) {
  const next = prng(seed);
  let out = '';
  while (out.length < size) {
    const n = 6 + (next() % 8);
    const line = [];
    for (let i = 0; i < n; i++) line.push(WORDS[next() % WORDS.length]);
    out += line.join(' ') + '.\n';
  }
  return Buffer.from(out.slice(0, size - 1) + '\n', 'latin1');
}

/** x86-64 code bytes (typical prologues, moves, calls with rel32 operands), for the BCJ filter. */
function code(size, seed) {
  const next = prng(seed);
  const out = [];
  const push = (...bs) => out.push(...bs);
  const rel32 = () => { const v = (next() % 0x20000) - 0x10000; push(v & 255, (v >> 8) & 255, (v >> 16) & 255, (v >> 24) & 255); };
  while (out.length < size) {
    switch (next() % 9) {
      case 0: push(0x55, 0x48, 0x89, 0xe5); break; // push rbp; mov rbp, rsp
      case 1: push(0xe8); rel32(); break; // call rel32
      case 2: push(0xe9); rel32(); break; // jmp rel32
      case 3: push(0x48, 0x8b, 0x45, next() & 0x7f); break; // mov rax, [rbp+d8]
      case 4: push(0x48, 0x89, 0x45, next() & 0x7f); break; // mov [rbp+d8], rax
      case 5: push(0x48, 0x83, 0xec, (next() & 0x3f) * 8); break; // sub rsp, imm8
      case 6: push(0x48, 0x8d, 0x05); rel32(); break; // lea rax, [rip+rel32]
      case 7: push(0x0f, 0x1f, 0x44, 0x00, 0x00); break; // nop dword [rax+rax]
      default: push(0x5d, 0xc3); break; // pop rbp; ret
    }
  }
  return Buffer.from(out.slice(0, size));
}

function numbers(count) {
  let s = '';
  for (let i = 1; i <= count; i++) s += i + '\n';
  return Buffer.from(s, 'latin1');
}

const BASE = Date.UTC(2024, 0, 2, 3, 4, 6) / 1000; // 2024-01-02 03:04:06 UTC, an even second

/** [path, content] of every file; paths use '/'. Directories are implied. */
export function files() {
  return [
    ['hello.txt', Buffer.from('hello\nworld\n', 'latin1')],
    ['lorem.txt', text(60000, 1)],
    ['numbers.txt', numbers(20000)],
    ['random.bin', randomBytes(65536, 2)],
    ['empty', Buffer.alloc(0)],
    ['code.bin', code(49152, 3)],
    ['sub/other.txt', Buffer.from('second\n', 'latin1')],
    ['sub/deeper/nested.txt', Buffer.from('nested file\n', 'latin1')],
    ['sub/deeper/more.txt', text(4000, 4)],
  ];
}

/** Writes the tree into dir. */
export function writeTree(dir) {
  const dirs = new Set(['']);
  const list = files();
  list.forEach(([path, data], i) => {
    const full = join(dir, path);
    const parts = path.split('/');
    for (let k = 1; k < parts.length; k++) dirs.add(parts.slice(0, k).join('/'));
    mkdirSync(join(dir, ...parts.slice(0, -1)), { recursive: true });
    writeFileSync(full, data);
    const t = BASE + i * 7200; // every file a different even second
    utimesSync(full, t, t);
  });
  // Directories last (writing a file changes its directory's mtime), deepest first.
  [...dirs].sort((a, b) => b.split('/').length - a.split('/').length).forEach((d, i) => {
    const t = BASE + 100000 + i * 7200;
    utimesSync(join(dir, d), t, t);
  });
}

/**
 * Manifest of a directory: one line per entry, sorted by path ('/' separators): path, size, SHA-256 and mtime
 * (UTC, millisecond precision; "-" without `mtime`), tab-separated. Directories end in '/', with size 0 and "-" for the hash.
 */
export function manifest(dir, { mtime: withMtime = true } = {}) {
  const lines = [];
  const walk = (rel) => {
    const full = rel ? join(dir, rel) : dir;
    for (const name of readdirSync(full)) {
      const r = rel ? `${rel}/${name}` : name;
      const st = statSync(join(dir, r), { bigint: true });
      const mtime = withMtime ? new Date(Number(st.mtimeNs / 1000000n)).toISOString() : '-';
      if (st.isDirectory()) {
        lines.push([`${r}/`, 0, '-', mtime]);
        walk(r);
      } else {
        lines.push([r, st.size, createHash('sha256').update(readFileSync(join(dir, r))).digest('hex'), mtime]);
      }
    }
  };
  walk('');
  lines.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
  return lines.map((l) => l.join('\t')).join('\n') + '\n';
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  if (process.argv.length !== 3) { console.error('usage: node tests/sevenzip/tree.mjs DIR'); process.exit(2); }
  writeTree(process.argv[2]);
  process.stdout.write(manifest(process.argv[2]));
}
