//! Service-level tests for `package_install_service` (#506).
//!
//! The source fixture (`tests/fixtures/install-package/`) is a minimal synthetic
//! package modeled on the canonical `com.mudemocracy.governance` package shape
//! (fields, type, relation-type, lifecycle, view, document-view, blueprint,
//! protocol). Per CLAUDE.md ("Working with the Spec Repo") the real spec-repo
//! package is not referenced from tests.

use srs_core::extensions::import_tracking::ConflictState;
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_repository::json_store::JsonStore;
use srs_repository::package_install_service::{install_package, InstallPackageInput};
use srs_repository::package_service;
use srs_repository::repository_lifecycle::{
    InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::store::{FileStore, RepositoryStore};
use srs_repository::validation::validate_repository;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/install-package")
}

fn init_input() -> InitializeRepositoryInput {
    InitializeRepositoryInput {
        repository: RepositoryMetadata {
            // Must be a UUID: the root-container embed inherits this id, and
            // validation checks the embed when no container file exists.
            repository_id: "17575e57-0000-4000-8000-175753e57000".to_string(),
            namespace: "com.test.install".to_string(),
            srs_version: "2.0-draft".to_string(),
            title: Some("Install Test".to_string()),
            description: None,
        },
        primary_package: PrimaryPackageMetadata {
            id: "install-test-primary".to_string(),
            namespace: "com.test.install".to_string(),
            name: "primary".to_string(),
            version: "1.0.0".to_string(),
        },
    }
}

fn fresh_file_repo() -> (TempDir, FileStore) {
    let temp = TempDir::new().expect("temp dir");
    let store = FileStore::new(temp.path());
    store.initialize_repository(&init_input()).expect("init");
    (temp, store)
}

fn install_input() -> InstallPackageInput {
    InstallPackageInput {
        source_dir: fixture_dir().display().to_string(),
        boundary_path: None,
        strict: false,
    }
}

/// A `precedes` relation type under a different UUID than the fixture ships.
fn local_precedes() -> RelationTypeDefinition {
    serde_json::from_value(serde_json::json!({
        "id": "00000000-aaaa-4bbb-8ccc-000000000099",
        "version": 1,
        "key": "precedes",
        "namespace": "com.test.install",
        "label": "Precedes",
        "description": "Local precedes.",
        "category": "sequence",
        "createdAt": "2026-01-01T00:00:00Z",
        "irreflexive": true
    }))
    .expect("local precedes parses")
}

fn assert_zero_errors(store: &dyn RepositoryStore) {
    let report = validate_repository(store).expect("validate runs");
    assert_eq!(
        report.summary.errors, 0,
        "expected 0 validation errors, got: {:?}",
        report.diagnostics
    );
}

/// The fixture ships 9 definitions across 8 kinds.
const FIXTURE_DEFINITION_COUNT: usize = 9;

// ── (a) install into an empty repo ──────────────────────────────────────────

