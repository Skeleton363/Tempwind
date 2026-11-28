//! The store check (`translator.md`, "Memory checks"): a hit exits with
//! `EXIT_FAULT`, `FAULT_CODE_WRITE` and the lowest address of the store's
//! range on a marked page; every page a `rep` string range covers is tested
//! before the first iteration, up to the page-state table's end. Run in
//! wasmtime with a shared memory as in a Wine process.

mod common;

use common::{Process, hex, translate};
use x64wasm::Options;
use x64wasm::abi;

const BASE: u64 = 0x1_0000;
const RCX: usize = 1;
const RAX: usize = 0;
const RSI: usize = 6;
const RDI: usize = 7;

/// Registers by GPR number.
type Regs = &'static [(usize, u64)];

fn option_sets() -> [Options; 3] {
    let store = Options {
        store_check: true,
        ..Options::default()
    };
    [
        store.clone(),
        Options {
            null_check: true,
            ..store.clone()
        },
        Options {
            structured: false,
            sync_all: true,
            ..store
        },
    ]
}

/// Written to the fault fields before a run, so a test sees that the run
/// wrote them.
const STALE: u64 = 0xdead_beef_dead_beef;

struct Outcome {
    exit: u32,
    state: abi::CpuState,
    /// The fault kind and address the run left in the state block.
    fault: (u32, u64),
    /// The bytes of `[watch.0, watch.0 + watch.1)` after the run.
    bytes: Vec<u8>,
}

/// Runs `code` at [`BASE`] with `regs`, `PAGE_CODE` on the code page and on
/// each page in `code_pages`, the source filled with 0x5a.
fn run(
    code: &str,
    regs: &[(usize, u64)],
    code_pages: &[u64],
    opts: &Options,
    watch: (u64, usize),
) -> Outcome {
    run_with(code, regs, &[], code_pages, opts, watch)
}

/// [`run`], with the u64 values `mem` written to guest memory first.
fn run_with(
    code: &str,
    regs: &[(usize, u64)],
    mem: &[(u64, u64)],
    code_pages: &[u64],
    opts: &Options,
    watch: (u64, usize),
) -> Outcome {
    run_in(Process::new(), code, regs, mem, code_pages, opts, watch)
}

/// [`run_with`] in the process `p`.
fn run_in(
    mut p: Process,
    code: &str,
    regs: &[(usize, u64)],
    mem: &[(u64, u64)],
    code_pages: &[u64],
    opts: &Options,
    watch: (u64, usize),
) -> Outcome {
    let code = hex(code);
    let wasm = translate(&code, BASE, opts);
    p.write(BASE, &code);
    p.set_page_flags(BASE, code.len() as u64, abi::PAGE_CODE);
    for &page in code_pages {
        p.set_page_flags(page, 1, abi::PAGE_CODE);
    }
    p.write(0x4_0000, &[0x5a; 0x1_0000]);
    p.enter_state(BASE, regs);
    p.write_u64(common::STATE + abi::STATE_FAULT_ADDR, STALE);
    p.write_u32(common::STATE + abi::STATE_FAULT_KIND, STALE as u32);
    for &(addr, v) in mem {
        p.write_u64(addr, v);
    }
    let module = p.link(&wasm);
    let exit = p.call_run(&module, "run");
    let state = p.state();
    assert_eq!(state.exit, exit);
    let fault = (
        p.read_u32(common::STATE + abi::STATE_FAULT_KIND),
        p.read_u64(common::STATE + abi::STATE_FAULT_ADDR),
    );
    Outcome {
        exit,
        state,
        fault,
        bytes: p.read(watch.0, watch.1),
    }
}

