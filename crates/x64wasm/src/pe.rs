//! PE32+ (x86-64) image reader: headers, sections, imports, exports,
//! relocations, exception data (`.pdata`), TLS and the COFF symbol table.
//! Read-only parsing, shared by the census (`testkit census`) and, from M2,
//! ahead-of-time translation of whole images. Built in Phase 1 M1 track B.
//!
//! `Pe::parse` reads the headers and the section table. Each directory has its
//! own accessor that parses on demand, so a damaged directory doesn't stop the
//! rest of the image from being read. Nothing here panics on bad input: every
//! out-of-range read becomes a [`PeError`].
//!
//! Terms: an RVA is an offset from the image base once loaded; a VA is
//! `image_base + RVA`; a file offset indexes the bytes given to `parse`.

use std::fmt;

/// Machine type of x86-64 images.
pub const MACHINE_AMD64: u16 = 0x8664;
/// `IMAGE_SCN_MEM_EXECUTE`.
pub const SCN_MEM_EXECUTE: u32 = 0x2000_0000;
/// `IMAGE_SCN_CNT_CODE`.
pub const SCN_CNT_CODE: u32 = 0x0000_0020;

/// Why an image could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeError {
    /// A read went past the end of the file; names the structure.
    Truncated(&'static str),
    /// A field has a value the format doesn't allow; names what and why.
    Invalid(&'static str),
    /// A valid image of a kind this reader doesn't handle (PE32, other machines).
    Unsupported(&'static str),
    /// An RVA that no section maps to file bytes.
    UnmappedRva(u32),
}

impl fmt::Display for PeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeError::Truncated(what) => write!(f, "truncated PE image: {what}"),
            PeError::Invalid(what) => write!(f, "invalid PE image: {what}"),
            PeError::Unsupported(what) => write!(f, "unsupported PE image: {what}"),
            PeError::UnmappedRva(rva) => write!(f, "RVA {rva:#x} is not backed by file data"),
        }
    }
}

impl std::error::Error for PeError {}

pub type Result<T, E = PeError> = std::result::Result<T, E>;

/// Bounds-checked little-endian reads.
fn bytes<'a>(data: &'a [u8], off: usize, len: usize, what: &'static str) -> Result<&'a [u8]> {
    off.checked_add(len)
        .and_then(|end| data.get(off..end))
        .ok_or(PeError::Truncated(what))
}

