//! Tests of the PE reader on synthetic images built here, one per directory
//! the reader parses, plus malformed inputs. No external files are needed.

use x64wasm::image::{self, EntrySet, LoadPlan};
use x64wasm::pe::{
    EntryKind, Pe, PeError, RELOC_DIR64, RELOC_HIGHLOW, SCN_CNT_CODE, SCN_MEM_EXECUTE,
};

pub const IMAGE_BASE: u64 = 0x1_4000_0000;

const TEXT_RVA: u32 = 0x1000;
const RDATA_RVA: u32 = 0x2000;
const RELOC_RVA: u32 = 0x3000;
const PDATA_RVA: u32 = 0x4000;
const DEBUG_RVA: u32 = 0x5000;

const IMPORT_RVA: u32 = RDATA_RVA;
const EXPORT_RVA: u32 = RDATA_RVA + 0x400;
const EXPORT_SIZE: u32 = 0x100;
const RELOC_TARGET_RVA: u32 = RDATA_RVA + 0x500;
const UNWIND_RVA: u32 = RDATA_RVA + 0x600;
const TLS_RVA: u32 = RDATA_RVA + 0x700;

fn put(buf: &mut [u8], at: usize, bytes: &[u8]) {
    buf[at..at + bytes.len()].copy_from_slice(bytes);
}

fn put16(buf: &mut [u8], at: usize, v: u16) {
    put(buf, at, &v.to_le_bytes());
}

fn put32(buf: &mut [u8], at: usize, v: u32) {
    put(buf, at, &v.to_le_bytes());
}

fn put64(buf: &mut [u8], at: usize, v: u64) {
    put(buf, at, &v.to_le_bytes());
}

fn cstr(buf: &mut [u8], at: usize, s: &str) {
    put(buf, at, s.as_bytes());
    buf[at + s.len()] = 0;
}

struct Sect {
    name: [u8; 8],
    rva: u32,
    virtual_size: u32,
    raw_offset: u32,
    raw_size: u32,
    characteristics: u32,
}

/// Raw offsets: headers 0x200, .text 0x200, .rdata 0x400, .reloc 0x1400,
/// .pdata 0x1600, the long-named section 0x1800, COFF symbols 0x1a00.
fn sections() -> Vec<Sect> {
    let name = |s: &str| {
        let mut n = [0u8; 8];
        n[..s.len()].copy_from_slice(s.as_bytes());
        n
    };
    vec![
        Sect {
            name: name(".text"),
            rva: TEXT_RVA,
            virtual_size: 0x180,
            raw_offset: 0x200,
            raw_size: 0x200,
            characteristics: SCN_CNT_CODE | SCN_MEM_EXECUTE | 0x4000_0000,
        },
        Sect {
            name: name(".rdata"),
            rva: RDATA_RVA,
            virtual_size: 0x1000,
            raw_offset: 0x400,
            raw_size: 0x1000,
            characteristics: 0x4000_0040,
        },
        Sect {
            name: name(".reloc"),
            rva: RELOC_RVA,
            virtual_size: 0x40,
            raw_offset: 0x1400,
            raw_size: 0x200,
            characteristics: 0x4200_0040,
        },
        Sect {
            name: name(".pdata"),
            rva: PDATA_RVA,
            virtual_size: 0x30,
            raw_offset: 0x1600,
            raw_size: 0x200,
            characteristics: 0x4000_0040,
        },
        // "/4": the name is at offset 4 of the string table.
        Sect {
            name: name("/4"),
            rva: DEBUG_RVA,
            virtual_size: 0x10,
            raw_offset: 0x1800,
            raw_size: 0x200,
            characteristics: 0x4200_0040,
        },
    ]
}

