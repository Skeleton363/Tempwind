//! Replay of L1 fixtures through the translator, in wasmtime.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Mutex;

use crate::host::Module;
use anyhow::{Context, Result};
use x64wasm::abi::{
    self, CpuState, EXIT_FAULT, EXIT_LEFT_CODE, EXIT_UNSUPPORTED, GPR_NAMES, STATE_SIZE, flags,
};
use x64wasm::{CodeRegion, Options};

use super::fixture::{self, Case, Fault, FaultKind, Fixture, FpState};
use super::{CODE_ADDR, CODE_SIZE, DATA_ADDR, DATA_SIZE, MEMORY_BYTES, STATE_ADDR};
use crate::host::Host;

/// Failures shown in full per form.
const DETAILS_PER_FORM: usize = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub cases: usize,
    pub pass: usize,
    pub fail: usize,
    pub unsupported: usize,
}

impl Stats {
    fn add(&mut self, other: &Stats) {
        self.cases += other.cases;
        self.pass += other.pass;
        self.fail += other.fail;
        self.unsupported += other.unsupported;
    }
}

enum Outcome {
    Pass,
    Fail(String),
    Unsupported(String),
}

/// Per-family and per-form results of one replay, plus readable details of
/// the first failures of each form.
#[derive(Debug, Default)]
pub struct ReplayReport {
    pub families: Vec<(String, Stats)>,
    pub forms: Vec<(String, Stats)>,
    pub details: Vec<String>,
}

impl ReplayReport {
    pub fn total(&self) -> Stats {
        let mut t = Stats::default();
        for (_, s) in &self.families {
            t.add(s);
        }
        t
    }

    /// True when every case passed. Unsupported counts as failure.
    pub fn ok(&self) -> bool {
        let t = self.total();
        t.cases > 0 && t.pass == t.cases
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for d in &self.details {
            let _ = writeln!(out, "{d}");
        }
        let failing: Vec<&(String, Stats)> = self
            .forms
            .iter()
            .filter(|(_, s)| s.pass != s.cases)
            .collect();
        if !failing.is_empty() {
            let _ = writeln!(out, "forms with failures ({}):", failing.len());
            for (form, s) in failing {
                let _ = writeln!(
                    out,
                    "  {form:<20} {:>4} cases {:>4} fail {:>4} unsupported",
                    s.cases, s.fail, s.unsupported
                );
            }
            out.push('\n');
        }
        let _ = writeln!(
            out,
            "{:<8} {:>6} {:>6} {:>6} {:>12}",
            "family", "cases", "pass", "fail", "unsupported"
        );
        let row = |out: &mut String, name: &str, s: &Stats| {
            let _ = writeln!(
                out,
                "{name:<8} {:>6} {:>6} {:>6} {:>12}",
                s.cases, s.pass, s.fail, s.unsupported
            );
        };
        for (name, s) in &self.families {
            row(&mut out, name, s);
        }
        row(&mut out, "total", &self.total());
        out
    }
}

/// Replays every `*.json` fixture in `dir`.
pub fn replay_dir(dir: &Path) -> Result<ReplayReport> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    let mut fixtures = Vec::new();
    for path in &paths {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let (_, f) =
            fixture::parse(&text).with_context(|| format!("parsing {}", path.display()))?;
        fixtures.push(f);
    }
    let host = Host::new()?;
    let next = Mutex::new(0usize);
    let results: Mutex<Vec<Option<FamilyResult>>> =
        Mutex::new((0..fixtures.len()).map(|_| None).collect());
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(fixtures.len())
        .max(1);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let i = {
                        let mut n = next.lock().unwrap();
                        let i = *n;
                        *n += 1;
                        i
                    };
                    let Some(f) = fixtures.get(i) else { break };
                    let r = replay_fixture(&host, f);
                    results.lock().unwrap()[i] = Some(r);
                }
            });
        }
    });
    let mut report = ReplayReport::default();
    for r in results.into_inner().unwrap().into_iter().flatten() {
        report.families.push((r.family, r.stats));
        report.forms.extend(r.forms);
        report.details.extend(r.details);
    }
    Ok(report)
}

