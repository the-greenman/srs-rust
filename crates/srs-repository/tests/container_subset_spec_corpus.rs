//! Corpus-level proof for the container-subset render defect (srs#682/#693,
//! srs-rust "render: container-subset renders subtree members twice").
//!
//! PR srs#693 wired nine `container-subset` sections (one per RFC-042 Part
//! container) into `spec-document-view`/`unified-document-view` and the
//! render went from 357 headings to 1040 across only 304 unique heading
//! texts. That PR is itself BLOCKED on this fix landing here, so it has not
//! merged — `origin/master`'s `spec-document-view` still sources only
//! `discovery-query` sections, and (as this test's own discovery run showed)
//! the *whole document*'s heading texts already legitimately repeat in
//! places unrelated to this bug (shared subsection labels like "Example"),
//! so a raw whole-document `total == unique` assertion is not a sound proxy.
//!
//! Instead this test reproduces the defect directly against real corpus data:
//! it registers one throwaway `container-subset` composition (in a scratch
//! copy of the spec repo, never committed) sourced from the real RFC-042
//! `Part: Foundations` container — whose declared membership genuinely is
//! the whole `contains` subtree, exactly as PR srs#693 described — and
//! asserts every member renders exactly once.
//!
//! Skipped when the spec repo is not present — same convention as
//! `core_bundle_drift.rs` / `rfc_035_parity.rs`. Prefer `SRS_SPEC_DIR` pointed
//! at a fresh `origin/master` clone (never a long-lived sibling — srs-rust#874).

use srs_repository::render_service::{render_composition, RenderCompositionOptions};
use srs_repository::store::FileStore;
use srs_repository::RepositoryStore;
use std::path::PathBuf;

const FOUNDATIONS_PART_CONTAINER_ID: &str = "752dad23-8a6d-44e5-98c9-f081d2cc634e";
const SPEC_HEADING_FIELD_ID: &str = "1a000001-0000-4000-a000-000000000001";
const TEST_COMPOSITION_ID: &str = "3a000099-0000-4000-a000-000000000099";

/// The spec repo checkout, or `None` when it is not available. Mirrors
/// `rfc_035_parity.rs`'s `spec_repo()` / `core_bundle_drift.rs`'s inline
/// equivalent: `SRS_SPEC_DIR` first (CI, and any local run), a sibling
/// checkout as a loud (never silently stale) fallback.
fn spec_repo() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SRS_SPEC_DIR") {
        let p = PathBuf::from(dir);
        if p.join("srs/manifest.json").is_file() {
            return Some(p);
        }
        return None; // an explicit but unusable SRS_SPEC_DIR: skip, don't silently fall through
    }
    let sibling = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../srs");
    if !sibling.join("srs/manifest.json").is_file() {
        return None;
    }
    if let Ok(out) = std::process::Command::new("git")
        .args([
            "-C",
            sibling.to_str().unwrap_or("."),
            "rev-parse",
            "--abbrev-ref",
            "HEAD",
        ])
        .output()
    {
        if out.status.success() {
            let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if branch != "master" {
                panic!(
                    "srs sibling checkout at {} is on branch '{branch}', not master — its render \
                     may be stale (srs-rust#874's false-green trap). Set SRS_SPEC_DIR to a fresh \
                     `origin/master` checkout instead.",
                    sibling.display()
                );
            }
        }
    }
    Some(sibling)
}

/// Copy the spec repo's `srs/` SRS repository into a scratch temp dir, then
/// register one throwaway `container-subset` composition sourced from the
/// real `Part: Foundations` container (`scripts/part-container-membership.mjs`
/// keeps its declared membership equal to its `contains` subtree, exactly the
/// RFC-042 shape PR srs#693 reported). Nothing here is committed back to the
/// spec repo.
fn scratch_store_with_test_composition(spec: &std::path::Path) -> (tempfile::TempDir, FileStore) {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let dest = tmp.path().join("srs");
    let status = std::process::Command::new("cp")
        .args([
            "-r",
            spec.join("srs").to_str().unwrap(),
            dest.to_str().unwrap(),
        ])
        .status()
        .expect("cp must run");
    assert!(
        status.success(),
        "copying the spec repo's srs/ tree must succeed"
    );

    let subpkg_path = dest.join("package/spec-authoring-core/package.json");
    let mut subpkg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&subpkg_path).unwrap()).unwrap();
    subpkg["compositions"]
        .as_array_mut()
        .expect("compositions must be an array")
        .push(serde_json::json!(
            "compositions/test-container-subset-foundations.json"
        ));
    std::fs::write(&subpkg_path, serde_json::to_string_pretty(&subpkg).unwrap()).unwrap();

    let composition = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/composition.json",
        "id": TEST_COMPOSITION_ID,
        "namespace": "com.semanticops.spec",
        "name": "test-container-subset-foundations",
        "version": 1,
        "description": "Test-only composition reproducing srs#682/#693 against the real Foundations Part container.",
        "sections": [{
            "sectionId": "body",
            "title": "Foundations (test)",
            "order": 0,
            "source": {
                "type": "container-subset",
                "containerId": FOUNDATIONS_PART_CONTAINER_ID,
            },
            "titleFieldId": SPEC_HEADING_FIELD_ID,
            "emptyBehavior": "hide",
        }],
        "exportConfig": { "format": "markdown" },
        "createdAt": "2026-01-01T00:00:00Z",
    });
    std::fs::write(
        dest.join(
            "package/spec-authoring-core/compositions/test-container-subset-foundations.json",
        ),
        serde_json::to_string_pretty(&composition).unwrap(),
    )
    .unwrap();

    let store = FileStore::new(dest);
    (tmp, store)
}

