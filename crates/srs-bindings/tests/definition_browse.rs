//! Integration tests for the WASM definition-browse bindings (#330, #411):
//! list_fields, get_field, list_types, get_type, list_views, get_view, list_packages,
//! list_relation_types.
//!
//! Native Rust tests (not #[wasm_bindgen_test]) — run with `cargo test -p srs-bindings`.
//! Exercises the underlying services via srs_repository::srsj::open_srsj; to_js() is not called
//! (it calls js_sys::JSON::parse which panics off-wasm). The wasm-pack build proves the
//! #[wasm_bindgen] export compiles.
//!
//! Gallery fixture entity counts used in assertions:
//!   - 26 fields (24 namespace "governance" + 2 from implicit com.semanticops.core merge)
//!   - 5 types (4 namespace "governance" + 1 from implicit com.semanticops.core merge)
//!   - 1 L1 view (id "faebd240-83c4-4bc8-a383-8335f841a234", namespace "governance", name "decision-log")
//!   - 1 package (id "90677fae-16a7-49ec-8aee-1872cbf8e381", namespace "com.limoma", name "governance-core")
//!   - 8 relation types: 4 namespace "governance" ("delegates", "derived-from", "precedes",
//!     "evidences" — no status set) + 4 from implicit com.semanticops.core merge ("contains",
//!     "depends-on", "supersedes", "refines" — the canonical keys gallery doesn't already
//!     declare itself; "derived-from"/"precedes"/"evidences" are skipped as already-present)

use serde::Deserialize;
use srs_repository::package_service::{
    get_field_by_id, get_type_by_id_latest, list_fields_filtered, list_packages,
    list_relation_types_filtered, list_types_filtered, FieldListFilter, GetFieldResult,
    GetTypeResult, RelationTypeListFilter, TypeListFilter,
};
use srs_repository::view_service::{get_view_by_id, list_views_summary, GetViewResult};
use srs_repository::FileStore;

/// Mirrors the private `FieldListBindingFilter` / `TypeListBindingFilter` in `lib.rs`.
/// Duplicated here to test the JSON → binding filter struct → service filter mapping
/// without requiring access to the private type or calling WASM methods.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TestListBindingFilter {
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    package: Option<String>,
}

fn gallery_store() -> FileStore {
    let srsj = include_str!("../../srs-repository/tests/fixtures/gallery.srsj");
    srs_repository::srsj::open_srsj(srsj).expect("gallery srsj must load")
}

// ── list_fields ───────────────────────────────────────────────────────────────

#[test]
fn list_fields_returns_all_fields() {
    let store = gallery_store();
    let fields =
        list_fields_filtered(&store, FieldListFilter::default()).expect("list_fields must succeed");
    assert_eq!(
        fields.len(),
        26,
        "gallery has 26 fields (24 governance + 2 core)"
    );
}

#[test]
fn list_fields_filters_by_namespace() {
    let store = gallery_store();
    let fields = list_fields_filtered(
        &store,
        FieldListFilter {
            namespace: Some("governance".to_string()),
            package: None,
        },
    )
    .expect("list_fields filtered must succeed");
    assert!(!fields.is_empty(), "governance namespace has fields");
    assert!(
        fields.iter().all(|f| f.namespace == "governance"),
        "all results must have governance namespace"
    );
}

// ── get_field ─────────────────────────────────────────────────────────────────

#[test]
fn get_field_found() {
    let store = gallery_store();
    // "title" field in gallery
    let result = get_field_by_id(&store, "d7e82557-9045-5e92-a494-d99112bbec4a")
        .expect("get_field must not error");
    match result {
        GetFieldResult::Found(field) => assert_eq!(field.name, "title"),
        GetFieldResult::NotFound => panic!("expected to find field"),
    }
}

