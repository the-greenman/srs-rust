//! srs-rust#1668: `repo guidance set` writes only `manifest.aiGuidance`, round-trips
//! through `get`, and refuses an entry point that names no instance.

use serde_json::{json, Value};
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
    serde_json::from_slice(&child.wait_with_output().unwrap().stdout).unwrap()
}

fn ok(dir: &Path, args: &[&str], stdin: &str) -> Value {
    let v = srs(dir, args, stdin);
    assert_eq!(v["ok"], true, "{args:?}: {v}");
    v
}

fn manifest(dir: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap()
}

#[test]
fn guidance_set_round_trips_and_writes_only_ai_guidance() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    ok(
        d,
        &["repo", "create", "--namespace", "com.example.guided"],
        "",
    );
    let note = ok(
        d,
        &["note", "create"],
        r#"{"title":"Guide","sections":[{"name":"body","content":"g"}]}"#,
    );
    let note_id = note["payload"]["note"]["instanceId"].as_str().unwrap();
    assert_eq!(
        ok(d, &["repo", "guidance", "get"], "")["payload"],
        json!({"aiGuidance": null})
    );
    let before = manifest(d);

    let set = ok(
        d,
        &["repo", "guidance", "set"],
        &json!({"summary": "S", "navigationHints": "H", "suggestedEntryPoints": [note_id]})
            .to_string(),
    );
    let guidance = &set["payload"]["aiGuidance"];
    let path = guidance["suggestedEntryPoints"][0].as_str().unwrap();
    assert!(
        path.starts_with("records/") && path.ends_with(".json"),
        "{path}"
    );
    assert_eq!(
        ok(d, &["repo", "guidance", "get"], "")["payload"],
        set["payload"]
    );

    let mut after = manifest(d);
    assert_eq!(after["aiGuidance"], *guidance);
    after.as_object_mut().unwrap().remove("aiGuidance");
    assert_eq!(after, before, "set changed more than manifest.aiGuidance");

    // Refusals write nothing.
    let unchanged = manifest(d);
    let v = srs(
        d,
        &["repo", "guidance", "set"],
        r#"{"suggestedEntryPoints":["records/notes/nothing.json"]}"#,
    );
    assert_eq!(v["errors"][0]["code"], "instance-not-found", "{v}");
    let v = srs(d, &["repo", "guidance", "set"], r#"{"other":"x"}"#);
    assert_eq!(v["ok"], false, "an undeclared key is refused: {v}");
    assert_eq!(manifest(d), unchanged);
}
