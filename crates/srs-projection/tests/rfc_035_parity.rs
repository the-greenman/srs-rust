//! RFC-035 byte-parity: the Rust projection vs the Node reference emitter.
//!
//! `projection-rules.md` says a conforming emitter — "including the #260
//! `srs-projection` Rust binding" — MUST produce byte-identical output. This
//! test is that claim, executed: it loads the frozen
//! `com.semanticops.srs/metamodel` package from the spec repo, projects the two
//! meta-model entities plus the bundle envelope, and compares the bytes against
//! the reference emitter's committed goldens (`tests/rfc-035/goldens/`).
//!
//! Comparing against the *goldens* rather than shelling out to Node keeps the
//! test hermetic (no Node required in CI) while still comparing to the
//! reference: the goldens are what `tests/rfc-035/run.mjs` asserts the Node
//! emitter produces, so agreeing with them is agreeing with it.
//!
//! Skipped when the spec repo is not present as a sibling checkout — the same
//! convention `core_bundle_drift.rs` uses.

use srs_projection::json_schema::to_canonical_json;
use srs_projection::{
    schema_bundle, type_to_json_schema, SchemaBundleInput, TypeToJsonSchemaInput,
};
use srs_repository::store::{FileStore, RepositoryStore};
use std::path::{Path, PathBuf};

/// The spec repo checkout, or `None` when it is not available.
///
/// Prefer `SRS_SPEC_DIR` (CI, and any local run — point it at a fresh
/// `origin/master` checkout, never a long-lived sibling). The sibling
/// fallback below is srs-rust#874's exact false-green trap: `srs-rust`'s
/// standard dev layout keeps a sibling `srs` checkout that may sit on a
/// stale, non-master branch (as this machine's did) — silently comparing
/// against its goldens produced a false green while CI (checking out
/// `origin/master` fresh) was red. So the fallback is loud, not quiet: it
/// panics rather than comparing against a sibling whose goldens can't be
/// trusted.
fn spec_repo() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SRS_SPEC_DIR") {
        let p = PathBuf::from(dir);
        if p.join("srs/package/metamodel").is_dir() {
            return Some(p);
        }
        return None; // an explicit but unusable SRS_SPEC_DIR: skip, don't silently fall through
    }
    let sibling = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../srs");
    if !sibling.join("srs/package/metamodel").is_dir() {
        return None;
    }
    if !sibling.join("tests/rfc-035/goldens").is_dir() {
        panic!(
            "srs sibling checkout at {} has no tests/rfc-035/goldens/ — set SRS_SPEC_DIR to a \
             fresh `origin/master` checkout instead of relying on this sibling (srs-rust#874)",
            sibling.display()
        );
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
                    "srs sibling checkout at {} is on branch '{branch}', not master — its goldens \
                     may be stale (srs-rust#874's exact false-green trap: a stale sibling silently \
                     passed locally while CI, on a fresh checkout, was red). Set SRS_SPEC_DIR to a \
                     fresh `origin/master` checkout instead.",
                    sibling.display()
                );
            }
        }
        // else: not a git checkout (e.g. an extracted archive) — nothing to verify, proceed.
    }
    Some(sibling)
}

/// A store over the spec repo's own `srs/` SRS repository, whose package
/// includes the frozen metamodel definitions.
fn metamodel_store(spec: &Path) -> FileStore {
    FileStore::new(spec.join("srs"))
}