#[test]
fn container_subset_renders_real_part_container_members_exactly_once() {
    let Some(spec) = spec_repo() else {
        eprintln!("skipping: spec repo not found (set SRS_SPEC_DIR)");
        return;
    };
    let (_tmp, store) = scratch_store_with_test_composition(&spec);

    let package = store.load_package().expect("package must load");
    let container =
        srs_repository::container_service::get_container(&store, FOUNDATIONS_PART_CONTAINER_ID)
            .expect("Part: Foundations container must load");
    let member_ids = container
        .member_instance_ids
        .clone()
        .expect("Part: Foundations must declare memberInstanceIds");
    assert!(
        member_ids.len() > 1,
        "fixture assumption: the real Foundations Part container must have more than one \
         declared member for this test to be meaningful"
    );

    let result = render_composition(RenderCompositionOptions {
        store: &store,
        view_id: TEST_COMPOSITION_ID,
        format: Some("markdown"),
        theme_variant: None,
        container_id: None,
        instance_id_filter: None,
    })
    .expect("test composition must render");

    let heading_field = package
        .fields
        .iter()
        .find(|f| f.id == SPEC_HEADING_FIELD_ID)
        .expect("spec heading field must resolve");

    // Every declared member's own heading-field value must appear in the
    // render exactly as many times as there are DISTINCT members declaring
    // that same heading text — the container's full `contains` subtree
    // renders each member once each, at whatever depth `contains` recursion
    // puts it, not once per section-loop entry AND again via an ancestor's
    // recursion. A flat `== 1` check is unsound here: real corpus content can
    // legitimately have two different members share a title (e.g. a legacy
    // "Type" subsection stub and the RFC-042 "Type" concept it points readers
    // to via `derived-from` — two distinct records, two distinct headings,
    // same text) without either one being double-rendered. Comparing actual
    // occurrences against the expected multiset (count of distinct member IDs
    // per heading text) still catches genuine duplication — where a single
    // member's own heading renders more times than the number of members that
    // declare it — while tolerating legitimate same-titled distinct members.
    let mut expected_counts: std::collections::HashMap<String, usize> = Default::default();
    let mut checked_any = 0;
    for id in &member_ids {
        let Some(instance) = srs_repository::record_store::get_instance_by_id(&store, id)
            .expect("instance lookup must not error")
        else {
            continue;
        };
        let Some(record) = instance.as_record() else {
            continue; // Tier-0 notes aren't rendered as headings
        };
        let Some(heading) = record.value_str(&heading_field.name) else {
            continue;
        };
        if heading.trim().is_empty() {
            continue;
        }
        checked_any += 1;
        *expected_counts.entry(heading.to_string()).or_insert(0) += 1;
    }
    for (heading, expected) in &expected_counts {
        // Count only markdown HEADING lines matching this heading text, not
        // arbitrary substring occurrences in body prose (a record's title may
        // legitimately appear as prose elsewhere, e.g. "...Field, Type,
        // Vocabulary and Term, record tiers..." in an unrelated description) —
        // that would false-positive this assertion on real spec content.
        let count = result
            .rendered
            .lines()
            .filter(|line| line.trim_start_matches('#').trim() == heading)
            .filter(|line| line.starts_with('#'))
            .count();
        assert_eq!(
            count, *expected,
            "heading {heading:?} is declared by {expected} distinct container member(s) \
             but rendered {count} times in the container-subset section (srs#682/#693's \
             defect: the section loop must render only subtree roots, letting `contains` \
             recursion render every descendant exactly once)"
        );
    }
    assert!(
        checked_any > 5,
        "fixture assumption: expected to check headings for more than a handful of the \
         Foundations Part's real members; only checked {checked_any}"
    );
}
