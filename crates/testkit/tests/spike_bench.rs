//! Translator measurements in wasmtime (ignored by default; numbers only
//! mean something in a release build, on an idle machine):
//!
//! ```text
//! cargo test -p testkit --release --test spike_bench -- --ignored --nocapture --test-threads=1
//! ```
//!
//! - translation throughput, wasm size and Cranelift compile time of Phase
//!   0's 201k-instruction region, for several `max_function_insns`;
//! - sum_loop with n = 10^8, and the call-heavy fib_rec(30) and byte_calls
//!   (a leaf call per byte of 1 MiB), in wasmtime vs the same machine code
//!   run natively;
//! - the deepest guest recursion wasmtime survives without the call-depth
//!   guard.
//!
//! The V8 side is `tests/bench/translator/bench.mjs`.

use std::time::Duration;

use testkit::host::Host;
use testkit::spike::bench::{self, REGION_BASE, REGION_PARTS, best_of};
use testkit::spike::{self, Case};
use x64wasm::{CodeRegion, Options};

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

#[test]
#[ignore = "measurement; run with --release --ignored --nocapture"]
fn translation_throughput() {
    let (code, _) = bench::generated_region(REGION_BASE, REGION_PARTS);
    let region = CodeRegion {
        base: REGION_BASE,
        bytes: &code,
    };
    let opts = Options::default();
    let (with_dump, t) = best_of(5, || {
        x64wasm::translate(region, &[REGION_BASE], &opts).unwrap()
    });
    let (no_dump, t2) = best_of(5, || {
        x64wasm::translate_without_dump(region, &[REGION_BASE], &opts).unwrap()
    });
    assert_eq!(t.wasm, t2.wasm);
    let (lift, _) = best_of(5, || x64wasm::lift(region, &[REGION_BASE]).unwrap());
    let n = t.guest_instructions as f64;
    println!(
        "region: {} guest instructions, {} code bytes, {} blocks",
        t.guest_instructions,
        code.len(),
        t.blocks.len()
    );
    println!(
        "translate (with IR dump):    {:>8.2} ms  {:>6.2} M insn/s",
        ms(with_dump),
        n / with_dump.as_secs_f64() / 1e6
    );
    println!(
        "translate_without_dump:      {:>8.2} ms  {:>6.2} M insn/s",
        ms(no_dump),
        n / no_dump.as_secs_f64() / 1e6
    );
    println!(
        "  of which lift (decode, IR with text, flag liveness): {:>8.2} ms",
        ms(lift)
    );
    let host = Host::new().unwrap();
    for max in [500, 1000, 2000, 5000, 10_000, 20_000] {
        let opts = Options {
            max_function_insns: max,
            max_chunk_insns: 10 * max,
            ..Options::default()
        };
        let t = x64wasm::translate_without_dump(region, &[REGION_BASE], &opts).unwrap();
        let chunks: usize = t.functions.iter().map(|f| f.chunks.len()).sum();
        let (compile, _) = best_of(3, || host.compile(&t.wasm).unwrap());
        println!(
            "max_function_insns {max:>5}: {} functions, {chunks:>4} chunks, wasm {} bytes ({:.1} B/insn, {:.2} B/code byte), cranelift compile {:.0} ms",
            t.functions.len(),
            t.wasm.len(),
            t.wasm.len() as f64 / n,
            t.wasm.len() as f64 / code.len() as f64,
            ms(compile)
        );
    }
    // Time to the first `run` of the default module (compile, instantiate,
    // run `f`).
    let case = bench::region_case(REGION_PARTS).unwrap();
    let t = case.translate(&Options::default()).unwrap();
    let (first, ()) = best_of(3, || {
        let (out, machine) = case.run(&host, &t.wasm).unwrap();
        case.check(&out, &machine).unwrap();
    });
    println!("region, default options: first run (compile + instantiate + run) {:.0} ms", ms(first));
    for case in spike::cases().unwrap() {
        let t = case.translate(&Options::default()).unwrap();
        println!(
            "{:>14}: {:>4} insns, {:>6} wasm bytes ({:.0} B/insn incl. trampoline/lookup/flags)",
            case.name,
            t.guest_instructions,
            t.wasm.len(),
            t.wasm.len() as f64 / t.guest_instructions as f64
        );
    }
}

