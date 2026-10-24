//! ABI v3 linking in the wasmtime host (`docs/design/abi.md`): modules that
//! call each other through the shared table and lookup table, lookup misses
//! resolved by linking more code, and the host-call protocol.

use iced_x86::BlockEncoderOptions;
use iced_x86::code_asm::*;
use testkit::host::{Host, HostCall, Machine, Module};
use x64wasm::abi::{self, CpuState};
use x64wasm::{CodeRegion, Options};

const MEMORY: u64 = 0x40_0000;
const A: u64 = 0x10_0000;
const A_G: u64 = 0x10_0100;
const B: u64 = 0x20_0000;
const B_H: u64 = 0x20_0100;
const STACK_TOP: u64 = 0x2f_f000;
const STATE: u64 = 0x30_0000;
/// A host function's guest address (an import thunk in the runtime).
const THUNK: u64 = 0x38_0000;
const RAX: usize = 0;
const RCX: usize = 1;
const RSP: usize = 4;

/// Emits a function body.
type Body<'a> = &'a dyn Fn(&mut CodeAssembler);

/// Assembles functions at `(address, body)`; the gaps are `int3`.
fn code(base: u64, funcs: &[(u64, Body<'_>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for &(addr, body) in funcs {
        let at = (addr - base) as usize;
        assert!(at >= out.len(), "functions overlap");
        out.resize(at, 0xcc);
        let mut a = CodeAssembler::new(64).unwrap();
        body(&mut a);
        out.extend(a.assemble(addr).unwrap());
    }
    out
}

fn translate(host: &Host, base: u64, bytes: &[u8], entries: &[u64]) -> (Module, Vec<u8>) {
    let t = x64wasm::translate(CodeRegion { base, bytes }, entries, &Options::default()).unwrap();
    (host.compile(&t.wasm).unwrap(), bytes.to_vec())
}

/// Module A: `main` calls B's `f` directly, then B's `h` indirectly; `g`
/// adds 100.
fn module_a(host: &Host) -> (Module, Vec<u8>) {
    let bytes = code(
        A,
        &[
            (A, &|a| {
                a.mov(eax, 1).unwrap();
                a.call(B).unwrap();
                a.mov(rcx, B_H).unwrap();
                a.call(rcx).unwrap();
                a.ret().unwrap();
            }),
            (A_G, &|a| {
                a.add(rax, 100).unwrap();
                a.ret().unwrap();
            }),
        ],
    );
    translate(host, A, &bytes, &[A, A_G])
}

/// Module B: `f` adds 10 and calls A's `g` directly; `h` adds 1000, calls
/// A's `g` indirectly, then adds 10000.
fn module_b(host: &Host) -> (Module, Vec<u8>) {
    let bytes = code(
        B,
        &[
            (B, &|a| {
                a.add(rax, 10).unwrap();
                a.call(A_G).unwrap();
                a.ret().unwrap();
            }),
            (B_H, &|a| {
                a.add(rax, 1000).unwrap();
                a.mov(rdx, A_G).unwrap();
                a.call(rdx).unwrap();
                a.add(rax, 10000).unwrap();
                a.ret().unwrap();
            }),
        ],
    );
    translate(host, B, &bytes, &[B, B_H])
}

/// 1 + 10 + 100 (f -> g) + 1000 + 100 + 10000 (h -> g).
const AB_RESULT: u64 = 11_211;

/// A machine with `code` written and a state that calls `entry` with the
/// sentinel return address on the stack.
fn machine(host: &Host, code: &[(u64, &[u8])], entry: u64) -> (Machine, CpuState) {
    let mut m = host.machine(MEMORY).unwrap();
    for &(addr, bytes) in code {
        m.write(addr, bytes).unwrap();
    }
    let stack = STACK_TOP - 8;
    m.write(stack, &abi::SENTINEL_RET.to_le_bytes()).unwrap();
    let mut state = CpuState {
        rip: entry,
        rflags: 0x202,
        ..CpuState::default()
    };
    state.gpr[RSP] = stack;
    (m, state)
}

fn enter(m: &mut Machine, module: &Module, state: &CpuState) -> CpuState {
    m.write_state(STATE, state).unwrap();
    let exit = m.enter(module, STATE).unwrap();
    let out = m.read_state(STATE).unwrap();
    assert_eq!(out.exit, exit);
    out
}

#[test]
fn two_modules_call_each_other() {
    let host = Host::new().unwrap();
    let (a, a_code) = module_a(&host);
    let (b, b_code) = module_b(&host);
    let (mut m, state) = machine(&host, &[(A, &a_code), (B, &b_code)], A);
    let base_a = m.link(&a).unwrap();
    let base_b = m.link(&b).unwrap();
    assert!(
        base_b >= base_a + 2,
        "A has two chunks, B starts after them"
    );
    // Either module's `run` behaves the same.
    for runner in [&a, &b] {
        let out = enter(&mut m, runner, &state);
        assert_eq!(out.exit, abi::EXIT_LEFT_CODE);
        assert_eq!(out.rip, abi::SENTINEL_RET);
        assert_eq!(out.gpr[RAX], AB_RESULT);
        assert_eq!(out.gpr[RSP], STACK_TOP);
    }
}

#[test]
fn a_miss_exits_and_linking_the_module_resumes() {
    let host = Host::new().unwrap();
    let (a, a_code) = module_a(&host);
    let (b, b_code) = module_b(&host);
    let (mut m, state) = machine(&host, &[(A, &a_code), (B, &b_code)], A);
    m.link(&a).unwrap();
    // B is not linked: A's direct call to B misses.
    let out = enter(&mut m, &a, &state);
    assert_eq!(out.exit, abi::EXIT_LEFT_CODE);
    assert_eq!(out.rip, B);
    assert_eq!(out.gpr[RAX], 1);
    assert_eq!(out.gpr[RSP], STACK_TOP - 16, "the return address is pushed");
    // Link B and resume from the exit state.
    m.link(&b).unwrap();
    let out = enter(&mut m, &a, &CpuState { exit: 0, ..out });
    assert_eq!(out.exit, abi::EXIT_LEFT_CODE);
    assert_eq!(out.rip, abi::SENTINEL_RET);
    assert_eq!(out.gpr[RAX], AB_RESULT);
    assert_eq!(out.gpr[RSP], STACK_TOP);
}

#[test]
fn the_first_translation_of_an_address_wins() {
    let host = Host::new().unwrap();
    let one = code(A, &[(A, &|a| a.mov(eax, 1).and_then(|_| a.ret()).unwrap())]);
    let two = code(A, &[(A, &|a| a.mov(eax, 2).and_then(|_| a.ret()).unwrap())]);
    let (m1, _) = translate(&host, A, &one, &[A]);
    let (m2, _) = translate(&host, A, &two, &[A]);
    let (mut m, state) = machine(&host, &[(A, &one)], A);
    m.link(&m1).unwrap();
    m.link(&m2).unwrap();
    assert_eq!(enter(&mut m, &m2, &state).gpr[RAX], 1);
    // `run` relinks just the module it is given.
    let out = m.run_state(&m2, STATE, &state).unwrap();
    assert_eq!(out.gpr[RAX], 2);
    let out = m.run_state(&m1, STATE, &state).unwrap();
    assert_eq!(out.gpr[RAX], 1);
}

#[test]
fn the_lookup_table_grows() {
    // 1500 one-byte functions (`ret`), every one an entry: the lookup table
    // rehashes several times while the module is linked.
    let host = Host::new().unwrap();
    let n = 1500u64;
    let bytes = vec![0xc3u8; n as usize];
    let entries: Vec<u64> = (0..n).map(|i| A + i).collect();
    let (module, _) = translate(&host, A, &bytes, &entries);
    let (mut m, state) = machine(&host, &[(A, &bytes)], A);
    m.link(&module).unwrap();
    assert!(m.lookup_slots() >= 2 * n as usize);
    for i in [0, 1, 700, n - 1] {
        let out = enter(
            &mut m,
            &module,
            &CpuState {
                rip: A + i,
                ..state
            },
        );
        assert_eq!(out.exit, abi::EXIT_LEFT_CODE, "{i}");
        assert_eq!(out.rip, abi::SENTINEL_RET, "{i}");
    }
    let out = enter(
        &mut m,
        &module,
        &CpuState {
            rip: A + n,
            ..state
        },
    );
    assert_eq!(out.rip, A + n, "no block past the end");
}

/// `main`: `mov ecx, 7; call [rip+slot]; add rax, 1; ret`, with the slot
/// holding THUNK (an import's IAT slot); `alt`: `add rax, 1000; ret`.
fn host_call_module(host: &Host) -> (Module, Vec<u8>, u64) {
    let mut a = CodeAssembler::new(64).unwrap();
    let mut slot = a.create_label();
    let mut alt = a.create_label();
    a.mov(ecx, 7).unwrap();
    a.call(qword_ptr(slot)).unwrap();
    a.add(rax, 1).unwrap();
    a.ret().unwrap();
    a.set_label(&mut alt).unwrap();
    a.add(rax, 1000).unwrap();
    a.ret().unwrap();
    a.set_label(&mut slot).unwrap();
    a.dq(&[THUNK]).unwrap();
    let r = a
        .assemble_options(A, BlockEncoderOptions::RETURN_NEW_INSTRUCTION_OFFSETS)
        .unwrap();
    let alt = r.label_ip(&alt).unwrap();
    let bytes = r.inner.code_buffer;
    let (module, bytes) = translate(host, A, &bytes, &[A, alt]);
    (module, bytes, alt)
}

/// Links the host-call module and a host function at THUNK (entry id 5).
fn host_call_machine(
    f: impl Fn(&mut HostCall<'_>) -> anyhow::Result<i32> + Send + Sync + 'static,
) -> (Machine, Module, CpuState, u64) {
    let host = Host::new().unwrap();
    let (module, bytes, alt) = host_call_module(&host);
    let (mut m, state) = machine(&host, &[(A, &bytes)], A);
    m.link(&module).unwrap();
    let func = m.add_host_function(f).unwrap();
    m.insert(THUNK, func, 5).unwrap();
    (m, module, state, alt)
}

#[test]
fn host_function_returns_to_the_guest() {
    let (mut m, module, state, _) = host_call_machine(|call| {
        assert_eq!(call.entry, 5);
        let x = call.gpr(RCX);
        call.set_gpr(RAX, x * 6);
        call.ret()
    });
    let out = enter(&mut m, &module, &state);
    assert_eq!(out.exit, abi::EXIT_LEFT_CODE);
    assert_eq!(out.rip, abi::SENTINEL_RET);
    assert_eq!(out.gpr[RAX], 43);
    assert_eq!(out.gpr[RCX], 7);
    assert_eq!(out.gpr[RSP], STACK_TOP);
}

#[test]
fn host_function_ret_to_another_address_unwinds() {
    let (mut m, module, state, alt) = host_call_machine(|call| {
        call.set_gpr(RAX, 42);
        // Return somewhere else than the caller expects.
        let alt = call.read_u64(call.state + abi::gpr_offset(15))?;
        let stack = call.gpr(RSP);
        call.write_u64(stack, alt)?;
        let status = call.ret()?;
        assert_eq!(status, abi::STATUS_UNWIND);
        Ok(status)
    });
    // R15 carries alt's address to the host function.
    let mut state = state;
    state.gpr[15] = alt;
    let out = enter(&mut m, &module, &state);
    assert_eq!(out.exit, abi::EXIT_LEFT_CODE);
    assert_eq!(out.rip, abi::SENTINEL_RET);
    assert_eq!(out.gpr[RAX], 1042);
    assert_eq!(out.gpr[RSP], STACK_TOP);
}

#[test]
fn host_function_stops_execution() {
    let (mut m, module, state, _) = host_call_machine(|call| call.stop(THUNK));
    let out = enter(&mut m, &module, &state);
    assert_eq!(out.exit, abi::EXIT_HOST);
    assert_eq!(out.rip, THUNK);
    assert_eq!(out.gpr[RCX], 7);
    assert_eq!(
        out.gpr[RSP],
        STACK_TOP - 16,
        "the return address stays pushed"
    );
}
