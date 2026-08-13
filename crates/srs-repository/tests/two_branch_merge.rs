//! RFC-038 acceptance tests 1 and 2, as real git-fixture scenarios.
//!
//! Two branches from one base each add a Tier-0 Note (test 1) or a Relation
//! (test 2). Neither diff touches `manifest.json` or any shared file, the
//! merge is textually clean, and enumeration from the merge result finds both
//! objects. This is the property the whole cutover exists to buy: repository
//! writes are conflict-free single-object file operations.
//!
//! The scenarios run real `git` (init → branch → merge) in a tempdir; git is
//! available anywhere the test suite runs (CI checks out the repo with it).

use srs_repository::store::{FileStore, RepositoryStore};
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "conformance")
        .env("GIT_AUTHOR_EMAIL", "conformance@test")
        .env("GIT_COMMITTER_NAME", "conformance")
        .env("GIT_COMMITTER_EMAIL", "conformance@test")
        .output()
        .expect("git must be runnable");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A minimal final-format repository: manifest + marker + one base note.
fn write_base_repo(root: &Path) {
    std::fs::create_dir_all(root.join(".srs")).unwrap();
    std::fs::write(root.join(".srs/.gitkeep"), "").unwrap();
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "dataModelRevision": 2,
            "srsVersion": "2.0-draft",
            "repositoryId": "00000000-0000-4000-8000-00000000ba5e",
            "namespace": "com.test.merge"
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("records/notes")).unwrap();
    std::fs::write(
        root.join("records/notes/base.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "instanceId": "00000000-0000-4000-8000-000000000ba5",
            "title": "Base",
            "sections": [{"name": "body", "content": "base"}]
        }))
        .unwrap(),
    )
    .unwrap();
}

fn note_json(id: &str, title: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "instanceId": id,
        "title": title,
        "sections": [{"name": "body", "content": title}]
    }))
    .unwrap()
}

fn relation_json(id: &str, source: &str, target: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/relation.json",
        "relationId": id,
        "relationType": "precedes",
        "sourceInstanceId": source,
        "targetInstanceId": target,
        "createdAt": "2026-01-01T00:00:00Z"
    }))
    .unwrap()
}

/// Run one two-branch scenario: `on_branch_a`/`on_branch_b` each add files,
/// the branches merge, and the merged tree must be textually clean with
/// `manifest.json` untouched since base.
fn run_two_branch_scenario(
    on_branch_a: impl Fn(&Path),
    on_branch_b: impl Fn(&Path),
) -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write_base_repo(root);
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);

    git(root, &["checkout", "-q", "-b", "branch-a"]);
    on_branch_a(root);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "a"]);

    git(root, &["checkout", "-q", "main"]);
    git(root, &["checkout", "-q", "-b", "branch-b"]);
    on_branch_b(root);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "b"]);

    git(root, &["checkout", "-q", "main"]);
    git(root, &["merge", "-q", "--no-edit", "branch-a"]);
    // The second merge is the one that would conflict under index-based
    // storage; it must be clean.
    git(root, &["merge", "-q", "--no-edit", "branch-b"]);

    // Neither branch touched manifest.json.
    let out = Command::new("git")
        .args([
            "diff",
            "--name-only",
            "HEAD~2",
            "HEAD",
            "--",
            "manifest.json",
        ])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.stdout.is_empty(),
        "manifest.json must be untouched by either branch"
    );
    tmp
}

#[test]
fn two_branches_adding_notes_merge_cleanly_and_both_enumerate() {
    let note_a = "aaaaaaaa-0000-4000-8000-000000000001";
    let note_b = "bbbbbbbb-0000-4000-8000-000000000002";
    let tmp = run_two_branch_scenario(
        |root| {
            std::fs::write(
                root.join("records/notes/from-a.json"),
                note_json(note_a, "From A"),
            )
            .unwrap();
        },
        |root| {
            std::fs::write(
                root.join("records/notes/from-b.json"),
                note_json(note_b, "From B"),
            )
            .unwrap();
        },
    );

    let cat = FileStore::new(tmp.path())
        .catalog()
        .expect("merged repo enumerates");
    assert!(cat.diagnostics.is_empty(), "{:?}", cat.diagnostics);
    let ids: Vec<&str> = cat.instances.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&note_a), "note from branch A enumerates");
    assert!(ids.contains(&note_b), "note from branch B enumerates");
}

#[test]
fn two_branches_adding_relations_merge_cleanly_and_both_enumerate() {
    let base = "00000000-0000-4000-8000-000000000ba5";
    let note_a = "aaaaaaaa-0000-4000-8000-000000000001";
    let note_b = "bbbbbbbb-0000-4000-8000-000000000002";
    let rel_a = "cccccccc-0000-4000-8000-000000000001";
    let rel_b = "dddddddd-0000-4000-8000-000000000002";
    let tmp = run_two_branch_scenario(
        |root| {
            std::fs::write(
                root.join("records/notes/from-a.json"),
                note_json(note_a, "From A"),
            )
            .unwrap();
            std::fs::create_dir_all(root.join("relations")).unwrap();
            std::fs::write(
                root.join(format!("relations/{rel_a}.json")),
                relation_json(rel_a, base, note_a),
            )
            .unwrap();
        },
        |root| {
            std::fs::write(
                root.join("records/notes/from-b.json"),
                note_json(note_b, "From B"),
            )
            .unwrap();
            std::fs::create_dir_all(root.join("relations")).unwrap();
            std::fs::write(
                root.join(format!("relations/{rel_b}.json")),
                relation_json(rel_b, base, note_b),
            )
            .unwrap();
        },
    );

    let cat = FileStore::new(tmp.path())
        .catalog()
        .expect("merged repo enumerates");
    assert!(cat.diagnostics.is_empty(), "{:?}", cat.diagnostics);
    let rel_ids: Vec<&str> = cat.relations.iter().map(|e| e.id.as_str()).collect();
    assert!(
        rel_ids.contains(&rel_a),
        "relation from branch A enumerates"
    );
    assert!(
        rel_ids.contains(&rel_b),
        "relation from branch B enumerates"
    );
}