/// A valid image exercising every directory. `dirs` overrides data
/// directories by index.
fn build_image() -> Vec<u8> {
    let mut f = vec![0u8; 0x1a00];
    put(&mut f, 0, b"MZ");
    put32(&mut f, 0x3c, 0x40);
    put(&mut f, 0x40, b"PE\0\0");
    let coff = 0x44;
    let sects = sections();
    put16(&mut f, coff, 0x8664);
    put16(&mut f, coff + 2, sects.len() as u16);
    put32(&mut f, coff + 4, 0x6000_0000);
    put32(&mut f, coff + 8, 0x1a00); // symbol table
    put32(&mut f, coff + 12, 5); // 5 records (one is auxiliary)
    put16(&mut f, coff + 16, 240);
    put16(&mut f, coff + 18, 0x0022);
    let opt = coff + 20;
    put16(&mut f, opt, 0x20b);
    put32(&mut f, opt + 16, TEXT_RVA); // entry point
    put64(&mut f, opt + 24, IMAGE_BASE);
    put32(&mut f, opt + 32, 0x1000);
    put32(&mut f, opt + 36, 0x200);
    put32(&mut f, opt + 56, 0x6000);
    put32(&mut f, opt + 60, 0x200);
    put16(&mut f, opt + 68, 3); // console
    put16(&mut f, opt + 70, 0x8160);
    put64(&mut f, opt + 72, 0x20_0000);
    put64(&mut f, opt + 80, 0x1000);
    put64(&mut f, opt + 88, 0x10_0000);
    put64(&mut f, opt + 96, 0x1000);
    put32(&mut f, opt + 108, 16);
    let dir = |f: &mut Vec<u8>, i: usize, rva: u32, size: u32| {
        put32(f, opt + 112 + i * 8, rva);
        put32(f, opt + 116 + i * 8, size);
    };
    dir(&mut f, 0, EXPORT_RVA, EXPORT_SIZE);
    dir(&mut f, 1, IMPORT_RVA, 0x3c);
    dir(&mut f, 3, PDATA_RVA, 24);
    dir(&mut f, 5, RELOC_RVA, 0x10);
    dir(&mut f, 9, TLS_RVA, 40);
    for (i, s) in sects.iter().enumerate() {
        let h = opt + 240 + i * 40;
        put(&mut f, h, &s.name);
        put32(&mut f, h + 8, s.virtual_size);
        put32(&mut f, h + 12, s.rva);
        put32(&mut f, h + 16, s.raw_size);
        put32(&mut f, h + 20, s.raw_offset);
        put32(&mut f, h + 36, s.characteristics);
    }
    f.resize(0x1a00 + 5 * 18 + 64, 0);

    // .text
    f[0x200..0x380].fill(0x90);
    // .rdata, addressed by RVA
    let r = |rva: u32| (rva - RDATA_RVA) as usize + 0x400;
    // Imports: KERNEL32 (ILT and IAT: ExitProcess hint 1, ordinal 5) and
    // msvcrt (no ILT: the IAT is read; puts hint 7).
    put32(&mut f, r(IMPORT_RVA), RDATA_RVA + 0x40); // OriginalFirstThunk
    put32(&mut f, r(IMPORT_RVA) + 12, RDATA_RVA + 0x80); // Name
    put32(&mut f, r(IMPORT_RVA) + 16, RDATA_RVA + 0x60); // FirstThunk
    put32(&mut f, r(IMPORT_RVA) + 20 + 12, RDATA_RVA + 0xe0);
    put32(&mut f, r(IMPORT_RVA) + 20 + 16, RDATA_RVA + 0xc0);
    put64(&mut f, r(RDATA_RVA + 0x40), (RDATA_RVA + 0xa0) as u64);
    put64(&mut f, r(RDATA_RVA + 0x48), (1 << 63) | 5);
    put64(&mut f, r(RDATA_RVA + 0x60), (RDATA_RVA + 0xa0) as u64);
    put64(&mut f, r(RDATA_RVA + 0x68), (1 << 63) | 5);
    cstr(&mut f, r(RDATA_RVA + 0x80), "KERNEL32.dll");
    put16(&mut f, r(RDATA_RVA + 0xa0), 1);
    cstr(&mut f, r(RDATA_RVA + 0xa2), "ExitProcess");
    put64(&mut f, r(RDATA_RVA + 0xc0), (RDATA_RVA + 0xf0) as u64);
    cstr(&mut f, r(RDATA_RVA + 0xe0), "msvcrt.dll");
    put16(&mut f, r(RDATA_RVA + 0xf0), 7);
    cstr(&mut f, r(RDATA_RVA + 0xf2), "puts");
    // Exports: ordinal base 5, three functions (alpha, forwarder, gamma);
    // names alpha and gamma. Function 1 is a forwarder string in the directory.
    let e = r(EXPORT_RVA);
    put32(&mut f, e + 12, EXPORT_RVA + 0x60); // dll name
    put32(&mut f, e + 16, 5);
    put32(&mut f, e + 20, 3);
    put32(&mut f, e + 24, 2);
    put32(&mut f, e + 28, EXPORT_RVA + 0x30);
    put32(&mut f, e + 32, EXPORT_RVA + 0x40);
    put32(&mut f, e + 36, EXPORT_RVA + 0x50);
    put32(&mut f, e + 0x30, TEXT_RVA + 0x10);
    put32(&mut f, e + 0x34, EXPORT_RVA + 0x90);
    put32(&mut f, e + 0x38, TEXT_RVA + 0x20);
    put32(&mut f, e + 0x40, EXPORT_RVA + 0x70);
    put32(&mut f, e + 0x44, EXPORT_RVA + 0x78);
    put16(&mut f, e + 0x50, 0);
    put16(&mut f, e + 0x52, 2);
    cstr(&mut f, e + 0x60, "test.dll");
    cstr(&mut f, e + 0x70, "alpha");
    cstr(&mut f, e + 0x78, "gamma");
    cstr(&mut f, e + 0x90, "NTDLL.RtlFoo");
    // The DIR64 relocation at RELOC_TARGET_RVA points into .text.
    put64(
        &mut f,
        r(RELOC_TARGET_RVA),
        IMAGE_BASE + (TEXT_RVA + 0x30) as u64,
    );
    put32(&mut f, r(UNWIND_RVA), 0x0001_0401);
    // TLS: callbacks at TLS_RVA + 0x40 (VA of the array in the directory).
    let t = r(TLS_RVA);
    put64(&mut f, t, IMAGE_BASE + 0x2800);
    put64(&mut f, t + 8, IMAGE_BASE + 0x2810);
    put64(&mut f, t + 16, IMAGE_BASE + 0x2820);
    put64(&mut f, t + 24, IMAGE_BASE + (TLS_RVA + 0x40) as u64);
    put32(&mut f, t + 32, 0x20);
    put32(&mut f, t + 36, 0x0010_0000);
    put64(&mut f, t + 0x40, IMAGE_BASE + (TEXT_RVA + 0x40) as u64);
    put64(&mut f, t + 0x48, IMAGE_BASE + (TEXT_RVA + 0x50) as u64);
    // .reloc: one block for page RDATA_RVA: DIR64 at +0x500, padding, HIGHLOW at +0x510.
    put32(&mut f, 0x1400, RDATA_RVA);
    put32(&mut f, 0x1404, 16);
    put16(&mut f, 0x1408, (RELOC_DIR64 as u16) << 12 | 0x500);
    put16(&mut f, 0x140a, 0);
    put16(&mut f, 0x140c, (RELOC_HIGHLOW as u16) << 12 | 0x510);
    put16(&mut f, 0x140e, 0);
    // .pdata
    for (i, (b, e)) in [(0x1000u32, 0x1010u32), (0x1010, 0x1020)]
        .iter()
        .enumerate()
    {
        put32(&mut f, 0x1600 + i * 12, *b);
        put32(&mut f, 0x1604 + i * 12, *e);
        put32(&mut f, 0x1608 + i * 12, UNWIND_RVA);
    }
    // COFF symbols: ".text" with one auxiliary record, "main" (function,
    // section 1, value 0x60), a long-named function (string table offset 16,
    // value 0x70) and an undefined data symbol. The string table follows.
    let s = 0x1a00;
    cstr(&mut f, s, ".text");
    put16(&mut f, s + 12, 1);
    f[s + 16] = 3;
    f[s + 17] = 1;
    // auxiliary record at s + 18 stays zero
    let m = s + 36;
    cstr(&mut f, m, "main");
    put32(&mut f, m + 8, 0x60);
    put16(&mut f, m + 12, 1);
    put16(&mut f, m + 14, 0x20);
    f[m + 16] = 2;
    let l = s + 54;
    put32(&mut f, l + 4, 16);
    put32(&mut f, l + 8, 0x70);
    put16(&mut f, l + 12, 1);
    put16(&mut f, l + 14, 0x20);
    f[l + 16] = 2;
    let u = s + 72;
    cstr(&mut f, u, "_data");
    f[u + 16] = 2;
    let st = s + 90;
    put32(&mut f, st, 64);
    cstr(&mut f, st + 4, ".debug_info");
    cstr(&mut f, st + 16, "a_rather_long_function_name");
    f
}

