//! CLI integration tests for `srs package install` (#506).

use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn run_srs(dir: &Path, args: &[&str]) -> Value {
    let exe = env!("CARGO_BIN_EXE_srs");
    let output = Command::new(exe)
        .env_remove("SRS_ACTOR")
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
            "fieldType": {"datatype": "string"},
            "description": "Short label.",
            "aiGuidance": {"purpose": "Short label."},
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
            "fieldType": {"datatype": "string", "format": "markdown"},
            "description": "Free-text notes.",
            "aiGuidance": {"purpose": "Free-text notes."},
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

// ── .srspkg export / install --bundle (#632, #690; ADR-050) ─────────────────

/// Raw runner: exit status plus the parsed stdout envelope (if any).
fn run_raw(dir: &Path, args: &[&str]) -> (bool, Option<Value>) {
    let output = Command::new(env!("CARGO_BIN_EXE_srs"))
        .env_remove("SRS_ACTOR")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("Failed to execute srs command");
    let stdout = String::from_utf8_lossy(&output.stdout);
    (output.status.success(), serde_json::from_str(&stdout).ok())
}

const AT: &str = "2026-10-03T00:00:00Z";
const SELECTOR: &str = "packages/cli-fixture";

/// A workspace with repo A (source package installed) and an empty repo B.
fn two_repos() -> (TempDir, String, String) {
    let workspace = TempDir::new().unwrap();
    let source_dir = workspace.path().join("source-pkg");
    std::fs::create_dir_all(&source_dir).unwrap();
    write_source_package(&source_dir);
    let a = workspace.path().join("a").to_string_lossy().into_owned();
    let b = workspace.path().join("b").to_string_lossy().into_owned();
    for (repo, ns) in [(&a, "com.cli.a"), (&b, "com.cli.b")] {
        std::fs::create_dir_all(repo).unwrap();
        run_srs(
            workspace.path(),
            &["--repo", repo, "repo", "create", "--namespace", ns],
        );
    }
    run_srs(
        workspace.path(),
        &[
            "--repo",
            &a,
            "package",
            "install",
            &source_dir.to_string_lossy(),
        ],
    );
    (workspace, a, b)
}

fn export(dir: &Path, repo: &str, out: &str) -> Value {
    run_srs(
        dir,
        &[
            "--repo",
            repo,
            "package",
            "export",
            "--selector",
            SELECTOR,
            "--output",
            out,
            "--published-at",
            AT,
        ],
    )
}

#[test]
fn package_export_cli_writes_srspkg_and_reports_sha256() {
    use sha2::{Digest, Sha256};
    let (ws, a, _b) = two_repos();
    let out = ws.path().join("p.srspkg");
    let result = export(ws.path(), &a, &out.to_string_lossy());
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["command"], "package export");
    let bytes = std::fs::read(&out).unwrap();
    let p = &result["payload"];
    assert_eq!(
        p["sha256"],
        format!("sha256:{}", hex::encode(Sha256::digest(&bytes)))
    );
    assert_eq!(p["byteLength"], bytes.len());
    assert_eq!(p["definitionCount"], 3);
    assert_eq!(p["publishedAt"], AT);
    // The item Type references both of its own Fields.
    assert_eq!(p["dependencyRefCount"], 2);
    assert_eq!(p["mode"], "bundled");
    assert_eq!(p["notes"], serde_json::json!([]));
    let bundle: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(bundle["dependencyRefs"].as_array().unwrap().len(), 2);
}

#[test]
fn package_export_cli_is_deterministic_with_published_at() {
    let (ws, a, _b) = two_repos();
    let (p1, p2) = (ws.path().join("1.srspkg"), ws.path().join("2.srspkg"));
    export(ws.path(), &a, &p1.to_string_lossy());
    export(ws.path(), &a, &p2.to_string_lossy());
    assert_eq!(std::fs::read(&p1).unwrap(), std::fs::read(&p2).unwrap());
}