fn u16_at(data: &[u8], off: usize, what: &'static str) -> Result<u16> {
    let b = bytes(data, off, 2, what)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(data: &[u8], off: usize, what: &'static str) -> Result<u32> {
    let b = bytes(data, off, 4, what)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u64_at(data: &[u8], off: usize, what: &'static str) -> Result<u64> {
    let b = bytes(data, off, 8, what)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(b);
    Ok(u64::from_le_bytes(a))
}

/// Index of the data directories.
pub mod dir {
    pub const EXPORT: usize = 0;
    pub const IMPORT: usize = 1;
    pub const EXCEPTION: usize = 3;
    pub const BASERELOC: usize = 5;
    pub const TLS: usize = 9;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DataDirectory {
    pub rva: u32,
    pub size: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// Name; long names (`/4` in the header) are resolved through the COFF string table.
    pub name: String,
    pub virtual_size: u32,
    pub rva: u32,
    pub raw_size: u32,
    pub raw_offset: u32,
    pub characteristics: u32,
}

impl Section {
    pub fn is_executable(&self) -> bool {
        self.characteristics & SCN_MEM_EXECUTE != 0
    }

    /// Size of the section in memory: `virtual_size`, or the raw size when the
    /// linker left it zero.
    pub fn mapped_size(&self) -> u32 {
        if self.virtual_size == 0 {
            self.raw_size
        } else {
            self.virtual_size
        }
    }

    /// Bytes of the section backed by the file, never more than `mapped_size`.
    pub fn file_size(&self) -> u32 {
        self.raw_size.min(self.mapped_size())
    }

    pub fn contains_rva(&self, rva: u32) -> bool {
        rva >= self.rva && (rva - self.rva) < self.mapped_size()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedFunction {
    /// `None` when imported by ordinal.
    pub name: Option<String>,
    /// Ordinal (by-ordinal imports) or the hint (by-name imports).
    pub ordinal_or_hint: u16,
    /// RVA of this function's slot in the import address table.
    pub iat_rva: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportDll {
    pub name: String,
    pub functions: Vec<ImportedFunction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    pub ordinal: u32,
    pub name: Option<String>,
    /// Function RVA; for a forwarder, the RVA of the forwarder string.
    pub rva: u32,
    /// `"DLL.Function"` when the export forwards to another DLL.
    pub forwarder: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exports {
    /// Name of the DLL as recorded in the export directory.
    pub dll_name: String,
    pub ordinal_base: u32,
    pub exports: Vec<Export>,
}

/// One base relocation that rewrites a pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relocation {
    /// RVA of the field to patch.
    pub rva: u32,
    /// 3 = HIGHLOW (32-bit), 10 = DIR64 (64-bit); others are kept as found.
    pub kind: u8,
}

pub const RELOC_HIGHLOW: u8 = 3;
pub const RELOC_DIR64: u8 = 10;

/// An entry of the `.pdata` exception directory: one function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeFunction {
    pub begin_rva: u32,
    /// One past the last byte.
    pub end_rva: u32,
    pub unwind_rva: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tls {
    pub start_va: u64,
    pub end_va: u64,
    pub index_va: u64,
    pub zero_fill: u32,
    pub characteristics: u32,
    /// RVAs of the TLS callbacks, in call order.
    pub callbacks: Vec<u32>,
}

/// A COFF symbol (auxiliary records are skipped).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub value: u32,
    /// 1-based section number; 0 = undefined, -1 = absolute, -2 = debug.
    pub section: i16,
    pub typ: u16,
    pub storage_class: u8,
}

impl Symbol {
    /// The symbol's type says "function" (`DTYPE_FUNCTION` in the high byte).
    pub fn is_function(&self) -> bool {
        self.typ >> 4 == 2
    }
}

/// Why an address is a function entry for ahead-of-time translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntryKind {
    EntryPoint,
    Export,
    TlsCallback,
    /// Start of a `.pdata` RUNTIME_FUNCTION.
    Pdata,
    /// Target of a 64-bit base relocation that points into code.
    Relocation,
    /// A function symbol of the COFF symbol table.
    Symbol,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionEntry {
    pub rva: u32,
    pub kind: EntryKind,
}

/// A parsed PE32+ x86-64 image, borrowing the file bytes.
#[derive(Debug)]
pub struct Pe<'a> {
    data: &'a [u8],
    pub characteristics: u16,
    pub timestamp: u32,
    pub entry_rva: u32,
    pub image_base: u64,
    pub section_alignment: u32,
    pub file_alignment: u32,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub subsystem: u16,
    pub dll_characteristics: u16,
    pub stack_reserve: u64,
    pub stack_commit: u64,
    pub heap_reserve: u64,
    pub heap_commit: u64,
    pub data_directories: Vec<DataDirectory>,
    pub sections: Vec<Section>,
    symtab_offset: u32,
    symtab_count: u32,
}

impl<'a> Pe<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Pe<'a>> {
        if bytes(data, 0, 2, "DOS header")? != b"MZ" {
            return Err(PeError::Invalid("missing MZ signature"));
        }
        let pe_off = u32_at(data, 0x3c, "DOS header e_lfanew")? as usize;
        if bytes(data, pe_off, 4, "PE signature")? != b"PE\0\0" {
            return Err(PeError::Invalid("missing PE signature"));
        }
        let coff = pe_off + 4;
        let machine = u16_at(data, coff, "COFF header")?;
        if machine != MACHINE_AMD64 {
            return Err(PeError::Unsupported("machine is not x86-64"));
        }
        let nsections = u16_at(data, coff + 2, "COFF header")? as usize;
        let timestamp = u32_at(data, coff + 4, "COFF header")?;
        let symtab_offset = u32_at(data, coff + 8, "COFF header")?;
        let symtab_count = u32_at(data, coff + 12, "COFF header")?;
        let opt_size = u16_at(data, coff + 16, "COFF header")? as usize;
        let characteristics = u16_at(data, coff + 18, "COFF header")?;

        let opt = coff + 20;
        match u16_at(data, opt, "optional header")? {
            0x20b => {}
            0x10b => return Err(PeError::Unsupported("PE32 (32-bit) image")),
            _ => return Err(PeError::Invalid("unknown optional header magic")),
        }
        if opt_size < 112 {
            return Err(PeError::Invalid("optional header too small for PE32+"));
        }
        let entry_rva = u32_at(data, opt + 16, "optional header")?;
        let image_base = u64_at(data, opt + 24, "optional header")?;
        let section_alignment = u32_at(data, opt + 32, "optional header")?;
        let file_alignment = u32_at(data, opt + 36, "optional header")?;
        let size_of_image = u32_at(data, opt + 56, "optional header")?;
        let size_of_headers = u32_at(data, opt + 60, "optional header")?;
        let subsystem = u16_at(data, opt + 68, "optional header")?;
        let dll_characteristics = u16_at(data, opt + 70, "optional header")?;
        let stack_reserve = u64_at(data, opt + 72, "optional header")?;
        let stack_commit = u64_at(data, opt + 80, "optional header")?;
        let heap_reserve = u64_at(data, opt + 88, "optional header")?;
        let heap_commit = u64_at(data, opt + 96, "optional header")?;
        let ndirs = u32_at(data, opt + 108, "optional header")? as usize;
        if ndirs > 16 || 112 + ndirs * 8 > opt_size {
            return Err(PeError::Invalid("bad NumberOfRvaAndSizes"));
        }
        let mut data_directories = Vec::with_capacity(ndirs);
        for i in 0..ndirs {
            let off = opt + 112 + i * 8;
            data_directories.push(DataDirectory {
                rva: u32_at(data, off, "data directory")?,
                size: u32_at(data, off + 4, "data directory")?,
            });
        }

        let mut pe = Pe {
            data,
            characteristics,
            timestamp,
            entry_rva,
            image_base,
            section_alignment,
            file_alignment,
            size_of_image,
            size_of_headers,
            subsystem,
            dll_characteristics,
            stack_reserve,
            stack_commit,
            heap_reserve,
            heap_commit,
            data_directories,
            sections: Vec::with_capacity(nsections),
            symtab_offset,
            symtab_count,
        };
        let table = opt + opt_size;
        for i in 0..nsections {
            let off = table + i * 40;
            let raw_name = bytes(data, off, 8, "section header")?;
            let end = raw_name.iter().position(|&b| b == 0).unwrap_or(8);
            let name = String::from_utf8_lossy(&raw_name[..end]).into_owned();
            pe.sections.push(Section {
                name,
                virtual_size: u32_at(data, off + 8, "section header")?,
                rva: u32_at(data, off + 12, "section header")?,
                raw_size: u32_at(data, off + 16, "section header")?,
                raw_offset: u32_at(data, off + 20, "section header")?,
                characteristics: u32_at(data, off + 36, "section header")?,
            });
        }
        // MinGW names sections longer than 8 bytes "/<decimal offset>" into the
        // string table (".debug_info", ".eh_frame" is short enough, ".debug_*" is not).
        for i in 0..pe.sections.len() {
            let name = &pe.sections[i].name;
            if let Some(num) = name.strip_prefix('/')
                && let Ok(off) = num.parse::<u32>()
                && let Ok(long) = pe.string_table_entry(off)
            {
                pe.sections[i].name = long;
            }
        }
        Ok(pe)
    }

    /// The bytes given to `parse`.
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub fn directory(&self, index: usize) -> DataDirectory {
        self.data_directories
            .get(index)
            .copied()
            .unwrap_or_default()
    }

    pub fn executable_sections(&self) -> impl Iterator<Item = &Section> {
        self.sections.iter().filter(|s| s.is_executable())
    }

    pub fn section_by_name(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    pub fn section_for_rva(&self, rva: u32) -> Option<&Section> {
        self.sections.iter().find(|s| s.contains_rva(rva))
    }

    pub fn rva_to_va(&self, rva: u32) -> u64 {
        self.image_base.wrapping_add(rva as u64)
    }

    /// `None` if the VA is below the image base or beyond a 32-bit RVA.
    pub fn va_to_rva(&self, va: u64) -> Option<u32> {
        va.checked_sub(self.image_base)
            .and_then(|r| u32::try_from(r).ok())
    }

    /// File offset of an RVA, if file bytes back it (headers or a section's raw data).
    pub fn rva_to_offset(&self, rva: u32) -> Option<usize> {
        if let Some(s) = self.section_for_rva(rva) {
            let delta = rva - s.rva;
            if delta < s.file_size() {
                return (s.raw_offset as usize).checked_add(delta as usize);
            }
            return None;
        }
        if rva < self.size_of_headers {
            return Some(rva as usize);
        }
        None
    }

    /// The `len` bytes at `rva`, which must lie in one section's file data.
    pub fn read_rva(&self, rva: u32, len: usize) -> Result<&'a [u8]> {
        let off = self.rva_to_offset(rva).ok_or(PeError::UnmappedRva(rva))?;
        if len > 0 {
            // The last byte must be backed by the same section.
            let last = u32::try_from(len - 1)
                .ok()
                .and_then(|n| rva.checked_add(n))
                .ok_or(PeError::UnmappedRva(rva))?;
            if self.rva_to_offset(last) != off.checked_add(len - 1) {
                return Err(PeError::UnmappedRva(rva));
            }
        }
        bytes(self.data, off, len, "image data")
    }

    fn rva_u16(&self, rva: u32) -> Result<u16> {
        let b = self.read_rva(rva, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn rva_u32(&self, rva: u32) -> Result<u32> {
        let b = self.read_rva(rva, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn rva_u64(&self, rva: u32) -> Result<u64> {
        let b = self.read_rva(rva, 8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    /// NUL-terminated string at `rva`.
    pub fn read_cstr(&self, rva: u32) -> Result<String> {
        let off = self.rva_to_offset(rva).ok_or(PeError::UnmappedRva(rva))?;
        let s = self.section_for_rva(rva);
        // Stop at the end of the backing section's file data (or the file).
        let limit = match s {
            Some(s) => {
                let end = s.raw_offset as usize + s.file_size() as usize;
                end.min(self.data.len())
            }
            None => self.data.len(),
        };
        let tail = self
            .data
            .get(off..limit)
            .ok_or(PeError::Truncated("string"))?;
        let len = tail
            .iter()
            .position(|&b| b == 0)
            .ok_or(PeError::Truncated("unterminated string"))?;
        Ok(String::from_utf8_lossy(&tail[..len]).into_owned())
    }

    /// Import directory: one entry per DLL with its functions in table order.
    pub fn imports(&self) -> Result<Vec<ImportDll>> {
        let d = self.directory(dir::IMPORT);
        if d.rva == 0 {
            return Ok(Vec::new());
        }
        let mut dlls = Vec::new();
        let mut desc = d.rva;
        loop {
            let b = self.read_rva(desc, 20)?;
            if b.iter().all(|&x| x == 0) {
                break;
            }
            let ilt = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            let name_rva = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
            let iat = u32::from_le_bytes([b[16], b[17], b[18], b[19]]);
            let name = self.read_cstr(name_rva)?;
            let lookup = if ilt != 0 { ilt } else { iat };
            let mut functions = Vec::new();
            for i in 0u32.. {
                let thunk = self.rva_u64(lookup.wrapping_add(i.wrapping_mul(8)))?;
                if thunk == 0 {
                    break;
                }
                let iat_rva = iat.wrapping_add(i.wrapping_mul(8));
                if thunk >> 63 != 0 {
                    functions.push(ImportedFunction {
                        name: None,
                        ordinal_or_hint: (thunk & 0xffff) as u16,
                        iat_rva,
                    });
                } else {
                    let hint_rva = (thunk & 0x7fff_ffff) as u32;
                    functions.push(ImportedFunction {
                        name: Some(self.read_cstr(hint_rva.wrapping_add(2))?),
                        ordinal_or_hint: self.rva_u16(hint_rva)?,
                        iat_rva,
                    });
                }
            }
            dlls.push(ImportDll { name, functions });
            desc = desc
                .checked_add(20)
                .ok_or(PeError::Invalid("import table"))?;
        }
        Ok(dlls)
    }

    /// Export directory, or `None` if the image has none.
    pub fn exports(&self) -> Result<Option<Exports>> {
        let d = self.directory(dir::EXPORT);
        if d.rva == 0 {
            return Ok(None);
        }
        let at = |o: u32| d.rva.wrapping_add(o);
        let base = self.rva_u32(at(16))?;
        let nfuncs = self.rva_u32(at(20))?;
        let nnames = self.rva_u32(at(24))?;
        let funcs = self.rva_u32(at(28))?;
        let names = self.rva_u32(at(32))?;
        let ordinals = self.rva_u32(at(36))?;
        let dll_name = self.read_cstr(self.rva_u32(at(12))?)?;
        // Reading the whole tables first rejects absurd counts before allocating.
        let func_tbl = self.read_rva(funcs, (nfuncs as usize).saturating_mul(4))?;
        let name_tbl = self.read_rva(names, (nnames as usize).saturating_mul(4))?;
        let ord_tbl = self.read_rva(ordinals, (nnames as usize).saturating_mul(2))?;
        let mut exports: Vec<Export> = Vec::with_capacity(nfuncs as usize);
        for (i, c) in func_tbl.chunks_exact(4).enumerate() {
            let rva = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            // A function RVA inside the export directory is a forwarder string.
            let forwarder = if rva >= d.rva && rva - d.rva < d.size {
                Some(self.read_cstr(rva)?)
            } else {
                None
            };
            exports.push(Export {
                ordinal: base.wrapping_add(i as u32),
                name: None,
                rva,
                forwarder,
            });
        }
        for (n, o) in name_tbl.chunks_exact(4).zip(ord_tbl.chunks_exact(2)) {
            let name_rva = u32::from_le_bytes([n[0], n[1], n[2], n[3]]);
            let index = u16::from_le_bytes([o[0], o[1]]) as usize;
            let export = exports
                .get_mut(index)
                .ok_or(PeError::Invalid("export name ordinal out of range"))?;
            export.name = Some(self.read_cstr(name_rva)?);
        }
        Ok(Some(Exports {
            dll_name,
            ordinal_base: base,
            exports,
        }))
    }

    /// Base relocations that patch a pointer (padding entries are skipped).
    pub fn relocations(&self) -> Result<Vec<Relocation>> {
        let d = self.directory(dir::BASERELOC);
        if d.rva == 0 || d.size == 0 {
            return Ok(Vec::new());
        }
        let table = self.read_rva(d.rva, d.size as usize)?;
        let mut relocs = Vec::new();
        let mut pos = 0usize;
        while pos < table.len() {
            let page = u32_at(table, pos, "relocation block")?;
            let size = u32_at(table, pos + 4, "relocation block")? as usize;
            if size < 8 || pos + size > table.len() {
                return Err(PeError::Invalid("relocation block size"));
            }
            for e in table[pos + 8..pos + size].chunks_exact(2) {
                let v = u16::from_le_bytes([e[0], e[1]]);
                let kind = (v >> 12) as u8;
                if kind != 0 {
                    relocs.push(Relocation {
                        rva: page.wrapping_add((v & 0xfff) as u32),
                        kind,
                    });
                }
            }
            pos += size;
        }
        Ok(relocs)
    }

    /// `.pdata` entries (x86-64 `RUNTIME_FUNCTION`), in table order.
    pub fn runtime_functions(&self) -> Result<Vec<RuntimeFunction>> {
        let d = self.directory(dir::EXCEPTION);
        if d.rva == 0 || d.size == 0 {
            return Ok(Vec::new());
        }
        let table = self.read_rva(d.rva, (d.size / 12 * 12) as usize)?;
        Ok(table
            .chunks_exact(12)
            .map(|c| RuntimeFunction {
                begin_rva: u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                end_rva: u32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                unwind_rva: u32::from_le_bytes([c[8], c[9], c[10], c[11]]),
            })
            .collect())
    }

    /// TLS directory with its callbacks, or `None` if the image has none.
    pub fn tls(&self) -> Result<Option<Tls>> {
        let d = self.directory(dir::TLS);
        if d.rva == 0 {
            return Ok(None);
        }
        let at = |o: u32| d.rva.wrapping_add(o);
        let start_va = self.rva_u64(at(0))?;
        let end_va = self.rva_u64(at(8))?;
        let index_va = self.rva_u64(at(16))?;
        let callbacks_va = self.rva_u64(at(24))?;
        let zero_fill = self.rva_u32(at(32))?;
        let characteristics = self.rva_u32(at(36))?;
        let mut callbacks = Vec::new();
        if callbacks_va != 0 {
            let array = self
                .va_to_rva(callbacks_va)
                .ok_or(PeError::Invalid("TLS callback array address"))?;
            for i in 0u32.. {
                let va = self.rva_u64(array.wrapping_add(i.wrapping_mul(8)))?;
                if va == 0 {
                    break;
                }
                callbacks.push(
                    self.va_to_rva(va)
                        .ok_or(PeError::Invalid("TLS callback address"))?,
                );
            }
        }
        Ok(Some(Tls {
            start_va,
            end_va,
            index_va,
            zero_fill,
            characteristics,
            callbacks,
        }))
    }

    /// String-table entry at `offset` (which counts the 4-byte size field).
    fn string_table_entry(&self, offset: u32) -> Result<String> {
        let table = (self.symtab_offset as usize)
            .checked_add((self.symtab_count as usize).saturating_mul(18))
            .ok_or(PeError::Invalid("symbol table"))?;
        let size = u32_at(self.data, table, "string table")? as usize;
        let strings = bytes(self.data, table, size, "string table")?;
        let tail = strings
            .get(offset as usize..)
            .ok_or(PeError::Invalid("string table offset"))?;
        let len = tail
            .iter()
            .position(|&b| b == 0)
            .ok_or(PeError::Truncated("string table entry"))?;
        Ok(String::from_utf8_lossy(&tail[..len]).into_owned())
    }

    /// The COFF symbol table (empty if the image is stripped).
    pub fn symbols(&self) -> Result<Vec<Symbol>> {
        if self.symtab_offset == 0 || self.symtab_count == 0 {
            return Ok(Vec::new());
        }
        let base = self.symtab_offset as usize;
        let count = self.symtab_count as usize;
        let table = bytes(self.data, base, count.saturating_mul(18), "symbol table")?;
        let mut symbols = Vec::new();
        let mut i = 0;
        while i < count {
            let r = &table[i * 18..i * 18 + 18];
            let name = if r[..4] == [0; 4] {
                let off = u32::from_le_bytes([r[4], r[5], r[6], r[7]]);
                self.string_table_entry(off)?
            } else {
                let end = r[..8].iter().position(|&b| b == 0).unwrap_or(8);
                String::from_utf8_lossy(&r[..end]).into_owned()
            };
            symbols.push(Symbol {
                name,
                value: u32::from_le_bytes([r[8], r[9], r[10], r[11]]),
                section: i16::from_le_bytes([r[12], r[13]]),
                typ: u16::from_le_bytes([r[14], r[15]]),
                storage_class: r[16],
            });
            i += 1 + r[17] as usize;
        }
        Ok(symbols)
    }

    /// RVA of a defined symbol (section number >= 1), if its section exists.
    pub fn symbol_rva(&self, symbol: &Symbol) -> Option<u32> {
        let index = usize::try_from(symbol.section).ok()?.checked_sub(1)?;
        self.sections.get(index)?.rva.checked_add(symbol.value)
    }

    /// Candidate function starts for ahead-of-time translation, sorted by RVA
    /// and deduplicated (the first reason in [`EntryKind`] order is kept). All
    /// lie in an executable section. Damaged directories are skipped, so this
    /// never fails; use the individual accessors to see errors.
    pub fn function_entries(&self) -> Vec<FunctionEntry> {
        let mut all: Vec<FunctionEntry> = Vec::new();
        let mut add = |rva: u32, kind: EntryKind| all.push(FunctionEntry { rva, kind });
        add(self.entry_rva, EntryKind::EntryPoint);
        if let Ok(Some(ex)) = self.exports() {
            for e in ex.exports.iter().filter(|e| e.forwarder.is_none()) {
                add(e.rva, EntryKind::Export);
            }
        }
        if let Ok(Some(tls)) = self.tls() {
            for &cb in &tls.callbacks {
                add(cb, EntryKind::TlsCallback);
            }
        }
        if let Ok(funcs) = self.runtime_functions() {
            for f in funcs {
                add(f.begin_rva, EntryKind::Pdata);
            }
        }
        if let Ok(relocs) = self.relocations() {
            for r in relocs.iter().filter(|r| r.kind == RELOC_DIR64) {
                if let Ok(b) = self.read_rva(r.rva, 8)
                    && let Some(target) = self.va_to_rva(u64::from_le_bytes(b.try_into().unwrap()))
                {
                    add(target, EntryKind::Relocation);
                }
            }
        }
        if let Ok(symbols) = self.symbols() {
            for s in symbols.iter().filter(|s| s.is_function() && s.section > 0) {
                if let Some(rva) = self.symbol_rva(s) {
                    add(rva, EntryKind::Symbol);
                }
            }
        }
        all.retain(|e| {
            self.section_for_rva(e.rva)
                .is_some_and(|s| s.is_executable())
        });
        all.sort_by_key(|e| (e.rva, e.kind));
        all.dedup_by_key(|e| e.rva);
        all
    }

    /// File bytes of a function range `[begin_rva, end_rva)`.
    pub fn code_bytes(&self, begin_rva: u32, end_rva: u32) -> Result<&'a [u8]> {
        let len = end_rva
            .checked_sub(begin_rva)
            .ok_or(PeError::Invalid("function range"))?;
        self.read_rva(begin_rva, len as usize)
    }
}
