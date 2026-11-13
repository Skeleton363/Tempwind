//! RFLAGS images beyond the L1 fixtures (`translator.md`, "RFLAGS images"):
//! - NT and ID round-trip through `popfq` then `pushfq` (the fixtures can't
//!   start a pushfq from them: ptrace drops ID from the input RFLAGS);
//! - a popfq image with TF or AC set exits with `EXIT_UNSUPPORTED` before
//!   any change: RIP at the popfq, RSP, RFLAGS and memory as before.

use testkit::host::{Host, Machine, Module};
use x64wasm::abi::{self, CpuState, flags};
use x64wasm::{CodeRegion, Options};

const CODE: u64 = 0x1000;
const STATE: u64 = 0x2000;
const DATA: u64 = 0x3000;
const MEMORY: u64 = 0x1_0000;
/// RSP at the start: the popfq image is the qword here.
const STACK: u64 = DATA + 0x100;

const NT: u64 = 1 << 14;
const AC: u64 = 1 << 18;
const ID: u64 = 1 << 21;

/// RFLAGS at the start: IF, bit 1, ZF and SF.
const START: u64 = 0x202 | flags::ZF | flags::SF;

const POPFQ: &[u8] = &[0x9d];
const POPFQ_PUSHFQ: &[u8] = &[0x9d, 0x9c];

#[derive(Debug)]
struct Out {
    exit: u32,
    rip: u64,
    rsp: u64,
    rflags: u64,
    /// The qword at `STACK`.
    top: u64,
}

fn module(host: &Host, code: &[u8]) -> Module {
    let t = x64wasm::translate(
        CodeRegion {
            base: CODE,
            bytes: code,
        },
        &[CODE],
        &Options::default(),
    )
    .unwrap();
    assert!(t.unsupported.is_empty(), "{code:02x?} is unsupported");
    host.compile(&t.wasm).unwrap()
}

fn run(m: &mut Machine, module: &Module, rflags: u64, image: u64) -> Out {
    let mut state = CpuState {
        rip: CODE,
        rflags,
        ..CpuState::default()
    };
    state.gpr[x64wasm::ir::RSP as usize] = STACK;
    m.write(STATE, &state.to_bytes()).unwrap();
    m.write(STACK, &image.to_le_bytes()).unwrap();
    let exit = m.run(module, STATE).unwrap();
    let out = m.read_state(STATE).unwrap();
    Out {
        exit,
        rip: out.rip,
        rsp: out.gpr[x64wasm::ir::RSP as usize],
        rflags: out.rflags,
        top: u64::from_le_bytes(m.read(STACK, 8).unwrap().try_into().unwrap()),
    }
}

#[test]
fn nt_and_id_round_trip_through_popfq_and_pushfq() {
    let host = Host::new().unwrap();
    let module = module(&host, POPFQ_PUSHFQ);
    let mut m = host.machine(MEMORY).unwrap();
    // (RFLAGS at the start, the popped image): NT and ID set by the image,
    // then cleared by it.
    let cases = [
        (START, 0x202 | NT | ID | flags::CF | flags::DF),
        (START | NT | ID | flags::DF, 0x202 | flags::OF),
    ];
    for (rflags, image) in cases {
        let out = run(&mut m, &module, rflags, image);
        assert_eq!(out.exit, abi::EXIT_LEFT_CODE, "{image:#x}");
        assert_eq!(out.rsp, STACK, "{image:#x}");
        // The pushed image and RFLAGS are the popped image: IF and bit 1
        // are the same in both, every other bit was loaded.
        assert_eq!(out.top, image, "{image:#x}: pushed image");
        assert_eq!(out.rflags, image, "{image:#x}: RFLAGS");
    }
}

#[test]
fn tf_or_ac_exit_before_any_change() {
    let host = Host::new().unwrap();
    let module = module(&host, POPFQ);
    let mut m = host.machine(MEMORY).unwrap();
    let rest = 0x202 | NT | ID | flags::CF | flags::DF;
    for stop in [flags::TF, AC, flags::TF | AC] {
        let image = rest | stop;
        let out = run(&mut m, &module, START, image);
        assert_eq!(out.exit, abi::EXIT_UNSUPPORTED, "{image:#x}");
        assert_eq!(out.rip, CODE, "{image:#x}");
        assert_eq!(out.rsp, STACK, "{image:#x}: RSP changed");
        assert_eq!(out.rflags, START, "{image:#x}: RFLAGS changed");
        assert_eq!(out.top, image, "{image:#x}: memory changed");
    }
    // Without TF and AC the same image loads.
    let out = run(&mut m, &module, START, rest);
    assert_eq!(out.exit, abi::EXIT_LEFT_CODE);
    assert_eq!(out.rsp, STACK + 8);
    assert_eq!(out.rflags, rest);
}