/// The live sibling spec repo is migrated by the #297 train's spec-cutover
/// unit, after this binary releases. Until then a post-flip binary rejects it
/// ([R2]/[R21]) — skip rather than fail, exactly like the pre-cutover skip in
/// `discovery_conformance.rs`. Delete this guard once the spec repo is
/// migrated (it will then never fire).
///
/// srs-rust#924 (srs#525, binary-first choreography): the metamodel package's
/// `type_id_of` loads the *whole* package (`store.load_package()`), including
/// `com.semanticops.srs`'s own compositions — `spec-authoring-core`'s
/// `spec-document-view.json` still carries a `type-query` section on
/// `origin/master` until srs#525 merges and re-pins. Requiring revision >= 7
/// (`DISCOVERY_QUERY_CUTOVER_REVISION`) — not just >= 2 — keeps this test's
/// existing "skip until the corpus catches up" contract honest for that new
/// shape too, rather than hard-failing during the expected red-by-construction
/// window (CLAUDE.md "Gates and choreography"). Lower this back to >= 2 only
/// if a future change decouples the metamodel projection from the full
/// package load.
fn spec_repo_is_migrated(spec: &Path) -> bool {
    let manifest_path = spec.join("srs/manifest.json");
    let Ok(raw) = std::fs::read_to_string(&manifest_path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    value.get("instanceIndex").is_none()
        && value.get("dataModelRevision").and_then(|v| v.as_u64()) >= Some(7)
}

fn type_id_of(store: &FileStore, namespace: &str, name: &str) -> String {
    let package = store.load_package().expect("spec package must load");
    package
        .record_types
        .iter()
        .find(|t| t.namespace == namespace && t.name == name)
        .unwrap_or_else(|| panic!("metamodel must define {namespace}/{name}"))
        .id
        .clone()
}

fn golden(spec: &Path, name: &str) -> String {
    std::fs::read_to_string(spec.join("tests/rfc-035/goldens").join(name))
        .unwrap_or_else(|e| panic!("golden {name} must be readable: {e}"))
}

#[test]
fn entity_schemas_match_the_reference_emitter_byte_for_byte() {
    let Some(spec) = spec_repo() else {
        eprintln!("skipping: spec repo not found (set SRS_SPEC_DIR)");
        return;
    };
    if !spec_repo_is_migrated(&spec) {
        eprintln!(
            "skipping: sibling spec repo is pre-RFC-038 format or below data-model revision 7 \
             (awaiting the #297 spec-cutover unit and/or srs#525/srs-rust#924's \
             discovery-query-cutover corpus re-pin)"
        );
        return;
    }
    let store = metamodel_store(&spec);

    for entity in ["field", "type"] {
        let type_id = type_id_of(&store, "com.semanticops.srs", entity);
        let result = type_to_json_schema(
            &store,
            TypeToJsonSchemaInput {
                type_id,
                type_version: None,
            },
        )
        .unwrap_or_else(|e| panic!("projecting {entity} must succeed: {e}"));

        let got = to_canonical_json(&result.schema).expect("serializes");
        let want = golden(&spec, &format!("{entity}.json"));
        assert_eq!(
            got, want,
            "the Rust projection of `{entity}` must be byte-identical to the reference emitter's \
             golden. projection-rules.md requires byte-parity, so a difference here is a \
             conformance failure, not a formatting nit."
        );
    }
}

#[test]
fn bundle_envelope_matches_the_reference_emitter_byte_for_byte() {
    let Some(spec) = spec_repo() else {
        eprintln!("skipping: spec repo not found (set SRS_SPEC_DIR)");
        return;
    };
    if !spec_repo_is_migrated(&spec) {
        eprintln!(
            "skipping: sibling spec repo is pre-RFC-038 format or below data-model revision 7 \
             (awaiting the #297 spec-cutover unit and/or srs#525/srs-rust#924's \
             discovery-query-cutover corpus re-pin)"
        );
        return;
    }
    let store = metamodel_store(&spec);

    let result = schema_bundle(
        &store,
        SchemaBundleInput {
            entities: vec!["field".to_string(), "type".to_string()],
        },
    )
    .expect("bundle must emit");

    // RFC-033 [R6]: the bundle carries the revision it was generated for. Read
    // the expectation from the spec's own manifest.json rather than cloning
    // its current revision into a literal here — a literal breaks every time
    // the spec bumps dataModelRevision (2->3 in 38386cfa, 3->4 above, and
    // 4->5 is already queued behind srs PR #510). The manifest's stamp is
    // asserted non-zero first, so this still proves the bundle's stamp is
    // being read rather than defaulted to 0.
    let manifest_raw = std::fs::read_to_string(spec.join("srs/manifest.json"))
        .expect("spec repo's srs/manifest.json must be readable");
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_raw).expect("srs/manifest.json must be valid JSON");
    let expected_revision = manifest["dataModelRevision"]
        .as_u64()
        .expect("srs/manifest.json must stamp dataModelRevision");
    assert!(
        expected_revision > 0,
        "dataModelRevision must not be the absent-default 0"
    );
    assert_eq!(result.bundle.data_model_revision, expected_revision);

    let got = to_canonical_json(&result.bundle).expect("serializes");
    let want = golden(&spec, "bundle.json");
    assert_eq!(got, want, "bundle envelope must be byte-identical");
}

