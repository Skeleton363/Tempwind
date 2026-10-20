//! Replay of L1 fixtures through the translator, in wasmtime.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use wasmtime::Module;
use x64wasm::abi::{CpuState, EXIT_LEFT_CODE, EXIT_UNSUPPORTED, GPR_NAMES, flags};
use x64wasm::{CodeRegion, Options};

use super::fixture::{self, Case, Fixture};
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

fn execute(host: &Host, module: &Module, case: &Case) -> Result<Outcome> {
    let mut m = host.machine(MEMORY_BYTES)?;
    m.write(CODE_ADDR, &case.bytes)?;
    for b in &case.input.mem {
        m.write(b.addr, &b.bytes)?;
    }
    let state = CpuState {
        gpr: case.input.gprs,
        rip: CODE_ADDR,
        rflags: case.input.rflags,
        exit: 0,
    };
    let out = match m.run_state(module, STATE_ADDR, &state) {
        Ok(out) => out,
        Err(e) => return Ok(Outcome::Fail(format!("    run trapped: {e:#}"))),
    };
    if out.exit == EXIT_UNSUPPORTED {
        return Ok(Outcome::Unsupported(format!(
            "    exit EXIT_UNSUPPORTED at rip {:#x}",
            out.rip
        )));
    }
    let want = &case.output;
    let mut diffs = Vec::new();
    if out.exit != EXIT_LEFT_CODE {
        diffs.push(format!(
            "    exit: expected {EXIT_LEFT_CODE} (EXIT_LEFT_CODE) got {}",
            out.exit
        ));
    }
    if out.rip != want.rip {
        diffs.push(format!(
            "    rip: expected {:#x} got {:#x}",
            want.rip, out.rip
        ));
    }
    for (i, name) in GPR_NAMES.iter().enumerate() {
        if out.gpr[i] != want.gprs[i] {
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
