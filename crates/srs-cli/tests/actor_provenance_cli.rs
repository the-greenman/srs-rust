//! RFC-046 CLI session actor (srs-rust#1171): `--actor` / `SRS_ACTOR` stamp `createdBy`,
//! a request-supplied `createdBy` is refused, an invalid actor refuses creation, and an
//! actor session refuses a pre-9 corpus.

use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

fn srs(dir: &std::path::Path, args: &[&str], env_actor: Option<&str>, stdin: &str) -> Value {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_srs"));
    cmd.args(args)
        .current_dir(dir)
        .env_remove("SRS_ACTOR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(a) = env_actor {
        cmd.env("SRS_ACTOR", a);
    }
    let mut child = cmd.spawn().unwrap();
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

fn new_repo() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let r = srs(
        temp.path(),
        &["repo", "create", "--namespace", "com.example.actor"],
        None,
        "",
    );
    assert_eq!(r["ok"], true, "{r}");
    temp
}

const NOTE: &str = r#"{"title":"T","sections":[{"name":"intro","content":"text"}]}"#;
const AGENT: &str = r#"{"kind":"ai","id":"agent-1","name":"Scribe"}"#;

fn refusal(v: &Value) -> String {
    assert_eq!(v["ok"], false, "{v}");
    v["diagnostics"].to_string()
}

#[test]
fn flag_and_env_stamp_created_by_and_no_actor_is_unattributed() {
    let repo = new_repo();
    let by_flag = srs(
        repo.path(),
        &["--actor", AGENT, "note", "create"],
        None,
        NOTE,
    );
    assert_eq!(by_flag["ok"], true, "{by_flag}");
    assert_eq!(
        by_flag["payload"]["note"]["createdBy"],
        serde_json::from_str::<Value>(AGENT).unwrap()
    );
    let by_env = srs(repo.path(), &["note", "create"], Some(AGENT), NOTE);
    assert_eq!(by_env["payload"]["note"]["createdBy"]["id"], "agent-1");
    let none = srs(repo.path(), &["note", "create"], None, NOTE);
    assert_eq!(none["ok"], true);
    assert!(none["payload"]["note"].get("createdBy").is_none());
}

#[test]
fn supplied_invalid_and_too_old_are_refused_with_their_codes() {
    let repo = new_repo();
    let supplied = srs(
        repo.path(),
        &["--actor", AGENT, "note", "create"],
        None,
        r#"{"title":"T","sections":[{"name":"i","content":"x"}],"createdBy":{"kind":"human","id":"u"}}"#,
    );
    assert!(refusal(&supplied).contains("actor-supplied"));

    let invalid = srs(
        repo.path(),
        &["--actor", r#"{"kind":"ai","id":""}"#, "note", "create"],
        None,
        NOTE,
    );
    assert!(refusal(&invalid).contains("actor-invalid"));
    let not_json = srs(
        repo.path(),
        &["--actor", "alice", "note", "create"],
        None,
        NOTE,
    );
    assert!(refusal(&not_json).contains("actor-invalid"));

    // Pre-9 corpus: actor session refused, unattributed session fine.
    let manifest_path = repo.path().join("manifest.json");
    let mut m: Value = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    m["dataModelRevision"] = serde_json::json!(8);
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&m).unwrap()).unwrap();
    let old = srs(
        repo.path(),
        &["--actor", AGENT, "note", "create"],
        None,
        NOTE,
    );
    assert!(refusal(&old).contains("revision-too-old"));
    assert_eq!(
        srs(repo.path(), &["note", "create"], None, NOTE)["ok"],
        true
    );
}

#[test]
fn malformed_created_by_is_actor_supplied_and_precedence_holds() {
    let repo = new_repo();
    let note = r#"{"title":"T","sections":[{"name":"i","content":"x"}],"createdBy":"x"}"#;
    let rel = r#"{"relationType":"evidences","sourceInstanceId":"a","targetInstanceId":"b","createdBy":"x"}"#;
    for (cmd, body) in [("note", note), ("relation", rel)] {
        let r = srs(repo.path(), &["--actor", AGENT, cmd, "create"], None, body);
        assert!(refusal(&r).contains("actor-supplied"), "{cmd}: {r}");
        let r = srs(repo.path(), &[cmd, "create"], None, body);
        assert!(
            refusal(&r).contains("actor-supplied"),
            "{cmd} (no actor): {r}"
        );
        let r = srs(repo.path(), &["--actor", "bad", cmd, "create"], None, body);
        assert!(refusal(&r).contains("actor-invalid"), "{cmd}: {r}");
    }
}
