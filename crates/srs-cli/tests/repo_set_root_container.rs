//! RFC-052 Change D / [R8] (owner rulings on srs#953 and srs#963): setting the root is
//! pointer-only, an unknown id is `container-not-found`, unsetting the root and deleting it
//! are refused. Every refusal and every swap leaves each container file byte-identical.

use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn srs(dir: &Path, args: &[&str], stdin: &str) -> Value {
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
    serde_json::from_slice(&out.stdout).unwrap()
}

fn ok(dir: &Path, args: &[&str], stdin: &str) -> Value {
    let v = srs(dir, args, stdin);
    assert_eq!(v["ok"], true, "{args:?}: {v}");
    v
}

fn code(v: &Value) -> &str {
    assert_eq!(v["ok"], false, "{v}");
    v["errors"][0]["code"].as_str().unwrap()
}

/// Every file under `containers/`, by path.
fn containers(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir.join("containers"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&p).unwrap(),
            )
        })
        .collect()
}

fn root_id(dir: &Path) -> String {
    let m: Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    m["rootContainerId"].as_str().unwrap().to_string()
}

/// A `repo create` repository plus one ordinary container `B` holding one note.
fn repo() -> (tempfile::TempDir, String, String) {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    ok(
        d,
        &["repo", "create", "--namespace", "com.example.root"],
        "",
    );
    let note = ok(
        d,
        &["note", "create"],
        r#"{"title":"B identity","sections":[{"name":"body","content":"b"}]}"#,
    );
    let note_id = note["payload"]["note"]["instanceId"].as_str().unwrap();
    let b = ok(
        d,
        &["container", "create"],
        &format!(
            r#"{{"title":"Container B","identityInstanceId":"{note_id}","memberInstanceIds":[{{"instanceId":"{note_id}"}}]}}"#
        ),
    );
    let b_id = b["payload"]["container"]["containerId"]
        .as_str()
        .unwrap()
        .to_string();
    let a_id = root_id(d);
    (tmp, a_id, b_id)
}

#[test]
fn a_swap_and_back_changes_only_the_manifest() {
    let (tmp, a_id, b_id) = repo();
    let d = tmp.path();
    let before = containers(d);

    let v = ok(
        d,
        &["repo", "set-root-container", "--container-id", &b_id],
        "",
    );
    // The promoted container is reported as stored: nothing copied into it.
    assert_eq!(v["payload"]["title"], "Container B");
    assert_eq!(root_id(d), b_id);
    assert_eq!(containers(d), before, "the swap changed a container file");
    // The previous root is an ordinary container; the repository still validates.
    let validate = ok(d, &["repo", "validate"], "");
    assert_eq!(validate["payload"]["summary"]["errors"], 0, "{validate}");

    ok(
        d,
        &["repo", "set-root-container", "--container-id", &a_id],
        "",
    );
    assert_eq!(root_id(d), a_id);
    assert_eq!(
        containers(d),
        before,
        "swapping back changed a container file"
    );
}

#[test]
fn an_unknown_id_is_container_not_found_and_writes_nothing() {
    let (tmp, a_id, _) = repo();
    let d = tmp.path();
    let manifest = std::fs::read(d.join("manifest.json")).unwrap();
    let before = containers(d);
    let v = srs(
        d,
        &[
            "repo",
            "set-root-container",
            "--container-id",
            "99999999-0000-4000-8000-000000000099",
        ],
        "",
    );
    assert_eq!(code(&v), "container-not-found");
    assert_eq!(std::fs::read(d.join("manifest.json")).unwrap(), manifest);
    assert_eq!(containers(d), before);
    assert_eq!(root_id(d), a_id);
}

#[test]
fn unsetting_or_deleting_the_root_is_refused_and_writes_nothing() {
    let (tmp, a_id, _) = repo();
    let d = tmp.path();
    let manifest = std::fs::read(d.join("manifest.json")).unwrap();
    let before = containers(d);

    let v = srs(d, &["repo", "unset-root-container"], "");
    assert_eq!(code(&v), "manifest-root-container-missing");
    let v = srs(d, &["container", "delete", &a_id], "");
    assert_eq!(code(&v), "container-root-delete");

    assert_eq!(std::fs::read(d.join("manifest.json")).unwrap(), manifest);
    assert_eq!(containers(d), before);
}
