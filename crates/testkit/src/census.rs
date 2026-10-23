//! Instruction and import census of PE files (`testkit census`): counts per
//! iced-x86 `Code`, mnemonic and CPUID feature, the import table, and the
//! check that every in-scope form has L1 fixtures. The scope rule is in
//! `docs/design/instruction-scope.md`.
//!
//! What is decoded: exactly the function ranges of `.pdata`, plus a linear
//! sweep of the rest of each executable section (the gaps). Gap instructions
//! are tallied apart from function instructions; `int3`/`nop` runs and zero
//! bytes between functions count as padding and are not tallied as code.
//! A "form" is a `Code` plus whether the instruction has a memory operand.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use iced_x86::{Code, CpuidFeature, Decoder, DecoderOptions, Instruction, Mnemonic, OpKind};
use serde_json::{Value, json};
use x64wasm::pe::Pe;

use crate::oracle::{CODE_ADDR, fixture};

/// Arguments of `testkit census`.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// PE files to analyze.
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
    /// Write the full census as JSON.
    #[arg(long)]
    pub json: Option<PathBuf>,
    /// Write a Markdown summary.
    #[arg(long)]
    pub md: Option<PathBuf>,
    /// Fail if an in-scope form has no L1 fixtures.
    #[arg(long)]
    pub check: bool,
}

/// Instruction classes, in increasing rank; an instruction takes the highest
/// class among its CPUID features.
pub const CLASSES: [&str; 9] = [
    "other", "integer", "x87", "MMX", "SSE", "SSE2", "SSE3+", "AVX+", "crypto",
];

fn class_rank(f: CpuidFeature) -> usize {
    use CpuidFeature::*;
    match f {
        AES | PCLMULQDQ | SHA | VAES | VPCLMULQDQ | GFNI | SHA512 | SM3 | SM4 | KL | AESKLE
        | WIDE_KL => 8,
        AVX | AVX2 | AVX512_4FMAPS | AVX512_4VNNIW | AVX512_BF16 | AVX512_BITALG | AVX512_IFMA
        | AVX512_VBMI | AVX512_VBMI2 | AVX512_VNNI | AVX512_VP2INTERSECT | AVX512_VPOPCNTDQ
        | AVX512BW | AVX512CD | AVX512DQ | AVX512ER | AVX512F | AVX512PF | AVX512VL
        | AVX512_FP16 | AVX_VNNI | AVX_IFMA | AVX_NE_CONVERT | AVX_VNNI_INT8 | AVX_VNNI_INT16
        | FMA | FMA4 | F16C | BMI1 | BMI2 | LZCNT | TBM | XOP | ADX | MOVBE => 7,
        SSE3 | SSSE3 | SSE4_1 | SSE4_2 | SSE4A | POPCNT => 6,
        SSE2 => 5,
        SSE => 4,
        MMX => 3,
        FPU | FPU287 | FPU287XL_ONLY | FPU387 | FPU387SL_ONLY | CYRIX_FPU => 2,
        INTEL8086 | INTEL8086_ONLY | INTEL186 | INTEL286 | INTEL286_ONLY | INTEL386
        | INTEL386_ONLY | INTEL386_A0_ONLY | INTEL486 | INTEL486_A_ONLY | X64 | CMOV | CX8
        | CMPXCHG16B | PAUSE | MULTIBYTENOP => 1,
        _ => 0,
    }
}

/// Class of an instruction with these CPUID features.
pub fn class_of(features: &[CpuidFeature]) -> &'static str {
    let rank = features.iter().map(|&f| class_rank(f)).max().unwrap_or(0);
    CLASSES[rank]
}

/// Whether the Phase 1 CPUID profile advertises this feature. `PAUSE` and
/// `MULTIBYTENOP` are no CPUID bits: `pause` and `0F 1F` nops run on every
/// x86-64 CPU.
pub fn in_profile(f: CpuidFeature) -> bool {
    use CpuidFeature::*;
    matches!(
        f,
        INTEL8086
            | INTEL186
            | INTEL286
            | INTEL386
            | INTEL486
            | X64
            | CMOV
            | CX8
            | CMPXCHG16B
            | FPU
            | FPU287
            | FPU387
            | MMX
            | SSE
            | SSE2
            | FXSR
            | SYSCALL
            | PAUSE
            | MULTIBYTENOP
    )
}

/// Why a form is outside Phase 1 even when its features are in the profile
/// (the exclusions of the scope rule), or `None`.
pub fn exclusion(insn: &Instruction) -> Option<&'static str> {
    use Mnemonic::*;
    match insn.mnemonic() {
        Hlt | In | Out | Insb | Insw | Insd | Outsb | Outsw | Outsd | Cli | Sti | Clts | Invd
        | Wbinvd | Invlpg | Lgdt | Lidt | Sgdt | Sidt | Lldt | Sldt | Ltr | Str | Lmsw | Smsw
        | Rdmsr | Wrmsr | Rdpmc | Sysret | Sysexit | Iret | Iretd | Iretq | Swapgs | Lar | Lsl
        | Verr | Verw | Clac | Stac | Xsetbv => Some("privileged or system"),
        Syscall | Sysenter => Some("syscall"),
        Cpuid | Rdtsc | Rdtscp | Rdrand | Rdseed => Some("nondeterministic"),
        Mov => {
            let special = (0..insn.op_count()).any(|i| {
                matches!(insn.op_kind(i), OpKind::Register) && {
                    let r = insn.op_register(i);
                    r.is_cr() || r.is_dr() || r.is_tr()
                }
            });
            special.then_some("privileged or system")
        }
        _ => None,
    }
}

