//! Jump-table recovery (`docs/design/translator.md`, "Linking and images"):
//! GCC's relative tables behind a bounds check, read from the image, their
//! targets made block leaders; and near misses that must not count as tables.

use iced_x86::code_asm::*;
use iced_x86::{BlockEncoderOptions, Code, Instruction, MemoryOperand, Register};
use x64wasm::ir::{JumpTable, Program};
use x64wasm::lift::{Source, lift_source};
use x64wasm::{CodeRegion, Options};

const BASE: u64 = 0x1000;
/// A data-only region (an image's `.rdata`).
const RDATA: u64 = 0x8000;

/// What the code under test is told about its table.
struct Built {
    code: Vec<u8>,
    /// Address of the table and of each case.
    table: u64,
    cases: Vec<u64>,
    jmp: u64,
}

/// Assembles `body` twice: once to find the labels, then with the table
/// entries (`case - table`, i32) filled in. `body` gets the table label,
/// the case labels and the entries, and places everything, including the
/// table when it lives in the code.
fn build(
    n_cases: usize,
    body: impl Fn(&mut CodeAssembler, &mut CodeLabel, &mut [CodeLabel], &mut CodeLabel, &[u32]),
) -> Built {
    let pass = |entries: &[u32]| {
        let mut a = CodeAssembler::new(64).unwrap();
        let mut table = a.create_label();
        let mut jmp = a.create_label();
        let mut cases: Vec<CodeLabel> = (0..n_cases).map(|_| a.create_label()).collect();
        body(&mut a, &mut table, &mut cases, &mut jmp, entries);
        let r = a
            .assemble_options(BASE, BlockEncoderOptions::RETURN_NEW_INSTRUCTION_OFFSETS)
            .unwrap();
        let table = r.label_ip(&table).unwrap_or(RDATA);
        let cases: Vec<u64> = cases.iter().map(|c| r.label_ip(c).unwrap()).collect();
        let jmp = r.label_ip(&jmp).unwrap();
        (r.inner.code_buffer, table, cases, jmp)
    };
    let (_, table, cases, _) = pass(&vec![0; n_cases]);
    let entries: Vec<u32> = cases
        .iter()
        .map(|&c| (c.wrapping_sub(table)) as u32)
        .collect();
    let (code, table2, cases2, jmp) = pass(&entries);
    assert_eq!((table, &cases), (table2, &cases2), "layout is stable");
    Built {
        code,
        table,
        cases,
        jmp,
    }
}

/// Case bodies: `mov eax, i; ret`.
fn cases(a: &mut CodeAssembler, cases: &mut [CodeLabel]) {
    for (i, c) in cases.iter_mut().enumerate() {
        a.set_label(c).unwrap();
        a.mov(eax, i as u32 + 10).unwrap();
        a.ret().unwrap();
    }
}

fn table_data(a: &mut CodeAssembler, table: &mut CodeLabel, entries: &[u32]) {
    a.set_label(table).unwrap();
    a.dd(entries).unwrap();
}

