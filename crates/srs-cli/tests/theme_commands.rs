use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

// ---------- helpers ----------

fn create_temp_repo_with_themes() -> TempDir {
    let temp = TempDir::new().expect("Failed to create temp dir");
    std::fs::create_dir_all(temp.path().join(".srs")).unwrap();
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_string_pretty(&serde_json::json!({ "instanceIndex": [] })).unwrap(),
    )
    .unwrap();
    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(package_dir.join("themes")).unwrap();
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

fn run_srs(dir: &std::path::Path, args: &[&str]) -> Value {
    let exe = env!("CARGO_BIN_EXE_srs");
    let output = Command::new(exe)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("Failed to execute srs command");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "srs command failed.\nstderr: {}\nstdout: {}",
        stderr,
        stdout_str
    );

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
    serde_json::from_str(&stdout).expect("Failed to parse JSON output")
}

fn run_srs_stdin(dir: &std::path::Path, args: &[&str], stdin: &str) -> Value {
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
        "srs command failed.\nstderr: {}\nstdout: {}",
        stderr,
        stdout_str
    );

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
    serde_json::from_str(&stdout).expect("Failed to parse JSON output")
}

fn minimal_theme_json(name: &str) -> String {
    serde_json::json!({
        "id": "",
        "namespace": "com.test",
        "name": name,
        "version": 1,
        "description": "A test theme",
        "targets": ["markdown"],
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string()
}

fn minimal_document_view_with_theme_json(theme_id: &str) -> String {
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
        "themeRef": {
            "mode": "bundled",
            "themeId": theme_id
        },
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string()
}

// ---------- tests ----------

#[test]
fn theme_list_returns_ok_envelope() {
    let temp = create_temp_repo_with_themes();
    let result = run_srs(temp.path(), &["theme", "list"]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["command"], "theme list");
    assert!(result["payload"]["themes"].is_array());
    assert_eq!(result["payload"]["themes"].as_array().unwrap().len(), 0);
}

#[test]
fn theme_create_returns_theme_with_id() {
    let temp = create_temp_repo_with_themes();
    let result = run_srs_stdin(
        temp.path(),
        &["theme", "create"],
        &minimal_theme_json("my-theme"),
    );
    assert_eq!(result["ok"], true, "theme create failed: {:?}", result);
    let id = result["payload"]["theme"]["id"].as_str().unwrap();
    assert!(!id.is_empty(), "id should be assigned");
}

#[test]
fn theme_list_contains_created_theme() {
    let temp = create_temp_repo_with_themes();
    let created = run_srs_stdin(
        temp.path(),
        &["theme", "create"],
        &minimal_theme_json("listed-theme"),
    );
    let id = created["payload"]["theme"]["id"].as_str().unwrap();

    let result = run_srs(temp.path(), &["theme", "list"]);
    assert_eq!(result["ok"], true);
    let themes = result["payload"]["themes"].as_array().unwrap();
    assert!(
        themes.iter().any(|t| t["id"] == id),
        "created theme should appear in list"
    );
}

#[test]
fn theme_get_returns_created_theme() {
    let temp = create_temp_repo_with_themes();
    let created = run_srs_stdin(
        temp.path(),
        &["theme", "create"],
        &minimal_theme_json("get-theme"),
    );
    let id = created["payload"]["theme"]["id"].as_str().unwrap();

    let result = run_srs(temp.path(), &["theme", "get", id]);
    assert_eq!(result["ok"], true);
    assert_eq!(result["payload"]["theme"]["name"], "get-theme");
}

#[test]
fn theme_get_unknown_id_returns_ok_false() {
    let temp = create_temp_repo_with_themes();
    let result = run_srs(
        temp.path(),
        &["theme", "get", "00000000-0000-0000-0000-000000000000"],
    );
    assert_eq!(
        result["ok"], false,
        "expected ok=false for unknown theme, got: {:?}",
        result
    );
}

#[test]
fn theme_update_overwrites_description() {
    let temp = create_temp_repo_with_themes();
    let created = run_srs_stdin(
        temp.path(),
        &["theme", "create"],
        &minimal_theme_json("update-me"),
    );
    let id = created["payload"]["theme"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let updated_json = serde_json::json!({
        "id": id,
        "namespace": "com.test",
        "name": "update-me",
        "version": 1,
        "description": "Updated description",
        "targets": ["markdown"],
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string();

    let result = run_srs_stdin(temp.path(), &["theme", "update", &id], &updated_json);
    assert_eq!(result["ok"], true, "theme update failed: {:?}", result);
    assert_eq!(
        result["payload"]["theme"]["description"],
        "Updated description"
    );
}

#[test]
fn theme_delete_removes_from_list() {
    let temp = create_temp_repo_with_themes();
    let created = run_srs_stdin(
        temp.path(),
        &["theme", "create"],
        &minimal_theme_json("delete-me"),
    );
    let id = created["payload"]["theme"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let result = run_srs(temp.path(), &["theme", "delete", &id]);
    assert_eq!(result["ok"], true, "theme delete failed: {:?}", result);
    assert_eq!(result["payload"]["id"], id);

    // Verify not in list anymore
    let list = run_srs(temp.path(), &["theme", "list"]);
    let themes = list["payload"]["themes"].as_array().unwrap();
    assert!(
        themes.iter().all(|t| t["id"] != id),
        "deleted theme should not appear in list"
    );
}

#[test]
fn theme_delete_blocked_when_document_view_references_it() {
    let temp = create_temp_repo_with_themes();

    // Create the package with documentViews array too
    let package_dir = temp.path().join("package");
    std::fs::create_dir_all(package_dir.join("document-views")).unwrap();
    let pkg = serde_json::json!({
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
    });
    std::fs::write(
        package_dir.join("package.json"),
        serde_json::to_string_pretty(&pkg).unwrap(),
    )
    .unwrap();

    // Create theme
    let created = run_srs_stdin(
        temp.path(),
        &["theme", "create"],
        &minimal_theme_json("ref-theme"),
    );
    let theme_id = created["payload"]["theme"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Create a document-view that references the theme
    let dv_json = minimal_document_view_with_theme_json(&theme_id);
    let dv_result = run_srs_stdin(temp.path(), &["document-view", "create"], &dv_json);
    assert_eq!(
        dv_result["ok"], true,
        "document-view create failed: {:?}",
        dv_result
    );

    // Now try to delete the theme — should fail
    let exe = env!("CARGO_BIN_EXE_srs");
    let output = Command::new(exe)
        .args(["theme", "delete", &theme_id])
        .current_dir(temp.path())
        .output()
        .expect("Failed to execute srs command");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
    let result: Value = serde_json::from_str(&stdout).expect("Failed to parse JSON output");

    assert_eq!(
        result["ok"], false,
        "theme delete should be blocked when referenced by document-view, got: {:?}",
        result
    );
}

#[test]
fn theme_list_filters_by_namespace() {
    let temp = create_temp_repo_with_themes();

    run_srs_stdin(
        temp.path(),
        &["theme", "create"],
        &minimal_theme_json("ns-theme"),
    );

    let other_theme = serde_json::json!({
        "id": "",
        "namespace": "com.other",
        "name": "other-theme",
        "version": 1,
        "description": "Other namespace theme",
        "targets": ["html"],
        "createdAt": "2026-01-01T00:00:00Z"
    })
    .to_string();
    run_srs_stdin(temp.path(), &["theme", "create"], &other_theme);

    let result = run_srs(temp.path(), &["theme", "list", "--namespace", "com.test"]);
    assert_eq!(result["ok"], true);
    let themes = result["payload"]["themes"].as_array().unwrap();
    assert!(
        themes.iter().all(|t| t["namespace"] == "com.test"),
        "filtered list should only contain com.test themes"
    );
    assert!(
        !themes.is_empty(),
        "filtered list should contain the com.test theme"
    );
}
