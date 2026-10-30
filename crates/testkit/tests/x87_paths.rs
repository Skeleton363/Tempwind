//! x87 beyond the L1 fixtures (`translator.md`, "x87"): an unmasked
//! exception, a stack fault with IE unmasked, and a waiting instruction that
//! finds ES set (#MF) all exit with `EXIT_UNSUPPORTED` before any change:
//! RIP at the instruction, the x87 state, memory, registers and flags as
//! before. With the exception masked, the same instruction completes.

use testkit::host::{Host, Machine, Module};
use x64wasm::abi::{self, CpuState};
use x64wasm::{CodeRegion, Options};

const CODE: u64 = 0x1000;
const STATE: u64 = 0x2000;
const DATA: u64 = 0x3000;
const MEMORY: u64 = 0x1_0000;

/// FCW, FSW, the full tag word and R0-R7 as (significand, sign and exponent).
#[derive(Clone, Debug, PartialEq, Eq)]
struct X87 {
    fcw: u16,
    fsw: u16,
    ftw: u16,
    regs: [(u64, u16); 8],
}

const ONE: (u64, u16) = (1 << 63, 0x3fff);
const THREE: (u64, u16) = (3 << 62, 0x4000);
const ZERO: (u64, u16) = (0, 0);
const QNAN: (u64, u16) = (3 << 62, 0x7fff);

/// TOP = 6: ST(0) = R6, ST(1) = R7 hold `st0` and `st1`, the rest empty.
fn two(fcw: u16, st0: (u64, u16), st1: (u64, u16)) -> X87 {
    let tag = |v: (u64, u16)| -> u16 {
        match v.1 & 0x7fff {
            0x7fff => 2,
            0 if v.0 == 0 => 1,
            0 => 2,
            _ if v.0 >> 63 == 1 => 0,
            _ => 2,
        }
    };
    let mut regs = [ZERO; 8];
    regs[6] = st0;
    regs[7] = st1;
    X87 {
        fcw,
        fsw: 6 << 11,
        ftw: 0x0fff | tag(st0) << 12 | tag(st1) << 14,
        regs,
    }
}

struct Runner {
    host: Host,
}

#[derive(Debug, PartialEq, Eq)]
struct Out {
    exit: u32,
    rip: u64,
    gpr: [u64; 16],
    rflags: u64,
    x87: X87,
    data: Vec<u8>,
}

impl Runner {
    fn module(&self, code: &[u8]) -> Module {
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
        self.host.compile(&t.wasm).unwrap()
    }

    fn run(&self, m: &mut Machine, module: &Module, x87: &X87, data: &[u8]) -> Out {
        let mut state = CpuState {
            rip: CODE,
            rflags: 0x202 | abi::flags::ZF | abi::flags::SF,
            ..CpuState::default()
        };
        state.gpr[0] = 0x1234_5678_9abc_def0;
        state.gpr[3] = DATA;
        m.write(STATE, &state.to_bytes()).unwrap();
        m.write(STATE + abi::STATE_FCW, &x87.fcw.to_le_bytes())
            .unwrap();
        m.write(STATE + abi::STATE_FSW, &x87.fsw.to_le_bytes())
            .unwrap();
        m.write(STATE + abi::STATE_FTW, &x87.ftw.to_le_bytes())
            .unwrap();
        for (p, &(sig, se)) in x87.regs.iter().enumerate() {
            let at = STATE + abi::STATE_FPR + 16 * p as u64;
            m.write(at, &sig.to_le_bytes()).unwrap();
            m.write(at + 8, &se.to_le_bytes()).unwrap();
        }
        m.write(DATA, data).unwrap();
        let exit = m.run(module, STATE).unwrap();
        let out = m.read_state(STATE).unwrap();
        let u16_at =
            |m: &Machine, at| u16::from_le_bytes(m.read(at, 2).unwrap().try_into().unwrap());
        let mut regs = [ZERO; 8];
        for (p, r) in regs.iter_mut().enumerate() {
            let at = STATE + abi::STATE_FPR + 16 * p as u64;
            let sig = u64::from_le_bytes(m.read(at, 8).unwrap().try_into().unwrap());
            *r = (sig, u16_at(m, at + 8));
        }
        Out {
            exit,
            rip: out.rip,
            gpr: out.gpr,
            rflags: out.rflags,
            x87: X87 {
                fcw: u16_at(m, STATE + abi::STATE_FCW),
                fsw: u16_at(m, STATE + abi::STATE_FSW),
                ftw: u16_at(m, STATE + abi::STATE_FTW),
                regs,
            },
            data: m.read(DATA, data.len()).unwrap(),
        }
    }
}