fn lift(b: &Built, data: &[CodeRegion<'_>]) -> Program {
    let code = [CodeRegion {
        base: BASE,
        bytes: &b.code,
    }];
    let data = if data.is_empty() { &code[..] } else { data };
    let src = Source {
        code: &code,
        data,
        known: &[],
    };
    lift_source(&src, &[BASE], false).unwrap()
}

fn has_blocks(prog: &Program, addrs: &[u64]) -> bool {
    addrs
        .iter()
        .all(|a| prog.blocks.iter().any(|b| b.addr == *a))
}

fn assert_table(b: &Built, prog: &Program) {
    assert_eq!(
        prog.jump_tables,
        [JumpTable {
            jmp: b.jmp,
            table: b.table,
            targets: b.cases.clone(),
        }]
    );
    assert!(has_blocks(prog, &b.cases), "every case is a block");
    assert_eq!(prog.indirect_jumps, 1);
}

fn assert_no_table(b: &Built, prog: &Program) {
    assert!(prog.jump_tables.is_empty(), "{:x?}", prog.jump_tables);
    assert!(!has_blocks(prog, &b.cases[..1]));
}

/// `cmp ecx, 3; ja default; lea rdx, [table]; movsxd rax, [rdx+rcx*4];
/// add rax, rdx; jmp rax`, with the table after the code.
fn basic(
    a: &mut CodeAssembler,
    t: &mut CodeLabel,
    c: &mut [CodeLabel],
    j: &mut CodeLabel,
    e: &[u32],
) {
    let mut default = a.create_label();
    a.cmp(ecx, 3).unwrap();
    a.ja(default).unwrap();
    a.lea(rdx, ptr(*t)).unwrap();
    a.movsxd(rax, dword_ptr(rdx + rcx * 4)).unwrap();
    a.add(rax, rdx).unwrap();
    a.set_label(j).unwrap();
    a.jmp(rax).unwrap();
    cases(a, c);
    a.set_label(&mut default).unwrap();
    a.xor(eax, eax).unwrap();
    a.ret().unwrap();
    table_data(a, t, e);
}

#[test]
fn basic_table_targets_become_blocks() {
    let b = build(4, basic);
    let prog = lift(&b, &[]);
    assert_table(&b, &prog);
    // The whole translation still validates.
    let region = CodeRegion {
        base: BASE,
        bytes: &b.code,
    };
    let t = x64wasm::translate_regions(&[region], &[BASE], &[], &Options::default()).unwrap();
    assert!(b.cases.iter().all(|c| t.blocks.contains(c)));
    assert_eq!(t.jump_tables.len(), 1);
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&t.wasm)
        .unwrap();
}

#[test]
fn gcc_variants() {
    // Index and base in r8/r9, `add idx, base; jmp idx`; bound `cmp r8d`.
    let b = build(3, |a, t, c, j, e| {
        let mut default = a.create_label();
        a.cmp(r8d, 2).unwrap();
        a.ja(default).unwrap();
        a.lea(r9, ptr(*t)).unwrap();
        a.movsxd(r8, dword_ptr(r9 + r8 * 4)).unwrap();
        a.add(r8, r9).unwrap();
        a.set_label(j).unwrap();
        a.jmp(r8).unwrap();
        cases(a, c);
        a.set_label(&mut default).unwrap();
        a.ret().unwrap();
        table_data(a, t, e);
    });
    assert_table(&b, &lift(&b, &[]));

    // `mov edx, edx` (zero extension) between the lea and the movsxd.
    let b = build(3, |a, t, c, j, e| {
        let mut default = a.create_label();
        a.cmp(edx, 2).unwrap();
        a.ja(default).unwrap();
        a.lea(rcx, ptr(*t)).unwrap();
        a.mov(edx, edx).unwrap();
        a.movsxd(rax, dword_ptr(rcx + rdx * 4)).unwrap();
        a.add(rax, rcx).unwrap();
        a.set_label(j).unwrap();
        a.jmp(rax).unwrap();
        cases(a, c);
        a.set_label(&mut default).unwrap();
        a.ret().unwrap();
        table_data(a, t, e);
    });
    assert_table(&b, &lift(&b, &[]));

    // A 16-bit bound and `movzx eax, ax`.
    let b = build(5, |a, t, c, j, e| {
        let mut default = a.create_label();
        a.sub(eax, 0x62).unwrap();
        a.cmp(ax, 4).unwrap();
        a.ja(default).unwrap();
        a.lea(rdx, ptr(*t)).unwrap();
        a.movzx(eax, ax).unwrap();
        a.movsxd(rax, dword_ptr(rdx + rax * 4)).unwrap();
        a.add(rax, rdx).unwrap();
        a.set_label(j).unwrap();
        a.jmp(rax).unwrap();
        cases(a, c);
        a.set_label(&mut default).unwrap();
        a.ret().unwrap();
        table_data(a, t, e);
    });
    assert_table(&b, &lift(&b, &[]));

    // The bound on a memory operand, loaded before the lea.
    let b = build(6, |a, t, c, j, e| {
        let mut default = a.create_label();
        a.cmp(dword_ptr(rcx), 5).unwrap();
        a.ja(default).unwrap();
        a.mov(eax, dword_ptr(rcx)).unwrap();
        a.lea(r8, ptr(*t)).unwrap();
        a.movsxd(rax, dword_ptr(r8 + rax * 4)).unwrap();
        a.add(rax, r8).unwrap();
        a.set_label(j).unwrap();
        a.jmp(rax).unwrap();
        cases(a, c);
        a.set_label(&mut default).unwrap();
        a.ret().unwrap();
        table_data(a, t, e);
    });
    assert_table(&b, &lift(&b, &[]));

    // `cmp; jbe table_block` with the table block elsewhere, `jae` bounds
    // (count = imm), and `add base, idx; jmp base`.
    let b = build(4, |a, t, c, j, e| {
        let mut block = a.create_label();
        a.cmp(ecx, 4).unwrap();
        a.jb(block).unwrap();
        a.xor(eax, eax).unwrap();
        a.ret().unwrap();
        a.set_label(&mut block).unwrap();
        a.lea(rdx, ptr(*t)).unwrap();
        a.movsxd(rax, dword_ptr(rdx + rcx * 4)).unwrap();
        a.add(rdx, rax).unwrap();
        a.set_label(j).unwrap();
        a.jmp(rdx).unwrap();
        cases(a, c);
        table_data(a, t, e);
    });
    assert_table(&b, &lift(&b, &[]));
}