/// A range over three or more pages with `PAGE_CODE` only on a middle page
/// faults with `FAULT_CODE_WRITE` at the instruction, before any byte is
/// written, at the marked page's base (the range's lowest address there).
#[test]
fn rep_ranges_check_every_page() {
    // (code, offset of the string instruction, registers)
    let cases: [(&str, u64, Regs); 5] = [
        // rep stosb (the byte fast path)
        ("f3aa c3", 0, &[(RDI, 0x2_0000), (RCX, 0x3000), (RAX, 0xaa)]),
        // rep stosq (the element loop)
        ("f348ab c3", 0, &[(RDI, 0x2_0000), (RCX, 0x600), (RAX, !0)]),
        // std; rep stosq; cld: downwards, from the range's last element
        (
            "fd f348ab fc c3",
            1,
            &[(RDI, 0x2_2ff8), (RCX, 0x600), (RAX, !0)],
        ),
        // rep movsb (the memmove fast path)
        (
            "f3a4 c3",
            0,
            &[(RSI, 0x4_0000), (RDI, 0x2_0000), (RCX, 0x3000)],
        ),
        // std; rep movsd; cld
        (
            "fd f3a5 fc c3",
            1,
            &[(RSI, 0x4_2ffc), (RDI, 0x2_2ffc), (RCX, 0xc00)],
        ),
    ];
    for opts in option_sets() {
        for (code, at, regs) in cases {
            let out = run(code, regs, &[0x2_1000], &opts, (0x2_0000, 0x3000));
            let ctx = format!("{code} with {opts:?}");
            assert_eq!(out.exit, abi::EXIT_FAULT, "{ctx}");
            assert_eq!(out.fault, (abi::FAULT_CODE_WRITE, 0x2_1000), "{ctx}");
            assert_eq!(out.state.rip, BASE + at, "{ctx}: at the instruction");
            for &(r, v) in regs {
                assert_eq!(out.state.gpr[r], v, "{ctx}: register {r} unchanged");
            }
            assert!(out.bytes.iter().all(|&b| b == 0), "{ctx}: nothing stored");
        }
        // Sixteen pages, the code on the ninth.
        let out = run(
            "f3aa c3",
            &[(RDI, 0x2_0000), (RCX, 0x1_0000), (RAX, 1)],
            &[0x2_8000],
            &opts,
            (0x2_0000, 0x1_0000),
        );
        assert_eq!(out.exit, abi::EXIT_FAULT);
        assert_eq!(out.fault, (abi::FAULT_CODE_WRITE, 0x2_8000));
        assert!(out.bytes.iter().all(|&b| b == 0));
    }
}

/// Ranges next to a code page, and RCX = 0 on one, store as usual.
#[test]
fn rep_ranges_beside_code_run() {
    let cases: [(&str, Regs, (u64, usize)); 4] = [
        // Up to the last byte before the code page.
        (
            "f3aa c3",
            &[(RDI, 0x1_e000), (RCX, 0x3000), (RAX, 0xaa)],
            (0x1_e000, 0x3000),
        ),
        // From the first byte after it, over three pages.
        (
            "f348ab c3",
            &[(RDI, 0x2_2000), (RCX, 0x600), (RAX, !0)],
            (0x2_2000, 0x3000),
        ),
        // Downwards, ending at the first byte after it.
        (
            "fd f348ab fc c3",
            &[(RDI, 0x2_4ff8), (RCX, 0x600), (RAX, !0)],
            (0x2_2000, 0x3000),
        ),
        // RCX = 0 on the code page checks nothing.
        ("f3aa c3", &[(RDI, 0x2_1000), (RCX, 0)], (0x2_1000, 0)),
    ];
    for opts in option_sets() {
        for (code, regs, watch) in cases {
            let out = run(code, regs, &[0x2_1000], &opts, watch);
            let ctx = format!("{code} with {opts:?}");
            assert_eq!(out.exit, abi::EXIT_LEFT_CODE, "{ctx}");
            assert_eq!(out.state.rip, abi::SENTINEL_RET, "{ctx}");
            assert_eq!(out.state.gpr[RCX], 0, "{ctx}");
            assert!(out.bytes.iter().all(|&b| b != 0), "{ctx}: stored");
        }
    }
}

