// Copyright 2026 the Tempwind authors
//
// This library is free software; you can redistribute it and/or
// modify it under the terms of the GNU Lesser General Public
// License as published by the Free Software Foundation; either
// version 2.1 of the License, or (at your option) any later version.

//! The rebase of docs/design/wine-build.md, "Packaging for the Unix side".
//!
//! Every closure DLL moves to a unique base in [`RANGE_START`, `RANGE_END`),
//! except `ntdll.dll`, `kernelbase.dll`, `kernel32.dll` and `apisetschema.dll`.
//! A file changes in `ImageBase`, `CheckSum` and its `IMAGE_REL_BASED_DIR64`
//! targets, and nowhere else.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The first base of the DLLs that keep their own: the packing starts below it.
pub const RANGE_END: u64 = 0x1_7000_0000;
/// `TW_IMAGE_RANGE_END` (native/wine/include/tempwind/abi.h): the packing fails below this base.
pub const RANGE_START: u64 = 0x1_6000_0000;
/// Modules that keep their bases (and `apisetschema.dll`, which is data).
pub const KEEP: [&str; 4] = [
    "ntdll.dll",
    "kernelbase.dll",
    "kernel32.dll",
    "apisetschema.dll",
];

const REL_ABSOLUTE: u16 = 0;
const REL_DIR64: u16 = 10;
const FILE_DLL: u16 = 0x2000;

fn rd16(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
fn rd32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}
fn rd64(d: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(o..o + 8)?.try_into().ok()?))
}

struct Section {
    va: u32,
    raw_ptr: u32,
    raw_size: u32,
}

/// The parts of a PE32+ image the rebase needs.
pub struct Pe {
    image_base_off: usize,
    checksum_off: usize,
    pub image_base: u64,
    pub size_of_image: u32,
    pub is_dll: bool,
    sections: Vec<Section>,
    reloc_rva: u32,
    reloc_size: u32,
}

impl Pe {
    /// Parses the headers of `data`; `name` is the file, for errors.
    pub fn parse(name: &str, data: &[u8]) -> Result<Pe> {
        let bad = || anyhow!("{name}: not a PE32+ image (headers do not parse)");
        ensure!(data.get(..2) == Some(b"MZ"), "{name}: no MZ signature");
        let pe = rd32(data, 0x3c).ok_or_else(bad)? as usize;
        ensure!(
            data.get(pe..pe + 4) == Some(b"PE\0\0"),
            "{name}: no PE signature"
        );
        let coff = pe + 4;
        let nsec = rd16(data, coff + 2).ok_or_else(bad)? as usize;
        let opt_size = rd16(data, coff + 16).ok_or_else(bad)? as usize;
        let chars = rd16(data, coff + 18).ok_or_else(bad)?;
        let opt = coff + 20;
        ensure!(
            rd16(data, opt) == Some(0x20b),
            "{name}: not PE32+ (optional header magic)"
        );
        let image_base = rd64(data, opt + 24).ok_or_else(bad)?;
        let size_of_image = rd32(data, opt + 56).ok_or_else(bad)?;
        let ndirs = rd32(data, opt + 108).ok_or_else(bad)? as usize;
        let (reloc_rva, reloc_size) = if ndirs > 5 {
            (
                rd32(data, opt + 112 + 5 * 8).ok_or_else(bad)?,
                rd32(data, opt + 112 + 5 * 8 + 4).ok_or_else(bad)?,
            )
        } else {
            (0, 0)
        };
        let mut sections = Vec::new();
        for i in 0..nsec {
            let s = opt + opt_size + i * 40;
            sections.push(Section {
                va: rd32(data, s + 12).ok_or_else(bad)?,
                raw_size: rd32(data, s + 16).ok_or_else(bad)?,
                raw_ptr: rd32(data, s + 20).ok_or_else(bad)?,
            });
        }
        Ok(Pe {
            image_base_off: opt + 24,
            checksum_off: opt + 64,
            image_base,
            size_of_image,
            is_dll: chars & FILE_DLL != 0,
            sections,
            reloc_rva,
            reloc_size,
        })
    }

