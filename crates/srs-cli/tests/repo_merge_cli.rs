//! `srs repo merge-preview` / `merge-apply` (srs-rust#1580): the envelope, the stable codes
//! and the stdin selection. The semantics are tested on the service (`merge_copies`).

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::TempDir;

const REPO_ID: &str = "15800000-0000-4000-8000-000000000011";

fn srs(repo: &std::path::Path, args: &[&str], stdin: &str) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_srs"))
        .args(["--repo", repo.to_str().unwrap()])
        .args(args)
        .env("SRS_ACTOR", r#"{"kind":"ai","id":"agent-1"}"#)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)))
}

fn create(dir: &std::path::Path, id: &str) {
    let got = srs(
        dir,
        &[
            "repo",
            "create",
            "--repository-id",
            id,
            "--namespace",
            "com.test.merge",
        ],
        "",
    );
    assert_eq!(got["ok"], true, "{got}");
}

#[test]
fn preview_then_apply_brings_a_note_in_and_keeps_its_created_by() {
    let origin = TempDir::new().unwrap();
    create(origin.path(), REPO_ID);
    let archive_dir = TempDir::new().unwrap();
    let first = archive_dir.path().join("first.srs");
    let second = archive_dir.path().join("second.srs");

    let packed = srs(
        origin.path(),
        &["archive", "pack", "--output", first.to_str().unwrap()],
        "",
    );
    assert_eq!(packed["ok"], true, "{packed}");
    let target = TempDir::new().unwrap();
    let unpacked = srs(
        target.path(),
        &[
            "archive",
            "unpack",
            first.to_str().unwrap(),
            "--target",
            target.path().to_str().unwrap(),
        ],
        "",
    );
    assert_eq!(unpacked["ok"], true, "{unpacked}");

    let note = srs(
        origin.path(),
        &["note", "create"],
        &json!({"title": "added", "sections": [{"name": "body", "content": "x"}]}).to_string(),
    );
    assert_eq!(note["ok"], true, "{note}");
    let id = note["payload"]["note"]["instanceId"].as_str().unwrap();
    srs(
        origin.path(),
        &["archive", "pack", "--output", second.to_str().unwrap()],
        "",
    );

    let from = second.to_str().unwrap();
    let preview = srs(
        target.path(),
        &["repo", "merge-preview", "--from", from],
        "",
    );
    assert_eq!(preview["ok"], true, "{preview}");
    assert_eq!(preview["command"], "repo merge-preview");
    assert_eq!(preview["payload"]["applied"], false);
    assert_eq!(
        preview["payload"]["bring"]["instances"][0]["instanceId"],
        id
    );

    let checked = srs(
        target.path(),
        &["repo", "merge-preview", "--from", from, "--selection"],
        r#"{"instanceIds": []}"#,
    );
    assert_eq!(checked["payload"]["notChosen"][0]["id"], id);

    let applied = srs(
        target.path(),
        &["repo", "merge-apply", "--from", from],
        &json!({"instanceIds": [id]}).to_string(),
    );
    assert_eq!(applied["ok"], true, "{applied}");
    assert_eq!(applied["payload"]["applied"], true);

    let stored = srs(target.path(), &["note", "get", id], "");
    assert_eq!(
        stored["payload"]["note"]["createdBy"]["id"], "agent-1",
        "{stored}"
    );
}

#[test]
fn another_repository_is_refused_with_a_stable_code() {
    let a = TempDir::new().unwrap();
    let b = TempDir::new().unwrap();
    create(a.path(), REPO_ID);
    create(b.path(), "15800000-0000-4000-8000-000000000022");
    let archive_dir = TempDir::new().unwrap();
    let path = archive_dir.path().join("b.srs");
    srs(
        b.path(),
        &["archive", "pack", "--output", path.to_str().unwrap()],
        "",
    );
    let got = srs(
        a.path(),
        &["repo", "merge-apply", "--from", path.to_str().unwrap()],
        "{}",
    );
    assert_eq!(got["ok"], false, "{got}");
    assert_eq!(got["errors"][0]["code"], "merge-repository-mismatch");
}

#[test]
fn a_supplied_created_by_is_refused_with_a_stable_code() {
    let a = TempDir::new().unwrap();
    create(a.path(), REPO_ID);
    let archive_dir = TempDir::new().unwrap();
    let path = archive_dir.path().join("a.srs");
    srs(
        a.path(),
        &["archive", "pack", "--output", path.to_str().unwrap()],
        "",
    );
    let got = srs(
        a.path(),
        &["repo", "merge-apply", "--from", path.to_str().unwrap()],
        r#"{"createdBy": {"kind": "human", "id": "someone"}}"#,
    );
    assert_eq!(got["errors"][0]["code"], "actor-supplied", "{got}");
}