fn parse(image: &[u8]) -> Pe<'_> {
    Pe::parse(image).expect("synthetic image parses")
}

#[test]
fn headers() {
    let image = build_image();
    let pe = parse(&image);
    assert_eq!(pe.entry_rva, TEXT_RVA);
    assert_eq!(pe.image_base, IMAGE_BASE);
    assert_eq!(pe.section_alignment, 0x1000);
    assert_eq!(pe.file_alignment, 0x200);
    assert_eq!(pe.size_of_image, 0x6000);
    assert_eq!(pe.subsystem, 3);
    assert_eq!(pe.dll_characteristics, 0x8160);
    assert_eq!(pe.stack_reserve, 0x20_0000);
    assert_eq!(pe.stack_commit, 0x1000);
    assert_eq!(pe.heap_reserve, 0x10_0000);
    assert_eq!(pe.heap_commit, 0x1000);
    assert_eq!(pe.characteristics, 0x0022);
    assert_eq!(pe.data_directories.len(), 16);
    assert_eq!(pe.directory(1).rva, IMPORT_RVA);
    assert_eq!(pe.directory(99).rva, 0);
}

#[test]
fn sections_and_addresses() {
    let image = build_image();
    let pe = parse(&image);
    let names: Vec<_> = pe.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        [".text", ".rdata", ".reloc", ".pdata", ".debug_info"],
        "the long name comes from the string table"
    );
    let exec: Vec<_> = pe.executable_sections().map(|s| s.name.as_str()).collect();
    assert_eq!(exec, [".text"]);
    assert_eq!(pe.section_by_name(".pdata").unwrap().rva, PDATA_RVA);
    assert_eq!(pe.section_for_rva(0x2400).unwrap().name, ".rdata");
    assert!(
        pe.section_for_rva(0x1180).is_none(),
        "past the virtual size"
    );
    assert_eq!(pe.rva_to_va(0x1000), IMAGE_BASE + 0x1000);
    assert_eq!(pe.va_to_rva(IMAGE_BASE + 0x1234), Some(0x1234));
    assert_eq!(pe.va_to_rva(IMAGE_BASE - 1), None);
    assert_eq!(pe.va_to_rva(IMAGE_BASE + (1 << 32)), None);
    assert_eq!(pe.rva_to_offset(0x1000), Some(0x200));
    assert_eq!(pe.rva_to_offset(0x2010), Some(0x410));
    assert_eq!(
        pe.rva_to_offset(0x10),
        Some(0x10),
        "headers map to themselves"
    );
    assert_eq!(pe.rva_to_offset(0x9000), None);
    // .debug_info: virtual size 0x10 is smaller than the raw size.
    assert_eq!(pe.rva_to_offset(DEBUG_RVA + 0x10), None);
    assert_eq!(pe.read_rva(0x1000, 4).unwrap(), &[0x90; 4]);
    assert!(
        pe.read_rva(0x1170, 0x20).is_err(),
        "crosses the section end"
    );
    assert_eq!(pe.code_bytes(0x1000, 0x1008).unwrap().len(), 8);
    assert!(pe.code_bytes(0x1008, 0x1000).is_err());
}

