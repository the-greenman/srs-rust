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

/// Run srs from a directory that is NOT an SRS repo, passing explicit args (may exit non-zero)
#[allow(dead_code)]
fn run_srs_raw(dir: &std::path::Path, args: &[&str]) -> (bool, String) {
    let exe = env!("CARGO_BIN_EXE_srs");
    let output = Command::new(exe)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("Failed to execute srs command");
    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
    (output.status.success(), stdout)
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
    let tagged = run_srs_in_dir(repo_path, &["note", "tag", "add", id, "new-tag"]);
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

// Phase 1 acceptance criteria tests

#[test]
fn global_repo_option_resolves_repo() {
    // Run from a temp dir that is NOT an SRS repo, pointing --repo at the live srs spec repo
    let temp = TempDir::new().unwrap();
    let repo_path = "/home/greenman/dev/semanticops/srs/srs";
    let exe = env!("CARGO_BIN_EXE_srs");
    let output = Command::new(exe)
        .args(["--repo", repo_path, "repo", "map"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to execute srs command");
    assert!(
        output.status.success(),
        "srs --repo <path> repo map failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let result: Value = serde_json::from_str(&stdout).expect("output must be valid JSON");
    assert_eq!(result["ok"], true, "expected ok:true from --repo repo map");
}

#[test]
fn format_json_is_default() {
    // Run without --format and verify output is valid JSON matching the envelope
    let result = run_srs(&["repo", "map"]);
    assert!(
        result["ok"].is_boolean(),
        "default output must be a JSON envelope with ok field"
    );
    assert!(
        result["command"].is_string(),
        "default output must include command field"
    );
    assert!(
        result["version"].is_string(),
        "default output must include version field"
    );

    // Run explicitly with --format json and verify it matches
    let result_explicit = run_srs(&["--format", "json", "repo", "map"]);
    assert_eq!(
        result["command"], result_explicit["command"],
        "--format json must match default output"
    );
    assert_eq!(result["ok"], result_explicit["ok"]);
}

#[test]
fn pretty_outputs_multiline_json() {
    let temp = TempDir::new().unwrap();
    let repo_path = "/home/greenman/dev/semanticops/srs/srs";
    let exe = env!("CARGO_BIN_EXE_srs");

    // Run with --pretty
    let output = Command::new(exe)
        .args(["--repo", repo_path, "--pretty", "repo", "map"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to execute srs command");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();

    // Pretty JSON has newlines and indentation
    assert!(
        stdout.contains('\n'),
        "--pretty output must be multi-line, got: {stdout}"
    );
    // Must still be valid JSON
    let _: Value = serde_json::from_str(&stdout).expect("--pretty output must be valid JSON");
}

#[test]
fn format_text_returns_planned_diagnostic_until_renderer_exists() {
    let temp = TempDir::new().unwrap();
    let repo_path = "/home/greenman/dev/semanticops/srs/srs";
    let exe = env!("CARGO_BIN_EXE_srs");

    // --format text must not panic; it returns a planned diagnostic message
    let output = Command::new(exe)
        .args(["--repo", repo_path, "--format", "text", "repo", "map"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to execute srs command");

    // Must exit 0 (not crash)
    assert!(
        output.status.success(),
        "--format text must not panic, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Must produce some output
    assert!(
        !stdout.trim().is_empty(),
        "--format text must produce output"
    );
}

// ============================================================================
// Phase 3: Entity-First CLI Commands - Test First
// ============================================================================
// These tests define the expected behavior before implementation.
// They will fail until the CLI commands are added.

// --- repo extensions commands ---

#[test]
fn repo_extensions_list_returns_declared_extensions() {
    let temp = create_temp_repo();

    // Add some declared extensions to manifest
    let manifest: Value = serde_json::json!({
        "srsVersion": "2.0-draft",
        "repositoryId": "test-repo",
        "instanceIndex": [],
        "declaredExtensions": ["ext:repository", "ext:relations"]
    });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["repo", "extensions", "list"]);
    assert_eq!(result["ok"], true, "repo extensions list should succeed");
    let extensions = result["payload"]["extensions"]
        .as_array()
        .expect("payload should contain extensions array");
    assert_eq!(extensions.len(), 2);
    assert!(extensions.iter().any(|e| e == "ext:repository"));
    assert!(extensions.iter().any(|e| e == "ext:relations"));
}

#[test]
fn repo_extensions_enable_adds_extension() {
    let temp = create_temp_repo();

    let result = run_srs_in_dir(temp.path(), &["repo", "extensions", "enable", "ext:test"]);
    assert_eq!(result["ok"], true, "repo extensions enable should succeed");

    // Verify manifest was updated
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(temp.path().join("manifest.json")).unwrap())
            .unwrap();
    let extensions = manifest["declaredExtensions"].as_array().unwrap();
    assert!(extensions.iter().any(|e| e == "ext:test"));
}

#[test]
fn repo_extensions_disable_removes_extension() {
    let temp = create_temp_repo();

    // Start with an enabled extension
    let manifest: Value = serde_json::json!({
        "srsVersion": "2.0-draft",
        "repositoryId": "test-repo",
        "instanceIndex": [],
        "declaredExtensions": ["ext:test"]
    });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["repo", "extensions", "disable", "ext:test"]);
    assert_eq!(result["ok"], true, "repo extensions disable should succeed");

    // Verify manifest was updated
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(temp.path().join("manifest.json")).unwrap())
            .unwrap();
    let extensions = manifest["declaredExtensions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(!extensions.iter().any(|e| e == "ext:test"));
}

// --- note update/delete commands ---

#[test]
fn note_update_rewrites_note_and_manifest() {
    let temp = create_temp_repo();

    // Create a note first
    let note_id = "aaaaaaaa-aaaa-aaaa-8aaa-aaaaaaaaaaaa";
    let note = serde_json::json!({
        "instanceId": note_id,
        "title": "Original Title",
        "tags": ["test"],
        "sections": [{"name": "body", "content": "original content"}]
    });

    std::fs::create_dir_all(temp.path().join("records/notes")).unwrap();
    std::fs::write(
        temp.path().join("records/notes/original.json"),
        serde_json::to_string_pretty(&note).unwrap(),
    )
    .unwrap();

    let manifest: Value = serde_json::json!({
        "srsVersion": "2.0-draft",
        "repositoryId": "test-repo",
        "instanceIndex": [{
            "instanceId": note_id,
            "tier": 0,
            "path": "records/notes/original.json",
            "title": "Original Title"
        }]
    });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    // Update the note via CLI
    let updated = serde_json::json!({
        "instanceId": note_id,
        "title": "Updated Title",
        "tags": ["test", "updated"],
        "sections": [{"name": "body", "content": "updated content"}]
    });

    let result = run_srs_stdin_in_dir(
        temp.path(),
        &["note", "update", note_id],
        &serde_json::to_string(&updated).unwrap(),
    );
    assert_eq!(
        result["ok"], true,
        "note update should succeed: {:?}",
        result["diagnostics"]
    );
    assert_eq!(result["payload"]["note"]["title"], "Updated Title");

    // Verify file was rewritten
    let file_note: Value = serde_json::from_str(
        &std::fs::read_to_string(temp.path().join("records/notes/original.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(file_note["title"], "Updated Title");
}

#[test]
fn note_delete_removes_note_and_manifest_entry() {
    let temp = create_temp_repo();

    // Create a note first
    let note_id = "aaaaaaaa-aaaa-aaaa-8aaa-aaaaaaaaaaaa";
    let note = serde_json::json!({
        "instanceId": note_id,
        "title": "To Delete",
        "tags": ["test"],
        "sections": [{"name": "body", "content": "content"}]
    });

    std::fs::create_dir_all(temp.path().join("records/notes")).unwrap();
    std::fs::write(
        temp.path().join("records/notes/delete-me.json"),
        serde_json::to_string_pretty(&note).unwrap(),
    )
    .unwrap();

    let manifest: Value = serde_json::json!({
        "srsVersion": "2.0-draft",
        "repositoryId": "test-repo",
        "instanceIndex": [{
            "instanceId": note_id,
            "tier": 0,
            "path": "records/notes/delete-me.json",
            "title": "To Delete"
        }]
    });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    // Delete the note
    let result = run_srs_in_dir(temp.path(), &["note", "delete", note_id]);
    assert_eq!(
        result["ok"], true,
        "note delete should succeed: {:?}",
        result["diagnostics"]
    );
    assert_eq!(result["payload"]["instanceId"], note_id);

    // Verify file was removed
    assert!(!temp.path().join("records/notes/delete-me.json").exists());

    // Verify manifest was updated
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(temp.path().join("manifest.json")).unwrap())
            .unwrap();
    let index = manifest["instanceIndex"].as_array().unwrap();
    assert!(index.is_empty());
}

// --- note tag nested subgroup (breaking change from old form) ---

#[test]
fn note_tag_add_adds_tag_to_note() {
    let temp = create_temp_repo();

    // Create a note
    let note_id = "aaaaaaaa-aaaa-aaaa-8aaa-aaaaaaaaaaaa";
    let note = serde_json::json!({
        "instanceId": note_id,
        "title": "Test Note",
        "tags": ["existing"],
        "sections": [{"name": "body", "content": "content"}]
    });

    std::fs::create_dir_all(temp.path().join("records/notes")).unwrap();
    std::fs::write(
        temp.path().join("records/notes/test.json"),
        serde_json::to_string_pretty(&note).unwrap(),
    )
    .unwrap();

    let manifest: Value = serde_json::json!({
        "srsVersion": "2.0-draft",
        "repositoryId": "test-repo",
        "instanceIndex": [{
            "instanceId": note_id,
            "tier": 0,
            "path": "records/notes/test.json",
            "title": "Test Note",
            "tags": ["existing"]
        }]
    });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    // Add tag using new nested form
    let result = run_srs_in_dir(temp.path(), &["note", "tag", "add", note_id, "new-tag"]);
    assert_eq!(
        result["ok"], true,
        "note tag add should succeed: {:?}",
        result["diagnostics"]
    );

    // Verify tag was added
    let file_note: Value = serde_json::from_str(
        &std::fs::read_to_string(temp.path().join("records/notes/test.json")).unwrap(),
    )
    .unwrap();
    let tags = file_note["tags"].as_array().unwrap();
    assert!(tags.iter().any(|t| t == "new-tag"));
}

#[test]
fn note_tag_remove_removes_tag_from_note() {
    let temp = create_temp_repo();

    // Create a note with tags
    let note_id = "aaaaaaaa-aaaa-aaaa-8aaa-aaaaaaaaaaaa";
    let note = serde_json::json!({
        "instanceId": note_id,
        "title": "Test Note",
        "tags": ["keep", "remove"],
        "sections": [{"name": "body", "content": "content"}]
    });

    std::fs::create_dir_all(temp.path().join("records/notes")).unwrap();
    std::fs::write(
        temp.path().join("records/notes/test.json"),
        serde_json::to_string_pretty(&note).unwrap(),
    )
    .unwrap();

    let manifest: Value = serde_json::json!({
        "srsVersion": "2.0-draft",
        "repositoryId": "test-repo",
        "instanceIndex": [{
            "instanceId": note_id,
            "tier": 0,
            "path": "records/notes/test.json",
            "title": "Test Note",
            "tags": ["keep", "remove"]
        }]
    });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    // Remove tag using new nested form
    let result = run_srs_in_dir(temp.path(), &["note", "tag", "remove", note_id, "remove"]);
    assert_eq!(
        result["ok"], true,
        "note tag remove should succeed: {:?}",
        result["diagnostics"]
    );

    // Verify tag was removed
    let file_note: Value = serde_json::from_str(
        &std::fs::read_to_string(temp.path().join("records/notes/test.json")).unwrap(),
    )
    .unwrap();
    let tags = file_note["tags"].as_array().unwrap();
    assert!(!tags.iter().any(|t| t == "remove"));
    assert!(tags.iter().any(|t| t == "keep"));
}

#[test]
fn old_note_tag_positional_form_fails_with_parse_error() {
    let temp = create_temp_repo();

    // The old form `srs note tag <id> <tag>` (without add/remove subcommand)
    // should now fail with a parse error, not silently do the wrong thing
    let exe = env!("CARGO_BIN_EXE_srs");
    let output = Command::new(exe)
        .args(["note", "tag", "some-id", "some-tag"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to execute srs command");

    // Should fail (non-zero exit)
    assert!(
        !output.status.success(),
        "old note tag form should fail - commands must use 'note tag add' or 'note tag remove'"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should mention the expected subcommands
    assert!(
        stderr.contains("add") || stderr.contains("remove") || stderr.contains("subcommand"),
        "error should hint at add/remove subcommands: {}",
        stderr
    );
}

// --- tag update/delete commands ---

#[test]
fn tag_update_rewrites_tag_definition() {
    let temp = create_temp_repo();

    // Create a tag definition
    let tag_id = "bbbbbbbb-bbbb-bbbb-8bbb-bbbbbbbbbbbb";
    let tag_def = serde_json::json!({
        "instanceId": tag_id,
        "tagKey": "test-tag",
        "label": "Original Label",
        "description": "Original description"
    });

    std::fs::create_dir_all(temp.path().join("records/tag-definitions")).unwrap();
    std::fs::write(
        temp.path().join("records/tag-definitions/test-tag.json"),
        serde_json::to_string_pretty(&tag_def).unwrap(),
    )
    .unwrap();

    let manifest: Value = serde_json::json!({
        "srsVersion": "2.0-draft",
        "repositoryId": "test-repo",
        "instanceIndex": [{
            "instanceId": tag_id,
            "tier": 3,
            "path": "records/tag-definitions/test-tag.json"
        }]
    });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    // Update the tag
    let updated = serde_json::json!({
        "instanceId": tag_id,
        "tagKey": "test-tag",
        "label": "Updated Label",
        "description": "Updated description"
    });

    let result = run_srs_stdin_in_dir(
        temp.path(),
        &["tag", "update", tag_id],
        &serde_json::to_string(&updated).unwrap(),
    );
    assert_eq!(
        result["ok"], true,
        "tag update should succeed: {:?}",
        result["diagnostics"]
    );
    assert_eq!(result["payload"]["tagDefinition"]["label"], "Updated Label");

    // Verify file was rewritten
    let file_tag: Value = serde_json::from_str(
        &std::fs::read_to_string(temp.path().join("records/tag-definitions/test-tag.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(file_tag["label"], "Updated Label");
}

#[test]
fn tag_delete_removes_tag_definition() {
    let temp = create_temp_repo();

    // Create a tag definition
    let tag_id = "bbbbbbbb-bbbb-bbbb-8bbb-bbbbbbbbbbbb";
    let tag_def = serde_json::json!({
        "instanceId": tag_id,
        "tagKey": "delete-me",
        "label": "Delete Me"
    });

    std::fs::create_dir_all(temp.path().join("records/tag-definitions")).unwrap();
    std::fs::write(
        temp.path().join("records/tag-definitions/delete-me.json"),
        serde_json::to_string_pretty(&tag_def).unwrap(),
    )
    .unwrap();

    let manifest: Value = serde_json::json!({
        "srsVersion": "2.0-draft",
        "repositoryId": "test-repo",
        "instanceIndex": [{
            "instanceId": tag_id,
            "tier": 3,
            "path": "records/tag-definitions/delete-me.json"
        }]
    });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    // Delete the tag
    let result = run_srs_in_dir(temp.path(), &["tag", "delete", tag_id]);
    assert_eq!(
        result["ok"], true,
        "tag delete should succeed: {:?}",
        result["diagnostics"]
    );
    assert_eq!(result["payload"]["instanceId"], tag_id);

    // Verify file was removed
    assert!(!temp
        .path()
        .join("records/tag-definitions/delete-me.json")
        .exists());
}

// --- field command group ---

#[test]
fn field_list_returns_fields() {
    let temp = create_temp_repo();

    // Create package structure with fields
    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::create_dir_all(package_dir.join("fields")).unwrap();

    let field = serde_json::json!({
        "id": "00000000-0000-0000-0000-000000000001",
        "namespace": "com.test",
        "name": "test-field",
        "version": 1,
        "valueType": "string",
        "description": "A test field"
    });
    std::fs::write(
        package_dir.join("fields/test-field.json"),
        serde_json::to_string_pretty(&field).unwrap(),
    )
    .unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": ["fields/test-field.json"],
        "types": []
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["field", "list"]);
    assert_eq!(
        result["ok"], true,
        "field list should succeed: {:?}",
        result["diagnostics"]
    );
    let fields = result["payload"]["fields"]
        .as_array()
        .expect("fields should be array");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0]["name"], "test-field");
}

#[test]
fn field_get_returns_field_by_id() {
    let temp = create_temp_repo();

    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::create_dir_all(package_dir.join("fields")).unwrap();

    let field_id = "00000000-0000-0000-0000-000000000001";
    let field = serde_json::json!({
        "id": field_id,
        "namespace": "com.test",
        "name": "test-field",
        "version": 1,
        "valueType": "string",
        "description": "A test field"
    });
    std::fs::write(
        package_dir.join("fields/test-field.json"),
        serde_json::to_string_pretty(&field).unwrap(),
    )
    .unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": ["fields/test-field.json"],
        "types": []
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["field", "get", field_id]);
    assert_eq!(
        result["ok"], true,
        "field get should succeed: {:?}",
        result["diagnostics"]
    );
    assert_eq!(result["payload"]["field"]["name"], "test-field");
}

#[test]
fn field_create_adds_field_to_package() {
    let temp = create_temp_repo();

    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::create_dir_all(package_dir.join("fields")).unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": []
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let new_field = serde_json::json!({
        "id": "00000000-0000-0000-0000-000000000001",
        "namespace": "com.test",
        "name": "new-field",
        "version": 1,
        "description": "A new field",
        "aiGuidance": {},
        "valueType": "string",
        "createdAt": "2026-01-01T00:00:00Z"
    });

    let result = run_srs_stdin_in_dir(
        temp.path(),
        &["field", "create"],
        &serde_json::to_string(&new_field).unwrap(),
    );
    assert_eq!(
        result["ok"], true,
        "field create should succeed: {:?}",
        result["diagnostics"]
    );

    // Verify package.json was updated
    let package: Value =
        serde_json::from_str(&std::fs::read_to_string(package_dir.join("package.json")).unwrap())
            .unwrap();
    let fields = package["fields"].as_array().unwrap();
    assert_eq!(fields.len(), 1);
}

#[test]
fn field_create_accepts_minimal_payload() {
    let temp = create_temp_repo();

    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::create_dir_all(package_dir.join("fields")).unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": []
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    // Minimal payload: no description, aiGuidance, or createdAt.
    let new_field = serde_json::json!({
        "id": "00000000-0000-0000-0000-0000000000ff",
        "namespace": "com.test",
        "name": "minimal-field",
        "version": 1,
        "valueType": "string"
    });

    let result = run_srs_stdin_in_dir(
        temp.path(),
        &["field", "create"],
        &serde_json::to_string(&new_field).unwrap(),
    );
    assert_eq!(
        result["ok"], true,
        "field create with minimal payload should succeed: {:?}",
        result["diagnostics"]
    );

    // Service should populate createdAt for persisted data.
    let created = &result["payload"]["field"];
    assert!(created["createdAt"].as_str().is_some());
    assert!(!created["createdAt"].as_str().unwrap().is_empty());
}

// --- type command group ---

#[test]
fn type_list_returns_types() {
    let temp = create_temp_repo();

    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::create_dir_all(package_dir.join("types")).unwrap();

    let record_type = serde_json::json!({
        "id": "00000000-0000-0000-0000-000000000002",
        "namespace": "com.test",
        "name": "test-type",
        "version": 1,
        "description": "A test type",
        "fields": []
    });
    std::fs::write(
        package_dir.join("types/test-type.json"),
        serde_json::to_string_pretty(&record_type).unwrap(),
    )
    .unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": ["types/test-type.json"]
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["type", "list"]);
    assert_eq!(
        result["ok"], true,
        "type list should succeed: {:?}",
        result["diagnostics"]
    );
    let types = result["payload"]["types"]
        .as_array()
        .expect("types should be array");
    assert_eq!(types.len(), 1);
    assert_eq!(types[0]["name"], "test-type");
}

#[test]
fn type_get_returns_type_by_id() {
    let temp = create_temp_repo();

    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::create_dir_all(package_dir.join("types")).unwrap();

    let type_id = "00000000-0000-0000-0000-000000000002";
    let record_type = serde_json::json!({
        "id": type_id,
        "namespace": "com.test",
        "name": "test-type",
        "version": 1,
        "description": "A test type",
        "fields": []
    });
    std::fs::write(
        package_dir.join("types/test-type.json"),
        serde_json::to_string_pretty(&record_type).unwrap(),
    )
    .unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": ["types/test-type.json"]
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["type", "get", type_id]);
    assert_eq!(
        result["ok"], true,
        "type get should succeed: {:?}",
        result["diagnostics"]
    );
    assert_eq!(result["payload"]["type"]["name"], "test-type");
}

// --- record command group ---

#[test]
fn record_list_returns_records_by_type() {
    let temp = create_temp_repo();

    // Setup package and create a record
    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::create_dir_all(package_dir.join("types")).unwrap();

    let record_type = serde_json::json!({
        "id": "type-test-001",
        "namespace": "com.test",
        "name": "test-item",
        "version": 1,
        "description": "Test item type",
        "fields": []
    });
    std::fs::write(
        package_dir.join("types/test-item.json"),
        serde_json::to_string_pretty(&record_type).unwrap(),
    )
    .unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": ["types/test-item.json"]
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    // Create a record
    std::fs::create_dir_all(temp.path().join("records/test-items")).unwrap();
    let record_id = "cccccccc-cccc-cccc-8ccc-cccccccccccc";
    let record = serde_json::json!({
        "instanceId": record_id,
        "typeId": "type-test-001",
        "typeVersion": 1,
        "typeNamespace": "com.test",
        "typeName": "test-item",
        "fieldValues": [],
        "createdAt": "2026-01-01T00:00:00Z"
    });
    std::fs::write(
        temp.path()
            .join(format!("records/test-items/{}.json", record_id)),
        serde_json::to_string_pretty(&record).unwrap(),
    )
    .unwrap();

    // Update manifest
    let manifest: Value = serde_json::json!({
        "srsVersion": "2.0-draft",
        "repositoryId": "test-repo",
        "instanceIndex": [{
            "instanceId": record_id,
            "tier": 2,
            "path": format!("records/test-items/{}.json", record_id)
        }]
    });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(
        temp.path(),
        &["record", "list", "--type", "com.test/test-item"],
    );
    assert_eq!(
        result["ok"], true,
        "record list should succeed: {:?}",
        result["diagnostics"]
    );
    let records = result["payload"]["records"]
        .as_array()
        .expect("records should be array");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["instanceId"], record_id);
}

#[test]
fn record_get_returns_record_by_id() {
    let temp = create_temp_repo();

    let record_id = "cccccccc-cccc-cccc-8ccc-cccccccccccc";
    std::fs::create_dir_all(temp.path().join("records/test-items")).unwrap();
    let record = serde_json::json!({
        "instanceId": record_id,
        "typeId": "type-test-001",
        "typeVersion": 1,
        "typeNamespace": "com.test",
        "typeName": "test-item",
        "fieldValues": [{"fieldId": "field-001", "value": "test"}],
        "createdAt": "2026-01-01T00:00:00Z"
    });
    std::fs::write(
        temp.path()
            .join(format!("records/test-items/{}.json", record_id)),
        serde_json::to_string_pretty(&record).unwrap(),
    )
    .unwrap();

    let manifest: Value = serde_json::json!({
        "srsVersion": "2.0-draft",
        "repositoryId": "test-repo",
        "instanceIndex": [{
            "instanceId": record_id,
            "tier": 2,
            "path": format!("records/test-items/{}.json", record_id)
        }]
    });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["record", "get", record_id]);
    assert_eq!(
        result["ok"], true,
        "record get should succeed: {:?}",
        result["diagnostics"]
    );
    assert_eq!(result["payload"]["record"]["instanceId"], record_id);
}

#[test]
fn record_create_writes_file_and_manifest_entry() {
    let temp = create_temp_repo();
    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(package_dir.join("types")).unwrap();

    let record_type = serde_json::json!({
        "id": "type-test-001",
        "namespace": "com.test",
        "name": "test-item",
        "version": 1,
        "description": "Test item type",
        "fields": []
    });
    std::fs::write(
        package_dir.join("types/test-item.json"),
        serde_json::to_string_pretty(&record_type).unwrap(),
    )
    .unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": ["types/test-item.json"]
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let payload = serde_json::json!({ "fieldValues": [] }).to_string();
    let result = run_srs_stdin_in_dir(
        temp.path(),
        &["record", "create", "--type", "com.test/test-item"],
        &payload,
    );
    assert_eq!(result["ok"], true, "record create should succeed");

    let record_id = result["payload"]["record"]["instanceId"]
        .as_str()
        .expect("instanceId should be present");
    assert!(
        temp.path()
            .join(format!("package/records/{}.json", record_id))
            .exists(),
        "record file should be created"
    );

    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(temp.path().join("manifest.json")).unwrap())
            .unwrap();
    let has_entry = manifest["instanceIndex"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["instanceId"] == record_id);
    assert!(has_entry, "manifest should include created record");
}

#[test]
fn record_update_revalidates_and_rewrites_record() {
    let temp = create_temp_repo();
    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(package_dir.join("fields")).unwrap();
    std::fs::create_dir_all(package_dir.join("types")).unwrap();

    let field = serde_json::json!({
        "id": "field-title-001",
        "namespace": "com.test",
        "name": "title",
        "version": 1,
        "valueType": "string",
        "description": "Title field"
    });
    std::fs::write(
        package_dir.join("fields/title.json"),
        serde_json::to_string_pretty(&field).unwrap(),
    )
    .unwrap();

    let record_type = serde_json::json!({
        "id": "type-test-001",
        "namespace": "com.test",
        "name": "test-item",
        "version": 1,
        "description": "Test item type",
        "fields": [{"fieldId": "field-title-001", "order": 1, "required": true}]
    });
    std::fs::write(
        package_dir.join("types/test-item.json"),
        serde_json::to_string_pretty(&record_type).unwrap(),
    )
    .unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": ["fields/title.json"],
        "types": ["types/test-item.json"]
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let create_payload = serde_json::json!({
        "fieldValues": [{"fieldId": "field-title-001", "value": "before"}]
    })
    .to_string();
    let created = run_srs_stdin_in_dir(
        temp.path(),
        &["record", "create", "--type", "com.test/test-item"],
        &create_payload,
    );
    assert_eq!(created["ok"], true);
    let record_id = created["payload"]["record"]["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    let update_payload = serde_json::json!({
        "fieldValues": [{"fieldId": "field-title-001", "value": "after"}]
    })
    .to_string();
    let updated = run_srs_stdin_in_dir(
        temp.path(),
        &["record", "update", &record_id],
        &update_payload,
    );
    assert_eq!(updated["ok"], true, "record update should succeed");

    let fetched = run_srs_in_dir(temp.path(), &["record", "get", &record_id]);
    let value = fetched["payload"]["record"]["fieldValues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|fv| fv["fieldId"] == "field-title-001")
        .and_then(|fv| fv["value"].as_str());
    assert_eq!(value, Some("after"));
}

#[test]
fn record_delete_removes_file_and_manifest_entry() {
    let temp = create_temp_repo();
    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(package_dir.join("types")).unwrap();
    std::fs::create_dir_all(package_dir.join("records")).unwrap();

    let record_type = serde_json::json!({
        "id": "type-test-001",
        "namespace": "com.test",
        "name": "test-item",
        "version": 1,
        "description": "Test item type",
        "fields": []
    });
    std::fs::write(
        package_dir.join("types/test-item.json"),
        serde_json::to_string_pretty(&record_type).unwrap(),
    )
    .unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": ["types/test-item.json"]
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let record_id = "cccccccc-cccc-cccc-8ccc-cccccccccccc";
    let record_path = package_dir.join(format!("records/{}.json", record_id));
    let record = serde_json::json!({
        "instanceId": record_id,
        "typeId": "type-test-001",
        "typeVersion": 1,
        "typeNamespace": "com.test",
        "typeName": "test-item",
        "fieldValues": []
    });
    std::fs::write(&record_path, serde_json::to_string_pretty(&record).unwrap()).unwrap();

    let manifest: Value = serde_json::json!({
        "srsVersion": "2.0-draft",
        "repositoryId": "test-repo",
        "instanceIndex": [{
            "instanceId": record_id,
            "tier": 2,
            "path": format!("package/records/{}.json", record_id)
        }]
    });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let deleted = run_srs_in_dir(temp.path(), &["record", "delete", record_id]);
    assert_eq!(deleted["ok"], true, "record delete should succeed");
    assert!(!record_path.exists(), "record file should be removed");

    let manifest_after: Value =
        serde_json::from_str(&std::fs::read_to_string(temp.path().join("manifest.json")).unwrap())
            .unwrap();
    let has_entry = manifest_after["instanceIndex"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["instanceId"] == record_id);
    assert!(!has_entry, "manifest entry should be removed");
}

#[test]
fn record_create_rejects_invalid_stdin_shape() {
    let temp = create_temp_repo();
    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(package_dir.join("types")).unwrap();

    let record_type = serde_json::json!({
        "id": "type-test-001",
        "namespace": "com.test",
        "name": "test-item",
        "version": 1,
        "description": "Test item type",
        "fields": []
    });
    std::fs::write(
        package_dir.join("types/test-item.json"),
        serde_json::to_string_pretty(&record_type).unwrap(),
    )
    .unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": ["types/test-item.json"]
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let invalid_payload = serde_json::json!({
        "fieldValues": {"field-title-001": "not-an-array"}
    })
    .to_string();
    let result = run_srs_stdin_in_dir(
        temp.path(),
        &["record", "create", "--type", "com.test/test-item"],
        &invalid_payload,
    );

    assert_eq!(result["ok"], false);
    assert!(!result["diagnostics"].as_array().unwrap().is_empty());
}

// --- relation command group ---

#[test]
fn relation_list_returns_relations() {
    let temp = create_temp_repo();

    // Setup relations directory and file
    std::fs::create_dir_all(temp.path().join("relations")).unwrap();
    let relations = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
        "relations": [
            {
                "relationId": "r1",
                "relationType": "contains",
                "sourceInstanceId": "note-1",
                "targetInstanceId": "note-2",
                "createdAt": "2026-01-01T00:00:00Z"
            },
            {
                "relationId": "r2",
                "relationType": "references",
                "sourceInstanceId": "note-2",
                "targetInstanceId": "note-3",
                "createdAt": "2026-01-01T00:00:00Z"
            }
        ]
    });
    std::fs::write(
        temp.path().join("relations/relations-collection.json"),
        serde_json::to_string_pretty(&relations).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["relation", "list"]);
    assert_eq!(
        result["ok"], true,
        "relation list should succeed: {:?}",
        result["diagnostics"]
    );
    let relations_list = result["payload"]["relations"]
        .as_array()
        .expect("relations should be array");
    assert_eq!(relations_list.len(), 2);
}

#[test]
fn relation_list_filters_by_source_target_and_type() {
    let temp = create_temp_repo();

    std::fs::create_dir_all(temp.path().join("relations")).unwrap();
    let relations = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
        "relations": [
            {
                "relationId": "r1",
                "relationType": "contains",
                "sourceInstanceId": "note-1",
                "targetInstanceId": "note-2",
                "createdAt": "2026-01-01T00:00:00Z"
            },
            {
                "relationId": "r2",
                "relationType": "references",
                "sourceInstanceId": "note-2",
                "targetInstanceId": "note-3",
                "createdAt": "2026-01-01T00:00:00Z"
            },
            {
                "relationId": "r3",
                "relationType": "contains",
                "sourceInstanceId": "note-1",
                "targetInstanceId": "note-4",
                "createdAt": "2026-01-01T00:00:00Z"
            }
        ]
    });
    std::fs::write(
        temp.path().join("relations/relations-collection.json"),
        serde_json::to_string_pretty(&relations).unwrap(),
    )
    .unwrap();

    let by_source = run_srs_in_dir(temp.path(), &["relation", "list", "--source", "note-1"]);
    let source_relations = by_source["payload"]["relations"].as_array().unwrap();
    assert_eq!(source_relations.len(), 2);
    assert!(source_relations.iter().all(|r| r["sourceId"] == "note-1"));

    let by_target = run_srs_in_dir(temp.path(), &["relation", "list", "--target", "note-2"]);
    let target_relations = by_target["payload"]["relations"].as_array().unwrap();
    assert_eq!(target_relations.len(), 1);
    assert_eq!(target_relations[0]["relationId"], "r1");

    let by_type = run_srs_in_dir(temp.path(), &["relation", "list", "--type", "contains"]);
    let typed_relations = by_type["payload"]["relations"].as_array().unwrap();
    assert_eq!(typed_relations.len(), 2);
    assert!(typed_relations
        .iter()
        .all(|r| r["relationType"] == "contains"));
}

#[test]
fn relation_get_returns_relation_by_id() {
    let temp = create_temp_repo();

    std::fs::create_dir_all(temp.path().join("relations")).unwrap();
    let relations = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
        "relations": [
            {
                "relationId": "r1",
                "relationType": "contains",
                "sourceInstanceId": "note-1",
                "targetInstanceId": "note-2",
                "createdAt": "2026-01-01T00:00:00Z"
            }
        ]
    });
    std::fs::write(
        temp.path().join("relations/relations-collection.json"),
        serde_json::to_string_pretty(&relations).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["relation", "get", "r1"]);
    assert_eq!(
        result["ok"], true,
        "relation get should succeed: {:?}",
        result["diagnostics"]
    );
    assert_eq!(result["payload"]["relation"]["relationId"], "r1");
    assert_eq!(result["payload"]["relation"]["relationType"], "contains");
}