#[test]
#[ignore = "srs-rust#783 Phase 3 KNOWN GAP: RFC-014 install writes upstreamPackage provenance into the boundary package.json, but package-manifest.json (additionalProperties: false) denies it — every installed sub-package is fatally catalog-invalid under RFC-038. Spec-level conflict, owner decision needed (schema mirrors are read-only)."]
fn install_into_empty_repo_installs_everything() {
    let (_temp, store) = fresh_file_repo();
    let result = install_package(&store, install_input()).expect("install succeeds");

    assert_eq!(result.boundary_path, "packages/install-fixture");
    assert_eq!(result.package_id, "9a1b0c2d-1111-4aaa-8bbb-000000000001");
    assert_eq!(result.namespace, "com.example.install");
    assert_eq!(result.name, "install-fixture");
    assert_eq!(result.version, "1.0.0");
    assert!(!result.installed_at.is_empty());
    assert_eq!(result.installed, FIXTURE_DEFINITION_COUNT);
    assert_eq!(result.skipped_identical, 0);
    assert!(result.conflicts.is_empty());

    // Per-kind breakdown covers all 8 kinds shipped by the fixture.
    let kinds: Vec<(&str, usize)> = result
        .kinds
        .iter()
        .map(|k| (k.kind.as_str(), k.installed))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("field", 2),
            ("type", 1),
            ("relationType", 1),
            ("lifecycle", 1),
            ("view", 1),
            ("documentView", 1),
            ("blueprint", 1),
            ("protocol", 1),
        ]
    );

    // The boundary is registered and the compiled package sees the definitions.
    let packages = package_service::list_packages(&store).unwrap();
    assert!(packages.iter().any(|p| p.boundary_path.as_deref()
        == Some("packages/install-fixture")
        && p.id == "9a1b0c2d-1111-4aaa-8bbb-000000000001"));

    let package = store.load_package().unwrap();
    assert!(package
        .fields
        .iter()
        .any(|f| f.namespace == "com.example.install" && f.name == "title"));
    assert!(package
        .record_types
        .iter()
        .any(|t| t.namespace == "com.example.install" && t.name == "entry"));
    assert!(package
        .relation_type_definitions
        .iter()
        .any(|rt| rt.key == "precedes" && rt.namespace == "com.example.install"));
    assert!(package.views.iter().any(|v| v.name == "entry-view"));
    assert!(package
        .document_views
        .iter()
        .any(|dv| dv.name == "entry-log"));
    assert!(package
        .blueprints
        .iter()
        .any(|b| b.blueprint.name == "entry-log"));
    assert!(package
        .protocols
        .iter()
        .any(|p| p.protocol.protocol_name == "entry"));
    assert!(package
        .lifecycles
        .iter()
        .any(|lc| lc.name == "simple_lifecycle"));

    // Provenance: field list payload reports the boundary as source_package.
    let fields = package_service::list_fields(&store).unwrap();
    let title = fields
        .iter()
        .find(|f| f.name == "title" && f.namespace == "com.example.install")
        .expect("installed title field listed");
    assert_eq!(
        title.source_package.as_deref(),
        Some("packages/install-fixture")
    );

    // Upstream provenance stamp on the boundary package.json.
    let pkg_json = store
        .load_instance_json("packages/install-fixture/package.json")
        .unwrap();
    assert_eq!(
        pkg_json["upstreamPackage"]["packageId"].as_str(),
        Some("9a1b0c2d-1111-4aaa-8bbb-000000000001")
    );
    assert_eq!(
        pkg_json["upstreamPackage"]["version"].as_str(),
        Some("1.0.0")
    );
    assert_eq!(
        pkg_json["upstreamPackage"]["installedAt"].as_str(),
        Some(result.installed_at.as_str())
    );

    // Repo validates with zero errors (dangling-container warnings are expected —
    // the fixture document view ships a gallery container UUID by design).
    assert_zero_errors(&store);
}

#[test]
fn install_honours_explicit_boundary_override() {
    let (_temp, store) = fresh_file_repo();
    let result = install_package(
        &store,
        InstallPackageInput {
            boundary_path: Some("packages/custom".to_string()),
            ..install_input()
        },
    )
    .expect("install succeeds");
    assert_eq!(result.boundary_path, "packages/custom");
    let packages = package_service::list_packages(&store).unwrap();
    assert!(packages
        .iter()
        .any(|p| p.boundary_path.as_deref() == Some("packages/custom")));
}

// ── (b) identical-UUID definitions are skipped ───────────────────────────────

