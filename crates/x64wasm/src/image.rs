//! Whole PE images (M2): the load plan the JS runtime maps an image from
//! (`docs/design/runtime.md`, "Load plan"), and ahead-of-time translation of
//! an image into one v3 module (`docs/design/translator.md`, "Linking and
//! images").

use std::fmt::{self, Write as _};

use sha2::{Digest, Sha256};

use crate::lift::Source;
use crate::pe::{self, Pe, PeError};
use crate::{CodeRegion, Error, Options, Translation};

/// `IMAGE_FILE_RELOCS_STRIPPED`.
const FILE_RELOCS_STRIPPED: u16 = 0x0001;

/// Lowercase hex SHA-256 of `data`.
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// One section of a [`LoadPlan`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanSection {
    pub name: String,
    pub va: u64,
    pub virtual_size: u32,
    pub file_offset: u32,
    pub file_size: u32,
    pub characteristics: u32,
}

/// One imported function: by name, or by ordinal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanImport {
    pub name: Option<String>,
    pub ordinal: Option<u16>,
    /// VA of its IAT slot.
    pub iat: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanDll {
    pub dll: String,
    pub functions: Vec<PlanImport>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanTls {
    pub start_of_raw_data: u64,
    pub end_of_raw_data: u64,
    pub address_of_index: u64,
    pub size_of_zero_fill: u32,
    /// Callback VAs, in call order.
    pub callbacks: Vec<u64>,
}

/// What the runtime needs to map and start an image (`runtime.md`, "Load
/// plan"). Serialized by [`LoadPlan::to_json`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadPlan {
    pub file: String,
    pub sha256: String,
    pub machine: u16,
    pub image_base: u64,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub entry_point: u64,
    pub subsystem: u16,
    pub stack_reserve: u64,
    pub stack_commit: u64,
    pub sections: Vec<PlanSection>,
    pub imports: Vec<PlanDll>,
    pub tls: Option<PlanTls>,
    pub has_relocations: bool,
}

/// Version of the load plan format.
pub const PLAN_VERSION: u32 = 1;

impl LoadPlan {
    /// The plan of `pe`, whose file is named `file` (no directory). Fails if
    /// the import or TLS directory is damaged.
    pub fn from_pe(pe: &Pe<'_>, file: &str) -> Result<LoadPlan, PeError> {
        let sections = pe
            .sections
            .iter()
            .map(|s| PlanSection {
                name: s.name.clone(),
                va: pe.rva_to_va(s.rva),
                virtual_size: s.mapped_size(),
                file_offset: s.raw_offset,
                file_size: s.raw_size,
                characteristics: s.characteristics,
            })
            .collect();
        let imports = pe
            .imports()?
            .into_iter()
            .map(|d| PlanDll {
                dll: d.name,
                functions: d
                    .functions
                    .into_iter()
                    .map(|f| PlanImport {
                        ordinal: f.name.is_none().then_some(f.ordinal_or_hint),
                        name: f.name,
                        iat: pe.rva_to_va(f.iat_rva),
                    })
                    .collect(),
            })
            .collect();
        let tls = pe.tls()?.map(|t| PlanTls {
            start_of_raw_data: t.start_va,
            end_of_raw_data: t.end_va,
            address_of_index: t.index_va,
            size_of_zero_fill: t.zero_fill,
            callbacks: t.callbacks.iter().map(|&rva| pe.rva_to_va(rva)).collect(),
        });
        let reloc = pe.directory(pe::dir::BASERELOC);
        Ok(LoadPlan {
            file: file.to_string(),
            sha256: sha256_hex(pe.data()),
            machine: pe::MACHINE_AMD64,
            image_base: pe.image_base,
            size_of_image: pe.size_of_image,
            size_of_headers: pe.size_of_headers,
            entry_point: pe.rva_to_va(pe.entry_rva),
            subsystem: pe.subsystem,
            stack_reserve: pe.stack_reserve,
            stack_commit: pe.stack_commit,
            sections,
            imports,
            tls,
            has_relocations: reloc.rva != 0
                && reloc.size != 0
                && pe.characteristics & FILE_RELOCS_STRIPPED == 0,
        })
    }

    /// The plan as JSON (`runtime.md`): u64 values and sizes as `"0x…"`
    /// strings, addresses as VAs.
    pub fn to_json(&self) -> String {
        let mut o = String::new();
        let _ = self.write_json(&mut o);
        o
    }

