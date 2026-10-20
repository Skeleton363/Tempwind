//! Instruction oracle: records single-instruction results on a real x86-64
//! CPU and replays them through the translator.

use anyhow::{Result, bail};

/// `testkit record [--check]`.
pub fn record(check: bool) -> Result<()> {
    let _ = check;
    bail!("record: not implemented yet")
}

/// `testkit replay`.
pub fn replay() -> Result<()> {
    bail!("replay: not implemented yet")
}
