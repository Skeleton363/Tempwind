//! x86-64 to WebAssembly translator.
//!
//! Pipeline: decode (iced-x86) -> IR -> WebAssembly (wasm-encoder, memory64).
//! The module ABI is in [`abi`] and `docs/design/abi.md`.

pub mod abi;

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
    /// Human-readable IR dump.
    pub ir: String,
}

/// Translation errors.
#[derive(Debug)]
pub enum Error {
    /// An entry point lies outside the code region.
    EntryOutsideRegion(u64),
    /// Not implemented yet.
    Unimplemented(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::EntryOutsideRegion(addr) => {
                write!(f, "entry {addr:#x} is outside the code region")
            }
            Error::Unimplemented(what) => write!(f, "not implemented: {what}"),
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
    let _ = (region, entries, opts);
    Err(Error::Unimplemented("translate"))
}
