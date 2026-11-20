// Copyright 2026 the Tempwind authors
//
// This library is free software; you can redistribute it and/or
// modify it under the terms of the GNU Lesser General Public
// License as published by the Free Software Foundation; either
// version 2.1 of the License, or (at your option) any later version.

//! pe-rebase --tree target/wine/pe --closure target/wine/closure.json --out target/wine/rebased
//! pe-rebase --verify --tree ... --closure ... --out ...
//!
//! Moves the closure's DLLs to unique bases and writes them and `rebase.json`
//! under `--out`; `--verify` compares the moved files with the originals.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "pe-rebase",
    about = "Rebase the DLLs of Wine's PE tree to unique bases"
)]
struct Cli {
    /// The release's PE tree (holds lib/wine/x86_64-windows).
    #[arg(long)]
    tree: PathBuf,
    /// The release's closure.json.
    #[arg(long)]
    closure: PathBuf,
    /// Output directory (rebase.json and the moved files).
    #[arg(long)]
    out: PathBuf,
    /// Compare each moved file with its original instead of writing.
    #[arg(long)]
    verify: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.verify {
        let n = pe_rebase::verify(&cli.tree, &cli.closure, &cli.out)?;
        println!("verified {n} rebased modules");
    } else {
        let n = pe_rebase::rebase(&cli.tree, &cli.closure, &cli.out)?;
        println!("rebased {n} modules into {}", cli.out.display());
    }
    Ok(())
}