/// True if `insn` has a memory operand (string instructions included).
pub fn has_memory(insn: &Instruction) -> bool {
    insn.op_kinds().any(|k| {
        matches!(
            k,
            OpKind::Memory
                | OpKind::MemorySegSI
                | OpKind::MemorySegESI
                | OpKind::MemorySegRSI
                | OpKind::MemorySegDI
                | OpKind::MemorySegEDI
                | OpKind::MemorySegRDI
                | OpKind::MemoryESDI
                | OpKind::MemoryESEDI
                | OpKind::MemoryESRDI
        )
    })
}

/// The form of an instruction: its `Code` and register or memory variant.
pub fn form_name(insn: &Instruction) -> String {
    format!(
        "{:?}/{}",
        insn.code(),
        if has_memory(insn) { "mem" } else { "reg" }
    )
}

/// Counts of one `Code`, with an instruction that stands for it.
#[derive(Clone, Debug)]
struct Entry {
    rep: Instruction,
    reg: u64,
    mem: u64,
}

impl Entry {
    fn total(&self) -> u64 {
        self.reg + self.mem
    }
}

/// Decoded instructions, counted per `Code`.
#[derive(Clone, Debug, Default)]
struct Tally {
    instructions: u64,
    /// Byte positions that did not decode (each skipped one byte).
    invalid: u64,
    codes: BTreeMap<Code, Entry>,
}

impl Tally {
    fn add(&mut self, insn: &Instruction) {
        self.instructions += 1;
        let e = self.codes.entry(insn.code()).or_insert_with(|| Entry {
            rep: *insn,
            reg: 0,
            mem: 0,
        });
        if has_memory(insn) {
            e.mem += 1;
        } else {
            e.reg += 1;
        }
    }

    fn merge(&mut self, other: &Tally) {
        self.instructions += other.instructions;
        self.invalid += other.invalid;
        for (code, o) in &other.codes {
            let e = self.codes.entry(*code).or_insert_with(|| Entry {
                rep: o.rep,
                reg: 0,
                mem: 0,
            });
            e.reg += o.reg;
            e.mem += o.mem;
        }
    }

    /// Instructions per class.
    fn classes(&self) -> BTreeMap<&'static str, u64> {
        let mut m = BTreeMap::new();
        for e in self.codes.values() {
            *m.entry(class_of(e.rep.cpuid_features())).or_insert(0) += e.total();
        }
        m
    }

    /// Instructions per mnemonic.
    fn mnemonics(&self) -> BTreeMap<String, u64> {
        let mut m = BTreeMap::new();
        for e in self.codes.values() {
            *m.entry(mnemonic_name(&e.rep)).or_insert(0) += e.total();
        }
        m
    }

    /// Instructions per CPUID feature (an instruction counts for each of its
    /// features) and the distinct `Code`s using it.
    fn features(&self) -> BTreeMap<String, (u64, u64)> {
        let mut m: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for e in self.codes.values() {
            for f in e.rep.cpuid_features() {
                let v = m.entry(format!("{f:?}")).or_default();
                v.0 += e.total();
                v.1 += 1;
            }
        }
        m
    }
}

fn mnemonic_name(insn: &Instruction) -> String {
    format!("{:?}", insn.mnemonic()).to_lowercase()
}

fn is_padding(insn: &Instruction, bytes: &[u8]) -> bool {
    matches!(insn.mnemonic(), Mnemonic::Int3 | Mnemonic::Nop) || bytes.iter().all(|&b| b == 0)
}