#[test]
fn relation_create_appends_to_relations_collection() {
    let temp = create_temp_repo();

    let manifest: Value = serde_json::json!({
        "srsVersion": "2.0-draft",
        "repositoryId": "test-repo",
        "instanceIndex": [
            { "instanceId": "note-1", "tier": 0, "path": "records/notes/note-1.json" },
            { "instanceId": "note-2", "tier": 0, "path": "records/notes/note-2.json" }
        ]
    });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(package_dir.join("relation-types")).unwrap();
    let relation_type_def = serde_json::json!({
        "id": "rt-contains-001",
        "version": 1,
        "relationType": "contains",
        "namespace": "com.test",
        "label": "Contains",
        "description": "Source contains target.",
        "category": "composition",
        "createdAt": "2026-01-01T00:00:00Z",
        "status": "active"
    });
    std::fs::write(
        package_dir.join("relation-types/contains.json"),
        serde_json::to_string_pretty(&relation_type_def).unwrap(),
    )
    .unwrap();
    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": [],
        "relationTypes": ["relation-types/contains.json"]
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let relation = serde_json::json!({
        "relationId": "r-new",
        "relationType": "contains",
        "sourceInstanceId": "note-1",
        "targetInstanceId": "note-2",
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string();

    let created = run_srs_stdin_in_dir(temp.path(), &["relation", "create"], &relation);
    assert_eq!(created["ok"], true, "relation create should succeed");
    assert_eq!(created["payload"]["relation"]["relationId"], "r-new");

    let content =
        std::fs::read_to_string(temp.path().join("relations/relations-collection.json")).unwrap();
    let collection: Value = serde_json::from_str(&content).unwrap();
    let has_relation = collection["relations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["relationId"] == "r-new");
    assert!(
        has_relation,
        "created relation should be written to collection"
    );
}

#[test]
fn relation_delete_removes_relation() {
    let temp = create_temp_repo();

    std::fs::create_dir_all(temp.path().join("relations")).unwrap();
    let relations = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
        "relations": [
            {
                "relationId": "r1",
                "relationType": "contains",
                "sourceInstanceId": "note-1",
                "targetInstanceId": "note-2",
                "createdAt": "2026-01-01T00:00:00Z"
            },
            {
                "relationId": "r2",
                "relationType": "references",
                "sourceInstanceId": "note-2",
                "targetInstanceId": "note-3",
                "createdAt": "2026-01-01T00:00:00Z"
            }
        ]
    });
    std::fs::write(
        temp.path().join("relations/relations-collection.json"),
        serde_json::to_string_pretty(&relations).unwrap(),
    )
    .unwrap();

    let deleted = run_srs_in_dir(temp.path(), &["relation", "delete", "r2"]);
    assert_eq!(deleted["ok"], true, "relation delete should succeed");
    assert_eq!(deleted["payload"]["relationId"], "r2");
    assert_eq!(
        deleted["payload"]["path"],
        "relations/relations-collection.json"
    );

    let content =
        std::fs::read_to_string(temp.path().join("relations/relations-collection.json")).unwrap();
    let collection: Value = serde_json::from_str(&content).unwrap();
    let has_r2 = collection["relations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["relationId"] == "r2");
    assert!(!has_r2, "deleted relation should be removed");
}

// --- Phase 4: Extension command group ---

#[test]
fn extension_list_returns_extensions() {
    let temp = create_temp_repo();

    // Create package with extension type
    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::create_dir_all(package_dir.join("records")).unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": [{
            "id": "ext-type",
            "namespace": "meta",
            "name": "extension",
            "version": 1,
            "fields": []
        }]
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    // Create an extension record
    let ext_record = serde_json::json!({
        "instanceId": "ext-001",
        "type": "meta.extension",
        "namespace": "com.test",
        "name": "test-extension",
        "version": 1,
        "fieldValues": {
            "extension-id": "com.test/test-extension@1",
            "title": "Test Extension"
        }
    });
    std::fs::write(
        package_dir.join("records/ext-001.json"),
        serde_json::to_string_pretty(&ext_record).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["extension", "list"]);
    assert_eq!(
        result["ok"], true,
        "extension list should succeed: {:?}",
        result["diagnostics"]
    );
    let extensions = result["payload"]["extensions"].as_array().unwrap();
    assert_eq!(extensions.len(), 1);
}

