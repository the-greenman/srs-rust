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
//!
//! srs-rust#941 strengthens this further: `PackageBoundarySnapshot` (the
//! struct `export_package_boundary`/`import_package_boundary` route every
//! definition kind through) had no `protocols` field at all, so a Protocol
//! definition vanished across `repo copy` with a `{"ok":true}` false-green —
//! `assert_no_duplicate_ids`/`assert_one_home_per_definition` above only ever
//! checked for *extra* copies, never for silent loss. The fixture below adds
//! one Protocol, and `assert_definitions_identical` asserts every definition
//! kind's id *set* (not just a duplicate-free count) survives the round trip
//! unchanged — the exact assertion that must fail red against the pre-#941
//! snapshot shape and pass green once `protocols` is carried through it.
//!
//! srs-rust#946/#947 close a sibling gap in the same struct: it carried every
//! *definition* kind (fields, types, ... protocols) but nothing of the
//! boundary's own `package.json` scalar metadata. `import_package_boundary`
//! hardcoded `title` <- `name`, `description` <- `""`, `status` <- `"active"`,
//! `createdAt` <- a fixed placeholder, and silently dropped
//! `packageDependencies` entirely (found rebuilding `com.mudemocracy.governance`
//! 1.2.1 via `repo copy` — srs#553/#947 — and by PR #945's sweep — #946).
//! `assert_boundary_metadata_identical` below reads the boundary's own
//! `package.json` before and after and asserts `title`/`description`/`status`/
//! `createdAt`/`updatedAt`/`dataModelRevision`/`packageDependencies` are
//! byte-identical — the fixture's primary and sub-package boundaries both
//! carry distinct, non-default values for every one of those keys, so a
//! hardcoded-default reconstruction cannot accidentally pass.

use srs_repository::catalog;
use srs_repository::field_type_migration_service::CURRENT_DATA_MODEL_REVISION;
use srs_repository::repository_portability::copy_repository;
use srs_repository::srsj::open_srsj;
use srs_repository::store::{FileStore, RepositoryStore};
use std::collections::{BTreeMap, BTreeSet};

const SUB_FIELD_ID: &str = "00000000-0000-4000-8000-0000000000f1";
const PRIMARY_FIELD_ID: &str = "00000000-0000-4000-8000-0000000000f0";
const PROTOCOL_ID: &str = "00000000-0000-4000-8000-0000000000f2";

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
                    "title": "Primary Package Display Title",
                    "description": "A substantial multi-sentence description of the primary \
                                     boundary, distinct from its machine name — the exact \
                                     shape srs#553's governance-seed rebuild found downgraded \
                                     to the bare package name and an empty string.",
                    "status": "deprecated",
                    "createdAt": "2025-03-14T09:26:00Z",
                    "updatedAt": "2025-11-02T18:00:00Z",
                    "dataModelRevision": CURRENT_DATA_MODEL_REVISION,
                    "packageDependencies": [
                        {
                            "namespace": "com.semanticops.roundtrip.external",
                            "name": "external-dep",
                            "version": "3.2.1",
                        },
                    ],
                    "fields": ["fields/primary.json"],
                    "types": [],
                    "protocols": ["protocols/decision.json"],
                },
                "package/protocols/decision.json": {
                    "$schema": srs_schema::PROTOCOL_SCHEMA_ID,
                    "id": PROTOCOL_ID,
                    "namespace": "com.semanticops.roundtrip",
                    "name": "decision_protocol",
                    "version": 1,
                    "targetType": "00000000-0000-4000-8000-0000000000f3",
                    "stages": [
                        { "stageId": "s1", "name": "Draft", "order": 1 },
                    ],
                    "createdAt": "2026-01-01T00:00:00Z",
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
                    "title": "Sub Package Display Title",
                    "description": "Sub-package boundary — owns exactly one field, distinct \
                                     from the primary's, with its own distinct display title \
                                     and description.",
                    "status": "draft",
                    "createdAt": "2025-06-01T12:00:00Z",
                    "packageDependencies": [
                        {
                            "namespace": "com.semanticops.roundtrip.sub.external",
                            "name": "sub-external-dep",
                            "version": "0.9.0",
                        },
                    ],
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

/// The package.json scalar/passthrough properties srs-rust#946/#947 fixed —
/// everything in the schema besides identity (id/namespace/name/version,
/// asserted separately by `definition_id_sets`/`assert_one_home_per_definition`)
/// and the definition-file path arrays (which are legitimately recomputed by
/// every import, never expected to stay byte-identical).
const BOUNDARY_METADATA_KEYS: &[&str] = &[
    "title",
    "description",
    "status",
    "createdAt",
    "updatedAt",
    "dataModelRevision",
    "packageDependencies",
];

fn extract_boundary_metadata(
    pkg_json: &serde_json::Value,
) -> BTreeMap<&'static str, serde_json::Value> {
    BOUNDARY_METADATA_KEYS
        .iter()
        .filter_map(|&key| pkg_json.get(key).map(|v| (key, v.clone())))
        .collect()
}

