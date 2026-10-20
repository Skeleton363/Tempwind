//! Phase 0 measurements (ignored by default; numbers only mean something in
//! a release build):
//!
//! ```text
//! cargo test -p testkit --release --test spike_bench -- --ignored --nocapture
//! ```
//!
//! - translation throughput (guest instructions per second) and wasm bytes
//!   per guest instruction on a large generated region;
//! - sum_loop with n = 10^8 in wasmtime vs the same machine code run natively.

use std::time::{Duration, Instant};

use iced_x86::code_asm::*;
use testkit::host::Host;
use testkit::spike;
use x64wasm::{CodeRegion, Options};

/// A large region of straight-line code with forward branches, calls and
/// returns, using every supported instruction family.
fn generated_region(base: u64, chunks: usize) -> Vec<u8> {
    let mut a = CodeAssembler::new(64).unwrap();
    let regs = [rax, rcx, rdx, rbx, rsi, rdi, r8, r9, r10, r11, r12, r13, r14, r15];
    let mut f = a.create_label();
    for i in 0..chunks {
        let r = regs[i % regs.len()];
        let s = regs[(i * 7 + 3) % regs.len()];
        let mut skip = a.create_label();
        a.mov(r, s).unwrap();
        a.add(r, (i as i32) & 0x7f).unwrap();
        a.sub(r, qword_ptr(rbx + s * 8 + 0x40)).unwrap();
        a.lea(s, ptr(r + s * 4 + 0x1234)).unwrap();
        a.mov(qword_ptr(rbp - 0x20), r).unwrap();
        a.cmp(r, s).unwrap();
        a.jae(skip).unwrap();
        a.push(r).unwrap();
        a.add(dword_ptr(rsp), 3).unwrap();
        a.pop(s).unwrap();
        a.set_label(&mut skip).unwrap();
        if i % 16 == 15 {
            a.call(f).unwrap();
        }
    }
    a.ret().unwrap();
    a.set_label(&mut f).unwrap();
    a.add(rax, 1).unwrap();
    a.ret().unwrap();
    a.assemble(base).unwrap()
}

fn best_of<T>(runs: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut best = Duration::MAX;
    let mut out = None;
    for _ in 0..runs {
        let start = Instant::now();
        let v = f();
        best = best.min(start.elapsed());
        out = Some(v);
    }
    (best, out.unwrap())
}

#[test]
#[ignore = "measurement; run with --release --ignored --nocapture"]
fn translation_throughput() {
    let base = 0x1_4000_1000;
    let code = generated_region(base, 20_000);
    let region = CodeRegion { base, bytes: &code };
    let opts = Options::default();
    let (with_dump, t) = best_of(5, || x64wasm::translate(region, &[base], &opts).unwrap());
    let (no_dump, t2) =
        best_of(5, || x64wasm::translate_without_dump(region, &[base], &opts).unwrap());
    assert_eq!(t.wasm, t2.wasm);
    let n = t.guest_instructions as f64;
    println!(
        "region: {} guest instructions, {} code bytes, {} blocks",
        t.guest_instructions,
        code.len(),
        t.blocks.len()
    );
    println!(
        "translate (with IR dump):    {:>8.2} ms  {:>6.2} M insn/s",
        with_dump.as_secs_f64() * 1e3,
        n / with_dump.as_secs_f64() / 1e6
    );
    println!(
        "translate_without_dump:      {:>8.2} ms  {:>6.2} M insn/s",
        no_dump.as_secs_f64() * 1e3,
        n / no_dump.as_secs_f64() / 1e6
    );
    println!(
        "wasm: {} bytes, {:.1} bytes per guest instruction ({:.2} per guest code byte)",
        t.wasm.len(),
        t.wasm.len() as f64 / n,
        t.wasm.len() as f64 / code.len() as f64
    );
    let host = Host::new().unwrap();
    let (compile, _) = best_of(1, || host.compile(&t.wasm).unwrap());
    println!(
        "wasmtime (cranelift) compile: {:.1} ms",
        compile.as_secs_f64() * 1e3
    );
    for case in spike::cases().unwrap() {
        let t = case.translate(&opts).unwrap();
        println!(
            "{:>14}: {:>3} insns, {:>5} wasm bytes ({:.0} B/insn incl. fixed prologue/epilogue/helpers)",
            case.name,
            t.guest_instructions,
            t.wasm.len(),
            t.wasm.len() as f64 / t.guest_instructions as f64
        );
    }
}

#[cfg(all(target_arch = "x86_64", unix))]
#[test]
#[ignore = "measurement; run with --release --ignored --nocapture"]
fn sum_loop_wasm_vs_native() {
    use testkit::spike::native::ExecPage;
    let n: u64 = 100_000_000;
    let case = spike::sum_loop(n).unwrap();
    let t = case.translate(&Options::default()).unwrap();
    let host = Host::new().unwrap();
    let module = host.compile(&t.wasm).unwrap();
    let (wasm_time, ()) = best_of(3, || {
        let mut machine = host.machine(case.memory_bytes).unwrap();
        for (addr, bytes) in case.all_init_mem() {
            machine.write(addr, &bytes).unwrap();
        }
        let out = machine
            .run_state(&module, case.state_addr, &case.init_state())
            .unwrap();
        case.check(&out, &machine).unwrap();
    });

    let page = ExecPage::new(&case.code).unwrap();
    let expected = n * (n + 1) / 2;
    let (native_time, sum) = best_of(3, || {
        // SAFETY: sum_loop only reads rcx and returns with ret.
        unsafe { page.call_win64(std::hint::black_box(n)) }
    });
    assert_eq!(sum, expected);
    println!(
        "sum_loop n=1e8: wasmtime {:.1} ms, native {:.1} ms, ratio {:.2}x ({:.2} ns/iteration wasm, {:.2} native)",
        wasm_time.as_secs_f64() * 1e3,
        native_time.as_secs_f64() * 1e3,
        wasm_time.as_secs_f64() / native_time.as_secs_f64(),
        wasm_time.as_secs_f64() * 1e9 / n as f64,
        native_time.as_secs_f64() * 1e9 / n as f64
    );
}