#[test]
fn imports() {
    let image = build_image();
    let pe = parse(&image);
    let dlls = pe.imports().unwrap();
    assert_eq!(dlls.len(), 2);
    assert_eq!(dlls[0].name, "KERNEL32.dll");
    let f = &dlls[0].functions;
    assert_eq!(f.len(), 2);
    assert_eq!(f[0].name.as_deref(), Some("ExitProcess"));
    assert_eq!(f[0].ordinal_or_hint, 1);
    assert_eq!(f[0].iat_rva, RDATA_RVA + 0x60);
    assert_eq!(f[1].name, None);
    assert_eq!(f[1].ordinal_or_hint, 5);
    assert_eq!(f[1].iat_rva, RDATA_RVA + 0x68);
    assert_eq!(dlls[1].name, "msvcrt.dll");
    assert_eq!(dlls[1].functions.len(), 1);
    assert_eq!(dlls[1].functions[0].name.as_deref(), Some("puts"));
    assert_eq!(dlls[1].functions[0].ordinal_or_hint, 7);
    assert_eq!(dlls[1].functions[0].iat_rva, RDATA_RVA + 0xc0);
}

#[test]
fn exports() {
    let image = build_image();
    let pe = parse(&image);
    let ex = pe.exports().unwrap().expect("has exports");
    assert_eq!(ex.dll_name, "test.dll");
    assert_eq!(ex.ordinal_base, 5);
    let e = &ex.exports;
    assert_eq!(e.len(), 3);
    assert_eq!(
        (e[0].ordinal, e[0].name.as_deref(), e[0].rva),
        (5, Some("alpha"), 0x1010)
    );
    assert_eq!(e[0].forwarder, None);
    assert_eq!(e[1].ordinal, 6);
    assert_eq!(e[1].name, None);
    assert_eq!(e[1].forwarder.as_deref(), Some("NTDLL.RtlFoo"));
    assert_eq!(
        (e[2].ordinal, e[2].name.as_deref(), e[2].rva),
        (7, Some("gamma"), 0x1020)
    );
}

