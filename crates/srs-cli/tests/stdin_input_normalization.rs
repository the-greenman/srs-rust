//! End-to-end tests for issue #511 against the real binary:
//! - `document-view create` succeeds without `createdAt` on stdin (normalization)
//! - `note create` with a wrong section shape fails with a JSON-path-bearing error

use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn create_temp_repo() -> TempDir {
    let temp = TempDir::new().expect("Failed to create temp dir");
    std::fs::create_dir_all(temp.path().join(".srs")).unwrap();
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&serde_json::json!({ "instanceIndex": [] })).unwrap(),
    )
    .unwrap();
    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "id": "primary-pkg",
            "namespace": "com.test",
            "name": "primary",
            "version": "1.0.0",
            "fields": [],
            "types": [],
            "relationTypes": [],
            "views": [],
            "documentViews": [],
            "themes": []
        }))
        .unwrap(),
    )
    .unwrap();
    temp
}

/// Run `srs` with stdin; returns (parsed JSON envelope, success flag).
fn run_srs_stdin(dir: &std::path::Path, args: &[&str], stdin: &str) -> (Value, bool) {
    let exe = env!("CARGO_BIN_EXE_srs");
    let mut child = Command::new(exe)
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to spawn srs command");

    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();

    let output = child
        .wait_with_output()
        .expect("Failed to wait for srs command");
    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
    let envelope: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("Failed to parse JSON output: {e}\nstdout: {stdout}"));
    (envelope, output.status.success())
}

#[test]
fn document_view_create_succeeds_without_created_at() {
    let repo = create_temp_repo();
    let body = serde_json::json!({
        "namespace": "com.test",
        "name": "governance-doc",
        "version": 1,
        "sections": [
            {
                "sectionId": "s1",
                "order": 0,
                "source": { "type": "fixed-instances", "instanceIds": [] }
            }
        ]
    });
    let (envelope, success) =
        run_srs_stdin(repo.path(), &["document-view", "create"], &body.to_string());
    assert!(
        success && envelope["ok"] == true,
        "document-view create without createdAt should succeed, got: {envelope}"
    );
    let dv = &envelope["payload"]["documentView"];
    assert!(
        dv["createdAt"].as_str().is_some_and(|s| !s.is_empty()),
        "createdAt should be stamped, got: {envelope}"
    );
}

#[test]
fn note_create_wrong_section_shape_reports_json_path() {
    let repo = create_temp_repo();
    // Issue #511 repro: sections use {heading, body} instead of {name, content}.
    let body = serde_json::json!({
        "title": "Test note",
        "sections": [ { "heading": "Intro", "body": "Some text" } ]
    });
    let (envelope, success) = run_srs_stdin(repo.path(), &["note", "create"], &body.to_string());
    assert!(
        !success && envelope["ok"] == false,
        "note create with wrong section shape should fail, got: {envelope}"
    );
    let diagnostics = envelope["diagnostics"].to_string();
    assert!(
        diagnostics.contains("sections[0]"),
        "error should name the JSON path into sections[0], got: {diagnostics}"
    );
    assert!(
        diagnostics.contains("missing field"),
        "error should say which field is missing, got: {diagnostics}"
    );
}