/// Linear-sweep `code` (at address `ip`) into `tally`; with `padding`, runs of
/// `int3`/`nop`/zero bytes go to its counters instead. A byte that doesn't
/// decode is counted and skipped.
fn sweep(code: &[u8], ip: u64, tally: &mut Tally, mut padding: Option<&mut Padding>) {
    let mut decoder = Decoder::with_ip(64, code, ip, DecoderOptions::NONE);
    let mut insn = Instruction::default();
    while decoder.can_decode() {
        let start = decoder.position();
        decoder.decode_out(&mut insn);
        if insn.is_invalid() {
            tally.invalid += 1;
            // Skip one byte and resynchronize.
            let _ = decoder.set_position(start + 1);
            decoder.set_ip(ip + (start + 1) as u64);
            continue;
        }
        let bytes = &code[start..start + insn.len()];
        match padding.as_deref_mut() {
            Some(p) if is_padding(&insn, bytes) => {
                p.instructions += 1;
                p.bytes += insn.len() as u64;
            }
            _ => tally.add(&insn),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Padding {
    instructions: u64,
    bytes: u64,
}

#[derive(Clone, Debug)]
struct SectionInfo {
    name: String,
    rva: u32,
    size: u32,
    executable: bool,
}

#[derive(Clone, Debug)]
struct Binary {
    name: String,
    size: u64,
    image_base: u64,
    entry_rva: u32,
    subsystem: u16,
    sections: Vec<SectionInfo>,
    functions: usize,
    function_bytes: u64,
    gap_bytes: u64,
    symbols: usize,
    /// `.pdata` entries that don't lie in file-backed executable bytes.
    bad_ranges: usize,
    in_functions: Tally,
    in_gaps: Tally,
    padding: Padding,
    /// DLL name (lowercased) to imported names (`#N` for ordinals).
    imports: BTreeMap<String, Vec<String>>,
    /// Functions containing instructions outside the CPUID profile: the
    /// symbol name (or the RVA), and how many such instructions.
    out_of_profile: Vec<(String, u64)>,
}

impl Binary {
    fn all(&self) -> Tally {
        let mut t = self.in_functions.clone();
        t.merge(&self.in_gaps);
        t
    }
}

fn analyze(name: &str, data: &[u8]) -> Result<Binary> {
    let pe = Pe::parse(data).with_context(|| format!("{name}: not a PE32+ x86-64 image"))?;
    let sections: Vec<SectionInfo> = pe
        .sections
        .iter()
        .map(|s| SectionInfo {
            name: s.name.clone(),
            rva: s.rva,
            size: s.mapped_size(),
            executable: s.is_executable(),
        })
        .collect();
    let mut funcs = pe
        .runtime_functions()
        .with_context(|| format!("{name}: bad .pdata"))?;
    // Empty entries (begin == end, as the linker leaves for dropped
    // functions) aren't functions.
    funcs.retain(|f| f.begin_rva < f.end_rva);
    funcs.sort_by_key(|f| (f.begin_rva, f.end_rva));
    funcs.dedup_by_key(|f| f.begin_rva);

    let mut b = Binary {
        name: name.to_string(),
        size: data.len() as u64,
        image_base: pe.image_base,
        entry_rva: pe.entry_rva,
        subsystem: pe.subsystem,
        sections,
        functions: funcs.len(),
        function_bytes: 0,
        gap_bytes: 0,
        symbols: pe.symbols().map(|s| s.len()).unwrap_or(0),
        bad_ranges: 0,
        in_functions: Tally::default(),
        in_gaps: Tally::default(),
        padding: Padding::default(),
        imports: BTreeMap::new(),
        out_of_profile: Vec::new(),
    };
    let names: BTreeMap<u32, String> = pe
        .symbols()
        .unwrap_or_default()
        .iter()
        .filter(|s| s.is_function() && s.section > 0)
        .filter_map(|s| pe.symbol_rva(s).map(|rva| (rva, s.name.clone())))
        .collect();

    // Function ranges, clipped so that overlapping entries aren't decoded twice.
    let mut covered: Vec<(u32, u32)> = Vec::new();
    let mut end_so_far = 0u32;
    for f in &funcs {
        let begin = f.begin_rva.max(end_so_far);
        if begin >= f.end_rva {
            continue;
        }
        match pe.code_bytes(begin, f.end_rva) {
            Ok(code) if pe.section_for_rva(begin).is_some_and(|s| s.is_executable()) => {
                let mut one = Tally::default();
                sweep(code, pe.rva_to_va(begin), &mut one, None);
                let outside: u64 = one
                    .codes
                    .values()
                    .filter(|e| !e.rep.cpuid_features().iter().all(|&f| in_profile(f)))
                    .map(Entry::total)
                    .sum();
                if outside > 0 {
                    let name = names
                        .get(&begin)
                        .cloned()
                        .unwrap_or_else(|| format!("{begin:#x}"));
                    b.out_of_profile.push((name, outside));
                }
                b.in_functions.merge(&one);
                b.function_bytes += code.len() as u64;
                covered.push((begin, f.end_rva));
                end_so_far = f.end_rva;
            }
            _ => b.bad_ranges += 1,
        }
    }

    // Gaps: what is left of each executable section's file bytes.
    for s in pe.executable_sections() {
        let end = s.rva + s.file_size();
        let mut at = s.rva;
        let mut gaps: Vec<(u32, u32)> = Vec::new();
        for &(cb, ce) in covered.iter().filter(|&&(cb, ce)| ce > s.rva && cb < end) {
            if cb > at {
                gaps.push((at, cb));
            }
            at = at.max(ce);
        }
        if at < end {
            gaps.push((at, end));
        }
        for (gb, ge) in gaps {
            let code = pe.code_bytes(gb, ge)?;
            sweep(code, pe.rva_to_va(gb), &mut b.in_gaps, Some(&mut b.padding));
            b.gap_bytes += code.len() as u64;
        }
    }

    for dll in pe
        .imports()
        .with_context(|| format!("{name}: bad import table"))?
    {
        let names = b.imports.entry(dll.name.to_lowercase()).or_default();
        for f in dll.functions {
            names.push(match f.name {
                Some(n) => n,
                None => format!("#{}", f.ordinal_or_hint),
            });
        }
    }
    Ok(b)
}

/// The in-scope forms: those of `tally` whose features are all in the
/// profile and that no exclusion applies to. Maps form name to its `Code`.
fn scope_forms(tally: &Tally) -> BTreeSet<String> {
    let mut forms = BTreeSet::new();
    for e in tally.codes.values() {
        if !e.rep.cpuid_features().iter().all(|&f| in_profile(f)) || exclusion(&e.rep).is_some() {
            continue;
        }
        let code = e.rep.code();
        if e.reg > 0 {
            forms.insert(format!("{code:?}/reg"));
        }
        if e.mem > 0 {
            forms.insert(format!("{code:?}/mem"));
        }
    }
    forms
}

/// Forms present in the L1 fixtures under `dir`.
fn fixture_forms(dir: &Path) -> Result<BTreeSet<String>> {
    let mut forms = BTreeSet::new();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    for path in paths {
        let text = std::fs::read_to_string(&path)?;
        let (_, fx) =
            fixture::parse(&text).with_context(|| format!("parsing {}", path.display()))?;
        for case in &fx.cases {
            let mut decoder = Decoder::with_ip(64, &case.bytes, CODE_ADDR, DecoderOptions::NONE);
            let insn = decoder.decode();
            if insn.is_invalid() {
                bail!("{}: case `{}` does not decode", path.display(), case.asm);
            }
            forms.insert(form_name(&insn));
        }
    }
    Ok(forms)
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/l1")
}

/// Coverage of the in-scope forms by the L1 fixtures.
struct Coverage {
    in_scope: usize,
    covered: usize,
    missing: Vec<String>,
}

fn coverage(scope: &BTreeSet<String>, fixtures: &BTreeSet<String>) -> Coverage {
    let missing: Vec<String> = scope.difference(fixtures).cloned().collect();
    Coverage {
        in_scope: scope.len(),
        covered: scope.len() - missing.len(),
        missing,
    }
}

fn pct(n: u64, d: u64) -> String {
    if d == 0 {
        "-".to_string()
    } else {
        format!("{:.1}%", n as f64 * 100.0 / d as f64)
    }
}

fn code_json(e: &Entry, gap: Option<&Entry>) -> Value {
    let features: Vec<String> = e
        .rep
        .cpuid_features()
        .iter()
        .map(|f| format!("{f:?}"))
        .collect();
    let mut v = json!({
        "mnemonic": mnemonic_name(&e.rep),
        "class": class_of(e.rep.cpuid_features()),
        "features": features,
        "in_profile": e.rep.cpuid_features().iter().all(|&f| in_profile(f)),
        "reg": e.reg,
        "mem": e.mem,
    });
    if let Some(reason) = exclusion(&e.rep) {
        v["excluded"] = json!(reason);
    }
    if let Some(g) = gap {
        v["gap"] = json!(g.total());
    }
    v
}

fn tally_json(all: &Tally, gaps: &Tally) -> Value {
    let codes: BTreeMap<String, Value> = all
        .codes
        .iter()
        .map(|(c, e)| (format!("{c:?}"), code_json(e, gaps.codes.get(c))))
        .collect();
    json!({
        "instructions": all.instructions,
        "classes": all.classes(),
        "mnemonics": all.mnemonics(),
        "features": all.features().into_iter()
            .map(|(k, (n, c))| (k, json!({"instructions": n, "codes": c})))
            .collect::<BTreeMap<_, _>>(),
        "codes": codes,
    })
}

fn to_json(
    bins: &[Binary],
    combined: &Tally,
    scope: &BTreeSet<String>,
    cov: &Option<Coverage>,
) -> Value {
    let binaries: Vec<Value> = bins
        .iter()
        .map(|b| {
            let all = b.all();
            let mut v = tally_json(&all, &b.in_gaps);
            v["name"] = json!(b.name);
            v["size"] = json!(b.size);
            v["image_base"] = json!(format!("{:#x}", b.image_base));
            v["entry_rva"] = json!(format!("{:#x}", b.entry_rva));
            v["subsystem"] = json!(b.subsystem);
            v["sections"] = b
                .sections
                .iter()
                .map(|s| json!({"name": s.name, "rva": format!("{:#x}", s.rva), "size": s.size, "executable": s.executable}))
                .collect();
            v["functions"] = json!(b.functions);
            v["function_bytes"] = json!(b.function_bytes);
            v["gap_bytes"] = json!(b.gap_bytes);
            v["bad_ranges"] = json!(b.bad_ranges);
            v["symbols"] = json!(b.symbols);
            v["function_instructions"] = json!(b.in_functions.instructions);
            v["gap_instructions"] = json!(b.in_gaps.instructions);
            v["padding_instructions"] = json!(b.padding.instructions);
            v["padding_bytes"] = json!(b.padding.bytes);
            v["invalid_bytes_functions"] = json!(b.in_functions.invalid);
            v["invalid_bytes_gaps"] = json!(b.in_gaps.invalid);
            v["imports"] = json!(b.imports);
            v["out_of_profile_functions"] = b
                .out_of_profile
                .iter()
                .map(|(n, c)| json!({"name": n, "instructions": c}))
                .collect();
            v
        })
        .collect();
    let mut imports: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for b in bins {
        for (dll, names) in &b.imports {
            imports
                .entry(dll)
                .or_default()
                .extend(names.iter().map(String::as_str));
        }
    }
    let mut combined_json = tally_json(combined, &Tally::default());
    combined_json["imports"] = json!(imports);
    let mut v = json!({
        "format": 1,
        "binaries": binaries,
        "combined": combined_json,
        "scope_forms": scope,
    });
    if let Some(c) = cov {
        v["fixtures"] = json!({
            "in_scope": c.in_scope,
            "covered": c.covered,
            "missing": c.missing,
        });
    }
    v
}

fn to_markdown(
    bins: &[Binary],
    combined: &Tally,
    scope: &BTreeSet<String>,
    gap_only: &BTreeSet<String>,
    cov: &Option<Coverage>,
) -> String {
    let mut s = String::new();
    let all: Vec<Tally> = bins.iter().map(Binary::all).collect();
    let names: Vec<&str> = bins.iter().map(|b| b.name.as_str()).collect();
    let row = |cells: Vec<String>| format!("| {} |\n", cells.join(" | "));
    let sep = |n: usize| format!("|{}|\n", vec!["---"; n].join("|"));

    let _ = writeln!(s, "## Instruction census\n");
    let _ = writeln!(
        s,
        "Function ranges from `.pdata`, plus a linear sweep of the gaps in executable sections (`int3`, `nop` and zero padding counted apart).\n"
    );

    let mut head = vec!["".to_string()];
    head.extend(names.iter().map(|n| n.to_string()));
    head.push("combined".to_string());
    let n_cols = head.len();
    s += &row(head.clone());
    s += &sep(n_cols);
    let line = |label: &str, f: &dyn Fn(&Binary, &Tally) -> String, total: String| {
        let mut cells = vec![label.to_string()];
        cells.extend(bins.iter().zip(&all).map(|(b, t)| f(b, t)));
        cells.push(total);
        row(cells)
    };
    let sum = |f: &dyn Fn(&Binary) -> u64| bins.iter().map(f).sum::<u64>();
    s += &line(
        "file size (bytes)",
        &|b, _| b.size.to_string(),
        sum(&|b| b.size).to_string(),
    );
    s += &line(
        "functions (`.pdata`)",
        &|b, _| b.functions.to_string(),
        sum(&|b| b.functions as u64).to_string(),
    );
    s += &line(
        "bytes in functions",
        &|b, _| b.function_bytes.to_string(),
        sum(&|b| b.function_bytes).to_string(),
    );
    s += &line(
        "instructions in functions",
        &|b, _| b.in_functions.instructions.to_string(),
        sum(&|b| b.in_functions.instructions).to_string(),
    );
    s += &line(
        "gap bytes",
        &|b, _| b.gap_bytes.to_string(),
        sum(&|b| b.gap_bytes).to_string(),
    );
    s += &line(
        "instructions in gaps",
        &|b, _| b.in_gaps.instructions.to_string(),
        sum(&|b| b.in_gaps.instructions).to_string(),
    );
    s += &line(
        "padding instructions",
        &|b, _| b.padding.instructions.to_string(),
        sum(&|b| b.padding.instructions).to_string(),
    );
    s += &line(
        "undecodable bytes in functions / gaps",
        &|b, _| format!("{} / {}", b.in_functions.invalid, b.in_gaps.invalid),
        format!(
            "{} / {}",
            sum(&|b| b.in_functions.invalid),
            sum(&|b| b.in_gaps.invalid)
        ),
    );
    s += &line(
        "distinct `Code`s",
        &|_, t| t.codes.len().to_string(),
        combined.codes.len().to_string(),
    );
    s += &line(
        "COFF symbols",
        &|b, _| b.symbols.to_string(),
        sum(&|b| b.symbols as u64).to_string(),
    );

    // Classes.
    let _ = writeln!(s, "\n### Instructions per class\n");
    let mut head = vec!["class".to_string()];
    head.extend(names.iter().map(|n| n.to_string()));
    head.push("combined".to_string());
    head.push("share".to_string());
    head.push("distinct `Code`s".to_string());
    s += &row(head.clone());
    s += &sep(head.len());
    let comb_classes = combined.classes();
    let per_bin: Vec<_> = all.iter().map(Tally::classes).collect();
    for class in CLASSES.iter().rev() {
        let total = comb_classes.get(class).copied().unwrap_or(0);
        if total == 0 {
            continue;
        }
        let distinct = combined
            .codes
            .values()
            .filter(|e| class_of(e.rep.cpuid_features()) == *class)
            .count();
        let mut cells = vec![(*class).to_string()];
        cells.extend(
            per_bin
                .iter()
                .map(|m| m.get(class).copied().unwrap_or(0).to_string()),
        );
        cells.push(total.to_string());
        cells.push(pct(total, combined.instructions));
        cells.push(distinct.to_string());
        s += &row(cells);
    }

    // Out-of-profile features.
    let _ = writeln!(s, "\n### Features outside the Phase 1 CPUID profile\n");
    let mut out_of_profile: BTreeMap<String, (u64, BTreeSet<String>)> = BTreeMap::new();
    for e in combined.codes.values() {
        for &f in e.rep.cpuid_features() {
            if !in_profile(f) {
                let v = out_of_profile.entry(format!("{f:?}")).or_default();
                v.0 += e.total();
                v.1.insert(mnemonic_name(&e.rep));
            }
        }
    }
    if out_of_profile.is_empty() {
        let _ = writeln!(s, "None.");
    } else {
        s += &row(vec![
            "feature".into(),
            "instructions".into(),
            "mnemonics".into(),
        ]);
        s += &sep(3);
        let mut rows: Vec<_> = out_of_profile.iter().collect();
        rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
        for (f, (n, mn)) in rows {
            let list: Vec<&str> = mn.iter().map(String::as_str).collect();
            s += &row(vec![
                f.clone(),
                n.to_string(),
                format!("`{}`", list.join("`, `")),
            ]);
        }
    }

    for b in bins.iter().filter(|b| !b.out_of_profile.is_empty()) {
        let _ = writeln!(
            s,
            "\n`{}`: {} of {} functions contain such instructions (all behind CPUID dispatch if the program is correct): {}",
            b.name,
            b.out_of_profile.len(),
            b.functions,
            b.out_of_profile
                .iter()
                .map(|(n, c)| format!("`{n}` ({c})"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // Mnemonics.
    let _ = writeln!(s, "\n### Top mnemonics (combined)\n");
    let mut mn: Vec<(String, u64)> = combined.mnemonics().into_iter().collect();
    mn.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    s += &row(vec![
        "mnemonic".into(),
        "instructions".into(),
        "share".into(),
    ]);
    s += &sep(3);
    for (m, n) in mn.iter().take(30) {
        s += &row(vec![
            format!("`{m}`"),
            n.to_string(),
            pct(*n, combined.instructions),
        ]);
    }
    let _ = writeln!(s, "\n{} distinct mnemonics in all.", mn.len());

    // Scope and fixtures.
    let _ = writeln!(s, "\n### Scope and fixtures\n");
    let excluded: Vec<_> = combined
        .codes
        .iter()
        .filter_map(|(c, e)| exclusion(&e.rep).map(|r| (c, e.total(), r)))
        .collect();
    let _ = writeln!(
        s,
        "{} in-scope forms (a `Code` as register or memory variant); {} `Code`s out of profile; {} excluded: {}.\n",
        scope.len(),
        combined
            .codes
            .values()
            .filter(|e| !e.rep.cpuid_features().iter().all(|&f| in_profile(f)))
            .count(),
        excluded.len(),
        if excluded.is_empty() {
            "none".to_string()
        } else {
            excluded
                .iter()
                .map(|(c, n, r)| format!("`{c:?}` ({n}, {r})"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    if !gap_only.is_empty() {
        let _ = writeln!(
            s,
            "{} of them occur only in gaps (import thunks, `___chkstk_ms`, or bytes that are data): {}.\n",
            gap_only.len(),
            gap_only
                .iter()
                .map(|f| format!("`{f}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    match cov {
        Some(c) => {
            let _ = writeln!(
                s,
                "**Fixtures cover {} of {} in-scope forms** ({}).",
                c.covered,
                c.in_scope,
                pct(c.covered as u64, c.in_scope as u64)
            );
            if !c.missing.is_empty() {
                let _ = writeln!(
                    s,
                    "\nMissing ({}): {}",
                    c.missing.len(),
                    c.missing
                        .iter()
                        .map(|m| format!("`{m}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        None => {
            let _ = writeln!(
                s,
                "Fixtures cover ? of {} in-scope forms (fixtures could not be read).",
                scope.len()
            );
        }
    }

    // Imports.
    let _ = writeln!(s, "\n### Imports\n");
    let mut by_dll: BTreeMap<&str, BTreeMap<&str, Vec<&str>>> = BTreeMap::new();
    for b in bins {
        for (dll, fns) in &b.imports {
            let d = by_dll.entry(dll).or_default();
            for f in fns {
                d.entry(f).or_default().push(&b.name);
            }
        }
    }
    let mut head = vec!["DLL".to_string(), "distinct functions".to_string()];
    head.extend(names.iter().map(|n| n.to_string()));
    s += &row(head.clone());
    s += &sep(head.len());
    for (dll, fns) in &by_dll {
        let mut cells = vec![format!("`{dll}`"), fns.len().to_string()];
        for b in bins {
            cells.push(b.imports.get(*dll).map_or(0, |v| v.len()).to_string());
        }
        s += &row(cells);
    }
    let total: usize = by_dll.values().map(BTreeMap::len).sum();
    let _ = writeln!(
        s,
        "\n{} DLLs, {total} distinct imported functions.\n",
        by_dll.len()
    );
    for (dll, fns) in &by_dll {
        let list: Vec<&str> = fns.keys().copied().collect();
        let _ = writeln!(s, "- `{dll}` ({}): {}", list.len(), list.join(", "));
    }
    s
}

pub fn run(args: &Args) -> Result<()> {
    let mut bins = Vec::new();
    for path in &args.files {
        let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        bins.push(analyze(&name, &data)?);
    }
    let mut combined = Tally::default();
    for b in &bins {
        combined.merge(&b.all());
    }
    let scope = scope_forms(&combined);
    let cov = fixture_forms(&fixtures_dir())
        .ok()
        .map(|f| coverage(&scope, &f));

    let mut in_functions = Tally::default();
    for b in &bins {
        in_functions.merge(&b.in_functions);
    }
    let gap_only: BTreeSet<String> = scope
        .difference(&scope_forms(&in_functions))
        .cloned()
        .collect();
    let md = to_markdown(&bins, &combined, &scope, &gap_only, &cov);
    if let Some(path) = &args.json {
        let text = serde_json::to_string_pretty(&to_json(&bins, &combined, &scope, &cov))?;
        std::fs::write(path, text + "\n").with_context(|| format!("writing {}", path.display()))?;
    }
    if let Some(path) = &args.md {
        std::fs::write(path, &md).with_context(|| format!("writing {}", path.display()))?;
    }
    for b in &bins {
        println!(
            "{}: {} functions, {} instructions in functions, {} in gaps, {} padding, {} distinct Codes",
            b.name,
            b.functions,
            b.in_functions.instructions,
            b.in_gaps.instructions,
            b.padding.instructions,
            b.all().codes.len()
        );
    }
    println!(
        "combined: {} instructions, {} distinct Codes, {} in-scope forms",
        combined.instructions,
        combined.codes.len(),
        scope.len()
    );
    match &cov {
        Some(c) => println!(
            "fixtures cover {} of {} in-scope forms",
            c.covered, c.in_scope
        ),
        None => println!("fixtures could not be read"),
    }
    if args.check {
        let c = cov.context("--check: the L1 fixtures could not be read")?;
        if !c.missing.is_empty() {
            eprintln!("{} in-scope forms have no L1 fixtures:", c.missing.len());
            for m in &c.missing {
                eprintln!("  {m}");
            }
            bail!(
                "fixtures cover {} of {} in-scope forms",
                c.covered,
                c.in_scope
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u64 = 0x1_4000_0000;

    fn put(buf: &mut [u8], at: usize, bytes: &[u8]) {
        buf[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// A PE32+ image with `.text` (rva 0x1000, 0x20 bytes) and `.rdata`
    /// (rva 0x2000) holding one `.pdata` entry and one imported DLL.
    ///
    /// `.text`: a function `[0x1000, 0x1014)` of six instructions, four `int3`,
    /// a four-byte unlisted function (`xor rax,rax; ret`) and four zero bytes.
    fn image() -> Vec<u8> {
        let mut f = vec![0u8; 0x800];
        put(&mut f, 0, b"MZ");
        put(&mut f, 0x3c, &0x40u32.to_le_bytes());
        put(&mut f, 0x40, b"PE\0\0");
        let coff = 0x44;
        put(&mut f, coff, &0x8664u16.to_le_bytes());
        put(&mut f, coff + 2, &2u16.to_le_bytes());
        put(&mut f, coff + 16, &240u16.to_le_bytes());
        let opt = coff + 20;
        put(&mut f, opt, &0x20bu16.to_le_bytes());
        put(&mut f, opt + 16, &0x1000u32.to_le_bytes());
        put(&mut f, opt + 24, &BASE.to_le_bytes());
        put(&mut f, opt + 56, &0x3000u32.to_le_bytes());
        put(&mut f, opt + 60, &0x200u32.to_le_bytes());
        put(&mut f, opt + 108, &16u32.to_le_bytes());
        // Import directory (1) and exception directory (3).
        put(&mut f, opt + 112 + 8, &0x2100u32.to_le_bytes());
        put(&mut f, opt + 112 + 12, &0x28u32.to_le_bytes());
        put(&mut f, opt + 112 + 24, &0x2000u32.to_le_bytes());
        put(&mut f, opt + 112 + 28, &12u32.to_le_bytes());
        let sect =
            |f: &mut Vec<u8>, i: usize, name: &[u8], size: u32, rva: u32, raw: u32, ch: u32| {
                let h = opt + 240 + i * 40;
                put(f, h, name);
                put(f, h + 8, &size.to_le_bytes());
                put(f, h + 12, &rva.to_le_bytes());
                put(f, h + 16, &0x200u32.to_le_bytes());
                put(f, h + 20, &raw.to_le_bytes());
                put(f, h + 36, &ch.to_le_bytes());
            };
        sect(&mut f, 0, b".text", 0x20, 0x1000, 0x200, 0x6000_0020);
        sect(&mut f, 1, b".rdata", 0x200, 0x2000, 0x400, 0x4000_0040);
        let code: &[u8] = &[
            0x48, 0x01, 0xd8, // add rax, rbx
            0x48, 0x8b, 0x03, // mov rax, [rbx]
            0x66, 0x0f, 0x6f, 0x03, // movdqa xmm0, [rbx]
            0xf3, 0x48, 0x0f, 0xb8, 0xc3, // popcnt rax, rbx
            0xc5, 0xf9, 0x6f, 0x03, // vmovdqa xmm0, [rbx]
            0xc3, // ret
            0xcc, 0xcc, 0xcc, 0xcc, // padding
            0x48, 0x31, 0xc0, 0xc3, // xor rax, rax; ret (no .pdata entry)
            0, 0, 0, 0, // padding
        ];
        put(&mut f, 0x200, code);
        // .pdata at rva 0x2000: [0x1000, 0x1014).
        put(&mut f, 0x400, &0x1000u32.to_le_bytes());
        put(&mut f, 0x404, &0x1014u32.to_le_bytes());
        put(&mut f, 0x408, &0x2080u32.to_le_bytes());
        // Imports at rva 0x2100: KERNEL32.dll!ExitProcess.
        put(&mut f, 0x500, &0x2140u32.to_le_bytes()); // ILT
        put(&mut f, 0x50c, &0x2180u32.to_le_bytes()); // name
        put(&mut f, 0x510, &0x2150u32.to_le_bytes()); // IAT
        put(&mut f, 0x540, &0x21a0u64.to_le_bytes());
        put(&mut f, 0x550, &0x21a0u64.to_le_bytes());
        put(&mut f, 0x580, b"KERNEL32.dll\0");
        put(&mut f, 0x5a2, b"ExitProcess\0");
        f
    }

    #[test]
    fn counts_functions_gaps_and_padding() {
        let data = image();
        let b = analyze("synthetic.exe", &data).unwrap();
        assert_eq!(b.functions, 1);
        assert_eq!(b.function_bytes, 0x14);
        assert_eq!(b.gap_bytes, 0x0c);
        assert_eq!(b.in_functions.instructions, 6);
        assert_eq!(b.in_functions.invalid, 0);
        // Gap: four int3 and two `add [rax], al` of zeros are padding.
        assert_eq!(b.padding.instructions, 4 + 2);
        assert_eq!(b.padding.bytes, 8);
        assert_eq!(b.in_gaps.instructions, 2, "xor and ret");
        assert_eq!(b.all().instructions, 8);
        assert_eq!(b.imports["kernel32.dll"], ["ExitProcess"]);
    }

    #[test]
    fn splits_register_and_memory_variants() {
        let data = image();
        let b = analyze("synthetic.exe", &data).unwrap();
        let mov = &b.in_functions.codes[&Code::Mov_r64_rm64];
        assert_eq!((mov.reg, mov.mem), (0, 1));
        let add = &b.in_functions.codes[&Code::Add_rm64_r64];
        assert_eq!((add.reg, add.mem), (1, 0));
        assert_eq!(
            form_name(&b.in_functions.codes[&Code::Mov_r64_rm64].rep),
            "Mov_r64_rm64/mem"
        );
    }

    #[test]
    fn classifies_and_checks_the_profile() {
        let data = image();
        let b = analyze("synthetic.exe", &data).unwrap();
        let classes = b.all().classes();
        assert_eq!(classes["integer"], 5, "add, mov, ret, xor, ret");
        assert_eq!(classes["SSE2"], 1);
        assert_eq!(classes["SSE3+"], 1, "popcnt");
        assert_eq!(classes["AVX+"], 1, "vmovdqa");
        let scope = scope_forms(&b.all());
        assert!(scope.contains("Add_rm64_r64/reg"));
        assert!(scope.contains("Mov_r64_rm64/mem"));
        assert!(scope.contains("Movdqa_xmm_xmmm128/mem"));
        assert!(!scope.iter().any(|f| f.starts_with("Popcnt")));
        assert!(!scope.iter().any(|f| f.starts_with("VEX_Vmovdqa")));
        assert_eq!(scope.len(), 6 - 1, "xor and ret appear once, ret in both");
    }

    #[test]
    fn exclusions() {
        let decode = |bytes: &[u8]| Decoder::new(64, bytes, DecoderOptions::NONE).decode();
        assert_eq!(exclusion(&decode(&[0xf4])), Some("privileged or system")); // hlt
        assert_eq!(exclusion(&decode(&[0x0f, 0x05])), Some("syscall"));
        assert_eq!(exclusion(&decode(&[0x0f, 0xa2])), Some("nondeterministic")); // cpuid
        assert_eq!(exclusion(&decode(&[0x0f, 0x31])), Some("nondeterministic")); // rdtsc
        assert_eq!(
            exclusion(&decode(&[0x0f, 0x20, 0xc0])), // mov rax, cr0
            Some("privileged or system")
        );
        assert_eq!(exclusion(&decode(&[0x48, 0x89, 0xd8])), None); // mov rax, rbx
    }

    #[test]
    fn class_ranking() {
        use CpuidFeature::*;
        assert_eq!(class_of(&[INTEL8086]), "integer");
        assert_eq!(class_of(&[FPU]), "x87");
        assert_eq!(class_of(&[MMX]), "MMX");
        assert_eq!(class_of(&[SSE2]), "SSE2");
        assert_eq!(class_of(&[SSSE3]), "SSE3+");
        assert_eq!(class_of(&[AVX, AES]), "crypto");
        assert_eq!(class_of(&[AVX2]), "AVX+");
        assert_eq!(class_of(&[BMI1]), "AVX+");
        assert_eq!(class_of(&[MSR]), "other");
        assert!(in_profile(X64) && in_profile(SSE2) && in_profile(FPU));
        assert!(!in_profile(SSE3) && !in_profile(POPCNT) && !in_profile(AVX));
    }

    #[test]
    fn rejects_non_pe_input() {
        assert!(analyze("x", b"not a pe file").is_err());
    }

    #[test]
    fn fixture_forms_are_read() {
        let forms = fixture_forms(&fixtures_dir()).unwrap();
        assert!(forms.contains("Add_rm64_r64/reg"), "{forms:?}");
        assert!(forms.contains("Add_rm64_r64/mem"));
        let data = image();
        let b = analyze("synthetic.exe", &data).unwrap();
        let scope = scope_forms(&b.all());
        let c = coverage(&scope, &forms);
        assert_eq!(c.in_scope, scope.len());
        assert_eq!(c.covered + c.missing.len(), c.in_scope);
        assert!(c.covered >= 1, "add is in the fixtures");
    }
}
