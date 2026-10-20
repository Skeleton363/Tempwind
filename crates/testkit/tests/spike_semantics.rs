//! Hand-checked translator semantics: exits (LEFT_CODE / UNSUPPORTED),
//! partial registers, push/pop RSP corner cases and RFLAGS preservation.
//! Expected values come from the Intel SDM, not from native execution.

use iced_x86::code_asm::*;
use testkit::host::Host;
use x64wasm::abi::{self, CpuState, flags};
use x64wasm::{CodeRegion, Error, Options};

const BASE: u64 = 0x1_0000;
const STACK: u64 = 0x8_0000;
const STATE: u64 = 0x1000;
const MEMORY: u64 = 0x10_0000;

fn code(build: impl FnOnce(&mut CodeAssembler) -> Result<(), IcedError>) -> Vec<u8> {
    let mut a = CodeAssembler::new(64).unwrap();
    build(&mut a).unwrap();
    a.assemble(BASE).unwrap()
}

fn state(regs: &[(usize, u64)]) -> CpuState {
    let mut s = CpuState {
        rip: BASE,
        rflags: 0x202,
        ..CpuState::default()
    };
    s.gpr[4] = STACK;
    for &(r, v) in regs {
        s.gpr[r] = v;
    }
    s
}

struct Run {
    out: CpuState,
    machine: testkit::host::Machine,
}

fn run_with(bytes: &[u8], entries: &[u64], input: &CpuState, mem: &[(u64, &[u8])]) -> Run {
    let t = x64wasm::translate(
        CodeRegion { base: BASE, bytes },
        entries,
        &Options::default(),
    )
    .unwrap();
    let host = Host::new().unwrap();
    let module = host.compile(&t.wasm).unwrap();
    let mut machine = host.machine(MEMORY).unwrap();
    machine.write(BASE, bytes).unwrap();
    for (addr, data) in mem {
        machine.write(*addr, data).unwrap();
    }
    let out = machine.run_state(&module, STATE, input).unwrap();
    Run { out, machine }
}

fn run(bytes: &[u8], input: &CpuState) -> CpuState {
    run_with(bytes, &[BASE], input, &[]).out
}

fn end(bytes: &[u8]) -> u64 {
    BASE + bytes.len() as u64
}

const RAX: usize = 0;
const RCX: usize = 1;
const RDX: usize = 2;
const RBX: usize = 3;
const RSP: usize = 4;

#[test]
fn falls_off_the_region_with_left_code() {
    let c = code(|a| a.mov(eax, 7));
    let out = run(&c, &state(&[(RAX, u64::MAX)]));
    assert_eq!(out.exit, abi::EXIT_LEFT_CODE);
    assert_eq!(out.rip, end(&c));
    assert_eq!(out.gpr[RAX], 7, "mov r32 zero-extends");
}

#[test]
fn unsupported_exits_before_the_instruction() {
    // mov eax, imm32 (5 bytes); cmp rcx, rdx (3 bytes); ud2
    let ud2_at = BASE + 8;
    let c = code(|a| {
        a.mov(eax, 5)?;
        a.cmp(rcx, rdx)?;
        a.ud2()?;
        a.mov(eax, 6)
    });
    let out = run(&c, &state(&[(RCX, 1), (RDX, 2)]));
    assert_eq!(out.exit, abi::EXIT_UNSUPPORTED);
    assert_eq!(out.rip, ud2_at);
    assert_eq!(out.gpr[RAX], 5);
    // Flags of `cmp 1, 2`: CF, SF, AF, PF (0xff has even parity).
    let status = out.rflags & flags::STATUS;
    assert_eq!(status, flags::CF | flags::SF | flags::AF | flags::PF);
    assert_eq!(out.rflags & !flags::STATUS, 0x202);
}