    /// The file offset of the 8 bytes at `rva`, which must lie in a section's raw data.
    fn file_offset(&self, rva: u32, len: usize) -> Option<usize> {
        let s = self
            .sections
            .iter()
            .find(|s| rva >= s.va && (rva - s.va) < s.raw_size)?;
        let rel = (rva - s.va) as usize;
        (rel + len <= s.raw_size as usize).then(|| s.raw_ptr as usize + rel)
    }

    /// File offsets of every DIR64 target. ABSOLUTE entries are padding; any other type fails.
    pub fn dir64_targets(&self, name: &str, data: &[u8]) -> Result<Vec<usize>> {
        ensure!(
            self.reloc_size != 0,
            "{name}: no .reloc (base relocation directory is empty), the image cannot move"
        );
        let start = self
            .file_offset(self.reloc_rva, self.reloc_size as usize)
            .with_context(|| {
                format!("{name}: base relocation directory is outside the sections")
            })?;
        let end = start + self.reloc_size as usize;
        let mut out = Vec::new();
        let mut pos = start;
        while pos + 8 <= end {
            let page = rd32(data, pos).context("short reloc")?;
            let size = rd32(data, pos + 4).context("short reloc")? as usize;
            ensure!(
                size >= 8 && size.is_multiple_of(2) && pos + size <= end,
                "{name}: malformed relocation block at file offset {pos:#x}"
            );
            for e in 0..(size - 8) / 2 {
                let v = rd16(data, pos + 8 + e * 2).context("short reloc")?;
                let (ty, off) = (v >> 12, (v & 0xfff) as u32);
                match ty {
                    REL_ABSOLUTE => {}
                    REL_DIR64 => {
                        let o = self
                            .file_offset(page + off, 8)
                            .with_context(|| format!("{name}: DIR64 target at rva {:#x} is outside the sections' raw data", page + off))?;
                        out.push(o);
                    }
                    t => bail!(
                        "{name}: unsupported relocation type {t} at rva {:#x}",
                        page + off
                    ),
                }
            }
            pos += size;
        }
        Ok(out)
    }
}

/// The PE checksum (the imagehlp algorithm): the 16-bit words of the file
/// with the CheckSum field as zero, folded at each step, plus the length.
pub fn checksum(data: &[u8], checksum_off: usize) -> u32 {
    let mut sum: u32 = 0;
    for (i, chunk) in data.chunks(2).enumerate() {
        let at = i * 2;
        let w = if (checksum_off..checksum_off + 4).contains(&at) {
            0
        } else {
            u32::from(chunk[0]) | chunk.get(1).map_or(0, |&b| u32::from(b) << 8)
        };
        sum += w;
        sum = (sum & 0xffff) + (sum >> 16);
    }
    sum = (sum & 0xffff) + (sum >> 16);
    sum + data.len() as u32
}

/// Moves `data` to `new_base`: ImageBase, every DIR64 target by the delta, CheckSum.
pub fn rebase_image(name: &str, data: &[u8], new_base: u64) -> Result<Vec<u8>> {
    let pe = Pe::parse(name, data)?;
    let targets = pe.dir64_targets(name, data)?;
    let delta = new_base.wrapping_sub(pe.image_base);
    let mut out = data.to_vec();
    for t in targets {
        let v = rd64(&out, t).unwrap().wrapping_add(delta);
        out[t..t + 8].copy_from_slice(&v.to_le_bytes());
    }
    out[pe.image_base_off..pe.image_base_off + 8].copy_from_slice(&new_base.to_le_bytes());
    let cs = checksum(&out, pe.checksum_off);
    out[pe.checksum_off..pe.checksum_off + 4].copy_from_slice(&cs.to_le_bytes());
    Ok(out)
}

