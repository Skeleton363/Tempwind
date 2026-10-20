//! `testkit record [--check]`: generate cases, run them natively, write or
//! compare the fixture files.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};

use super::fixture::{self, Case, Fixture, Header, MemBlock, Output};
use super::forms::{self, Family, GenCase};
use super::{SEED, fixtures_dir};

/// How many differences `--check` prints before summarizing the rest.
const MAX_DIFFS: usize = 40;

pub fn record(check: bool) -> Result<()> {
    let start = Instant::now();
    let families = forms::generate()?;
    let generated = start.elapsed();
    let fixtures = run_native(&families)?;
    let ran = start.elapsed() - generated;
    let total: usize = fixtures.iter().map(|f| f.cases.len()).sum();
    eprintln!(
        "recorded {total} cases in {} families: generate {:.2?}, native {:.2?}",
        fixtures.len(),
        generated,
        ran
    );
    let dir = fixtures_dir();
    if check {
        check_against(&dir, &fixtures)
    } else {
        write_all(&dir, &fixtures)
    }
}

/// Fixtures with the generated inputs and empty outputs, for checking the
/// committed inputs on machines that cannot record.
pub fn generated_inputs() -> Result<Vec<Fixture>> {
    let families = forms::generate()?;
    Ok(families
        .iter()
        .map(|family| {
            let cases = family
                .cases
                .iter()
                .map(|c| to_case(c, c.input.gprs, 0, 0, Vec::new()))
                .collect();
            Fixture {
                family: family.name.to_string(),
                seed: SEED,
                default_gprs: forms::default_gprs(),
                cases,
            }
        })
        .collect())
}

fn to_case(c: &GenCase, gprs: [u64; 16], rip: u64, rflags: u64, mem: Vec<MemBlock>) -> Case {
    Case {
        form: c.form.clone(),
        bytes: c.bytes.clone(),
        asm: c.asm.clone(),
        flags_mask: c.flags_mask,
        input: c.input.clone(),
        output: Output {
            gprs,
            rip,
            rflags,
            mem,
        },
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn run_native(_families: &[Family]) -> Result<Vec<Fixture>> {
    bail!(
        "testkit record needs x86-64 Linux: it single-steps each instruction on the host CPU under ptrace (this build is {}-{})",
        std::env::consts::ARCH,
        std::env::consts::OS
    )
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn run_native(families: &[Family]) -> Result<Vec<Fixture>> {
    use super::native::Tracee;
    use super::{DATA_ADDR, DATA_SIZE};
    use x64wasm::abi::flags;

    let mut tracee = Tracee::spawn()?;
    let defaults = forms::default_gprs();
    let mut out = Vec::new();
    for family in families {
        let mut cases = Vec::new();
        for (n, c) in family.cases.iter().enumerate() {
            let at = || format!("{}[{n}] {} `{}` ({:02x?})", family.name, c.form, c.asm, c.bytes);
            let mut image = vec![0u8; DATA_SIZE as usize];
            for block in &c.input.mem {
                let off = (block.addr - DATA_ADDR) as usize;
                image[off..off + block.bytes.len()].copy_from_slice(&block.bytes);
            }
            let step = tracee
                .step(&c.bytes, &image, &c.input.gprs, c.input.rflags)
                .with_context(at)?;
            ensure!(
                step.rip == c.expect_rip,
                "{}: rip after the instruction is {:#x}, the generator expected {:#x}",
                at(),
                step.rip,
                c.expect_rip
            );
            let kept = !(flags::STATUS | flags::TF);
            ensure!(
                (step.rflags ^ c.input.rflags) & kept == 0,
                "{}: rflags {:#x} -> {:#x} changed a non-status bit",
                at(),
                c.input.rflags,
                step.rflags
            );
            // Every changed byte must lie inside a compared range.
            let mut inside = vec![false; DATA_SIZE as usize];
            for block in &c.input.mem {
                let off = (block.addr - DATA_ADDR) as usize;
                inside[off..off + block.bytes.len()].fill(true);
            }
            if let Some(i) = (0..image.len()).find(|&i| image[i] != step.data[i] && !inside[i]) {
                bail!(
                    "{}: wrote {:#x}, outside the compared ranges (generator bug)",
                    at(),
                    DATA_ADDR + i as u64
                );
            }
            let mem = c
                .input
                .mem
                .iter()
                .map(|block| {
                    let off = (block.addr - DATA_ADDR) as usize;
                    MemBlock {
                        addr: block.addr,
                        bytes: step.data[off..off + block.bytes.len()].to_vec(),
                    }
                })
                .collect();
            cases.push(to_case(c, step.gprs, step.rip, step.rflags, mem));
        }
        out.push(Fixture {
            family: family.name.to_string(),
            seed: SEED,
            default_gprs: defaults,
            cases,
        });
    }
    Ok(out)
}

fn file_name(family: &str) -> String {
    format!("{family}.json")
}

fn header() -> Header {
    Header {
        cpu: cpu_model(),
        kernel: kernel_release(),
        recorded: today(),
    }
}

fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn kernel_release() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| format!("Linux {}", s.trim()))
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Today's UTC date as YYYY-MM-DD.
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

fn write_all(dir: &Path, fixtures: &[Fixture]) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let header = header();
    let mut total_bytes = 0;
    for f in fixtures {
        let path = dir.join(file_name(&f.family));
        let text = fixture::to_json(f, &header);
        // Sanity: the file must parse back to exactly what was recorded.
        let (_, back) = fixture::parse(&text).context("re-parsing a fresh fixture")?;
        ensure!(&back == f, "{}: fixture does not round-trip", f.family);
        let unchanged = std::fs::read_to_string(&path)
            .ok()
            .and_then(|old| fixture::parse(&old).ok())
            .is_some_and(|(_, old)| &old == f);
        let size = if unchanged {
            std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
        } else {
            std::fs::write(&path, &text).with_context(|| format!("writing {}", path.display()))?;
            text.len() as u64
        };
        total_bytes += size;
        println!(
            "{:<10} {:>5} cases {:>8} bytes  {}",
            file_name(&f.family),
            f.cases.len(),
            size,
            if unchanged { "unchanged" } else { "written" }
        );
    }
    println!("total {total_bytes} bytes in {}", dir.display());
    Ok(())
}

fn check_against(dir: &Path, fixtures: &[Fixture]) -> Result<()> {
    let mut diffs = Vec::new();
    for f in fixtures {
        let path = dir.join(file_name(&f.family));
        let committed = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                diffs.push(format!("{}: cannot read: {e}", path.display()));
                continue;
            }
        };
        match fixture::parse(&committed) {
            Ok((_, old)) => diffs.extend(fixture::diff(&old, f)),
            Err(e) => diffs.push(format!("{}: {e:#}", path.display())),
        }
    }
    let known: Vec<String> = fixtures.iter().map(|f| file_name(&f.family)).collect();
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut extra: Vec<String> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".json") && !known.contains(name))
            .collect();
        extra.sort();
        for name in extra {
            diffs.push(format!("{name}: committed but no longer recorded"));
        }
    }
    if diffs.is_empty() {
        println!(
            "record --check: {} fixture files match a fresh recording",
            fixtures.len()
        );
        return Ok(());
    }
    for d in diffs.iter().take(MAX_DIFFS) {
        eprintln!("{d}");
    }
    if diffs.len() > MAX_DIFFS {
        eprintln!("... and {} more differences", diffs.len() - MAX_DIFFS);
    }
    bail!(
        "record --check: {} differences between the committed fixtures and a fresh recording",
        diffs.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_746), (2026, 10, 20));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }
}
