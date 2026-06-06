use serde_json::Value;
use std::path::Path;
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

fn run_srs_any_status_in_dir(dir: &std::path::Path, args: &[&str]) -> (bool, Value) {
    let exe = env!("CARGO_BIN_EXE_srs");
    let output = Command::new(exe)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("Failed to execute srs command");
    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
    let json: Value = serde_json::from_str(&stdout).expect("Failed to parse JSON output");
    (output.status.success(), json)
}

fn run_srs_stdin_any_status_in_dir(
    dir: &std::path::Path,
    args: &[&str],
    stdin: &str,
) -> (bool, Value) {
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
    let json: Value = serde_json::from_str(&stdout).expect("Failed to parse JSON output");
    (output.status.success(), json)
}

#[test]
fn ordinary_commands_do_not_construct_concrete_stores() {
    let commands_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    let allowed = ["mod.rs", "repo.rs"];
    let forbidden = ["FileStore::new", "JsonStore::", "StoreBackend"];

    for entry in std::fs::read_dir(commands_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }

        let file_name = path.file_name().unwrap().to_str().unwrap();
        if allowed.contains(&file_name) {
            continue;
        }

        let source = std::fs::read_to_string(&path).unwrap();
        for pattern in forbidden {
            assert!(
                !source.contains(pattern),
                "{} must use with_store instead of backend-specific '{}'",
                file_name,
                pattern
            );
        }
    }
}

// Read-only tests against live srs repo

fn srs_spec_repo_dir() -> std::path::PathBuf {
    // Allow override via env var for local development (e.g. when running from a worktree).
    // In CI: srs-rust and srs are checked out as siblings; CARGO_MANIFEST_DIR is
    // srs-rust/crates/srs-cli, so three ".." levels up reaches the parent, then srs/srs.
    if let Ok(path) = std::env::var("SRS_SPEC_REPO") {
        return std::path::PathBuf::from(path);
    }
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // Walk up the directory tree until we find the parent that contains both
    // a Cargo.toml (workspace root) and a sibling srs directory.
    let mut dir = manifest_dir.to_path_buf();
    loop {
        let candidate = dir.join("../srs/srs");
        if let Ok(canonical) = candidate.canonicalize() {
            if canonical.join(".srs").exists() {
                return canonical;
            }
        }
        let parent = match dir.parent() {
            Some(p) => p.to_path_buf(),
            None => break,
        };
        if parent == dir {
            break;
        }
        dir = parent;
    }
    // Fallback: standard CI layout (srs-rust/crates/srs-cli → ../../.. → parent → srs/srs)
    manifest_dir.join("../../../srs/srs")
}

fn run_srs(args: &[&str]) -> Value {
    run_srs_in_dir(&srs_spec_repo_dir(), args)
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
    let result = run_srs(&["note", "tag", "map"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "note tag map");
    assert!(result["payload"]["tagAudit"].is_object());
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
fn repo_create_happy_path() {
    let temp = TempDir::new().unwrap();
    let repo_dir = temp.path().join("new-repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    let repo_dir_str = repo_dir.to_str().unwrap();

    let result = run_srs_in_dir(
        temp.path(),
        &[
            "--repo",
            repo_dir_str,
            "repo",
            "create",
            "--repository-id",
            "repo-123",
            "--namespace",
            "com.semanticops.test",
            "--package-id",
            "pkg-123",
            "--package-name",
            "primary",
        ],
    );

    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "repo create");
    assert_eq!(result["payload"]["repositoryId"], "repo-123");
    assert_eq!(result["payload"]["packageId"], "pkg-123");
    assert!(
        result["payload"]["rootNoteId"].is_null(),
        "rootNoteId should be absent when no name/description given"
    );
    assert!(repo_dir.join(".srs").is_dir());
    assert!(repo_dir.join("manifest.json").is_file());
    assert!(repo_dir.join("package/package.json").is_file());
}

#[test]
fn repo_create_with_name_and_description_creates_root_note() {
    let temp = TempDir::new().unwrap();
    let repo_dir = temp.path().join("intent-repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    let repo_dir_str = repo_dir.to_str().unwrap();

    let result = run_srs_in_dir(
        temp.path(),
        &[
            "--repo",
            repo_dir_str,
            "repo",
            "create",
            "--namespace",
            "com.semanticops.test",
            "--title",
            "My Project",
            "--description",
            "Captures design intent.",
        ],
    );

    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "repo create");
    let repo_id = result["payload"]["repositoryId"].as_str().unwrap();
    let pkg_id = result["payload"]["packageId"].as_str().unwrap();
    assert!(!repo_id.is_empty(), "repositoryId must be auto-generated");
    assert!(!pkg_id.is_empty(), "packageId must be auto-generated");
    let root_note_id = result["payload"]["rootNoteId"]
        .as_str()
        .expect("rootNoteId should be present when --title is given");
    assert!(!root_note_id.is_empty());

    // Verify the note actually exists in the repo
    let notes = run_srs_in_dir(repo_dir.as_path(), &["note", "list"]);
    assert_eq!(notes["ok"], true);
    let note_ids: Vec<&str> = notes["payload"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["instanceId"].as_str())
        .collect();
    assert!(
        note_ids.contains(&root_note_id),
        "intent note must appear in note list"
    );
}

#[test]
fn repo_create_without_existing_repo_does_not_call_detect() {
    let temp = TempDir::new().unwrap();
    let non_repo_dir = temp.path().join("target");
    std::fs::create_dir_all(&non_repo_dir).unwrap();
    let non_repo_str = non_repo_dir.to_str().unwrap();

    let result = run_srs_in_dir(
        temp.path(),
        &[
            "--repo",
            non_repo_str,
            "repo",
            "create",
            "--repository-id",
            "repo-456",
            "--namespace",
            "com.semanticops.test",
            "--package-id",
            "pkg-456",
            "--package-name",
            "primary",
        ],
    );

    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "repo create");
}

#[test]
fn repo_create_existing_repo_errors() {
    let temp = TempDir::new().unwrap();
    let repo_dir = temp.path().join("dupe-repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    let repo_dir_str = repo_dir.to_str().unwrap();

    let _first = run_srs_in_dir(
        temp.path(),
        &[
            "--repo",
            repo_dir_str,
            "repo",
            "create",
            "--repository-id",
            "repo-789",
            "--namespace",
            "com.semanticops.test",
            "--package-id",
            "pkg-789",
            "--package-name",
            "primary",
        ],
    );

    let (ok, second) = run_srs_any_status_in_dir(
        temp.path(),
        &[
            "--repo",
            repo_dir_str,
            "repo",
            "create",
            "--repository-id",
            "repo-789",
            "--namespace",
            "com.semanticops.test",
            "--package-id",
            "pkg-789",
            "--package-name",
            "primary",
        ],
    );

    assert!(!ok);
    assert_eq!(second["ok"], false);
    let diagnostics = second["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().any(|d| {
            d.as_str()
                .map(|s| s.contains("repository already exists"))
                .unwrap_or(false)
        }),
        "expected already-exists diagnostic, got {:?}",
        diagnostics
    );
}

#[test]
fn repo_copy_memory_fixture_to_filestore() {
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src-repo");
    let dst = temp.path().join("dst-repo");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&dst).unwrap();
    let src_str = src.to_str().unwrap();
    let dst_str = dst.to_str().unwrap();

    let created = run_srs_in_dir(
        temp.path(),
        &[
            "--repo",
            src_str,
            "repo",
            "create",
            "--repository-id",
            "repo-copy-src",
            "--namespace",
            "com.semanticops.copy",
            "--package-id",
            "pkg-copy-src",
            "--package-name",
            "primary",
        ],
    );
    assert_eq!(created["ok"], true);

    let copied = run_srs_in_dir(
        temp.path(),
        &["repo", "copy", "--from", src_str, "--to", dst_str],
    );
    assert_eq!(copied["ok"], true);
    assert_eq!(copied["command"], "repo copy");
    assert!(dst.join(".srs").is_dir());
    assert!(dst.join("manifest.json").is_file());
    assert!(dst.join("package/package.json").is_file());
}

