//! RFC-051 per-entry container storage at `dataModelRevision` 11 (srs-rust#1448, #1449): the
//! read path (classification, [R2], decoding, [R14] diagnostics, archive readers), the write
//! path (exact write sets as one-line files, never the head, create/delete/copy, repair) and
//! the merge property the storage exists for, with real `git`.

use serde_json::{json, Value};
use srs_core::arrangement::Placement;
use srs_core::types::container::{Container, ContainerEntry};
use srs_core::types::note::Note;
use srs_repository::container_service::{
    add_member, add_member_relative, copy_container, create_container, delete_container,
    get_container, move_member, remove_member, repair_members, ContainerCopyInput,
};
use srs_repository::container_store::{entry_path, head_path, line};
use srs_repository::repository_lifecycle::{
    create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::store::{FileStore, RepositoryStore};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn repo() -> (TempDir, FileStore) {
    let tmp = TempDir::new().unwrap();
    let store = FileStore::new(tmp.path());
    create_repository(
        &store,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "05100000-0000-4000-8000-000000000001".into(),
                namespace: "com.test.rfc051".into(),
                srs_version: "2.0-draft".into(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "05100000-0000-4000-8000-000000000002".into(),
                namespace: "com.test.rfc051".into(),
                name: "primary".into(),
                version: "1.0.0".into(),
            },
        },
    )
    .unwrap();
    (tmp, store)
}

fn note(store: &FileStore, id: &str) -> String {
    let n: Note = serde_json::from_value(json!({
        "instanceId": id, "title": id, "sections": [{"name": "body", "content": "text"}]
    }))
    .unwrap();
    srs_repository::services::create_note(store, n)
        .unwrap()
        .note
        .instance_id
}

/// Instance ids in byte order: `id(1) < id(2) < ...`.
fn id(n: u32) -> String {
    format!("05100000-0000-4000-8000-0000000001{n:02x}")
}

const C: &str = "05100000-0000-4000-8000-0000000000c1";

fn container(store: &FileStore, cid: &str) {
    create_container(
        store,
        serde_json::from_value::<Container>(json!({"containerId": cid, "title": "Box"})).unwrap(),
    )
    .unwrap();
}

/// Every file of a container directory, by path, as text.
fn files(dir: &Path, cid: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for p in rd.map(|e| e.unwrap().path()) {
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                out.insert(rel, std::fs::read_to_string(&p).unwrap());
            }
        }
    }
    let base = dir.join(format!("containers/{cid}"));
    walk(&base, &base, &mut out);
    out
}

/// The paths whose content differs between two snapshots (added, changed or removed).
fn changed(a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) -> Vec<String> {
    let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .filter(|k| a.get(*k) != b.get(*k))
        .cloned()
        .collect()
}

fn entry(cid: &str, iid: &str, parent: Option<&str>, key: &str) -> String {
    let mut v = json!({"containerId": cid, "instanceId": iid, "key": key});
    if let Some(p) = parent {
        v["parent"] = json!(p);
    }
    line(&v)
}

