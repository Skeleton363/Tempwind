// The syscall wrapper generator (docs/design/wine-unix.md, section 11; gate 4
// of section 2).
//
//   node native/wine/gen-syscalls.mjs WINE_TREE OUT.c
//
// Table 0, ntdll's: reads the patched tree's dlls/ntdll/ntsyscalls.h (the
// x86-64 list, ALL_SYSCALLS under _WIN64) and include/winternl.h (the
// prototypes), and writes one wrapper per entry,
//     ULONG_PTR tw_sys_<name>( const ULONG_PTR *args )
// which calls the real function with each args[i] cast to its parameter's
// type, and the tables by number: tw_syscall_table, tw_syscall_args (Wine's
// argument bytes), tw_syscall_names and tw_syscall_count. Wine calls its own
// table through a pointer with raw register arguments; in wasm a call
// through a pointer with another signature traps, hence the wrappers.
//
// Table 1, win32u's: the same from dlls/win32u/win32syscalls.h with the
// prototypes of include/ntuser.h and include/ntgdi.h, into the same file:
// wrappers tw_win32_sys_<name> and the tables tw_win32_syscall_table,
// tw_win32_syscall_args, tw_win32_syscall_names and tw_win32_syscall_count.
//
// Checks (gate 4), each failure printed, then exit 1:
// - the ids run 0, 1, 2 ... in order (table 1: 0x1000, 0x1001 ...), and each
//   variant macro (SYSCALL_ENTRY_<name>) is defined as a plain SYSCALL_ENTRY;
// - every entry other than a stub (ALL_SYSCALL_STUBS) has a prototype, and
//   its parameter count is argbytes / 8;
// - every parameter is a scalar or a pointer (no struct, union or float by
//   value), except NtSetLdtEntries's two LDT_ENTRY, passed by value: the
//   wrapper copies each from its 8-byte argument;
// - table 0: every return type is NTSTATUS, except
//   NtGetCurrentProcessorNumber's ULONG. Both are zero-extended into RAX.
// - table 1: every return type is void, a scalar or a pointer of at most
//   8 bytes; one narrower than 64 bits is zero-extended, and void returns 0.
// The stubs' wrappers stop loudly (tempwind: unsupported: syscall NAME, or
// win32u syscall NAME); wine-unix.md section 13 records the deferral. The
// generator prints each table's split.
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

// ---- the syscall lists: ntsyscalls.h, win32syscalls.h ------------------------
// The x86-64 entries (ALL_SYSCALLS under _WIN64), their ids from firstId on,
// and the stubs (ALL_SYSCALL_STUBS).
function readList(rel, firstId) {
  const path = join(tree, rel);
  const src = readFileSync(path, 'utf8');
  const file = rel.replace(/.*\//, '');
  const block = /#ifdef _WIN64\s*\n#define ALL_SYSCALLS \\\n([\s\S]*?)\n#else/.exec(src);
  if (!block) {
    console.error(`gen-syscalls: ${path}: no "#ifdef _WIN64 / #define ALL_SYSCALLS" block`);
    process.exit(1);
  }
  const entries = [];
  for (const line of block[1].split('\n')) {
    const m = /^\s*SYSCALL_ENTRY(_\w+)?\(\s*(0x[0-9a-fA-F]+)\s*,\s*(\w+)\s*,\s*(\d+)\s*\)\s*\\?\s*$/.exec(line);
    if (!m) { fail(`${file}: cannot read the entry line: ${line.trim()}`); continue; }
    const [, variant, id, name, args] = m;
    if (variant) {
      const def = new RegExp(`#define SYSCALL_ENTRY${variant}\\(id,name,args\\) SYSCALL_ENTRY\\(id,name,args\\)`);
      if (variant !== `_${name}` || !def.test(src)) fail(`${file}: ${name}: variant macro SYSCALL_ENTRY${variant} is not defined as a plain entry`);
    }
    const want = firstId + entries.length;
    if (Number(id) !== want) fail(`${file}: ${name}: id ${id}, expected 0x${want.toString(16)}`);
    entries.push({ id: Number(id), name, argbytes: Number(args) });
  }
  const stubBlock = /#define ALL_SYSCALL_STUBS \\\n((?:\s*SYSCALL_STUB\(\s*\w+\s*\)\s*\\?\n?)+)/.exec(src);
  if (!stubBlock) fail(`${file}: no ALL_SYSCALL_STUBS`);
  const stubs = new Set(stubBlock ? [...stubBlock[1].matchAll(/SYSCALL_STUB\(\s*(\w+)\s*\)/g)].map((m) => m[1]) : []);
  for (const s of stubs) if (!entries.some((e) => e.name === s)) fail(`${file}: stub ${s} has no entry`);
  return { file, entries, stubs };
}

// ---- include/: prototypes and the types passed by value that are records ---
const incDir = join(tree, 'include');
// A parameter's type without its name, if it has one ("HANDLE handle",
// "const OBJECT_ATTRIBUTES *attr", "unsigned int x"): the last identifier is
// a name when a type word comes before it. An array parameter ("RECT r[]")
// is the pointer it decays to.
const QUALIFIERS = new Set(['const', 'volatile', 'struct', 'union', 'enum']);
const BUILTINS = new Set(['unsigned', 'signed', 'short', 'long', 'int', 'char', 'float', 'double', 'void']);
function withoutName(param) {
  let array = false;
  param = param.replace(/\s*\[[^\]]*\]\s*$/, () => { array = true; return ''; });
  const ids = [...param.matchAll(/[A-Za-z_]\w*/g)].filter((m) => !QUALIFIERS.has(m[0]));
  const last = ids.at(-1);
  let type = param;
  if (ids.length >= 2 && !BUILTINS.has(last[0]))
    type = (param.slice(0, last.index) + param.slice(last.index + last[0].length)).replace(/\s+/g, ' ').trim();
  return array ? `${type} *` : type;
}