/// Times `case` translated (run only, compiled once, best of `runs`) and
/// natively, and prints the ratio.
#[cfg(all(target_arch = "x86_64", unix))]
fn wasm_vs_native(label: &str, case: &Case, runs: usize) {
    let t = case.translate(&Options::default()).unwrap();
    let host = Host::new().unwrap();
    let module = host.compile(&t.wasm).unwrap();
    let mut machine = host.machine(case.memory_bytes).unwrap();
    let mut wasm_time = Duration::MAX;
    for _ in 0..runs {
        for (addr, bytes) in case.all_init_mem() {
            machine.write(addr, &bytes).unwrap();
        }
        machine.write_state(case.state_addr, &case.init_state()).unwrap();
        let start = std::time::Instant::now();
        machine.run(&module, case.state_addr).unwrap();
        wasm_time = wasm_time.min(start.elapsed());
        let out = machine.read_state(case.state_addr).unwrap();
        case.check(&out, &machine).unwrap();
    }
    let native_time = bench::native_time(case, runs).unwrap();
    println!(
        "{label}: wasmtime {:.2} ms, native {:.2} ms, ratio {:.2}x",
        ms(wasm_time),
        ms(native_time),
        wasm_time.as_secs_f64() / native_time.as_secs_f64()
    );
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
        ms(wasm_time),
        ms(native_time),
        wasm_time.as_secs_f64() / native_time.as_secs_f64(),
        wasm_time.as_secs_f64() * 1e9 / n as f64,
        native_time.as_secs_f64() * 1e9 / n as f64
    );
}

#[cfg(all(target_arch = "x86_64", unix))]
#[test]
#[ignore = "measurement; run with --release --ignored --nocapture"]
fn call_heavy_wasm_vs_native() {
    wasm_vs_native("fib_rec(30)", &spike::fib_rec(30).unwrap(), 5);
    wasm_vs_native(
        "byte_calls 1 MiB",
        &bench::byte_calls(1 << 20).unwrap(),
        5,
    );
}

/// Deepest `n` in `lo..hi` for which `ok(n)`, assuming it is monotonic.
fn deepest(mut lo: u64, mut hi: u64, mut ok: impl FnMut(u64) -> bool) -> u64 {
    assert!(ok(lo));
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if ok(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

#[test]
#[ignore = "measurement; run with --release --ignored --nocapture"]
fn deepest_recursion_without_guard() {
    let host = Host::new().unwrap();
    let no_guard = Options {
        call_depth_limit: None,
        ..Options::default()
    };
    for (label, make) in [
        ("deep_rec", &(|n| bench::deep_probe(n).unwrap()) as &dyn Fn(u64) -> Case),
        ("deep_fat (2000-insn frame)", &|n| bench::deep_fat(n, 2000).unwrap()),
    ] {
        let probe = make(1);
        let t = probe.translate(&no_guard).unwrap();
        let module = host.compile(&t.wasm).unwrap();
        let depth = deepest(1, 4_000_000, |n| {
            let case = make(n);
            let mut machine = host.machine(case.memory_bytes).unwrap();
            for (addr, bytes) in case.all_init_mem() {
                machine.write(addr, &bytes).unwrap();
            }
            match machine.run_state(&module, case.state_addr, &case.init_state()) {
                Ok(out) => {
                    case.check(&out, &machine).unwrap();
                    true
                }
                Err(_) => false,
            }
        });
        println!("{label}: wasmtime (default 512 KiB wasm stack) survives depth {depth}");
    }
}
