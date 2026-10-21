//! Checks, on any platform, that the committed L1 fixtures hold exactly the
//! cases the generator produces (forms, bytes, disassembly, inputs, flag
//! masks). The outputs can only be checked natively: `testkit record --check`.

use testkit::oracle::{fixture, fixtures_dir, generated_inputs};

#[test]
fn fixture_inputs_match_the_generator() {
    let generated = generated_inputs().expect("generating cases");
    let mut diffs = Vec::new();
    for g in &generated {
        let path = fixtures_dir().join(format!("{}.json", g.family));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        let (_, committed) =
            fixture::parse(&text).unwrap_or_else(|e| panic!("parsing {}: {e:#}", path.display()));
        diffs.extend(fixture::diff_inputs(&committed, g));
    }
    assert!(
        diffs.is_empty(),
        "committed fixtures differ from the generator (re-record with `cargo run -p testkit --release -- record`):\n{}",
        diffs
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
