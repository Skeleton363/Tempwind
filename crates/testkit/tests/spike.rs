//! The spike programs, translated and run in wasmtime with fixed expected
//! results (checked natively too on x86-64 Unix), plus the manifest
//! `testkit spike` writes for the JS hosts.

use testkit::host::Host;
use testkit::spike::{self, Case};
use x64wasm::Options;
use x64wasm::abi;
use x64wasm::ir::Term;

fn run_case_with(case: &Case, opts: &Options) {
    let t = case.translate(opts).unwrap();
    let host = Host::new().unwrap();
    let (out, machine) = case.run(&host, &t.wasm).unwrap();
    case.check(&out, &machine)
        .unwrap_or_else(|e| panic!("{e:#} with {opts:?}"));
}

fn run_case(case: &Case) {
    run_case_with(case, &Options::default());
}

/// Option sets under which every case must still pass: correctness must not
/// depend on where chunks are cut or how deep calls nest on the wasm stack.
fn option_sweep() -> Vec<Options> {
    let o = |max_function_insns, max_chunk_insns, call_depth_limit| Options {
        max_function_insns,
        max_chunk_insns,
        call_depth_limit,
        ..Options::default()
    };
    vec![
        Options::default(),
        // Small chunks, loops kept whole.
        o(8, 1000, Some(1000)),
        // Every block its own chunk, loops cut too.
        o(1, 1, Some(1000)),
        // Every call unwinds to the trampoline.
        o(Options::DEFAULT_MAX_FUNCTION_INSNS, 20_000, Some(0)),
        o(1, 1, Some(0)),
        o(30, 60, Some(3)),
        // The reference sync: every value at every boundary.
        Options {
            sync_all: true,
            ..Options::default()
        },
        Options {
            sync_all: true,
            ..o(1, 1, Some(0))
        },
    ]
}

#[test]
fn sum_loop() {
    let case = spike::sum_loop(100).unwrap();
    assert_eq!(case.expect_regs[0], (spike::reg("rax"), 0x13ba));
    run_case(&case);
    run_case(&spike::sum_loop(0).unwrap());
    run_case(&spike::sum_loop(1).unwrap());
    run_case(&spike::sum_loop(1_000_000).unwrap());
}

#[test]
fn fib_recursive() {
    for n in [0, 1, 2, 10, 20] {
        run_case(&spike::fib_recursive(n).unwrap());
    }
    assert_eq!(spike::fib(20), 6765);
}

#[test]
fn array() {
    for n in [0, 1, 64, 1000] {
        run_case(&spike::array(n).unwrap());
    }
}

#[test]
fn high_address() {
    let case = spike::high_address().unwrap();
    assert_eq!(case.code_base, 0x1_4000_1000);
    assert!(case.memory_bytes > 1 << 32);
    let rsp = case.init_regs.iter().find(|r| r.0 == 4).unwrap().1;
    assert!(rsp > 1 << 32, "stack above 4 GiB");
    run_case(&case);
}

#[test]
fn fib_rec() {
    for n in [0, 1, 2, 10, 20] {
        run_case(&spike::fib_rec(n).unwrap());
    }
}

#[test]
fn deep_rec() {
    run_case(&spike::deep_rec(10).unwrap());
    run_case(&spike::deep_rec(100_000).unwrap());
}

#[test]
fn tail_rec() {
    for n in [0, 1, 2, 7, 100_000] {
        run_case(&spike::tail_rec(n).unwrap());
    }
}

#[test]
fn ret_tricks() {
    run_case(&spike::ret_tricks().unwrap());
}

#[test]
fn call_table() {
    run_case(&spike::call_table().unwrap());
}

#[test]
fn shared_tail() {
    let case = spike::shared_tail().unwrap();
    // main, fa, fb: the epilogue belongs to fa, and fb reaches it by a tail
    // transfer.
    let t = case.translate(&Options::default()).unwrap();
    assert_eq!(t.functions.len(), 3);
    let epilogue = *t.blocks.last().unwrap();
    assert!(t.functions[1].chunks[0].blocks.contains(&epilogue));
    assert!(!t.functions[2].chunks[0].blocks.contains(&epilogue));
    run_case(&case);
}

#[test]
fn longjmp_like() {
    run_case(&spike::longjmp_like().unwrap());
}

#[test]
fn loop_branch() {
    run_case(&spike::loop_branch(100).unwrap());
}

#[test]
fn merge_call() {
    for n in [0, 4, 5, 9] {
        run_case(&spike::merge_call(n).unwrap());
    }
}

#[test]
fn loop_calls() {
    for n in [0, 1, 2, 100] {
        run_case(&spike::loop_calls(n).unwrap());
    }
}