#[test]
fn install_skips_identical_uuid_definitions() {
    let (_temp, store) = fresh_file_repo();

    // Pre-create the fixture's `title` field in the PRIMARY package with the
    // exact same UUID — the installer must skip it, wherever it lives.
    let title = srs_core::types::field::Field {
        schema: None,
        id: "9a1b0c2d-0001-4aaa-8bbb-00000000f001".to_string(),
        namespace: "com.example.install".to_string(),
        name: "title".to_string(),
        version: 1,
        field_type: srs_core::types::field::FieldType::string(),
        description: "Short label for this record.".to_string(),
        instructions: None,
        ai_guidance: srs_core::types::field::AiGuidance::default(),
        default_value: None,
        editor_hint: None,
        tags: None,
        lineage: None,
        provenance: None,
        deprecated_at: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
    };
    package_service::create_field(&store, title).expect("pre-create field");

    let result = install_package(&store, install_input()).expect("install succeeds");
    assert_eq!(result.installed, FIXTURE_DEFINITION_COUNT - 1);
    assert_eq!(result.skipped_identical, 1);
    assert!(result.conflicts.is_empty());

    let field_kind = result.kinds.iter().find(|k| k.kind == "field").unwrap();
    assert_eq!(field_kind.installed, 1);
    assert_eq!(field_kind.skipped_identical, 1);

    // No duplicate: exactly one field with that UUID in the compiled package.
    let package = store.load_package().unwrap();
    let titles: Vec<_> = package
        .fields
        .iter()
        .filter(|f| f.id == "9a1b0c2d-0001-4aaa-8bbb-00000000f001")
        .collect();
    assert_eq!(titles.len(), 1);
}

// ── (c) same-key / different-UUID → conflict, not duplicate ─────────────────

#[test]
#[ignore = "srs-rust#783 Phase 3 KNOWN GAP: RFC-014 install writes upstreamPackage provenance into the boundary package.json, but package-manifest.json (additionalProperties: false) denies it — every installed sub-package is fatally catalog-invalid under RFC-038. Spec-level conflict, owner decision needed (schema mirrors are read-only)."]
fn install_flags_same_key_different_uuid_relation_type_as_conflict() {
    let (_temp, store) = fresh_file_repo();

    // Target already defines `precedes` under a DIFFERENT UUID.
    package_service::create_relation_type(&store, local_precedes(), None).expect("pre-create rt");

    let result = install_package(&store, install_input()).expect("install succeeds with warning");
    assert_eq!(result.installed, FIXTURE_DEFINITION_COUNT - 1);
    assert_eq!(result.conflicts.len(), 1);
    let c = &result.conflicts[0];
    assert_eq!(c.kind, "relationType");
    assert_eq!(c.key, "precedes");
    assert_eq!(c.source_id, "9a1b0c40-0004-4aaa-8bbb-00000000e001");
    assert_eq!(c.existing_id, "00000000-aaaa-4bbb-8ccc-000000000099");

    // Not duplicated: exactly one `precedes` definition; load_package still works
    // (a same-key duplicate would hard-error as RelationTypeDefinitionConflict).
    let package = store.load_package().unwrap();
    let precedes: Vec<_> = package
        .relation_type_definitions
        .iter()
        .filter(|rt| rt.key == "precedes")
        .collect();
    assert_eq!(precedes.len(), 1);
    assert_eq!(precedes[0].id, "00000000-aaaa-4bbb-8ccc-000000000099");

    assert_zero_errors(&store);
}

#[test]
fn strict_install_fails_on_conflict_without_writing() {
    let (_temp, store) = fresh_file_repo();
    package_service::create_relation_type(&store, local_precedes(), None).expect("pre-create rt");

    let err = install_package(
        &store,
        InstallPackageInput {
            strict: true,
            ..install_input()
        },
    )
    .expect_err("strict install must fail on conflict");
    assert!(
        matches!(
            err,
            srs_repository::error::RepositoryError::PackageInstallConflicts { count: 1, .. }
        ),
        "got: {err:?}"
    );

    // Nothing was written: no boundary registered.
    let packages = package_service::list_packages(&store).unwrap();
    assert_eq!(packages.len(), 1, "only the primary boundary must exist");
}

// ── (d) re-run → idempotent ──────────────────────────────────────────────────