    fn write_json(&self, o: &mut String) -> fmt::Result {
        writeln!(o, "{{")?;
        writeln!(o, "  \"version\": {PLAN_VERSION},")?;
        writeln!(o, "  \"file\": {},", json_str(&self.file))?;
        writeln!(o, "  \"sha256\": {},", json_str(&self.sha256))?;
        writeln!(o, "  \"machine\": \"{:#x}\",", self.machine)?;
        writeln!(o, "  \"imageBase\": \"{:#x}\",", self.image_base)?;
        writeln!(o, "  \"sizeOfImage\": \"{:#x}\",", self.size_of_image)?;
        writeln!(o, "  \"sizeOfHeaders\": \"{:#x}\",", self.size_of_headers)?;
        writeln!(o, "  \"entryPoint\": \"{:#x}\",", self.entry_point)?;
        writeln!(o, "  \"subsystem\": {},", self.subsystem)?;
        writeln!(o, "  \"stackReserve\": \"{:#x}\",", self.stack_reserve)?;
        writeln!(o, "  \"stackCommit\": \"{:#x}\",", self.stack_commit)?;
        writeln!(o, "  \"sections\": [")?;
        for (i, s) in self.sections.iter().enumerate() {
            let comma = if i + 1 < self.sections.len() { "," } else { "" };
            writeln!(
                o,
                "    {{ \"name\": {}, \"va\": \"{:#x}\", \"virtualSize\": \"{:#x}\", \
                 \"fileOffset\": \"{:#x}\", \"fileSize\": \"{:#x}\", \
                 \"characteristics\": \"{:#x}\" }}{comma}",
                json_str(&s.name),
                s.va,
                s.virtual_size,
                s.file_offset,
                s.file_size,
                s.characteristics
            )?;
        }
        writeln!(o, "  ],")?;
        writeln!(o, "  \"imports\": [")?;
        for (i, d) in self.imports.iter().enumerate() {
            writeln!(o, "    {{ \"dll\": {}, \"functions\": [", json_str(&d.dll))?;
            for (j, f) in d.functions.iter().enumerate() {
                let comma = if j + 1 < d.functions.len() { "," } else { "" };
                let name = f.name.as_deref().map_or("null".into(), json_str);
                let ordinal = f.ordinal.map_or("null".into(), |n| n.to_string());
                writeln!(
                    o,
                    "      {{ \"name\": {name}, \"ordinal\": {ordinal}, \"iat\": \"{:#x}\" }}{comma}",
                    f.iat
                )?;
            }
            let comma = if i + 1 < self.imports.len() { "," } else { "" };
            writeln!(o, "    ] }}{comma}")?;
        }
        writeln!(o, "  ],")?;
        match &self.tls {
            None => writeln!(o, "  \"tls\": null,")?,
            Some(t) => {
                let callbacks: Vec<String> =
                    t.callbacks.iter().map(|c| format!("\"{c:#x}\"")).collect();
                writeln!(
                    o,
                    "  \"tls\": {{ \"startOfRawData\": \"{:#x}\", \"endOfRawData\": \"{:#x}\", \
                     \"addressOfIndex\": \"{:#x}\", \"sizeOfZeroFill\": \"{:#x}\", \
                     \"callbacks\": [{}] }},",
                    t.start_of_raw_data,
                    t.end_of_raw_data,
                    t.address_of_index,
                    t.size_of_zero_fill,
                    callbacks.join(", ")
                )?;
            }
        }
        writeln!(o, "  \"hasRelocations\": {}", self.has_relocations)?;
        writeln!(o, "}}")
    }
}

/// A JSON string literal.
fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Which function entries an image translation starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntrySet {
    /// Every entry of [`Pe::function_entries`].
    All,
    /// The entry point only.
    EntryPoint,
}

/// Why an image could not be translated.
#[derive(Debug)]
pub enum ImageError {
    Pe(PeError),
    Translate(Error),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::Pe(e) => write!(f, "{e}"),
            ImageError::Translate(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ImageError {}

impl From<PeError> for ImageError {
    fn from(e: PeError) -> Self {
        ImageError::Pe(e)
    }
}

impl From<Error> for ImageError {
    fn from(e: Error) -> Self {
        ImageError::Translate(e)
    }
}

/// The executable sections as mapped: `(va, bytes)`, the file data followed
/// by zeros up to the section's size in memory.
pub fn executable_sections(pe: &Pe<'_>) -> Result<Vec<(u64, Vec<u8>)>, PeError> {
    mapped(pe, pe.executable_sections())
}

/// Every section as mapped, like [`executable_sections`].
pub fn all_sections(pe: &Pe<'_>) -> Result<Vec<(u64, Vec<u8>)>, PeError> {
    mapped(pe, pe.sections.iter())
}

fn mapped<'s>(
    pe: &Pe<'_>,
    sections: impl Iterator<Item = &'s pe::Section>,
) -> Result<Vec<(u64, Vec<u8>)>, PeError> {
    let mut out = Vec::new();
    for s in sections {
        let mut bytes = vec![0u8; s.mapped_size() as usize];
        let n = s.file_size() as usize;
        if n > 0 {
            let start = s.raw_offset as usize;
            let src = start
                .checked_add(n)
                .and_then(|end| pe.data().get(start..end))
                .ok_or(PeError::Truncated("section data"))?;
            bytes[..n].copy_from_slice(src);
        }
        out.push((pe.rva_to_va(s.rva), bytes));
    }
    Ok(out)
}

fn regions(sections: &[(u64, Vec<u8>)]) -> Vec<CodeRegion<'_>> {
    sections
        .iter()
        .map(|(base, bytes)| CodeRegion { base: *base, bytes })
        .collect()
}

/// Translates an image into one v3 module. Only executable sections are
/// decoded: a transfer anywhere else is a transfer outside the module. Jump
/// tables are read from any section.
pub fn translate(
    pe: &Pe<'_>,
    entries: EntrySet,
    opts: &Options,
) -> Result<Translation, ImageError> {
    let code = executable_sections(pe)?;
    let data = all_sections(pe)?;
    let (code, data) = (regions(&code), regions(&data));
    let entries: Vec<u64> = match entries {
        EntrySet::All => pe
            .function_entries()
            .iter()
            .map(|e| pe.rva_to_va(e.rva))
            .collect(),
        EntrySet::EntryPoint => vec![pe.rva_to_va(pe.entry_rva)],
    };
    let src = Source {
        code: &code,
        data: &data,
        known: &[],
    };
    Ok(crate::translate_source(&src, &entries, opts)?)
}