#[test]
fn relocations() {
    let image = build_image();
    let pe = parse(&image);
    let r = pe.relocations().unwrap();
    assert_eq!(r.len(), 2, "the padding entry is dropped");
    assert_eq!((r[0].rva, r[0].kind), (RELOC_TARGET_RVA, RELOC_DIR64));
    assert_eq!((r[1].rva, r[1].kind), (RDATA_RVA + 0x510, RELOC_HIGHLOW));
}

#[test]
fn runtime_functions() {
    let image = build_image();
    let pe = parse(&image);
    let f = pe.runtime_functions().unwrap();
    assert_eq!(f.len(), 2);
    assert_eq!(
        (f[0].begin_rva, f[0].end_rva, f[0].unwind_rva),
        (0x1000, 0x1010, UNWIND_RVA)
    );
    assert_eq!((f[1].begin_rva, f[1].end_rva), (0x1010, 0x1020));
}

#[test]
fn tls() {
    let image = build_image();
    let pe = parse(&image);
    let t = pe.tls().unwrap().expect("has TLS");
    assert_eq!(t.start_va, IMAGE_BASE + 0x2800);
    assert_eq!(t.end_va, IMAGE_BASE + 0x2810);
    assert_eq!(t.index_va, IMAGE_BASE + 0x2820);
    assert_eq!(t.zero_fill, 0x20);
    assert_eq!(t.characteristics, 0x0010_0000);
    assert_eq!(t.callbacks, [TEXT_RVA + 0x40, TEXT_RVA + 0x50]);
}

#[test]
fn symbols() {
    let image = build_image();
    let pe = parse(&image);
    let s = pe.symbols().unwrap();
    let names: Vec<_> = s.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        [".text", "main", "a_rather_long_function_name", "_data"]
    );
    assert!(!s[0].is_function());
    assert!(s[1].is_function());
    assert_eq!(pe.symbol_rva(&s[1]), Some(TEXT_RVA + 0x60));
    assert_eq!(pe.symbol_rva(&s[2]), Some(TEXT_RVA + 0x70));
    assert_eq!(pe.symbol_rva(&s[3]), None, "undefined");
}

#[test]
fn function_entries() {
    let image = build_image();
    let pe = parse(&image);
    let got: Vec<_> = pe
        .function_entries()
        .iter()
        .map(|e| (e.rva, e.kind))
        .collect();
    let want = [
        (0x1000, EntryKind::EntryPoint),
        (0x1010, EntryKind::Export),
        (0x1020, EntryKind::Export),
        (0x1030, EntryKind::Relocation),
        (0x1040, EntryKind::TlsCallback),
        (0x1050, EntryKind::TlsCallback),
        (0x1060, EntryKind::Symbol),
        (0x1070, EntryKind::Symbol),
    ];
    assert_eq!(got, want);
}

#[test]
fn image_without_directories() {
    let mut image = build_image();
    // Zero every data directory and the symbol table pointer.
    let opt = 0x44 + 20;
    image[opt + 112..opt + 112 + 128].fill(0);
    image[0x44 + 8..0x44 + 16].fill(0);
    let pe = parse(&image);
    assert!(pe.imports().unwrap().is_empty());
    assert!(pe.exports().unwrap().is_none());
    assert!(pe.relocations().unwrap().is_empty());
    assert!(pe.runtime_functions().unwrap().is_empty());
    assert!(pe.tls().unwrap().is_none());
    assert!(pe.symbols().unwrap().is_empty());
    let entries = pe.function_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, EntryKind::EntryPoint);
}

