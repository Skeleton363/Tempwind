//! Whole-image measurements of the Phase 1 targets (ignored by default;
//! numbers only mean something in a release build on an idle machine):
//!
//! ```text
//! scripts/fetch-artifact.sh "$(cat tests/targets/targets.lock)" 7zz.exe target/targets
//! scripts/fetch-artifact.sh "$(cat tests/targets/targets.lock)" coremark.exe target/targets
//! scripts/fetch-artifact.sh "$(cat tests/targets/targets.lock)" census.json target/targets
//! cargo test -p x64wasm-cli --release --test images -- --ignored --nocapture
//! ```
//!
//! Per image, with `--entries all`: `translate-pe` time and peak RSS (the
//! CLI as a child process), in-process translation time (median of 3), wasm
//! bytes raw and gzip -9, functions, chunks, blocks, guest instructions as a
//! share of the census, indirect jumps and recovered jump tables,
//! `EXIT_UNSUPPORTED` sites by mnemonic, the census's out-of-profile
//! functions, wasmparser validation and wasmtime (Cranelift) compile time
//! (median of 3). Recovered tables are listed in
//! `target/bench/images/<stem>.tables.txt`.

use std::collections::{BTreeMap, HashMap};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use iced_x86::{Decoder, Mnemonic};
use x64wasm::image::{self, EntrySet};
use x64wasm::pe::Pe;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Runs `translate-pe` as a child; returns (wall ms, peak RSS in KiB).
// The child is reaped with wait4, which also gives its own peak RSS.
#[allow(clippy::zombie_processes)]
fn translate_pe(exe: &Path, out_dir: &Path) -> (f64, i64) {
    let start = Instant::now();
    let child = Command::new(env!("CARGO_BIN_EXE_x64wasm"))
        .arg("translate-pe")
        .arg(exe)
        .arg("--out-dir")
        .arg(out_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut status = 0;
    // SAFETY: plain wait4 on our own child, with valid out pointers.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let pid = unsafe { libc::wait4(child.id() as i32, &mut status, 0, &mut usage) };
    assert_eq!(pid, child.id() as i32);
    assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0);
    (start.elapsed().as_secs_f64() * 1e3, usage.ru_maxrss)
}

fn gzip_len(bytes: &[u8]) -> usize {
    let mut child = Command::new("gzip")
        .arg("-9c")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("gzip");
    let mut stdin = child.stdin.take().unwrap();
    let data = bytes.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&data).unwrap());
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    out.stdout.len()
}

struct Census {
    functions: u64,
    instructions: u64,
    out_of_profile: Vec<String>,
}

fn census(name: &str) -> Census {
    let path = root().join("target/targets/census.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; fetch it first (see the doc comment)",
            path.display()
        )
    });
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let b = v["binaries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"].as_str().is_some_and(|n| n.ends_with(name)))
        .unwrap_or_else(|| panic!("{name} not in census.json"));
    Census {
        functions: b["functions"].as_u64().unwrap(),
        instructions: b["function_instructions"].as_u64().unwrap(),
        out_of_profile: b["out_of_profile_functions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap().to_string())
            .collect(),
    }
}

