//! x86-64 to WebAssembly translator.
//!
//! Pipeline: decode (iced-x86) -> IR -> WebAssembly (wasm-encoder, memory64).
//! The module ABI is in [`abi`] and `docs/design/abi.md`.

pub mod abi;
pub mod emit;
pub mod func;
pub mod ir;
pub mod lift;
pub mod pe;

use std::fmt;

/// iced-x86 decoder options matching the guest CPU the translator presents
/// (the Phase 1 CPUID profile in `docs/design/instruction-scope.md`): no BMI1
/// and no LZCNT, so `F3 0F BC` and `F3 0F BD` execute as `bsf` and `bsr`
/// (the `F3` prefix is ignored), not as `tzcnt` and `lzcnt`. MinGW's gdtoa
/// (`__trailz_D2A`) uses that encoding.
pub const DECODER_OPTIONS: u32 =
    iced_x86::DecoderOptions::NO_MPFX_0FBC | iced_x86::DecoderOptions::NO_MPFX_0FBD;

/// Translation options.
#[derive(Clone, Debug)]
pub struct Options {
    /// Import guest memory as shared memory (needed when several workers run guest threads).
    pub shared_memory: bool,
    /// Functions with more guest instructions than this are cut into chunks
    /// (one wasm function each), never inside a loop nest (soft limit).
    pub max_function_insns: usize,
    /// Loop nests larger than this are cut too (hard limit).
    pub max_chunk_insns: usize,
    /// Guest calls nested deeper than this on the wasm stack unwind to the
    /// trampoline instead. `None` disables the guard (for measurements only:
    /// deep guest recursion then overflows the wasm stack).
    pub call_depth_limit: Option<u32>,
}

impl Options {
    /// Default [`Options::max_function_insns`]: where compile time starts
    /// to grow with chunk size (`docs/design/translator.md`, "Defaults").
    pub const DEFAULT_MAX_FUNCTION_INSNS: usize = 2000;
    /// Default [`Options::max_chunk_insns`]: 10x the soft limit.
    pub const DEFAULT_MAX_CHUNK_INSNS: usize = 20_000;
    /// Default [`Options::call_depth_limit`]: a third of the shallowest
    /// measured stack (wasmtime's 512 KiB, about 1500 guest calls).
    pub const DEFAULT_CALL_DEPTH_LIMIT: u32 = 500;
}

impl Default for Options {
    fn default() -> Self {
        Options {
            shared_memory: false,
            max_function_insns: Self::DEFAULT_MAX_FUNCTION_INSNS,
            max_chunk_insns: Self::DEFAULT_MAX_CHUNK_INSNS,
            call_depth_limit: Some(Self::DEFAULT_CALL_DEPTH_LIMIT),
        }
    }
}

/// Guest code to translate: `bytes` are mapped at guest address `base`.
#[derive(Clone, Copy, Debug)]
pub struct CodeRegion<'a> {
    pub base: u64,
    pub bytes: &'a [u8],
}

/// One guest function of a [`Translation`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionInfo {
    /// Guest address of the function's entry.
    pub addr: u64,
    /// Its chunks, one wasm function each.
    pub chunks: Vec<ChunkInfo>,
}

/// One chunk (wasm function) of a guest function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkInfo {
    /// Index of the wasm function in the module.
    pub wasm_index: u32,
    /// Guest addresses of its blocks, sorted; a block's position is its
    /// `entry` index.
    pub blocks: Vec<u64>,
}

/// The result of translating a code region.
#[derive(Clone, Debug)]
pub struct Translation {
    /// A memory64 WebAssembly module following [`abi`].
    pub wasm: Vec<u8>,
    /// Guest addresses of every translated basic block, sorted.
    pub blocks: Vec<u64>,
    /// Guest functions in entry address order.
    pub functions: Vec<FunctionInfo>,
    /// Human-readable IR dump grouped by function and chunk (empty from
    /// [`translate_without_dump`]).
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
    let part = func::partition(&prog, entries, opts);
    let wasm = emit::emit(&prog, &part, opts);
    let functions = part
        .functions
        .iter()
        .map(|f| FunctionInfo {
            addr: f.entry,
            chunks: f
                .chunks
                .iter()
                .map(|&c| ChunkInfo {
                    wasm_index: emit::F_CHUNK0 + c as u32,
                    blocks: part.chunks[c]
                        .blocks
                        .iter()
                        .map(|&b| prog.blocks[b].addr)
                        .collect(),
                })
                .collect(),
        })
        .collect();
    Ok(Translation {
        wasm,
        blocks: prog.blocks.iter().map(|b| b.addr).collect(),
        functions,
        ir: if dump {
            part.dump(&prog, emit::F_CHUNK0)
        } else {
            String::new()
        },
        guest_instructions: prog.insn_count(),
    })
}

#[cfg(test)]
mod tests {
    use iced_x86::{Decoder, Mnemonic};

    use super::DECODER_OPTIONS;

    #[test]
    fn rep_bsf_and_rep_bsr_decode_as_bsf_and_bsr() {
        let decode = |bytes: &[u8]| Decoder::new(64, bytes, DECODER_OPTIONS).decode();
        // tzcnt/lzcnt on a CPU with BMI1/LZCNT; bsf/bsr under our profile.
        assert_eq!(
            decode(&[0xf3, 0x48, 0x0f, 0xbc, 0xc1]).mnemonic(),
            Mnemonic::Bsf
        );
        assert_eq!(decode(&[0xf3, 0x0f, 0xbd, 0xc1]).mnemonic(), Mnemonic::Bsr);
        assert_eq!(decode(&[0x0f, 0xbc, 0xc1]).mnemonic(), Mnemonic::Bsf);
    }
}