#[test]
fn unsupported_forms() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("fs segment", code(|a| a.mov(rax, qword_ptr(0x10).fs()))),
        ("gs segment", code(|a| a.add(qword_ptr(rbx).gs(), 1))),
        ("lock prefix", code(|a| a.lock().add(qword_ptr(rbx), rax))),
        ("address size prefix", vec![0x67, 0x48, 0x8d, 0x04, 0x08]), // lea rax, [eax+ecx]
        ("push r16", vec![0x66, 0x50]),
        ("pop r16", vec![0x66, 0x58]),
        ("push imm16", vec![0x66, 0x6a, 0x01]),
        ("mov from segment register", vec![0x8c, 0xd8]), // mov eax, ds
        ("invalid opcode", vec![0x06, 0x90]),            // push es: invalid in 64-bit mode
        ("retf", vec![0xcb]),
        ("int3", vec![0xcc]),
    ];
    for (name, c) in cases {
        let input = state(&[(RAX, 0x1234), (RBX, 0x9000)]);
        let out = run(&c, &input);
        assert_eq!(out.exit, abi::EXIT_UNSUPPORTED, "{name}");
        assert_eq!(out.rip, BASE, "{name}");
        assert_eq!(out.gpr, input.gpr, "{name}: registers unchanged");
        assert_eq!(out.rflags, input.rflags, "{name}: flags unchanged");
    }
}

#[test]
fn truncated_instruction_at_region_end_leaves_code() {
    // mov eax, 1; then the first 3 bytes of movabs rax, imm64.
    let c = [0xb8, 1, 0, 0, 0, 0x48, 0xb8, 0x11];
    let out = run(&c, &state(&[]));
    assert_eq!(out.exit, abi::EXIT_LEFT_CODE);
    assert_eq!(out.rip, BASE + 5);
    assert_eq!(out.gpr[RAX], 1);
    // The entry itself is cut off.
    let out = run(&[0x48, 0xb8, 0x11], &state(&[]));
    assert_eq!((out.exit, out.rip), (abi::EXIT_LEFT_CODE, BASE));
}

#[test]
fn branches_leaving_the_region() {
    // jmp outside
    let c = code(|a| a.jmp(0x5_0000u64));
    let out = run(&c, &state(&[]));
    assert_eq!((out.exit, out.rip), (abi::EXIT_LEFT_CODE, 0x5_0000));

    // jcc taken outside, not taken falls off the end
    let c = code(|a| {
        a.cmp(rcx, 0)?;
        a.je(0x5_0000u64)
    });
    let out = run(&c, &state(&[(RCX, 0)]));
    assert_eq!((out.exit, out.rip), (abi::EXIT_LEFT_CODE, 0x5_0000));
    let out = run(&c, &state(&[(RCX, 1)]));
    assert_eq!((out.exit, out.rip), (abi::EXIT_LEFT_CODE, end(&c)));

    // call outside: the return address is pushed first
    let c = code(|a| a.call(0x5_0000u64));
    let r = run_with(&c, &[BASE], &state(&[]), &[]);
    assert_eq!((r.out.exit, r.out.rip), (abi::EXIT_LEFT_CODE, 0x5_0000));
    assert_eq!(r.out.gpr[RSP], STACK - 8);
    let pushed = r.machine.read(STACK - 8, 8).unwrap();
    assert_eq!(u64::from_le_bytes(pushed.try_into().unwrap()), end(&c));

    // ret to an unknown address
    let c = code(|a| a.ret());
    let r = run_with(
        &c,
        &[BASE],
        &state(&[]),
        &[(STACK, &0xdead_0000u64.to_le_bytes())],
    );
    assert_eq!((r.out.exit, r.out.rip), (abi::EXIT_LEFT_CODE, 0xdead_0000));
    assert_eq!(r.out.gpr[RSP], STACK + 8);

    // jmp r64 to an address in the region that has no block
    let c = code(|a| {
        a.jmp(rcx)?;
        a.nop()
    });
    let out = run(&c, &state(&[(RCX, BASE + 2)]));
    assert_eq!((out.exit, out.rip), (abi::EXIT_LEFT_CODE, BASE + 2));
}

