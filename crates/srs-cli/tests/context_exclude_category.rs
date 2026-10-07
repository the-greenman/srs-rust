//! `srs context record --exclude-category` (#1188): an unknown category is refused by the
//! shared `FromStr` parser, a known one is accepted.

use std::process::Command;

fn run(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_srs"))
        .env_remove("SRS_ACTOR")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

#[test]
fn unknown_category_is_rejected_known_is_parsed() {
    let tmp = tempfile::tempdir().unwrap();
    let bad = run(
        tmp.path(),
        &["context", "record", "x", "--exclude-category", "nope"],
    );
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unknown relation category 'nope'"));

    // Parses: the failure is now the missing repository, not the flag.
    let ok = run(
        tmp.path(),
        &["context", "record", "x", "--exclude-category", "sequence"],
    );
    assert!(!String::from_utf8_lossy(&ok.stderr).contains("unknown relation category"));
}
