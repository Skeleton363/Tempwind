//! Programs for the translator measurements (`docs/design/translator.md`),
//! shared by `tests/spike_bench.rs` (wasmtime) and `testkit spike --bench`,
//! which writes them for the Node scripts in `tests/bench/translator/`.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use iced_x86::code_asm::*;
use serde_json::json;
use x64wasm::abi;
use x64wasm::{CodeRegion, Options};

use super::{Case, asm, asm_with_labels, deep_rec, fib_rec, reg, sentinel_at, sum_loop};
use crate::host::Host;

/// Base address of [`generated_region`].
pub const REGION_BASE: u64 = 0x1_4000_1000;
/// `generated_region(REGION_BASE, REGION_PARTS)` is Phase 0's 201k-instruction region.
pub const REGION_PARTS: usize = 20_000;

/// A large region of straight-line code with forward branches, calls and
/// returns, using every supported instruction family (Phase 0's
/// measurement region). Returns the code and the address of the small
/// function `f` it calls every 16 parts.
pub fn generated_region(base: u64, parts: usize) -> (Vec<u8>, u64) {
    let mut a = CodeAssembler::new(64).unwrap();
    let regs = [
        rax, rcx, rdx, rbx, rsi, rdi, r8, r9, r10, r11, r12, r13, r14, r15,
    ];
    let mut f = a.create_label();
    for i in 0..parts {
        let r = regs[i % regs.len()];
        let s = regs[(i * 7 + 3) % regs.len()];
        let mut skip = a.create_label();
        a.mov(r, s).unwrap();
        a.add(r, (i as i32) & 0x7f).unwrap();
        a.sub(r, qword_ptr(rbx + s * 8 + 0x40)).unwrap();
        a.lea(s, ptr(r + s * 4 + 0x1234)).unwrap();
        a.mov(qword_ptr(rbp - 0x20), r).unwrap();
        a.cmp(r, s).unwrap();
        a.jae(skip).unwrap();
        a.push(r).unwrap();
        a.add(dword_ptr(rsp), 3).unwrap();
        a.pop(s).unwrap();
        a.set_label(&mut skip).unwrap();
        if i % 16 == 15 {
            a.call(f).unwrap();
        }
    }
    a.ret().unwrap();
    a.set_label(&mut f).unwrap();
    a.add(rax, 1).unwrap();
    a.ret().unwrap();
    let (code, addrs) = asm_with_labels(&mut a, base, &[f]).unwrap();
    (code, addrs[0])
}

/// A case that runs only `f` of [`generated_region`]: `rax += 1`. Its
/// module is the whole region, so running it measures the time to the
/// first call of a large module.
pub fn region_case(parts: usize) -> Result<Case> {
    let (code, f) = generated_region(REGION_BASE, parts);
    let stack = REGION_BASE + code.len() as u64 + 0x1_0000;
    Ok(Case {
        name: "region",
        code_base: REGION_BASE,
        entry: f,
        extra_entries: vec![REGION_BASE],
        memory_bytes: stack + 0x1000,
        state_addr: stack + 0x100,
        init_regs: vec![(reg("rsp"), stack), (reg("rax"), 41)],
        init_rflags: 0x202,
        init_mem: vec![sentinel_at(stack)],
        expect_exit: abi::EXIT_LEFT_CODE,
        expect_rip: abi::SENTINEL_RET,
        expect_regs: vec![(reg("rax"), 42), (reg("rsp"), stack + 8)],
        expect_mem: vec![],
        expect_rflags: None,
        code,
    })
}

// Layout of byte_calls: the buffer above the stack.
const BYTES_CODE: u64 = 0x10_0000;
const BYTES_STACK: u64 = 0x1f_f000;
const BYTES_STATE: u64 = 0x30_0000;
const BYTES_BUF: u64 = 0x40_0000;

/// Reference for [`byte_calls`].
fn byte_calls_ref(buf: &[u8]) -> u64 {
    let mut acc = 0u64;
    for &b in buf {
        for _ in 0..8 {
            acc = acc.wrapping_mul(3).wrapping_add(1).wrapping_add(b as u64);
        }
    }
    acc
}