#[test]
fn rejects_bad_headers() {
    let image = build_image();
    assert!(matches!(Pe::parse(&[]), Err(PeError::Truncated(_))));
    assert!(matches!(Pe::parse(b"MZ"), Err(PeError::Truncated(_))));

    let mut bad = image.clone();
    bad[0] = b'X';
    assert!(matches!(Pe::parse(&bad), Err(PeError::Invalid(_))));

    let mut bad = image.clone();
    bad[0x40] = b'X';
    assert!(matches!(Pe::parse(&bad), Err(PeError::Invalid(_))));

    let mut bad = image.clone();
    bad[0x3c..0x40].copy_from_slice(&0xffff_fff0u32.to_le_bytes());
    assert!(matches!(Pe::parse(&bad), Err(PeError::Truncated(_))));

    let mut bad = image.clone();
    bad[0x44..0x46].copy_from_slice(&0x14cu16.to_le_bytes());
    assert!(matches!(Pe::parse(&bad), Err(PeError::Unsupported(_))));

    let mut bad = image.clone();
    bad[0x44 + 20..0x44 + 22].copy_from_slice(&0x10bu16.to_le_bytes());
    assert!(matches!(Pe::parse(&bad), Err(PeError::Unsupported(_))));

    let mut bad = image.clone();
    bad[0x44 + 20..0x44 + 22].copy_from_slice(&0x1234u16.to_le_bytes());
    assert!(matches!(Pe::parse(&bad), Err(PeError::Invalid(_))));

    // More data directories than the header has room for.
    let mut bad = image.clone();
    bad[0x44 + 20 + 108..0x44 + 20 + 112].copy_from_slice(&17u32.to_le_bytes());
    assert!(matches!(Pe::parse(&bad), Err(PeError::Invalid(_))));

    // Section table past the end of the file.
    let mut bad = image.clone();
    bad[0x44 + 2..0x44 + 4].copy_from_slice(&60000u16.to_le_bytes());
    assert!(matches!(Pe::parse(&bad), Err(PeError::Truncated(_))));
}

#[test]
fn rejects_damaged_directories() {
    let opt = 0x44 + 20;
    let set_dir = |image: &mut Vec<u8>, i: usize, rva: u32, size: u32| {
        put32(image, opt + 112 + i * 8, rva);
        put32(image, opt + 116 + i * 8, size);
    };

    // Imports: the table runs off the end of the image.
    let mut bad = build_image();
    set_dir(&mut bad, 1, 0x2ff8, 0x40);
    let pe = parse(&bad);
    assert!(pe.imports().is_err());

    // Imports: unmapped DLL name.
    let mut bad = build_image();
    put32(&mut bad, 0x400 + 12, 0x9000);
    assert!(parse(&bad).imports().is_err());

    // Exports: a huge function count.
    let mut bad = build_image();
    put32(&mut bad, 0x400 + 0x400 + 20, 0x4000_0000);
    assert!(parse(&bad).exports().is_err());

    // Exports: a name ordinal beyond the function table.
    let mut bad = build_image();
    put16(&mut bad, 0x400 + 0x400 + 0x52, 9);
    assert!(parse(&bad).exports().is_err());

    // Relocations: a block smaller than its header.
    let mut bad = build_image();
    put32(&mut bad, 0x1404, 4);
    assert!(parse(&bad).relocations().is_err());

    // Relocations: a block longer than the directory.
    let mut bad = build_image();
    put32(&mut bad, 0x1404, 0x100);
    assert!(parse(&bad).relocations().is_err());

    // Exception directory outside any section.
    let mut bad = build_image();
    set_dir(&mut bad, 3, 0x9000, 24);
    assert!(parse(&bad).runtime_functions().is_err());

    // TLS callback array without a terminator before the section ends.
    let mut bad = build_image();
    for i in 0..0x1c8 {
        put64(&mut bad, 0x400 + 0x740 + i * 8, IMAGE_BASE + 0x1000);
    }
    assert!(parse(&bad).tls().is_err());

    // TLS callback VA below the image base.
    let mut bad = build_image();
    put64(&mut bad, 0x400 + 0x740, 0x1000);
    assert!(parse(&bad).tls().is_err());

    // Symbol table past the end of the file.
    let mut bad = build_image();
    put32(&mut bad, 0x44 + 8, 0x10_0000);
    let pe = parse(&bad);
    assert!(pe.symbols().is_err());
    // The other accessors and function_entries still work.
    assert!(pe.imports().is_ok());
    assert!(!pe.function_entries().is_empty());

    // Symbol with a string table offset outside the table.
    let mut bad = build_image();
    put32(&mut bad, 0x1a00 + 54 + 4, 0x1000);
    assert!(parse(&bad).symbols().is_err());
}

/// Run every accessor; none may panic, whatever the bytes.
fn exercise(data: &[u8]) {
    if let Ok(pe) = Pe::parse(data) {
        let _ = pe.imports();
        let _ = pe.exports();
        let _ = pe.relocations();
        let _ = pe.runtime_functions();
        let _ = pe.tls();
        let _ = pe.symbols();
        let _ = pe.function_entries();
        let _ = pe.read_cstr(0x1000);
        let _ = pe.read_rva(u32::MAX, 16);
        let _ = pe.code_bytes(0, u32::MAX);
        let _ = LoadPlan::from_pe(&pe, "x.exe");
    }
}