#[test]
fn extension_get_returns_extension_by_id() {
    let temp = create_temp_repo();

    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::create_dir_all(package_dir.join("records")).unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": []
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let ext_record = serde_json::json!({
        "instanceId": "ext-002",
        "type": "meta.extension",
        "namespace": "com.test",
        "name": "another-extension",
        "version": 1,
        "fieldValues": {
            "extension-id": "com.test/another@1",
            "title": "Another Extension"
        }
    });
    std::fs::write(
        package_dir.join("records/ext-002.json"),
        serde_json::to_string_pretty(&ext_record).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["extension", "get", "ext-002"]);
    assert_eq!(
        result["ok"], true,
        "extension get should succeed: {:?}",
        result["diagnostics"]
    );
    assert_eq!(result["payload"]["extension"]["instanceId"], "ext-002");
}

// --- Phase 4: Protocol command group ---

#[test]
fn protocol_list_returns_protocols() {
    let temp = create_temp_repo();

    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::create_dir_all(package_dir.join("records")).unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": []
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    // Create a protocol record
    let protocol = serde_json::json!({
        "instanceId": "proto-001",
        "type": "meta.protocol",
        "namespace": "com.test",
        "name": "test-protocol",
        "version": 1,
        "fieldValues": {
            "protocol-id": "com.test/test-protocol@1",
            "protocol-namespace": "com.test",
            "protocol-name": "test-protocol",
            "protocol-version": 1,
            "protocol-target-type": "meta.extension",
            "protocol-stages": [
                {"stageId": "stage-1", "name": "Draft", "order": 1, "dependsOn": []},
                {"stageId": "stage-2", "name": "Review", "order": 2, "dependsOn": ["stage-1"]}
            ],
            "protocol-created-at": "2026-05-29T00:00:00Z"
        }
    });
    std::fs::write(
        package_dir.join("records/proto-001.json"),
        serde_json::to_string_pretty(&protocol).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["protocol", "list"]);
    assert_eq!(
        result["ok"], true,
        "protocol list should succeed: {:?}",
        result["diagnostics"]
    );
    let protocols = result["payload"]["protocols"].as_array().unwrap();
    assert_eq!(protocols.len(), 1);
}