struct FamilyResult {
    family: String,
    stats: Stats,
    forms: Vec<(String, Stats)>,
    details: Vec<String>,
}

fn replay_fixture(host: &Host, f: &Fixture) -> FamilyResult {
    let mut cache: HashMap<Vec<u8>, Result<Module, Outcome>> = HashMap::new();
    let mut forms: BTreeMap<String, Stats> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut details = Vec::new();
    let mut shown: HashMap<String, usize> = HashMap::new();
    let mut stats = Stats::default();
    for (n, case) in f.cases.iter().enumerate() {
        if !forms.contains_key(&case.form) {
            order.push(case.form.clone());
        }
        let s = forms.entry(case.form.clone()).or_default();
        s.cases += 1;
        let outcome = run_case(host, &mut cache, case);
        let (msg, kind) = match outcome {
            Outcome::Pass => {
                s.pass += 1;
                continue;
            }
            Outcome::Fail(m) => {
                s.fail += 1;
                (m, "FAIL")
            }
            Outcome::Unsupported(m) => {
                s.unsupported += 1;
                (m, "UNSUPPORTED")
            }
        };
        let count = shown.entry(case.form.clone()).or_default();
        *count += 1;
        if *count <= DETAILS_PER_FORM {
            details.push(format!(
                "{kind} {}[{n}] {}: {}  [{}]\n{}\n{msg}",
                f.family,
                case.form,
                case.asm,
                hex_bytes(&case.bytes),
                describe_input(f, case)
            ));
        }
    }
    let forms: Vec<(String, Stats)> = order
        .into_iter()
        .map(|name| {
            let s = forms[&name];
            stats.add(&s);
            (name, s)
        })
        .collect();
    FamilyResult {
        family: f.family.clone(),
        stats,
        forms,
        details,
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    let parts: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    parts.join(" ")
}

fn describe_input(f: &Fixture, case: &Case) -> String {
    let mut out = format!("    input: rflags={:#x}", case.input.rflags);
    for (i, name) in GPR_NAMES.iter().enumerate() {
        if case.input.gprs[i] != f.default_gprs[i] {
            let _ = write!(out, " {name}={:#x}", case.input.gprs[i]);
        }
    }
    out.push_str(" (other gprs: default_gprs)");
    for (name, v) in [
        ("fs_base", case.input.fs_base),
        ("gs_base", case.input.gs_base),
    ] {
        if v != 0 {
            let _ = write!(out, " {name}={v:#x}");
        }
    }
    for f in 0..fixture::FP_FIELDS {
        let v = case.input.fp.get(f);
        if v != FpState::DEFAULT.get(f) {
            let _ = write!(out, " {}={v:#x}", FpState::field_name(f));
        }
    }
    for b in &case.input.mem {
        let _ = write!(
            out,
            "\n    input mem {:#x}: {}",
            b.addr,
            hex_bytes(&b.bytes)
        );
    }
    out
}

fn flag_names(bits: u64) -> String {
    let names = [
        (flags::CF, "CF"),
        (flags::PF, "PF"),
        (flags::AF, "AF"),
        (flags::ZF, "ZF"),
        (flags::SF, "SF"),
        (flags::OF, "OF"),
        (flags::DF, "DF"),
    ];
    let v: Vec<&str> = names
        .iter()
        .filter(|(b, _)| bits & b != 0)
        .map(|(_, n)| *n)
        .collect();
    if v.is_empty() {
        format!("{bits:#x}")
    } else {
        v.join(" ")
    }
}

fn compile(host: &Host, bytes: &[u8]) -> Result<Module, Outcome> {
    let region = CodeRegion {
        base: CODE_ADDR,
        bytes,
    };
    let t = match x64wasm::translate(region, &[CODE_ADDR], &Options::default()) {
        Ok(t) => t,
        Err(x64wasm::Error::Unimplemented(what)) => {
            return Err(Outcome::Unsupported(format!(
                "    translate: not implemented: {what}"
            )));
        }
        Err(e) => return Err(Outcome::Fail(format!("    translate failed: {e}"))),
    };
    host.compile(&t.wasm)
        .map_err(|e| Outcome::Fail(format!("    translated module does not compile: {e:#}")))
}

fn clone_outcome(o: &Outcome) -> Outcome {
    match o {
        Outcome::Pass => Outcome::Pass,
        Outcome::Fail(m) => Outcome::Fail(m.clone()),
        Outcome::Unsupported(m) => Outcome::Unsupported(m.clone()),
    }
}

/// Describes differing byte runs between two equally long images at `base`.
fn mem_diffs(base: u64, expected: &[u8], got: &[u8], out: &mut Vec<String>) {
    let mut i = 0;
    while i < expected.len() && out.len() < 8 {
        if expected[i] == got[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < expected.len() && expected[i] != got[i] && i - start < 32 {
            i += 1;
        }
        out.push(format!(
            "    mem {:#x}: expected {} got {}",
            base + start as u64,
            hex_bytes(&expected[start..i]),
            hex_bytes(&got[start..i])
        ));
    }
}

fn run_case(
    host: &Host,
    cache: &mut HashMap<Vec<u8>, Result<Module, Outcome>>,
    case: &Case,
) -> Outcome {
    let module = cache
        .entry(case.bytes.clone())
        .or_insert_with(|| compile(host, &case.bytes));
    let module = match module {
        Ok(m) => m.clone(),
        Err(o) => return clone_outcome(o),
    };
    match execute(host, &module, case) {
        Ok(o) => o,
        Err(e) => Outcome::Fail(format!("    run failed: {e:#}")),
    }
}

/// The state block for `case`'s input (`docs/design/fixtures.md`, "Replay").
fn state_block(case: &Case) -> Vec<u8> {
    let i = &case.input;
    let mut b = CpuState {
        gpr: i.gprs,
        rip: CODE_ADDR,
        rflags: i.rflags,
        exit: 0,
    }
    .to_bytes()
    .to_vec();
    let mut put = |at: u64, bytes: &[u8]| b[at as usize..][..bytes.len()].copy_from_slice(bytes);
    put(abi::STATE_FS_BASE, &i.fs_base.to_le_bytes());
    put(abi::STATE_GS_BASE, &i.gs_base.to_le_bytes());
    for (n, x) in i.fp.xmm.iter().enumerate() {
        put(abi::STATE_XMM + 16 * n as u64, &x.to_le_bytes());
    }
    put(abi::STATE_MXCSR, &i.fp.mxcsr.to_le_bytes());
    put(abi::STATE_FCW, &i.fp.fcw.to_le_bytes());
    put(abi::STATE_FSW, &i.fp.fsw.to_le_bytes());
    put(abi::STATE_FTW, &i.fp.ftw.to_le_bytes());
    for (p, r) in i.fp.physical().iter().enumerate() {
        put(abi::STATE_FPR + 16 * p as u64, &r.to_le_bytes()[..10]);
    }
    b
}

/// The FP state and fault fields of a state block after a run.
fn read_block(b: &[u8]) -> (FpState, u32, u64) {
    let at = |off: u64, len: usize| &b[off as usize..][..len];
    let u128_at = |off: u64, len: usize| {
        let mut buf = [0u8; 16];
        buf[..len].copy_from_slice(at(off, len));
        u128::from_le_bytes(buf)
    };
    let mut fp = FpState {
        xmm: std::array::from_fn(|n| u128_at(abi::STATE_XMM + 16 * n as u64, 16)),
        mxcsr: u128_at(abi::STATE_MXCSR, 4) as u32,
        fcw: u128_at(abi::STATE_FCW, 2) as u16,
        fsw: u128_at(abi::STATE_FSW, 2) as u16,
        ftw: u128_at(abi::STATE_FTW, 2) as u16,
        st: [0; 8],
    };
    let r: [u128; 8] = std::array::from_fn(|p| u128_at(abi::STATE_FPR + 16 * p as u64, 10));
    fp.set_physical(&r);
    (
        fp,
        u128_at(abi::STATE_FAULT_KIND, 4) as u32,
        u128_at(abi::STATE_FAULT_ADDR, 8) as u64,
    )
}

/// The fixture fault an `EXIT_FAULT` kind stands for.
fn fault_of(kind: u32, addr: u64) -> Option<Fault> {
    let kind = match kind {
        abi::FAULT_READ | abi::FAULT_WRITE | abi::FAULT_EXECUTE => FaultKind::Pf,
        abi::FAULT_DIVIDE => FaultKind::De,
        abi::FAULT_INVALID_OPCODE => FaultKind::Ud,
        abi::FAULT_GENERAL_PROTECTION => FaultKind::Gp,
        _ => return None,
    };
    Some(Fault { kind, addr })
}

fn execute(host: &Host, module: &Module, case: &Case) -> Result<Outcome> {
    let mut m = host.machine(MEMORY_BYTES)?;
    m.write(CODE_ADDR, &case.bytes)?;
    for b in &case.input.mem {
        m.write(b.addr, &b.bytes)?;
    }
    m.write(STATE_ADDR, &state_block(case))?;
    let exit = match m.run(module, STATE_ADDR) {
        Ok(exit) => exit,
        Err(e) => return Ok(Outcome::Fail(format!("    run trapped: {e:#}"))),
    };
    let block = m.read(STATE_ADDR, STATE_SIZE as usize)?;
    let out = CpuState::from_bytes(&block);
    if out.exit != exit {
        return Ok(Outcome::Fail(format!(
            "    run returned {exit} but the state block says {}",
            out.exit
        )));
    }
    if out.exit == EXIT_UNSUPPORTED {
        return Ok(Outcome::Unsupported(format!(
            "    exit EXIT_UNSUPPORTED at rip {:#x}",
            out.rip
        )));
    }
    let (fp, fault_kind, fault_addr) = read_block(&block);
    let want = &case.output;
    let mut diffs = Vec::new();
    match want.fault {
        None if out.exit != EXIT_LEFT_CODE => diffs.push(format!(
            "    exit: expected {EXIT_LEFT_CODE} (EXIT_LEFT_CODE) got {}{}",
            out.exit,
            if out.exit == EXIT_FAULT {
                format!(" (EXIT_FAULT kind {fault_kind}, address {fault_addr:#x})")
            } else {
                String::new()
            }
        )),
        Some(f) if out.exit != EXIT_FAULT => diffs.push(format!(
            "    exit: expected {EXIT_FAULT} (EXIT_FAULT, fault {f}) got {}",
            out.exit
        )),
        Some(f) if fault_of(fault_kind, fault_addr) != Some(f) => diffs.push(format!(
            "    fault: expected {f} got kind {fault_kind}, address {fault_addr:#x}"
        )),
        _ => {}
    }
    if out.rip != want.rip {
        diffs.push(format!(
            "    rip: expected {:#x} got {:#x}",
            want.rip, out.rip
        ));
    }
    let undef = &case.undefined;
    for (i, name) in GPR_NAMES.iter().enumerate() {
        if (out.gpr[i] ^ want.gprs[i]) & !undef.gprs[i] != 0 {
            diffs.push(format!(
                "    {name}: expected {:#x} got {:#x}",
                want.gprs[i], out.gpr[i]
            ));
        }
    }
    let flag_diff = (out.rflags ^ want.rflags) & case.flags_mask;
    if flag_diff != 0 {
        diffs.push(format!(
            "    rflags & {:#x}: expected {:#x} got {:#x} (differs in {})",
            case.flags_mask,
            want.rflags & case.flags_mask,
            out.rflags & case.flags_mask,
            flag_names(flag_diff)
        ));
    }
    for f in 0..fixture::FP_FIELDS {
        if (fp.get(f) ^ want.fp.get(f)) & !undef.fp.get(f) != 0 {
            diffs.push(format!(
                "    {}: expected {:#x} got {:#x}",
                FpState::field_name(f),
                want.fp.get(f),
                fp.get(f)
            ));
        }
    }
    let mut expected = vec![0u8; DATA_SIZE as usize];
    for b in &want.mem {
        let off = (b.addr - DATA_ADDR) as usize;
        expected[off..off + b.bytes.len()].copy_from_slice(&b.bytes);
    }
    let got = m.read(DATA_ADDR, DATA_SIZE as usize)?;
    mem_diffs(DATA_ADDR, &expected, &got, &mut diffs);
    let mut code = vec![0u8; CODE_SIZE as usize];
    code[..case.bytes.len()].copy_from_slice(&case.bytes);
    let got_code = m.read(CODE_ADDR, CODE_SIZE as usize)?;
    mem_diffs(CODE_ADDR, &code, &got_code, &mut diffs);
    Ok(if diffs.is_empty() {
        Outcome::Pass
    } else {
        Outcome::Fail(diffs.join("\n"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::fixture::{Input, Output, Undefined};

    #[test]
    fn state_block_round_trips_fp_fields() {
        let mut input = Input::new([3; 16], 0x202, Vec::new());
        input.fs_base = 0x1234;
        input.gs_base = 0x5678;
        input.fp.xmm[7] = u128::MAX - 5;
        input.fp.mxcsr = 0x9fc0;
        input.fp.fcw = 0x27f;
        input.fp.fsw = 5 << 11;
        input.fp.ftw = 0x3fff;
        input.fp.st = std::array::from_fn(|i| 0x4000_8000_0000_0000_0000 + i as u128);
        let case = Case {
            form: String::new(),
            bytes: vec![0x90],
            asm: String::new(),
            flags_mask: 0,
            undefined: Undefined::NONE,
            input,
            output: Output::empty(),
        };
        let b = state_block(&case);
        assert_eq!(b.len(), STATE_SIZE as usize);
        let (fp, kind, addr) = read_block(&b);
        assert_eq!(fp, case.input.fp);
        assert_eq!((kind, addr), (0, 0));
        // ST(0) = R[TOP] lands in physical slot 5.
        let at = (abi::STATE_FPR + 16 * 5) as usize;
        assert_eq!(&b[at..at + 10], &case.input.fp.st[0].to_le_bytes()[..10]);
        assert_eq!(&b[at + 10..at + 16], &[0; 6]);
        let fs = abi::STATE_FS_BASE as usize;
        assert_eq!(&b[fs..fs + 8], &0x1234u64.to_le_bytes());
    }

    #[test]
    fn fault_kinds_map_to_fixture_faults() {
        let pf = |addr| {
            Some(Fault {
                kind: FaultKind::Pf,
                addr,
            })
        };
        assert_eq!(fault_of(abi::FAULT_READ, 0x10), pf(0x10));
        assert_eq!(fault_of(abi::FAULT_WRITE, 0x10), pf(0x10));
        assert_eq!(fault_of(abi::FAULT_EXECUTE, 0x10), pf(0x10));
        let kind = |k| fault_of(k, 0).map(|f| f.kind);
        assert_eq!(kind(abi::FAULT_DIVIDE), Some(FaultKind::De));
        assert_eq!(kind(abi::FAULT_INVALID_OPCODE), Some(FaultKind::Ud));
        assert_eq!(kind(abi::FAULT_GENERAL_PROTECTION), Some(FaultKind::Gp));
        assert_eq!(kind(0), None);
        assert_eq!(kind(7), None);
    }
}
