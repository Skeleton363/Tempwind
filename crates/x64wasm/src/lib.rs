//! x86-64 to WebAssembly translator.
//!
//! Pipeline: decode (iced-x86) -> IR -> WebAssembly (wasm-encoder, memory64).
//! The module ABI is in [`abi`] and `docs/design/abi.md`.

pub mod abi;
pub mod emit;
pub mod ir;
pub mod lift;
pub mod pe;

use std::fmt;

/// Translation options.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Import guest memory as shared memory (needed when several workers run guest threads).
    pub shared_memory: bool,
}

/// Guest code to translate: `bytes` are mapped at guest address `base`.
#[derive(Clone, Copy, Debug)]
pub struct CodeRegion<'a> {
    pub base: u64,
    pub bytes: &'a [u8],
}

/// The result of translating a code region.
#[derive(Clone, Debug)]
pub struct Translation {
    /// A memory64 WebAssembly module following [`abi`].
    pub wasm: Vec<u8>,
    /// Guest addresses of every translated basic block, sorted.
    pub blocks: Vec<u64>,
    /// Human-readable IR dump (empty from [`translate_without_dump`]).
    pub ir: String,
    /// Number of guest instructions translated.
    pub guest_instructions: usize,
}

/// Translation errors.
#[derive(Debug)]
pub enum Error {
    /// An entry point lies outside the code region.
    EntryOutsideRegion(u64),
    /// Not implemented yet.
    Unimplemented(&'static str),
    /// No entry points were given.
    NoEntries,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::EntryOutsideRegion(addr) => {
                write!(f, "entry {addr:#x} is outside the code region")
            }
            Error::Unimplemented(what) => write!(f, "not implemented: {what}"),
            Error::NoEntries => write!(f, "no entry points"),
        }
    }
}

impl std::error::Error for Error {}

/// Translates the code reachable from `entries` inside `region`.
///
/// Control flow that leaves the region becomes an [`abi::EXIT_LEFT_CODE`] exit.
pub fn translate(
    region: CodeRegion<'_>,
    entries: &[u64],
    opts: &Options,
) -> Result<Translation, Error> {
    translate_impl(region, entries, opts, true)
}

/// Like [`translate`] but skips the disassembly and IR dump (faster; used by
/// throughput measurements and hosts that never look at the dump).
pub fn translate_without_dump(
    region: CodeRegion<'_>,
    entries: &[u64],
    opts: &Options,
) -> Result<Translation, Error> {
    translate_impl(region, entries, opts, false)
}

/// Decodes and lowers into the IR, with flag liveness applied.
pub fn lift(region: CodeRegion<'_>, entries: &[u64]) -> Result<ir::Program, Error> {
    let mut prog = lift::lift(region, entries, true)?;
    prog.analyze_flags();
    Ok(prog)
}

fn translate_impl(
    region: CodeRegion<'_>,
    entries: &[u64],
    opts: &Options,
    dump: bool,
) -> Result<Translation, Error> {
    if entries.is_empty() {
        return Err(Error::NoEntries);
    }
    let mut prog = lift::lift(region, entries, dump)?;
    prog.analyze_flags();
    let wasm = emit::emit(&prog, opts);
    Ok(Translation {
        wasm,
        blocks: prog.blocks.iter().map(|b| b.addr).collect(),
        ir: if dump { prog.dump() } else { String::new() },
        guest_instructions: prog.insn_count(),
    })
}
