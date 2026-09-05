//! `repo copy` round trip at the binary's current data-model revision, with a
//! real sub-package (srs-rust#934, srs#256/#242 standing check).
//!
//! srs#256's scoping comment (2026-09-04) reproduced a real defect against
//! the `srs` spec corpus at revision 7: `repo copy` (file -> `.srsj`) flattened
//! every sub-package's fields/types/compositions into the primary package on
//! export, so re-importing the bundle failed with 446
//! `SRS038-R12-DUPLICATE-ID` diagnostics — the same definition claimed by two
//! objects. Root cause: `export_package_boundary`'s primary-boundary branch
//! called `RepositoryStore::load_package()`, which *intentionally* merges
//! every `manifest.packageRefs` sub-package into one resolved view for
//! validation/type-resolution consumers, instead of reading the primary
//! boundary's own `package.json` in isolation like every other boundary.
//!
//! Revision-independent by construction: the fixture below is built at
//! whatever `CURRENT_DATA_MODEL_REVISION` currently is, so this test does not
//! go stale the next time a data-model migration lands (unlike the one-shot
//! srs#256 audit it replaces).

use srs_repository::catalog;
use srs_repository::field_type_migration_service::CURRENT_DATA_MODEL_REVISION;
use srs_repository::repository_portability::copy_repository;
use srs_repository::srsj::open_srsj;
use srs_repository::store::{FileStore, RepositoryStore};

const SUB_FIELD_ID: &str = "00000000-0000-4000-8000-0000000000f1";
const PRIMARY_FIELD_ID: &str = "00000000-0000-4000-8000-0000000000f0";

/// A minimal repository at `CURRENT_DATA_MODEL_REVISION` with two package
/// boundaries: the implicit primary (`package/package.json`) and one
/// explicit sub-package declared via `manifest.packageRefs` (`package/sub`),
/// matching the multi-root layout `srs/srs` actually uses (base/core/
/// spec-authoring-core/... — RFC-014 "two origins"). Each boundary declares
/// exactly one field it alone owns, so any flattening across boundaries
/// shows up as a literal duplicate id.
fn current_revision_source_with_subpackage() -> FileStore {
    open_srsj(
        &serde_json::json!({
            "srsj": "2",
            "manifest": {
                "$schema": srs_schema::MANIFEST_SCHEMA_ID,
                "srsVersion": "2.0-draft",
                "dataModelRevision": CURRENT_DATA_MODEL_REVISION,
                "repositoryId": "00000000-0000-4000-8000-00000000dddd",
                "namespace": "com.semanticops.roundtrip",
                "title": "Package Boundary Roundtrip Fixture",
                "createdAt": "2026-01-01T00:00:00Z",
                "container": {
                    "containerId": "00000000-0000-4000-8000-00000000eeee",
                    "title": "Package Boundary Roundtrip Fixture",
                },
                "packageRefs": [
                    { "mode": "local", "path": "package/sub" },
                ],
            },
            "data": {
                "package/package.json": {
                    "$schema": srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
                    "id": "00000000-0000-4000-8000-00000000a0a0",
                    "namespace": "com.semanticops.roundtrip",
                    "name": "primary",
                    "version": "1.0.0",
                    "title": "Primary Package",
                    "description": "Primary boundary — owns exactly one field.",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "fields": ["fields/primary.json"],
                    "types": [],
                },
                "package/fields/primary.json": {
                    "$schema": srs_schema::FIELD_SCHEMA_ID,
                    "id": PRIMARY_FIELD_ID,
                    "namespace": "com.semanticops.roundtrip",
                    "name": "primary_field",
                    "version": 1,
                    "description": "The primary boundary's own field.",
                    "aiGuidance": { "purpose": "Fixture field." },
                    "createdAt": "2026-01-01T00:00:00Z",
                    "fieldType": { "datatype": "string", "format": "plain" },
                },
                "package/sub/package.json": {
                    "$schema": srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
                    "id": "00000000-0000-4000-8000-00000000b0b0",
                    "namespace": "com.semanticops.roundtrip.sub",
                    "name": "sub",
                    "version": "1.0.0",
                    "title": "Sub Package",
                    "description": "Sub-package boundary — owns exactly one field, distinct \
                                     from the primary's.",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "fields": ["fields/sub.json"],
                    "types": [],
                },
                "package/sub/fields/sub.json": {
                    "$schema": srs_schema::FIELD_SCHEMA_ID,
                    "id": SUB_FIELD_ID,
                    "namespace": "com.semanticops.roundtrip.sub",
                    "name": "sub_field",
                    "version": 1,
                    "description": "The sub-package boundary's own field.",
                    "aiGuidance": { "purpose": "Fixture field." },
                    "createdAt": "2026-01-01T00:00:00Z",
                    "fieldType": { "datatype": "string", "format": "plain" },
                },
            },
        })
        .to_string(),
    )
    .unwrap()
}

fn assert_no_duplicate_ids(store: &dyn RepositoryStore, label: &str) {
    match catalog::build_checked(store) {
        Ok(_) => {}
        Err(e) => panic!("{label}: catalog must load with no duplicate/fatal diagnostics: {e}"),
    }
}

/// Each package boundary keeps its one on-disk home: the primary's
/// `package.json` must never re-declare the sub-package's field (it may
/// legitimately carry its own field plus any core-injected built-ins that no
/// sub-package claims — this only asserts against the srs-rust#934
/// flattening, not against core injection).
fn assert_one_home_per_definition(store: &dyn RepositoryStore) {
    let primary_pkg = store.load_instance_json("package/package.json").unwrap();
    let primary_fields: Vec<String> = primary_pkg["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        primary_fields.iter().any(|p| p.contains("primary")),
        "the primary boundary must keep its own field: {primary_fields:?}"
    );
    assert!(
        !primary_fields.iter().any(|p| p.contains("sub")),
        "the sub-package's field must not have been flattened into the primary boundary: {primary_fields:?}"
    );
}

#[test]
fn repo_copy_round_trip_preserves_package_boundaries_at_current_revision() {
    let source = current_revision_source_with_subpackage();
    assert_no_duplicate_ids(&source, "source");

    // file -> .srsj (export): the exact step srs#256 found broken.
    let exported = srs_repository::tree_session::new_tree_session();
    copy_repository(&source, &exported).expect("export must succeed");
    assert_one_home_per_definition(&exported);
    assert_no_duplicate_ids(&exported, "exported");

    // .srsj -> file (reimport): srs#256 saw this fail with 446
    // SRS038-R12-DUPLICATE-ID diagnostics; the catalog must now load clean.
    let reimported = srs_repository::tree_session::new_tree_session();
    copy_repository(&exported, &reimported).expect("reimport must succeed");
    assert_one_home_per_definition(&reimported);
    assert_no_duplicate_ids(&reimported, "reimported");

    // Both definitions must still be present exactly once each, under their
    // own boundary — not merely "no duplicates" but "nothing lost" either.
    let sub_pkg = reimported
        .load_instance_json("package/sub/package.json")
        .expect("sub-package boundary must survive the round trip");
    let sub_fields: Vec<String> = sub_pkg["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        sub_fields.len(),
        1,
        "sub-package must keep its own field: {sub_fields:?}"
    );
}