// The prototypes "<prefix> RET WINAPI Nt...( ... );" of one header.
function readProtos(header, prefix) {
  const src = stripComments(readFileSync(join(incDir, header), 'utf8'));
  const protos = new Map();
  for (const m of src.matchAll(new RegExp(`${prefix}\\s+([\\w\\s*]+?)\\s+WINAPI\\s+(Nt\\w+)\\s*\\(`, 'g'))) {
    // the parameter list, with balanced parentheses
    let depth = 1, i = m.index + m[0].length;
    const start = i;
    while (i < src.length && depth) { if (src[i] === '(') depth++; else if (src[i] === ')') depth--; i++; }
    const list = src.slice(start, i - 1);
    const params = [];
    let cur = '', d = 0;
    for (const ch of list) {
      if (ch === '(') d++;
      if (ch === ')') d--;
      if (ch === ',' && !d) { params.push(cur); cur = ''; } else cur += ch;
    }
    params.push(cur);
    const ps = params.map((p) => p.replace(/\s+/g, ' ').trim()).filter((p) => p && p !== 'void' && p !== 'VOID')
      .map(withoutName);
    protos.set(m[2], { ret: m[1].replace(/\s+/g, ' ').trim(), params: ps, header });
  }
  return protos;
}

// Type names that denote a struct or union by value, from every typedef in
// include/*.h (and their plain aliases), and those that denote a float.
const records = new Set();
const floats = new Set(['float', 'double']);
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
for (const set of [records, floats]) {
  for (let grew = true; grew;) {
    grew = false;
    for (const [from, decls] of aliases) {
      if (!set.has(from)) continue;
      for (const decl of decls.split(',')) {
        if (decl.includes('*')) continue;
        const ids = decl.replace(/\[[^\]]*\]/g, '').match(/[A-Za-z_]\w*/g);
        const name = ids?.at(-1);
        if (name && !set.has(name)) { set.add(name); grew = true; }
      }
    }
  }
}
const isRecord = (type) => {
  if (type.includes('*')) return false;
  if (/\b(struct|union)\b/.test(type)) return true;
  return type.split(' ').some((w) => records.has(w));
};
const isFloat = (type) => !type.includes('*') && type.split(' ').some((w) => floats.has(w));

// ---- the checks and the wrappers -----------------------------------------------
// One wrapper per entry; ret(e, p, call) checks the return type and gives
// the statements that return the call's value.
function wrap({ entries, stubs }, protos, headers, { prefix, stubWhat, byValue = {}, ret }) {
  const wrappers = [];
  for (const e of entries) {
    if (stubs.has(e.name)) {
      wrappers.push(`ULONG_PTR ${prefix}${e.name}( const ULONG_PTR *args )\n{\n    tw_unsupported( "${stubWhat} ${e.name}" );\n}\n`);
      continue;
    }
    const p = protos.get(e.name);
    if (!p) { fail(`${e.name}: no prototype in ${headers}`); continue; }
    if (e.argbytes % 8 || p.params.length !== e.argbytes / 8)
      fail(`${e.name}: ${p.params.length} parameters in include/${p.header}, but ${e.argbytes} argument bytes`);
    const pre = [], args = [];
    p.params.forEach((type, i) => {
      if (isRecord(type)) {
        if (byValue[e.name] !== type) { fail(`${e.name}: parameter ${i} (${type}) is a struct or union passed by value`); return; }
        pre.push(`    ${type} a${i};\n    C_ASSERT( sizeof(a${i}) == sizeof(ULONG_PTR) );\n    memcpy( &a${i}, &args[${i}], sizeof(a${i}) );\n`);
        args.push(`a${i}`);
      } else if (isFloat(type)) {
        fail(`${e.name}: parameter ${i} (${type}) is a float passed by value`);
      } else {
        args.push(`(${type})args[${i}]`);
      }
    });
    const body = ret(e, p, `${e.name}( ${args.join(', ')} )`);
    if (body) wrappers.push(`ULONG_PTR ${prefix}${e.name}( const ULONG_PTR *args )\n{\n${pre.join('')}${body}}\n`);
  }
  return wrappers;
}

