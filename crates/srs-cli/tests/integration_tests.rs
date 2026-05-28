use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn create_temp_repo() -> TempDir {
    let temp = TempDir::new().expect("Failed to create temp dir");
    let srs_dir = temp.path().join(".srs");
    std::fs::create_dir(&srs_dir).expect("Failed to create .srs dir");

    // Create minimal manifest.json
    let manifest = serde_json::json!({
        "instanceIndex": []
    });
    let manifest_path = temp.path().join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .expect("Failed to write manifest");

    temp
}

fn run_srs_in_dir(dir: &std::path::Path, args: &[&str]) -> Value {
    let exe = env!("CARGO_BIN_EXE_srs");
    let output = Command::new(exe)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("Failed to execute srs command");

    assert!(
        output.status.success(),
        "srs command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
    serde_json::from_str(&stdout).expect("Failed to parse JSON output")
}

fn run_srs_stdin_in_dir(dir: &std::path::Path, args: &[&str], stdin: &str) -> Value {
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

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "srs command failed with exit code {:?}.\nstderr: {}\nstdout: {}",
        output.status.code(),
        stderr,
        stdout_str
    );

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
    serde_json::from_str(&stdout).expect("Failed to parse JSON output")
}

// Read-only tests against live srs repo

fn run_srs(args: &[&str]) -> Value {
    run_srs_in_dir(
        std::path::Path::new("/home/greenman/dev/semanticops/srs"),
        args,
    )
}

