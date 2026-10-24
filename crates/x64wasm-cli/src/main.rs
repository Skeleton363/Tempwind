//! `x64wasm`: translate x86-64 code, dump IR and WAT.
//!
//! ```text
//! x64wasm translate --base 0x140001000 --hex "48 01 d8 c3" -o out.wasm
//! x64wasm ir        --base 0x1000 --file code.bin --entry 0x1010
//! x64wasm wat       --base 0x1000 --hex "4801d8c3" --shared
//! x64wasm translate --base 0x1000 --file code.bin --max-function-insns 500 --no-depth-guard -o out.wasm
//! x64wasm translate-pe basic.exe --out-dir out [--entries all|entry]
//! ```

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use x64wasm::image::{self, EntrySet, LoadPlan};
use x64wasm::pe::Pe;
use x64wasm::{CodeRegion, Options};

#[derive(Parser)]
#[command(
    name = "x64wasm",
    version,
    about = "x86-64 to WebAssembly (memory64) translator"
)]
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
    /// Translate a whole PE image ahead of time: writes DIR/<stem>.wasm (a
    /// v3 module) and DIR/<stem>.plan.json (its load plan).
    TranslatePe {
        /// The PE file (x86-64).
        file: PathBuf,
        /// Output directory (created if missing).
        #[arg(long)]
        out_dir: PathBuf,
        /// Start from every function entry the image reveals, or only the
        /// entry point.
        #[arg(long, value_enum, default_value_t = Entries::All)]
        entries: Entries,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Entries {
    All,
    Entry,
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
    /// Cut functions with more guest instructions than this into chunks
    /// (never inside a loop nest).
    #[arg(long, default_value_t = Options::DEFAULT_MAX_FUNCTION_INSNS)]
    max_function_insns: usize,
    /// Cut loop nests with more guest instructions than this too.
    #[arg(long, default_value_t = Options::DEFAULT_MAX_CHUNK_INSNS)]
    max_chunk_insns: usize,
    /// Nested guest calls on the wasm stack before a call unwinds instead.
    #[arg(long, default_value_t = Options::DEFAULT_CALL_DEPTH_LIMIT)]
    call_depth_limit: u32,
    /// Disable the call-depth guard (measurements only: deep recursion then
    /// overflows the wasm stack).
    #[arg(long, conflicts_with = "call_depth_limit")]
    no_depth_guard: bool,
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
    if !digits.len().is_multiple_of(2) {
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
            max_function_insns: self.max_function_insns,
            max_chunk_insns: self.max_chunk_insns,
            call_depth_limit: (!self.no_depth_guard).then_some(self.call_depth_limit),
        };
        let region = CodeRegion {
            base: self.base,
            bytes: &bytes,
        };
        let t = x64wasm::translate(region, &entries, &opts)?;
        validate(&t.wasm)?;
        Ok(t)
    }
}

fn validate(wasm: &[u8]) -> Result<()> {
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(wasm)
        .context("translated module failed validation")?;
    Ok(())
}

fn translate_pe(file: &Path, out_dir: &Path, entries: Entries) -> Result<()> {
    let data = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let pe = Pe::parse(&data).with_context(|| format!("parsing {}", file.display()))?;
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .context("file name is not UTF-8")?;
    let stem = file
        .file_stem()
        .and_then(|n| n.to_str())
        .context("file name is not UTF-8")?;
    let plan = LoadPlan::from_pe(&pe, name).context("building the load plan")?;
    let start = Instant::now();
    let set = match entries {
        Entries::All => EntrySet::All,
        Entries::Entry => EntrySet::EntryPoint,
    };
    let t = image::translate(&pe, set, &Options::default()).context("translating")?;
    let ms = start.elapsed().as_secs_f64() * 1e3;
    validate(&t.wasm)?;
    std::fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    let wasm_path = out_dir.join(format!("{stem}.wasm"));
    let plan_path = out_dir.join(format!("{stem}.plan.json"));
    std::fs::write(&wasm_path, &t.wasm)
        .with_context(|| format!("writing {}", wasm_path.display()))?;
    std::fs::write(&plan_path, plan.to_json())
        .with_context(|| format!("writing {}", plan_path.display()))?;
    println!(
        "{name}: {} functions, {} blocks, {} guest instructions, {} wasm bytes, {ms:.0} ms",
        t.functions.len(),
        t.blocks.len(),
        t.guest_instructions,
        t.wasm.len()
    );
    eprintln!(
        "  {} indirect jumps, {} jump tables, {} unsupported sites",
        t.indirect_jumps,
        t.jump_tables.len(),
        t.unsupported.len()
    );
    Ok(())
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Translate { input, output } => {
            let t = input.translate()?;
            std::fs::write(&output, &t.wasm)
                .with_context(|| format!("writing {}", output.display()))?;
            let chunks: usize = t.functions.iter().map(|f| f.chunks.len()).sum();
            eprintln!(
                "{} guest instructions, {} blocks, {} functions ({} chunks), {} bytes -> {}",
                t.guest_instructions,
                t.blocks.len(),
                t.functions.len(),
                chunks,
                t.wasm.len(),
                output.display()
            );
        }
        Command::Ir { input } => print!("{}", input.translate()?.ir),
        Command::Wat { input } => {
            let t = input.translate()?;
            println!("{}", wasmprinter::print_bytes(&t.wasm)?);
        }
        Command::TranslatePe {
            file,
            out_dir,
            entries,
        } => translate_pe(&file, &out_dir, entries)?,
    }
    Ok(())
}