#[test]
fn ret_to_a_return_site_dispatches() {
    // call f; mov ebx, 1; ret(sentinel) / f: mov ecx, 2; ret
    let c = code(|a| {
        let mut f = a.create_label();
        a.call(f)?;
        a.mov(ebx, 1)?;
        a.ret()?;
        a.set_label(&mut f)?;
        a.mov(ecx, 2)?;
        a.ret()
    });
    let r = run_with(
        &c,
        &[BASE],
        &state(&[]),
        &[(STACK, &abi::SENTINEL_RET.to_le_bytes())],
    );
    assert_eq!(
        (r.out.exit, r.out.rip),
        (abi::EXIT_LEFT_CODE, abi::SENTINEL_RET)
    );
    assert_eq!((r.out.gpr[RBX], r.out.gpr[RCX]), (1, 2));
    assert_eq!(r.out.gpr[RSP], STACK + 8);
}

#[test]
fn multiple_entries_and_entry_errors() {
    let c = code(|a| {
        a.mov(eax, 1)?;
        a.ret()?;
        a.mov(eax, 2)?;
        a.ret()
    });
    let second = BASE + 6;
    let mut input = state(&[]);
    input.rip = second;
    let r = run_with(
        &c,
        &[BASE, second],
        &input,
        &[(STACK, &abi::SENTINEL_RET.to_le_bytes())],
    );
    assert_eq!(r.out.gpr[RAX], 2);
    assert_eq!(r.out.rip, abi::SENTINEL_RET);

    // Entering at an address without a block exits immediately.
    let mut input = state(&[(RAX, 9)]);
    input.rip = 0x7777;
    let out = run(&c, &input);
    assert_eq!(
        (out.exit, out.rip, out.gpr[RAX]),
        (abi::EXIT_LEFT_CODE, 0x7777, 9)
    );

    let err = x64wasm::translate(
        CodeRegion {
            base: BASE,
            bytes: &c,
        },
        &[BASE + 100],
        &Options::default(),
    )
    .unwrap_err();
    assert!(matches!(err, Error::EntryOutsideRegion(a) if a == BASE + 100));
}

#[test]
fn partial_register_writes() {
    let all = u64::MAX;
    let c = code(|a| a.mov(ah, 0x12));
    assert_eq!(
        run(&c, &state(&[(RAX, all)])).gpr[RAX],
        0xffff_ffff_ffff_12ff
    );
    let c = code(|a| a.mov(cl, dh));
    let out = run(&c, &state(&[(RCX, all), (RDX, 0xab00)]));
    assert_eq!(out.gpr[RCX], 0xffff_ffff_ffff_ffab);
    let c = code(|a| a.mov(sil, al));
    let out = run(&c, &state(&[(6, all), (RAX, 0x42)]));
    assert_eq!(out.gpr[6], 0xffff_ffff_ffff_ff42);
    let c = code(|a| a.mov(r9w, 0x1234));
    assert_eq!(run(&c, &state(&[(9, all)])).gpr[9], 0xffff_ffff_ffff_1234);
    let c = code(|a| a.mov(r10d, r11d));
    let out = run(&c, &state(&[(10, all), (11, 0xaaaa_bbbb_cccc_dddd)]));
    assert_eq!(out.gpr[10], 0xcccc_dddd);
    let c = code(|a| a.add(r12d, 0));
    assert_eq!(
        run(&c, &state(&[(12, all)])).gpr[12],
        0xffff_ffff,
        "32-bit add zero-extends"
    );
    let c = code(|a| a.add(bh, 1));
    let out = run(&c, &state(&[(RBX, 0x1122_3344_5566_ff88)]));
    assert_eq!(out.gpr[RBX], 0x1122_3344_5566_0088);
    let st = out.rflags & flags::STATUS;
    assert_eq!(st, flags::CF | flags::ZF | flags::AF | flags::PF);
    // cmp writes nothing
    let c = code(|a| a.cmp(rax, 1));
    assert_eq!(run(&c, &state(&[(RAX, 5)])).gpr[RAX], 5);
}