/// The bases of `modules` (name, SizeOfImage): in name order, each at the
/// highest 64 KiB-aligned base that leaves room below the previous one; the
/// first goes below `RANGE_END`. Fails below `RANGE_START`.
pub fn pack(modules: &[(String, u32)]) -> Result<Vec<(String, u64)>> {
    let mut sorted: Vec<&(String, u32)> = modules.iter().collect();
    sorted.sort();
    let mut top = RANGE_END;
    let mut out = Vec::new();
    for (name, size) in sorted {
        let base = top
            .checked_sub(u64::from(*size))
            .map(|b| b & !0xffff)
            .filter(|&b| b >= RANGE_START);
        let base = base.with_context(|| format!("{name}: size {size:#x} does not fit below {top:#x} (range starts at {RANGE_START:#x})"))?;
        out.push((name.clone(), base));
        top = base;
    }
    Ok(out)
}

#[derive(Deserialize)]
struct Closure {
    wine: String,
    modules: Vec<ClosureModule>,
}

#[derive(Deserialize)]
struct ClosureModule {
    name: String,
    path: String,
    via: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct RebaseJson {
    schema: u32,
    wine: String,
    range: [String; 2],
    modules: Vec<RebasedModule>,
}

#[derive(Serialize, Deserialize)]
struct RebasedModule {
    name: String,
    path: String,
    old_base: String,
    base: String,
    size: String,
    sha256: String,
}

fn hex(n: u64) -> String {
    format!("{n:#x}")
}

fn unhex(s: &str) -> Result<u64> {
    u64::from_str_radix(s.strip_prefix("0x").unwrap_or(s), 16)
        .with_context(|| format!("bad hex {s:?}"))
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn load_closure(path: &Path) -> Result<Closure> {
    let text = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&text).with_context(|| format!("parsing {}", path.display()))
}

/// The closure's modules that move: library DLLs (not roots, not `KEEP`).
fn movers<'a>(tree: &Path, closure: &'a Closure) -> Result<Vec<(&'a ClosureModule, Vec<u8>, Pe)>> {
    let mut out = Vec::new();
    for m in &closure.modules {
        if m.via.iter().any(|v| v == "root") || KEEP.contains(&m.name.as_str()) {
            continue;
        }
        let file = tree.join(&m.path);
        let data = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
        let pe = Pe::parse(&m.path, &data)?;
        if !pe.is_dll {
            continue; // an EXE keeps its base
        }
        out.push((m, data, pe));
    }
    Ok(out)
}

/// Rebases the closure's DLLs from `tree` into `out` and writes `out/rebase.json`.
/// Returns how many moved.
pub fn rebase(tree: &Path, closure_path: &Path, out: &Path) -> Result<usize> {
    let closure = load_closure(closure_path)?;
    let mut mods = movers(tree, &closure)?;
    let sizes: Vec<(String, u32)> = mods
        .iter()
        .map(|(m, _, pe)| (m.name.clone(), pe.size_of_image))
        .collect();
    let bases = pack(&sizes)?;
    mods.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    let _ = std::fs::remove_dir_all(out.join("lib"));
    let mut list = Vec::new();
    for ((m, data, pe), (_, base)) in mods.iter().zip(&bases) {
        let moved = rebase_image(&m.path, data, *base)?;
        let dest = out.join(&m.path);
        std::fs::create_dir_all(dest.parent().unwrap())?;
        std::fs::write(&dest, &moved).with_context(|| format!("writing {}", dest.display()))?;
        list.push(RebasedModule {
            name: m.name.clone(),
            path: m.path.clone(),
            old_base: hex(pe.image_base),
            base: hex(*base),
            size: hex(u64::from(pe.size_of_image)),
            sha256: sha256_hex(&moved),
        });
    }
    let n = list.len();
    let json = RebaseJson {
        schema: 1,
        wine: closure.wine,
        range: [hex(RANGE_START), hex(RANGE_END)],
        modules: list,
    };
    std::fs::create_dir_all(out)?;
    std::fs::write(
        out.join("rebase.json"),
        serde_json::to_string_pretty(&json)? + "\n",
    )?;
    Ok(n)
}