#[test]
fn clobber_calls() {
    run_case(&spike::clobber_calls().unwrap());
}

#[test]
fn flags_across_call() {
    for (rcx, rdx) in [(3, 9), (9, 3), (5, 5), (0, u64::MAX)] {
        run_case(&spike::flags_across_call(rcx, rdx).unwrap());
    }
}

#[test]
fn unwind_dirty() {
    for n in [0, 1, 10, 2000] {
        run_case(&spike::unwind_dirty(n).unwrap());
    }
}

/// Runs a case built with counters; returns the counter block.
fn counted(case: &Case, opts: &Options) -> [u64; 12] {
    let t = case.translate(opts).unwrap();
    let host = Host::new().unwrap();
    let (out, machine) = case.run(&host, &t.wasm).unwrap();
    case.check(&out, &machine).unwrap();
    machine.counters().unwrap()
}

#[test]
fn counters_are_deterministic_and_validate() {
    let opts = Options {
        counters: true,
        ..Options::default()
    };
    let case = spike::loop_branch(100).unwrap();
    // Compiling in wasmtime validates the module.
    let c = counted(&case, &opts);
    assert_eq!(c, counted(&case, &opts), "deterministic");
    // One entry at the function's own entry block; the back edge of the
    // 100 iterations goes through `$dispatch` 99 times.
    assert_eq!((c[0], c[1]), (1, 0), "entries");
    assert_eq!(c[2], 99, "dispatches");
    assert_eq!((c[4], c[5], c[6]), (0, 0, 0), "no calls");
    assert_eq!(c[7], 1, "the final ret unwinds to the sentinel");
    // Entry loads what is live into the entry block (RCX, RSP); the ret
    // stores what the function wrote: RAX, RCX, RSP and the flag record
    // (four locals).
    assert_eq!((c[8], c[9]), (7, 2), "stores and loads of the state");
    assert_eq!((c[10], c[11]), (1, 0), "only the ret loads from memory");
    let all = counted(
        &case,
        &Options {
            sync_all: true,
            ..opts.clone()
        },
    );
    assert_eq!(
        (all[8], all[9]),
        (20, 20),
        "sync_all: one load and one store of the whole state"
    );
}

/// Values moved per call (stores plus loads, in locals) under `opts`.
fn moved_per_call(case: &Case, opts: &Options) -> f64 {
    let c = counted(case, opts);
    (c[8] + c[9]) as f64 / (c[4] + c[5]) as f64
}

#[test]
fn sync_moves_only_live_and_dirty_values() {
    let opts = Options {
        counters: true,
        ..Options::default()
    };
    let all = Options {
        sync_all: true,
        ..opts.clone()
    };
    for case in [
        spike::fib_rec(15).unwrap(),
        spike::loop_calls(50).unwrap(),
        spike::unwind_dirty(50).unwrap(),
    ] {
        let (fast, reference) = (moved_per_call(&case, &opts), moved_per_call(&case, &all));
        assert!(
            reference >= 80.0,
            "{}: sync_all moves {reference}",
            case.name
        );
        assert!(
            fast * 3.0 < reference,
            "{}: {fast} values per call, sync_all {reference}",
            case.name
        );
    }
}

#[test]
fn counters_count_calls() {
    let opts = Options {
        counters: true,
        ..Options::default()
    };
    let c = counted(&spike::fib_rec(10).unwrap(), &opts);
    assert!(c[4] > 0, "direct calls {c:?}");
    let all = counted(
        &spike::fib_rec(10).unwrap(),
        &Options {
            sync_all: true,
            ..opts
        },
    );
    assert_eq!(all[8] % 20, 0);
    assert_eq!(all[9] % 20, 0);
}

#[test]
fn counters_off_leaves_the_bytes_alone() {
    let case = spike::loop_branch(100).unwrap();
    let off = case.translate(&Options::default()).unwrap().wasm;
    let explicit = case
        .translate(&Options {
            counters: false,
            ..Options::default()
        })
        .unwrap()
        .wasm;
    assert_eq!(off, explicit);
    let on = case
        .translate(&Options {
            counters: true,
            ..Options::default()
        })
        .unwrap()
        .wasm;
    assert_ne!(off, on);
}