/// A schema that quietly emits `{}` and says nothing looks complete while
/// under-validating, so the projection must name every approximated feature
/// it hits — per `docs/schema/2.0/metamodel-fidelity.md`'s "approximated"
/// rows.
///
/// This is a self-contained fixture, not a live-corpus check: the frozen
/// `field`/`type` entities used to carry exactly one approximated feature
/// (`Field.defaultValue`, `dependent` on the field's own `fieldType`), which
/// is how the original version of this test was written. RFC-040 retired
/// `defaultValue` outright (no `dependent`-datatype field survives anywhere
/// in the current metamodel — `field.json`/`type.json`'s `inexpressible` set
/// is genuinely empty now), so pinning this test to the live corpus was
/// fragile: it broke not because the *mechanism* regressed, but because the
/// corpus stopped incidentally exercising it. Building the fixture directly
/// tests the mechanism the dashboard actually documents (`dependent` and
/// `vocabularyRef`, both still **approximated** per the dashboard), and
/// survives future corpus changes that are unrelated to this test's concern.
#[test]
fn the_projection_reports_what_it_could_not_express() {
    use srs_core::types::field::{Datatype, Field, FieldType};
    use srs_core::types::record_type::{FieldAssignment, RecordType};
    use srs_repository::manifest::Manifest;
    use srs_repository::package::Package;
    use srs_repository::store::memory::MemoryStore;

    let dependent_ft = FieldType {
        depends_on: Some("self".to_string()),
        ..FieldType::new(Datatype::Dependent)
    };
    let dependent_field = Field {
        description: String::new(),
        ai_guidance: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        ..Field::new("f-dependent", "com.probe", "default_value", dependent_ft)
    };
    let vocab_field = Field {
        description: String::new(),
        ai_guidance: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        ..Field::new(
            "f-vocab",
            "com.probe",
            "status",
            FieldType::closed_by_ref("11111111-0000-4000-8000-000000000001"),
        )
    };
    let assign = |field_id: &str, order: u32| FieldAssignment {
        field_id: field_id.to_string(),
        order,
        required: false,
        display_label: None,
        description: None,
    };
    let probe_type = RecordType {
        schema: None,
        ai_guidance: None,
        tags: None,
        id: "t-probe".to_string(),
        namespace: "com.probe".to_string(),
        name: "probe".to_string(),
        version: 1,
        description: "probe type".to_string(),
        fields: vec![assign("f-dependent", 0), assign("f-vocab", 1)],
        extends_type_id: None,
        extends_type_version: None,
        field_order: None,
        field_assignment_overrides: None,
        identity_field_id: None,
        lifecycle: None,
        lifecycle_ref: None,
        validation_rules: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        lineage: None,
        provenance: None,
    };
    let package = Package {
        id: "pkg".to_string(),
        namespace: "com.probe".to_string(),
        name: "probe".to_string(),
        version: "1.0.0".to_string(),
        fields: vec![dependent_field, vocab_field],
        record_types: vec![probe_type],
        relation_type_definitions: vec![],
        views: vec![],
        compositions: vec![],
        themes: vec![],
        blueprints: vec![],
        protocols: vec![],
        root: std::path::PathBuf::new(),
        package_dependencies: vec![],
        vocabularies: vec![],
        lifecycles: vec![],
    };
    let store = MemoryStore::new(Manifest::default(), package);

    let result = type_to_json_schema(
        &store,
        TypeToJsonSchemaInput {
            type_id: "t-probe".to_string(),
            type_version: None,
        },
    )
    .expect("projection must succeed");

    assert_eq!(
        result.inexpressible.len(),
        2,
        "expected the dependent field and the vocabularyRef field: {:?}",
        result.inexpressible
    );
    assert!(result.inexpressible[0].contains("default_value"));
    assert!(result.inexpressible[0].contains("dependent"));
    assert!(result.inexpressible[1].contains("status"));
    assert!(result.inexpressible[1].contains("vocabulary"));

    // ...and the node it could not constrain really is the unconstrained one.
    // `com.probe` is a domain package, so the property key is `Field.name`
    // verbatim (RFC-039 [R2a]/[R2b]) — no metamodel case transform applies.
    let dependent_node = result
        .schema
        .properties
        .iter()
        .find(|(k, _)| *k == "default_value")
        .map(|(_, v)| v)
        .expect("probe must project a default_value property");
    assert_eq!(
        *dependent_node,
        srs_projection::json_schema::SchemaNode::default(),
        "a `dependent` field projects to an unconstrained node"
    );
}

