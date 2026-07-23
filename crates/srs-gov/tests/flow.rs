/// Integration tests for srs-gov.
///
/// The top-level / get / create / explain tests are READ-ONLY against the
/// gallery-project-v2 example. The `list_*` composition tests (#298) instead build
/// their OWN temp `.srsj` repos via `srs-gov repo-create` + `srs` writes, so they are
/// self-contained and do not depend on the spec gallery (CI checks out srs `master`).
/// No spec content is embedded here — gallery paths resolve from the repo root.
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
fn top_level_lists_only_decision_log() {
    // Release 1 is decision-log-only: srs-gov surfaces only sections whose typeNamespace/typeName
    // match a known ContainerTypeDef, and ignores any unknown sections.
    let repo = setup_repo("top-only");
    let out = gov_out(&repo.path, &[]);
    assert!(
        out.contains("decision_log"),
        "expected decision_log section\n{out}"
    );
    // ⊕ icon confirms by_root_type matched the ContainerTypeDef; · would mean fallback path.
    assert!(out.contains("⊕"), "expected ⊕ icon from matched ContainerTypeDef (check root_type_namespace/root_type_name constants)\n{out}");
    assert!(
        !out.contains("articles"),
        "articles section should not be surfaced in release 1\n{out}"
    );
    assert!(
        !out.contains("roles"),
        "roles section should not be surfaced in release 1\n{out}"
    );
}

#[test]
fn top_level_reports_nonzero_decision_log_members() {
    // setup_repo creates 4 decisions, so member count > 0.
    let repo = setup_repo("top-nonzero");
    let out = gov_out(&repo.path, &[]);

    let decision_log_line = out
        .lines()
        .find(|line| line.contains("decision_log"))
        .unwrap_or_else(|| panic!("expected decision_log row\n{out}"));
    let columns: Vec<&str> = decision_log_line.split_whitespace().collect();
    let count = columns
        .iter()
        .rev()
        .nth(1)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_else(|| panic!("expected numeric count in row: {decision_log_line}"));

    assert!(count > 0, "expected decision_log count > 0\n{out}");
}

#[test]
fn decision_log_list_renders_decisions() {
    // setup_repo inserts "Adopt monthly cadence" and "Records live in the system" decisions.
    let repo = setup_repo("list-render");
    let out = gov_out(&repo.path, &["list", "decision_log"]);
    assert!(
        out.contains("Adopt monthly cadence") || out.contains("Records live in the system"),
        "expected known decision text\n{out}"
    );
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
    // Use a self-contained governance repo — field IDs are package constants regardless of repo.
    let repo = setup_repo("create-cmd");
    let out = gov_out(&repo.path, &["create", "decision_log", "decision", "--dry-run"]);
    assert!(
        out.contains("srs record create"),
        "expected srs record create\n{out}"
    );
    assert!(out.contains("governance/decision"), "expected type\n{out}");
    assert!(
        out.contains("--container"),
        "expected --container flag\n{out}"
    );
    // fieldIds are governance package constants, identical across all repos
    assert!(out.contains("d7e82557"), "expected title fieldId\n{out}"); // title
    assert!(
        out.contains("de1296e0"),
        "expected statement fieldId\n{out}"
    ); // decision_statement
}

#[test]
fn create_decision_dry_run_does_not_mutate() {
    use std::fs;
    let repo = setup_repo("create-nomutate");
    let before = fs::read(&repo.path).expect("read srsj");
    gov_out(&repo.path, &["create", "decision_log", "decision", "--dry-run"]);
    let after = fs::read(&repo.path).expect("re-read srsj");
    assert_eq!(before, after, "srsj changed — create is not dry-run!");
}