/// Asserts a package boundary's own `package.json` scalar metadata — `title`,
/// `description`, `status`, `createdAt`, `updatedAt`, `dataModelRevision`,
/// `packageDependencies` — is byte-identical between `before` (the source's
/// on-disk package.json) and `after` (the same boundary's package.json post
/// round-trip). Red against pre-#946/#947 code: `import_package_boundary`
/// hardcoded `title`<-name, `description`<-"", `status`<-"active",
/// `createdAt`<-a fixed placeholder, and never wrote `packageDependencies`,
/// `updatedAt`, or `dataModelRevision` at all.
fn assert_boundary_metadata_identical(
    before: &serde_json::Value,
    after: &serde_json::Value,
    label: &str,
) {
    let before_meta = extract_boundary_metadata(before);
    let after_meta = extract_boundary_metadata(after);
    assert_eq!(
        before_meta, after_meta,
        "{label}: package.json metadata changed across the round trip \
         (title/description/status/createdAt/updatedAt/dataModelRevision/packageDependencies \
         must survive verbatim) — before={before_meta:?} after={after_meta:?}"
    );
}

/// Every definition kind `Package` carries, mapped to its set of ids. Used to assert
/// a `copy_repository` round trip preserves not just "no duplicates" (a drop and a
/// duplicate can otherwise cancel out in a bare count) but full identity: the exact
/// same ids, per kind, before and after (srs-rust#941).
fn definition_id_sets(store: &dyn RepositoryStore) -> BTreeMap<&'static str, BTreeSet<String>> {
    let pkg = store.load_package().expect("load_package must succeed");
    BTreeMap::from([
        ("fields", pkg.fields.iter().map(|f| f.id.clone()).collect()),
        (
            "record_types",
            pkg.record_types.iter().map(|t| t.id.clone()).collect(),
        ),
        (
            "relation_type_definitions",
            pkg.relation_type_definitions
                .iter()
                .map(|t| t.id.clone())
                .collect(),
        ),
        ("views", pkg.views.iter().map(|v| v.id.clone()).collect()),
        (
            "compositions",
            pkg.compositions.iter().map(|c| c.id.clone()).collect(),
        ),
        (
            "blueprints",
            pkg.blueprints
                .iter()
                .map(|lb| lb.blueprint.id.clone())
                .collect(),
        ),
        ("themes", pkg.themes.iter().map(|t| t.id.clone()).collect()),
        (
            "vocabularies",
            pkg.vocabularies.iter().map(|v| v.id.clone()).collect(),
        ),
        (
            "lifecycles",
            pkg.lifecycles.iter().map(|l| l.id.clone()).collect(),
        ),
        (
            "protocols",
            pkg.protocols
                .iter()
                .map(|lp| lp.protocol.id.clone())
                .collect(),
        ),
    ])
}

