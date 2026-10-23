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