#[test]
fn type_version_selects_the_version_the_caller_asked_for() {
    // A UUID lineage has many versions and `name` addresses all of them, so a
    // projection that resolves the version and then re-looks-up by name returns
    // a *different* Type with `ok: true` and no diagnostic — a wrong answer
    // from the capability's primary entry point.
    use srs_core::types::field::{Field, FieldType};
    use srs_core::types::record_type::{FieldAssignment, RecordType};
    use srs_repository::manifest::Manifest;
    use srs_repository::package::Package;
    use srs_repository::store::memory::MemoryStore;

    const TID: &str = "aaaaaaaa-0000-4000-8000-00000000000a";

    let mk_field = |id: &str, name: &str| Field {
        description: String::new(),
        ai_guidance: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        ..Field::new(id, "com.probe", name, FieldType::string())
    };
    let assign = |field_id: &str| FieldAssignment {
        field_id: field_id.to_string(),
        order: 0,
        required: false,
        display_label: None,
        description: None,
    };
    let mk_type = |version: u32, field_id: &str| RecordType {
        schema: None,
        ai_guidance: None,
        tags: None,
        id: TID.to_string(),
        namespace: "com.probe".to_string(),
        name: "thing".to_string(),
        version,
        description: format!("v{version} shape"),
        fields: vec![assign(field_id)],
        extends_type_id: None,
        extends_type_version: None,
        field_order: None,
        field_assignment_overrides: None,
        identity_field_id: None,
        lifecycle: None,
        lifecycle_ref: None,
        validation_rules: None,
        lineage: None,
        provenance: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
    };

    let package = Package {
        id: "pkg".to_string(),
        namespace: "com.probe".to_string(),
        name: "probe".to_string(),
        version: "1.0.0".to_string(),
        fields: vec![mk_field("f-v1", "alpha"), mk_field("f-v2", "beta")],
        record_types: vec![mk_type(1, "f-v1"), mk_type(2, "f-v2")],
        relation_type_definitions: vec![],
        views: vec![],
        compositions: vec![],
        themes: vec![],
        blueprints: vec![],
        protocols: vec![],
        root: std::path::PathBuf::new(),
        package_dependencies: vec![],
        vocabularies: vec![],
        lifecycles: vec![],
    };
    let store = MemoryStore::new(Manifest::default(), package);

    for (requested, expected_property, expected_id_suffix) in
        [(1u32, "alpha", "thing/1.json"), (2, "beta", "thing/2.json")]
    {
        let result = type_to_json_schema(
            &store,
            TypeToJsonSchemaInput {
                type_id: TID.to_string(),
                type_version: Some(requested),
            },
        )
        .expect("projects");
        assert!(
            result.schema.id.ends_with(expected_id_suffix),
            "v{requested} must project its own $id, got {}",
            result.schema.id
        );
        assert_eq!(
            result.schema.properties.iter().next().map(|(k, _)| k),
            Some(expected_property),
            "v{requested} must project its own fields"
        );
    }

    // Omitting the version resolves the latest.
    let latest = type_to_json_schema(
        &store,
        TypeToJsonSchemaInput {
            type_id: TID.to_string(),
            type_version: None,
        },
    )
    .expect("projects");
    assert!(
        latest.schema.id.ends_with("thing/2.json"),
        "{}",
        latest.schema.id
    );
}