/// Asserts `after` carries, per definition kind, the identical set of ids as `before`
/// — same count AND same members, never merely "nothing extra" (that only catches
/// duplication, srs-rust#934's failure mode) or "nothing duplicated" (that only
/// catches inflation, not loss — srs-rust#941's failure mode: `protocols` had no
/// field in `PackageBoundarySnapshot` at all, so every protocol vanished silently).
fn assert_definitions_identical(
    before: &BTreeMap<&'static str, BTreeSet<String>>,
    after: &BTreeMap<&'static str, BTreeSet<String>>,
    label: &str,
) {
    for (kind, before_ids) in before {
        let after_ids = after.get(kind).cloned().unwrap_or_default();
        assert_eq!(
            before_ids, &after_ids,
            "{label}: '{kind}' definition set changed across the round trip — \
             before={before_ids:?} after={after_ids:?} (a definition was silently \
             dropped, replaced, or duplicated)"
        );
    }
}

#[test]
fn repo_copy_round_trip_preserves_package_boundaries_at_current_revision() {
    let source = current_revision_source_with_subpackage();
    assert_no_duplicate_ids(&source, "source");
    let before = definition_id_sets(&source);
    assert_eq!(
        before["protocols"].len(),
        1,
        "fixture must declare exactly one protocol"
    );
    let source_primary_pkg = source.load_instance_json("package/package.json").unwrap();
    let source_sub_pkg = source
        .load_instance_json("package/sub/package.json")
        .unwrap();
    // Sanity: the fixture itself must actually carry non-default values for
    // every metadata key under test, or a hardcoded-default reconstruction
    // could accidentally satisfy the equality assertions below.
    let source_primary_meta = extract_boundary_metadata(&source_primary_pkg);
    for key in BOUNDARY_METADATA_KEYS {
        assert!(
            source_primary_meta.contains_key(key),
            "fixture's primary package.json must declare '{key}' for this test to be meaningful"
        );
    }
    assert_ne!(
        source_primary_meta["title"],
        serde_json::json!("primary"),
        "fixture title must differ from the package name (else a name-as-title \
         bug would go undetected)"
    );
    assert_ne!(
        source_primary_meta["description"],
        serde_json::json!(""),
        "fixture description must be non-empty (else an empty-string-default bug \
         would go undetected)"
    );

    // file -> .srsj (export): the exact step srs#256 found broken.
    let exported = srs_repository::tree_session::new_tree_session();
    copy_repository(&source, &exported).expect("export must succeed");
    assert_one_home_per_definition(&exported);
    assert_no_duplicate_ids(&exported, "exported");
    assert_definitions_identical(
        &before,
        &definition_id_sets(&exported),
        "export (file -> .srsj)",
    );
    assert_boundary_metadata_identical(
        &source_primary_pkg,
        &exported.load_instance_json("package/package.json").unwrap(),
        "export (file -> .srsj), primary boundary",
    );
    assert_boundary_metadata_identical(
        &source_sub_pkg,
        &exported
            .load_instance_json("package/sub/package.json")
            .unwrap(),
        "export (file -> .srsj), sub-package boundary",
    );

    // .srsj -> file (reimport): srs#256 saw this fail with 446
    // SRS038-R12-DUPLICATE-ID diagnostics; the catalog must now load clean.
    let reimported = srs_repository::tree_session::new_tree_session();
    copy_repository(&exported, &reimported).expect("reimport must succeed");
    assert_one_home_per_definition(&reimported);
    assert_no_duplicate_ids(&reimported, "reimported");
    assert_definitions_identical(
        &before,
        &definition_id_sets(&reimported),
        "reimport (.srsj -> file)",
    );
    assert_boundary_metadata_identical(
        &source_primary_pkg,
        &reimported
            .load_instance_json("package/package.json")
            .unwrap(),
        "reimport (.srsj -> file), primary boundary",
    );
    assert_boundary_metadata_identical(
        &source_sub_pkg,
        &reimported
            .load_instance_json("package/sub/package.json")
            .unwrap(),
        "reimport (.srsj -> file), sub-package boundary",
    );

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
