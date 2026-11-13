//! The committed C header (`abi.md`, "The C header") is what `abi.rs`
//! renders: any difference fails, naming the command that regenerates it.

use std::path::Path;

use x64wasm::abi;

#[test]
fn committed_header_matches_abi_rs() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(abi::C_HEADER_PATH);
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "reading {}: {e}; run: {}",
            path.display(),
            abi::C_HEADER_COMMAND
        )
    });
    assert!(
        committed == abi::c_header(),
        "{} differs from abi.rs; run: {}",
        path.display(),
        abi::C_HEADER_COMMAND
    );
}
