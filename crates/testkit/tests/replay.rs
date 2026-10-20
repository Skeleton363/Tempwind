//! Replays every L1 fixture (crates/testkit/fixtures/l1) through the
//! translator and fails on any mismatch. Unsupported instructions count as
//! failures: Phase 0 requires every recorded form.

use testkit::oracle::{fixtures_dir, replay_dir};

#[test]
fn replay_l1_fixtures() {
    let report = replay_dir(&fixtures_dir()).expect("replaying fixtures");
    let summary = report.render();
    println!("{summary}");
    let total = report.total();
    assert!(total.cases > 0, "no L1 fixtures found in {}", fixtures_dir().display());
    assert!(
        report.ok(),
        "{summary}\nL1 replay: {} of {} cases failed, {} unsupported",
        total.fail,
        total.cases,
        total.unsupported
    );
}
