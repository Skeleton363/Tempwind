//! The Phase 0 spike programs, translated and run in wasmtime with fixed
//! expected results, plus the manifest `testkit spike` writes for the JS
//! hosts.

use testkit::host::Host;
use testkit::spike::{self, Case};
use x64wasm::Options;
use x64wasm::abi;

fn run_case(case: &Case) {
    let t = case.translate(&Options::default()).unwrap();
    let host = Host::new().unwrap();
    let (out, machine) = case.run(&host, &t.wasm).unwrap();
    case.check(&out, &machine).unwrap();
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
        ["sum_loop", "fib_recursive", "array", "high_address"]
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
                assert!(hex.len() % 2 == 0 && hex.chars().all(|c| c.is_ascii_hexdigit()));
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