#[test]
fn arithmetic_flag_boundaries() {
    let st = |c: &[u8], regs: &[(usize, u64)]| run(c, &state(regs)).rflags & flags::STATUS;
    use flags::*;
    // add: signed overflow without carry, carry without overflow
    let c = code(|a| a.add(eax, ecx));
    assert_eq!(st(&c, &[(RAX, 0x7fff_ffff), (RCX, 1)]), OF | SF | AF | PF);
    assert_eq!(st(&c, &[(RAX, 0xffff_ffff), (RCX, 1)]), CF | ZF | AF | PF);
    assert_eq!(
        st(&c, &[(RAX, 0x8000_0000), (RCX, 0x8000_0000)]),
        CF | OF | ZF | PF
    );
    // sub: borrow, overflow
    let c = code(|a| a.sub(rax, rcx));
    assert_eq!(st(&c, &[(RAX, 0), (RCX, 1)]), CF | SF | AF | PF);
    assert_eq!(st(&c, &[(RAX, 1 << 63), (RCX, 1)]), OF | AF | PF);
    assert_eq!(st(&c, &[(RAX, 5), (RCX, 5)]), ZF | PF);
    // 8-bit: 0x7f + 1 overflows; parity of the low byte only
    let c = code(|a| a.add(al, 1));
    assert_eq!(st(&c, &[(RAX, 0x7f)]), OF | SF | AF);
    let c = code(|a| a.add(ax, 0x100));
    assert_eq!(st(&c, &[(RAX, 0x0003)]), PF);
    // 16-bit cmp
    let c = code(|a| a.cmp(cx, dx));
    assert_eq!(st(&c, &[(RCX, 0x8000), (RDX, 1)]), OF | AF | PF);
    // imm8 sign extension: add rax, -1 on 0 sets SF, no carry
    let c = code(|a| a.add(rax, -1));
    assert_eq!(st(&c, &[(RAX, 0)]), SF | PF);
    assert_eq!(st(&c, &[(RAX, 1)]), CF | ZF | AF | PF);
}

#[test]
fn rflags_control_bits_are_preserved() {
    let c = code(|a| a.add(rax, rcx));
    let mut input = state(&[(RAX, 1), (RCX, 2)]);
    // ID, AC, DF, IF, bit 1, and stale status flags that the add overwrites.
    input.rflags = (1 << 21) | (1 << 18) | flags::DF | flags::IF | 0x2 | flags::STATUS;
    let out = run(&c, &input);
    assert_eq!(out.rflags & !flags::STATUS, input.rflags & !flags::STATUS);
    assert_eq!(out.rflags & flags::STATUS, flags::PF);
    // No flag writer: the entry status flags come back unchanged.
    let c = code(|a| a.mov(rax, rcx));
    assert_eq!(run(&c, &input).rflags, input.rflags);
}

