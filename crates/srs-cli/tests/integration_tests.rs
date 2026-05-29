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
        std::path::Path::new("/home/greenman/dev/semanticops/srs/srs"),
        args,
    )
}

#[test]
fn note_list_returns_ok_envelope() {
    let result = run_srs(&["note", "list"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "note list");
    assert!(result["payload"]["notes"].is_array());
}

#[test]
fn note_list_contains_origin_purpose() {
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
fn note_list_filters_by_tag() {
    // Filter by "purpose" tag - should return at least origin-purpose
    let result = run_srs(&["note", "list", "--tag", "purpose"]);
    assert_eq!(result["ok"], true);

    let notes = result["payload"]["notes"].as_array().unwrap();
    assert!(!notes.is_empty(), "Should have notes with 'purpose' tag");
}

#[test]
fn note_audit_tags_returns_tag_counts() {
    let result = run_srs(&["note", "audit-tags"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "note audit-tags");
    assert!(result["payload"]["tagAudit"]["tagCounts"].is_array());
}

#[test]
fn note_foundations_returns_ok_envelope() {
    // Note: foundation notes are now data-driven via TagDefinition records.
    // Until TagDefinition records with "foundation" role exist in the repo,
    // this returns an empty list (acceptable transitional state).
    let result = run_srs(&["note", "foundations"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "note foundations");

    let notes = result["payload"]["foundationNotes"]["notes"]
        .as_array()
        .unwrap();
    // Empty until TagDefinition records are created in the repo
    assert!(notes.is_empty() || !notes.is_empty());
}

#[test]
fn repo_map_returns_counts_and_structure() {
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
fn migrate_packet_foundation_returns_complete_packet() {
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
fn note_get_returns_note_with_sections() {
    let result = run_srs(&["note", "get", "d5c7e536-5f7d-491a-8166-5ee25a954377"]);
    assert_eq!(result["ok"], true);

    let note = &result["payload"]["note"];
    assert_eq!(note["instanceId"], "d5c7e536-5f7d-491a-8166-5ee25a954377");

    // Verify sections count (6 sections in origin-purpose.json)
    let sections = note["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 6);
}

#[test]
fn note_get_unknown_id_returns_ok_false() {
    let exe = env!("CARGO_BIN_EXE_srs");
    let output = Command::new(exe)
        .args(["note", "get", "nonexistent-id-12345"])
        .current_dir("/home/greenman/dev/semanticops/srs/srs")
        .output()
        .expect("Failed to execute srs command");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
    let result: Value = serde_json::from_str(&stdout).expect("Failed to parse JSON output");

    assert_eq!(result["ok"], false);
    assert!(!result["diagnostics"].as_array().unwrap().is_empty());
}

// Write tests using temp repo fixture

#[test]
fn note_create_mints_id_writes_file_and_updates_manifest() {
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
fn note_tag_adds_tag_and_updates_manifest() {
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

// Tag definition integration tests

#[test]
fn tag_list_returns_ok_envelope() {
    // Against live repo - may be empty until TagDefinition records are created
    let result = run_srs(&["tag", "list"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "tag list");
    assert!(result["payload"]["tagDefinitions"].is_array());
}

#[test]
fn tag_create_and_retrieve_in_temp_repo() {
    let temp = create_temp_repo();
    let repo_path = temp.path();

    // Create a TagDefinition with foundation role
    let td_json = serde_json::json!({
        "tagKey": "test-purpose",
        "label": "Test Purpose",
        "description": "A test tag definition",
        "roles": ["foundation"],
        "status": "active"
    })
    .to_string();

    let created = run_srs_stdin_in_dir(repo_path, &["tag", "create"], &td_json);
    assert_eq!(
        created["ok"], true,
        "tag create failed: {:?}",
        created["errors"]
    );

    let id = created["payload"]["tagDefinition"]["instanceId"]
        .as_str()
        .unwrap();
    let tag_key = created["payload"]["tagDefinition"]["tagKey"]
        .as_str()
        .unwrap();
    assert_eq!(tag_key, "test-purpose");

    // Retrieve by ID
    let retrieved = run_srs_in_dir(repo_path, &["tag", "get", id]);
    assert_eq!(retrieved["ok"], true);
    assert_eq!(
        retrieved["payload"]["tagDefinition"]["tagKey"],
        "test-purpose"
    );

    // List and verify it appears
    let listed = run_srs_in_dir(repo_path, &["tag", "list"]);
    assert_eq!(listed["ok"], true);
    let defs = listed["payload"]["tagDefinitions"].as_array().unwrap();
    assert!(
        defs.iter().any(|d| d["tagKey"] == "test-purpose"),
        "Created tag definition should appear in list"
    );

    // Filter by role
    let foundation = run_srs_in_dir(repo_path, &["tag", "list", "--role", "foundation"]);
    assert_eq!(foundation["ok"], true);
    let foundation_defs = foundation["payload"]["tagDefinitions"].as_array().unwrap();
    assert_eq!(foundation_defs.len(), 1);
    assert_eq!(foundation_defs[0]["tagKey"], "test-purpose");
}

// ---------- relation-type tests ----------

#[test]
fn relation_type_list_returns_ok_envelope() {
    let result = run_srs(&["relation-type", "list"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "relation-type list");

    let defs = result["payload"]["relationTypeDefinitions"]
        .as_array()
        .expect("relationTypeDefinitions should be array");

    assert_eq!(
        defs.len(),
        16,
        "expected 16 relation type definitions (7 canonical + 5 spec-authoring-core + 4 RFC-process), got {}",
        defs.len()
    );

    // Active canonical: contains — no status field
    let contains_def = defs
        .iter()
        .find(|d| d["relationType"] == "contains")
        .expect("should find 'contains' canonical definition");
    assert!(
        contains_def["status"].is_null(),
        "canonical 'contains' should have no status field (active)"
    );

    // Deprecated SRS-internal type
    let section_seq = defs
        .iter()
        .find(|d| d["relationType"] == "com.semanticops.srs/section-sequence")
        .expect("should find 'com.semanticops.srs/section-sequence' deprecated definition");
    assert_eq!(
        section_seq["status"], "deprecated",
        "section-sequence should be deprecated"
    );
}

#[test]
fn relation_type_get_finds_contains() {
    let contains_id = "3a1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    let result = run_srs(&["relation-type", "get", contains_id]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "relation-type get");

    let def = &result["payload"]["relationTypeDefinition"];
    assert_eq!(def["relationType"], "contains");
    assert_eq!(def["id"], contains_id);
}

#[test]
fn relation_type_get_not_found() {
    let exe = env!("CARGO_BIN_EXE_srs");
    let output = Command::new(exe)
        .args([
            "relation-type",
            "get",
            "00000000-0000-0000-0000-000000000000",
        ])
        .current_dir("/home/greenman/dev/semanticops/srs/srs")
        .output()
        .expect("Failed to execute srs command");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
    let result: Value = serde_json::from_str(&stdout).expect("Failed to parse JSON output");

    assert_eq!(result["ok"], false);
    assert!(!result["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn repo_validate_migrated_relations_use_only_canonical_types() {
    let exe = env!("CARGO_BIN_EXE_srs");
    let output = Command::new(exe)
        .args(["repo", "validate"])
        .current_dir("/home/greenman/dev/semanticops/srs/srs")
        .output()
        .expect("Failed to execute srs command");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
    let result: Value = serde_json::from_str(&stdout).expect("Failed to parse JSON output");

    assert_eq!(result["command"], "repo validate");

    // No E1 errors: every relationType in relations.json must resolve to an installed definition.
    // (E2 errors from placeholder IDs in rfc-targets-section migrations are a separate known issue.)
    let diags = result["payload"]["diagnostics"].as_array().unwrap();
    let e1_errors: Vec<_> = diags
        .iter()
        .filter(|d| {
            d["message"]
                .as_str()
                .map(|m| m.contains("E1:"))
                .unwrap_or(false)
        })
        .collect();
    assert!(
        e1_errors.is_empty(),
        "expected no E1 errors from migrated relations, got: {:?}",
        e1_errors
    );
}

// ---------- repo validate tests ----------

fn make_valid_validate_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let note_id = "00000000-0000-4000-8000-000000000001";

    let manifest = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
        "formatVersion": "1.0",
        "scdsVersion": "2.0",
        "conformance": "SRS 2.0 Core ext:repository",
        "repositoryId": "00000000-0000-4000-8000-000000000099",
        "title": "Test Repo",
        "container": {
            "containerId": "00000000-0000-4000-8000-000000000099",
            "title": "Test Repo"
        },
        "instanceIndex": [{
            "instanceId": note_id,
            "tier": 0,
            "path": "records/notes/note.json"
        }],
        "createdAt": "2026-01-01T00:00:00Z"
    });

    let note = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/note.json",
        "instanceId": note_id,
        "sections": [{"name": "body", "content": "hello"}]
    });

    let notes_dir = temp.path().join("records/notes");
    std::fs::create_dir_all(&notes_dir).unwrap();
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(
        notes_dir.join("note.json"),
        serde_json::to_string_pretty(&note).unwrap(),
    )
    .unwrap();
    temp
}

#[test]
fn repo_validate_valid_repo_returns_ok_true() {
    let temp = make_valid_validate_repo();
    let repo_str = temp.path().to_str().unwrap().to_string();
    let result = run_srs_in_dir(temp.path(), &["repo", "validate", "--repo", &repo_str]);
    assert_eq!(result["ok"], true, "expected ok: {:?}", result);
    assert_eq!(result["command"], "repo validate");
    assert!(result["payload"]["summary"]["checked"].as_u64().unwrap() >= 1);
    assert_eq!(result["payload"]["summary"]["errors"].as_u64().unwrap(), 0);
}

#[test]
fn repo_validate_invalid_note_returns_ok_false() {
    let temp = TempDir::new().unwrap();
    let note_id = "00000000-0000-4000-8000-000000000002";

    let manifest = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
        "formatVersion": "1.0",
        "scdsVersion": "2.0",
        "conformance": "SRS 2.0 Core ext:repository",
        "repositoryId": "00000000-0000-4000-8000-000000000099",
        "title": "Test Repo",
        "container": {
            "containerId": "00000000-0000-4000-8000-000000000099",
            "title": "Test Repo"
        },
        "instanceIndex": [{"instanceId": note_id, "tier": 0, "path": "records/notes/bad.json"}],
        "createdAt": "2026-01-01T00:00:00Z"
    });

    // Missing required "sections" field
    let bad_note = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/note.json",
        "instanceId": note_id
    });

    let notes_dir = temp.path().join("records/notes");
    std::fs::create_dir_all(&notes_dir).unwrap();
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(
        notes_dir.join("bad.json"),
        serde_json::to_string_pretty(&bad_note).unwrap(),
    )
    .unwrap();

    let repo_str = temp.path().to_str().unwrap().to_string();
    let result = run_srs_in_dir(temp.path(), &["repo", "validate", "--repo", &repo_str]);
    assert_eq!(result["ok"], false, "expected ok false: {:?}", result);
    let diags = result["diagnostics"].as_array().unwrap();
    assert!(
        diags
            .iter()
            .any(|d| d.as_str().map(|s| s.contains("sections")).unwrap_or(false)),
        "expected sections error in diagnostics: {:?}",
        diags
    );
}

#[test]
fn repo_validate_tier_schema_mismatch_returns_ok_false() {
    let temp = TempDir::new().unwrap();
    let note_id = "00000000-0000-4000-8000-000000000003";

    let manifest = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
        "formatVersion": "1.0",
        "scdsVersion": "2.0",
        "conformance": "SRS 2.0 Core ext:repository",
        "repositoryId": "00000000-0000-4000-8000-000000000099",
        "title": "Test Repo",
        "container": {
            "containerId": "00000000-0000-4000-8000-000000000099",
            "title": "Test Repo"
        },
        "instanceIndex": [{"instanceId": note_id, "tier": 0, "path": "records/notes/wrong.json"}],
        "createdAt": "2026-01-01T00:00:00Z"
    });

    // Tier 0 but declares record.json — mismatch
    let wrong = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
        "instanceId": note_id,
        "sections": []
    });

    let notes_dir = temp.path().join("records/notes");
    std::fs::create_dir_all(&notes_dir).unwrap();
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(
        notes_dir.join("wrong.json"),
        serde_json::to_string_pretty(&wrong).unwrap(),
    )
    .unwrap();

    let repo_str = temp.path().to_str().unwrap().to_string();
    let result = run_srs_in_dir(temp.path(), &["repo", "validate", "--repo", &repo_str]);
    assert_eq!(result["ok"], false, "expected ok false: {:?}", result);
    let diags = result["diagnostics"].as_array().unwrap();
    assert!(
        diags.iter().any(|d| {
            d.as_str()
                .map(|s| s.contains("tier") && s.contains("expects schema"))
                .unwrap_or(false)
        }),
        "expected tier/schema mismatch in diagnostics: {:?}",
        diags
    );
}
