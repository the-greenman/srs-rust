//! CLI integration tests for `srs package install` (#506).

use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn run_srs(dir: &Path, args: &[&str]) -> Value {
    let exe = env!("CARGO_BIN_EXE_srs");
    let output = Command::new(exe)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("Failed to execute srs command");
    assert!(
        output.status.success(),
        "srs {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
    serde_json::from_str(&stdout).expect("Failed to parse JSON output")
}

/// Write a minimal external source package (two fields, one type) into `dir`.
fn write_source_package(dir: &Path) {
    let write = |rel: &str, value: Value| {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    };
    write(
        "package.json",
        serde_json::json!({
            "id": "11111111-2222-4333-8444-555555555555",
            "namespace": "com.cli.install",
            "name": "cli-fixture",
            "version": "1.0.0",
            "fields": ["fields/label.json", "fields/notes.json"],
            "types": ["types/item.json"]
        }),
    );
    write(
        "fields/label.json",
        serde_json::json!({
            "id": "11111111-0001-4333-8444-555555555555",
            "namespace": "com.cli.install",
            "name": "label",
            "version": 1,
            "valueType": "string",
            "description": "Short label.",
            "createdAt": "2026-01-01T00:00:00Z"
        }),
    );
    write(
        "fields/notes.json",
        serde_json::json!({
            "id": "11111111-0002-4333-8444-555555555555",
            "namespace": "com.cli.install",
            "name": "notes",
            "version": 1,
            "valueType": "text",
            "description": "Free-text notes.",
            "createdAt": "2026-01-01T00:00:00Z"
        }),
    );
    write(
        "types/item.json",
        serde_json::json!({
            "id": "11111111-0003-4333-8444-555555555555",
            "namespace": "com.cli.install",
            "name": "item",
            "version": 1,
            "description": "An item.",
            "createdAt": "2026-01-01T00:00:00Z",
            "fields": [
                {"fieldId": "11111111-0001-4333-8444-555555555555", "order": 0, "required": true},
                {"fieldId": "11111111-0002-4333-8444-555555555555", "order": 1, "required": false}
            ]
        }),
    );
}

#[test]
fn package_install_cli_end_to_end() {
    let workspace = TempDir::new().expect("temp dir");
    let repo_dir = workspace.path().join("repo");
    let source_dir = workspace.path().join("source-pkg");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::create_dir_all(&source_dir).unwrap();
    write_source_package(&source_dir);

    let repo = repo_dir.to_string_lossy().into_owned();
    let source = source_dir.to_string_lossy().into_owned();

    // Create a fresh repository with the real binary.
    let created = run_srs(
        workspace.path(),
        &[
            "--repo",
            &repo,
            "repo",
            "create",
            "--namespace",
            "com.cli.install.repo",
            "--title",
            "Install CLI Test",
        ],
    );
    assert_eq!(created["ok"], true);

    // Install the external package.
    let result = run_srs(
        workspace.path(),
        &["--repo", &repo, "package", "install", &source],
    );
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "package install");
    let payload = &result["payload"];
    assert_eq!(payload["boundaryPath"], "packages/cli-fixture");
    assert_eq!(payload["packageId"], "11111111-2222-4333-8444-555555555555");
    assert_eq!(payload["installed"], 3);
    assert_eq!(payload["skippedIdentical"], 0);
    assert_eq!(payload["conflicts"].as_array().unwrap().len(), 0);
    assert!(payload["installedAt"].as_str().is_some());
    let kinds = payload["kinds"].as_array().unwrap();
    assert_eq!(kinds[0]["kind"], "field");
    assert_eq!(kinds[0]["installed"], 2);
    assert_eq!(kinds[1]["kind"], "type");
    assert_eq!(kinds[1]["installed"], 1);

    // Re-run: idempotent — everything skipped, same boundary.
    let rerun = run_srs(
        workspace.path(),
        &["--repo", &repo, "package", "install", &source],
    );
    assert_eq!(rerun["payload"]["installed"], 0);
    assert_eq!(rerun["payload"]["skippedIdentical"], 3);
    assert_eq!(rerun["payload"]["boundaryPath"], "packages/cli-fixture");

    // The installed definitions are listed with source-package provenance.
    let fields = run_srs(workspace.path(), &["--repo", &repo, "field", "list"]);
    let listed = fields["payload"]["fields"].as_array().unwrap();
    let label = listed
        .iter()
        .find(|f| f["name"] == "label" && f["namespace"] == "com.cli.install")
        .expect("installed field listed");
    assert_eq!(label["sourcePackage"], "packages/cli-fixture");

    // The target repository validates with zero errors.
    let validate = run_srs(workspace.path(), &["--repo", &repo, "repo", "validate"]);
    assert_eq!(
        validate["payload"]["summary"]["errors"], 0,
        "expected 0 validation errors: {validate}"
    );
}

#[test]
fn package_install_cli_boundary_override() {
    let workspace = TempDir::new().expect("temp dir");
    let repo_dir = workspace.path().join("repo");
    let source_dir = workspace.path().join("source-pkg");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::create_dir_all(&source_dir).unwrap();
    write_source_package(&source_dir);

    let repo = repo_dir.to_string_lossy().into_owned();
    let source = source_dir.to_string_lossy().into_owned();

    run_srs(
        workspace.path(),
        &[
            "--repo",
            &repo,
            "repo",
            "create",
            "--namespace",
            "com.cli.install.repo",
        ],
    );

    let result = run_srs(
        workspace.path(),
        &[
            "--repo",
            &repo,
            "package",
            "install",
            &source,
            "--boundary",
            "packages/custom-slot",
        ],
    );
    assert_eq!(result["payload"]["boundaryPath"], "packages/custom-slot");

    let list = run_srs(workspace.path(), &["--repo", &repo, "package", "list"]);
    let packages = list["payload"]["packages"].as_array().unwrap();
    assert!(packages
        .iter()
        .any(|p| p["boundaryPath"] == "packages/custom-slot"));
}