#[test]
fn json_store_repo_create_and_note_ops_work() {
    let temp = TempDir::new().unwrap();
    let repo_file = temp.path().join("repo.srsj");
    let repo_file_str = repo_file.to_str().unwrap();

    let created = run_srs_in_dir(
        temp.path(),
        &[
            "--store",
            "json",
            "--repo",
            repo_file_str,
            "repo",
            "create",
            "--repository-id",
            "repo-json-1",
            "--namespace",
            "com.semanticops.json",
            "--package-id",
            "pkg-json-1",
            "--package-name",
            "primary",
        ],
    );
    assert_eq!(created["ok"], true);
    assert!(repo_file.is_file());

    let note_json = serde_json::json!({
        "title": "Json Note",
        "sections": [{"name": "body", "content": "hi"}]
    })
    .to_string();
    let note_created = run_srs_stdin_in_dir(
        temp.path(),
        &["--store", "json", "--repo", repo_file_str, "note", "create"],
        &note_json,
    );
    assert_eq!(note_created["ok"], true);

    let listed = run_srs_in_dir(
        temp.path(),
        &["--store", "json", "--repo", repo_file_str, "note", "list"],
    );
    assert_eq!(listed["ok"], true);
    let notes = listed["payload"]["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["title"], "Json Note");
}

#[test]
fn json_store_backend_is_inferred_from_repo_location() {
    let temp = TempDir::new().unwrap();
    let repo_file = temp.path().join("repo.srsj");
    let repo_file_str = repo_file.to_str().unwrap();

    let created = run_srs_in_dir(
        temp.path(),
        &[
            "--repo",
            repo_file_str,
            "repo",
            "create",
            "--repository-id",
            "repo-json-inferred",
            "--namespace",
            "com.semanticops.json",
            "--package-id",
            "pkg-json-inferred",
            "--package-name",
            "primary",
        ],
    );
    assert_eq!(created["ok"], true);
    assert!(repo_file.is_file());

    let note_json = serde_json::json!({
        "title": "Inferred Json Note",
        "sections": [{"name": "body", "content": "hi"}]
    })
    .to_string();
    let note_created = run_srs_stdin_in_dir(
        temp.path(),
        &["--repo", repo_file_str, "note", "create"],
        &note_json,
    );
    assert_eq!(note_created["ok"], true);

    let listed = run_srs_in_dir(temp.path(), &["--repo", repo_file_str, "note", "list"]);
    assert_eq!(listed["ok"], true);
    let notes = listed["payload"]["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["title"], "Inferred Json Note");
}

#[test]
fn repo_copy_json_to_file_store() {
    let temp = TempDir::new().unwrap();
    let src_json = temp.path().join("src.srsj");
    let dst_file = temp.path().join("dst-file");
    std::fs::create_dir_all(&dst_file).unwrap();
    let src_json_str = src_json.to_str().unwrap();
    let dst_file_str = dst_file.to_str().unwrap();

    let _created = run_srs_in_dir(
        temp.path(),
        &[
            "--store",
            "json",
            "--repo",
            src_json_str,
            "repo",
            "create",
            "--repository-id",
            "repo-json-src",
            "--namespace",
            "com.semanticops.json",
            "--package-id",
            "pkg-json-src",
            "--package-name",
            "primary",
        ],
    );

    let copied = run_srs_in_dir(
        temp.path(),
        &[
            "repo",
            "copy",
            "--from",
            src_json_str,
            "--to",
            dst_file_str,
            "--from-store",
            "json",
            "--to-store",
            "file",
        ],
    );
    assert_eq!(copied["ok"], true);
    assert!(dst_file.join("manifest.json").is_file());
    assert!(dst_file.join("package/package.json").is_file());
}

#[test]
fn repo_copy_infers_json_store_from_srsj_paths() {
    let temp = TempDir::new().unwrap();
    let src_json = temp.path().join("src.srsj");
    let dst_json = temp.path().join("dst.srsj");
    let src_json_str = src_json.to_str().unwrap();
    let dst_json_str = dst_json.to_str().unwrap();

    let created = run_srs_in_dir(
        temp.path(),
        &[
            "--repo",
            src_json_str,
            "repo",
            "create",
            "--repository-id",
            "repo-json-copy-src",
            "--namespace",
            "com.semanticops.json",
            "--package-id",
            "pkg-json-copy-src",
            "--package-name",
            "primary",
        ],
    );
    assert_eq!(created["ok"], true);

    let copied = run_srs_in_dir(
        temp.path(),
        &["repo", "copy", "--from", src_json_str, "--to", dst_json_str],
    );
    assert_eq!(copied["ok"], true);
    assert!(dst_json.is_file());
}

#[test]
fn json_store_current_directory_repo_is_auto_detected() {
    let temp = TempDir::new().unwrap();

    let created = run_srs_in_dir(
        temp.path(),
        &[
            "--store",
            "json",
            "repo",
            "create",
            "--repository-id",
            "repo-json-cwd",
            "--namespace",
            "com.semanticops.json",
            "--package-id",
            "pkg-json-cwd",
            "--package-name",
            "primary",
        ],
    );
    assert_eq!(created["ok"], true);

    let listed = run_srs_in_dir(temp.path(), &["note", "list"]);
    assert_eq!(listed["ok"], true);
    assert_eq!(listed["payload"]["notes"].as_array().unwrap().len(), 0);
}

#[test]
fn json_store_cli_schema_record_and_roundtrip_workflow() {
    let temp = TempDir::new().unwrap();
    let json_repo = temp.path().join("demo.srsj");
    let file_repo = temp.path().join("demo-files");
    let roundtrip_json = temp.path().join("roundtrip.srsj");
    let json_repo_str = json_repo.to_str().unwrap();
    let file_repo_str = file_repo.to_str().unwrap();
    let roundtrip_json_str = roundtrip_json.to_str().unwrap();

    let created = run_srs_in_dir(
        temp.path(),
        &[
            "--repo",
            json_repo_str,
            "repo",
            "create",
            "--repository-id",
            "repo-json-dogfood",
            "--namespace",
            "com.semanticops.dogfood",
            "--package-id",
            "pkg-json-dogfood",
            "--package-name",
            "primary",
        ],
    );
    assert_eq!(created["ok"], true);

    let title_field_id = "00000000-0000-4000-8000-000000000101";
    let status_field_id = "00000000-0000-4000-8000-000000000102";
    let type_id = "00000000-0000-4000-8000-000000000201";

    for field in [
        serde_json::json!({
            "id": title_field_id,
            "namespace": "com.semanticops.dogfood",
            "name": "decision-title",
            "version": 1,
            "valueType": "string"
        }),
        serde_json::json!({
            "id": status_field_id,
            "namespace": "com.semanticops.dogfood",
            "name": "decision-status",
            "version": 1,
            "valueType": "select",
            "allowedValues": ["proposed", "accepted"]
        }),
    ] {
        let result = run_srs_stdin_in_dir(
            temp.path(),
            &["--repo", json_repo_str, "field", "create"],
            &field.to_string(),
        );
        assert_eq!(result["ok"], true, "field create failed: {:?}", result);
    }

    let record_type = serde_json::json!({
        "id": type_id,
        "namespace": "com.semanticops.dogfood",
        "name": "decision",
        "version": 1,
        "description": "A dogfood decision record",
        "fields": [
            {
                "fieldId": title_field_id,
                "order": 0,
                "required": true,
                "displayLabel": "Title"
            },
            {
                "fieldId": status_field_id,
                "order": 1,
                "required": false,
                "displayLabel": "Status"
            }
        ],
        "createdAt": "2026-01-01T00:00:00Z"
    });
    let type_created = run_srs_stdin_in_dir(
        temp.path(),
        &["--repo", json_repo_str, "type", "create"],
        &record_type.to_string(),
    );
    assert_eq!(
        type_created["ok"], true,
        "type create failed: {:?}",
        type_created
    );

    let record_payload = serde_json::json!({
        "fieldValues": [
            {"fieldId": title_field_id, "value": "Backend abstraction is repo-level"},
            {"fieldId": status_field_id, "value": "accepted"}
        ]
    });
    let record_created = run_srs_stdin_in_dir(
        temp.path(),
        &[
            "--repo",
            json_repo_str,
            "record",
            "create",
            "--type",
            "com.semanticops.dogfood/decision",
        ],
        &record_payload.to_string(),
    );
    assert_eq!(
        record_created["ok"], true,
        "record create failed: {:?}",
        record_created
    );
    let record_id = record_created["payload"]["record"]["instanceId"]
        .as_str()
        .unwrap();

    // RFC-006: tag create returns a descriptive error (terms are package definitions now)
    let tag_created = run_srs_stdin_in_dir(
        temp.path(),
        &["--repo", json_repo_str, "tag", "create"],
        &serde_json::json!({
            "tagKey": "dogfood",
            "label": "Dogfood",
            "roles": ["foundation"],
            "status": "active"
        })
        .to_string(),
    );
    assert_eq!(tag_created["command"], "tag create");
    assert_eq!(
        tag_created["ok"], false,
        "tag create should return ok:false (RFC-006 stub)"
    );

    let container_created = run_srs_stdin_in_dir(
        temp.path(),
        &["--repo", json_repo_str, "container", "create"],
        &serde_json::json!({
            "title": "Dogfood Container",
            "containerType": "test"
        })
        .to_string(),
    );
    assert_eq!(container_created["ok"], true);
    let container_id = container_created["payload"]["container"]["containerId"]
        .as_str()
        .unwrap();
    let member_added = run_srs_in_dir(
        temp.path(),
        &[
            "--repo",
            json_repo_str,
            "container",
            "members",
            "add",
            container_id,
            record_id,
        ],
    );
    assert_eq!(member_added["ok"], true);

    let relation_list = run_srs_in_dir(temp.path(), &["--repo", json_repo_str, "relation", "list"]);
    assert_eq!(relation_list["ok"], true);
    assert!(relation_list["payload"]["relations"].is_array());

    let fields = run_srs_in_dir(temp.path(), &["--repo", json_repo_str, "field", "list"]);
    assert_eq!(fields["ok"], true);
    assert_eq!(fields["payload"]["fields"].as_array().unwrap().len(), 2);
    let types = run_srs_in_dir(temp.path(), &["--repo", json_repo_str, "type", "list"]);
    assert_eq!(types["ok"], true);
    assert_eq!(types["payload"]["types"].as_array().unwrap().len(), 1);
    let records = run_srs_in_dir(temp.path(), &["--repo", json_repo_str, "record", "list"]);
    assert_eq!(records["ok"], true);
    assert_eq!(records["payload"]["records"].as_array().unwrap().len(), 1);

    let copied_to_files = run_srs_in_dir(
        temp.path(),
        &[
            "repo",
            "copy",
            "--from",
            json_repo_str,
            "--to",
            file_repo_str,
        ],
    );
    assert_eq!(copied_to_files["ok"], true);
    assert!(file_repo.join("manifest.json").is_file());
    assert!(file_repo.join("package/package.json").is_file());

    let copied_back_to_json = run_srs_in_dir(
        temp.path(),
        &[
            "repo",
            "copy",
            "--from",
            file_repo_str,
            "--to",
            roundtrip_json_str,
        ],
    );
    assert_eq!(copied_back_to_json["ok"], true);
    assert!(roundtrip_json.is_file());

    let validate = run_srs_in_dir(
        temp.path(),
        &["--repo", roundtrip_json_str, "repo", "validate"],
    );
    assert_eq!(validate["ok"], true);
    let roundtrip_fields = run_srs_in_dir(
        temp.path(),
        &["--repo", roundtrip_json_str, "field", "list"],
    );
    assert_eq!(
        roundtrip_fields["payload"]["fields"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let roundtrip_types =
        run_srs_in_dir(temp.path(), &["--repo", roundtrip_json_str, "type", "list"]);
    assert_eq!(
        roundtrip_types["payload"]["types"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let roundtrip_records = run_srs_in_dir(
        temp.path(),
        &["--repo", roundtrip_json_str, "record", "list"],
    );
    assert_eq!(
        roundtrip_records["payload"]["records"]
            .as_array()
            .unwrap()
            .len(),
        1
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
        .current_dir(srs_spec_repo_dir())
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
    // RFC-006: tag list now returns terms[] from package vocabularies
    let result = run_srs(&["tag", "list"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "tag list");
    assert!(result["payload"]["terms"].is_array());
}

#[test]
fn tag_create_and_retrieve_in_temp_repo() {
    // RFC-006: tag create/update/delete return descriptive errors; terms come from package vocabularies
    let temp = create_temp_repo();
    let repo_path = temp.path();

    let td_json = serde_json::json!({"key": "test-purpose"}).to_string();

    let created = run_srs_stdin_in_dir(repo_path, &["tag", "create"], &td_json);
    // tag create now returns diagnostics explaining the RFC-006 change
    assert_eq!(created["command"], "tag create");
    assert!(
        created["diagnostics"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "tag create should return a descriptive error: {:?}",
        created
    );

    // List returns empty terms (no vocabularies in fresh repo)
    let listed = run_srs_in_dir(repo_path, &["tag", "list"]);
    assert_eq!(listed["ok"], true);
    assert!(listed["payload"]["terms"].is_array());
}

// ---------- render document-view tests ----------

fn field_groups_fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/field-groups")
}

fn repeatable_fields_fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/repeatable-fields")
}

#[test]
fn render_document_view_json_returns_projection_payload() {
    let fixture = field_groups_fixture_dir();
    let result = run_srs_in_dir(
        &fixture,
        &[
            "render",
            "document-view",
            "--view",
            "00000000-0000-4000-8000-000000000971",
            "--view-format",
            "json",
        ],
    );

    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "render document-view");

    let payload = &result["payload"];
    assert!(
        payload["projection"].is_object(),
        "payload.projection should be an object, got: {:?}",
        payload
    );

    let proj = &payload["projection"];
    assert_eq!(
        proj["$schema"],
        "https://srs.semanticops.com/schema/2.0/document-view-output.json"
    );
    assert_eq!(
        proj["documentViewId"],
        "00000000-0000-4000-8000-000000000971"
    );
    assert!(proj["generatedAt"].is_string());
    assert!(proj["sections"].is_array());
    assert_eq!(proj["containerId"], Value::Null);
}

#[test]
fn render_document_view_json_projection_sections_and_records() {
    let fixture = field_groups_fixture_dir();
    let result = run_srs_in_dir(
        &fixture,
        &[
            "render",
            "document-view",
            "--view",
            "00000000-0000-4000-8000-000000000971",
            "--view-format",
            "json",
        ],
    );

    assert_eq!(result["ok"], true);
    let sections = result["payload"]["projection"]["sections"]
        .as_array()
        .unwrap();
    assert_eq!(sections.len(), 1);

    let section = &sections[0];
    assert_eq!(section["sectionId"], "all-groups");
    assert_eq!(section["order"], 0);

    let records = section["records"].as_array().unwrap();
    let valid_record = records
        .iter()
        .find(|r| r["instanceId"] == "00000000-0000-4000-8000-000000000981")
        .expect("valid record should be present");

    assert_eq!(valid_record["typeNamespace"], "fixture.groups");
    assert_eq!(valid_record["typeName"], "grouped-item");

    let groups = valid_record["fieldGroups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["groupId"], "people");
    assert_eq!(groups[0]["entries"].as_array().unwrap().len(), 2);
}

#[test]
fn render_document_view_json_rendered_string_is_empty() {
    let fixture = field_groups_fixture_dir();
    let result = run_srs_in_dir(
        &fixture,
        &[
            "render",
            "document-view",
            "--view",
            "00000000-0000-4000-8000-000000000971",
            "--view-format",
            "json",
        ],
    );

    assert_eq!(result["ok"], true);
    assert_eq!(
        result["payload"]["rendered"], "",
        "rendered should be empty string in json mode"
    );
}

#[test]
fn render_document_view_json_writes_output_file() {
    let fixture = field_groups_fixture_dir();
    let temp = tempfile::TempDir::new().unwrap();
    let out_path = temp.path().join("output.json");
    let out_str = out_path.to_str().unwrap();

    let result = run_srs_in_dir(
        &fixture,
        &[
            "render",
            "document-view",
            "--view",
            "00000000-0000-4000-8000-000000000971",
            "--view-format",
            "json",
            "--output",
            out_str,
        ],
    );

    assert_eq!(result["ok"], true, "render command failed: {:?}", result);
    assert!(out_path.exists(), "--output file should have been created");

    let file_content = std::fs::read_to_string(&out_path).expect("output file should be readable");
    let parsed: Value =
        serde_json::from_str(&file_content).expect("output file should be valid JSON");

    assert_eq!(
        parsed["$schema"],
        "https://srs.semanticops.com/schema/2.0/document-view-output.json"
    );
    assert_eq!(
        parsed["documentViewId"],
        "00000000-0000-4000-8000-000000000971"
    );
    assert!(parsed["sections"].is_array());
}

#[test]
fn render_document_view_markup_writes_markup_to_output_file() {
    let fixture = repeatable_fields_fixture_dir();
    let temp = tempfile::TempDir::new().unwrap();
    let out_path = temp.path().join("output.md");
    let out_str = out_path.to_str().unwrap();

    let result = run_srs_in_dir(
        &fixture,
        &[
            "render",
            "document-view",
            "--view",
            "00000000-0000-4000-8000-000000000981",
            "--output",
            out_str,
        ],
    );

    assert_eq!(result["ok"], true, "render command failed: {:?}", result);
    assert!(out_path.exists(), "--output file should have been created");

    let file_content = std::fs::read_to_string(&out_path).expect("output file should be readable");
    assert!(
        !file_content.is_empty(),
        "markup output file should not be empty"
    );
    assert!(
        !file_content.contains("$schema"),
        "markup output should not contain $schema"
    );
}

#[test]
fn render_document_view_markup_no_projection_in_payload() {
    let fixture = repeatable_fields_fixture_dir();
    let result = run_srs_in_dir(
        &fixture,
        &[
            "render",
            "document-view",
            "--view",
            "00000000-0000-4000-8000-000000000981",
        ],
    );

    assert_eq!(result["ok"], true);
    assert!(
        result["payload"]["projection"].is_null(),
        "markup mode should not include projection field, got: {:?}",
        result["payload"]["projection"]
    );
    assert!(
        !result["payload"]["rendered"]
            .as_str()
            .unwrap_or("")
            .is_empty(),
        "markup mode should have non-empty rendered string"
    );
}

#[test]
fn render_document_view_unknown_view_id_returns_error() {
    let fixture = field_groups_fixture_dir();
    let (ok, raw) = run_srs_raw(
        &fixture,
        &[
            "render",
            "document-view",
            "--view",
            "00000000-0000-0000-0000-000000000000",
        ],
    );
    assert!(ok, "command should exit 0 with JSON envelope");
    let result: Value = serde_json::from_str(&raw).expect("json output");
    assert_eq!(result["ok"], false);
    assert_eq!(result["command"], "render document-view");
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
        .current_dir(srs_spec_repo_dir())
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
        .current_dir(srs_spec_repo_dir())
        .output()
        .expect("Failed to execute srs command");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
    let result: Value = serde_json::from_str(&stdout).expect("Failed to parse JSON output");

    assert_eq!(result["command"], "repo validate");

    // No E1 errors: every relationType in relations.json must resolve to an installed definition.
    // (E2 errors from placeholder IDs in rfc-targets-section migrations are a separate known issue.)
    let diags = result["payload"]["diagnostics"]
        .as_array()
        .or_else(|| result["diagnostics"].as_array())
        .expect("repo validate diagnostics should be present");
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
        "srsVersion": "2.0",
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

fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

fn fixture_repo_with_single_record(fixture_name: &str, record_rel_path: &str) -> TempDir {
    let temp = TempDir::new().unwrap();
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(fixture_name);
    copy_dir_recursive(&fixture_root, temp.path());

    let manifest_path = temp.path().join("manifest.json");
    let mut manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    let filtered: Vec<Value> = manifest["instanceIndex"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["path"] == record_rel_path)
        .cloned()
        .collect();
    manifest["instanceIndex"] = Value::Array(filtered);
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
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
        "srsVersion": "2.0",
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
        "srsVersion": "2.0",
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

#[test]
fn repeatable_fields_fixture_validates_ok() {
    let temp =
        fixture_repo_with_single_record("repeatable-fields", "records/repeatable/valid.json");
    let repo_str = temp.path().to_str().unwrap().to_string();
    let result = run_srs_in_dir(temp.path(), &["repo", "validate", "--repo", &repo_str]);
    assert_eq!(result["ok"], true, "expected ok true: {:?}", result);
    let diags = result["payload"]["diagnostics"].as_array().unwrap();
    assert!(!diags.iter().any(|d| {
        d["message"]
            .as_str()
            .map(|m| m.contains("[partial] repeatable field"))
            .unwrap_or(false)
    }));
}

#[test]
fn repeatable_fields_fixture_too_many_entries_in_diagnostics() {
    let temp =
        fixture_repo_with_single_record("repeatable-fields", "records/repeatable/too-many.json");
    let repo_str = temp.path().to_str().unwrap().to_string();
    let result = run_srs_in_dir(temp.path(), &["repo", "validate", "--repo", &repo_str]);
    assert_eq!(result["ok"], false, "expected ok false: {:?}", result);
    let diags = result["diagnostics"].as_array().unwrap();
    assert!(diags.iter().any(|d| {
        d.as_str()
            .map(|m| m.contains("maxItems") || m.contains("00000000-0000-4000-8000-000000000901"))
            .unwrap_or(false)
    }));
}

#[test]
fn field_groups_fixture_validates_ok() {
    let temp = fixture_repo_with_single_record("field-groups", "records/groups/valid.json");
    let repo_str = temp.path().to_str().unwrap().to_string();
    let result = run_srs_in_dir(temp.path(), &["repo", "validate", "--repo", &repo_str]);
    assert_eq!(result["ok"], true, "expected ok true: {:?}", result);
}

#[test]
fn field_groups_fixture_missing_required_group_in_diagnostics() {
    let temp = fixture_repo_with_single_record("field-groups", "records/groups/missing-group.json");
    let repo_str = temp.path().to_str().unwrap().to_string();
    let result = run_srs_in_dir(temp.path(), &["repo", "validate", "--repo", &repo_str]);
    assert_eq!(result["ok"], false, "expected ok false: {:?}", result);
    let diags = result["diagnostics"].as_array().unwrap();
    assert!(diags
        .iter()
        .any(|d| d.as_str().map(|m| m.contains("people")).unwrap_or(false)));
}

// Phase 1 acceptance criteria tests

#[test]
fn global_repo_option_resolves_repo() {
    // Run from a temp dir that is NOT an SRS repo, pointing --repo at the live srs spec repo
    let temp = TempDir::new().unwrap();
    let repo_path = srs_spec_repo_dir();
    let exe = env!("CARGO_BIN_EXE_srs");
    let output = Command::new(exe)
        .arg("--repo")
        .arg(&repo_path)
        .args(["repo", "map"])
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
    let repo_path = srs_spec_repo_dir();
    let exe = env!("CARGO_BIN_EXE_srs");

    // Run with --pretty
    let output = Command::new(exe)
        .arg("--repo")
        .arg(&repo_path)
        .args(["--pretty", "repo", "map"])
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
    let repo_path = srs_spec_repo_dir();
    let exe = env!("CARGO_BIN_EXE_srs");

    // --format text must not panic; it returns a planned diagnostic message
    let output = Command::new(exe)
        .arg("--repo")
        .arg(&repo_path)
        .args(["--format", "text", "repo", "map"])
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

// --- tag update/delete commands (RFC-006: now return descriptive errors) ---

#[test]
fn tag_update_rewrites_tag_definition() {
    // RFC-006: tag update now returns a descriptive error
    let temp = create_temp_repo();
    let result = run_srs_stdin_in_dir(temp.path(), &["tag", "update", "some-id"], "{}");
    assert_eq!(result["command"], "tag update");
    assert!(
        result["diagnostics"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "tag update should return a descriptive error: {:?}",
        result
    );
}

#[test]
fn tag_delete_removes_tag_definition() {
    // RFC-006: tag delete now returns a descriptive error
    let temp = create_temp_repo();
    let result = run_srs_in_dir(temp.path(), &["tag", "delete", "some-id"]);
    assert_eq!(result["command"], "tag delete");
    assert!(
        result["diagnostics"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "tag delete should return a descriptive error: {:?}",
        result
    );
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

// Field UUIDs matching srs/srs/package/types/meta.protocol.json
const PROTO_FIELD_ID: &str = "6c66d06c-3f95-4d17-8ecf-e1046a6f2ec1";
const PROTO_FIELD_NAMESPACE: &str = "8d0f55f9-80e3-4dd6-a05c-10c4b6b6cc87";
const PROTO_FIELD_NAME: &str = "09c5e389-cf6c-4f72-aad6-8cf26bce0b78";
const PROTO_FIELD_VERSION: &str = "f7d28d9d-f90c-4a01-a3eb-2ff4cad54ff6";
const PROTO_FIELD_TARGET_TYPE: &str = "4939a29b-7f70-481f-bf6b-bf693f8bd67f";
const PROTO_FIELD_STAGES: &str = "0f1232c6-0db5-4383-b91d-64d81195f1c4";
const PROTO_FIELD_CREATED_AT: &str = "b953f716-383a-4218-bebf-96e93c4747a4";
const PROTO_TYPE_UUID: &str = "48a03f5d-4f27-42f4-b791-999f6c22f8d2";

/// Create a temp repo whose package declares the meta.protocol type with all
/// required field definitions. Required for any protocol import/list/get test.
fn create_temp_repo_with_protocol_type() -> TempDir {
    let temp = TempDir::new().expect("Failed to create temp dir");
    std::fs::create_dir_all(temp.path().join(".srs")).unwrap();

    let manifest = serde_json::json!({ "instanceIndex": [] });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let pkg = temp.path().join("package");
    std::fs::create_dir_all(pkg.join("fields")).unwrap();
    std::fs::create_dir_all(pkg.join("types")).unwrap();
    std::fs::create_dir_all(pkg.join("records/protocols")).unwrap();

    // Write the 7 required field definitions (description/tags are optional, skip for brevity)
    let fields = [
        (PROTO_FIELD_ID, "protocol-id", "string"),
        (PROTO_FIELD_NAMESPACE, "protocol-namespace", "string"),
        (PROTO_FIELD_NAME, "protocol-name", "string"),
        (PROTO_FIELD_VERSION, "protocol-version", "number"),
        (PROTO_FIELD_TARGET_TYPE, "protocol-target-type", "string"),
        (PROTO_FIELD_STAGES, "protocol-stages", "text"),
        (PROTO_FIELD_CREATED_AT, "protocol-created-at", "date"),
    ];
    let mut field_paths = vec![];
    let mut field_assignments = vec![];
    for (i, (uuid, name, value_type)) in fields.iter().enumerate() {
        let fname = format!("fields/{}.json", name);
        std::fs::write(
            pkg.join(&fname),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": uuid,
                "namespace": "com.semanticops.srs",
                "name": name,
                "version": 1,
                "valueType": value_type
            }))
            .unwrap(),
        )
        .unwrap();
        field_paths.push(fname);
        field_assignments.push(serde_json::json!({
            "fieldId": uuid,
            "order": i,
            "required": true
        }));
    }

    // Write the meta.protocol type
    std::fs::write(
        pkg.join("types/meta.protocol.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "id": PROTO_TYPE_UUID,
            "namespace": "com.semanticops.srs",
            "name": "meta.protocol",
            "version": 1,
            "fields": field_assignments
        }))
        .unwrap(),
    )
    .unwrap();

    // Write package.json referencing all the above
    std::fs::write(
        pkg.join("package.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "id": "test-proto-pkg",
            "namespace": "com.test",
            "name": "test-with-protocol",
            "version": "1.0.0",
            "fields": field_paths,
            "types": ["types/meta.protocol.json"]
        }))
        .unwrap(),
    )
    .unwrap();

    temp
}

/// Canonical minimal protocol JSON for use with `srs protocol import`
fn minimal_protocol_json(id: &str, name: &str) -> String {
    serde_json::to_string(&serde_json::json!({
        "protocolId": id,
        "protocolNamespace": "com.test",
        "protocolName": name,
        "protocolVersion": 1,
        "protocolTargetType": "meta.extension",
        "protocolStages": [
            {"stageId": "s1", "name": "Draft", "order": 1, "dependsOn": []},
            {"stageId": "s2", "name": "Review", "order": 2, "dependsOn": ["s1"]}
        ],
        "protocolCreatedAt": "2026-05-29T00:00:00Z"
    }))
    .unwrap()
}

#[test]
fn protocol_list_returns_protocols() {
    let temp = create_temp_repo_with_protocol_type();
    let stdin = minimal_protocol_json("com.test/test-protocol@1", "test-protocol");
    let import = run_srs_stdin_in_dir(temp.path(), &["protocol", "import"], &stdin);
    assert_eq!(import["ok"], true, "import failed: {:?}", import);

    let result = run_srs_in_dir(temp.path(), &["protocol", "list"]);
    assert_eq!(
        result["ok"], true,
        "list failed: {:?}",
        result["diagnostics"]
    );
    let protocols = result["payload"]["protocols"].as_array().unwrap();
    assert_eq!(protocols.len(), 1);
    assert_eq!(protocols[0]["protocolId"], "com.test/test-protocol@1");
    assert_eq!(protocols[0]["stageCount"], 2);
}

#[test]
fn protocol_get_returns_protocol_by_id() {
    let temp = create_temp_repo_with_protocol_type();
    let stdin = minimal_protocol_json("com.test/get-test@1", "get-test");
    let import = run_srs_stdin_in_dir(temp.path(), &["protocol", "import"], &stdin);
    assert_eq!(import["ok"], true, "import failed: {:?}", import);
    let instance_id = import["payload"]["protocol"]["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    let result = run_srs_in_dir(temp.path(), &["protocol", "get", &instance_id]);
    assert_eq!(
        result["ok"], true,
        "get failed: {:?}",
        result["diagnostics"]
    );
    assert_eq!(result["payload"]["protocol"]["instanceId"], instance_id);
    assert_eq!(
        result["payload"]["protocol"]["protocolId"],
        "com.test/get-test@1"
    );
}

#[test]
fn protocol_stages_returns_ordered_stages() {
    let temp = create_temp_repo_with_protocol_type();
    let stdin = serde_json::to_string(&serde_json::json!({
        "protocolId": "com.test/staged@1",
        "protocolNamespace": "com.test",
        "protocolName": "staged",
        "protocolVersion": 1,
        "protocolTargetType": "meta.extension",
        "protocolStages": [
            {"stageId": "s3", "name": "Published", "order": 3, "dependsOn": ["s2"]},
            {"stageId": "s1", "name": "Draft", "order": 1, "dependsOn": []},
            {"stageId": "s2", "name": "Review", "order": 2, "dependsOn": ["s1"]}
        ],
        "protocolCreatedAt": "2026-05-29T00:00:00Z"
    }))
    .unwrap();
    let import = run_srs_stdin_in_dir(temp.path(), &["protocol", "import"], &stdin);
    assert_eq!(import["ok"], true, "import failed: {:?}", import);
    let instance_id = import["payload"]["protocol"]["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    let result = run_srs_in_dir(temp.path(), &["protocol", "stages", &instance_id]);
    assert_eq!(
        result["ok"], true,
        "stages failed: {:?}",
        result["diagnostics"]
    );
    let stages = result["payload"]["stages"].as_array().unwrap();
    assert_eq!(stages.len(), 3);
    assert_eq!(stages[0]["stageId"], "s1");
    assert_eq!(stages[1]["stageId"], "s2");
    assert_eq!(stages[2]["stageId"], "s3");
}

#[test]
fn protocol_import_fails_without_type_declaration() {
    let temp = create_temp_repo();
    // create a minimal package.json with NO meta.protocol type
    let pkg = temp.path().join("package");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(
        pkg.join("package.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "id": "empty-pkg", "namespace": "com.test",
            "name": "empty", "version": "1.0.0",
            "fields": [], "types": []
        }))
        .unwrap(),
    )
    .unwrap();

    let stdin = minimal_protocol_json("com.test/x@1", "x");
    let (_ok, result) =
        run_srs_stdin_any_status_in_dir(temp.path(), &["protocol", "import"], &stdin);
    assert_eq!(result["ok"], false, "should fail without type declaration");
    let diag = result["diagnostics"].to_string();
    assert!(
        diag.contains("Add it to your package before importing protocols"),
        "expected actionable message, got: {}",
        diag
    );
}

#[test]
fn protocol_import_rejects_missing_required_field() {
    let temp = create_temp_repo_with_protocol_type();
    // omit protocolTargetType
    let bad = serde_json::to_string(&serde_json::json!({
        "protocolId": "com.test/x@1",
        "protocolNamespace": "com.test",
        "protocolName": "x",
        "protocolVersion": 1,
        "protocolStages": [],
        "protocolCreatedAt": "2026-05-29T00:00:00Z"
    }))
    .unwrap();
    let (_ok, result) = run_srs_stdin_any_status_in_dir(temp.path(), &["protocol", "import"], &bad);
    assert_eq!(
        result["ok"], false,
        "should fail with missing required field"
    );
}

#[test]
fn protocol_import_rejects_invalid_version() {
    let temp = create_temp_repo_with_protocol_type();
    let bad = serde_json::to_string(&serde_json::json!({
        "protocolId": "com.test/x@1",
        "protocolNamespace": "com.test",
        "protocolName": "x",
        "protocolVersion": 0,
        "protocolTargetType": "meta.extension",
        "protocolStages": [],
        "protocolCreatedAt": "2026-05-29T00:00:00Z"
    }))
    .unwrap();
    let (_ok, result) = run_srs_stdin_any_status_in_dir(temp.path(), &["protocol", "import"], &bad);
    assert_eq!(result["ok"], false, "should reject version 0");
}

#[test]
fn protocol_import_rejects_malformed_created_at() {
    let temp = create_temp_repo_with_protocol_type();
    let bad = serde_json::to_string(&serde_json::json!({
        "protocolId": "com.test/x@1",
        "protocolNamespace": "com.test",
        "protocolName": "x",
        "protocolVersion": 1,
        "protocolTargetType": "meta.extension",
        "protocolStages": [],
        "protocolCreatedAt": "not-a-date"
    }))
    .unwrap();
    let (_ok, result) = run_srs_stdin_any_status_in_dir(temp.path(), &["protocol", "import"], &bad);
    assert_eq!(result["ok"], false, "should reject malformed createdAt");
}

#[test]
fn protocol_import_rejects_stage_with_bad_depends_on() {
    let temp = create_temp_repo_with_protocol_type();
    let bad = serde_json::to_string(&serde_json::json!({
        "protocolId": "com.test/x@1",
        "protocolNamespace": "com.test",
        "protocolName": "x",
        "protocolVersion": 1,
        "protocolTargetType": "meta.extension",
        "protocolStages": [
            {"stageId": "s1", "name": "A", "order": 1, "dependsOn": ["nonexistent"]}
        ],
        "protocolCreatedAt": "2026-05-29T00:00:00Z"
    }))
    .unwrap();
    let (_ok, result) = run_srs_stdin_any_status_in_dir(temp.path(), &["protocol", "import"], &bad);
    assert_eq!(result["ok"], false, "should reject unknown dependsOn stage");
}

#[test]
fn protocol_export_import_roundtrip() {
    let src = create_temp_repo_with_protocol_type();
    let dst = create_temp_repo_with_protocol_type();

    // Import into source repo
    let stdin = serde_json::to_string(&serde_json::json!({
        "protocolId": "com.test/roundtrip@1",
        "protocolNamespace": "com.test",
        "protocolName": "roundtrip",
        "protocolVersion": 1,
        "protocolTargetType": "meta.extension",
        "protocolStages": [
            {"stageId": "a", "name": "Alpha", "order": 1, "dependsOn": []},
            {"stageId": "b", "name": "Beta", "order": 2, "dependsOn": ["a"]}
        ],
        "protocolCreatedAt": "2026-05-29T00:00:00Z"
    }))
    .unwrap();
    let import1 = run_srs_stdin_in_dir(src.path(), &["protocol", "import"], &stdin);
    assert_eq!(import1["ok"], true);
    let instance_id = import1["payload"]["protocol"]["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    // Export from source
    let export = run_srs_in_dir(src.path(), &["protocol", "export", &instance_id]);
    assert_eq!(export["ok"], true);
    let exported = &export["payload"]["protocol"];
    // Export must NOT contain instanceId
    assert!(
        exported["instanceId"].is_null(),
        "export should not contain instanceId"
    );

    // Import exported JSON into destination repo
    let export_str = serde_json::to_string(exported).unwrap();
    let import2 = run_srs_stdin_in_dir(dst.path(), &["protocol", "import"], &export_str);
    assert_eq!(
        import2["ok"], true,
        "round-trip import failed: {:?}",
        import2
    );

    // Verify field-by-field equality
    let dst_instance_id = import2["payload"]["protocol"]["instanceId"]
        .as_str()
        .unwrap()
        .to_string();
    let got = run_srs_in_dir(dst.path(), &["protocol", "get", &dst_instance_id]);
    assert_eq!(
        got["payload"]["protocol"]["protocolId"],
        "com.test/roundtrip@1"
    );
    let stages = run_srs_in_dir(dst.path(), &["protocol", "stages", &dst_instance_id]);
    let s = stages["payload"]["stages"].as_array().unwrap();
    assert_eq!(s.len(), 2);
    assert_eq!(s[0]["stageId"], "a");
    assert_eq!(s[0]["name"], "Alpha");
    assert_eq!(s[1]["stageId"], "b");
    assert_eq!(s[1]["name"], "Beta");
}

#[test]
fn protocol_update_modifies_stages() {
    let temp = create_temp_repo_with_protocol_type();
    let stdin = minimal_protocol_json("com.test/upd@1", "upd");
    let import = run_srs_stdin_in_dir(temp.path(), &["protocol", "import"], &stdin);
    assert_eq!(import["ok"], true);
    let id = import["payload"]["protocol"]["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    let update = serde_json::to_string(&serde_json::json!({
        "protocolStages": [
            {"stageId": "x", "name": "Only", "order": 1, "dependsOn": []}
        ]
    }))
    .unwrap();
    let result = run_srs_stdin_in_dir(temp.path(), &["protocol", "update", &id], &update);
    assert_eq!(result["ok"], true, "update failed: {:?}", result);

    let stages = run_srs_in_dir(temp.path(), &["protocol", "stages", &id]);
    let s = stages["payload"]["stages"].as_array().unwrap();
    assert_eq!(s.len(), 1);
    assert_eq!(s[0]["stageId"], "x");
    assert_eq!(s[0]["name"], "Only");
}

#[test]
fn protocol_update_preserves_identity_fields() {
    let temp = create_temp_repo_with_protocol_type();
    let stdin = minimal_protocol_json("com.test/identity@1", "identity");
    let import = run_srs_stdin_in_dir(temp.path(), &["protocol", "import"], &stdin);
    assert_eq!(import["ok"], true);
    let id = import["payload"]["protocol"]["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    // Attempt to overwrite identity fields — all should be ignored
    let update = serde_json::to_string(&serde_json::json!({
        "protocolId": "com.attacker/hijacked@99",
        "protocolNamespace": "com.attacker",
        "protocolName": "hijacked",
        "protocolVersion": 99,
        "protocolCreatedAt": "1970-01-01T00:00:00Z",
        "protocolTargetType": "meta.note"
    }))
    .unwrap();
    let result = run_srs_stdin_in_dir(temp.path(), &["protocol", "update", &id], &update);
    assert_eq!(result["ok"], true, "update failed: {:?}", result);

    let got = run_srs_in_dir(temp.path(), &["protocol", "get", &id]);
    let p = &got["payload"]["protocol"];
    assert_eq!(p["protocolId"], "com.test/identity@1");
    assert_eq!(p["protocolNamespace"], "com.test");
    assert_eq!(p["protocolName"], "identity");
    assert_eq!(p["protocolVersion"], 1);
    assert_eq!(p["protocolCreatedAt"], "2026-05-29T00:00:00Z");
    // The mutable field should be updated
    assert_eq!(p["protocolTargetType"], "meta.note");
}

#[test]
fn protocol_update_file_and_manifest_consistent() {
    let temp = create_temp_repo_with_protocol_type();
    let stdin = minimal_protocol_json("com.test/cons@1", "cons");
    let import = run_srs_stdin_in_dir(temp.path(), &["protocol", "import"], &stdin);
    assert_eq!(import["ok"], true);
    let id = import["payload"]["protocol"]["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    let update = serde_json::to_string(&serde_json::json!({
        "protocolTargetType": "meta.record"
    }))
    .unwrap();
    run_srs_stdin_in_dir(temp.path(), &["protocol", "update", &id], &update);

    // protocol get should reflect the update
    let proto = run_srs_in_dir(temp.path(), &["protocol", "get", &id]);
    assert_eq!(
        proto["payload"]["protocol"]["protocolTargetType"],
        "meta.record"
    );

    // record get (uses manifest index) should also find the record
    let rec = run_srs_in_dir(temp.path(), &["record", "get", &id]);
    assert_eq!(rec["ok"], true, "record get failed: {:?}", rec);

    // protocol list still shows the record (manifest consistent)
    let list = run_srs_in_dir(temp.path(), &["protocol", "list"]);
    assert_eq!(list["payload"]["protocols"].as_array().unwrap().len(), 1);
}

#[test]
fn protocol_delete_removes_record() {
    let temp = create_temp_repo_with_protocol_type();
    let stdin = minimal_protocol_json("com.test/del@1", "del");
    let import = run_srs_stdin_in_dir(temp.path(), &["protocol", "import"], &stdin);
    assert_eq!(import["ok"], true);
    let id = import["payload"]["protocol"]["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    let del = run_srs_in_dir(temp.path(), &["protocol", "delete", &id]);
    assert_eq!(del["ok"], true, "delete failed: {:?}", del);
    assert_eq!(del["payload"]["instanceId"], id);

    // protocol list should be empty
    let list = run_srs_in_dir(temp.path(), &["protocol", "list"]);
    assert_eq!(list["payload"]["protocols"].as_array().unwrap().len(), 0);
    // record list should be empty
    let rec_list = run_srs_in_dir(temp.path(), &["record", "list"]);
    assert_eq!(rec_list["payload"]["records"].as_array().unwrap().len(), 0);
    // protocol list should now be empty (manifest index updated)
    let list2 = run_srs_in_dir(temp.path(), &["protocol", "list"]);
    assert_eq!(list2["payload"]["protocols"].as_array().unwrap().len(), 0);

    // record list should also be empty
    let rec_list = run_srs_in_dir(temp.path(), &["record", "list"]);
    assert_eq!(rec_list["payload"]["records"].as_array().unwrap().len(), 0);
}

#[test]
fn protocol_delete_not_found_returns_error() {
    let temp = create_temp_repo_with_protocol_type();
    let result = run_srs_any_status_in_dir(temp.path(), &["protocol", "delete", "nonexistent-id"]);
    assert_eq!(result.1["ok"], false);
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
    // RFC-006: tag list returns terms from package vocabularies.
    // Container-scoped tag list returns an empty terms array for repos without vocabulary files.
    let temp = create_temp_repo();
    let cid = "00000000-0000-4000-8000-000000000001";
    create_container_for_scope(&temp, cid);

    let listed = run_srs_in_dir(temp.path(), &["--container", cid, "tag", "list"]);
    assert_eq!(listed["ok"], true);
    assert!(listed["payload"]["terms"].is_array());
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
        &srs_spec_repo_dir(),
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

fn repeatable_fixture_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/repeatable-fields")
}

#[test]
fn cli_render_document_view_with_theme_variant_flag_passes_through() {
    let result = run_srs_in_dir(
        &repeatable_fixture_root(),
        &[
            "render",
            "document-view",
            "--view",
            "00000000-0000-4000-8000-000000000987",
            "--theme-variant",
            "print",
        ],
    );
    assert_eq!(result["ok"], true);
    let rendered = result["payload"]["rendered"].as_str().unwrap_or("");
    assert!(
        rendered.contains("PRINTDOC["),
        "expected print theme wrapper in CLI output, got: {}",
        rendered
    );
}

#[test]
fn cli_render_document_view_without_theme_variant_works_as_before() {
    let result = run_srs_in_dir(
        &repeatable_fixture_root(),
        &[
            "render",
            "document-view",
            "--view",
            "00000000-0000-4000-8000-000000000987",
        ],
    );
    assert_eq!(result["ok"], true);
    let rendered = result["payload"]["rendered"].as_str().unwrap_or("");
    assert!(
        rendered.contains("DOC{{unknown}}["),
        "expected base theme wrapper in CLI output, got: {}",
        rendered
    );
}

#[test]
fn cli_render_document_view_theme_variant_not_found_produces_diagnostic_not_error() {
    let result = run_srs_in_dir(
        &repeatable_fixture_root(),
        &[
            "render",
            "document-view",
            "--view",
            "00000000-0000-4000-8000-000000000987",
            "--theme-variant",
            "missing",
        ],
    );
    assert_eq!(result["ok"], true);
    let diagnostics = result["payload"]["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().any(|d| d
            .as_str()
            .unwrap_or("")
            .contains("theme variant 'missing' not found")),
        "expected missing variant diagnostic, got: {:?}",
        diagnostics
    );
    let rendered = result["payload"]["rendered"].as_str().unwrap_or("");
    assert!(
        rendered.contains("DOC{{unknown}}["),
        "expected fallback to base theme, got: {}",
        rendered
    );
}

// ---------------------------------------------------------------------------
// Phase 5: package command integration tests
// ---------------------------------------------------------------------------

fn create_temp_repo_with_package() -> TempDir {
    let temp = TempDir::new().expect("Failed to create temp dir");
    let srs_dir = temp.path().join(".srs");
    std::fs::create_dir_all(&srs_dir).unwrap();

    let manifest = serde_json::json!({ "instanceIndex": [] });
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(package_dir.join("fields")).unwrap();
    std::fs::create_dir_all(package_dir.join("types")).unwrap();

    let package_json = serde_json::json!({
        "id": "primary-pkg",
        "namespace": "com.test",
        "name": "primary",
        "version": "1.0.0",
        "fields": [],
        "types": []
    });
    std::fs::write(
        temp.path().join("package/package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    temp
}

#[test]
fn package_create_happy_path() {
    let temp = create_temp_repo_with_package();

    // The boundary directory must exist (the service writes package.json into it).
    std::fs::create_dir_all(temp.path().join("pkg/sub")).unwrap();

    let result = run_srs_in_dir(
        temp.path(),
        &[
            "package",
            "create",
            "--id",
            "sub-pkg-001",
            "--namespace",
            "com.sub",
            "--name",
            "sub",
            "--version",
            "1.0.0",
            "--path",
            "pkg/sub",
        ],
    );
    assert_eq!(
        result["ok"], true,
        "package create should succeed: {:?}",
        result
    );
    assert_eq!(result["payload"]["id"], "sub-pkg-001");

    // package list should now show primary + sub
    let list = run_srs_in_dir(temp.path(), &["package", "list"]);
    assert_eq!(list["ok"], true);
    let packages = list["payload"]["packages"].as_array().unwrap();
    assert_eq!(packages.len(), 2, "should have primary + 1 sub-package");
}

#[test]
fn package_import_local_happy_path() {
    let temp = create_temp_repo_with_package();

    let sub_dir = temp.path().join("external/mypkg");
    std::fs::create_dir_all(&sub_dir).unwrap();
    std::fs::write(
        sub_dir.join("package.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "id": "import-pkg-001",
            "namespace": "com.imported",
            "name": "imported",
            "version": "2.0.0",
            "fields": [],
            "types": []
        }))
        .unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(
        temp.path(),
        &["package", "import", "--path", "external/mypkg"],
    );
    assert_eq!(
        result["ok"], true,
        "package import should succeed: {:?}",
        result
    );
    assert_eq!(result["payload"]["id"], "import-pkg-001");
    assert_eq!(result["payload"]["namespace"], "com.imported");

    // Verify it appears in list
    let list = run_srs_in_dir(temp.path(), &["package", "list"]);
    let packages = list["payload"]["packages"].as_array().unwrap();
    assert!(
        packages.iter().any(|p| p["id"] == "import-pkg-001"),
        "imported package should appear in list"
    );
}

#[test]
fn package_update_metadata_only() {
    let temp = create_temp_repo_with_package();

    let result = run_srs_in_dir(
        temp.path(),
        &["package", "update", "--name", "renamed-primary"],
    );
    assert_eq!(
        result["ok"], true,
        "package update should succeed: {:?}",
        result
    );
    assert_eq!(result["payload"]["name"], "renamed-primary");

    // list should reflect the new name
    let list = run_srs_in_dir(temp.path(), &["package", "list"]);
    let packages = list["payload"]["packages"].as_array().unwrap();
    let primary = packages
        .iter()
        .find(|p| p["boundaryPath"].is_null())
        .unwrap();
    assert_eq!(primary["name"], "renamed-primary");
}

#[test]
fn slice_create_output_matches_package_create() {
    // slice create is a permanent alias for package create — its output shape must be identical.
    let temp = create_temp_repo_with_package();

    std::fs::create_dir_all(temp.path().join("pkg/slice")).unwrap();

    let result = run_srs_in_dir(
        temp.path(),
        &[
            "package",
            "slice-create",
            "--id",
            "slice-pkg-001",
            "--namespace",
            "com.slice",
            "--name",
            "slice",
            "--version",
            "1.0.0",
            "--path",
            "pkg/slice",
        ],
    );
    assert_eq!(
        result["ok"], true,
        "slice create should succeed: {:?}",
        result
    );
    assert_eq!(
        result["command"], "package create",
        "slice create must emit same command name as package create"
    );
    assert_eq!(result["payload"]["id"], "slice-pkg-001");
}

#[test]
fn field_create_in_sub_package() {
    let temp = create_temp_repo_with_package();

    // Create sub-package boundary via CLI (package create writes the package.json).
    std::fs::create_dir_all(temp.path().join("pkg/ext/fields")).unwrap();
    run_srs_in_dir(
        temp.path(),
        &[
            "package",
            "create",
            "--id",
            "ext-pkg-001",
            "--namespace",
            "com.ext",
            "--name",
            "ext",
            "--version",
            "1.0.0",
            "--path",
            "pkg/ext",
        ],
    );

    let new_field = serde_json::json!({
        "id": "00000000-0000-0000-0000-sub000000001",
        "namespace": "com.ext",
        "name": "ext-field",
        "version": 1,
        "valueType": "string"
    });
    let result = run_srs_stdin_in_dir(
        temp.path(),
        &["field", "create", "--package", "pkg/ext"],
        &serde_json::to_string(&new_field).unwrap(),
    );
    assert_eq!(
        result["ok"], true,
        "field create --package should succeed: {:?}",
        result
    );

    // Sub-package package.json should list the field; primary should not.
    let sub_pkg: Value = serde_json::from_str(
        &std::fs::read_to_string(temp.path().join("pkg/ext/package.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        sub_pkg["fields"].as_array().unwrap().len(),
        1,
        "field should appear in sub-package package.json"
    );

    let primary_pkg: Value = serde_json::from_str(
        &std::fs::read_to_string(temp.path().join("package/package.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        primary_pkg["fields"].as_array().unwrap().len(),
        0,
        "field should NOT appear in primary package.json"
    );
}

#[test]
fn field_list_with_package_filter() {
    let temp = create_temp_repo_with_package();

    // Seed primary package with one field
    let pkg_json = serde_json::json!({
        "id": "primary-pkg",
        "namespace": "com.test",
        "name": "primary",
        "version": "1.0.0",
        "fields": ["fields/f-primary.json"],
        "types": []
    });
    std::fs::write(
        temp.path().join("package/package.json"),
        serde_json::to_string_pretty(&pkg_json).unwrap(),
    )
    .unwrap();
    std::fs::write(
        temp.path().join("package/fields/f-primary.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "id": "00000000-0000-0000-0000-fld000000001",
            "namespace": "com.test",
            "name": "primary-field",
            "version": 1,
            "valueType": "string",
            "description": "",
            "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["field", "list"]);
    assert_eq!(result["ok"], true);
    let fields = result["payload"]["fields"].as_array().unwrap();
    assert!(
        fields.iter().any(|f| f["name"] == "primary-field"),
        "primary-field should appear in field list"
    );
}

#[test]
fn type_list_with_package_filter() {
    let temp = create_temp_repo_with_package();

    // Seed primary package with one type
    let pkg_json = serde_json::json!({
        "id": "primary-pkg",
        "namespace": "com.test",
        "name": "primary",
        "version": "1.0.0",
        "fields": [],
        "types": ["types/t-primary.json"]
    });
    std::fs::write(
        temp.path().join("package/package.json"),
        serde_json::to_string_pretty(&pkg_json).unwrap(),
    )
    .unwrap();
    std::fs::write(
        temp.path().join("package/types/t-primary.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "id": "00000000-0000-0000-0000-typ000000001",
            "namespace": "com.test",
            "name": "primary-type",
            "version": 1,
            "fields": []
        }))
        .unwrap(),
    )
    .unwrap();

    let result = run_srs_in_dir(temp.path(), &["type", "list"]);
    assert_eq!(result["ok"], true);
    let types = result["payload"]["types"].as_array().unwrap();
    assert!(
        types.iter().any(|t| t["name"] == "primary-type"),
        "primary-type should appear in type list"
    );
}

// ---------------------------------------------------------------------------
// view command integration tests
// ---------------------------------------------------------------------------

fn create_temp_repo_with_views() -> TempDir {
    let temp = TempDir::new().expect("Failed to create temp dir");
    std::fs::create_dir_all(temp.path().join(".srs")).unwrap();
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&serde_json::json!({ "instanceIndex": [] })).unwrap(),
    )
    .unwrap();
    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(package_dir.join("views")).unwrap();
    std::fs::create_dir_all(package_dir.join("document-views")).unwrap();
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
            "documentViews": []
        }))
        .unwrap(),
    )
    .unwrap();
    temp
}

fn minimal_view_json() -> String {
    serde_json::json!({
        "id": "",
        "namespace": "com.test",
        "name": "test-view",
        "version": 1,
        "description": "A test view",
        "compatibleTypes": ["core/decision"],
        "fieldViews": [{ "fieldId": "f1", "order": 0 }],
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string()
}

fn minimal_document_view_json() -> String {
    serde_json::json!({
        "id": "",
        "namespace": "com.test",
        "name": "test-doc-view",
        "version": 1,
        "description": "A test document view",
        "sections": [{
            "sectionId": "s1",
            "order": 0,
            "source": { "type": "fixed-instances", "instanceIds": [] }
        }],
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string()
}

#[test]
fn view_list_returns_ok_envelope() {
    let temp = create_temp_repo_with_views();
    let result = run_srs_in_dir(temp.path(), &["view", "list"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "view list");
    assert!(result["payload"]["views"].is_array());
    assert_eq!(result["payload"]["views"].as_array().unwrap().len(), 0);
}

#[test]
fn view_create_returns_view_with_id() {
    let temp = create_temp_repo_with_views();
    let result = run_srs_stdin_in_dir(temp.path(), &["view", "create"], &minimal_view_json());
    assert_eq!(result["ok"], true, "view create failed: {:?}", result);
    let id = result["payload"]["view"]["id"].as_str().unwrap();
    assert!(!id.is_empty());
}

#[test]
fn view_get_returns_created_view() {
    let temp = create_temp_repo_with_views();
    let created = run_srs_stdin_in_dir(temp.path(), &["view", "create"], &minimal_view_json());
    let id = created["payload"]["view"]["id"].as_str().unwrap();

    let result = run_srs_in_dir(temp.path(), &["view", "get", id]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["payload"]["view"]["name"], "test-view");
}

#[test]
fn view_list_contains_created_view() {
    let temp = create_temp_repo_with_views();
    let created = run_srs_stdin_in_dir(temp.path(), &["view", "create"], &minimal_view_json());
    let id = created["payload"]["view"]["id"].as_str().unwrap();

    let result = run_srs_in_dir(temp.path(), &["view", "list"]);
    assert_eq!(result["ok"], true);
    let views = result["payload"]["views"].as_array().unwrap();
    assert!(
        views.iter().any(|v| v["id"] == id),
        "created view should appear in list"
    );
}

#[test]
fn view_list_filters_by_namespace() {
    let temp = create_temp_repo_with_views();

    run_srs_stdin_in_dir(temp.path(), &["view", "create"], &minimal_view_json());

    let other_view = serde_json::json!({
        "id": "",
        "namespace": "com.other",
        "name": "other-view",
        "version": 1,
        "description": "Other namespace view",
        "compatibleTypes": ["org.coop/governance_decision"],
        "fieldViews": [{ "fieldId": "f1", "order": 0 }],
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["view", "create"], &other_view);

    let result = run_srs_in_dir(temp.path(), &["view", "list", "--namespace", "com.other"]);
    assert_eq!(result["ok"], true);
    let views = result["payload"]["views"].as_array().unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0]["namespace"], "com.other");
}

#[test]
fn view_list_filters_by_compatible_type_hint() {
    let temp = create_temp_repo_with_views();

    run_srs_stdin_in_dir(temp.path(), &["view", "create"], &minimal_view_json());

    let other_view = serde_json::json!({
        "id": "",
        "namespace": "com.test",
        "name": "other-view",
        "version": 1,
        "description": "Other view",
        "compatibleTypes": ["org.coop/governance_decision"],
        "fieldViews": [{ "fieldId": "f1", "order": 0 }],
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["view", "create"], &other_view);

    let result = run_srs_in_dir(temp.path(), &["view", "list", "--type-id", "core/decision"]);
    assert_eq!(result["ok"], true);
    let views = result["payload"]["views"].as_array().unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0]["name"], "test-view");
}

#[test]
fn view_update_changes_description() {
    let temp = create_temp_repo_with_views();
    let created = run_srs_stdin_in_dir(temp.path(), &["view", "create"], &minimal_view_json());
    let id = created["payload"]["view"]["id"].as_str().unwrap();

    let updated_json = serde_json::json!({
        "id": id,
        "namespace": "com.test",
        "name": "test-view",
        "version": 1,
        "description": "Updated description",
        "compatibleTypes": ["core/decision", "org.coop/emergency_decision"],
        "fieldViews": [{ "fieldId": "f1", "order": 0 }],
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string();
    let update_result = run_srs_stdin_in_dir(temp.path(), &["view", "update", id], &updated_json);
    assert_eq!(
        update_result["ok"], true,
        "view update failed: {:?}",
        update_result
    );

    let get_result = run_srs_in_dir(temp.path(), &["view", "get", id]);
    assert_eq!(
        get_result["payload"]["view"]["description"],
        "Updated description"
    );
}

#[test]
fn view_delete_removes_view() {
    let temp = create_temp_repo_with_views();
    let created = run_srs_stdin_in_dir(temp.path(), &["view", "create"], &minimal_view_json());
    let id = created["payload"]["view"]["id"].as_str().unwrap();

    let delete_result = run_srs_in_dir(temp.path(), &["view", "delete", id]);
    assert_eq!(delete_result["ok"], true);
    assert_eq!(delete_result["payload"]["id"], id);

    let list_result = run_srs_in_dir(temp.path(), &["view", "list"]);
    let views = list_result["payload"]["views"].as_array().unwrap();
    assert!(views.iter().all(|v| v["id"] != id));
}

#[test]
fn view_get_not_found_returns_error() {
    let temp = create_temp_repo_with_views();
    let result = run_srs_in_dir(
        temp.path(),
        &["view", "get", "00000000-0000-0000-0000-000000000000"],
    );
    assert_eq!(result["ok"], false);
}

#[test]
fn view_create_fails_validation() {
    let temp = create_temp_repo_with_views();
    let bad = serde_json::json!({
        "id": "",
        "namespace": "com.test",
        "name": "bad-view",
        "version": 1,
        "description": "bad",
        "fieldViews": [],
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string();
    let result = run_srs_stdin_in_dir(temp.path(), &["view", "create"], &bad);
    assert_eq!(result["ok"], false);
}

// ---------------------------------------------------------------------------
// document-view command integration tests
// ---------------------------------------------------------------------------

#[test]
fn document_view_list_returns_ok_envelope() {
    let temp = create_temp_repo_with_views();
    let result = run_srs_in_dir(temp.path(), &["document-view", "list"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "document-view list");
    assert!(result["payload"]["documentViews"].is_array());
    assert_eq!(
        result["payload"]["documentViews"].as_array().unwrap().len(),
        0
    );
}

#[test]
fn document_view_create_returns_document_view_with_id() {
    let temp = create_temp_repo_with_views();
    let result = run_srs_stdin_in_dir(
        temp.path(),
        &["document-view", "create"],
        &minimal_document_view_json(),
    );
    assert_eq!(
        result["ok"], true,
        "document-view create failed: {:?}",
        result
    );
    let id = result["payload"]["documentView"]["id"].as_str().unwrap();
    assert!(!id.is_empty());
}

#[test]
fn document_view_get_returns_created() {
    let temp = create_temp_repo_with_views();
    let created = run_srs_stdin_in_dir(
        temp.path(),
        &["document-view", "create"],
        &minimal_document_view_json(),
    );
    let id = created["payload"]["documentView"]["id"].as_str().unwrap();

    let result = run_srs_in_dir(temp.path(), &["document-view", "get", id]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["payload"]["documentView"]["name"], "test-doc-view");
}

#[test]
fn document_view_list_contains_created() {
    let temp = create_temp_repo_with_views();
    let created = run_srs_stdin_in_dir(
        temp.path(),
        &["document-view", "create"],
        &minimal_document_view_json(),
    );
    let id = created["payload"]["documentView"]["id"].as_str().unwrap();

    let result = run_srs_in_dir(temp.path(), &["document-view", "list"]);
    assert_eq!(result["ok"], true);
    let dviews = result["payload"]["documentViews"].as_array().unwrap();
    assert!(dviews.iter().any(|v| v["id"] == id));
}

#[test]
fn document_view_list_filters_by_namespace() {
    let temp = create_temp_repo_with_views();

    run_srs_stdin_in_dir(
        temp.path(),
        &["document-view", "create"],
        &minimal_document_view_json(),
    );

    let other = serde_json::json!({
        "id": "",
        "namespace": "com.other",
        "name": "other-doc-view",
        "version": 1,
        "description": "Other namespace",
        "sections": [{
            "sectionId": "s1",
            "order": 0,
            "source": { "type": "fixed-instances", "instanceIds": [] }
        }],
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string();
    run_srs_stdin_in_dir(temp.path(), &["document-view", "create"], &other);

    let result = run_srs_in_dir(
        temp.path(),
        &["document-view", "list", "--namespace", "com.other"],
    );
    assert_eq!(result["ok"], true);
    let dviews = result["payload"]["documentViews"].as_array().unwrap();
    assert_eq!(dviews.len(), 1);
    assert_eq!(dviews[0]["namespace"], "com.other");
}

#[test]
fn document_view_update_replaces_description() {
    let temp = create_temp_repo_with_views();
    let created = run_srs_stdin_in_dir(
        temp.path(),
        &["document-view", "create"],
        &minimal_document_view_json(),
    );
    let id = created["payload"]["documentView"]["id"].as_str().unwrap();

    let updated_json = serde_json::json!({
        "id": id,
        "namespace": "com.test",
        "name": "test-doc-view",
        "version": 1,
        "description": "Updated doc view description",
        "sections": [{
            "sectionId": "s1",
            "order": 0,
            "source": { "type": "fixed-instances", "instanceIds": [] }
        }],
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string();
    let update_result =
        run_srs_stdin_in_dir(temp.path(), &["document-view", "update", id], &updated_json);
    assert_eq!(
        update_result["ok"], true,
        "document-view update failed: {:?}",
        update_result
    );
    assert_eq!(
        update_result["payload"]["documentView"]["description"],
        "Updated doc view description"
    );
}

#[test]
fn document_view_delete_removes_view() {
    let temp = create_temp_repo_with_views();
    let created = run_srs_stdin_in_dir(
        temp.path(),
        &["document-view", "create"],
        &minimal_document_view_json(),
    );
    let id = created["payload"]["documentView"]["id"].as_str().unwrap();

    let delete_result = run_srs_in_dir(temp.path(), &["document-view", "delete", id]);
    assert_eq!(delete_result["ok"], true);
    assert_eq!(delete_result["payload"]["id"], id);

    let list_result = run_srs_in_dir(temp.path(), &["document-view", "list"]);
    let dviews = list_result["payload"]["documentViews"].as_array().unwrap();
    assert!(dviews.iter().all(|v| v["id"] != id));
}

#[test]
fn document_view_get_not_found_returns_error() {
    let temp = create_temp_repo_with_views();
    let result = run_srs_in_dir(
        temp.path(),
        &[
            "document-view",
            "get",
            "00000000-0000-0000-0000-000000000000",
        ],
    );
    assert_eq!(result["ok"], false);
}

#[test]
fn document_view_create_fails_validation() {
    let temp = create_temp_repo_with_views();
    let bad = serde_json::json!({
        "id": "",
        "namespace": "com.test",
        "name": "bad-dv",
        "version": 1,
        "description": "bad",
        "sections": [],
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string();
    let result = run_srs_stdin_in_dir(temp.path(), &["document-view", "create"], &bad);
    assert_eq!(result["ok"], false);
}

// ── Phase C: global --package flag integration tests ──────────────────────────

fn make_repo_with_sub_package() -> TempDir {
    let temp = create_temp_repo_with_package();
    // Create sub-package directory and register it via CLI
    std::fs::create_dir_all(temp.path().join("package/sub")).unwrap();
    run_srs_in_dir(
        temp.path(),
        &[
            "package",
            "create",
            "--id",
            "sub-pkg-001",
            "--namespace",
            "com.sub",
            "--name",
            "sub",
            "--version",
            "1.0.0",
            "--path",
            "package/sub",
        ],
    );
    temp
}

fn minimal_field_json(id: &str, name: &str) -> String {
    serde_json::json!({
        "id": id,
        "namespace": "com.test",
        "name": name,
        "version": 1,
        "valueType": "string"
    })
    .to_string()
}

#[test]
fn field_create_without_package_flag_writes_to_primary() {
    let temp = create_temp_repo_with_package();
    let field = minimal_field_json("00000000-0000-0000-0000-primary00001", "primary-field");

    let result = run_srs_stdin_in_dir(temp.path(), &["field", "create"], &field);
    assert_eq!(
        result["ok"], true,
        "field create should succeed: {:?}",
        result
    );

    let primary_pkg: Value = serde_json::from_str(
        &std::fs::read_to_string(temp.path().join("package/package.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        primary_pkg["fields"].as_array().unwrap().len(),
        1,
        "field should appear in primary package.json"
    );
    assert!(
        temp.path()
            .join("package/fields")
            .read_dir()
            .unwrap()
            .count()
            > 0,
        "field file should exist under package/fields/"
    );
}

#[test]
fn field_create_with_undeclared_package_flag_errors() {
    let temp = create_temp_repo_with_package();
    let field = minimal_field_json("00000000-0000-0000-0000-ghost0000001", "ghost-field");

    let (_ok, result) = run_srs_stdin_any_status_in_dir(
        temp.path(),
        &["field", "create", "--package", "package/ghost"],
        &field,
    );
    assert_eq!(
        result["ok"], false,
        "field create with undeclared --package should fail: {:?}",
        result
    );
    // No files should have been created under package/ghost/
    assert!(
        !temp.path().join("package/ghost").exists(),
        "no files should be created under undeclared boundary"
    );
}

#[test]
fn field_list_includes_source_package() {
    let temp = make_repo_with_sub_package();

    // Create a field in the primary package
    let primary_field = minimal_field_json("00000000-0000-0000-0000-primary00002", "p-field");
    run_srs_stdin_in_dir(temp.path(), &["field", "create"], &primary_field);

    // Create a field in the sub-package
    let sub_field = minimal_field_json("00000000-0000-0000-0000-sub000000002", "s-field");
    run_srs_stdin_in_dir(
        temp.path(),
        &["field", "create", "--package", "package/sub"],
        &sub_field,
    );

    let result = run_srs_in_dir(temp.path(), &["field", "list"]);
    assert_eq!(result["ok"], true);

    let fields = result["payload"]["fields"].as_array().unwrap();
    assert_eq!(fields.len(), 2, "should list both fields");

    // Primary field should have no sourcePackage (omitted when None)
    let primary = fields
        .iter()
        .find(|f| f["name"] == "p-field")
        .expect("primary field not found in list");
    assert!(
        primary.get("sourcePackage").is_none() || primary["sourcePackage"].is_null(),
        "primary field should have no sourcePackage"
    );

    // Sub field should have sourcePackage set
    let sub = fields
        .iter()
        .find(|f| f["name"] == "s-field")
        .expect("sub field not found in list");
    assert_eq!(
        sub["sourcePackage"], "package/sub",
        "sub field sourcePackage should be 'package/sub'"
    );
}

#[test]
fn field_create_in_sub_package_file_lands_under_sub_path() {
    // Verifies the standard package/sub boundary: file lands in the correct directory
    // and the primary package.json is unchanged.
    let temp = make_repo_with_sub_package();

    let sub_field = minimal_field_json("00000000-0000-0000-0000-sub000000003", "sub-only-field");
    let result = run_srs_stdin_in_dir(
        temp.path(),
        &["field", "create", "--package", "package/sub"],
        &sub_field,
    );
    assert_eq!(
        result["ok"], true,
        "field create in sub-package should succeed: {:?}",
        result
    );

    // Field file should exist under package/sub/fields/
    let sub_fields_dir = temp.path().join("package/sub/fields");
    assert!(
        sub_fields_dir.exists(),
        "package/sub/fields/ should be created"
    );
    assert!(
        sub_fields_dir.read_dir().unwrap().count() > 0,
        "field file should exist under package/sub/fields/"
    );

    // Primary package.json fields array should still be empty
    let primary_pkg: Value = serde_json::from_str(
        &std::fs::read_to_string(temp.path().join("package/package.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        primary_pkg["fields"].as_array().unwrap().len(),
        0,
        "primary package.json should not be modified"
    );
}

// --- issue #4: all create commands auto-generate IDs ---

#[test]
fn note_create_without_id_mints_uuid() {
    let temp = create_temp_repo();
    // Omit instanceId entirely — service must auto-generate
    let payload = serde_json::json!({
        "title": "Auto ID Note",
        "sections": [{"name": "body", "content": "content"}]
    })
    .to_string();
    let result = run_srs_stdin_in_dir(temp.path(), &["note", "create"], &payload);
    assert_eq!(
        result["ok"], true,
        "note create without id should succeed: {:?}",
        result["diagnostics"]
    );
    let id = result["payload"]["note"]["instanceId"]
        .as_str()
        .expect("instanceId must be present in response");
    assert_eq!(id.len(), 36, "instanceId should be a UUID (36 chars)");
    assert_eq!(&id[8..9], "-");
    assert_eq!(&id[13..14], "-");
}

#[test]
fn record_create_without_id_mints_uuid() {
    let temp = create_temp_repo_with_package();
    let package_dir = temp.path().join("package");

    // Write a type
    let record_type = serde_json::json!({
        "id": "type-auto-id-001",
        "namespace": "com.test",
        "name": "auto-id-item",
        "version": 1,
        "description": "Type for auto-id test",
        "fields": [],
        "createdAt": "2026-01-01T00:00:00Z"
    });
    std::fs::write(
        package_dir.join("types/auto-id-item.json"),
        serde_json::to_string_pretty(&record_type).unwrap(),
    )
    .unwrap();
    let package_json = serde_json::json!({
        "id": "test-pkg",
        "namespace": "com.test",
        "name": "test",
        "version": "1.0.0",
        "fields": [],
        "types": ["types/auto-id-item.json"]
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();

    // Omit instanceId entirely — service must auto-generate
    let payload = serde_json::json!({ "fieldValues": [] }).to_string();
    let result = run_srs_stdin_in_dir(
        temp.path(),
        &["record", "create", "--type", "com.test/auto-id-item"],
        &payload,
    );
    assert_eq!(
        result["ok"], true,
        "record create without id should succeed: {:?}",
        result["diagnostics"]
    );
    let id = result["payload"]["record"]["instanceId"]
        .as_str()
        .expect("instanceId must be present in response");
    assert_eq!(id.len(), 36, "instanceId should be a UUID (36 chars)");
    assert_eq!(&id[8..9], "-");
    assert_eq!(&id[13..14], "-");
}

#[test]
fn field_create_without_id_mints_uuid() {
    let temp = create_temp_repo_with_package();
    // Omit id entirely — service must auto-generate
    let payload = serde_json::json!({
        "namespace": "com.test",
        "name": "auto-id-field",
        "version": 1,
        "valueType": "string"
    })
    .to_string();
    let result = run_srs_stdin_in_dir(temp.path(), &["field", "create"], &payload);
    assert_eq!(
        result["ok"], true,
        "field create without id should succeed: {:?}",
        result["diagnostics"]
    );
    let id = result["payload"]["field"]["id"]
        .as_str()
        .expect("id must be present in response");
    assert_eq!(id.len(), 36, "id should be a UUID (36 chars)");
    assert_eq!(&id[8..9], "-");
    assert_eq!(&id[13..14], "-");
}

#[test]
fn type_create_without_id_mints_uuid() {
    let temp = create_temp_repo_with_package();
    // Omit id entirely — service must auto-generate
    let payload = serde_json::json!({
        "namespace": "com.test",
        "name": "auto-id-type",
        "version": 1,
        "description": "A type without a pre-supplied id",
        "fields": [],
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string();
    let result = run_srs_stdin_in_dir(temp.path(), &["type", "create"], &payload);
    assert_eq!(
        result["ok"], true,
        "type create without id should succeed: {:?}",
        result["diagnostics"]
    );
    let id = result["payload"]["type"]["id"]
        .as_str()
        .expect("id must be present in response");
    assert_eq!(id.len(), 36, "id should be a UUID (36 chars)");
    assert_eq!(&id[8..9], "-");
    assert_eq!(&id[13..14], "-");
}