fn outline(store: &FileStore, cid: &str) -> Vec<ContainerEntry> {
    get_container(store, cid)
        .unwrap()
        .member_instance_ids
        .unwrap_or_default()
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn catalog_codes(store: &FileStore) -> Vec<(String, String)> {
    store
        .catalog_unchecked()
        .unwrap()
        .diagnostics
        .iter()
        .map(|d| (d.code.to_string(), format!("{:?}", d.severity)))
        .collect()
}

// ── Write path ([R1], [R3], [R8], [R9]) ─────────────────────────────────────────────

#[test]
fn create_writes_the_head_and_appends_write_one_line_each_never_the_head() {
    let (tmp, store) = repo();
    container(&store, C);
    let d = tmp.path();
    let created = files(d, C);
    assert_eq!(created.keys().collect::<Vec<_>>(), ["container.json"]);
    let head = created["container.json"].clone();
    assert!(
        head.ends_with("}\n") && head.lines().count() == 1,
        "{head:?}"
    );

    let mut before = created;
    for (n, key) in [(1, "a0"), (2, "a1"), (3, "a2")] {
        let iid = note(&store, &id(n));
        add_member(&store, C, &iid, None, None).unwrap();
        let after = files(d, C);
        assert_eq!(changed(&before, &after), [format!("entries/{iid}.json")]);
        assert_eq!(
            after[&format!("entries/{iid}.json")],
            entry(C, &iid, None, key)
        );
        before = after;
    }
    assert_eq!(
        before["container.json"], head,
        "no member operation writes the head"
    );
}

#[test]
fn a_move_writes_only_the_moved_entry_and_a_remove_its_children() {
    let (tmp, store) = repo();
    container(&store, C);
    let (a, b, c) = (
        note(&store, &id(1)),
        note(&store, &id(2)),
        note(&store, &id(3)),
    );
    for i in [&a, &b, &c] {
        add_member(&store, C, i, None, None).unwrap();
    }
    let d = tmp.path();
    let before = files(d, C);
    move_member(&store, C, &c, Some(0), None).unwrap();
    let after = files(d, C);
    assert_eq!(changed(&before, &after), [format!("entries/{c}.json")]);
    assert_eq!(
        after[&format!("entries/{c}.json")],
        entry(C, &c, None, "Zz")
    );

    // b beneath a; removing a promotes b (rewritten without a parent) and removes a's file.
    move_member(&store, C, &b, None, Some(1)).unwrap();
    let before = files(d, C);
    let r = remove_member(&store, C, &a).unwrap();
    assert_eq!(r.promoted, std::slice::from_ref(&b));
    let after = files(d, C);
    assert_eq!(
        changed(&before, &after),
        [format!("entries/{a}.json"), format!("entries/{b}.json")]
    );
    assert!(!after.contains_key(&format!("entries/{a}.json")));
    assert!(!after[&format!("entries/{b}.json")].contains("parent"));
}

#[test]
fn a_record_delete_cascade_writes_exactly_the_write_set() {
    let (tmp, store) = repo();
    container(&store, C);
    let (a, b) = (note(&store, &id(1)), note(&store, &id(2)));
    add_member(&store, C, &a, None, None).unwrap();
    add_member(&store, C, &b, None, Some(1)).unwrap();
    let before = files(tmp.path(), C);
    srs_repository::services::delete_note(&store, &a, true).unwrap();
    let after = files(tmp.path(), C);
    assert_eq!(
        changed(&before, &after),
        [format!("entries/{a}.json"), format!("entries/{b}.json")]
    );
    assert_eq!(outline(&store, C), vec![ContainerEntry::new(&b)]);
}

#[test]
fn delete_removes_the_directory_and_copy_keeps_parent_and_key() {
    let (tmp, store) = repo();
    container(&store, C);
    let (a, b, c) = (
        note(&store, &id(1)),
        note(&store, &id(2)),
        note(&store, &id(3)),
    );
    for i in [&a, &b] {
        add_member(&store, C, i, None, None).unwrap();
    }
    add_member_relative(&store, C, &c, &a, Placement::After).unwrap(); // keyed a0V
    move_member(&store, C, &b, None, Some(1)).unwrap();
    let copy = copy_container(
        &store,
        C,
        ContainerCopyInput {
            title: None,
            container_id: Some("05100000-0000-4000-8000-0000000000c2".into()),
        },
    )
    .unwrap()
    .container;
    let src = files(tmp.path(), C);
    let dst = files(tmp.path(), &copy.container_id);
    for iid in [&a, &b, &c] {
        let k = format!("entries/{iid}.json");
        assert_eq!(
            dst[&k],
            src[&k].replace(C, &copy.container_id),
            "copy keeps parent and key"
        );
    }
    assert_eq!(outline(&store, &copy.container_id), outline(&store, C));
    delete_container(&store, &copy.container_id).unwrap();
    assert!(files(tmp.path(), &copy.container_id).is_empty());
}

/// [R8]'s worked example: `t` (a0) attached, orphans `b` and `o`. Inserting `n` after `t`
/// re-attaches b = a1 and o = a2, then keys n = a0V.
#[test]
fn an_operation_on_detached_entries_works_in_two_passes() {
    let (tmp, store) = repo();
    container(&store, C);
    let (t, b, o, n) = (
        note(&store, &id(1)),
        note(&store, &id(2)),
        note(&store, &id(3)),
        note(&store, &id(4)),
    );
    let gone = id(99);
    let d = tmp.path();
    write(d, &entry_path(C, &t), &entry(C, &t, None, "a0"));
    write(d, &entry_path(C, &b), &entry(C, &b, Some(&gone), "a0"));
    write(d, &entry_path(C, &o), &entry(C, &o, Some(&gone), "a0"));
    let store = FileStore::new(d);
    let before = files(d, C);
    add_member_relative(&store, C, &n, &t, Placement::After).unwrap();
    let after = files(d, C);
    assert_eq!(changed(&before, &after).len(), 3);
    for (iid, key) in [(&b, "a1"), (&o, "a2"), (&n, "a0V")] {
        assert_eq!(
            after[&format!("entries/{iid}.json")],
            entry(C, iid, None, key)
        );
    }
}

// ── Read path ([R2], [R5], [R6], [R11], [R12], [R14]) ───────────────────────────────

/// Orphans and loops are warnings, decoded deterministically ([R5]); repair re-attaches the
/// detached entries ([R11]), never removing one.
#[test]
fn orphans_and_loops_are_warnings_and_repair_reattaches_them() {
    let (tmp, store) = repo();
    container(&store, C);
    let ids: Vec<String> = (1..=5).map(|n| note(&store, &id(n))).collect();
    let (top, orphan, x, y, tail) = (&ids[0], &ids[1], &ids[2], &ids[3], &ids[4]);
    let d = tmp.path();
    write(d, &entry_path(C, top), &entry(C, top, None, "a0"));
    write(
        d,
        &entry_path(C, orphan),
        &entry(C, orphan, Some(&id(99)), "a0"),
    );
    // A loop x <-> y with a tail into it.
    write(d, &entry_path(C, x), &entry(C, x, Some(y), "a0"));
    write(d, &entry_path(C, y), &entry(C, y, Some(x), "a0"));
    write(d, &entry_path(C, tail), &entry(C, tail, Some(x), "a1"));
    let store = FileStore::new(d);
    let codes = catalog_codes(&store);
    assert!(
        codes.contains(&("arrangement-orphan".into(), "Warning".into())),
        "{codes:?}"
    );
    assert!(
        codes.contains(&("arrangement-cycle".into(), "Warning".into())),
        "{codes:?}"
    );
    assert!(store.catalog().is_ok(), "warnings are not fatal");
    // Top: the attached entry, then the orphan and the break (x, the lowest id on the loop),
    // in sibling order; y and the tail beneath x.
    assert_eq!(
        outline(&store, C),
        vec![
            ContainerEntry::new(top),
            ContainerEntry::new(orphan),
            ContainerEntry::new(x),
            ContainerEntry::at(y, 1),
            ContainerEntry::at(tail, 1),
        ]
    );
    let r = repair_members(&store, C).unwrap();
    assert_eq!(r.reattached, [orphan.clone(), x.clone()]);
    assert!(r.removed.is_empty());
    let f = files(d, C);
    assert_eq!(
        f[&format!("entries/{orphan}.json")],
        entry(C, orphan, None, "a1")
    );
    assert_eq!(f[&format!("entries/{x}.json")], entry(C, x, None, "a2"));
    assert!(catalog_codes(&store)
        .iter()
        .all(|(c, _)| !c.starts_with("arrangement-")));
}

/// A loop with no attached entry: only the break is re-attached, keyed `a0`.
#[test]
fn a_loop_with_no_attached_entry_reattaches_only_its_break() {
    let (tmp, store) = repo();
    container(&store, C);
    let (x, y) = (note(&store, &id(1)), note(&store, &id(2)));
    let d = tmp.path();
    write(d, &entry_path(C, &x), &entry(C, &x, Some(&y), "a3"));
    write(d, &entry_path(C, &y), &entry(C, &y, Some(&x), "a5"));
    let store = FileStore::new(d);
    let before = files(d, C);
    assert_eq!(
        repair_members(&store, C).unwrap().reattached,
        std::slice::from_ref(&x)
    );
    let after = files(d, C);
    assert_eq!(changed(&before, &after), [format!("entries/{x}.json")]);
    assert_eq!(
        after[&format!("entries/{x}.json")],
        entry(C, &x, None, "a0")
    );
}

/// [R14]: a directory with entries and no head is a warning and ignored; repair removes it.
#[test]
fn a_headless_directory_is_ignored_and_repair_removes_it() {
    let (tmp, store) = repo();
    let a = note(&store, &id(1));
    let gone = "05100000-0000-4000-8000-0000000000c9";
    write(
        tmp.path(),
        &entry_path(gone, &a),
        &entry(gone, &a, None, "a0"),
    );
    let store = FileStore::new(tmp.path());
    assert!(catalog_codes(&store).contains(&("container-head-missing".into(), "Warning".into())));
    assert!(store
        .catalog()
        .unwrap()
        .containers
        .iter()
        .all(|e| e.id != gone));
    assert_eq!(repair_members(&store, gone).unwrap().removed, [a]);
    assert!(files(tmp.path(), gone).is_empty());
}

/// [R2]/[R4]/[R12]/[R13]: each storage defect is reported with its own code, fatally, in the
/// directory reader and in the `.srsj` and `.srs` readers alike (all read the same tree).
#[test]
fn storage_defects_are_fatal_with_their_codes_on_disk_and_in_srsj() {
    let (tmp, store) = repo();
    container(&store, C);
    let a = note(&store, &id(1));
    add_member(&store, C, &a, None, None).unwrap();
    let srsj = srs_repository::srsj::to_srsj_string(&store).unwrap();
    const OTHER: &str = "05100000-0000-4000-8000-0000000000c3";
    type Mutation = Box<dyn Fn(&mut serde_json::Map<String, Value>)>;
    let cases: Vec<(&str, Mutation)> = vec![
        (
            "container-stray-file",
            Box::new(|d| {
                d.insert(format!("containers/{C}/notes.json"), json!({}));
            }),
        ),
        // Two files claiming one member: the second is named by another id.
        (
            "container-name-mismatch",
            Box::new(move |d| {
                let v = d[&entry_path(C, &id(1))].clone();
                d.insert(entry_path(C, &id(7)), v);
            }),
        ),
        (
            "container-entry-misplaced",
            Box::new(move |d| {
                d.insert(
                    entry_path(C, &id(1)),
                    json!({"containerId": OTHER, "instanceId": id(1), "key": "a0"}),
                );
            }),
        ),
        (
            "arrangement-key",
            Box::new(|d| {
                d.insert(
                    entry_path(C, &id(1)),
                    json!({"containerId": C, "instanceId": id(1), "key": "a10"}),
                );
            }),
        ),
        (
            "container-schema",
            Box::new(|d| {
                let mut h = d[&head_path(C)].clone();
                h["memberInstanceIds"] = json!([]);
                d.insert(head_path(C), h);
            }),
        ),
        (
            "container-migration-needed",
            Box::new(|d| {
                d.insert(
                    "containers/flat.json".into(),
                    json!({"containerId": OTHER, "title": "Flat"}),
                );
            }),
        ),
    ];
    for (code, mutate) in cases {
        let mut doc: Value = serde_json::from_str(&srsj).unwrap();
        mutate(doc["data"].as_object_mut().unwrap());
        if code == "container-migration-needed" {
            // [R13]: the pre-load transformer converts a flat file in memory (and says so);
            // on disk the same file is refused.
            let opened = srs_repository::srsj::open_srsj(&doc.to_string()).unwrap();
            assert!(opened.migration_notice().is_some());
            assert!(opened.catalog().is_ok());
            let disk = TempDir::new().unwrap();
            let raw = srs_repository::srsj::open_srsj_for_migration(&doc.to_string()).unwrap();
            for (path, bytes) in srs_repository::tree_session::export_tree(&raw).unwrap() {
                let p = disk.path().join(path);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, bytes).unwrap();
            }
            let err = FileStore::new(disk.path())
                .catalog()
                .unwrap_err()
                .to_string();
            assert!(err.contains(code), "disk {code}: {err}");
            continue;
        }
        let opened = srs_repository::srsj::open_srsj(&doc.to_string()).unwrap();
        let err = opened.catalog().unwrap_err().to_string();
        assert!(err.contains(code), "srsj {code}: {err}");
        // The same tree on disk.
        let disk = TempDir::new().unwrap();
        for (path, bytes) in srs_repository::tree_session::export_tree(&opened).unwrap() {
            let p = disk.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        }
        let err = FileStore::new(disk.path())
            .catalog()
            .unwrap_err()
            .to_string();
        assert!(err.contains(code), "disk {code}: {err}");
        // And packed as a `.srs`.
        let mut buf = std::io::Cursor::new(Vec::new());
        srs_repository::archive_pack(&FileStore::new(disk.path()), &mut buf).unwrap();
        let srs = srs_repository::archive::archive_to_tree(std::io::Cursor::new(buf.into_inner()))
            .unwrap();
        let err = srs.catalog().unwrap_err().to_string();
        assert!(err.contains(code), "srs {code}: {err}");
    }
    drop(tmp);
}

