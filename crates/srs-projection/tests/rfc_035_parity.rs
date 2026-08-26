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
fn spec_repo() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SRS_SPEC_DIR") {
        let p = PathBuf::from(dir);
        if p.join("srs/package/metamodel").is_dir() {
            return Some(p);
        }
    }
    let sibling = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../srs");
    sibling
        .join("srs/package/metamodel")
        .is_dir()
        .then(|| sibling.clone())
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
fn spec_repo_is_migrated(spec: &Path) -> bool {
    let manifest_path = spec.join("srs/manifest.json");
    let Ok(raw) = std::fs::read_to_string(&manifest_path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    value.get("instanceIndex").is_none()
        && value.get("dataModelRevision").and_then(|v| v.as_u64()) >= Some(2)
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
        eprintln!("skipping: sibling spec repo is pre-RFC-038 format (awaiting the #297 spec-cutover unit)");
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
        eprintln!("skipping: sibling spec repo is pre-RFC-038 format (awaiting the #297 spec-cutover unit)");
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

    // RFC-033 [R6]: the bundle carries the revision it was generated for. The
    // spec repo is stamped 2 (RFC-039 cutover, srs#242 Phase B); absent ⇒ 0,
    // so this also proves the stamp is being read rather than defaulted.
    assert_eq!(result.bundle.data_model_revision, 2);

    let got = to_canonical_json(&result.bundle).expect("serializes");
    let want = golden(&spec, "bundle.json");
    assert_eq!(got, want, "bundle envelope must be byte-identical");
}

#[test]
fn the_projection_reports_what_it_could_not_express() {
    // `Field.defaultValue` is `dependent` on the field's own `fieldType`, which
    // JSON Schema cannot express — it projects to `{}` in the golden. A schema
    // that quietly emits `{}` and says nothing looks complete while
    // under-validating, so the projection must name it.
    let Some(spec) = spec_repo() else {
        eprintln!("skipping: spec repo not found (set SRS_SPEC_DIR)");
        return;
    };
    if !spec_repo_is_migrated(&spec) {
        eprintln!("skipping: sibling spec repo is pre-RFC-038 format (awaiting the #297 spec-cutover unit)");
        return;
    }
    let store = metamodel_store(&spec);
    let type_id = type_id_of(&store, "com.semanticops.srs", "field");
    let result = type_to_json_schema(
        &store,
        TypeToJsonSchemaInput {
            type_id,
            type_version: None,
        },
    )
    .expect("projection must succeed");

    assert_eq!(
        result.inexpressible.len(),
        1,
        "expected exactly the `dependent` defaultValue: {:?}",
        result.inexpressible
    );
    let reported = &result.inexpressible[0];
    assert!(reported.contains("default_value"), "{reported}");
    assert!(reported.contains("dependent"), "{reported}");

    // ...and the node it could not constrain really is the unconstrained one.
    let default_value = result
        .schema
        .properties
        .iter()
        .find(|(k, _)| *k == "defaultValue")
        .map(|(_, v)| v)
        .expect("field must project a defaultValue property");
    assert_eq!(
        *default_value,
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
        created_at: "2026-01-01T00:00:00Z".to_string(),
        extra: Default::default(),
        lineage: None,
        provenance: None,
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
        document_views: vec![],
        themes: vec![],
        blueprints: vec![],
        protocols: vec![],
        root: std::path::PathBuf::new(),
        dependency_refs: vec![],
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
