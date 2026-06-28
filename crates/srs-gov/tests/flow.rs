/// Integration tests for srs-gov.
///
/// All tests are READ-ONLY against the gallery-project-v2 example.
/// No spec content is embedded here — paths resolve from the repo root.
/// Per srs-rust CLAUDE.md: "Do not embed spec content directly in Rust source or tests."
///
/// CI prerequisite: cargo build must run before these tests so that both
/// `srs` and `srs-gov` binaries exist in the same target dir. The SRS_BIN
/// env var is set by the test harness to the built `srs` sibling.
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    // tests/ lives in crates/srs-gov/tests/; workspace root is three levels up
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..") // crates/
        .join("..") // srs-rust/
        .join("..") // semanticops/
        .join("srs/docs/spec/examples/gallery-project-v2")
        .canonicalize()
        .expect("gallery-project-v2 not found — run from srs-rust workspace")
}

fn srs_gov_bin() -> PathBuf {
    // Same target dir as this test binary
    let exe = std::env::current_exe().expect("current_exe");
    let deps_dir = exe.parent().expect("bin dir");
    // In test mode the binary lands in target/<profile>/deps/; the real bins are one level up
    let bin_dir = if deps_dir.ends_with("deps") {
        deps_dir.parent().unwrap_or(deps_dir)
    } else {
        deps_dir
    };
    let candidate = bin_dir.join("srs-gov");
    if candidate.exists() {
        return candidate;
    }
    // Fallback: PATH
    PathBuf::from("srs-gov")
}

fn srs_bin() -> PathBuf {
    let candidate = srs_gov_bin()
        .parent()
        .map(|p| p.join("srs"))
        .unwrap_or_else(|| PathBuf::from("srs"));
    if candidate.exists() {
        return candidate;
    }
    PathBuf::from("srs")
}

fn run(args: &[&str]) -> (bool, String) {
    let repo = repo_root();
    let gov = srs_gov_bin();
    let srs = srs_bin();

    let mut cmd = Command::new(&gov);
    cmd.env("SRS_BIN", &srs);
    cmd.arg("--repo").arg(&repo);
    cmd.args(args);

    let out = cmd.output().expect("failed to run srs-gov");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let ok = out.status.success();
    if !ok {
        eprintln!("srs-gov stderr:\n{stderr}");
    }
    (ok, stdout)
}

#[test]
fn top_level_lists_governance_containers() {
    let (ok, out) = run(&[]);
    assert!(ok, "srs-gov top-level failed");
    assert!(
        out.contains("decision_log"),
        "expected decision_log section\n{out}"
    );
    assert!(out.contains("articles"), "expected articles section\n{out}");
    assert!(out.contains("roles"), "expected roles section\n{out}");
}

#[test]
fn decision_log_list_renders_decisions() {
    let (ok, out) = run(&["list", "decision_log"]);
    assert!(ok, "decision_log list failed");
    // The rendered view should contain known decision text from the gallery
    assert!(
        out.contains("Pilot duration") || out.contains("Eye level") || out.contains("Mounting"),
        "expected known decision text\n{out}"
    );
    // Should also include a member ID index
    assert!(
        out.contains("Member IDs"),
        "expected Member IDs section\n{out}"
    );
}

#[test]
fn decision_log_get_shows_field_labels() {
    // bf64442b is the decision_log root record (title: "Limoma Project Decision Log")
    let (ok, out) = run(&[
        "get",
        "decision_log",
        "bf64442b-1e2b-4597-95e7-a665439c7f6f",
    ]);
    assert!(ok, "decision_log get root failed\n{out}");
    // Should show Title label and value from the core schema
    assert!(out.contains("Title"), "expected Title label\n{out}");
    assert!(
        out.contains("Limoma"),
        "expected decision_log title value\n{out}"
    );
}

#[test]
fn create_decision_dry_run_emits_correct_command() {
    let (ok, out) = run(&["create", "decision_log", "decision"]);
    assert!(ok, "create dry-run failed\n{out}");
    // Should print srs record create command
    assert!(
        out.contains("srs record create"),
        "expected srs record create\n{out}"
    );
    assert!(out.contains("governance/decision"), "expected type\n{out}");
    assert!(out.contains("138e2fac"), "expected container id\n{out}");
    // fieldIds for required fields
    assert!(out.contains("d7e82557"), "expected title fieldId\n{out}"); // title
    assert!(
        out.contains("de1296e0"),
        "expected statement fieldId\n{out}"
    ); // decision_statement
}

#[test]
fn create_decision_dry_run_does_not_mutate() {
    use std::fs;
    let repo = repo_root();
    // Snapshot manifest before
    let manifest_path = repo.join("manifest.json");
    let before = fs::read(&manifest_path).expect("read manifest");

    let (ok, _) = run(&["create", "decision_log", "decision"]);
    assert!(ok, "create dry-run failed");

    let after = fs::read(&manifest_path).expect("re-read manifest");
    assert_eq!(before, after, "manifest changed — create is not dry-run!");
}

#[test]
fn explain_flag_prints_commands_without_running() {
    let (ok, out) = run(&["--explain", "list", "decision_log"]);
    assert!(ok, "explain list failed\n{out}");
    assert!(
        out.contains("srs"),
        "expected srs command output in explain mode\n{out}"
    );
    // Should NOT contain rendered decision content (since we didn't run)
    assert!(
        !out.contains("Pilot duration"),
        "render ran in explain mode\n{out}"
    );
}

#[test]
fn repo_create_produces_valid_srsj() {
    use std::fs;

    let tmp = std::env::temp_dir().join(format!(
        "srs-gov-test-{}.srsj",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    let path = tmp.to_string_lossy().into_owned();

    let gov = srs_gov_bin();
    let srs = srs_bin();
    let mut cmd = std::process::Command::new(&gov);
    cmd.env("SRS_BIN", &srs);
    cmd.args([
        "repo-create",
        "--output",
        &path,
        "--title",
        "Test Governance",
    ]);
    let out = cmd.output().expect("run srs-gov repo-create");
    assert!(
        out.status.success(),
        "repo-create failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Output file must exist
    assert!(tmp.exists(), "output file not created");

    // repositoryId must be a non-empty UUID distinct from the seed
    let content: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&tmp).unwrap()).unwrap();
    let repo_id = content["manifest"]["repositoryId"].as_str().unwrap_or("");
    assert!(!repo_id.is_empty(), "repositoryId is empty");
    assert_ne!(
        repo_id, "395ebea2-d8f6-497b-b18c-04c9eacafc94",
        "repositoryId not re-generated"
    );

    // title must be set
    assert_eq!(
        content["manifest"]["title"].as_str(),
        Some("Test Governance")
    );

    // upstreamPackage provenance must be preserved
    let ns = content["manifest"]["meta"]["upstreamPackage"]["namespace"]
        .as_str()
        .unwrap_or("");
    assert_eq!(ns, "com.mudemocracy.governance");

    // srs validate must pass
    let validate = std::process::Command::new(&srs)
        .args(["repo", "validate", "--repo", &path])
        .output()
        .expect("run srs repo validate");
    let vout: serde_json::Value = serde_json::from_slice(&validate.stdout).unwrap();
    assert_eq!(
        vout["payload"]["summary"]["errors"].as_u64(),
        Some(0),
        "validate errors"
    );

    fs::remove_file(&tmp).ok();
}