#[test]
fn package_install_cli_bundle_roundtrip() {
    let (ws, a, b) = two_repos();
    let out = ws.path().join("p.srspkg").to_string_lossy().into_owned();
    export(ws.path(), &a, &out);
    let installed = run_srs(
        ws.path(),
        &["--repo", &b, "package", "install", "--bundle", &out],
    );
    assert_eq!(installed["ok"], true, "{installed}");
    assert_eq!(installed["payload"]["installed"], 3);
    assert_eq!(installed["payload"]["notes"], serde_json::json!([]));
    let list = run_srs(ws.path(), &["--repo", &b, "package", "list"]);
    assert!(list["payload"]["packages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["boundaryPath"] == SELECTOR));
    let validate = run_srs(ws.path(), &["--repo", &b, "repo", "validate"]);
    assert_eq!(validate["payload"]["summary"]["errors"], 0, "{validate}");
}

/// srs-rust#1473: `type create` of version 2 writes beside version 1, so a record bound to
/// v1 stays valid, both versions are listed, and a bundle carries both into another repo.
#[test]
fn type_create_new_version_beside_old_and_bundle_roundtrip() {
    let (ws, a, b) = two_repos();
    let dir = ws.path();
    let item = "11111111-0003-4333-8444-555555555555";

    let rec = run_srs_stdin(
        dir,
        &[
            "--repo",
            &a,
            "record",
            "create",
            "--type",
            "com.cli.install/item",
        ],
        r#"{"fieldValues": {"label": "bound to v1"}}"#,
    );
    assert_eq!(rec["ok"], true, "{rec}");

    let v2 = serde_json::json!({
        "id": item,
        "namespace": "com.cli.install",
        "name": "item",
        "version": 2,
        "description": "An item, version 2.",
        "createdAt": "2026-10-28T00:00:00Z",
        "fields": [
            {"fieldId": "11111111-0001-4333-8444-555555555555", "order": 0, "required": true}
        ]
    });
    let created = run_srs_stdin(
        dir,
        &["--repo", &a, "type", "create", "--package", SELECTOR],
        &v2.to_string(),
    );
    assert_eq!(created["ok"], true, "{created}");
    assert!(Path::new(&a)
        .join(SELECTOR)
        .join("types/item-11111111-v2.json")
        .exists());

    let versions = |repo: &str| -> Vec<u64> {
        let list = run_srs(dir, &["--repo", repo, "type", "list"]);
        let mut v: Vec<u64> = list["payload"]["types"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["id"] == item)
            .map(|t| t["version"].as_u64().unwrap())
            .collect();
        v.sort_unstable();
        v
    };
    assert_eq!(versions(&a), vec![1, 2]);
    let validate = run_srs(dir, &["--repo", &a, "repo", "validate"]);
    assert_eq!(validate["payload"]["summary"]["errors"], 0, "{validate}");

    let out = dir
        .join("two-versions.srspkg")
        .to_string_lossy()
        .into_owned();
    export(dir, &a, &out);
    let installed = run_srs(dir, &["--repo", &b, "package", "install", "--bundle", &out]);
    assert_eq!(installed["ok"], true, "{installed}");
    assert_eq!(versions(&b), vec![1, 2]);
    let validate = run_srs(dir, &["--repo", &b, "repo", "validate"]);
    assert_eq!(validate["payload"]["summary"]["errors"], 0, "{validate}");
}

fn run_srs_stdin(dir: &Path, args: &[&str], stdin: &str) -> Value {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_srs"))
        .env_remove("SRS_ACTOR")
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to execute srs command");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    serde_json::from_slice(&output.stdout).expect("Failed to parse JSON output")
}

#[test]
fn package_install_cli_rejects_source_dir_with_bundle() {
    let (ws, _a, b) = two_repos();
    let (ok, _) = run_raw(
        ws.path(),
        &[
            "--repo", &b, "package", "install", "some-dir", "--bundle", "p.srspkg",
        ],
    );
    assert!(!ok, "clap must refuse <source_dir> together with --bundle");
}

#[test]
fn package_export_cli_unknown_selector_is_error_envelope() {
    let (ws, a, _b) = two_repos();
    let out = ws.path().join("x.srspkg").to_string_lossy().into_owned();
    let (_, env) = run_raw(
        ws.path(),
        &[
            "--repo",
            &a,
            "package",
            "export",
            "--selector",
            "packages/nope",
            "--output",
            &out,
        ],
    );
    let env = env.expect("an error envelope on stdout");
    assert_eq!(env["ok"], false, "{env}");
}

#[test]
fn package_install_cli_bundle_newer_revision_is_error_envelope() {
    let (ws, a, b) = two_repos();
    let out = ws.path().join("p.srspkg");
    export(ws.path(), &a, &out.to_string_lossy());
    let mut v: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    v["dataModelRevision"] = serde_json::json!(99);
    let newer = ws.path().join("newer.srspkg");
    std::fs::write(&newer, serde_json::to_vec(&v).unwrap()).unwrap();
    let (_, env) = run_raw(
        ws.path(),
        &[
            "--repo",
            &b,
            "package",
            "install",
            "--bundle",
            &newer.to_string_lossy(),
        ],
    );
    let env = env.expect("an error envelope on stdout");
    assert_eq!(env["ok"], false, "{env}");
    assert_eq!(env["errors"][0]["code"], "bundle-revision-too-new", "{env}");
}

/// `srs package export` with extra flags, against repo `repo`.
fn export_with(dir: &Path, repo: &str, out: &str, extra: &[&str]) -> (bool, Option<Value>) {
    let mut args = vec![
        "--repo",
        repo,
        "package",
        "export",
        "--selector",
        SELECTOR,
        "--output",
        out,
        "--published-at",
        AT,
    ];
    args.extend_from_slice(extra);
    run_raw(dir, &args)
}

#[test]
fn package_export_cli_mode_standalone() {
    let (ws, a, _b) = two_repos();
    let out = ws.path().join("s.srspkg").to_string_lossy().into_owned();
    let (ok, env) = export_with(ws.path(), &a, &out, &["--mode", "standalone"]);
    let env = env.unwrap();
    assert!(ok, "{env}");
    assert_eq!(env["payload"]["mode"], "standalone");
    let bundle: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(bundle["mode"], "standalone");
    let (ok, _) = export_with(ws.path(), &a, &out, &["--mode", "bogus"]);
    assert!(!ok, "clap must refuse an unknown --mode");
}

#[test]
fn package_export_cli_writes_homepage() {
    let (ws, a, _b) = two_repos();
    let out = ws.path().join("h.srspkg").to_string_lossy().into_owned();
    let (ok, env) = export_with(
        ws.path(),
        &a,
        &out,
        &["--homepage", "https://example.org/pkg"],
    );
    assert!(ok, "{env:?}");
    let bundle: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(bundle["homepage"], "https://example.org/pkg");
    export(ws.path(), &a, &out);
    let bundle: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert!(bundle.get("homepage").is_none());
}

#[test]
fn package_export_cli_from_a_repository_below_revision_10_is_refused() {
    // RFC-052 [R7] (owner ruling 1 on srs#953): a repository declaring revision 6 is refused
    // by its declared revision before export runs, naming the pending migrations, and no
    // bundle is written. The below-floor note is now reachable only from a library store
    // (package_bundle.rs `export_from_repository_below_reader_floor_*`).
    let (ws, a, _b) = two_repos();
    let out = ws.path().join("n.srspkg");
    srs_repository::field_type_migration_service::stamp_data_model_revision(
        &srs_repository::store::FileStore::new(Path::new(&a)),
        6,
    )
    .unwrap();
    let (_, env) = export_with(ws.path(), &a, &out.to_string_lossy(), &[]);
    let env = env.expect("JSON envelope");
    assert_eq!(env["ok"], false, "{env}");
    assert_eq!(
        env["errors"][0]["code"], "container-migration-needed",
        "{env}"
    );
    assert_eq!(
        env["errors"][0]["details"]["pending"][0], "discovery-query-cutover",
        "{env}"
    );
    assert!(!out.exists());
}

#[test]
fn package_install_cli_bundle_below_floor_is_error_envelope() {
    let (ws, a, b) = two_repos();
    let out = ws.path().join("p.srspkg");
    export(ws.path(), &a, &out.to_string_lossy());
    let mut v: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    v["dataModelRevision"] = serde_json::json!(6);
    let old = ws.path().join("old.srspkg");
    std::fs::write(&old, serde_json::to_vec(&v).unwrap()).unwrap();
    let (_, env) = run_raw(
        ws.path(),
        &[
            "--repo",
            &b,
            "package",
            "install",
            "--bundle",
            &old.to_string_lossy(),
        ],
    );
    let env = env.expect("an error envelope on stdout");
    assert_eq!(env["ok"], false, "{env}");
    assert_eq!(
        env["errors"][0]["code"], "bundle-migration-step-missing",
        "{env}"
    );
}

// ── package upgrade (#1152) ────────────────────────────────────────────────

#[test]
fn package_upgrade_cli_dry_run_then_real_run() {
    let (ws, a, b) = two_repos();
    let out = ws.path().join("p.srspkg").to_string_lossy().into_owned();
    export(ws.path(), &a, &out);
    run_srs(
        ws.path(),
        &["--repo", &b, "package", "install", "--bundle", &out],
    );
    // A newer release: version bump plus an in-place change to the item Type.
    let mut bundle: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    bundle["packageVersion"] = Value::from("1.1.0");
    bundle["types"][0]["description"] = Value::from("Changed upstream.");
    let newer = ws
        .path()
        .join("newer.srspkg")
        .to_string_lossy()
        .into_owned();
    std::fs::write(&newer, serde_json::to_vec(&bundle).unwrap()).unwrap();
    let version_of = || {
        let list = run_srs(ws.path(), &["--repo", &b, "package", "list"]);
        let pkgs = list["payload"]["packages"].as_array().unwrap().clone();
        let p = pkgs.iter().find(|p| p["boundaryPath"] == SELECTOR).unwrap();
        p["version"].clone()
    };
    let before = version_of();

    let dry = run_srs(
        ws.path(),
        &[
            "--repo",
            &b,
            "package",
            "upgrade",
            "--bundle",
            &newer,
            "--dry-run",
        ],
    );
    assert_eq!(dry["ok"], true, "{dry}");
    assert_eq!(dry["command"], "package upgrade");
    assert_eq!(dry["payload"]["dryRun"], true);
    assert_eq!(dry["payload"]["upgraded"], true);
    assert_eq!(dry["payload"]["updated"][0]["kind"], "type");
    assert_eq!(version_of(), before, "a dry run writes nothing");

    let real = run_srs(
        ws.path(),
        &[
            "--repo",
            &b,
            "package",
            "upgrade",
            "--bundle",
            &newer,
            "--boundary",
            SELECTOR,
        ],
    );
    assert_eq!(real["payload"]["dryRun"], false);
    assert_eq!(real["payload"]["previousVersion"], "1.0.0");
    assert_eq!(real["payload"]["updated"], dry["payload"]["updated"]);
    assert_eq!(version_of(), "1.1.0");
    let validate = run_srs(ws.path(), &["--repo", &b, "repo", "validate"]);
    assert_eq!(validate["payload"]["summary"]["errors"], 0, "{validate}");

    // A downgrade is an error envelope.
    let (ok, env) = run_raw(
        ws.path(),
        &["--repo", &b, "package", "upgrade", "--bundle", &out],
    );
    assert!(!ok);
    let env = env.unwrap();
    assert_eq!(
        env["errors"][0]["code"], "invalid-repository-initialization",
        "{env}"
    );
    // generic bucket code: the message says which refusal.
    assert!(env["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("downgrade refused"));
}

#[test]
fn package_upgrade_cli_prior_bundle_proves_and_adopt_consents() {
    let (ws, a, b) = two_repos();
    let out = ws.path().join("p.srspkg").to_string_lossy().into_owned();
    export(ws.path(), &a, &out);
    run_srs(
        ws.path(),
        &["--repo", &b, "package", "install", "--bundle", &out],
    );
    // Simulate an old install: no reference copies.
    let refs = std::path::Path::new(&b)
        .join(SELECTOR)
        .join(".srs-import/refs");
    std::fs::remove_dir_all(&refs).unwrap();
    let mut bundle: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    bundle["packageVersion"] = Value::from("1.1.0");
    bundle["types"][0]["description"] = Value::from("Changed upstream.");
    let newer = ws
        .path()
        .join("newer.srspkg")
        .to_string_lossy()
        .into_owned();
    std::fs::write(&newer, serde_json::to_vec(&bundle).unwrap()).unwrap();
    let up = |extra: &[&str]| {
        let mut args = vec!["--repo", &b, "package", "upgrade", "--bundle", &newer];
        args.extend_from_slice(extra);
        run_srs(ws.path(), &args)
    };

    let bare = up(&["--dry-run"]);
    assert_eq!(
        bare["payload"]["conflicts"][0]["conflictKind"],
        "no-reference-copy"
    );
    let id = bare["payload"]["conflicts"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let adopt = up(&["--dry-run", "--adopt", &id]);
    assert_eq!(adopt["payload"]["adopted"][0]["id"], id.as_str());
    let proven = up(&["--dry-run", "--prior-bundle", &out]);
    assert_eq!(
        proven["payload"]["conflicts"].as_array().unwrap().len(),
        0,
        "{proven}"
    );
    assert_eq!(proven["payload"]["updated"][0]["provenBy"], "1.0.0");
    let real = up(&["--prior-bundle", &out]);
    assert_eq!(real["payload"]["updated"][0]["provenBy"], "1.0.0");
    let validate = run_srs(ws.path(), &["--repo", &b, "repo", "validate"]);
    assert_eq!(validate["payload"]["summary"]["errors"], 0, "{validate}");
}
