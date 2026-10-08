//! ADR-053: every `ok:false` envelope carries `errors[]` aligned 1:1 with `diagnostics`.

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

fn srs(dir: &std::path::Path, args: &[&str], stdin: &str) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_srs"))
        .args(args)
        .current_dir(dir)
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

fn ok(dir: &std::path::Path, args: &[&str], stdin: &str) -> Value {
    let v = srs(dir, args, stdin);
    assert_eq!(v["ok"], true, "{args:?}: {v}");
    v
}

fn new_repo() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    ok(
        temp.path(),
        &["repo", "create", "--namespace", "com.example.errs"],
        "",
    );
    temp
}

/// Asserts the aligned-envelope contract and returns `errors[0]`.
fn first_error(v: &Value) -> &Value {
    assert_eq!(v["ok"], false, "{v}");
    let errors = v["errors"].as_array().expect("errors[] on ok:false");
    let diags = v["diagnostics"].as_array().unwrap();
    assert_eq!(errors.len(), diags.len(), "{v}");
    for (e, d) in errors.iter().zip(diags) {
        assert_eq!(e["message"], *d, "{v}");
    }
    &errors[0]
}

#[test]
fn error_envelope_lifecycle_not_defined() {
    let repo = new_repo();
    let p = repo.path();
    let field = "00000000-0000-4000-8000-000000000101";
    let type_id = "00000000-0000-4000-8000-000000000201";
    ok(
        p,
        &["field", "create"],
        &json!({"id": field, "namespace": "com.example.errs", "name": "title", "version": 1,
            "aiGuidance": {"purpose": "title"}, "valueType": "string"})
        .to_string(),
    );
    ok(
        p,
        &["type", "create"],
        &json!({"id": type_id, "namespace": "com.example.errs", "name": "thing", "version": 1,
            "description": "t", "fields": [{"fieldId": field, "order": 0, "required": true}],
            "createdAt": "2026-01-01T00:00:00Z"})
        .to_string(),
    );
    let created = ok(
        p,
        &["record", "create", "--type", "com.example.errs/thing"],
        r#"{"fieldValues":{"title":"x"}}"#,
    );
    let id = created["payload"]["record"]["instanceId"]
        .as_str()
        .or_else(|| created["payload"]["instanceId"].as_str())
        .unwrap_or_else(|| panic!("no id in {created}"))
        .to_string();
    let r = srs(
        p,
        &["record", "transition", "--id", &id],
        r#"{"to":"accepted"}"#,
    );
    assert_eq!(first_error(&r)["code"], "lifecycle-not-defined", "{r}");
}

#[test]
fn error_envelope_cannot_delete_in_use() {
    let repo = new_repo();
    let p = repo.path();
    let rt = "00000000-0000-4000-8000-0000000003aa";
    ok(
        p,
        &["relation-type", "create"],
        &json!({"id": rt, "version": 1, "key": "test-link", "namespace": "com.example.errs",
            "label": "Test link", "description": "d", "category": "dependency",
            "createdAt": "2026-01-01T00:00:00Z"})
        .to_string(),
    );
    let note = r#"{"title":"T","sections":[{"name":"i","content":"x"}]}"#;
    let a = ok(p, &["note", "create"], note);
    let b = ok(p, &["note", "create"], note);
    let id = |v: &Value| {
        v["payload"]["note"]["instanceId"]
            .as_str()
            .or_else(|| v["payload"]["instanceId"].as_str())
            .unwrap_or_else(|| panic!("no id in {v}"))
            .to_string()
    };
    ok(
        p,
        &["relation", "create"],
        &json!({"relationType": "test-link", "sourceInstanceId": id(&a), "targetInstanceId": id(&b)})
            .to_string(),
    );
    let r = srs(p, &["relation-type", "delete", rt], "");
    let e = first_error(&r);
    assert_eq!(e["code"], "cannot-delete-in-use", "{r}");
    assert!(
        !e["details"]["usedBy"].as_array().unwrap().is_empty(),
        "{r}"
    );
}

#[test]
fn error_envelope_handler_path_survives_reparse() {
    let repo = new_repo();
    let r = srs(
        repo.path(),
        &["relation", "get", "00000000-0000-4000-8000-000000000000"],
        "",
    );
    assert_eq!(first_error(&r)["code"], "relation-not-found", "{r}");
}
