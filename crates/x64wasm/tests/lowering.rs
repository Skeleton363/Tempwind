//! Lowering tests that need no wasm runtime (`docs/design/translator.md`,
//! "Tests, before each family's code"): flag liveness with readers and
//! partial and conditional records, fault checks before the first side
//! effect, and #UD for `ud2` and bytes that don't decode. The instruction
//! semantics themselves are checked by the L1 fixtures (`testkit replay`).

use x64wasm::ir::{Op, Program, Term};
use x64wasm::{CodeRegion, Options, abi};

const BASE: u64 = 0x1000;

fn hex(s: &str) -> Vec<u8> {
    let d: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..d.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&d[i..i + 2], 16).unwrap())
        .collect()
}

fn lift(code: &str) -> Program {
    let bytes = hex(code);
    x64wasm::lift(
        CodeRegion {
            base: BASE,
            bytes: &bytes,
        },
        &[BASE],
    )
    .unwrap()
}

/// `live` of every flag record, in order.
fn liveness(code: &str) -> Vec<bool> {
    lift(code)
        .blocks
        .iter()
        .flat_map(|b| b.insns.iter())
        .flat_map(|i| i.ops.iter())
        .filter_map(|op| match op {
            Op::Flags { live, .. } | Op::FlagsUpdate { live, .. } => Some(*live),
            _ => None,
        })
        .collect()
}

#[test]
fn partial_records_keep_the_previous_one() {
    // cmp rax, rbx; inc rcx; ret: inc keeps cmp's CF.
    assert_eq!(liveness("4839d8 48ffc1 c3"), [true, true]);
    // cmp; inc; add rax, rax; ret: add overwrites both.
    assert_eq!(liveness("4839d8 48ffc1 4801c0 c3"), [false, false, true]);
    // cmp; cmc; ret and cmp; cmc; add; ret
    assert_eq!(liveness("4839d8 f5 c3"), [true, true]);
    assert_eq!(liveness("4839d8 f5 4801c0 c3"), [false, false, true]);
    // cmp; bt rcx, rdx (CF only); ret
    assert_eq!(liveness("4839d8 480fa3d1 c3"), [true, true]);
    // cmp; rol rax, 1 (CF and OF only); ret
    assert_eq!(liveness("4839d8 48d1c0 c3"), [true, true]);
}

#[test]
fn conditional_records_keep_the_previous_one() {
    // cmp; shl rax, cl; ret: a count of 0 keeps cmp's flags.
    assert_eq!(liveness("4839d8 48d3e0 c3"), [true, true]);
    // cmp; shl rax, 3; ret: an immediate count is a full record.
    assert_eq!(liveness("4839d8 48c1e003 c3"), [false, true]);
    // cmp; shl rax, 0; ret: no record at all.
    assert_eq!(liveness("4839d8 48c1e000 c3"), [true]);
    // cmp; shl rax, cl; add; ret
    assert_eq!(liveness("4839d8 48d3e0 4801c0 c3"), [false, false, true]);
}

#[test]
fn compare_strings_write_records() {
    // cmp; cmpsb; ret: cmpsb is a full record.
    assert_eq!(liveness("4839d8 a6 c3"), [false]);
    // cmp; repe cmpsb; ret: RCX = 0 keeps cmp's flags.
    assert_eq!(liveness("4839d8 f3a6 c3"), [true]);
    // cmp; repne scasb; ret
    assert_eq!(liveness("4839d8 f2ae c3"), [true]);
    // cmp; rep movsb; ret: movs writes no flags.
    assert_eq!(liveness("4839d8 f3a4 c3"), [true]);
}

#[test]
fn readers_make_records_live() {
    // cmp; adc rax, rcx (reads CF); add; ret
    assert_eq!(liveness("4839d8 4811c8 4801c0 c3"), [true, false, true]);
    // cmp; setb al; add; ret
    assert_eq!(liveness("4839d8 0f92c0 4801c0 c3"), [true, true]);
    // cmp; cmovb rax, rcx; add; ret
    assert_eq!(liveness("4839d8 480f42c1 4801c0 c3"), [true, true]);
    // cmp; rcl rax, 1 (reads CF); add; ret
    assert_eq!(liveness("4839d8 48d1d0 4801c0 c3"), [true, false, true]);
}

#[test]
fn rcx_branches_read_no_flags() {
    // top: add rax, rax; jrcxz done; add rax, rcx; jmp top; done: xor eax, eax; ret
    // The first add is overwritten on both paths.
    assert_eq!(
        liveness("4801c0 e305 4801c8 ebf6 31c0 c3"),
        [false, false, true]
    );
    // add; loope top (reads ZF); ret
    assert_eq!(liveness("4801c0 e1fb c3"), [true]);
}

/// Every faulting form puts its fault checks before the instruction's first
/// side effect, so a fault sees the state from before the instruction.
#[test]
fn faults_come_before_side_effects() {
    let forms = [
        "f6f3",   // div bl
        "66f7f3", // div bx
        "f7f3",   // div ebx
        "48f7f3", // div rbx
        "f6fb",   // idiv bl
        "66f7fb", // idiv bx
        "f7fb",   // idiv ebx
        "48f7fb", // idiv rbx
        "f633",   // div byte [rbx]
        "48f73b", // idiv qword [rbx]
        "64f733", // div dword fs:[rbx]
        "f6f4",   // div ah
        "48f7f2", // div rdx
        "48f7f8", // idiv rax
    ];
    for code in forms {
        let prog = lift(code);
        let insn = &prog.blocks[0].insns[0];
        let faults: Vec<usize> = (0..insn.ops.len())
            .filter(|&i| matches!(insn.ops[i], Op::FaultIf { .. }))
            .collect();
        assert!(
            !faults.is_empty(),
            "{code}: no fault check\n{}",
            prog.dump()
        );
        let first_effect = insn
            .ops
            .iter()
            .position(Op::has_side_effect)
            .unwrap_or(insn.ops.len());
        assert!(
            faults.iter().all(|&f| f < first_effect),
            "{code}: a fault check after a side effect\n{}",
            prog.dump()
        );
    }
}

#[test]
fn ud2_and_undecodable_bytes_fault() {
    // ud2; and 06 (push es), invalid in 64-bit mode; and lock on a register
    // destination.
    for code in ["0f0b", "06c3", "f04801d8"] {
        let prog = lift(code);
        assert!(
            matches!(
                prog.blocks[0].term,
                Term::Fault {
                    kind: abi::FAULT_INVALID_OPCODE,
                    rip: BASE
                }
            ),
            "{code}: {}",
            prog.dump()
        );
    }
    // cpuid decodes: it stays unsupported.
    let prog = lift("0fa2");
    assert!(matches!(
        prog.blocks[0].term,
        Term::Exit {
            reason: abi::EXIT_UNSUPPORTED,
            rip: BASE
        }
    ));
    // Faults are not unsupported sites, and the module validates.
    let bytes = hex("0f0b");
    let t = x64wasm::translate(
        CodeRegion {
            base: BASE,
            bytes: &bytes,
        },
        &[BASE],
        &Options::default(),
    )
    .unwrap();
    assert!(t.unsupported.is_empty());
}