#[test]
fn table_in_a_data_region() {
    // The lea points at RDATA, outside the code; the table is read from the
    // data region and never decoded.
    let b = build(3, |a, _t, c, j, _e| {
        let mut default = a.create_label();
        a.cmp(ecx, 2).unwrap();
        a.ja(default).unwrap();
        // lea rdx, [rip + RDATA - next]: a RIP-relative operand's
        // displacement is the absolute target in iced.
        let lea = Instruction::with2(
            Code::Lea_r64_m,
            Register::RDX,
            MemoryOperand::with_base_displ(Register::RIP, RDATA as i64),
        )
        .unwrap();
        a.add_instruction(lea).unwrap();
        a.movsxd(rax, dword_ptr(rdx + rcx * 4)).unwrap();
        a.add(rax, rdx).unwrap();
        a.set_label(j).unwrap();
        a.jmp(rax).unwrap();
        cases(a, c);
        a.set_label(&mut default).unwrap();
        a.ret().unwrap();
    });
    let entries: Vec<u8> = b
        .cases
        .iter()
        .flat_map(|&c| ((c as i64 - RDATA as i64) as i32).to_le_bytes())
        .collect();
    let code = CodeRegion {
        base: BASE,
        bytes: &b.code,
    };
    let rdata = CodeRegion {
        base: RDATA,
        bytes: &entries,
    };
    let prog = lift(&b, &[code, rdata]);
    assert_eq!(prog.jump_tables.len(), 1);
    assert_eq!(prog.jump_tables[0].table, RDATA);
    assert!(has_blocks(&prog, &b.cases));
    assert!(prog.blocks.iter().all(|b| b.addr < RDATA));

    // Without the data region, nothing can be read: no table.
    let src = Source {
        code: &[code],
        data: &[code],
        known: &[],
    };
    let prog = lift_source(&src, &[BASE], false).unwrap();
    assert!(prog.jump_tables.is_empty());
    assert_eq!(prog.indirect_jumps, 1);
}

