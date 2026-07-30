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
    let store = metamodel_store(&spec);

    let result = schema_bundle(
        &store,
        SchemaBundleInput {
            entities: vec!["field".to_string(), "type".to_string()],
        },
    )
    .expect("bundle must emit");

    // RFC-033 [R6]: the bundle carries the revision it was generated for. The
    // spec repo is stamped 1, so this also proves the stamp is being read
    // rather than defaulted.
    assert_eq!(result.bundle.data_model_revision, 1);

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