/// byte_calls: a small leaf function called once per byte of a buffer
/// (CoreMark's `crcu8` pattern): `leaf` runs an 8-step loop mixing the byte
/// into an accumulator, with only the instructions the translator supports.
pub fn byte_calls(len: u64) -> Result<Case> {
    let mut a = CodeAssembler::new(64)?;
    let mut top = a.create_label();
    let mut done = a.create_label();
    let mut leaf = a.create_label();
    let mut step = a.create_label();
    a.mov(eax, 0)?;
    a.mov(r9, rdx)?;
    a.lea(r10, ptr(rdx + rcx))?;
    a.cmp(r9, r10)?;
    a.jae(done)?;
    a.set_label(&mut top)?;
    a.mov(ecx, 0)?;
    a.mov(cl, byte_ptr(r9))?;
    a.call(leaf)?;
    a.add(r9, 1)?;
    a.cmp(r9, r10)?;
    a.jb(top)?;
    a.set_label(&mut done)?;
    a.ret()?;
    a.set_label(&mut leaf)?;
    a.mov(edx, 8)?;
    a.set_label(&mut step)?;
    a.lea(rax, ptr(rax + rax * 2 + 1))?;
    a.add(rax, rcx)?;
    a.sub(rdx, 1)?;
    a.jne(step)?;
    a.ret()?;
    let buf: Vec<u8> = (0..len)
        .map(|i| (i.wrapping_mul(0x9e37_79b9) >> 7) as u8)
        .collect();
    Ok(Case {
        name: "byte_calls",
        code_base: BYTES_CODE,
        code: asm(&mut a, BYTES_CODE)?,
        entry: BYTES_CODE,
        extra_entries: vec![],
        memory_bytes: BYTES_BUF + len.max(1),
        state_addr: BYTES_STATE,
        init_regs: vec![
            (reg("rsp"), BYTES_STACK),
            (reg("rcx"), len),
            (reg("rdx"), BYTES_BUF),
        ],
        init_rflags: 0x202,
        init_mem: vec![sentinel_at(BYTES_STACK), (BYTES_BUF, buf.clone())],
        expect_exit: abi::EXIT_LEFT_CODE,
        expect_rip: abi::SENTINEL_RET,
        expect_regs: vec![
            (reg("rax"), byte_calls_ref(&buf)),
            (reg("rsp"), BYTES_STACK + 8),
        ],
        expect_mem: vec![],
        expect_rflags: None,
    })
}

// Layout of the deep recursion probes: a 64 MiB stack.
const PROBE_MEMORY: u64 = 0x410_0000;
const PROBE_STACK: u64 = 0x40f_f000;

/// deep_fat: like `deep_rec`, but every level runs through a function of
/// about `pad` straight-line instructions that keep many values live, to
/// measure the deepest recursion with a large wasm frame.
pub fn deep_fat(n: u64, pad: usize) -> Result<Case> {
    let mut a = CodeAssembler::new(64)?;
    let mut sum = a.create_label();
    let mut zero = a.create_label();
    let regs = [rbx, rsi, rdi, r8, r9, r10, r11, r12, r13, r14, r15];
    a.set_label(&mut sum)?;
    a.cmp(rcx, 0)?;
    a.je(zero)?;
    for i in 0..pad {
        let r = regs[i % regs.len()];
        let s = regs[(i * 5 + 3) % regs.len()];
        a.lea(r, ptr(r + s * 2 + (i as i32 & 0xff)))?;
    }
    a.push(rcx)?;
    a.sub(rcx, 1)?;
    a.call(sum)?;
    a.pop(rcx)?;
    a.add(rax, rcx)?;
    a.ret()?;
    a.set_label(&mut zero)?;
    a.mov(eax, 0)?;
    a.ret()?;
    let total = (n as u128 * (n as u128 + 1) / 2) as u64;
    Ok(Case {
        name: "deep_fat",
        memory_bytes: PROBE_MEMORY,
        init_regs: vec![(reg("rsp"), PROBE_STACK), (reg("rcx"), n)],
        init_mem: vec![sentinel_at(PROBE_STACK)],
        expect_regs: vec![(reg("rsp"), PROBE_STACK + 8), (reg("rax"), total)],
        code: asm(&mut a, super::LOW_CODE)?,
        ..deep_rec(0)?
    })
}

