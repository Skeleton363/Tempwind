//! L5 snapshot tests: IR and WAT of a few tiny functions, compared with the
//! committed files in `tests/snapshots/`. Regenerate with
//! `UPDATE_SNAPSHOTS=1 cargo test -p x64wasm --test snapshots`.

use std::path::PathBuf;

use x64wasm::{CodeRegion, Options};

fn hex(s: &str) -> Vec<u8> {
    let d: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..d.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&d[i..i + 2], 16).unwrap())
        .collect()
}

fn check(name: &str, base: u64, code: &str) {
    let bytes = hex(code);
    let t = x64wasm::translate(
        CodeRegion {
            base,
            bytes: &bytes,
        },
        &[base],
        &Options::default(),
    )
    .unwrap();
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&t.wasm)
        .unwrap();
    let wat = wasmprinter::print_bytes(&t.wasm).unwrap() + "\n";
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let update = std::env::var_os("UPDATE_SNAPSHOTS").is_some_and(|v| v != "0");
    let mut failures = Vec::new();
    for (ext, actual) in [("ir", &t.ir), ("wat", &wat)] {
        let path = dir.join(format!("{name}.{ext}"));
        if update {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, actual).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e}; run with UPDATE_SNAPSHOTS=1 to create it",
                path.display()
            )
        });
        if &expected != actual {
            failures.push(format!(
                "{} differs; rerun with UPDATE_SNAPSHOTS=1 if the change is intended.\n--- actual ---\n{actual}",
                path.display()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `add rax, rbx; ret`
#[test]
fn add_ret() {
    check("add_ret", 0x1_4000_1000, "48 01 d8 c3");
}

/// The sum_loop spike: `rax = 1 + ... + rcx`.
#[test]
fn sum_loop() {
    check(
        "sum_loop",
        0x10_0000,
        "b800000000 4883f900 7409 4801c8 4883e901 75f7 c3",
    );
}

/// `push rbx; mov rbx, rcx; call f; pop rbx; ret; f: lea rax, [rbx+rbx*2]; ret`
#[test]
fn call_lea() {
    check("call_lea", 0x1000, "53 4889cb e802000000 5b c3 488d045b c3");
}

/// A call, a return and a tail transfer between functions:
/// `call f; call g; ret / f: add rax, 1; jmp g / g: add rax, 2; ret`.
#[test]
fn call_ret_tail() {
    check(
        "call_ret_tail",
        0x1000,
        "e806000000 e807000000 c3 4883c001 eb00 4883c002 c3",
    );
}
