//! Instruction and import census of PE files (`testkit census`): counts per
//! iced-x86 `Code`, mnemonic and CPUID feature, the import table, and the
//! check that every in-scope form has L1 fixtures. The scope rule is in
//! `docs/design/instruction-scope.md`. Built in Phase 1 M1 track B.

use std::path::PathBuf;

use anyhow::{Result, bail};

/// Arguments of `testkit census`.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// PE files to analyze.
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
    /// Write the full census as JSON.
    #[arg(long)]
    pub json: Option<PathBuf>,
    /// Write a Markdown summary.
    #[arg(long)]
    pub md: Option<PathBuf>,
    /// Fail if an in-scope form has no L1 fixtures.
    #[arg(long)]
    pub check: bool,
}

pub fn run(args: &Args) -> Result<()> {
    bail!(
        "census of {} file(s): not built yet (Phase 1 M1 track B)",
        args.files.len()
    )
}