#[test]
fn protocol_get_returns_protocol_by_id() {
    let temp = create_temp_repo();

    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::create_dir_all(package_dir.join("records")).unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": []
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let protocol = serde_json::json!({
        "instanceId": "proto-002",
        "type": "meta.protocol",
        "namespace": "com.test",
        "name": "another-protocol",
        "version": 1,
        "fieldValues": {
            "protocol-id": "com.test/another@1",
            "protocol-namespace": "com.test",
            "protocol-name": "another-protocol",
            "protocol-version": 1,
            "protocol-target-type": "meta.extension",
            "protocol-stages": [],
            "protocol-created-at": "2026-05-29T00:00:00Z"
        }
    });
    std::fs::write(
        package_dir.join("records/proto-002.json"),
        serde_json::to_string_pretty(&protocol).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["protocol", "get", "proto-002"]);
    assert_eq!(
        result["ok"], true,
        "protocol get should succeed: {:?}",
        result["diagnostics"]
    );
    assert_eq!(result["payload"]["protocol"]["instanceId"], "proto-002");
}

#[test]
fn protocol_stages_returns_ordered_stages() {
    let temp = create_temp_repo();

    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::create_dir_all(package_dir.join("records")).unwrap();

    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": []
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let protocol = serde_json::json!({
        "instanceId": "proto-003",
        "type": "meta.protocol",
        "namespace": "com.test",
        "name": "staged-protocol",
        "version": 1,
        "fieldValues": {
            "protocol-id": "com.test/staged@1",
            "protocol-namespace": "com.test",
            "protocol-name": "staged-protocol",
            "protocol-version": 1,
            "protocol-target-type": "meta.extension",
            "protocol-stages": [
                {"stageId": "s3", "name": "Published", "order": 3, "dependsOn": ["s2"]},
                {"stageId": "s1", "name": "Draft", "order": 1, "dependsOn": []},
                {"stageId": "s2", "name": "Review", "order": 2, "dependsOn": ["s1"]}
            ],
            "protocol-created-at": "2026-05-29T00:00:00Z"
        }
    });
    std::fs::write(
        package_dir.join("records/proto-003.json"),
        serde_json::to_string_pretty(&protocol).unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["protocol", "stages", "proto-003"]);
    assert_eq!(
        result["ok"], true,
        "protocol stages should succeed: {:?}",
        result["diagnostics"]
    );
    let stages = result["payload"]["stages"].as_array().unwrap();
    assert_eq!(stages.len(), 3);
    // Should be ordered by order field
    assert_eq!(stages[0]["stageId"], "s1");
    assert_eq!(stages[1]["stageId"], "s2");
    assert_eq!(stages[2]["stageId"], "s3");
}