#[test]
fn big_function() {
    let case = spike::big_function(100).unwrap();
    let t = case.translate(&Options::default()).unwrap();
    assert!(t.guest_instructions > Options::DEFAULT_MAX_FUNCTION_INSNS);
    assert_eq!(t.functions.len(), 1);
    let chunks = &t.functions[0].chunks;
    assert!(chunks.len() >= 2, "{} chunks", chunks.len());
    // Each loop (from a backward branch's target to the branch) stays in
    // one chunk.
    let chunk_of = |addr: u64| chunks.iter().position(|c| c.blocks.contains(&addr));
    let prog = x64wasm::lift(
        x64wasm::CodeRegion {
            base: case.code_base,
            bytes: &case.code,
        },
        &case.entries(),
    )
    .unwrap();
    let mut loops = 0;
    for block in &prog.blocks {
        if let Term::Branch { taken, .. } = block.term
            && taken <= block.addr
        {
            let inside = t
                .blocks
                .iter()
                .filter(|&&a| (taken..=block.addr).contains(&a));
            for &a in inside {
                assert_eq!(
                    chunk_of(a),
                    chunk_of(block.addr),
                    "loop at {taken:#x} split"
                );
            }
            loops += 1;
        }
    }
    assert_eq!(loops, 100);
    run_case(&case);
    run_case(&spike::big_function(7).unwrap());
}

#[test]
fn cases_pass_under_every_option_set() {
    for opts in option_sweep() {
        for case in spike::cases().unwrap() {
            run_case_with(&case, &opts);
        }
    }
}

/// The expected results of every case hold for the machine code itself.
#[cfg(all(target_arch = "x86_64", unix))]
#[test]
fn cases_match_native() {
    for case in spike::cases().unwrap() {
        let run = spike::native::run_case(&case).unwrap();
        case.check_with(&run.state, |addr, len| run.read(addr, len))
            .unwrap_or_else(|e| panic!("native: {e:#}"));
    }
}

#[test]
fn manifest_follows_the_abi_doc() {
    let dir = std::env::temp_dir().join(format!("tempwind-spike-{}", std::process::id()));
    spike::write(&dir).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    let abi_obj = &manifest["abi"];
    assert_eq!(abi_obj["stateSize"], abi::STATE_SIZE);
    assert_eq!(abi_obj["gpr"], abi::STATE_GPR);
    assert_eq!(abi_obj["rip"], abi::STATE_RIP);
    assert_eq!(abi_obj["rflags"], abi::STATE_RFLAGS);
    assert_eq!(abi_obj["exit"], abi::STATE_EXIT);
    let cases = manifest["cases"].as_array().unwrap();
    let names: Vec<&str> = cases.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "sum_loop",
            "fib_recursive",
            "array",
            "high_address",
            "fib_rec",
            "deep_rec",
            "tail_rec",
            "ret_tricks",
            "call_table",
            "shared_tail",
            "longjmp_like",
            "big_function",
            "loop_branch",
            "merge_call",
            "loop_calls",
            "clobber_calls",
            "flags_across_call",
            "unwind_dirty",
        ]
    );
    let is_hex = |v: &serde_json::Value| {
        v.as_str()
            .and_then(|s| s.strip_prefix("0x"))
            .is_some_and(|h| u64::from_str_radix(h, 16).is_ok())
    };
    for case in cases {
        let wasm = std::fs::read(dir.join(case["wasm"].as_str().unwrap())).unwrap();
        assert_eq!(&wasm[..4], b"\0asm");
        for key in ["memoryBytes", "state"] {
            assert!(is_hex(&case[key]), "{key}");
        }
        assert!(is_hex(&case["init"]["rip"]) && is_hex(&case["init"]["rflags"]));
        assert_eq!(case["expect"]["exit"], abi::EXIT_LEFT_CODE);
        assert_eq!(case["expect"]["rip"], format!("{:#x}", abi::SENTINEL_RET));
        for section in ["init", "expect"] {
            for (name, value) in case[section]["regs"].as_object().unwrap() {
                assert!(abi::GPR_NAMES.contains(&name.as_str()), "{name}");
                assert!(is_hex(value));
            }
            for m in case[section]["mem"].as_array().unwrap() {
                assert!(is_hex(&m["addr"]));
                let hex: String = m["hex"].as_str().unwrap().split_whitespace().collect();
                assert!(hex.len().is_multiple_of(2) && hex.chars().all(|c| c.is_ascii_hexdigit()));
            }
        }
    }
    // The example in docs/design/abi.md: the sentinel at the stack top.
    let sum = &cases[0];
    assert_eq!(sum["init"]["regs"]["rcx"], "0x64");
    assert_eq!(sum["expect"]["regs"]["rax"], "0x13ba");
    let mem = sum["init"]["mem"].as_array().unwrap();
    assert!(
        mem.iter()
            .any(|m| m["addr"] == "0x2ff000" && m["hex"] == "0000ffff ff7f0000")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