#[test]
fn push_pop_rsp_semantics() {
    // push rsp stores the old value
    let c = code(|a| a.push(rsp));
    let r = run_with(&c, &[BASE], &state(&[]), &[]);
    assert_eq!(r.out.gpr[RSP], STACK - 8);
    assert_eq!(r.machine.read(STACK - 8, 8).unwrap(), STACK.to_le_bytes());

    // pop rsp loads rsp from the stack (the increment is lost)
    let c = code(|a| a.pop(rsp));
    let out = run_with(
        &c,
        &[BASE],
        &state(&[]),
        &[(STACK, &0x4_0000u64.to_le_bytes())],
    )
    .out;
    assert_eq!(out.gpr[RSP], 0x4_0000);

    // pop [rsp]: the address uses rsp after the increment
    let c = code(|a| a.pop(qword_ptr(rsp)));
    let r = run_with(
        &c,
        &[BASE],
        &state(&[]),
        &[(STACK, &0x1111u64.to_le_bytes())],
    );
    assert_eq!(r.out.gpr[RSP], STACK + 8);
    assert_eq!(
        r.machine.read(STACK + 8, 8).unwrap(),
        0x1111u64.to_le_bytes()
    );

    // push [rsp]: the address uses rsp before the decrement
    let c = code(|a| a.push(qword_ptr(rsp)));
    let r = run_with(
        &c,
        &[BASE],
        &state(&[]),
        &[(STACK, &0x2222u64.to_le_bytes())],
    );
    assert_eq!(
        r.machine.read(STACK - 8, 8).unwrap(),
        0x2222u64.to_le_bytes()
    );

    // push imm8 / imm32 are sign-extended to 64 bits
    let c = code(|a| {
        a.push(-2)?;
        a.push(-0x7000_0000)
    });
    let r = run_with(&c, &[BASE], &state(&[]), &[]);
    assert_eq!(r.machine.read(STACK - 8, 8).unwrap(), (-2i64).to_le_bytes());
    assert_eq!(
        r.machine.read(STACK - 16, 8).unwrap(),
        (-0x7000_0000i64).to_le_bytes()
    );

    // call rsp-relative memory: the target is read before the push
    let c = code(|a| {
        a.call(qword_ptr(rsp))?;
        a.nop()
    });
    let r = run_with(
        &c,
        &[BASE],
        &state(&[]),
        &[(STACK, &0x9_9990u64.to_le_bytes())],
    );
    assert_eq!((r.out.exit, r.out.rip), (abi::EXIT_LEFT_CODE, 0x9_9990));
    assert_eq!(
        r.machine.read(STACK - 8, 8).unwrap(),
        (BASE + 3).to_le_bytes()
    );

    // ret imm16 releases the extra bytes
    let c = code(|a| a.ret_1(0x20));
    let out = run_with(
        &c,
        &[BASE],
        &state(&[]),
        &[(STACK, &0x7_0000u64.to_le_bytes())],
    )
    .out;
    assert_eq!((out.rip, out.gpr[RSP]), (0x7_0000, STACK + 8 + 0x20));
}

#[test]
fn memory_addressing() {
    // [base + index*scale + disp] with a negative displacement, and RIP-relative.
    let c = code(|a| {
        let mut data = a.create_label();
        a.mov(rax, qword_ptr(rbx + rcx * 8 - 0x10))?;
        a.mov(rdx, qword_ptr(data))?;
        a.lea(rsi, ptr(data))?;
        a.mov(dword_ptr(rbx + rcx * 4 + 0x1000), 0xcafe_f00du32)?;
        a.mov(qword_ptr(rbx), -1)?;
        a.jmp(0x5_0000u64)?;
        a.set_label(&mut data)?;
        a.dq(&[0x0102_0304_0506_0708])
    });
    // The data qword is the last 8 bytes.
    let data_addr = end(&c) - 8;
    let r = run_with(
        &c,
        &[BASE],
        &state(&[(RBX, 0x2_0000), (RCX, 3)]),
        &[(0x2_0000 + 24 - 16, &0x55u64.to_le_bytes())],
    );
    assert_eq!(r.out.gpr[RAX], 0x55);
    assert_eq!(r.out.gpr[RDX], 0x0102_0304_0506_0708);
    assert_eq!(r.out.gpr[6], data_addr);
    assert_eq!(
        r.machine.read(0x2_0000 + 12 + 0x1000, 4).unwrap(),
        0xcafe_f00du32.to_le_bytes()
    );
    assert_eq!(r.machine.read(0x2_0000, 8).unwrap(), u64::MAX.to_le_bytes());
}
