//! `x64wasm`: translate x86-64 code, dump IR and WAT.
//!
//! ```text
//! x64wasm translate --base 0x140001000 --hex "48 01 d8 c3" -o out.wasm
//! x64wasm ir        --base 0x1000 --file code.bin --entry 0x1010
//! x64wasm wat       --base 0x1000 --hex "4801d8c3" --shared
//! ```

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use x64wasm::{CodeRegion, Options};

#[derive(Parser)]
#[command(name = "x64wasm", version, about = "x86-64 to WebAssembly (memory64) translator")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Translate code into a wasm module.
    Translate {
        #[command(flatten)]
        input: Input,
        /// Output .wasm file.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Print the IR.
    Ir {
        #[command(flatten)]
        input: Input,
    },
    /// Print the translated module as WAT.
    Wat {
        #[command(flatten)]
        input: Input,
    },
}

#[derive(Args)]
struct Input {
    /// Guest address of the first code byte (hex with 0x, or decimal).
    #[arg(long, value_parser = parse_u64)]
    base: u64,
    /// Code bytes as hex; spaces are ignored.
    #[arg(long, conflicts_with = "file", required_unless_present = "file")]
    hex: Option<String>,
    /// Raw code bytes from a file.
    #[arg(long)]
    file: Option<PathBuf>,
    /// Entry point(s); default: the base. May be repeated.
    #[arg(long, value_parser = parse_u64)]
    entry: Vec<u64>,
    /// Import shared memory (max 16 GiB).
    #[arg(long)]
    shared: bool,
}

fn parse_u64(s: &str) -> Result<u64, String> {
    let s = s.replace('_', "");
    let parsed = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => s.parse(),
    };
    parsed.map_err(|e| format!("{s}: {e}"))
}

fn parse_hex(s: &str) -> Result<Vec<u8>> {
    let digits: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.len() % 2 != 0 {
        bail!("odd number of hex digits");
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).context("bad hex"))
        .collect()
}

impl Input {
    fn translate(&self) -> Result<x64wasm::Translation> {
        let bytes = match (&self.hex, &self.file) {
            (Some(hex), _) => parse_hex(hex)?,
            (None, Some(path)) => {
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))?
            }
            (None, None) => bail!("give --hex or --file"),
        };
        let entries = if self.entry.is_empty() {
            vec![self.base]
        } else {
            self.entry.clone()
        };
        let opts = Options {
            shared_memory: self.shared,
        };
        let region = CodeRegion {
            base: self.base,
            bytes: &bytes,
        };
        let t = x64wasm::translate(region, &entries, &opts)?;
        wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
            .validate_all(&t.wasm)
            .context("translated module failed validation")?;
        Ok(t)
    }
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Translate { input, output } => {
            let t = input.translate()?;
            std::fs::write(&output, &t.wasm)
                .with_context(|| format!("writing {}", output.display()))?;
            eprintln!(
                "{} guest instructions, {} blocks, {} bytes -> {}",
                t.guest_instructions,
                t.blocks.len(),
                t.wasm.len(),
                output.display()
            );
        }
        Command::Ir { input } => print!("{}", input.translate()?.ir),
        Command::Wat { input } => {
            let t = input.translate()?;
            println!("{}", wasmprinter::print_bytes(&t.wasm)?);
        }
    }
    Ok(())
}