fn measure(stem: &str, host: &testkit::host::Host) {
    let file = format!("{stem}.exe");
    let exe = root().join("target/targets").join(&file);
    let data = std::fs::read(&exe).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; fetch it first (see the doc comment)",
            exe.display()
        )
    });
    let out_dir = root().join("target/bench/images");
    std::fs::create_dir_all(&out_dir).unwrap();
    let (cli_ms, rss_kib) = translate_pe(&exe, &out_dir);

    let pe = Pe::parse(&data).unwrap();
    let opts = x64wasm::Options::default();
    let mut times = Vec::new();
    let mut t = None;
    for _ in 0..3 {
        let start = Instant::now();
        let r = image::translate(&pe, EntrySet::All, &opts).unwrap();
        times.push(start.elapsed().as_secs_f64() * 1e3);
        t = Some(r);
    }
    let t = t.unwrap();
    let aot_ms = median(times);
    assert_eq!(
        std::fs::read(out_dir.join(format!("{stem}.wasm"))).unwrap(),
        t.wasm,
        "the CLI writes the same module"
    );
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&t.wasm)
        .unwrap();
    let gz = gzip_len(&t.wasm);
    let chunks: usize = t.functions.iter().map(|f| f.chunks.len()).sum();

    // Unsupported sites by mnemonic.
    let sections = image::executable_sections(&pe).unwrap();
    let mut by_mnemonic: HashMap<Mnemonic, usize> = HashMap::new();
    for &rip in &t.unsupported {
        let Some((base, bytes)) = sections
            .iter()
            .find(|(b, s)| rip >= *b && rip < b + s.len() as u64)
        else {
            continue;
        };
        let mut d = Decoder::with_ip(
            64,
            &bytes[(rip - base) as usize..],
            rip,
            x64wasm::DECODER_OPTIONS,
        );
        *by_mnemonic.entry(d.decode().mnemonic()).or_default() += 1;
    }
    let mut top: Vec<_> = by_mnemonic.into_iter().collect();
    top.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(format!("{:?}", a.0).cmp(&format!("{:?}", b.0)))
    });
    let top: Vec<String> = top
        .iter()
        .take(10)
        .map(|(m, n)| format!("{m:?} {n}"))
        .collect();

    // The census's out-of-profile functions: each is a translated function.
    let census = census(&file);
    let symbols: BTreeMap<String, u64> = pe
        .symbols()
        .unwrap_or_default()
        .iter()
        .filter(|s| s.is_function() && s.section > 0)
        .filter_map(|s| {
            pe.symbol_rva(s)
                .map(|rva| (s.name.clone(), pe.rva_to_va(rva)))
        })
        .collect();
    let mut oop_translated = 0;
    for name in &census.out_of_profile {
        let addr = symbols
            .get(name)
            .copied()
            .or_else(|| {
                u64::from_str_radix(name.trim_start_matches("0x"), 16)
                    .ok()
                    .map(|r| pe.rva_to_va(r as u32))
            })
            .unwrap_or_else(|| panic!("no symbol for {name}"));
        assert!(
            t.functions.iter().any(|f| f.addr == addr),
            "{name} ({addr:#x}) is not a translated function"
        );
        oop_translated += 1;
    }
    let pdata = pe.runtime_functions().unwrap_or_default();
    let mut pdata_starts: Vec<u64> = pdata
        .iter()
        .filter(|f| f.begin_rva < f.end_rva)
        .map(|f| pe.rva_to_va(f.begin_rva))
        .collect();
    pdata_starts.sort();
    pdata_starts.dedup();
    let pdata_translated = pdata_starts
        .iter()
        .filter(|a| t.blocks.binary_search(a).is_ok())
        .count();

    let mut compile = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        host.compile(&t.wasm).unwrap();
        compile.push(start.elapsed().as_secs_f64() * 1e3);
    }

    let mut tables = String::new();
    for jt in &t.jump_tables {
        tables += &format!(
            "{:#x} table {:#x} entries {}\n",
            jt.jmp,
            jt.table,
            jt.targets.len()
        );
    }
    std::fs::write(out_dir.join(format!("{stem}.tables.txt")), tables).unwrap();

    let pct = |a: f64, b: f64| 100.0 * a / b;
    println!("## {file}");
    println!("| | |\n|---|---|");
    println!(
        "| translate-pe (CLI, wall) | {cli_ms:.0} ms, peak RSS {} MiB |",
        rss_kib / 1024
    );
    println!("| AOT translation (in process, median of 3) | {aot_ms:.0} ms |");
    println!(
        "| wasm | {} bytes, gzip -9 {} bytes ({:.1} bytes per guest instruction) |",
        t.wasm.len(),
        gz,
        t.wasm.len() as f64 / t.guest_instructions as f64
    );
    println!(
        "| functions / chunks / blocks | {} / {} / {} |",
        t.functions.len(),
        chunks,
        t.blocks.len()
    );
    println!(
        "| guest instructions | {} ({:.1} % of the census's {} in .pdata functions) |",
        t.guest_instructions,
        pct(t.guest_instructions as f64, census.instructions as f64),
        census.instructions
    );
    println!(
        "| .pdata functions translated | {} of {} ({:.1} %; census: {}) |",
        pdata_translated,
        pdata_starts.len(),
        pct(pdata_translated as f64, pdata_starts.len() as f64),
        census.functions
    );
    println!(
        "| indirect jumps / recovered jump tables | {} / {} ({} targets) |",
        t.indirect_jumps,
        t.jump_tables.len(),
        t.jump_tables.iter().map(|j| j.targets.len()).sum::<usize>()
    );
    println!(
        "| EXIT_UNSUPPORTED sites | {}: {} |",
        t.unsupported.len(),
        top.join(", ")
    );
    println!(
        "| out-of-profile functions (census) | {} of {} translated |",
        oop_translated,
        census.out_of_profile.len()
    );
    println!("| wasmparser validation | ok |");
    println!(
        "| wasmtime compile (Cranelift, median of 3) | {:.0} ms |",
        median(compile)
    );
    println!();
}

#[test]
#[ignore = "measurement; needs target/targets (see the doc comment); run with --release --ignored --nocapture"]
fn image_measurements() {
    let host = testkit::host::Host::new().unwrap();
    for stem in ["coremark", "7zz"] {
        measure(stem, &host);
    }
}
