//! RFC-044 [R7]/[R10] (srs-rust#1173): Inv 43 (`ext:type-inheritance`) must never decide
//! cross-package base-type coverage from `namespace`/`name`, and — per the owner ruling
//! on #1173 (2026-10-10, Option A) — must report nothing for an unresolved cross-package
//! base type until real cross-package type-catalogue resolution exists (tracked as a
//! follow-up, srs-rust#1388, blocked on package distribution).

use serde_json::json;
use srs_repository::field_type_migration_service::CURRENT_DATA_MODEL_REVISION;
use srs_repository::srsj::open_srsj;
use srs_repository::validation::validate_repository;

const CHILD_TYPE_ID: &str = "e1000001-0000-4000-a000-000000000001";
const BASE_TYPE_ID: &str = "e1000002-0000-4000-a000-000000000002";
const RECORD_ID: &str = "e1000003-0000-4000-a000-000000000003";

/// A single-package repo with one Tier-2 record (needed to trigger package load for
/// Inv 43) of a type that `extends` a base type id not defined anywhere in the package,
/// simulating an unresolved cross-package base reference. `deps` is the package's
/// `packageDependencies` array.
fn repo(deps: serde_json::Value) -> srs_repository::store::FileStore {
    let srsj = json!({
        "srsj": "2",
        "manifest": {
            "$schema": srs_schema::MANIFEST_SCHEMA_ID,
            "srsVersion": "2.0-draft",
            "dataModelRevision": CURRENT_DATA_MODEL_REVISION,
            "repositoryId": "00000000-0000-4000-8000-00000000f0f0",
            "namespace": "com.example.inv43",
            "title": "Inv 43 fixture",
            "createdAt": "2026-01-01T00:00:00Z",
            "container": {
                "containerId": "00000000-0000-4000-8000-00000000f1f1",
                "title": "Inv 43 fixture",
            },
        },
        "data": {
            "package/package.json": {
                "$schema": srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
                "id": "00000000-0000-4000-8000-00000000f2f2",
                "namespace": "com.example.inv43",
                "name": "primary",
                "version": "1.0.0",
                "title": "primary",
                "description": "",
                "status": "active",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": [],
                "types": ["types/child.json"],
                "packageDependencies": deps,
            },
            "package/types/child.json": {
                "id": CHILD_TYPE_ID,
                "namespace": "com.example.inv43",
                "name": "child",
                "version": 1,
                "description": "extends an unresolved cross-package base type",
                "fields": [],
                "extendsTypeId": BASE_TYPE_ID,
                "extendsTypeVersion": 1,
                "createdAt": "2026-01-01T00:00:00Z",
            },
            "records/tier-2/child.json": {
                "instanceId": RECORD_ID,
                "typeId": CHILD_TYPE_ID,
                "typeVersion": 1,
                "typeNamespace": "com.example.inv43",
                "typeName": "child",
                "fieldValues": {},
            },
        },
    });
    open_srsj(&srsj.to_string()).unwrap()
}

fn type_inheritance_diagnostics(store: &dyn srs_repository::store::RepositoryStore) -> Vec<String> {
    validate_repository(store)
        .unwrap()
        .diagnostics
        .into_iter()
        .filter(|d| d.message.contains("ext:type-inheritance") || d.message.contains("Inv 43"))
        .map(|d| d.message)
        .collect()
}

#[test]
fn no_dependency_declared_emits_no_inv43_diagnostic() {
    let store = repo(json!([]));
    let diags = type_inheritance_diagnostics(&store);
    assert!(diags.is_empty(), "expected no Inv 43 diagnostics, got {diags:?}");
}

/// Reproduces the exact pre-fix bug condition: a `packageDependencies` entry whose
/// `namespace` happens to equal the specializing type's own namespace, but whose
/// `packageId` names a package wholly unrelated to the real base-type owner. The old
/// check would read this namespace coincidence as "covered" (RFC-044 [R10] violation,
/// deciding satisfaction from `namespace`). The fix must never consult `namespace` at
/// all here, so the result is identical to the no-dependency case: no diagnostic.
#[test]
fn namespace_coincidence_in_package_dependencies_is_never_consulted() {
    let store = repo(json!([
        {
            "packageId": "00000000-0000-4000-8000-0000000fffff",
            "namespace": "com.example.inv43",
            "name": "unrelated",
            "version": "1.0.0",
        }
    ]));
    let diags = type_inheritance_diagnostics(&store);
    assert!(diags.is_empty(), "expected no Inv 43 diagnostics, got {diags:?}");
}
