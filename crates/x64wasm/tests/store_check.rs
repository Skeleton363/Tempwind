//! The store check of `rep` string ranges (`translator.md`, "Memory checks"):
//! every page the range covers is tested before the first iteration, run in
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

struct Outcome {
    exit: u32,
    state: abi::CpuState,
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
    let code = hex(code);
    let wasm = translate(&code, BASE, opts);
    let mut p = Process::new();
    p.write(BASE, &code);
    p.set_page_flags(BASE, code.len() as u64, abi::PAGE_CODE);
    for &page in code_pages {
        p.set_page_flags(page, 1, abi::PAGE_CODE);
    }
    p.write(0x4_0000, &[0x5a; 0x1_0000]);
    p.enter_state(BASE, regs);
    let module = p.link(&wasm);
    let exit = p.call_run(&module, "run");
    let state = p.state();
    assert_eq!(state.exit, exit);
    Outcome {
        exit,
        state,
        bytes: p.read(watch.0, watch.1),
    }
}

/// A range over three or more pages with `PAGE_CODE` only on a middle page
/// exits EXIT_UNSUPPORTED at the instruction, before any byte is written.
#[test]
fn rep_ranges_check_every_page() {
    // (code, offset of the string instruction, registers)
    let cases: [(&str, u64, &[(usize, u64)]); 5] = [
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
            assert_eq!(out.exit, abi::EXIT_UNSUPPORTED, "{ctx}");
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
        assert_eq!(out.exit, abi::EXIT_UNSUPPORTED);
        assert!(out.bytes.iter().all(|&b| b == 0));
    }
}

/// Ranges next to a code page, and RCX = 0 on one, store as usual.
#[test]
fn rep_ranges_beside_code_run() {
    let cases: [(&str, &[(usize, u64)], (u64, usize)); 4] = [
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