#[test]
fn near_misses_are_not_tables() {
    // No bounds check: the entry is the lea.
    let b = build(3, |a, t, c, j, e| {
        a.lea(rdx, ptr(*t)).unwrap();
        a.movsxd(rax, dword_ptr(rdx + rcx * 4)).unwrap();
        a.add(rax, rdx).unwrap();
        a.set_label(j).unwrap();
        a.jmp(rax).unwrap();
        cases(a, c);
        table_data(a, t, e);
    });
    assert_no_table(&b, &lift(&b, &[]));

    // The bound checks another register than the index.
    let b = build(3, |a, t, c, j, e| {
        let mut default = a.create_label();
        a.cmp(r8d, 2).unwrap();
        a.ja(default).unwrap();
        a.lea(rdx, ptr(*t)).unwrap();
        a.movsxd(rax, dword_ptr(rdx + rcx * 4)).unwrap();
        a.add(rax, rdx).unwrap();
        a.set_label(j).unwrap();
        a.jmp(rax).unwrap();
        cases(a, c);
        a.set_label(&mut default).unwrap();
        a.ret().unwrap();
        table_data(a, t, e);
    });
    assert_no_table(&b, &lift(&b, &[]));

    // The jump adds another register than the table base.
    let b = build(3, |a, t, c, j, e| {
        let mut default = a.create_label();
        a.cmp(ecx, 2).unwrap();
        a.ja(default).unwrap();
        a.lea(rdx, ptr(*t)).unwrap();
        a.movsxd(rax, dword_ptr(rdx + rcx * 4)).unwrap();
        a.add(rax, rbx).unwrap();
        a.set_label(j).unwrap();
        a.jmp(rax).unwrap();
        cases(a, c);
        a.set_label(&mut default).unwrap();
        a.ret().unwrap();
        table_data(a, t, e);
    });
    assert_no_table(&b, &lift(&b, &[]));

    // The base register is overwritten between the lea and the jump.
    let b = build(3, |a, t, c, j, e| {
        let mut default = a.create_label();
        a.cmp(ecx, 2).unwrap();
        a.ja(default).unwrap();
        a.lea(rdx, ptr(*t)).unwrap();
        a.mov(edx, ecx).unwrap();
        a.movsxd(rax, dword_ptr(rdx + rcx * 4)).unwrap();
        a.add(rax, rdx).unwrap();
        a.set_label(j).unwrap();
        a.jmp(rax).unwrap();
        cases(a, c);
        a.set_label(&mut default).unwrap();
        a.ret().unwrap();
        table_data(a, t, e);
    });
    assert_no_table(&b, &lift(&b, &[]));

    // A signed bound (`jg`) doesn't bound the index from below.
    let b = build(3, |a, t, c, j, e| {
        let mut default = a.create_label();
        a.cmp(ecx, 2).unwrap();
        a.jg(default).unwrap();
        a.lea(rdx, ptr(*t)).unwrap();
        a.movsxd(rax, dword_ptr(rdx + rcx * 4)).unwrap();
        a.add(rax, rdx).unwrap();
        a.set_label(j).unwrap();
        a.jmp(rax).unwrap();
        cases(a, c);
        a.set_label(&mut default).unwrap();
        a.ret().unwrap();
        table_data(a, t, e);
    });
    assert_no_table(&b, &lift(&b, &[]));

    // The bound is one too large: the extra entry points outside the code,
    // so the whole table is rejected.
    let b = build(3, |a, t, c, j, e| {
        let mut default = a.create_label();
        a.cmp(ecx, 3).unwrap();
        a.ja(default).unwrap();
        a.lea(rdx, ptr(*t)).unwrap();
        a.movsxd(rax, dword_ptr(rdx + rcx * 4)).unwrap();
        a.add(rax, rdx).unwrap();
        a.set_label(j).unwrap();
        a.jmp(rax).unwrap();
        cases(a, c);
        a.set_label(&mut default).unwrap();
        a.ret().unwrap();
        table_data(a, t, e);
        a.dd(&[0x7fff_0000]).unwrap();
    });
    let prog = lift(&b, &[]);
    assert_no_table(&b, &prog);
    assert_eq!(prog.indirect_jumps, 1);
}
