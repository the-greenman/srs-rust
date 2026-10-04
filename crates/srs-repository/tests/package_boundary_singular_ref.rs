//! srs-rust#1225: `package_dependency_service::installed_set` falls back to
//! the singular manifest `packageRef` when `packageRefs` is absent, but
//! `FileStore::list_package_boundaries` and `FileStore::load_package` read
//! only the plural `packageRefs` — so a sub-package declared via the
//! (schema-current, non-deprecated) singular `packageRef` is "installed" for
//! RFC-044 dependency checks while being entirely invisible to the catalog:
//! its fields and types never load, and it never appears as a boundary.
//!
//! This reproduces the gap and proves the fix: all three readers must agree
//! on the declared package refs (`Manifest::declared_package_refs`, the one
//! canonical resolution).

use srs_repository::package_dependency_service::installed_set;
use srs_repository::srsj::open_srsj;
use srs_repository::store::RepositoryStore;

const PRIMARY_ID: &str = "00000000-0000-4000-8000-00000000a0a0";
const SUB_ID: &str = "00000000-0000-4000-8000-00000000b0b0";
const SUB_FIELD_ID: &str = "00000000-0000-4000-8000-0000000000f1";

/// A repository that declares its one sub-package via the singular
/// `packageRef` (not the plural `packageRefs` array) — a current, schema-legal
/// shape (`manifest.json`'s `packageRef` property is not deprecated) that
/// predates RFC-014 Change C's multi-package `packageRefs`.
fn repo_with_singular_package_ref() -> impl RepositoryStore {
    open_srsj(
        &serde_json::json!({
            "srsj": "2",
            "manifest": {
                "$schema": srs_schema::MANIFEST_SCHEMA_ID,
                "srsVersion": "2.0-draft",
                "dataModelRevision": srs_repository::field_type_migration_service::CURRENT_DATA_MODEL_REVISION,
                "repositoryId": "00000000-0000-4000-8000-00000000dddd",
                "namespace": "com.semanticops.singularref",
                "title": "Singular packageRef fixture",
                "createdAt": "2026-01-01T00:00:00Z",
                "container": {
                    "containerId": "00000000-0000-4000-8000-00000000eeee",
                    "title": "Singular packageRef fixture",
                },
                "packageRef": { "mode": "local", "path": "package/sub" },
            },
            "data": {
                "package/package.json": {
                    "$schema": srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
                    "id": PRIMARY_ID,
                    "namespace": "com.semanticops.singularref",
                    "name": "primary",
                    "version": "1.0.0",
                    "title": "Primary",
                    "description": "",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "fields": [],
                    "types": [],
                },
                "package/sub/package.json": {
                    "$schema": srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
                    "id": SUB_ID,
                    "namespace": "com.semanticops.singularref.sub",
                    "name": "sub",
                    "version": "1.0.0",
                    "title": "Sub",
                    "description": "",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "fields": ["fields/sub.json"],
                    "types": [],
                },
                "package/sub/fields/sub.json": {
                    "$schema": srs_schema::FIELD_SCHEMA_ID,
                    "id": SUB_FIELD_ID,
                    "namespace": "com.semanticops.singularref.sub",
                    "name": "sub_field",
                    "version": 1,
                    "description": "The sub-package's own field.",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "fieldType": { "datatype": "string", "format": "plain" },
                },
            },
        })
        .to_string(),
    )
    .unwrap()
}

#[test]
fn installed_set_includes_singular_package_ref_subpackage() {
    let store = repo_with_singular_package_ref();
    let ids: Vec<String> = installed_set(&store)
        .unwrap()
        .into_iter()
        .map(|m| m.package.id)
        .collect();
    assert!(
        ids.contains(&SUB_ID.to_string()),
        "installed_set must include the singular-packageRef sub-package: {ids:?}"
    );
}

#[test]
fn list_package_boundaries_includes_singular_package_ref_subpackage() {
    let store = repo_with_singular_package_ref();
    let boundaries = store.list_package_boundaries().unwrap();
    let has_sub = boundaries
        .iter()
        .any(|b| b.id == SUB_ID && b.selector == Some("package/sub".to_string()));
    assert!(
        has_sub,
        "list_package_boundaries must include the singular-packageRef sub-package boundary: {:?}",
        boundaries
            .iter()
            .map(|b| (&b.id, &b.selector))
            .collect::<Vec<_>>()
    );
}

#[test]
fn load_package_merges_singular_package_ref_subpackage_fields() {
    let store = repo_with_singular_package_ref();
    let package = store.load_package().unwrap();
    assert!(
        package.fields.iter().any(|f| f.id == SUB_FIELD_ID),
        "load_package must merge the singular-packageRef sub-package's field into the \
         catalog, not just the installed set: {:?}",
        package.fields.iter().map(|f| &f.id).collect::<Vec<_>>()
    );
}
