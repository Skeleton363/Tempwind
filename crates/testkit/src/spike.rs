//! Spike programs: small x86-64 functions translated into wasm plus a
//! manifest the JS hosts run (see `docs/design/abi.md`).

use std::path::Path;

use anyhow::{Result, bail};

/// `testkit spike --out DIR`.
pub fn write(out: &Path) -> Result<()> {
    let _ = out;
    bail!("spike: not implemented yet")
}