#[test]
#[ignore = "srs-rust#783 Phase 3 KNOWN GAP: RFC-014 install writes upstreamPackage provenance into the boundary package.json, but package-manifest.json (additionalProperties: false) denies it — every installed sub-package is fatally catalog-invalid under RFC-038. Spec-level conflict, owner decision needed (schema mirrors are read-only)."]
fn rerun_install_skips_everything_and_keeps_provenance() {
    let (_temp, store) = fresh_file_repo();
    let first = install_package(&store, install_input()).expect("first install");
    let second = install_package(&store, install_input()).expect("second install");

    assert_eq!(second.installed, 0);
    assert_eq!(second.skipped_identical, FIXTURE_DEFINITION_COUNT);
    assert!(second.conflicts.is_empty());
    assert_eq!(second.boundary_path, first.boundary_path);
    assert_eq!(
        second.installed_at, first.installed_at,
        "idempotent re-run keeps the original installedAt stamp"
    );

    // No duplicates in the boundary index.
    let boundary = store
        .load_package_boundary(&Some("packages/install-fixture".to_string()))
        .unwrap();
    assert_eq!(boundary.field_paths.len(), 2);
    assert_eq!(boundary.type_paths.len(), 1);

    assert_zero_errors(&store);
}

// ── Cross-store roundtrip: JsonStore install → srsj → re-parse → validate ───

#[test]
#[ignore = "srs-rust#783 Phase 3 KNOWN GAP: RFC-014 install writes upstreamPackage provenance into the boundary package.json, but package-manifest.json (additionalProperties: false) denies it — every installed sub-package is fatally catalog-invalid under RFC-038. Spec-level conflict, owner decision needed (schema mirrors are read-only)."]
fn json_store_install_survives_srsj_roundtrip() {
    let temp = TempDir::new().expect("temp dir");
    let srsj_path = temp.path().join("repo.srsj");
    let store = JsonStore::create(&srsj_path).expect("create json store");
    store.initialize_repository(&init_input()).expect("init");

    let result = install_package(&store, install_input()).expect("install");
    assert_eq!(result.installed, FIXTURE_DEFINITION_COUNT);

    // Roundtrip: serialize → re-parse → the boundary and definitions survive.
    let srsj = store.to_srsj_string().expect("to_srsj_string");
    let store2 = JsonStore::from_srsj(&srsj).expect("re-parse");

    let packages = package_service::list_packages(&store2).unwrap();
    assert!(packages
        .iter()
        .any(|p| p.boundary_path.as_deref() == Some("packages/install-fixture")));

    let fields = package_service::list_fields(&store2).unwrap();
    assert!(fields
        .iter()
        .any(|f| f.namespace == "com.example.install" && f.name == "title"));

    // Re-run against the re-parsed store: still idempotent across stores.
    let rerun = install_package(&store2, install_input()).expect("re-run on roundtripped store");
    assert_eq!(rerun.installed, 0);
    assert_eq!(rerun.skipped_identical, FIXTURE_DEFINITION_COUNT);

    assert_zero_errors(&store2);
}

// ── Input validation ─────────────────────────────────────────────────────────

#[test]
fn install_rejects_missing_source_dir() {
    let (_temp, store) = fresh_file_repo();
    let err = install_package(
        &store,
        InstallPackageInput {
            source_dir: "/nonexistent/package/dir".to_string(),
            boundary_path: None,
            strict: false,
        },
    )
    .expect_err("missing source must fail");
    assert!(
        matches!(
            err,
            srs_repository::error::RepositoryError::PackageRefMissing { .. }
        ),
        "got: {err:?}"
    );
}

#[test]
fn install_rejects_primary_boundary_target() {
    let (_temp, store) = fresh_file_repo();
    let err = install_package(
        &store,
        InstallPackageInput {
            boundary_path: Some("package".to_string()),
            ..install_input()
        },
    )
    .expect_err("primary boundary must be rejected");
    assert!(
        matches!(
            err,
            srs_repository::error::RepositoryError::InvalidRepositoryInitialization { .. }
        ),
        "got: {err:?}"
    );
}

