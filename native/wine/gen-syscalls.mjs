// The syscall wrapper generator (docs/design/wine-unix.md, section 11; gate 4
// of section 2).
//
//   node native/wine/gen-syscalls.mjs WINE_TREE OUT.c
//
// Reads the patched tree's dlls/ntdll/ntsyscalls.h (the x86-64 list,
// ALL_SYSCALLS under _WIN64) and include/winternl.h (the prototypes), and
// writes one wrapper per entry,
//     ULONG_PTR tw_sys_<name>( const ULONG_PTR *args )
// which calls the real function with each args[i] cast to its parameter's
// type, and the tables by number: tw_syscall_table, tw_syscall_args (Wine's
// argument bytes), tw_syscall_names and tw_syscall_count. Wine calls its own
// table through a pointer with raw register arguments; in wasm a call
// through a pointer with another signature traps, hence the wrappers.
//
// Checks (gate 4), each failure printed, then exit 1:
// - the ids run 0, 1, 2 ... in order, and each variant macro
//   (SYSCALL_ENTRY_<name>) is defined as a plain SYSCALL_ENTRY;
// - every entry other than a stub (ALL_SYSCALL_STUBS) has a prototype, and
//   its parameter count is argbytes / 8;
// - every parameter is a scalar or a pointer, except NtSetLdtEntries's two
//   LDT_ENTRY, passed by value: the wrapper copies each from its 8-byte
//   argument;
// - every return type is NTSTATUS, except NtGetCurrentProcessorNumber's
//   ULONG. Both are zero-extended into RAX.
// The six stubs' wrappers stop loudly (tempwind: unsupported: syscall NAME)
// until M5.
// The output is written only when it changes, so make doesn't rebuild it.
import { readFileSync, writeFileSync, readdirSync, existsSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';

const [tree, out] = process.argv.slice(2);
if (!tree || !out) {
  console.error('usage: node native/wine/gen-syscalls.mjs WINE_TREE OUT.c');
  process.exit(2);
}
const errors = [];
const fail = (msg) => errors.push(msg);
const stripComments = (s) => s.replace(/\/\*[\s\S]*?\*\//g, ' ').replace(/\/\/[^\n]*/g, ' ');

// ---- ntsyscalls.h ------------------------------------------------------------
const nsPath = join(tree, 'dlls/ntdll/ntsyscalls.h');
const ns = readFileSync(nsPath, 'utf8');
const block = /#ifdef _WIN64\s*\n#define ALL_SYSCALLS \\\n([\s\S]*?)\n#else/.exec(ns);
if (!block) {
  console.error(`gen-syscalls: ${nsPath}: no "#ifdef _WIN64 / #define ALL_SYSCALLS" block`);
  process.exit(1);
}
const entries = [];
for (const line of block[1].split('\n')) {
  const m = /^\s*SYSCALL_ENTRY(_\w+)?\(\s*(0x[0-9a-fA-F]+)\s*,\s*(\w+)\s*,\s*(\d+)\s*\)\s*\\?\s*$/.exec(line);
  if (!m) { fail(`ntsyscalls.h: cannot read the entry line: ${line.trim()}`); continue; }
  const [, variant, id, name, args] = m;
  if (variant) {
    const def = new RegExp(`#define SYSCALL_ENTRY${variant}\\(id,name,args\\) SYSCALL_ENTRY\\(id,name,args\\)`);
    if (variant !== `_${name}` || !def.test(ns)) fail(`ntsyscalls.h: ${name}: variant macro SYSCALL_ENTRY${variant} is not defined as a plain entry`);
  }
  if (Number(id) !== entries.length) fail(`ntsyscalls.h: ${name}: id ${id}, expected 0x${entries.length.toString(16)}`);
  entries.push({ id: Number(id), name, argbytes: Number(args) });
}
const stubBlock = /#define ALL_SYSCALL_STUBS \\\n((?:\s*SYSCALL_STUB\(\s*\w+\s*\)\s*\\?\n?)+)/.exec(ns);
if (!stubBlock) fail('ntsyscalls.h: no ALL_SYSCALL_STUBS');
const stubs = new Set(stubBlock ? [...stubBlock[1].matchAll(/SYSCALL_STUB\(\s*(\w+)\s*\)/g)].map((m) => m[1]) : []);
for (const s of stubs) if (!entries.some((e) => e.name === s)) fail(`ntsyscalls.h: stub ${s} has no entry`);

// ---- include/: prototypes and the types passed by value that are records ---
const incDir = join(tree, 'include');
const winternl = stripComments(readFileSync(join(incDir, 'winternl.h'), 'utf8'));
const protos = new Map();
for (const m of winternl.matchAll(/NTSYSAPI\s+([\w\s*]+?)\s+WINAPI\s+(Nt\w+)\s*\(/g)) {
  // the parameter list, with balanced parentheses
  let depth = 1, i = m.index + m[0].length, start = i;
  while (i < winternl.length && depth) { if (winternl[i] === '(') depth++; else if (winternl[i] === ')') depth--; i++; }
  const list = winternl.slice(start, i - 1);
  const params = [];
  let cur = '', d = 0;
  for (const ch of list) {
    if (ch === '(') d++;
    if (ch === ')') d--;
    if (ch === ',' && !d) { params.push(cur); cur = ''; } else cur += ch;
  }
  params.push(cur);
  const ps = params.map((p) => p.replace(/\s+/g, ' ').trim()).filter((p) => p && p !== 'void' && p !== 'VOID');
  protos.set(m[2], { ret: m[1].replace(/\s+/g, ' ').trim(), params: ps });
}

// Type names that denote a struct or union by value, from every typedef in
// include/*.h (and their plain aliases).
const records = new Set();
const aliases = [];
for (const f of readdirSync(incDir).filter((f) => f.endsWith('.h'))) {
  const src = stripComments(readFileSync(join(incDir, f), 'utf8')).replace(/^\s*#.*$/gm, ' ');
  for (const m of src.matchAll(/\btypedef\s+(?:struct|union)\b\s*\w*\s*/g)) {
    let i = m.index + m[0].length;
    if (src[i] === '{') {
      let depth = 0;
      for (; i < src.length; i++) { if (src[i] === '{') depth++; else if (src[i] === '}' && !--depth) { i++; break; } }
    }
    const end = src.indexOf(';', i);
    if (end < 0) continue;
    // the declarators, without alignment attributes; anything else with a
    // parenthesis is a function type or a macro, not a record by value
    const decls = src.slice(i, end).replace(/\bDECLSPEC_\w+\s*\([^()]*\)|__attribute__\s*\(\([^;]*?\)\)/g, ' ');
    if (decls.includes('(')) continue;
    for (const decl of decls.split(',')) {
      if (decl.includes('*')) continue;
      const ids = decl.replace(/\[[^\]]*\]/g, '').match(/[A-Za-z_]\w*/g);
      if (ids) records.add(ids.at(-1));
    }
  }
  for (const m of src.matchAll(/\btypedef\s+(?:const\s+)?(\w+)\s+([^;{}()]+);/g)) aliases.push([m[1], m[2]]);
}
for (let grew = true; grew;) {
  grew = false;
  for (const [from, decls] of aliases) {
    if (!records.has(from)) continue;
    for (const decl of decls.split(',')) {
      if (decl.includes('*')) continue;
      const ids = decl.replace(/\[[^\]]*\]/g, '').match(/[A-Za-z_]\w*/g);
      const name = ids?.at(-1);
      if (name && !records.has(name)) { records.add(name); grew = true; }
    }
  }
}
const isRecord = (type) => {
  if (type.includes('*')) return false;
  if (/\b(struct|union)\b/.test(type)) return true;
  return type.split(' ').some((w) => records.has(w));
};

// ---- the checks and the wrappers -----------------------------------------------
const BY_VALUE = { NtSetLdtEntries: 'LDT_ENTRY' };   // winternl.h L4797
const RETURNS = { NtGetCurrentProcessorNumber: 'ULONG' };
const wrappers = [];
for (const e of entries) {
  if (stubs.has(e.name)) {
    wrappers.push(`ULONG_PTR tw_sys_${e.name}( const ULONG_PTR *args )\n{\n    tw_unsupported( "syscall ${e.name}" );\n}\n`);
    continue;
  }
  const p = protos.get(e.name);
  if (!p) { fail(`${e.name}: no prototype in include/winternl.h`); continue; }
  if (e.argbytes % 8 || p.params.length !== e.argbytes / 8)
    fail(`${e.name}: ${p.params.length} parameters in winternl.h, but ${e.argbytes} argument bytes in ntsyscalls.h`);
  const ret = RETURNS[e.name] ?? 'NTSTATUS';
  if (p.ret !== ret) fail(`${e.name}: returns ${p.ret}, expected ${ret}`);
  const pre = [], args = [];
  p.params.forEach((type, i) => {
    if (isRecord(type)) {
      if (BY_VALUE[e.name] !== type) { fail(`${e.name}: parameter ${i} (${type}) is a struct or union passed by value`); return; }
      pre.push(`    ${type} a${i};\n    C_ASSERT( sizeof(a${i}) == sizeof(ULONG_PTR) );\n    memcpy( &a${i}, &args[${i}], sizeof(a${i}) );\n`);
      args.push(`a${i}`);
    } else {
      args.push(`(${type})args[${i}]`);
    }
  });
  wrappers.push(`ULONG_PTR tw_sys_${e.name}( const ULONG_PTR *args )\n{\n${pre.join('')}` +
    `    return (${ret === 'NTSTATUS' ? 'ULONG' : ret})${e.name}( ${args.join(', ')} );\n}\n`);
}
for (const name of Object.keys(BY_VALUE)) if (!entries.some((e) => e.name === name)) fail(`${name}: expected in ntsyscalls.h`);

if (errors.length) {
  for (const e of errors) console.error(`gate 4 (syscalls): ${e}`);
  console.error(`gen-syscalls: ${errors.length} entries could not be matched (wine-unix.md, section 11)`);
  process.exit(1);
}

const c = `/* Generated by native/wine/gen-syscalls.mjs from wine's dlls/ntdll/ntsyscalls.h
 * and include/winternl.h (docs/design/wine-unix.md, section 11); do not edit.
 * One wrapper per x86-64 syscall, called with the arguments the dispatcher
 * gathered (native/wine/src/signal_wasm64.c). */
#include <string.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winternl.h"

typedef ULONG_PTR (*tw_syscall_fn)( const ULONG_PTR *args );
_Noreturn void tw_unsupported( const char *what );

${wrappers.join('\n')}
const tw_syscall_fn tw_syscall_table[] =
{
${entries.map((e) => `    tw_sys_${e.name},`).join('\n')}
};

const BYTE tw_syscall_args[] =
{
${entries.map((e) => `    ${e.argbytes},  /* ${e.name} */`).join('\n')}
};

const char *const tw_syscall_names[] =
{
${entries.map((e) => `    "${e.name}",`).join('\n')}
};

const unsigned int tw_syscall_count = ${entries.length};
`;
mkdirSync(dirname(out), { recursive: true });
if (!existsSync(out) || readFileSync(out, 'utf8') !== c) writeFileSync(out, c);
console.log(`gen-syscalls: ${entries.length} syscalls (${stubs.size} stubs) -> ${out}`);