#[test]
fn test_note_list_ok() {
    let result = run_srs(&["note", "list"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "note list");
    assert!(result["payload"]["notes"].is_array());
}

#[test]
fn test_note_list_contains_known_note() {
    let result = run_srs(&["note", "list"]);
    let notes = result["payload"]["notes"].as_array().unwrap();

    // Check that origin-purpose note is present
    let origin_purpose = notes
        .iter()
        .find(|n| n["instanceId"].as_str() == Some("d5c7e536-5f7d-491a-8166-5ee25a954377"));
    assert!(
        origin_purpose.is_some(),
        "origin-purpose note should be in list"
    );
}

#[test]
fn test_note_list_filter_by_tag() {
    // Filter by "purpose" tag - should return at least origin-purpose
    let result = run_srs(&["note", "list", "--tag", "purpose"]);
    assert_eq!(result["ok"], true);

    let notes = result["payload"]["notes"].as_array().unwrap();
    assert!(!notes.is_empty(), "Should have notes with 'purpose' tag");
}

#[test]
fn test_note_audit_tags_ok() {
    let result = run_srs(&["note", "audit-tags"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "note audit-tags");
    assert!(result["payload"]["tagAudit"]["tagCounts"].is_array());
}

#[test]
fn test_note_foundations_ok() {
    let result = run_srs(&["note", "foundations"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "note foundations");

    let notes = result["payload"]["foundationNotes"]["notes"]
        .as_array()
        .unwrap();
    assert!(!notes.is_empty(), "Should find foundation notes");
}

#[test]
fn test_repo_map_ok() {
    let result = run_srs(&["repo", "map"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "repo map");
    assert!(
        result["payload"]["repoMap"]["counts"]["totalInstances"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn test_migrate_packet_foundation_ok() {
    let result = run_srs(&["migrate", "packet", "--foundation"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "migrate packet");
    assert_eq!(result["payload"]["profile"], "foundation");
    assert!(result["payload"]["aiHandoffGuidance"]
        .as_str()
        .unwrap()
        .contains("external AI"));
    assert!(result["payload"]["repository"].is_object());
    assert!(result["payload"]["tagAudit"].is_object());
}

#[test]
fn test_note_get_by_id() {
    let result = run_srs(&["note", "get", "d5c7e536-5f7d-491a-8166-5ee25a954377"]);
    assert_eq!(result["ok"], true);

    let note = &result["payload"]["note"];
    assert_eq!(note["instanceId"], "d5c7e536-5f7d-491a-8166-5ee25a954377");

    // Verify sections count (6 sections in origin-purpose.json)
    let sections = note["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 6);
}

#[test]
fn test_note_get_unknown_id_returns_error() {
    let exe = env!("CARGO_BIN_EXE_srs");
    let output = Command::new(exe)
        .args(&["note", "get", "nonexistent-id-12345"])
        .current_dir("/home/greenman/dev/semanticops/srs")
        .output()
        .expect("Failed to execute srs command");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
    let result: Value = serde_json::from_str(&stdout).expect("Failed to parse JSON output");

    assert_eq!(result["ok"], false);
    assert!(!result["diagnostics"].as_array().unwrap().is_empty());
}

// Write tests using temp repo fixture

#[test]
fn test_note_create_and_retrieve() {
    let temp = create_temp_repo();
    let repo_path = temp.path();

    // Create a minimal note via stdin
    let note_json = serde_json::json!({
        "title": "Test Note",
        "sections": [{"name": "test", "content": "Test content"}]
    })
    .to_string();

    let result = run_srs_stdin_in_dir(repo_path, &["note", "create"], &note_json);
    assert_eq!(
        result["ok"], true,
        "create failed: {:?}",
        result["diagnostics"]
    );

    let created = &result["payload"]["note"];
    let id = created["instanceId"].as_str().unwrap();
    assert!(!id.is_empty(), "instanceId should be minted");

    // Retrieve the created note
    let retrieved = run_srs_in_dir(repo_path, &["note", "get", id]);
    assert_eq!(retrieved["ok"], true);
    assert_eq!(retrieved["payload"]["note"]["instanceId"], id);
    assert_eq!(retrieved["payload"]["note"]["title"], "Test Note");

    // Verify file was created
    let note_file = repo_path.join("records/notes/test-note.json");
    assert!(note_file.exists(), "Note file should exist");

    // Verify manifest was updated
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(repo_path.join("manifest.json")).unwrap())
            .unwrap();
    let index = manifest["instanceIndex"].as_array().unwrap();
    assert!(!index.is_empty(), "Manifest should have entry");
    assert_eq!(index[0]["instanceId"], id);
}

#[test]
fn test_note_tag_adds_tag_and_updates_manifest() {
    let temp = create_temp_repo();
    let repo_path = temp.path();

    // Create a note first
    let note_json = serde_json::json!({
        "title": "Tag Test Note",
        "tags": ["initial"],
        "sections": [{"name": "test", "content": "Test content"}]
    })
    .to_string();

    let created = run_srs_stdin_in_dir(repo_path, &["note", "create"], &note_json);
    assert_eq!(created["ok"], true);
    let id = created["payload"]["note"]["instanceId"].as_str().unwrap();

    // Add a new tag
    let tagged = run_srs_in_dir(repo_path, &["note", "tag", id, "new-tag"]);
    assert_eq!(tagged["ok"], true);

    // Verify note file has the new tag
    let retrieved = run_srs_in_dir(repo_path, &["note", "get", id]);
    let tags = retrieved["payload"]["note"]["tags"].as_array().unwrap();
    let tag_strings: Vec<&str> = tags.iter().map(|t| t.as_str().unwrap()).collect();
    assert!(tag_strings.contains(&"initial"));
    assert!(tag_strings.contains(&"new-tag"));

    // Verify manifest was updated
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(repo_path.join("manifest.json")).unwrap())
            .unwrap();
    let index = manifest["instanceIndex"].as_array().unwrap();
    let entry = &index[0];
    let manifest_tags = entry["tags"].as_array().unwrap();
    let manifest_tag_strings: Vec<&str> =
        manifest_tags.iter().map(|t| t.as_str().unwrap()).collect();
    assert!(manifest_tag_strings.contains(&"initial"));
    assert!(manifest_tag_strings.contains(&"new-tag"));
}