// ── Import records: FileStore cross-store roundtrip ──────────────────────────

#[test]
fn file_store_install_writes_import_records_and_ref_copies() {
    let (_temp, store) = fresh_file_repo();
    let result = install_package(&store, install_input()).expect("install succeeds");

    // import-records.json must exist in the boundary directory.
    let summary_json = store
        .load_instance_json("packages/install-fixture/.srs-import/import-records.json")
        .expect("import-records.json must exist after FileStore install");

    let fields = summary_json["fields"].as_array().expect("fields array");
    assert_eq!(fields.len(), 2, "two fields in fixture");
    assert_eq!(
        fields[0]["conflictState"].as_str(),
        Some("clean"),
        "freshly installed field should be clean"
    );
    assert_eq!(
        summary_json["generatedAt"].as_str(),
        Some(result.installed_at.as_str()),
        "generatedAt must match installedAt"
    );

    // Reference copies must be present alongside the installed files.
    store
        .load_instance_json("packages/install-fixture/.srs-import/refs/fields/title-9a1b0c2d.json")
        .expect("reference copy for title field must exist");
    store
        .load_instance_json("packages/install-fixture/.srs-import/refs/fields/body-9a1b0c2e.json")
        .expect("reference copy for body field must exist");
    store
        .load_instance_json(
            "packages/install-fixture/.srs-import/refs/relation-types/precedes-9a1b0c40.json",
        )
        .expect("reference copy for precedes relation type must exist");
}

#[test]
fn file_store_list_package_imports_shows_clean_after_install() {
    use srs_repository::package_service::{list_package_imports, ListPackageImportsFilter};

    let (_temp, store) = fresh_file_repo();
    install_package(&store, install_input()).expect("install succeeds");

    let summary =
        list_package_imports(&store, ListPackageImportsFilter::default()).expect("list succeeds");

    assert_eq!(summary.fields.len(), 2, "two fields from fixture");
    for field in &summary.fields {
        assert_eq!(
            field.conflict_state,
            Some(ConflictState::Clean),
            "all fields should be clean immediately after install"
        );
    }
    // The relation type is tracked too (it has a reference copy).
    assert_eq!(summary.relation_types.len(), 1);
    assert_eq!(
        summary.relation_types[0].conflict_state,
        Some(ConflictState::Clean)
    );
}

#[test]
fn file_store_list_package_imports_detects_local_ahead_after_edit() {
    use srs_repository::package_service::{list_package_imports, ListPackageImportsFilter};

    let (temp, store) = fresh_file_repo();
    install_package(&store, install_input()).expect("install succeeds");

    // Modify the installed title field on disk to simulate a local edit.
    let title_path = temp
        .path()
        .join("packages/install-fixture/fields/title-9a1b0c2d.json");
    let original: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&title_path).unwrap()).unwrap();
    let mut modified = original.clone();
    modified["version"] = serde_json::json!(2);
    modified["description"] = serde_json::json!("Locally edited description.");
    std::fs::write(
        &title_path,
        serde_json::to_string_pretty(&modified).unwrap(),
    )
    .unwrap();

    let summary =
        list_package_imports(&store, ListPackageImportsFilter::default()).expect("list succeeds");

    let title = summary
        .fields
        .iter()
        .find(|f| f.name == "title")
        .expect("title field in summary");
    assert_eq!(
        title.conflict_state,
        Some(ConflictState::LocalAhead),
        "locally edited field must be detected as local-ahead"
    );

    // Unedited field is still clean.
    let body = summary
        .fields
        .iter()
        .find(|f| f.name == "body")
        .expect("body field in summary");
    assert_eq!(body.conflict_state, Some(ConflictState::Clean));
}
