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
    Replay {
        /// Translate in the reference sync (`Options::sync_all`).
        #[arg(long)]
        sync_all: bool,
        /// Translate in the reference layout (`Options::structured` false).
        #[arg(long)]
        no_structured: bool,
    },
    /// Write spike programs (wasm + manifest.json) for the JS hosts.
    Spike {
        #[arg(long, default_value = "target/spike")]
        out: PathBuf,
        /// Write the translator measurement programs instead (for
        /// tests/bench/translator/bench.mjs), e.g. to target/bench/translator.
        #[arg(long)]
        bench: bool,
    },
    /// Instruction and import census of PE files (docs/design/instruction-scope.md).
    Census(testkit::census::Args),
    /// DLL closure of the Wine build (docs/design/wine-build.md).
    Closure(testkit::closure::Args),
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Record { check } => testkit::oracle::record(check),
        Command::Replay {
            sync_all,
            no_structured,
        } => testkit::oracle::replay(&x64wasm::Options {
            sync_all,
            structured: !no_structured,
            ..x64wasm::Options::default()
        }),
        Command::Spike { out, bench: false } => testkit::spike::write(&out),
        Command::Spike { out, bench: true } => testkit::spike::bench::write(&out),
        Command::Census(args) => testkit::census::run(&args),
        Command::Closure(args) => testkit::closure::run(&args),
    }
}