/// `pop m64` (wine-11.0 `RtlCaptureContext`: `pushfq; pop qword [rcx+0x44]`):
/// the destination's checks come before RSP changes, and an RSP-based
/// destination uses RSP after the increment (SDM, POP).
#[test]
fn pop_to_memory_checks_first() {
    const RSP: usize = 4;
    const VALUE: u64 = 0x0123_4567_89ab_cdef;
    let store = option_sets()[0].clone();
    let both = Options {
        null_check: true,
        ..store.clone()
    };
    for opts in [store, both] {
        // pop qword [rcx+0x44]; ret: the destination ends on the code page.
        let out = run(
            "8f4144 c3",
            &[(RCX, 0x2_0fc0)],
            &[0x2_1000],
            &opts,
            (0x2_1000, 8),
        );
        assert_eq!((out.exit, out.state.rip), (abi::EXIT_FAULT, BASE));
        assert_eq!(out.fault, (abi::FAULT_CODE_WRITE, 0x2_1004));
        assert_eq!(out.state.gpr[RSP], common::STACK, "RSP unchanged");
        assert!(out.bytes.iter().all(|&b| b == 0), "nothing stored");
        // pop qword [rsp+8]: the destination, at the old RSP + 16, is on
        // the code page; the old RSP + 8 is not.
        let out = run(
            "8f442408 c3",
            &[(RSP, 0x2_0ff0)],
            &[0x2_1000],
            &opts,
            (0x2_1000, 8),
        );
        assert_eq!((out.exit, out.state.rip), (abi::EXIT_FAULT, BASE));
        assert_eq!(out.fault, (abi::FAULT_CODE_WRITE, 0x2_1000));
        assert_eq!(out.state.gpr[RSP], 0x2_0ff0, "RSP unchanged");
        assert!(out.bytes.iter().all(|&b| b == 0), "nothing stored");
        // Away from code it stores the popped value at the old RSP + 16,
        // and ret pops the sentinel at the old RSP + 8.
        let out = run_with(
            "8f442408 c3",
            &[(RSP, 0x2_0f00)],
            &[(0x2_0f00, VALUE), (0x2_0f08, abi::SENTINEL_RET)],
            &[0x2_1000],
            &opts,
            (0x2_0f10, 8),
        );
        assert_eq!(
            (out.exit, out.state.rip),
            (abi::EXIT_LEFT_CODE, abi::SENTINEL_RET)
        );
        assert_eq!(out.state.gpr[RSP], 0x2_0f10);
        assert_eq!(out.bytes, VALUE.to_le_bytes());
        // pop qword [rcx+0x44] stores at RCX + 0x44.
        let out = run_with(
            "8f4144 c3",
            &[(RCX, 0x2_0000)],
            &[
                (common::STACK, VALUE),
                (common::STACK + 8, abi::SENTINEL_RET),
            ],
            &[0x2_1000],
            &opts,
            (0x2_0044, 8),
        );
        assert_eq!(out.exit, abi::EXIT_LEFT_CODE);
        assert_eq!(out.bytes, VALUE.to_le_bytes());
    }
}

/// A single store's hit: `FAULT_CODE_WRITE` at the store, the state from
/// before it, and the store's lowest address on a marked page: `addr` when
/// its page is marked, else the base of the last byte's page.
#[test]
fn single_store_faults_with_its_address() {
    const RBX: usize = 3;
    // (RBX, marked pages besides the code's, fault address)
    let cases: [(u64, &[u64], u64); 4] = [
        // Inside a marked page.
        (0x2_1010, &[0x2_1000], 0x2_1010),
        // Crossing into a marked page: its base.
        (0x2_0ffe, &[0x2_1000], 0x2_1000),
        // Crossing out of a marked page: the first byte.
        (0x2_1ffe, &[0x2_1000], 0x2_1ffe),
        // Both pages marked: the first byte.
        (0x2_0ffe, &[0x2_0000, 0x2_1000], 0x2_0ffe),
    ];
    for opts in option_sets() {
        for (rbx, pages, want) in cases {
            // mov [rbx], ecx; ret
            let regs = [(RBX, rbx), (RCX, 0x1122_3344)];
            let out = run("890b c3", &regs, pages, &opts, (rbx, 4));
            let ctx = format!("{rbx:#x} with {opts:?}");
            assert_eq!((out.exit, out.state.rip), (abi::EXIT_FAULT, BASE), "{ctx}");
            assert_eq!(out.fault, (abi::FAULT_CODE_WRITE, want), "{ctx}");
            assert_eq!(out.state.gpr[RBX], rbx, "{ctx}: registers unchanged");
            assert_eq!(out.bytes, [0; 4], "{ctx}: nothing stored");
        }
        // Away from marked pages the store runs.
        let regs = [(RBX, 0x2_0ffe), (RCX, 0x1122_3344)];
        let out = run("890b c3", &regs, &[0x2_2000], &opts, (0x2_0ffe, 4));
        assert_eq!(out.exit, abi::EXIT_LEFT_CODE);
        assert_eq!(out.bytes, 0x1122_3344u32.to_le_bytes());
    }
}

