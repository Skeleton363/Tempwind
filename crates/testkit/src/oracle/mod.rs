//! Instruction oracle: records single-instruction results on a real x86-64
//! CPU (`testkit record`) and replays them through the translator
//! (`testkit replay`, `tests/replay.rs`).
//!
//! Layout shared by recording and replay (guest address == wasm offset):
//! - [`CODE_ADDR`]: one 4 KiB page holding the instruction, zeros after it.
//! - [`DATA_ADDR`]: the 64 KiB data window. Memory operands, RIP-relative
//!   operands and the stack of push/pop/call/ret all land here. It is zero
//!   except for each case's input ranges.
//! - [`STATE_ADDR`]: the CPU state block in replay, inside [`MEMORY_BYTES`]
//!   of guest memory.
//!
//! The fixture format is described in [`fixture`].

pub mod fixture;
pub mod forms;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod native;
mod record;
mod replay;
mod rng;

use std::path::PathBuf;

use anyhow::{Result, bail};

pub use record::generated_inputs;
pub use replay::{ReplayReport, Stats, replay_dir};

/// Address of the code page holding the instruction under test.
pub const CODE_ADDR: u64 = 0x10_0000;
/// Size of the code page.
pub const CODE_SIZE: u64 = 0x1000;
/// Address of the data window (memory operands and stack).
pub const DATA_ADDR: u64 = 0x20_0000;
/// Size of the data window.
pub const DATA_SIZE: u64 = 0x1_0000;
/// Address of the CPU state block during replay.
pub const STATE_ADDR: u64 = 0x30_0000;
/// Guest memory size during replay.
pub const MEMORY_BYTES: u64 = 4 << 20;
/// Seed of every generator stream ("Tempwind" in ASCII).
pub const SEED: u64 = 0x5465_6d70_7769_6e64;
/// Fixture format version.
pub const FORMAT: u32 = 2;

/// `crates/testkit/fixtures/l1`.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/l1")
}

/// `testkit record [--check]`.
pub fn record(check: bool) -> Result<()> {
    record::record(check)
}

/// `testkit replay`: replays every fixture and prints a summary per family.
pub fn replay() -> Result<()> {
    let report = replay_dir(&fixtures_dir())?;
    print!("{}", report.render());
    if !report.ok() {
        let t = report.total();
        bail!(
            "replay: {} of {} cases failed, {} unsupported",
            t.fail,
            t.cases,
            t.unsupported
        );
    }
    Ok(())
}
