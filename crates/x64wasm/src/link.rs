//! Linking support: the `x64wasm.blocks` custom section of a v3 module
//! (`docs/design/abi.md`), which lists the module's blocks for the host to
//! insert into the lookup table.

use std::fmt;

use crate::abi;

/// One block of a module: guest address, chunk (relative to the module's
/// `table_base`) and entry index within the chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockEntry {
    pub addr: u64,
    pub chunk: u32,
    pub entry: u32,
}

/// The contents of a module's `x64wasm.blocks` section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleBlocks {
    /// Number of chunk functions: the table slots the module needs.
    pub chunks: u32,
    /// Sorted by address.
    pub blocks: Vec<BlockEntry>,
}

/// A malformed module or `x64wasm.blocks` section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkError(pub String);

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LinkError {}

fn err<T>(msg: impl Into<String>) -> Result<T, LinkError> {
    Err(LinkError(msg.into()))
}

/// Encodes the section payload. `blocks` must be sorted by address.
pub fn encode_blocks(chunks: u32, blocks: &[BlockEntry]) -> Vec<u8> {
    let mut out = Vec::with_capacity(abi::BLOCKS_HEADER_SIZE + blocks.len() * 16);
    out.extend_from_slice(&abi::BLOCKS_VERSION.to_le_bytes());
    out.extend_from_slice(&chunks.to_le_bytes());
    out.extend_from_slice(&(blocks.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for b in blocks {
        out.extend_from_slice(&b.addr.to_le_bytes());
        out.extend_from_slice(&b.chunk.to_le_bytes());
        out.extend_from_slice(&b.entry.to_le_bytes());
    }
    out
}

fn u32_at(p: &[u8], at: usize) -> u32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&p[at..at + 4]);
    u32::from_le_bytes(b)
}

fn u64_at(p: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&p[at..at + 8]);
    u64::from_le_bytes(b)
}

/// Decodes and checks a section payload: version 1, exact length, blocks
/// strictly sorted by nonzero address, chunks in range.
pub fn decode_blocks(payload: &[u8]) -> Result<ModuleBlocks, LinkError> {
    if payload.len() < abi::BLOCKS_HEADER_SIZE {
        return err("x64wasm.blocks: truncated header");
    }
    let version = u32_at(payload, 0);
    if version != abi::BLOCKS_VERSION {
        return err(format!("x64wasm.blocks: unknown version {version}"));
    }
    let chunks = u32_at(payload, 4);
    let n = u32_at(payload, 8) as usize;
    let want = n
        .checked_mul(abi::BLOCKS_ENTRY_SIZE)
        .and_then(|b| b.checked_add(abi::BLOCKS_HEADER_SIZE));
    if want != Some(payload.len()) {
        return err(format!(
            "x64wasm.blocks: {n} blocks don't fit {} bytes",
            payload.len()
        ));
    }
    let mut blocks = Vec::with_capacity(n);
    for i in 0..n {
        let at = abi::BLOCKS_HEADER_SIZE + i * abi::BLOCKS_ENTRY_SIZE;
        let b = BlockEntry {
            addr: u64_at(payload, at),
            chunk: u32_at(payload, at + 8),
            entry: u32_at(payload, at + 12),
        };
        if b.addr == 0 {
            return err("x64wasm.blocks: block at address 0");
        }
        if b.chunk >= chunks {
            return err(format!(
                "x64wasm.blocks: block {:#x} in chunk {} of {chunks}",
                b.addr, b.chunk
            ));
        }
        if blocks.last().is_some_and(|p: &BlockEntry| p.addr >= b.addr) {
            return err(format!("x64wasm.blocks: {:#x} out of order", b.addr));
        }
        blocks.push(b);
    }
    Ok(ModuleBlocks { chunks, blocks })
}

fn leb_u32(wasm: &[u8], pos: &mut usize) -> Result<u32, LinkError> {
    let mut value: u64 = 0;
    for shift in (0..35).step_by(7) {
        let Some(&byte) = wasm.get(*pos) else {
            return err("wasm: truncated LEB128");
        };
        *pos += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return u32::try_from(value).or_else(|_| err("wasm: LEB128 overflow"));
        }
    }
    err("wasm: LEB128 too long")
}

/// The payload of the first custom section named `name`, if any. Only the
/// section framing is checked.
pub fn custom_section<'a>(wasm: &'a [u8], name: &str) -> Result<Option<&'a [u8]>, LinkError> {
    if wasm.len() < 8 || wasm[..4] != *b"\0asm" {
        return err("wasm: not a module");
    }
    let mut pos = 8;
    while pos < wasm.len() {
        let id = wasm[pos];
        pos += 1;
        let size = leb_u32(wasm, &mut pos)? as usize;
        let Some(section) = pos.checked_add(size).and_then(|end| wasm.get(pos..end)) else {
            return err("wasm: truncated section");
        };
        pos += size;
        if id == 0 {
            let mut at = 0;
            let len = leb_u32(section, &mut at)? as usize;
            let Some(n) = at.checked_add(len).and_then(|end| section.get(at..end)) else {
                return err("wasm: truncated custom section name");
            };
            if n == name.as_bytes() {
                return Ok(Some(&section[at + len..]));
            }
        }
    }
    Ok(None)
}

/// Reads and checks the `x64wasm.blocks` section of a translated module.
pub fn read_blocks(wasm: &[u8]) -> Result<ModuleBlocks, LinkError> {
    match custom_section(wasm, abi::BLOCKS_SECTION)? {
        Some(payload) => decode_blocks(payload),
        None => err("wasm: no x64wasm.blocks section"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_round_trip_and_checks() {
        let blocks = [
            BlockEntry {
                addr: 0x1000,
                chunk: 0,
                entry: 0,
            },
            BlockEntry {
                addr: 0x1010,
                chunk: 1,
                entry: 3,
            },
        ];
        let p = encode_blocks(2, &blocks);
        assert_eq!(p.len(), 16 + 32);
        assert_eq!(
            decode_blocks(&p).unwrap(),
            ModuleBlocks {
                chunks: 2,
                blocks: blocks.to_vec()
            }
        );
        assert!(decode_blocks(&encode_blocks(1, &blocks)).is_err(), "chunk");
        assert!(decode_blocks(&p[..p.len() - 1]).is_err(), "length");
        let mut swapped = blocks;
        swapped.swap(0, 1);
        assert!(decode_blocks(&encode_blocks(2, &swapped)).is_err(), "order");
        for len in 0..p.len() {
            let _ = decode_blocks(&p[..len]);
        }
    }

    #[test]
    fn finds_custom_sections() {
        // Header, a custom section "a" with payload 1 2, a type section, "b".
        let wasm = [
            0, b'a', b's', b'm', 1, 0, 0, 0, 0, 4, 1, b'a', 1, 2, 1, 1, 0, 0, 2, 1, b'b',
        ];
        assert_eq!(custom_section(&wasm, "a").unwrap(), Some(&[1u8, 2][..]));
        assert_eq!(custom_section(&wasm, "b").unwrap(), Some(&[][..]));
        assert_eq!(custom_section(&wasm, "c").unwrap(), None);
        for len in 0..wasm.len() {
            let _ = custom_section(&wasm[..len], "c");
        }
        assert!(custom_section(b"nope", "a").is_err());
    }
}
