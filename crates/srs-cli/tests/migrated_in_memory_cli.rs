//! srs-rust#1444 on a published seed: opening a pre-10 `.srsj` says it was migrated in
//! memory; reads leave the file byte-unchanged; a creating command is refused with
//! `migration-required-before-write` and leaves it byte-unchanged; after an explicit
//! `repo apply-migration` of every needed step the write succeeds.

use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

fn srs(dir: &std::path::Path, args: &[&str], stdin: &str) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_srs"))
        .args(args)
        .current_dir(dir)
        .env_remove("SRS_ACTOR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "bad output ({e}): {} / {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

const NOTE: &str = r#"{"title":"T","sections":[{"name":"intro","content":"text"}]}"#;
const AGENT: &str = r#"{"kind":"ai","id":"agent-1"}"#;

fn notices(v: &Value) -> Vec<String> {
    v["payload"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["message"].as_str())
        .filter(|m| m.starts_with("migrated-in-memory:"))
        .map(str::to_string)
        .collect()
}

#[test]
fn published_seed_is_never_saved_at_revision_10_unasked() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("seed.srsj");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../srs-repository/tests/fixtures/published/governance-1.2.2-seed.srsj"),
        &doc,
    )
    .unwrap();
    let original = std::fs::read(&doc).unwrap();
    let repo = doc.to_str().unwrap();

    // N6: one info diagnostic naming the source revision and the migrations.
    let v = srs(dir.path(), &["repo", "validate", "--repo", repo], "");
    assert_eq!(v["ok"], true, "{v}");
    let n = notices(&v);
    assert_eq!(n.len(), 1, "{v}");
    for part in [
        "dataModelRevision 7",
        "rfc043-container-entries",
        "rfc046-actor-provenance",
        "rfc052-root-container",
    ] {
        assert!(n[0].contains(part), "{}", n[0]);
    }

    // Reads work and do not touch the file.
    for args in [
        &["note", "list", "--repo", repo][..],
        &["repo", "navigation", "--repo", repo][..],
        &["record", "list", "--repo", repo][..],
    ] {
        let v = srs(dir.path(), args, "");
        assert_eq!(v["ok"], true, "{args:?}: {v}");
    }
    assert_eq!(std::fs::read(&doc).unwrap(), original);

    // N5: a creating command (with and without an actor) is refused; the file is unchanged.
    for extra in [&[][..], &["--actor", AGENT][..]] {
        let mut args = vec!["note", "create", "--repo", repo];
        args.extend_from_slice(extra);
        let v = srs(dir.path(), &args, NOTE);
        assert_eq!(v["ok"], false, "{v}");
        assert_eq!(
            v["errors"][0]["code"], "migration-required-before-write",
            "{v}"
        );
        assert_eq!(std::fs::read(&doc).unwrap(), original);
    }

    // The explicit act: apply the needed migrations, in registry order, re-reading the list
    // after each (an earlier step makes a later one applicable).
    let mut applied = Vec::new();
    loop {
        let v = srs(dir.path(), &["repo", "migrations", "--repo", repo], "");
        assert_eq!(v["ok"], true, "{v}");
        let Some(id) = v["payload"]["migrations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["status"]["needed"] == true)
            .map(|m| m["id"].as_str().unwrap().to_string())
        else {
            break;
        };
        let v = srs(
            dir.path(),
            &["repo", "apply-migration", "--id", &id, "--repo", repo],
            "",
        );
        assert_eq!(v["ok"], true, "{id}: {v}");
        assert!(applied.len() < 12, "{applied:?}");
        applied.push(id);
    }
    assert!(
        applied.iter().any(|id| id == "rfc052-root-container"),
        "{applied:?}"
    );
    assert_ne!(std::fs::read(&doc).unwrap(), original);

    let v = srs(
        dir.path(),
        &["note", "create", "--repo", repo, "--actor", AGENT],
        NOTE,
    );
    assert_eq!(v["ok"], true, "{v}");
    let v = srs(dir.path(), &["repo", "validate", "--repo", repo], "");
    assert!(notices(&v).is_empty(), "{v}");
}
