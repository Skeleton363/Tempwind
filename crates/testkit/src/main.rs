//! `testkit`: record and replay instruction fixtures, write spike programs,
//! take the instruction census of PE files.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "x64wasm test tooling: instruction oracle and spike programs")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Record L1 fixtures natively (x86-64 Linux only).
    Record {
        /// Re-record in memory and fail if anything differs from the committed fixtures.
        #[arg(long)]
        check: bool,
    },
    /// Replay L1 fixtures through the translator.
    Replay,
    /// Write spike programs (wasm + manifest.json) for the JS hosts.
    Spike {
        #[arg(long, default_value = "target/spike")]
        out: PathBuf,
    },
    /// Instruction and import census of PE files (docs/design/instruction-scope.md).
    Census(testkit::census::Args),
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Record { check } => testkit::oracle::record(check),
        Command::Replay => testkit::oracle::replay(),
        Command::Spike { out } => testkit::spike::write(&out),
        Command::Census(args) => testkit::census::run(&args),
    }
}
