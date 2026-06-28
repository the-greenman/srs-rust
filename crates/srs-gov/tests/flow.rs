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
fn top_level_reports_nonzero_decision_log_members() {
    let (ok, out) = run(&[]);
    assert!(ok, "srs-gov top-level failed");

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
fn create_decision_dry_run_escapes_quoted_values() {
    let (ok, out) = run(&[
        "create",
        "decision_log",
        "decision",
        "--title",
        r#"Adopt the "new" policy"#,
        "--statement",
        "Use quoted title safely",
    ]);
    assert!(ok, "create dry-run failed\n{out}");

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
fn json_flag_top_level_prints_raw_srs_envelope() {
    let (ok, out) = run(&["--json"]);
    assert!(ok, "json top-level failed\n{out}");

    let envelope: serde_json::Value =
        serde_json::from_str(&out).expect("top-level --json should print JSON");
    assert_eq!(envelope["ok"].as_bool(), Some(true), "expected ok envelope");
    assert!(
        envelope["payload"]["containers"].is_array(),
        "expected container list payload\n{out}"
    );
}

#[test]
fn json_flag_list_prints_raw_resolve_view_envelope() {
    let (ok, out) = run(&["--json", "list", "decision_log"]);
    assert!(ok, "json list failed\n{out}");

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

    let dl = srs_json(&path, &["container", "list"], None)["payload"]["containers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["containerType"] == "decision_log")
        .and_then(|c| c["containerId"].as_str())
        .expect("decision_log container")
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
    transition(&path, &superseded, "superseded");

    // closed (… -> ratified -> closed)
    let closed = create_decision(&path, &dl, "Close the first budget", "spending approved");
    transition(&path, &closed, "proposed");
    transition(&path, &closed, "ratified");
    transition(&path, &closed, "closed");

    TempGovRepo { path }
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