/// `deep_rec` with the probe layout's larger stack.
pub fn deep_probe(n: u64) -> Result<Case> {
    let total = (n as u128 * (n as u128 + 1) / 2) as u64;
    Ok(Case {
        memory_bytes: PROBE_MEMORY,
        init_regs: vec![(reg("rsp"), PROBE_STACK), (reg("rcx"), n)],
        init_mem: vec![sentinel_at(PROBE_STACK)],
        expect_regs: vec![
            (reg("rsp"), PROBE_STACK + 8),
            (reg("rax"), total),
            (reg("rcx"), n),
        ],
        ..deep_rec(n)?
    })
}

/// Best of `runs` timings, with the last result.
pub fn best_of<T>(runs: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut best = Duration::MAX;
    let mut out = None;
    for _ in 0..runs {
        let start = Instant::now();
        let v = f();
        best = best.min(start.elapsed());
        out = Some(v);
    }
    (best, out.unwrap())
}

/// Best native time of a case (x86-64 Unix only).
#[cfg(all(target_arch = "x86_64", unix))]
pub fn native_time(case: &Case, runs: usize) -> Result<Duration> {
    let mut best = Duration::MAX;
    for _ in 0..runs {
        let run = super::native::run_case(case)?;
        case.check_with(&run.state, |a, l| run.read(a, l))?;
        best = best.min(run.elapsed);
    }
    Ok(best)
}

/// `testkit spike --bench --out DIR`: writes the measurement modules and
/// `DIR/manifest.json` (the spike manifest format plus `options` and the
/// native time `nativeMs` where there is one) for
/// `tests/bench/translator/bench.mjs`.
pub fn write(out: &Path) -> Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let host = Host::new()?;
    let no_guard = Options {
        call_depth_limit: None,
        ..Options::default()
    };
    let mut cases: Vec<(String, Case, Options, bool)> = vec![
        ("sum_loop".into(), sum_loop(100_000_000)?, Options::default(), true),
        ("fib_rec".into(), fib_rec(30)?, Options::default(), true),
        ("byte_calls".into(), byte_calls(1 << 20)?, Options::default(), true),
        ("deep_probe".into(), deep_probe(1000)?, no_guard.clone(), false),
        ("deep_fat".into(), deep_fat(1000, 2000)?, no_guard.clone(), false),
    ];
    for max in [500, 1000, 2000, 5000, 10_000, 20_000] {
        let opts = Options {
            max_function_insns: max,
            max_chunk_insns: 10 * max,
            ..Options::default()
        };
        cases.push((format!("region_{max}"), region_case(REGION_PARTS)?, opts, false));
    }
    let mut entries = Vec::new();
    for (name, case, opts, timed) in cases {
        let t = x64wasm::translate_without_dump(
            CodeRegion {
                base: case.code_base,
                bytes: &case.code,
            },
            &case.entries(),
            &opts,
        )
        .map_err(|e| anyhow::anyhow!("translating {name}: {e}"))?;
        if !name.starts_with("region") || name == "region_2000" {
            let (state, machine) = case.run(&host, &t.wasm)?;
            case.check(&state, &machine)?;
        }
        let file = format!("{name}.wasm");
        std::fs::write(out.join(&file), &t.wasm)?;
        let mut entry = case.manifest_entry(&file);
        entry["name"] = json!(name);
        entry["options"] = json!({
            "maxFunctionInsns": opts.max_function_insns,
            "maxChunkInsns": opts.max_chunk_insns,
            "callDepthLimit": opts.call_depth_limit,
        });
        entry["guestInstructions"] = json!(t.guest_instructions);
        entry["functions"] = json!(t.functions.len());
        entry["chunks"] = json!(t.functions.iter().map(|f| f.chunks.len()).sum::<usize>());
        #[cfg(all(target_arch = "x86_64", unix))]
        if timed {
            let native = native_time(&case, 5)?;
            entry["nativeMs"] = json!(native.as_secs_f64() * 1e3);
        }
        let _ = timed;
        println!("{name}: {} wasm bytes", t.wasm.len());
        entries.push(entry);
    }
    let manifest = json!({
        "abi": {
            "stateSize": abi::STATE_SIZE,
            "gpr": abi::STATE_GPR,
            "rip": abi::STATE_RIP,
            "rflags": abi::STATE_RFLAGS,
            "exit": abi::STATE_EXIT,
        },
        "cases": entries,
    });
    let path = out.join("manifest.json");
    std::fs::write(&path, serde_json::to_string(&manifest)? + "\n")?;
    println!("wrote {}", path.display());
    Ok(())
}