/// A pointer naming no entry is RFC-043 [R3]'s validation error, not a load failure.
#[test]
fn a_dangling_pointer_is_a_validation_error_not_fatal() {
    let (tmp, store) = repo();
    container(&store, C);
    let a = note(&store, &id(1));
    add_member(&store, C, &a, None, None).unwrap();
    let hp = tmp.path().join(head_path(C));
    let mut head: Value = serde_json::from_str(&std::fs::read_to_string(&hp).unwrap()).unwrap();
    head["anchorInstanceId"] = json!(id(2));
    std::fs::write(&hp, line(&head)).unwrap();
    let store = FileStore::new(tmp.path());
    assert!(store.catalog().is_ok());
    let report = srs_repository::validation::validate_repository(&store).unwrap();
    assert!(report.summary.errors > 0, "{:?}", report.diagnostics);
}

// ── Merging with plain git ([R10]) ──────────────────────────────────────────────────

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git must be runnable")
}

fn ok(dir: &Path, args: &[&str]) {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repository with container C, committed on `main`, and `members` notes made on `main`.
fn base(members: u32) -> (TempDir, Vec<String>) {
    let (tmp, store) = repo();
    container(&store, C);
    let ids: Vec<String> = (1..=members).map(|n| note(&store, &id(n))).collect();
    let d = tmp.path();
    ok(d, &["init", "-q", "-b", "main"]);
    ok(d, &["add", "-A"]);
    ok(d, &["commit", "-q", "-m", "base", "--no-gpg-sign"]);
    (tmp, ids)
}

fn on_branch(d: &Path, branch: &str, from: &str, edit: impl FnOnce(&FileStore)) {
    ok(d, &["checkout", "-q", "-b", branch, from]);
    edit(&FileStore::new(d));
    ok(d, &["add", "-A"]);
    ok(d, &["commit", "-q", "-m", branch, "--no-gpg-sign"]);
}

fn merge(d: &Path, into: &str, from: &str) -> bool {
    ok(d, &["checkout", "-q", into]);
    git(d, &["merge", "-q", "--no-edit", "--no-gpg-sign", from])
        .status
        .success()
}

/// The acceptance test: two writers each append a member to the same container; plain git
/// merges cleanly and the result validates with both members.
#[test]
fn two_writers_appending_to_one_container_merge_cleanly() {
    let (tmp, ids) = base(2);
    let d = tmp.path();
    on_branch(d, "one", "main", |s| {
        add_member(s, C, &ids[0], None, None).unwrap();
    });
    on_branch(d, "two", "main", |s| {
        add_member(s, C, &ids[1], None, None).unwrap();
    });
    assert!(merge(d, "one", "two"), "the merge must be clean");
    let store = FileStore::new(d);
    let report = srs_repository::validation::validate_repository(&store).unwrap();
    assert_eq!(report.summary.errors, 0, "{:?}", report.diagnostics);
    // Equal keys; the instanceId tie-break orders them.
    assert_eq!(
        outline(&store, C),
        vec![ContainerEntry::new(&ids[0]), ContainerEntry::new(&ids[1])]
    );
}

/// [R10]: the same entry added with different values on both sides, and an entry removed on
/// one side and rewritten on the other, are visible conflicts; one-line files never pair as
/// a rename that would misapply a change.
#[test]
fn add_add_and_delete_modify_are_visible_conflicts() {
    let (tmp, ids) = base(3);
    let d = tmp.path();
    on_branch(d, "one", "main", |s| {
        add_member(s, C, &ids[0], None, None).unwrap();
    });
    on_branch(d, "two", "main", |s| {
        add_member(s, C, &ids[1], None, None).unwrap();
        add_member(s, C, &ids[0], Some(0), None).unwrap();
    });
    assert!(
        !merge(d, "one", "two"),
        "add/add with different keys conflicts"
    );
    ok(d, &["merge", "--abort"]);

    on_branch(d, "base2", "main", |s| {
        add_member(s, C, &ids[0], None, None).unwrap();
        add_member(s, C, &ids[1], None, None).unwrap();
    });
    on_branch(d, "drop", "base2", |s| {
        remove_member(s, C, &ids[0]).unwrap();
        add_member(s, C, &ids[2], None, None).unwrap();
    });
    on_branch(d, "moved", "base2", |s| {
        move_member(s, C, &ids[0], Some(1), None).unwrap();
    });
    assert!(
        !merge(d, "drop", "moved"),
        "delete against modify conflicts"
    );
}

/// Review finding 1: a member whose id cannot name a file (a hand-written record with a
/// path-like or non-canonical id) is refused before any write, and the repository stays
/// usable.
#[test]
fn a_member_id_that_cannot_name_a_file_is_refused_before_any_write() {
    let (tmp, store) = repo();
    container(&store, C);
    let d = tmp.path();
    for bad in ["../../../../escaped", "NOT-A-UUID"] {
        write(
            d,
            &format!("records/notes/bad-{}.json", bad.len()),
            &json!({"instanceId": bad, "title": "bad", "sections": [{"name": "b", "content": "x"}]})
                .to_string(),
        );
        let store = FileStore::new(d);
        let before = files(d, C);
        let err = add_member(&store, C, bad, None, None).unwrap_err();
        assert_eq!(err.code(), "container-name-mismatch", "{bad}: {err}");
        assert_eq!(files(d, C), before);
        assert!(!d.join("../escaped.json").exists());
        assert!(store.catalog().is_ok(), "{bad}: the repository still loads");
    }
}
