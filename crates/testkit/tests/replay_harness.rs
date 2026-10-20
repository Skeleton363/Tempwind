//! Self-test of the replay harness: mutated expectations must fail, and only
//! the mutated cases. Guards against a replay that passes vacuously.

use testkit::oracle::fixture::{self, Header};
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

    // Keep a few cases per mutation so the test stays fast.
    let mut add = load("add");
    let m = add
        .cases
        .iter()
        .position(|c| c.form.starts_with("add_m") && c.output.mem != c.input.mem)
        .expect("a memory-destination case");
    let mem_case = add.cases[m].clone();
    add.cases.truncate(4);
    add.cases.push(mem_case);
    let mut jcc = load("jcc");
    jcc.cases.truncate(8);

    // 1. A register result.
    add.cases[1].output.gprs[0] ^= 1 << 63;
    // 2. A defined flag.
    add.cases[2].output.rflags ^= flags::CF;
    // 3. A memory result.
    let changed = add.cases[4]
        .output
        .mem
        .iter()
        .zip(&add.cases[4].input.mem)
        .position(|(o, i)| o != i)
        .unwrap();
    add.cases[4].output.mem[changed].bytes[0] ^= 0x5a;
    // 4. The branch target.
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
    assert_eq!(total.cases, 13, "{summary}");
    if total.unsupported == 0 {
        assert_eq!(
            total.fail, 4,
            "exactly the 4 mutated cases must fail:\n{summary}"
        );
        for needle in ["rax: expected", "differs in CF", "mem 0x", "rip: expected"] {
            assert!(
                summary.contains(needle),
                "missing `{needle}` in:\n{summary}"
            );
        }
    }
    assert!(!report.ok());
}