#[test]
fn create_decision_dry_run_escapes_quoted_values() {
    let repo = setup_repo("create-escape");
    let out = gov_out(
        &repo.path,
        &[
            "create",
            "decision_log",
            "decision",
            "--dry-run",
            "--title",
            r#"Adopt the "new" policy"#,
            "--statement",
            "Use quoted title safely",
        ],
    );

    let start = out.find("{\n").expect("expected JSON heredoc body");
    let end = out[start..].find("\nEOF").expect("expected heredoc EOF") + start;
    let body = &out[start..end];
    let parsed: serde_json::Value =
        serde_json::from_str(body).unwrap_or_else(|err| panic!("invalid JSON body: {err}\n{body}"));
    let values = parsed["fieldValues"]
        .as_array()
        .expect("fieldValues should be an array");

    assert!(
        values
            .iter()
            .any(|field| field["value"].as_str() == Some(r#"Adopt the "new" policy"#)),
        "expected quoted title to round-trip\n{body}"
    );
}

#[test]
fn explain_flag_prints_commands_without_running() {
    let repo = setup_repo("explain");
    let out = gov_out(&repo.path, &["--explain", "list", "decision_log"]);
    assert!(
        out.contains("srs"),
        "expected srs command output in explain mode\n{out}"
    );
    // Should NOT contain rendered decision content (since we returned early in explain mode)
    assert!(
        !out.contains("Adopt monthly cadence"),
        "render ran in explain mode\n{out}"
    );
}

#[test]
fn json_flag_top_level_prints_raw_srs_envelope() {
    let repo = setup_repo("json-top");
    let out = gov_out(&repo.path, &["--json"]);

    let envelope: serde_json::Value =
        serde_json::from_str(&out).expect("top-level --json should print JSON");
    assert_eq!(envelope["ok"].as_bool(), Some(true), "expected ok envelope");
    assert!(
        envelope["payload"]["navigation"].is_object(),
        "expected navigation payload\n{out}"
    );
}

#[test]
fn json_flag_list_prints_raw_resolve_view_envelope() {
    let repo = setup_repo("json-list");
    let out = gov_out(&repo.path, &["--json", "list", "decision_log"]);

    let envelope: serde_json::Value =
        serde_json::from_str(&out).expect("list --json should print JSON");
    assert_eq!(envelope["ok"].as_bool(), Some(true), "expected ok envelope");
    assert!(
        envelope["payload"]["containerView"].is_object(),
        "expected resolve-view payload\n{out}"
    );
    assert!(
        !out.contains("Member IDs"),
        "--json should not include friendly rendered sections\n{out}"
    );
}

#[test]
fn tui_smoke_renders_first_frame() {
    let (ok, out) = run(&["tui", "--smoke"]);
    assert!(ok, "tui smoke failed\n{out}");
    assert!(
        out.contains("srs-gov tui smoke ok"),
        "expected smoke success message\n{out}"
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

    // upstreamPackage provenance must be preserved (RFC-014: top-level field, not meta)
    let ns = content["manifest"]["upstreamPackage"]["namespace"]
        .as_str()
        .unwrap_or("");
    assert_eq!(ns, "com.mudemocracy.governance");
    // Regression #428: contentHash must be absent (removed from spec schema, RFC-014 Rev 4)
    assert!(
        content["manifest"]["upstreamPackage"]["contentHash"].is_null(),
        "contentHash must be absent from upstreamPackage (removed from spec schema)"
    );

    // RFC-013: a required root container is scaffolded with identity + sections in the store.
    let container_embed = &content["manifest"]["container"];
    assert!(container_embed.is_object(), "manifest.container missing");
    let identity = container_embed["identityInstanceId"].as_str().unwrap_or("");
    assert!(!identity.is_empty(), "container has no identityInstanceId");
    let container_id = container_embed["containerId"].as_str().unwrap_or("");
    assert!(!container_id.is_empty(), "container has no containerId");

    // identity must resolve in the instance index
    let index: std::collections::HashSet<&str> = content["manifest"]["instanceIndex"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["instanceId"].as_str())
        .collect();
    assert!(
        index.contains(identity),
        "identity does not resolve in index"
    );

    // full container must be saved to the store with memberInstanceIds
    let container_key = format!("containers/{container_id}.json");
    let full_container = &content["data"][&container_key];
    assert!(
        full_container.is_object(),
        "root container not found in data[\"containers/{container_id}.json\"]"
    );
    let member_ids: Vec<&str> = full_container["memberInstanceIds"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(
        member_ids.contains(&identity),
        "identity not in root container memberInstanceIds"
    );
    // Root container must also contain at least one section (the decision-log root).
    assert!(
        member_ids.len() >= 2,
        "root container memberInstanceIds should contain identity + at least one section, got {:?}",
        member_ids
    );

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

#[test]
fn repo_create_explicit_namespace_applied() {
    use std::fs;

    let tmp = std::env::temp_dir().join(format!(
        "srs-gov-ns-{}.srsj",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    let path = tmp.to_string_lossy().into_owned();

    let gov = srs_gov_bin();
    let srs = srs_bin();
    let out = std::process::Command::new(&gov)
        .env("SRS_BIN", &srs)
        .args([
            "repo-create",
            "--output",
            &path,
            "--title",
            "Acme Governance",
            "--namespace",
            "com.acme.myorg",
        ])
        .output()
        .expect("run srs-gov repo-create --namespace");
    assert!(
        out.status.success(),
        "repo-create --namespace failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let content: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&tmp).unwrap()).unwrap();

    assert_eq!(
        content["manifest"]["namespace"].as_str(),
        Some("com.acme.myorg"),
        "explicit --namespace must appear in manifest"
    );

    // srs validate must still pass with the explicit namespace
    let validate = std::process::Command::new(&srs)
        .args(["repo", "validate", "--repo", &path])
        .output()
        .expect("run srs repo validate");
    let vout: serde_json::Value = serde_json::from_slice(&validate.stdout).unwrap();
    assert_eq!(
        vout["payload"]["summary"]["errors"].as_u64(),
        Some(0),
        "validate errors after --namespace"
    );

    fs::remove_file(&tmp).ok();
}

#[test]
fn repo_create_empty_namespace_is_rejected() {
    let tmp = std::env::temp_dir().join(format!(
        "srs-gov-empty-ns-{}.srsj",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    let path = tmp.to_string_lossy().into_owned();
    let gov = srs_gov_bin();
    let srs = srs_bin();
    let out = std::process::Command::new(&gov)
        .env("SRS_BIN", &srs)
        .args([
            "repo-create",
            "--output",
            &path,
            "--title",
            "Test",
            "--namespace",
            "",
        ])
        .output()
        .expect("run srs-gov repo-create --namespace empty");
    assert!(
        !out.status.success(),
        "empty --namespace should be rejected, but exited 0"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("namespace") || stderr.contains("empty"),
        "error message should mention namespace or empty: {stderr}"
    );
    std::fs::remove_file(&tmp).ok();
}

#[test]
fn repo_create_navigation_works() {
    let path = std::env::temp_dir()
        .join(format!(
            "srs-gov-nav-{}.srsj",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
        .to_string_lossy()
        .into_owned();
    // RAII cleanup on panic or success.
    let _guard = TempGovRepo { path: path.clone() };

    let gov = srs_gov_bin();
    let srs = srs_bin();

    let out = std::process::Command::new(&gov)
        .env("SRS_BIN", &srs)
        .args(["repo-create", "--output", &path, "--title", "Nav Test"])
        .output()
        .expect("run srs-gov repo-create");
    assert!(
        out.status.success(),
        "repo-create failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // srs repo navigation must succeed on a freshly created governance repo.
    let nav_out = std::process::Command::new(&srs)
        .args(["repo", "navigation", "--repo", &path])
        .output()
        .expect("run srs repo navigation");
    let nav: serde_json::Value = serde_json::from_slice(&nav_out.stdout)
        .unwrap_or_else(|_| serde_json::json!({"ok": false, "stdout": String::from_utf8_lossy(&nav_out.stdout).to_string()}));
    assert_eq!(
        nav["ok"].as_bool(),
        Some(true),
        "navigation returned error (exit={}, stderr={}):\n{}",
        nav_out.status,
        String::from_utf8_lossy(&nav_out.stderr),
        nav
    );
    let navigation = &nav["payload"]["navigation"];

    // identity must be the governance article (non-empty instanceId)
    let identity_id = navigation["identity"]["instanceId"].as_str().unwrap_or("");
    assert!(
        !identity_id.is_empty(),
        "navigation identity instanceId is empty"
    );

    // exactly one section: the decision-log root
    let sections = navigation["sections"]
        .as_array()
        .expect("sections is not array");
    assert_eq!(
        sections.len(),
        1,
        "expected 1 section, got {}: {:?}",
        sections.len(),
        sections
    );

    // no diagnostics
    let diagnostics = navigation["diagnostics"]
        .as_array()
        .expect("diagnostics not array");
    assert!(
        diagnostics.is_empty(),
        "navigation returned diagnostics: {:?}",
        diagnostics
    );
}

// ---------------------------------------------------------------------------
// Self-contained list-composition tests (#298, parent plan Section 4).
//
// Each test builds its OWN governance repo via `srs-gov repo-create` and adds
// decisions in draft/ratified/superseded/closed via `srs` writes, then exercises
// `srs-gov list` default-hidden behavior + runtime --all/--search/--tag. These do
// NOT depend on the spec gallery (CI checks out srs `master`, which lags the gallery
// change), so they prove the wiring regardless of cross-repo merge order.
// ---------------------------------------------------------------------------

const TITLE_FIELD: &str = "d7e82557-9045-5e92-a494-d99112bbec4a";
const STMT_FIELD: &str = "de1296e0-e083-58d9-97a0-cb2b91fec02e";

/// A temp `.srsj` governance repo, removed on drop.
struct TempGovRepo {
    path: String,
}

impl Drop for TempGovRepo {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}

/// Run a `srs` subcommand against `repo`, returning the parsed JSON envelope.
fn srs_json(repo: &str, args: &[&str], stdin: Option<&str>) -> serde_json::Value {
    use std::io::Write;
    use std::process::Stdio;

    let srs = srs_bin();
    let mut cmd = Command::new(&srs);
    cmd.args(["--repo", repo, "--format", "json"]);
    cmd.args(args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = cmd.spawn().expect("spawn srs");
    if let Some(s) = stdin {
        child
            .stdin
            .take()
            .expect("stdin pipe")
            .write_all(s.as_bytes())
            .expect("write stdin");
    }
    let out = child.wait_with_output().expect("srs output");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
        panic!(
            "srs {args:?} produced non-JSON: {}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_eq!(v["ok"], true, "srs {args:?} failed: {:?}", v["diagnostics"]);
    v
}

/// Run `srs-gov` against `repo`, asserting success and returning stdout.
fn gov_out(repo: &str, args: &[&str]) -> String {
    let gov = srs_gov_bin();
    let srs = srs_bin();
    let mut cmd = Command::new(&gov);
    cmd.env("SRS_BIN", &srs);
    cmd.arg("--repo").arg(repo);
    cmd.args(args);
    let out = cmd.output().expect("run srs-gov");
    assert!(
        out.status.success(),
        "srs-gov {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn create_decision(repo: &str, dl: &str, title: &str, statement: &str) -> String {
    let body = format!(
        r#"{{"fieldValues":[{{"fieldId":"{TITLE_FIELD}","value":"{title}"}},{{"fieldId":"{STMT_FIELD}","value":"{statement}"}}]}}"#
    );
    let v = srs_json(
        repo,
        &[
            "--container",
            dl,
            "record",
            "create",
            "--type",
            "governance/decision",
        ],
        Some(&body),
    );
    v["payload"]["record"]["instanceId"]
        .as_str()
        .expect("new decision instanceId")
        .to_string()
}

fn transition(repo: &str, id: &str, to: &str) {
    srs_json(
        repo,
        &["record", "transition", "--id", id],
        Some(&format!(r#"{{"to":"{to}"}}"#)),
    );
}

/// RFC-022: the seed's `superseded` state declares `requiresRelation`, so a bare
/// flip is rejected — fulfil the transition by adopting an existing record as the
/// successor (keeps the fixture's record count stable for the list assertions).
fn supersede(repo: &str, id: &str, successor_id: &str) {
    srs_json(
        repo,
        &["record", "transition", "--id", id],
        Some(&format!(
            r#"{{"byTransition":"supersede","fulfillment":{{"existingInstanceId":"{successor_id}"}}}}"#
        )),
    );
}

/// Build a governance repo with one decision in each of draft/ratified/superseded/
/// closed. The ratified decision is tagged `tooling`; its statement carries the
/// unique non-title token `zephyrstore` for the content-search test.
fn setup_repo(suffix: &str) -> TempGovRepo {
    let path = std::env::temp_dir()
        .join(format!(
            "srs-gov-list-{suffix}-{}.srsj",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
        .to_string_lossy()
        .into_owned();

    let gov = srs_gov_bin();
    let srs = srs_bin();
    let out = Command::new(&gov)
        .env("SRS_BIN", &srs)
        .args(["repo-create", "--output", &path, "--title", "Acme"])
        .output()
        .expect("repo-create");
    assert!(
        out.status.success(),
        "repo-create failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let nav = srs_json(&path, &["repo", "navigation"], None);
    let dl = nav["payload"]["navigation"]["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["typeNamespace"] == "governance" && s["typeName"] == "decision_log")
        .and_then(|s| s["sectionContainerId"].as_str())
        .expect("decision_log section via navigation")
        .to_string();

    // draft (initial state — no transition)
    create_decision(
        &path,
        &dl,
        "Adopt monthly cadence",
        "the team meets monthly",
    );

    // ratified (draft -> proposed -> ratified), tagged + unique statement token
    let ratified = create_decision(
        &path,
        &dl,
        "Records live in the system",
        "everything persists in zephyrstore",
    );
    transition(&path, &ratified, "proposed");
    transition(&path, &ratified, "ratified");
    srs_json(&path, &["record", "tag", "add", &ratified, "tooling"], None);

    // superseded (… -> ratified -> superseded)
    let superseded = create_decision(&path, &dl, "Old logo selection", "we picked logo alpha");
    transition(&path, &superseded, "proposed");
    transition(&path, &superseded, "ratified");
    supersede(&path, &superseded, &ratified);

    // closed (… -> ratified -> closed)
    let closed = create_decision(&path, &dl, "Close the first budget", "spending approved");
    transition(&path, &closed, "proposed");
    transition(&path, &closed, "ratified");
    transition(&path, &closed, "closed");

    TempGovRepo { path }
}

// ---------------------------------------------------------------------------
// srs#163: document views must bind to the containers the scaffold created,
// not the gallery-fixture container UUIDs the canonical package ships with.
// ---------------------------------------------------------------------------

/// Package-stable DocumentView UUIDs (com.mudemocracy.governance @1.0.0).
const DECISION_LOG_VIEW: &str = "b5c8d124-2084-4a6b-a231-425e800e1e55";
const DELIBERATION_VIEW: &str = "5a3ce87e-8340-4d91-a140-ab56b57f704f";
const GOV_DOCUMENT_VIEW: &str = "732a982b-3765-4f22-90e0-e456463bac54";

#[test]
fn repo_create_document_views_bind_to_scaffolded_containers() {
    let repo = setup_repo("dv-rebind");

    // 1. validate: zero errors AND zero dangling document-view container warnings
    //    (the #509 validate check would flag any gallery UUID that survived install).
    let v = srs_json(&repo.path, &["repo", "validate"], None);
    assert_eq!(v["payload"]["summary"]["errors"].as_u64(), Some(0));
    let dangling: Vec<_> = v["payload"]["diagnostics"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|d| {
            d["message"]
                .as_str()
                .is_some_and(|m| m.contains("references containerId"))
        })
        .collect();
    assert!(
        dangling.is_empty(),
        "fresh repo-create must not ship dangling document-view container refs: {dangling:?}"
    );

    // 2. articles-and-roles cannot bind in the release-1 (decision-log-only) shape
    //    and must be removed from the install.
    let list = srs_json(&repo.path, &["document-view", "list"], None);
    let names: Vec<&str> = list["payload"]["documentViews"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|dv| dv["name"].as_str())
        .collect();
    assert!(
        !names.contains(&"articles-and-roles"),
        "articles-and-roles must be trimmed from a fresh install, got {names:?}"
    );

    // 3. All three surviving views render ok and include real decision-log content
    //    (setup_repo created decisions; "Adopt monthly cadence" is the draft one).
    for view in [DECISION_LOG_VIEW, DELIBERATION_VIEW, GOV_DOCUMENT_VIEW] {
        let r = srs_json(
            &repo.path,
            &["render", "document-view", "--view", view],
            None,
        );
        let rendered = r["payload"]["rendered"].as_str().unwrap_or("");
        assert!(
            rendered.contains("Adopt monthly cadence"),
            "view {view} must render the decision-log container's content, got:\n{rendered}"
        );
    }
}

#[test]
fn list_hides_superseded_and_closed_by_default() {
    let repo = setup_repo("default");
    let out = gov_out(&repo.path, &["list", "decision_log"]);
    assert!(out.contains("Adopt monthly cadence"), "draft shown\n{out}");
    assert!(
        out.contains("Records live in the system"),
        "ratified shown\n{out}"
    );
    assert!(
        !out.contains("Old logo selection"),
        "superseded must be hidden by default\n{out}"
    );
    assert!(
        !out.contains("Close the first budget"),
        "closed must be hidden by default\n{out}"
    );
}

#[test]
fn list_all_flag_shows_hidden_states() {
    let repo = setup_repo("all");
    let out = gov_out(&repo.path, &["list", "decision_log", "--all"]);
    for title in [
        "Adopt monthly cadence",
        "Records live in the system",
        "Old logo selection",
        "Close the first budget",
    ] {
        assert!(out.contains(title), "--all must show {title}\n{out}");
    }
}

#[test]
fn list_search_narrows_by_content() {
    let repo = setup_repo("search");
    // `zephyrstore` appears only in the ratified decision's decision_statement (a
    // non-title field) — proves content recall over a field the old web filter missed.
    let out = gov_out(
        &repo.path,
        &["list", "decision_log", "--search", "zephyrstore"],
    );
    assert!(
        out.contains("Records live in the system"),
        "search must match the non-title statement\n{out}"
    );
    assert!(
        !out.contains("Adopt monthly cadence"),
        "non-matching decision excluded\n{out}"
    );
}

#[test]
fn list_tag_narrows_by_tag() {
    let repo = setup_repo("tag");
    let out = gov_out(&repo.path, &["list", "decision_log", "--tag", "tooling"]);
    assert!(
        out.contains("Records live in the system"),
        "tagged decision shown\n{out}"
    );
    assert!(
        !out.contains("Adopt monthly cadence"),
        "untagged decision excluded\n{out}"
    );
}

// ---------------------------------------------------------------------------
// Gate A: srs-gov attachment add / list  (#282)
//
// Attachment tests use a directory-format repo (not .srsj) because JsonStore
// silently discards binary file writes — the content file is never stored,
// so list_files_recursive returns nothing. Directory repos write to disk and
// list_files_recursive walks the filesystem correctly.
// ---------------------------------------------------------------------------

/// A temp directory-format SRS repo, removed on drop.
struct TempDirRepo {
    path: String,
}

impl Drop for TempDirRepo {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

/// Create a minimal directory-format SRS repo for attachment tests.
fn setup_dir_repo(suffix: &str) -> TempDirRepo {
    let dir = std::env::temp_dir()
        .join(format!(
            "srs-gov-dir-{suffix}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
        .to_string_lossy()
        .into_owned();

    let srs = srs_bin();
    let out = Command::new(&srs)
        .args([
            "repo",
            "create",
            "--repo",
            &dir,
            "--namespace",
            "com.test.gov",
            "--format",
            "json",
        ])
        .output()
        .expect("srs repo create");
    assert!(
        out.status.success(),
        "srs repo create failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    TempDirRepo { path: dir }
}

#[test]
fn srs_gov_attachment_add_and_list() {
    use std::io::Write;

    let repo = setup_dir_repo("attach");

    let src_path = std::env::temp_dir().join(format!(
        "test-brief-{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::File::create(&src_path)
        .unwrap()
        .write_all(b"Gate A test document")
        .unwrap();
    let src_str = src_path.to_str().unwrap();

    // add must succeed and print a friendly confirmation
    let out = gov_out(
        &repo.path,
        &["attachment", "add", src_str, "--title", "Gate A Brief"],
    );
    assert!(
        out.contains("Attachment stored"),
        "expected confirmation\n{out}"
    );
    assert!(
        out.contains(src_path.file_name().unwrap().to_str().unwrap()),
        "expected file name in output\n{out}"
    );

    // list must show the stored file
    let list_out = gov_out(&repo.path, &["attachment", "list"]);
    assert!(
        list_out.contains(src_path.file_name().unwrap().to_str().unwrap()),
        "file should appear in list\n{list_out}"
    );
    assert!(
        list_out.contains("Gate A Brief"),
        "title should appear in list\n{list_out}"
    );

    // validate must stay clean after the add
    let v = srs_json(&repo.path, &["repo", "validate"], None);
    assert_eq!(
        v["payload"]["summary"]["errors"].as_u64(),
        Some(0),
        "validate must report 0 errors after attachment add"
    );

    // raw srs attachment list must also surface the indexed entry with its title
    let raw = srs_json(&repo.path, &["attachment", "list"], None);
    let entries = raw["payload"]["entries"].as_array().expect("entries array");
    assert!(
        !entries.is_empty(),
        "raw srs attachment list must show entries"
    );
    let entry = entries
        .iter()
        .find(|e| e["title"].as_str() == Some("Gate A Brief"));
    assert!(
        entry.is_some(),
        "indexed entry with title not found: {entries:?}"
    );

    std::fs::remove_file(&src_path).ok();
}

#[test]
fn srs_gov_attachment_add_duplicate_rejected() {
    use std::io::Write;

    let repo = setup_dir_repo("attach-dup");
    let src_path = std::env::temp_dir().join(format!(
        "dup-test-{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::File::create(&src_path)
        .unwrap()
        .write_all(b"duplicate test")
        .unwrap();
    let src_str = src_path.to_str().unwrap();

    // First add succeeds
    gov_out(&repo.path, &["attachment", "add", src_str]);

    // Second add of the same file must fail
    let gov = srs_gov_bin();
    let srs = srs_bin();
    let out = std::process::Command::new(&gov)
        .env("SRS_BIN", &srs)
        .arg("--repo")
        .arg(&repo.path)
        .args(["attachment", "add", src_str])
        .output()
        .expect("run srs-gov");
    assert!(!out.status.success(), "second add of same file must fail");

    std::fs::remove_file(&src_path).ok();
}

// ---------------------------------------------------------------------------
// Phase 2: real create write path
// ---------------------------------------------------------------------------

#[test]
fn create_decision_writes_record() {
    let repo = setup_repo("create-write");

    // Count decisions before
    let before = srs_json(&repo.path, &["record", "list", "--type", "governance/decision"], None);
    let before_count = before["payload"]["records"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);

    // Real write (no --dry-run)
    let out = gov_out(
        &repo.path,
        &[
            "create",
            "decision_log",
            "decision",
            "--title",
            "Test Write Decision",
            "--statement",
            "this proves the write path",
        ],
    );
    assert!(out.contains("Created"), "expected Created header\n{out}");
    // Output must contain a UUID-shaped string (8 hex chars followed by -)
    assert!(
        out.chars().any(|c| c == '-') && out.len() > 20,
        "expected UUID in output\n{out}"
    );

    // Count must have increased
    let after = srs_json(&repo.path, &["record", "list", "--type", "governance/decision"], None);
    let after_count = after["payload"]["records"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(
        after_count > before_count,
        "expected record count to increase: before={before_count} after={after_count}"
    );

    // Validate must be clean
    let v = srs_json(&repo.path, &["repo", "validate"], None);
    assert_eq!(v["payload"]["summary"]["errors"].as_u64(), Some(0));
}

// ---------------------------------------------------------------------------
// Phase 3: transition verb
// ---------------------------------------------------------------------------

#[test]
fn transition_decision_succeeds() {
    let repo = setup_repo("transition-ok");

    // Get a draft decision from setup_repo (first created = "Adopt monthly cadence", still draft)
    let list = srs_json(&repo.path, &["record", "list", "--type", "governance/decision"], None);
    let draft_id = list["payload"]["records"]
        .as_array()
        .expect("records array")
        .iter()
        .find(|r| r["record"]["lifecycleState"].as_str() == Some("draft"))
        .and_then(|r| r["instanceId"].as_str())
        .expect("at least one draft decision from setup_repo")
        .to_string();

    let out = gov_out(&repo.path, &["transition", &draft_id, "--to", "proposed"]);
    assert!(out.contains("Transitioned"), "expected Transitioned header\n{out}");
    assert!(out.contains("proposed"), "expected new state in output\n{out}");

    // Verify via srs record get
    let record = srs_json(&repo.path, &["record", "get", &draft_id], None);
    assert_eq!(
        record["payload"]["record"]["lifecycleState"].as_str(),
        Some("proposed"),
        "lifecycleState must be proposed after transition"
    );

    let v = srs_json(&repo.path, &["repo", "validate"], None);
    assert_eq!(v["payload"]["summary"]["errors"].as_u64(), Some(0));
}

#[test]
fn transition_invalid_state_fails() {
    let repo = setup_repo("transition-bad");

    let list = srs_json(&repo.path, &["record", "list", "--type", "governance/decision"], None);
    let draft_id = list["payload"]["records"]
        .as_array()
        .expect("records array")
        .iter()
        .find(|r| r["record"]["lifecycleState"].as_str() == Some("draft"))
        .and_then(|r| r["instanceId"].as_str())
        .expect("draft decision")
        .to_string();

    let gov = srs_gov_bin();
    let srs = srs_bin();
    let out = std::process::Command::new(&gov)
        .env("SRS_BIN", &srs)
        .arg("--repo")
        .arg(&repo.path)
        .args(["transition", &draft_id, "--to", "nonexistent_state"])
        .output()
        .expect("run srs-gov");
    assert!(
        !out.status.success(),
        "transition to nonexistent_state must fail"
    );
}

#[test]
fn transition_explain_does_not_write() {
    let repo = setup_repo("transition-explain");

    let list = srs_json(&repo.path, &["record", "list", "--type", "governance/decision"], None);
    let draft_id = list["payload"]["records"]
        .as_array()
        .expect("records array")
        .iter()
        .find(|r| r["record"]["lifecycleState"].as_str() == Some("draft"))
        .and_then(|r| r["instanceId"].as_str())
        .expect("draft decision")
        .to_string();

    let out = gov_out(
        &repo.path,
        &["--explain", "transition", &draft_id, "--to", "proposed"],
    );
    assert!(
        out.contains("srs") || out.contains("stdin"),
        "explain mode must print commands\n{out}"
    );

    // State must still be draft
    let record = srs_json(&repo.path, &["record", "get", &draft_id], None);
    assert_eq!(
        record["payload"]["record"]["lifecycleState"].as_str(),
        Some("draft"),
        "--explain must not mutate lifecycle state"
    );
}

// ---------------------------------------------------------------------------
// Phase 4: relate, unrelate, relations verbs
// ---------------------------------------------------------------------------

#[test]
fn relate_and_unrelate() {
    let repo = setup_repo("relate-test");

    let list = srs_json(&repo.path, &["record", "list", "--type", "governance/decision"], None);
    let records = list["payload"]["records"].as_array().expect("records");
    // Use the two draft decisions as source and target (avoids needing to create fresh ones)
    // setup_repo creates: draft, ratified, superseded, closed — the first is always draft.
    let a_id = records
        .iter()
        .find(|r| r["record"]["lifecycleState"].as_str() == Some("draft"))
        .and_then(|r| r["instanceId"].as_str())
        .expect("draft decision A")
        .to_string();
    let b_id = records
        .iter()
        .find(|r| r["record"]["lifecycleState"].as_str() == Some("ratified"))
        .and_then(|r| r["instanceId"].as_str())
        .expect("ratified decision B")
        .to_string();

    // Create a supersedes relation (A draft supersedes B ratified)
    let out = gov_out(
        &repo.path,
        &["relate", &a_id, "--type", "supersedes", "--target", &b_id],
    );
    assert!(
        out.contains("supersedes") || out.contains("Relation"),
        "expected relation confirmation\n{out}"
    );

    // relations list must show it
    let rel_out = gov_out(&repo.path, &["relations", &a_id]);
    assert!(
        rel_out.contains("supersedes"),
        "relations list must show supersedes\n{rel_out}"
    );

    // Validate still clean
    let v = srs_json(&repo.path, &["repo", "validate"], None);
    assert_eq!(v["payload"]["summary"]["errors"].as_u64(), Some(0));

    // Get relation ID to unrelate
    let raw_rel = srs_json(
        &repo.path,
        &["relation", "list", "--source", &a_id],
        None,
    );
    let relation_id = raw_rel["payload"]["relations"]
        .as_array()
        .expect("relations array")
        .iter()
        .find(|r| r["relationType"].as_str() == Some("supersedes"))
        .and_then(|r| r["relationId"].as_str())
        .expect("supersedes relation ID")
        .to_string();

    // Unrelate
    let unrel_out = gov_out(&repo.path, &["unrelate", &relation_id]);
    assert!(
        unrel_out.contains("deleted") || unrel_out.contains("Relation"),
        "expected deletion confirmation\n{unrel_out}"
    );

    // relations list must no longer show supersedes
    let rel_out2 = gov_out(&repo.path, &["relations", &a_id]);
    assert!(
        !rel_out2.contains("supersedes"),
        "supersedes must be gone after unrelate\n{rel_out2}"
    );

    // Validate still clean
    let v2 = srs_json(&repo.path, &["repo", "validate"], None);
    assert_eq!(v2["payload"]["summary"]["errors"].as_u64(), Some(0));
}

#[test]
fn relate_invalid_type_rejected() {
    let repo = setup_repo("relate-bad-type");

    let list = srs_json(&repo.path, &["record", "list", "--type", "governance/decision"], None);
    let id = list["payload"]["records"]
        .as_array()
        .expect("records")
        .first()
        .and_then(|r| r["instanceId"].as_str())
        .expect("any decision")
        .to_string();

    let gov = srs_gov_bin();
    let srs = srs_bin();
    let out = std::process::Command::new(&gov)
        .env("SRS_BIN", &srs)
        .arg("--repo")
        .arg(&repo.path)
        .args(["relate", &id, "--type", "unknown_type", "--target", &id])
        .output()
        .expect("run srs-gov");
    assert!(
        !out.status.success(),
        "relate with unknown_type must fail"
    );
}

#[test]
fn relate_explain_does_not_write() {
    let repo = setup_repo("relate-explain");

    let list = srs_json(&repo.path, &["record", "list", "--type", "governance/decision"], None);
    let records = list["payload"]["records"].as_array().expect("records");
    let a_id = records
        .iter()
        .find(|r| r["record"]["lifecycleState"].as_str() == Some("draft"))
        .and_then(|r| r["instanceId"].as_str())
        .expect("draft decision")
        .to_string();
    let b_id = records
        .iter()
        .find(|r| r["record"]["lifecycleState"].as_str() == Some("ratified"))
        .and_then(|r| r["instanceId"].as_str())
        .expect("ratified decision")
        .to_string();

    let out = gov_out(
        &repo.path,
        &["--explain", "relate", &a_id, "--type", "supersedes", "--target", &b_id],
    );
    assert!(
        out.contains("srs") || out.contains("relation"),
        "explain mode must print commands\n{out}"
    );

    // No relation must exist after explain
    let raw_rel = srs_json(&repo.path, &["relation", "list", "--source", &a_id], None);
    let count = raw_rel["payload"]["relations"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(count, 0, "--explain must not create any relation");
}