#[test]
fn unmasked_exceptions_exit_before_any_change() {
    let runner = Runner {
        host: Host::new().unwrap(),
    };
    let masked = 0x037f;
    let f64_bytes = |x: f64| x.to_bits().to_le_bytes().to_vec();
    let junk = vec![0xa5u8; 16];
    // (name, code, x87 state with the exception unmasked, data at RBX)
    let cases: Vec<(&str, &[u8], X87, Vec<u8>)> = vec![
        // ZE unmasked: 1 / 0.
        (
            "fdiv st, st(1)",
            &[0xd8, 0xf1],
            two(0x037b, ONE, ZERO),
            junk.clone(),
        ),
        // ... with a pop.
        (
            "fdivp st(1), st",
            &[0xde, 0xf9],
            two(0x037b, ZERO, ONE),
            junk.clone(),
        ),
        // PE unmasked: 1 / 3 is inexact.
        (
            "fdivr st, st(1)",
            &[0xd8, 0xf9],
            two(0x035f, THREE, ONE),
            junk.clone(),
        ),
        // IE unmasked: an empty operand (stack underflow).
        (
            "fadd st, st(2)",
            &[0xd8, 0xc2],
            two(0x037e, ONE, ONE),
            junk.clone(),
        ),
        // IE unmasked: a push onto a full ST(7) (stack overflow).
        (
            "fldz",
            &[0xd9, 0xee],
            X87 {
                ftw: 0x0000,
                ..two(0x037e, ONE, ONE)
            },
            junk.clone(),
        ),
        // IE unmasked: an SNaN from memory.
        (
            "fld qword ptr [rbx]",
            &[0xdd, 0x03],
            two(0x037e, ONE, ONE),
            0x7ff0_0000_0000_0001u64.to_le_bytes().to_vec(),
        ),
        // PE unmasked: the store rounds; memory keeps its bytes.
        (
            "fstp dword ptr [rbx]",
            &[0xd9, 0x1b],
            two(0x035f, (0xffff_ffff_ffff_ffff, 0x3fff), ONE),
            junk.clone(),
        ),
        // IE unmasked: 1e300 doesn't fit an i32.
        (
            "fistp dword ptr [rbx]",
            &[0xdb, 0x1b],
            two(0x037e, (1 << 63, 0x3fff + 996), ONE),
            junk.clone(),
        ),
        // IE unmasked: fcomi with a QNaN; the flags stay.
        (
            "fcomi st, st(1)",
            &[0xdb, 0xf1],
            two(0x037e, ONE, QNAN),
            junk.clone(),
        ),
        // IE unmasked: fcomp with an empty operand.
        (
            "fcomp qword ptr [rbx]",
            &[0xdc, 0x1b],
            X87 {
                ftw: 0xffff,
                ..two(0x037e, ONE, ONE)
            },
            f64_bytes(2.0),
        ),
        // IE unmasked: sqrt of -1.
        (
            "fsqrt",
            &[0xd9, 0xfa],
            two(0x037e, (1 << 63, 0xbfff), ONE),
            junk.clone(),
        ),
    ];
    for (name, code, x87, data) in &cases {
        let module = runner.module(code);
        let mut m = runner.host.machine(MEMORY).unwrap();
        let before = runner.run(
            &mut m,
            &module,
            &X87 {
                fcw: masked,
                ..x87.clone()
            },
            data,
        );
        let out = runner.run(&mut m, &module, x87, data);
        assert_eq!(out.exit, abi::EXIT_UNSUPPORTED, "{name}");
        assert_eq!(out.rip, CODE, "{name}");
        assert_eq!(&out.x87, x87, "{name}: x87 state changed");
        assert_eq!(&out.data, data, "{name}: memory changed");
        assert_eq!(out.gpr[0], 0x1234_5678_9abc_def0, "{name}: RAX changed");
        assert_eq!(
            out.rflags,
            0x202 | abi::flags::ZF | abi::flags::SF,
            "{name}: RFLAGS changed"
        );
        // The same operands with the exception masked complete.
        assert_eq!(before.exit, abi::EXIT_LEFT_CODE, "{name} (masked)");
        assert_ne!(
            before.x87,
            X87 {
                fcw: masked,
                ..x87.clone()
            },
            "{name} (masked) changed nothing"
        );
    }
}

/// A waiting instruction that finds an unmasked exception pending (ES set)
/// raises #MF: `EXIT_UNSUPPORTED` before any change. The non-waiting forms
/// run.
#[test]
fn pending_exception_stops_waiting_forms() {
    let runner = Runner {
        host: Host::new().unwrap(),
    };
    // ZE pending and unmasked: ES and B set.
    let pending = X87 {
        fsw: 6 << 11 | 0x8084,
        ..two(0x037b, ONE, THREE)
    };
    let data = vec![
        0x7f, 0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    let waiting: &[(&str, &[u8])] = &[
        ("fwait", &[0x9b]),
        ("fadd st, st(1)", &[0xd8, 0xc1]),
        ("fld st(1)", &[0xd9, 0xc1]),
        ("fxch st(1)", &[0xd9, 0xc9]),
        ("fchs", &[0xd9, 0xe0]),
        ("fldcw [rbx]", &[0xd9, 0x2b]),
        ("fldenv [rbx]", &[0xd9, 0x23]),
        ("finit", &[0x9b, 0xdb, 0xe3]),
    ];
    for &(name, code) in waiting {
        let module = runner.module(code);
        let mut m = runner.host.machine(MEMORY).unwrap();
        let out = runner.run(&mut m, &module, &pending, &data);
        assert_eq!(out.exit, abi::EXIT_UNSUPPORTED, "{name}");
        assert_eq!(out.rip, CODE, "{name}");
        assert_eq!(out.x87, pending, "{name}: x87 state changed");
        assert_eq!(out.data, data, "{name}: memory changed");
    }
    let non_waiting: &[(&str, &[u8])] = &[
        ("fnstsw ax", &[0xdf, 0xe0]),
        ("fnstcw [rbx]", &[0xd9, 0x3b]),
        ("fnclex", &[0xdb, 0xe2]),
        ("fnstenv [rbx]", &[0xd9, 0x33]),
        ("fninit", &[0xdb, 0xe3]),
    ];
    for &(name, code) in non_waiting {
        let module = runner.module(code);
        let mut m = runner.host.machine(MEMORY).unwrap();
        let out = runner.run(&mut m, &module, &pending, &data);
        assert_eq!(out.exit, abi::EXIT_LEFT_CODE, "{name}");
    }
}