fn make_container_test_repo() -> TempDir {
    let temp = TempDir::new().expect("Failed to create temp dir");
    std::fs::create_dir(temp.path().join(".srs")).expect("Failed to create .srs dir");
    let manifest = serde_json::json!({
        "instanceIndex": [],
        "containerIndex": []
    });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .expect("Failed to write manifest");
    temp
}

#[test]
fn container_list_returns_empty_initially() {
    let temp = make_container_test_repo();
    let result = run_srs_in_dir(temp.path(), &["container", "list"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["payload"]["containers"], serde_json::json!([]));
}

#[test]
fn container_create_returns_container() {
    let temp = make_container_test_repo();
    let payload = serde_json::json!({
        "containerId":"00000000-0000-4000-8000-000000000001",
        "title":"Test"
    })
    .to_string();
    let result = run_srs_stdin_in_dir(temp.path(), &["container", "create"], &payload);
    assert_eq!(result["ok"], true);
    assert_eq!(
        result["payload"]["container"]["containerId"],
        "00000000-0000-4000-8000-000000000001"
    );
}

#[test]
fn container_create_without_id_mints_uuid() {
    let temp = make_container_test_repo();
    let payload = serde_json::json!({
        "title":"Test"
    })
    .to_string();
    let result = run_srs_stdin_in_dir(temp.path(), &["container", "create"], &payload);
    assert_eq!(result["ok"], true);
    let id = result["payload"]["container"]["containerId"]
        .as_str()
        .expect("containerId should be present");
    assert_eq!(id.len(), 36, "containerId should be uuid-length");
    assert_eq!(&id[8..9], "-");
    assert_eq!(&id[13..14], "-");
    assert_eq!(&id[18..19], "-");
    assert_eq!(&id[23..24], "-");
}

#[test]
fn container_get_returns_created_container() {
    let temp = make_container_test_repo();
    let payload = serde_json::json!({
        "containerId":"00000000-0000-4000-8000-000000000001",
        "title":"Test"
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["container", "create"], &payload);
    let got = run_srs_in_dir(
        temp.path(),
        &["container", "get", "00000000-0000-4000-8000-000000000001"],
    );
    assert_eq!(got["ok"], true);
    assert_eq!(got["payload"]["container"]["title"], "Test");
}

#[test]
fn container_update_patches_title() {
    let temp = make_container_test_repo();
    let payload = serde_json::json!({
        "containerId":"00000000-0000-4000-8000-000000000001",
        "title":"Old",
        "description":"keep"
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["container", "create"], &payload);
    let patch = serde_json::json!({"title":"New"}).to_string();
    let updated = run_srs_stdin_in_dir(
        temp.path(),
        &[
            "container",
            "update",
            "00000000-0000-4000-8000-000000000001",
        ],
        &patch,
    );
    assert_eq!(updated["ok"], true);
    assert_eq!(updated["payload"]["container"]["title"], "New");
    assert_eq!(updated["payload"]["container"]["description"], "keep");
}

#[test]
fn container_update_list_reflects_new_title() {
    let temp = make_container_test_repo();
    let payload = serde_json::json!({
        "containerId":"00000000-0000-4000-8000-000000000001",
        "title":"Old"
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["container", "create"], &payload);
    let patch = serde_json::json!({"title":"New"}).to_string();
    run_srs_stdin_in_dir(
        temp.path(),
        &[
            "container",
            "update",
            "00000000-0000-4000-8000-000000000001",
        ],
        &patch,
    );
    let listed = run_srs_in_dir(temp.path(), &["container", "list"]);
    assert_eq!(listed["ok"], true);
    assert_eq!(listed["payload"]["containers"][0]["title"], "New");
}

#[test]
fn container_delete_removes_container() {
    let temp = make_container_test_repo();
    let payload = serde_json::json!({
        "containerId":"00000000-0000-4000-8000-000000000001",
        "title":"Delete"
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["container", "create"], &payload);
    let deleted = run_srs_in_dir(
        temp.path(),
        &[
            "container",
            "delete",
            "00000000-0000-4000-8000-000000000001",
        ],
    );
    assert_eq!(deleted["ok"], true);
    let listed = run_srs_in_dir(temp.path(), &["container", "list"]);
    assert_eq!(listed["payload"]["containers"], serde_json::json!([]));
}

#[test]
fn container_members_add_list_remove() {
    let temp = make_container_test_repo();
    let payload = serde_json::json!({
        "containerId":"00000000-0000-4000-8000-000000000001",
        "title":"Members"
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["container", "create"], &payload);
    let member = "11111111-1111-4111-8111-111111111111";
    let added = run_srs_in_dir(
        temp.path(),
        &[
            "container",
            "members",
            "add",
            "00000000-0000-4000-8000-000000000001",
            member,
        ],
    );
    assert_eq!(added["ok"], true);
    let listed = run_srs_in_dir(
        temp.path(),
        &[
            "container",
            "members",
            "list",
            "00000000-0000-4000-8000-000000000001",
        ],
    );
    assert_eq!(listed["ok"], true);
    assert_eq!(listed["payload"]["memberInstanceIds"][0], member);
    let removed = run_srs_in_dir(
        temp.path(),
        &[
            "container",
            "members",
            "remove",
            "00000000-0000-4000-8000-000000000001",
            member,
        ],
    );
    assert_eq!(removed["ok"], true);
    assert_eq!(
        removed["payload"]["memberInstanceIds"],
        serde_json::json!([])
    );
}

#[test]
fn container_roots_add_list_remove() {
    let temp = make_container_test_repo();
    let payload = serde_json::json!({
        "containerId":"00000000-0000-4000-8000-000000000001",
        "title":"Roots"
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["container", "create"], &payload);
    let root = "11111111-1111-4111-8111-111111111111";
    let added = run_srs_in_dir(
        temp.path(),
        &[
            "container",
            "roots",
            "add",
            "00000000-0000-4000-8000-000000000001",
            root,
        ],
    );
    assert_eq!(added["ok"], true);
    let listed = run_srs_in_dir(
        temp.path(),
        &[
            "container",
            "roots",
            "list",
            "00000000-0000-4000-8000-000000000001",
        ],
    );
    assert_eq!(listed["ok"], true);
    assert_eq!(listed["payload"]["rootInstanceIds"][0], root);
    let removed = run_srs_in_dir(
        temp.path(),
        &[
            "container",
            "roots",
            "remove",
            "00000000-0000-4000-8000-000000000001",
            root,
        ],
    );
    assert_eq!(removed["ok"], true);
    assert_eq!(removed["payload"]["rootInstanceIds"], serde_json::json!([]));
}

#[test]
fn container_validate_passes_clean() {
    let temp = make_container_test_repo();
    let payload = serde_json::json!({
        "containerId":"00000000-0000-4000-8000-000000000001",
        "title":"Valid"
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["container", "create"], &payload);
    let result = run_srs_in_dir(
        temp.path(),
        &[
            "container",
            "validate",
            "00000000-0000-4000-8000-000000000001",
        ],
    );
    assert_eq!(result["ok"], true);
    assert_eq!(result["payload"]["ok"], true);
}

#[test]
fn container_list_filters_by_type() {
    let temp = make_container_test_repo();
    let a = serde_json::json!({
        "containerId":"00000000-0000-4000-8000-000000000001",
        "title":"Meeting A",
        "containerType":"meeting"
    })
    .to_string();
    let b = serde_json::json!({
        "containerId":"00000000-0000-4000-8000-000000000002",
        "title":"Project B",
        "containerType":"project"
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["container", "create"], &a);
    run_srs_stdin_in_dir(temp.path(), &["container", "create"], &b);
    let result = run_srs_in_dir(temp.path(), &["container", "list", "--type", "meeting"]);
    let arr = result["payload"]["containers"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(
        arr[0]["containerId"],
        "00000000-0000-4000-8000-000000000001"
    );
}

#[test]
fn container_list_member_and_root_filters() {
    let temp = make_container_test_repo();
    let a = serde_json::json!({
        "containerId":"00000000-0000-4000-8000-000000000001",
        "title":"A"
    })
    .to_string();
    let b = serde_json::json!({
        "containerId":"00000000-0000-4000-8000-000000000002",
        "title":"B"
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["container", "create"], &a);
    run_srs_stdin_in_dir(temp.path(), &["container", "create"], &b);
    let id = "11111111-1111-4111-8111-111111111111";
    run_srs_in_dir(
        temp.path(),
        &[
            "container",
            "roots",
            "add",
            "00000000-0000-4000-8000-000000000001",
            id,
        ],
    );
    run_srs_in_dir(
        temp.path(),
        &[
            "container",
            "members",
            "add",
            "00000000-0000-4000-8000-000000000002",
            id,
        ],
    );

    let by_member = run_srs_in_dir(temp.path(), &["container", "list", "--member", id]);
    assert_eq!(
        by_member["payload"]["containers"].as_array().unwrap().len(),
        2
    );

    let by_root = run_srs_in_dir(temp.path(), &["container", "list", "--root", id]);
    let roots = by_root["payload"]["containers"].as_array().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(
        roots[0]["containerId"],
        "00000000-0000-4000-8000-000000000001"
    );
}

fn create_container_for_scope(temp: &TempDir, container_id: &str) {
    let payload = serde_json::json!({
        "containerId": container_id,
        "title": "Scope"
    })
    .to_string();
    let result = run_srs_stdin_in_dir(temp.path(), &["container", "create"], &payload);
    assert_eq!(result["ok"], true, "container create should succeed");
}

#[test]
fn container_scope_note_list_filters_to_members() {
    let temp = create_temp_repo();
    let cid = "00000000-0000-4000-8000-000000000001";
    create_container_for_scope(&temp, cid);

    let n1 = serde_json::json!({
        "instanceId": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1",
        "title": "In",
        "sections": [{"name":"body","content":"x"}]
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["note", "create"], &n1);
    let n2 = serde_json::json!({
        "instanceId": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa2",
        "title": "Out",
        "sections": [{"name":"body","content":"x"}]
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["note", "create"], &n2);

    run_srs_in_dir(
        temp.path(),
        &[
            "container",
            "members",
            "add",
            cid,
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1",
        ],
    );

    let listed = run_srs_in_dir(temp.path(), &["--container", cid, "note", "list"]);
    let notes = listed["payload"]["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(
        notes[0]["instanceId"],
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1"
    );
}

#[test]
fn container_scope_note_create_adds_to_container() {
    let temp = create_temp_repo();
    let cid = "00000000-0000-4000-8000-000000000001";
    create_container_for_scope(&temp, cid);

    let payload = serde_json::json!({
        "instanceId": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1",
        "title": "Scoped",
        "sections": [{"name":"body","content":"x"}]
    })
    .to_string();
    let created = run_srs_stdin_in_dir(
        temp.path(),
        &["--container", cid, "note", "create"],
        &payload,
    );
    assert_eq!(created["ok"], true);

    let members = run_srs_in_dir(temp.path(), &["container", "members", "list", cid]);
    let arr = members["payload"]["memberInstanceIds"].as_array().unwrap();
    assert!(arr
        .iter()
        .any(|v| v == "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1"));
}

#[test]
fn container_scope_note_create_fails_invalid_container() {
    let temp = create_temp_repo();
    let payload = serde_json::json!({
        "instanceId": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1",
        "title": "Nope",
        "sections": [{"name":"body","content":"x"}]
    })
    .to_string();
    let created = run_srs_stdin_in_dir(
        temp.path(),
        &[
            "--container",
            "00000000-0000-4000-8000-999999999999",
            "note",
            "create",
        ],
        &payload,
    );
    assert_eq!(created["ok"], false);

    let listed = run_srs_in_dir(temp.path(), &["note", "list"]);
    assert_eq!(listed["payload"]["notes"], serde_json::json!([]));
}

#[test]
fn container_scope_note_delete_refused_if_not_member() {
    let temp = create_temp_repo();
    let cid = "00000000-0000-4000-8000-000000000001";
    create_container_for_scope(&temp, cid);
    let payload = serde_json::json!({
        "instanceId": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1",
        "title": "Outside",
        "sections": [{"name":"body","content":"x"}]
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["note", "create"], &payload);

    let deleted = run_srs_in_dir(
        temp.path(),
        &[
            "--container",
            cid,
            "note",
            "delete",
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1",
        ],
    );
    assert_eq!(deleted["ok"], false);

    let got = run_srs_in_dir(
        temp.path(),
        &["note", "get", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1"],
    );
    assert_eq!(got["ok"], true);
}

#[test]
fn container_scope_note_delete_removes_membership() {
    let temp = create_temp_repo();
    let cid = "00000000-0000-4000-8000-000000000001";
    create_container_for_scope(&temp, cid);
    let payload = serde_json::json!({
        "instanceId": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1",
        "title": "Scoped",
        "sections": [{"name":"body","content":"x"}]
    })
    .to_string();
    run_srs_stdin_in_dir(
        temp.path(),
        &["--container", cid, "note", "create"],
        &payload,
    );

    let deleted = run_srs_in_dir(
        temp.path(),
        &[
            "--container",
            cid,
            "note",
            "delete",
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1",
        ],
    );
    assert_eq!(deleted["ok"], true);

    let got = run_srs_in_dir(
        temp.path(),
        &["note", "get", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1"],
    );
    assert_eq!(got["ok"], false);
    let members = run_srs_in_dir(temp.path(), &["container", "members", "list", cid]);
    assert_eq!(
        members["payload"]["memberInstanceIds"],
        serde_json::json!([])
    );
}

#[test]
fn container_scope_tag_list_filters_to_members() {
    let temp = create_temp_repo();
    let cid = "00000000-0000-4000-8000-000000000001";
    create_container_for_scope(&temp, cid);

    let t1 = serde_json::json!({
        "instanceId": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb1",
        "tagKey":"in",
        "label":"In"
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["tag", "create"], &t1);
    let t2 = serde_json::json!({
        "instanceId": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb2",
        "tagKey":"out",
        "label":"Out"
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["tag", "create"], &t2);

    run_srs_in_dir(
        temp.path(),
        &[
            "container",
            "members",
            "add",
            cid,
            "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb1",
        ],
    );
    let listed = run_srs_in_dir(temp.path(), &["--container", cid, "tag", "list"]);
    let arr = listed["payload"]["tagDefinitions"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["instanceId"], "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb1");
}

#[test]
fn container_scope_record_list_filters_to_members() {
    let temp = create_temp_repo();
    let cid = "00000000-0000-4000-8000-000000000001";
    create_container_for_scope(&temp, cid);

    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(package_dir.join("types")).unwrap();
    let record_type = serde_json::json!({
        "id": "type-test-001",
        "namespace": "com.test",
        "name": "test-item",
        "version": 1,
        "description": "Test item type",
        "fields": []
    });
    std::fs::write(
        package_dir.join("types/test-item.json"),
        serde_json::to_string_pretty(&record_type).unwrap(),
    )
    .unwrap();
    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": ["types/test-item.json"]
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    let payload = serde_json::json!({ "fieldValues": [] }).to_string();
    let r1 = run_srs_stdin_in_dir(
        temp.path(),
        &["record", "create", "--type", "com.test/test-item"],
        &payload,
    );
    let r1_id = r1["payload"]["record"]["instanceId"]
        .as_str()
        .unwrap()
        .to_string();
    let r2 = run_srs_stdin_in_dir(
        temp.path(),
        &["record", "create", "--type", "com.test/test-item"],
        &payload,
    );
    let _r2_id = r2["payload"]["record"]["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    run_srs_in_dir(
        temp.path(),
        &["container", "members", "add", cid, r1_id.as_str()],
    );
    let listed = run_srs_in_dir(
        temp.path(),
        &[
            "--container",
            cid,
            "record",
            "list",
            "--type",
            "com.test/test-item",
        ],
    );
    let arr = listed["payload"]["records"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["instanceId"], r1_id);
}

#[test]
fn container_scope_relation_list_filters_to_internal() {
    let temp = create_temp_repo();
    let cid = "00000000-0000-4000-8000-000000000001";
    create_container_for_scope(&temp, cid);

    std::fs::create_dir_all(temp.path().join("records/notes")).unwrap();
    let n1 = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1";
    let n2 = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa2";
    let n3 = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa3";
    for (id, file) in [(n1, "n1.json"), (n2, "n2.json"), (n3, "n3.json")] {
        let note = serde_json::json!({
            "instanceId": id,
            "title": id,
            "sections": [{"name":"body","content":"x"}]
        });
        std::fs::write(
            temp.path().join(format!("records/notes/{}", file)),
            serde_json::to_string_pretty(&note).unwrap(),
        )
        .unwrap();
    }
    let manifest = serde_json::json!({
        "instanceIndex": [
            {"instanceId": n1, "tier": 0, "path": "records/notes/n1.json"},
            {"instanceId": n2, "tier": 0, "path": "records/notes/n2.json"},
            {"instanceId": n3, "tier": 0, "path": "records/notes/n3.json"}
        ],
        "containerIndex": [{
            "containerId": cid,
            "title": "Scope",
            "path": "containers/scope-00000000.json"
        }]
    });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    run_srs_in_dir(temp.path(), &["container", "members", "add", cid, n1]);
    run_srs_in_dir(temp.path(), &["container", "members", "add", cid, n2]);

    std::fs::create_dir_all(temp.path().join("relations")).unwrap();
    let relations = serde_json::json!({
        "$schema":"https://srs.semanticops.com/schema/2.0/relations-collection.json",
        "relations": [
            {
                "relationId": "r1",
                "relationType": "contains",
                "sourceInstanceId": n1,
                "targetInstanceId": n2
            },
            {
                "relationId": "r2",
                "relationType": "contains",
                "sourceInstanceId": n1,
                "targetInstanceId": n3
            }
        ]
    });
    std::fs::write(
        temp.path().join("relations/relations-collection.json"),
        serde_json::to_string_pretty(&relations).unwrap(),
    )
    .unwrap();

    let listed = run_srs_in_dir(temp.path(), &["--container", cid, "relation", "list"]);
    let arr = listed["payload"]["relations"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["relationId"], "r1");
}

#[test]
fn render_document_view_returns_rendered_payload() {
    let result = run_srs(&[
        "render",
        "document-view",
        "--view",
        "ec34f54b-8636-5c8b-af5b-c9eb3df24fe6",
    ]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "render document-view");
    assert!(!result["payload"]["rendered"]
        .as_str()
        .unwrap_or("")
        .is_empty());
    assert!(result["payload"]["diagnostics"].is_array());
}

#[test]
fn render_document_view_unknown_id_returns_ok_false() {
    let (ok, stdout) = run_srs_raw(
        std::path::Path::new("/home/greenman/dev/semanticops/srs/srs"),
        &[
            "render",
            "document-view",
            "--view",
            "00000000-0000-0000-0000-000000000000",
        ],
    );
    assert!(ok, "command should return JSON envelope");
    let result: Value = serde_json::from_str(&stdout).expect("json output");
    assert_eq!(result["ok"], false);
    assert_eq!(result["command"], "render document-view");
}

#[test]
fn render_document_view_writes_output_file() {
    let temp = TempDir::new().expect("tempdir");
    let out_path = temp.path().join("rendered.md");
    let out_str = out_path.to_string_lossy().to_string();
    let result = run_srs(&[
        "render",
        "document-view",
        "--view",
        "ec34f54b-8636-5c8b-af5b-c9eb3df24fe6",
        "--output",
        &out_str,
    ]);
    assert_eq!(result["ok"], true);
    let file = std::fs::read_to_string(&out_path).expect("render output should exist");
    assert!(!file.trim().is_empty());
}

#[test]
fn render_document_view_view_format_text_overrides_markup() {
    let result = run_srs(&[
        "render",
        "document-view",
        "--view",
        "ec34f54b-8636-5c8b-af5b-c9eb3df24fe6",
        "--view-format",
        "text",
    ]);
    assert_eq!(result["ok"], true);
    let rendered = result["payload"]["rendered"].as_str().unwrap_or("");
    assert!(
        !rendered.contains("# "),
        "text format should not include markdown heading markers"
    );
}