#[test]
fn never_panics_on_truncation() {
    let image = build_image();
    for len in 0..image.len() {
        exercise(&image[..len]);
    }
}

#[test]
fn never_panics_on_corruption() {
    let image = build_image();
    // Every byte of the headers and directories set to extreme values.
    let interesting = (0..0x400)
        .chain(0x1400..0x1420)
        .chain(0x1600..0x1620)
        .chain(0x1a00..0x1a60);
    for at in interesting.chain((0x400..0x1400).step_by(4)) {
        for v in [0x00u8, 0x7f, 0x80, 0xff] {
            let mut bad = image.clone();
            bad[at] = v;
            exercise(&bad);
        }
    }
}

#[test]
fn sha256_vectors() {
    assert_eq!(
        image::sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        image::sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn load_plan_follows_the_runtime_schema() {
    let image = build_image();
    let pe = parse(&image);
    let plan = LoadPlan::from_pe(&pe, "test \"1\".exe").unwrap();
    let got: serde_json::Value = serde_json::from_str(&plan.to_json()).unwrap();
    let want = serde_json::json!({
        "version": 1,
        "file": "test \"1\".exe",
        "sha256": image::sha256_hex(&image),
        "machine": "0x8664",
        "imageBase": "0x140000000",
        "sizeOfImage": "0x6000",
        "sizeOfHeaders": "0x200",
        "entryPoint": "0x140001000",
        "subsystem": 3,
        "stackReserve": "0x200000",
        "stackCommit": "0x1000",
        "sections": [
            { "name": ".text", "va": "0x140001000", "virtualSize": "0x180",
              "fileOffset": "0x200", "fileSize": "0x200", "characteristics": "0x60000020" },
            { "name": ".rdata", "va": "0x140002000", "virtualSize": "0x1000",
              "fileOffset": "0x400", "fileSize": "0x1000", "characteristics": "0x40000040" },
            { "name": ".reloc", "va": "0x140003000", "virtualSize": "0x40",
              "fileOffset": "0x1400", "fileSize": "0x200", "characteristics": "0x42000040" },
            { "name": ".pdata", "va": "0x140004000", "virtualSize": "0x30",
              "fileOffset": "0x1600", "fileSize": "0x200", "characteristics": "0x40000040" },
            // The builder's .text fill (file offset 0x200) overlaps this
            // header's last field.
            { "name": ".debug_info", "va": "0x140005000", "virtualSize": "0x10",
              "fileOffset": "0x1800", "fileSize": "0x200", "characteristics": "0x90909090" },
        ],
        "imports": [
            { "dll": "KERNEL32.dll", "functions": [
                { "name": "ExitProcess", "ordinal": null, "iat": "0x140002060" },
                { "name": null, "ordinal": 5, "iat": "0x140002068" },
            ] },
            { "dll": "msvcrt.dll", "functions": [
                { "name": "puts", "ordinal": null, "iat": "0x1400020c0" },
            ] },
        ],
        "tls": {
            "startOfRawData": "0x140002800",
            "endOfRawData": "0x140002810",
            "addressOfIndex": "0x140002820",
            "sizeOfZeroFill": "0x20",
            "callbacks": ["0x140001040", "0x140001050"],
        },
        "hasRelocations": true,
    });
    assert_eq!(got, want);

    // No TLS, no relocations.
    let mut bare = build_image();
    for d in [5, 9] {
        put32(&mut bare, 0x44 + 20 + 112 + d * 8, 0);
        put32(&mut bare, 0x44 + 20 + 116 + d * 8, 0);
    }
    let plan = LoadPlan::from_pe(&parse(&bare), "bare.exe").unwrap();
    let got: serde_json::Value = serde_json::from_str(&plan.to_json()).unwrap();
    assert_eq!(got["tls"], serde_json::Value::Null);
    assert_eq!(got["hasRelocations"], false);
}

/// `.text` of the synthetic image with code: `call 0x1010; jmp .rdata`, and
/// `ret` at 0x1010. The other entries run NOPs off the end of `.text`.
fn image_with_code() -> Vec<u8> {
    let mut f = build_image();
    let text = |rva: u32| 0x200 + (rva - TEXT_RVA) as usize;
    put(&mut f, text(0x1000), &[0xe8, 0x0b, 0, 0, 0]);
    put(&mut f, text(0x1005), &[0xe9, 0xf6, 0x0f, 0, 0]);
    put(&mut f, text(0x1010), &[0xc3]);
    f
}

#[test]
fn translates_only_executable_sections() {
    let image = image_with_code();
    let pe = parse(&image);
    let va = |rva: u32| IMAGE_BASE + rva as u64;
    let text = va(TEXT_RVA)..va(TEXT_RVA + 0x180);

    let t = image::translate(&pe, EntrySet::EntryPoint, &x64wasm::Options::default()).unwrap();
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&t.wasm)
        .unwrap();
    assert_eq!(t.blocks, [va(0x1000), va(0x1005), va(0x1010)]);
    let fns: Vec<u64> = t.functions.iter().map(|f| f.addr).collect();
    assert_eq!(fns, [va(0x1000), va(0x1010)]);
    let blocks = x64wasm::link::read_blocks(&t.wasm).unwrap();
    let addrs: Vec<u64> = blocks.blocks.iter().map(|b| b.addr).collect();
    assert_eq!(addrs, t.blocks);

    // Every entry of function_entries; nothing outside .text is decoded,
    // and the NOPs that run off its end stop there.
    let all = image::translate(&pe, EntrySet::All, &x64wasm::Options::default()).unwrap();
    for e in pe.function_entries() {
        assert!(all.blocks.contains(&va(e.rva)), "{:#x}", e.rva);
    }
    assert!(
        all.blocks.iter().all(|b| text.contains(b)),
        "{:x?}",
        all.blocks
    );
    assert!(!all.blocks.contains(&va(RDATA_RVA)));
    assert!(all.functions.len() >= pe.function_entries().len());
}

#[test]
fn image_translation_errors() {
    // Entry point outside every executable section.
    let mut bad = image_with_code();
    put32(&mut bad, 0x44 + 20 + 16, RDATA_RVA);
    let pe = parse(&bad);
    assert!(image::translate(&pe, EntrySet::EntryPoint, &x64wasm::Options::default()).is_err());
    // .text raw data past the end of the file.
    let image = image_with_code();
    let pe = parse(&image[..0x300]);
    assert!(image::translate(&pe, EntrySet::All, &x64wasm::Options::default()).is_err());
}

#[test]
fn image_jump_tables_are_read_from_rdata() {
    use iced_x86::code_asm::*;
    use iced_x86::{BlockEncoderOptions, Code, Instruction, MemoryOperand, Register};
    let table = IMAGE_BASE + (RDATA_RVA + 0xa00) as u64;
    let mut a = CodeAssembler::new(64).unwrap();
    let mut default = a.create_label();
    let mut cases: Vec<CodeLabel> = (0..3).map(|_| a.create_label()).collect();
    a.cmp(ecx, 2).unwrap();
    a.ja(default).unwrap();
    a.add_instruction(
        Instruction::with2(
            Code::Lea_r64_m,
            Register::RDX,
            MemoryOperand::with_base_displ(Register::RIP, table as i64),
        )
        .unwrap(),
    )
    .unwrap();
    a.movsxd(rax, dword_ptr(rdx + rcx * 4)).unwrap();
    a.add(rax, rdx).unwrap();
    a.jmp(rax).unwrap();
    for (i, c) in cases.iter_mut().enumerate() {
        a.set_label(c).unwrap();
        a.mov(eax, i as u32).unwrap();
        a.ret().unwrap();
    }
    a.set_label(&mut default).unwrap();
    a.ret().unwrap();
    let text = IMAGE_BASE + TEXT_RVA as u64;
    let r = a
        .assemble_options(text, BlockEncoderOptions::RETURN_NEW_INSTRUCTION_OFFSETS)
        .unwrap();
    let targets: Vec<u64> = cases.iter().map(|c| r.label_ip(c).unwrap()).collect();
    let mut image = build_image();
    put(&mut image, 0x200, &r.inner.code_buffer);
    for (i, &t) in targets.iter().enumerate() {
        put32(
            &mut image,
            0x400 + 0xa00 + 4 * i,
            t.wrapping_sub(table) as u32,
        );
    }
    let pe = parse(&image);
    let t = image::translate(&pe, EntrySet::EntryPoint, &x64wasm::Options::default()).unwrap();
    assert_eq!(t.jump_tables.len(), 1);
    assert_eq!(t.jump_tables[0].table, table);
    assert_eq!(t.jump_tables[0].targets, targets);
    assert!(targets.iter().all(|c| t.blocks.contains(c)));
    assert_eq!(t.indirect_jumps, 1);
}