// Table 0: NTSTATUS, or NtGetCurrentProcessorNumber's ULONG.
const BY_VALUE = { NtSetLdtEntries: 'LDT_ENTRY' };   // winternl.h L4797
const RETURNS = { NtGetCurrentProcessorNumber: 'ULONG' };
const nt = readList('dlls/ntdll/ntsyscalls.h', 0);
const ntWrappers = wrap(nt, readProtos('winternl.h', 'NTSYSAPI'), 'include/winternl.h', {
  prefix: 'tw_sys_', stubWhat: 'syscall', byValue: BY_VALUE,
  ret: (e, p, call) => {
    const ret = RETURNS[e.name] ?? 'NTSTATUS';
    if (p.ret !== ret) fail(`${e.name}: returns ${p.ret}, expected ${ret}`);
    return `    return (${ret === 'NTSTATUS' ? 'ULONG' : ret})${call};\n`;
  },
});
for (const name of Object.keys(BY_VALUE)) if (!nt.entries.some((e) => e.name === name)) fail(`${name}: expected in ntsyscalls.h`);

// Table 1: void as 0; any other scalar or pointer zero-extended from its
// size (wasm64 is little-endian, so the copy fills the low bytes).
const w32 = readList('dlls/win32u/win32syscalls.h', 0x1000);
const w32Protos = new Map([...readProtos('ntuser.h', 'W32KAPI'), ...readProtos('ntgdi.h', 'W32KAPI')]);
const w32Wrappers = wrap(w32, w32Protos, 'include/ntuser.h or include/ntgdi.h', {
  prefix: 'tw_win32_sys_', stubWhat: 'win32u syscall',
  ret: (e, p, call) => {
    if (p.ret === 'void' || p.ret === 'VOID') return `    ${call};\n    return 0;\n`;
    if (isRecord(p.ret) || isFloat(p.ret)) { fail(`${e.name}: returns ${p.ret} by value, not a scalar or a pointer`); return null; }
    return `    ${p.ret} r = ${call};\n    ULONG_PTR v = 0;\n    C_ASSERT( sizeof(r) <= sizeof(v) );\n` +
      `    memcpy( &v, &r, sizeof(r) );\n    return v;\n`;
  },
});
for (const name of Object.keys(BY_VALUE)) if (w32.entries.some((e) => e.name === name)) fail(`${name}: also in win32syscalls.h`);

if (errors.length) {
  for (const e of errors) console.error(`gate 4 (syscalls): ${e}`);
  console.error(`gen-syscalls: ${errors.length} entries could not be matched (wine-unix.md, section 11)`);
  process.exit(1);
}

const tables = (pre, entries, wprefix) => `const tw_syscall_fn ${pre}_table[] =
{
${entries.map((e) => `    ${wprefix}${e.name},`).join('\n')}
};

const BYTE ${pre}_args[] =
{
${entries.map((e) => `    ${e.argbytes},  /* ${e.name} */`).join('\n')}
};

const char *const ${pre}_names[] =
{
${entries.map((e) => `    "${e.name}",`).join('\n')}
};

const unsigned int ${pre}_count = ${entries.length};
`;

const c = `/* Generated by native/wine/gen-syscalls.mjs from wine's dlls/ntdll/ntsyscalls.h
 * and include/winternl.h (table 0), and dlls/win32u/win32syscalls.h with
 * include/ntuser.h and include/ntgdi.h (table 1) (docs/design/wine-unix.md,
 * section 11); do not edit. One wrapper per x86-64 syscall, called with the
 * arguments the dispatcher gathered (native/wine/src/signal_wasm64.c). */
#include <string.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winbase.h"
#include "winternl.h"
#include "ntuser.h"
#include "ntgdi.h"

typedef ULONG_PTR (*tw_syscall_fn)( const ULONG_PTR *args );
_Noreturn void tw_unsupported( const char *what );

/* ---- table 0, ntdll's ---- */

${ntWrappers.join('\n')}
${tables('tw_syscall', nt.entries, 'tw_sys_')}
/* ---- table 1, win32u's ---- */

${w32Wrappers.join('\n')}
${tables('tw_win32_syscall', w32.entries, 'tw_win32_sys_')}`;
mkdirSync(dirname(out), { recursive: true });
if (!existsSync(out) || readFileSync(out, 'utf8') !== c) writeFileSync(out, c);
console.log(`gen-syscalls: table 0: ${nt.entries.length} syscalls (${nt.stubs.size} stubs); ` +
  `table 1: ${w32.entries.length} syscalls, ${w32.stubs.size} stubs and ${w32.entries.length - w32.stubs.size} wrapped -> ${out}`);