#[test]
fn get_field_not_found_returns_null_variant() {
    let store = gallery_store();
    let result = get_field_by_id(&store, "00000000-0000-0000-0000-000000000000")
        .expect("get_field must not error");
    assert!(
        matches!(result, GetFieldResult::NotFound),
        "unknown id must produce NotFound"
    );
}

// ── list_types ────────────────────────────────────────────────────────────────

#[test]
fn list_types_returns_all_types() {
    let store = gallery_store();
    let types =
        list_types_filtered(&store, TypeListFilter::default()).expect("list_types must succeed");
    assert_eq!(
        types.len(),
        5,
        "gallery has 5 types (4 governance + 1 core)"
    );
}

#[test]
fn list_types_filters_by_namespace() {
    let store = gallery_store();
    let types = list_types_filtered(
        &store,
        TypeListFilter {
            namespace: Some("governance".to_string()),
            package: None,
        },
    )
    .expect("list_types filtered must succeed");
    assert!(!types.is_empty(), "governance namespace has types");
    assert!(
        types.iter().all(|t| t.namespace == "governance"),
        "all results must have governance namespace"
    );
}

// ── get_type ──────────────────────────────────────────────────────────────────

#[test]
fn get_type_found() {
    let store = gallery_store();
    // "decision" type in gallery
    let result = get_type_by_id_latest(&store, "1fcad6a2-9f78-5e41-94ba-d82e88b822f3")
        .expect("get_type must not error");
    match result {
        GetTypeResult::Found(rt) => assert_eq!(rt.name, "decision"),
        GetTypeResult::NotFound => panic!("expected to find type"),
    }
}

#[test]
fn get_type_not_found_returns_null_variant() {
    let store = gallery_store();
    let result = get_type_by_id_latest(&store, "00000000-0000-0000-0000-000000000000")
        .expect("get_type must not error");
    assert!(
        matches!(result, GetTypeResult::NotFound),
        "unknown id must produce NotFound"
    );
}

// ── list_views ────────────────────────────────────────────────────────────────

#[test]
fn list_views_returns_all_views() {
    let store = gallery_store();
    let views = list_views_summary(&store).expect("list_views must succeed");
    assert_eq!(views.len(), 1, "gallery has 1 L1 view");
}

// ── get_view ──────────────────────────────────────────────────────────────────

#[test]
fn get_view_found() {
    let store = gallery_store();
    // "decision-log" view in gallery
    let result = get_view_by_id(&store, "faebd240-83c4-4bc8-a383-8335f841a234")
        .expect("get_view must not error");
    match result {
        GetViewResult::Found(view) => assert_eq!(view.namespace, "governance"),
        GetViewResult::NotFound => panic!("expected to find view"),
    }
}

#[test]
fn get_view_not_found_returns_null_variant() {
    let store = gallery_store();
    let result = get_view_by_id(&store, "00000000-0000-0000-0000-000000000000")
        .expect("get_view must not error");
    assert!(
        matches!(result, GetViewResult::NotFound),
        "unknown id must produce NotFound"
    );
}

// ── list_packages ─────────────────────────────────────────────────────────────

#[test]
fn list_packages_returns_primary_package() {
    let store = gallery_store();
    let packages = list_packages(&store).expect("list_packages must succeed");
    assert_eq!(packages.len(), 1, "gallery has 1 package boundary");
    assert_eq!(packages[0].id, "90677fae-16a7-49ec-8aee-1872cbf8e381");
    assert_eq!(packages[0].namespace, "com.limoma");
    assert_eq!(packages[0].name, "governance-core");
}

// ── binding filter deserialization roundtrips ─────────────────────────────────
//
// These tests exercise the JSON → binding filter struct → FieldListFilter / TypeListFilter
// mapping chain that the WASM methods perform before calling the service.  to_js() is
// still excluded (panics off-wasm), but serde_json::from_str works in native tests.