/// Compares every file `rebase.json` lists with its original: only `ImageBase`,
/// `CheckSum` and the DIR64 targets may differ, the targets by exactly the delta.
/// Also checks the bases (range, alignment, no overlap) and that every DLL that
/// should have moved is listed. Returns how many modules it checked.
pub fn verify(tree: &Path, closure_path: &Path, out: &Path) -> Result<usize> {
    let closure = load_closure(closure_path)?;
    let text = std::fs::read(out.join("rebase.json")).context("reading rebase.json")?;
    let json: RebaseJson = serde_json::from_slice(&text).context("parsing rebase.json")?;
    ensure!(
        json.schema == 1,
        "rebase.json: schema {} is not 1",
        json.schema
    );
    ensure!(
        json.wine == closure.wine,
        "rebase.json is for {}, closure.json for {}",
        json.wine,
        closure.wine
    );
    let expected: BTreeSet<String> = movers(tree, &closure)?
        .into_iter()
        .map(|(m, _, _)| m.name.clone())
        .collect();
    let listed: BTreeSet<String> = json.modules.iter().map(|m| m.name.clone()).collect();
    ensure!(
        expected == listed,
        "rebase.json lists a different set of modules than the closure implies: missing {:?}, extra {:?}",
        expected.difference(&listed).collect::<Vec<_>>(),
        listed.difference(&expected).collect::<Vec<_>>()
    );
    let mut ranges: Vec<(u64, u64, &str)> = Vec::new();
    for m in &json.modules {
        let orig = std::fs::read(tree.join(&m.path))
            .with_context(|| format!("reading original {}", m.path))?;
        let moved = std::fs::read(out.join(&m.path))
            .with_context(|| format!("reading rebased {}", m.path))?;
        ensure!(
            orig.len() == moved.len(),
            "{}: size changed ({} -> {})",
            m.name,
            orig.len(),
            moved.len()
        );
        ensure!(
            sha256_hex(&moved) == m.sha256,
            "{}: sha256 differs from rebase.json",
            m.name
        );
        let (base, old_base, size) = (unhex(&m.base)?, unhex(&m.old_base)?, unhex(&m.size)?);
        ensure!(
            base.is_multiple_of(0x1_0000) && base >= RANGE_START && base + size <= RANGE_END,
            "{}: base {:#x} size {:#x} is outside the range or unaligned",
            m.name,
            base,
            size
        );
        let po = Pe::parse(&m.path, &orig)?;
        let pm = Pe::parse(&m.path, &moved)?;
        ensure!(
            po.image_base == old_base && pm.image_base == base,
            "{}: ImageBase {:#x} -> {:#x}, rebase.json says {:#x} -> {:#x}",
            m.name,
            po.image_base,
            pm.image_base,
            old_base,
            base
        );
        ensure!(
            u64::from(po.size_of_image) == size,
            "{}: SizeOfImage differs from rebase.json",
            m.name
        );
        let delta = base.wrapping_sub(old_base);
        let mut allowed: Vec<(usize, usize)> = vec![(po.image_base_off, 8), (po.checksum_off, 4)];
        for t in po.dir64_targets(&m.path, &orig)? {
            let (a, b) = (rd64(&orig, t).unwrap(), rd64(&moved, t).unwrap());
            ensure!(
                b == a.wrapping_add(delta),
                "{}: DIR64 target at file offset {t:#x} is {b:#x}, expected {:#x}",
                m.name,
                a.wrapping_add(delta)
            );
            allowed.push((t, 8));
        }
        let cs = rd32(&moved, pm.checksum_off).unwrap();
        ensure!(
            cs == checksum(&moved, pm.checksum_off),
            "{}: CheckSum {cs:#x} is not the file's checksum",
            m.name
        );
        for (i, (a, b)) in orig.iter().zip(&moved).enumerate() {
            if a != b {
                ensure!(
                    allowed.iter().any(|&(s, l)| i >= s && i < s + l),
                    "{}: byte at file offset {i:#x} changed outside ImageBase, CheckSum and the DIR64 targets",
                    m.name
                );
            }
        }
        ranges.push((base, base + size, &m.name));
    }
    // The modules that keep their bases take part in the overlap check.
    for m in closure
        .modules
        .iter()
        .filter(|m| KEEP.contains(&m.name.as_str()))
    {
        let data =
            std::fs::read(tree.join(&m.path)).with_context(|| format!("reading {}", m.path))?;
        let pe = Pe::parse(&m.path, &data)?;
        ranges.push((
            pe.image_base,
            pe.image_base + u64::from(pe.size_of_image),
            &m.name,
        ));
    }
    ranges.sort();
    for w in ranges.windows(2) {
        ensure!(
            w[0].1 <= w[1].0,
            "{} [{:#x}, {:#x}) overlaps {} [{:#x}, ...)",
            w[0].2,
            w[0].0,
            w[0].1,
            w[1].2,
            w[1].0
        );
    }
    Ok(json.modules.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Img {
        base: u64,
        dll: bool,
        /// DIR64 targets (rva in .text), ABSOLUTE padding, and other raw entries (type, rva).
        dir64: Vec<u32>,
        extra: Vec<(u16, u32)>,
        reloc: bool,
        size_of_image: u32,
    }

    impl Default for Img {
        fn default() -> Img {
            Img {
                base: 0x1_8000_0000,
                dll: true,
                dir64: vec![0x1000, 0x1010],
                extra: vec![],
                reloc: true,
                size_of_image: 0x3000,
            }
        }
    }

    /// A PE32+ of two sections: .text (rva 0x1000, file 0x200, 0x200 bytes of
    /// pointers to `base + 0x1100`) and .reloc (rva 0x2000, file 0x400).
    fn build(i: &Img) -> Vec<u8> {
        let mut d = vec![0u8; 0x600];
        d[..2].copy_from_slice(b"MZ");
        d[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        d[0x40..0x44].copy_from_slice(b"PE\0\0");
        let coff = 0x44;
        d[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
        d[coff + 2..coff + 4].copy_from_slice(&2u16.to_le_bytes());
        d[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
        let chars: u16 = 0x22 | if i.dll { 0x2000 } else { 0 };
        d[coff + 18..coff + 20].copy_from_slice(&chars.to_le_bytes());
        let opt = coff + 20;
        d[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        d[opt + 24..opt + 32].copy_from_slice(&i.base.to_le_bytes());
        d[opt + 56..opt + 60].copy_from_slice(&i.size_of_image.to_le_bytes());
        d[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        let mut block = Vec::<u8>::new();
        if i.reloc {
            block.extend_from_slice(&0x1000u32.to_le_bytes());
            block.extend_from_slice(&0u32.to_le_bytes());
            for &rva in &i.dir64 {
                block.extend_from_slice(&((REL_DIR64 << 12) | (rva & 0xfff) as u16).to_le_bytes());
            }
            for &(ty, rva) in &i.extra {
                block.extend_from_slice(&((ty << 12) | (rva & 0xfff) as u16).to_le_bytes());
            }
            block.extend_from_slice(&0u16.to_le_bytes()); // ABSOLUTE padding
            let n = block.len() as u32;
            block[4..8].copy_from_slice(&n.to_le_bytes());
            d[opt + 112 + 40..opt + 112 + 44].copy_from_slice(&0x2000u32.to_le_bytes());
            d[opt + 112 + 44..opt + 112 + 48].copy_from_slice(&n.to_le_bytes());
            d[0x400..0x400 + block.len()].copy_from_slice(&block);
        }
        let s = opt + 240;
        for (k, (va, raw)) in [(0x1000u32, 0x200u32), (0x2000, 0x400)].iter().enumerate() {
            let o = s + k * 40;
            d[o + 8..o + 12].copy_from_slice(&0x200u32.to_le_bytes());
            d[o + 12..o + 16].copy_from_slice(&va.to_le_bytes());
            d[o + 16..o + 20].copy_from_slice(&0x200u32.to_le_bytes());
            d[o + 20..o + 24].copy_from_slice(&raw.to_le_bytes());
        }
        for &rva in &i.dir64 {
            let o = 0x200 + (rva - 0x1000) as usize;
            d[o..o + 8].copy_from_slice(&(i.base + 0x1100).to_le_bytes());
        }
        // Bytes the relocation must leave alone, next to the targets.
        d[0x200 + 0x8..0x200 + 0x10].copy_from_slice(&0x1122334455667788u64.to_le_bytes());
        d
    }

    fn at(d: &[u8], o: usize) -> u64 {
        rd64(d, o).unwrap()
    }

    #[test]
    fn dir64_targets_move_by_the_delta() {
        let img = Img::default();
        let d = build(&img);
        let new = 0x1_6ff0_0000u64;
        let r = rebase_image("t.dll", &d, new).unwrap();
        let delta = new.wrapping_sub(img.base);
        assert_eq!(at(&r, 0x200), (img.base + 0x1100).wrapping_add(delta));
        assert_eq!(at(&r, 0x210), (img.base + 0x1100).wrapping_add(delta));
        assert_eq!(at(&r, 0x208), 0x1122334455667788, "not a target");
        let pe = Pe::parse("t.dll", &r).unwrap();
        assert_eq!(pe.image_base, new);
        // Nothing else differs but the checksum.
        let diffs: Vec<usize> = (0..d.len()).filter(|&k| d[k] != r[k]).collect();
        assert!(
            diffs.iter().all(|&k| (0x200..0x208).contains(&k)
                || (0x210..0x218).contains(&k)
                || (pe.image_base_off..pe.image_base_off + 8).contains(&k)
                || (pe.checksum_off..pe.checksum_off + 4).contains(&k)),
            "{diffs:x?}"
        );
        // The checksum is the file's.
        assert_eq!(
            rd32(&r, pe.checksum_off).unwrap(),
            checksum(&r, pe.checksum_off)
        );
    }

    #[test]
    fn rebasing_up_wraps_by_the_delta() {
        let d = build(&Img::default());
        let r = rebase_image("t.dll", &d, 0x1_9000_0000).unwrap();
        assert_eq!(at(&r, 0x200), 0x1_9000_1100);
    }

    #[test]
    fn absolute_padding_is_ignored() {
        let img = Img {
            dir64: vec![],
            ..Img::default()
        };
        let d = build(&img); // the block holds only the ABSOLUTE entry
        let r = rebase_image("t.dll", &d, 0x1_6000_0000).unwrap();
        assert_eq!(&d[0x200..0x400], &r[0x200..0x400]);
    }

    #[test]
    fn other_relocation_types_fail_naming_the_file() {
        let img = Img {
            extra: vec![(3, 0x20)],
            ..Img::default()
        }; // HIGHLOW
        let e = rebase_image("lib/odd.dll", &build(&img), 0x1_6000_0000)
            .unwrap_err()
            .to_string();
        assert!(e.contains("lib/odd.dll") && e.contains("type 3"), "{e}");
    }

    #[test]
    fn an_image_without_reloc_fails_naming_the_file() {
        let img = Img {
            reloc: false,
            ..Img::default()
        };
        let e = rebase_image("lib/fixed.dll", &build(&img), 0x1_6000_0000)
            .unwrap_err()
            .to_string();
        assert!(e.contains("lib/fixed.dll") && e.contains(".reloc"), "{e}");
    }

    #[test]
    fn a_target_outside_the_raw_data_fails() {
        let img = Img {
            dir64: vec![0x11f8],
            ..Img::default()
        }; // the last 8 bytes of .text
        assert!(rebase_image("a.dll", &build(&img), 0x1_6000_0000).is_ok());
        let img = Img {
            dir64: vec![0x11fc],
            ..Img::default()
        }; // crosses the end of the raw data
        let e = rebase_image("b.dll", &build(&img), 0x1_6000_0000)
            .unwrap_err()
            .to_string();
        assert!(e.contains("b.dll"), "{e}");
    }

    #[test]
    fn exes_are_not_dlls() {
        let d = build(&Img {
            dll: false,
            ..Img::default()
        });
        assert!(!Pe::parse("a.exe", &d).unwrap().is_dll);
    }

    fn sizes(v: &[(&str, u32)]) -> Vec<(String, u32)> {
        v.iter().map(|&(n, s)| (n.to_string(), s)).collect()
    }

    #[test]
    fn packing_is_in_name_order_aligned_and_below_the_previous() {
        let p = pack(&sizes(&[
            ("b.dll", 0x1_0000),
            ("a.dll", 0x1234),
            ("c.dll", 0x2_0001),
        ]))
        .unwrap();
        assert_eq!(
            p.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["a.dll", "b.dll", "c.dll"]
        );
        assert_eq!(
            p[0].1, 0x1_6fff_0000,
            "highest aligned base with room below RANGE_END"
        );
        assert_eq!(p[1].1, 0x1_6ffe_0000);
        assert_eq!(p[2].1, 0x1_6ffb_0000);
        for (w, size) in p.iter().zip([0x1234u64, 0x1_0000, 0x2_0001]) {
            assert_eq!(w.1 % 0x1_0000, 0);
            assert!(w.1 + size <= RANGE_END);
        }
        // Deterministic: the input order does not matter.
        let q = pack(&sizes(&[
            ("c.dll", 0x2_0001),
            ("a.dll", 0x1234),
            ("b.dll", 0x1_0000),
        ]))
        .unwrap();
        assert_eq!(p, q);
    }

    #[test]
    fn packing_stops_at_the_range_bound() {
        let whole = (RANGE_END - RANGE_START) as u32;
        let p = pack(&sizes(&[("a.dll", whole)])).unwrap();
        assert_eq!(p[0].1, RANGE_START, "a base equal to the bound fits");
        let e = pack(&sizes(&[("a.dll", whole - 0x1_0000), ("b.dll", 0x1_0001)]))
            .unwrap_err()
            .to_string();
        assert!(e.contains("b.dll"), "{e}");
        assert!(
            pack(&sizes(&[("big.dll", whole + 1)]))
                .unwrap_err()
                .to_string()
                .contains("big.dll")
        );
    }

    /// The packing's lower bound is the ABI's `TW_IMAGE_RANGE_END`, read from the header file
    /// only (the crate depends on none of ours).
    #[test]
    fn range_start_is_the_abi_headers_image_range_end() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../include/tempwind/abi.h");
        let text = std::fs::read_to_string(path).unwrap();
        let line = text
            .lines()
            .find(|l| l.starts_with("#define TW_IMAGE_RANGE_END"))
            .expect("abi.h defines TW_IMAGE_RANGE_END");
        let value = line
            .split("UINT64_C(")
            .nth(1)
            .and_then(|r| r.split(')').next())
            .unwrap();
        let value = u64::from_str_radix(value.trim_start_matches("0x"), 16).unwrap();
        assert_eq!(value, RANGE_START);
    }

    #[test]
    fn checksum_known_vectors() {
        // Words 1, 2: sum 3, plus the length 4.
        assert_eq!(checksum(&[1, 0, 2, 0], usize::MAX - 8), 7);
        // Words 0xffff, 2: the carry folds back (0x10001 -> 2), plus 4.
        assert_eq!(checksum(&[0xff, 0xff, 2, 0], usize::MAX - 8), 6);
        // An odd length pads the last word with zero: words 1, 3, plus 3.
        assert_eq!(checksum(&[1, 0, 3], usize::MAX - 8), 7);
        // The CheckSum field counts as zero: words 5, [field], 1.
        assert_eq!(checksum(&[5, 0, 0xaa, 0xbb, 0xcc, 0xdd, 1, 0], 2), 6 + 8);
    }
}