/// A `rep` range reports its lowest address on a marked page, whichever
/// way DF runs (`translator.md`, "Memory checks": for DF = 1 the CPU would
/// report the highest marked page's first store).
#[test]
fn rep_range_fault_address_both_directions() {
    // (code, offset of the string instruction, registers, marked pages,
    // fault address)
    type Case = (&'static str, u64, Regs, &'static [u64], u64);
    let cases: [Case; 6] = [
        // rep stosb from inside a marked page: its first byte.
        (
            "f3aa c3",
            0,
            &[(RDI, 0x2_1800), (RCX, 0x1000)],
            &[0x2_1000],
            0x2_1800,
        ),
        // The same range downwards: still its low end.
        (
            "fd f3aa fc c3",
            1,
            &[(RDI, 0x2_27ff), (RCX, 0x1000)],
            &[0x2_1000],
            0x2_1800,
        ),
        // Two marked pages, upwards: the lower one's base.
        (
            "f348ab c3",
            0,
            &[(RDI, 0x2_0000), (RCX, 0x800)],
            &[0x2_1000, 0x2_3000],
            0x2_1000,
        ),
        // Two marked pages, downwards: the lower one's base too.
        (
            "fd f348ab fc c3",
            1,
            &[(RDI, 0x2_3ff8), (RCX, 0x800)],
            &[0x2_1000, 0x2_3000],
            0x2_1000,
        ),
        // rep movsq downwards, ending inside a marked page: the last
        // element, the range's low end.
        (
            "fd f348a5 fc c3",
            1,
            &[(RSI, 0x4_2ff8), (RDI, 0x2_2ff8), (RCX, 0x500)],
            &[0x2_0000],
            0x2_0800,
        ),
        // rep stosq upwards, its one element crossing into a marked page.
        (
            "f348ab c3",
            0,
            &[(RDI, 0x2_0ffc), (RCX, 1)],
            &[0x2_1000],
            0x2_1000,
        ),
    ];
    for opts in option_sets() {
        for (code, at, regs, pages, want) in cases {
            let out = run(code, regs, pages, &opts, (0x2_0000, 0x4000));
            let ctx = format!("{code} {regs:x?} with {opts:?}");
            assert_eq!(out.exit, abi::EXIT_FAULT, "{ctx}");
            assert_eq!(out.state.rip, BASE + at, "{ctx}: at the instruction");
            assert_eq!(out.fault, (abi::FAULT_CODE_WRITE, want), "{ctx}");
            for &(r, v) in regs {
                assert_eq!(out.state.gpr[r], v, "{ctx}: register {r} unchanged");
            }
            assert!(out.bytes.iter().all(|&b| b == 0), "{ctx}: nothing stored");
        }
    }
}

/// The walk's bound (`translator.md`, "Memory checks", "`rep` string
/// ranges"): pages at or above 16 GiB (`abi::MAX_MEMORY_PAGES`) count as
/// marked, so a range that reaches them faults with `FAULT_CODE_WRITE` at
/// max(low end, 16 GiB), without reading past the page-state table.
#[test]
fn rep_walk_stops_at_the_table_end() {
    const TOP: u64 = abi::MAX_MEMORY_PAGES << 16;
    // The page-state table of the whole 16 GiB past the harness's lookup
    // table, then 1 MiB that an unbounded walk reads (all clear) before it
    // traps past the memory.
    let flags = common::MEMORY_PAGES << 16;
    let pages = (flags + (TOP >> abi::PAGE_FLAGS_SHIFT) + 0x10_0000) >> 16;
    let cases: [(&str, u64, Regs, u64); 5] = [
        // A garbage RCX from low memory: the walk crosses the whole table.
        ("f3aa c3", 0, &[(RDI, 0x2_0000), (RCX, 1 << 48)], TOP),
        // rep stosq up from just below 16 GiB.
        ("f348ab c3", 0, &[(RDI, TOP - 0x2000), (RCX, 1 << 40)], TOP),
        // Downwards across 16 GiB: the low end is below it.
        (
            "fd f3aa fc c3",
            1,
            &[(RDI, TOP + 0x1000), (RCX, 0x3000)],
            TOP,
        ),
        // A range that starts above 16 GiB: its low end.
        (
            "f3aa c3",
            0,
            &[(RDI, TOP + 0x1234), (RCX, 0x10)],
            TOP + 0x1234,
        ),
        // rep movsb whose destination starts at 16 GiB exactly.
        (
            "f3a4 c3",
            0,
            &[(RSI, 0x4_0000), (RDI, TOP), (RCX, 0x10)],
            TOP,
        ),
    ];
    for opts in option_sets() {
        for (code, at, regs, want) in cases {
            let p = Process::with_page_flags(flags, pages);
            let out = run_in(p, code, regs, &[], &[], &opts, (0x2_0000, 0x10));
            let ctx = format!("{code} {regs:x?} with {opts:?}");
            assert_eq!(out.exit, abi::EXIT_FAULT, "{ctx}");
            assert_eq!(out.state.rip, BASE + at, "{ctx}: at the instruction");
            assert_eq!(out.fault, (abi::FAULT_CODE_WRITE, want), "{ctx}");
            for &(r, v) in regs {
                assert_eq!(out.state.gpr[r], v, "{ctx}: register {r} unchanged");
            }
        }
    }
}