#[test]
fn field_filter_json_namespace_and_package_map_to_service_filter() {
    let store = gallery_store();
    let raw: TestListBindingFilter =
        serde_json::from_str(r#"{"namespace":"governance","package":"ext/path"}"#)
            .expect("filter json must parse");
    let filter = FieldListFilter {
        namespace: raw.namespace,
        package: raw.package.map(Some),
    };
    assert_eq!(filter.namespace, Some("governance".to_string()));
    assert_eq!(filter.package, Some(Some("ext/path".to_string())));
    // No gallery field lives under a sub-package path — result is empty but not an error.
    let fields = list_fields_filtered(&store, filter).expect("list_fields must succeed");
    assert!(
        fields.is_empty(),
        "no fields under a sub-package path in gallery"
    );
}

#[test]
fn field_filter_empty_json_produces_default_filter() {
    let store = gallery_store();
    let raw: TestListBindingFilter =
        serde_json::from_str("{}").expect("empty filter json must parse");
    let filter = FieldListFilter {
        namespace: raw.namespace,
        package: raw.package.map(Some),
    };
    assert_eq!(filter.namespace, None);
    assert_eq!(filter.package, None);
    let fields = list_fields_filtered(&store, filter).expect("list_fields must succeed");
    assert_eq!(
        fields.len(),
        26,
        "default filter returns all 26 gallery fields (24 governance + 2 core)"
    );
}

#[test]
fn type_filter_json_namespace_maps_to_service_filter() {
    let store = gallery_store();
    let raw: TestListBindingFilter =
        serde_json::from_str(r#"{"namespace":"governance"}"#).expect("filter json must parse");
    let filter = TypeListFilter {
        namespace: raw.namespace,
        package: raw.package.map(Some),
    };
    assert_eq!(filter.namespace, Some("governance".to_string()));
    assert_eq!(filter.package, None);
    let types = list_types_filtered(&store, filter).expect("list_types must succeed");
    assert_eq!(
        types.len(),
        4,
        "namespace filter returns all 4 governance types"
    );
}

// ── list_relation_types ───────────────────────────────────────────────────────

/// Mirrors the private `RelationTypeListBindingFilter` in `lib.rs`.
/// Duplicated here to test the JSON → binding filter struct → service mapping
/// without requiring access to the private type or calling WASM methods.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TestRelationTypeListBindingFilter {
    #[serde(default)]
    status: Option<String>,
}

#[test]
fn list_relation_types_returns_all_types() {
    let store = gallery_store();
    let relation_types = list_relation_types_filtered(&store, RelationTypeListFilter::default())
        .expect("list_relation_types must succeed");
    assert_eq!(
        relation_types.len(),
        8,
        "gallery has 4 of its own relation types plus 4 canonical types merged in from the \
         implicit core package (the other 3 canonical keys are already declared by gallery \
         itself and skipped)"
    );
}

#[test]
fn list_relation_types_status_filter_none_match() {
    let store = gallery_store();
    // Gallery relation types have no `status` field set; their serialized status is "".
    // A filter for "active" must return 0 results.
    let relation_types = list_relation_types_filtered(
        &store,
        RelationTypeListFilter {
            status: Some("active".to_string()),
        },
    )
    .expect("list_relation_types must succeed");
    assert!(
        relation_types.is_empty(),
        "no gallery relation types have status 'active'"
    );
}

#[test]
fn relation_type_filter_json_maps_to_service() {
    let store = gallery_store();
    let raw: TestRelationTypeListBindingFilter =
        serde_json::from_str(r#"{"status":"active"}"#).expect("filter json must parse");
    assert_eq!(raw.status, Some("active".to_string()));
    // Call the service with the deserialized filter — proving the full JSON → filter → service chain.
    let relation_types =
        list_relation_types_filtered(&store, RelationTypeListFilter { status: raw.status })
            .expect("list_relation_types must succeed");
    assert!(
        relation_types.is_empty(),
        "no gallery relation types match status 'active'"
    );
}
