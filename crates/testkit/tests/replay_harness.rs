//! Self-test of the replay harness: mutated expectations must fail, and only
//! the mutated cases. Guards against a replay that passes vacuously.

use testkit::oracle::fixture::{self, Fault, FaultKind, Header};
use testkit::oracle::{fixtures_dir, replay_dir};
use x64wasm::abi::flags;

fn load(family: &str) -> fixture::Fixture {
    let path = fixtures_dir().join(format!("{family}.json"));
    let text = std::fs::read_to_string(&path).expect("reading fixture");
    fixture::parse(&text).expect("parsing fixture").1
}

#[test]
fn replay_reports_mutated_expectations() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("replay_harness");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Keep a few cases per mutation so the test stays fast: one case of each
    // of 10 register forms (replay details at most 3 failures per form),
    // then a memory-destination case.
    let mut add = load("add");
    let m = add
        .cases
        .iter()
        .position(|c| c.form.starts_with("add_m") && c.output.mem != c.input.mem)
        .expect("a memory-destination case");
    let mem_case = add.cases[m].clone();
    // Two more memory-destination cases of other forms, for `undefined.mem`.
    let mut mask_cases: Vec<fixture::Case> = Vec::new();
    for c in &add.cases {
        if mask_cases.len() < 2
            && c.form.starts_with("add_m")
            && c.form != mem_case.form
            && c.output.mem != c.input.mem
            && mask_cases.iter().all(|p| p.form != c.form)
        {
            mask_cases.push(c.clone());
        }
    }
    assert_eq!(mask_cases.len(), 2);
    let mut picked: Vec<fixture::Case> = Vec::new();
    for c in &add.cases {
        if picked.len() < 10
            && c.form != mem_case.form
            && c.output.mem == c.input.mem
            && picked.iter().all(|p| p.form != c.form)
        {
            picked.push(c.clone());
        }
    }
    assert_eq!(picked.len(), 10);
    add.cases = picked;
    add.cases.push(mem_case);
    add.cases.extend(mask_cases);
    let mut jcc = load("jcc");
    jcc.cases.truncate(8);

    // 1. A register result.
    add.cases[1].output.gprs[0] ^= 1 << 63;
    // 2. A defined flag.
    add.cases[2].output.rflags ^= flags::CF;
    // 3. A memory result.
    let changed = add.cases[10]
        .output
        .mem
        .iter()
        .zip(&add.cases[10].input.mem)
        .position(|(o, i)| o != i)
        .unwrap();
    add.cases[10].output.mem[changed].bytes[0] ^= 0x5a;
    // 4. An XMM register.
    add.cases[4].output.fp.xmm[3] ^= 1;
    // 5. A fault where there is none.
    add.cases[5].output.fault = Some(Fault {
        kind: FaultKind::De,
        addr: 0,
    });
    // 6. An x87 field.
    add.cases[6].output.fp.fcw ^= 0x100;
    // Not a mutation: undefined bits are not compared.
    add.cases[7].undefined.gprs[0] = u64::MAX;
    add.cases[7].output.gprs[0] ^= 0xffff;
    // Not a mutation: FP inputs the instruction doesn't touch come back
    // unchanged (state block layout, ST(i) = R[(TOP + i) mod 8]).
    let c = &mut add.cases[8];
    c.input.fp.xmm[5] = 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210;
    c.input.fp.mxcsr = 0x9fc0;
    c.input.fp.fsw = 3 << 11;
    c.input.fp.st[0] = 0x3fff_8000_0000_0000_0000;
    c.input.fp.st[5] = 0x4000_c000_0000_0000_0000;
    c.input.fp.ftw = 0xff3f;
    c.output.fp = c.input.fp;
    // Not a mutation: memory bits under `undefined.mem` are not compared.
    // 8. A memory bit outside the mask (the mask covers the other bits).
    for (n, inside) in [(11, true), (12, false)] {
        let c = &mut add.cases[n];
        let k = c
            .output
            .mem
            .iter()
            .zip(&c.input.mem)
            .position(|(o, i)| o != i)
            .unwrap();
        let addr = c.output.mem[k].addr;
        c.output.mem[k].bytes[0] ^= 0x81;
        c.undefined.mem = vec![fixture::MemBlock {
            addr,
            bytes: vec![if inside { 0x81 } else { 0x7e }],
        }];
    }
    // 7. The branch target.
    jcc.cases[0].output.rip ^= 0x10;
    // Not a mutation: a bit outside flags_mask is not compared.
    add.cases[3].output.rflags ^= 1 << 18;

    for f in [&add, &jcc] {
        let text = fixture::to_json(f, &Header::default());
        std::fs::write(dir.join(format!("{}.json", f.family)), text).unwrap();
    }
    let report = replay_dir(&dir).expect("replaying mutated fixtures");
    let summary = report.render();
    println!("{summary}");
    let total = report.total();
    assert_eq!(total.cases, 21, "{summary}");
    if total.unsupported == 0 {
        assert_eq!(
            total.fail, 8,
            "exactly the 8 mutated cases must fail:\n{summary}"
        );
        for needle in [
            "rax: expected",
            "differs in CF",
            "mem 0x",
            "rip: expected",
            "xmm3: expected",
            "(EXIT_FAULT, fault #DE)",
            "fcw: expected",
            "FAIL add[12]",
        ] {
            assert!(
                summary.contains(needle),
                "missing `{needle}` in:\n{summary}"
            );
        }
        assert!(
            !summary.contains("FAIL add[11]"),
            "a bit under undefined.mem was compared:\n{summary}"
        );
    }
    assert!(!report.ok());
}
