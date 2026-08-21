//! Cross-service package-selector parity tests (#507).
//!
//! Every definition-create service must accept the same boundary selector form —
//! the boundary's repo-root-relative path as registered by `package create --path <path>`
//! (e.g. `packages/governance`) — and must reject unsafe selectors via the single shared
//! validator in [`crate::package_types::validate_package_selector`]. These tests pin that
//! parity so no service can grow its own divergent selector convention again.

use crate::blueprint_service::create_blueprint;
use crate::lifecycle_service::create_lifecycle;
use crate::package_service::{
    create_field_in_package, create_relation_type, create_type_in_package,
};
use crate::protocol_service::create_protocol;
use crate::store::memory::MemoryStore;
use crate::store::RepositoryStore;
use crate::theme_service::create_theme;
use crate::view_service::{create_document_view, create_view};
use srs_core::types::blueprint::{Blueprint, TypeRef};
use srs_core::types::field::{AiGuidance, Field, FieldType};
use srs_core::types::lifecycle::{Lifecycle, LifecycleState, LifecycleTransition};
use srs_core::types::protocol::Protocol;
use srs_core::types::record_type::RecordType;
use srs_core::types::relation_type_definition::{RelationTypeCategory, RelationTypeDefinition};
use srs_core::types::theme::Theme;
use srs_core::types::view::{DocumentSection, DocumentView, FieldView, SectionSource, View};

const BOUNDARY: &str = "packages/governance";

fn store_with_boundary() -> MemoryStore {
    let store = MemoryStore::default();
    store
        .register_package_boundary(&Some(BOUNDARY.to_string()))
        .unwrap();
    store
}

fn selector() -> Option<String> {
    Some(BOUNDARY.to_string())
}

fn make_field(id: &str, name: &str) -> Field {
    Field {
        schema: None,
        id: id.to_string(),
        namespace: "com.test".to_string(),
        name: name.to_string(),
        version: 1,
        field_type: FieldType::string(),
        description: "A test field".to_string(),
        instructions: None,
        ai_guidance: Some(AiGuidance {
            purpose: "Test guidance".to_string(),
            ..Default::default()
        }),
        editor_hint: None,
        tags: None,
        lineage: None,
        provenance: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn make_type(id: &str, name: &str) -> RecordType {
    RecordType {
        schema: None,
        ai_guidance: None,
        semantic_object_type: None,
        tags: None,
        id: id.to_string(),
        namespace: "com.test".to_string(),
        name: name.to_string(),
        version: 1,
        description: "A test type".to_string(),
        fields: vec![],
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
    }
}

fn make_view(name: &str) -> View {
    View {
        schema: None,
        ai_guidance: None,
        lineage: None,
        provenance: None,
        updated_at: None,
        id: String::new(),
        namespace: "com.test".to_string(),
        name: name.to_string(),
        version: 1,
        description: "test view".to_string(),
        field_views: vec![FieldView {
            display_hint: None,
            editor_hint_override: None,
            composite_renderer: None,
            field_id: "f1".to_string(),
            order: 0,
            required: None,
            visible: None,
            display_label: None,
        }],
        compatible_types: None,
        protection: None,
        export_config: None,
        tags: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn make_document_view(name: &str) -> DocumentView {
    DocumentView {
        schema: None,
        ai_guidance: None,
        lineage: None,
        provenance: None,
        updated_at: None,
        composite_renderers: None,
        id: String::new(),
        namespace: "com.test".to_string(),
        name: name.to_string(),
        version: 1,
        description: "test doc view".to_string(),
        container_type: None,
        root_type_refs: None,
        sections: vec![DocumentSection {
            composite_renderers: None,
            section_id: "s1".to_string(),
            title: None,
            description: None,
            order: 0,
            source: SectionSource::FixedInstances {
                instance_ids: vec![],
            },
            render_view_id: None,
            type_dispatch: None,
            title_field_id: None,
            ordering: None,
            required: None,
            empty_behavior: None,
            relations_presentation: None,
        }],
        navigation_links: None,
        preamble: None,
        format: None,
        depth_offset: None,
        theme_ref: None,
        theme_variants: None,
        tags: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn make_theme(name: &str) -> Theme {
    Theme {
        schema: None,
        lineage: None,
        provenance: None,
        updated_at: None,
        id: String::new(),
        namespace: "com.test".to_string(),
        name: name.to_string(),
        version: 1,
        description: "test theme".to_string(),
        targets: vec!["markdown".to_string()],
        assets: None,
        css_class_fields: None,
        page_templates: None,
        element_templates: None,
        stylesheet: None,
        typography: None,
        tags: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn make_blueprint(name: &str) -> Blueprint {
    Blueprint {
        schema: None,
        id: String::new(),
        namespace: "test".to_string(),
        name: name.to_string(),
        version: 1,
        description: "test blueprint".to_string(),
        root_types: vec![TypeRef {
            type_id: "core/decision".to_string(),
            type_version: None,
        }],
        structure: vec![],
        required_types: vec![],
        ai_guidance: None,
        tags: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        lineage: None,
        provenance: None,
    }
}

fn make_protocol_value(id: &str, name: &str) -> serde_json::Value {
    serde_json::to_value(Protocol {
        schema: None,
        protocol_id: id.to_string(),
        protocol_namespace: "com.test".to_string(),
        protocol_name: name.to_string(),
        protocol_version: 1,
        protocol_description: None,
        protocol_target_type: "type-a".to_string(),
        protocol_stages: vec![],
        protocol_tags: None,
        protocol_created_at: "2026-01-01T00:00:00Z".to_string(),
    })
    .unwrap()
}

fn make_relation_type(id: &str, key: &str) -> RelationTypeDefinition {
    RelationTypeDefinition {
        schema: None,
        id: id.to_string(),
        version: 1,
        key: key.to_string(),
        namespace: "com.test".to_string(),
        label: "Test Link".to_string(),
        description: "A test relation type".to_string(),
        category: RelationTypeCategory::Dependency,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        status: None,
        canonical_direction: None,
        inverse_type: None,
        irreflexive: None,
        allowed_source_types: None,
        allowed_target_types: None,
        require_same_semantic_object_type: None,
        updated_at: None,
        properties: None,
    }
}

fn make_lifecycle(name: &str) -> Lifecycle {
    Lifecycle {
        schema: None,
        tags: None,
        id: String::new(),
        version: 1,
        namespace: "com.test".to_string(),
        name: name.to_string(),
        states: vec![
            LifecycleState {
                id: Some("s1".to_string()),
                version: None,
                namespace: None,
                key: "draft".to_string(),
                label: None,
                description: None,
                aliases: None,
                is_initial: Some(true),
                is_final: None,
                status: None,
                requires_relation: None,
                properties: None,
            },
            LifecycleState {
                id: Some("s2".to_string()),
                version: None,
                namespace: None,
                key: "active".to_string(),
                label: None,
                description: None,
                aliases: None,
                is_initial: None,
                is_final: Some(true),
                status: None,
                requires_relation: None,
                properties: None,
            },
        ],
        transitions: vec![LifecycleTransition {
            id: Some("t1".to_string()),
            name: "publish".to_string(),
            from: "draft".to_string(),
            to: "active".to_string(),
            description: None,
            properties: None,
        }],
        initial_state: "draft".to_string(),
        extends_lifecycle_id: None,
        extends_lifecycle_version: None,
        description: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

/// Assert that a definition file was written under the boundary directory and that
/// its relative path is registered in the boundary's `package.json` array.
fn assert_in_boundary(store: &MemoryStore, dir: &str, pkg_json_key: &str) {
    let data = store.all_data();
    let prefix = format!("{BOUNDARY}/{dir}/");
    assert!(
        data.keys().any(|k| k.starts_with(&prefix)),
        "expected a definition file under {prefix}"
    );
    let pkg_json = data
        .get(&format!("{BOUNDARY}/package.json"))
        .unwrap_or_else(|| panic!("missing {BOUNDARY}/package.json"));
    assert!(
        pkg_json[pkg_json_key]
            .as_array()
            .unwrap_or_else(|| panic!("{pkg_json_key} missing in boundary package.json"))
            .iter()
            .any(|v| v.as_str().unwrap_or("").starts_with(&format!("{dir}/"))),
        "expected {pkg_json_key} entry in {BOUNDARY}/package.json"
    );
}

#[test]
fn all_definition_creates_accept_packages_prefixed_boundary() {
    let store = store_with_boundary();
    let sel = selector();

    create_field_in_package(
        &store,
        make_field("00000000-0000-0000-0000-000000000001", "gov-field"),
        sel.clone(),
    )
    .unwrap();
    assert_in_boundary(&store, "fields", "fields");

    create_type_in_package(
        &store,
        make_type("00000000-0000-0000-0000-000000000002", "gov-type"),
        sel.clone(),
    )
    .unwrap();
    assert_in_boundary(&store, "types", "types");

    create_view(&store, make_view("gov-view"), sel.clone()).unwrap();
    assert_in_boundary(&store, "views", "views");

    create_document_view(&store, make_document_view("gov-doc-view"), sel.clone()).unwrap();
    assert_in_boundary(&store, "document-views", "documentViews");

    create_theme(&store, make_theme("gov-theme"), sel.clone()).unwrap();
    assert_in_boundary(&store, "themes", "themes");

    create_blueprint(&store, make_blueprint("gov-bp"), sel.clone()).unwrap();
    assert_in_boundary(&store, "blueprints", "blueprints");

    create_protocol(
        &store,
        make_protocol_value("proto-gov-001", "gov-proto"),
        sel.clone(),
    )
    .unwrap();
    assert_in_boundary(&store, "protocols", "protocols");

    create_relation_type(
        &store,
        make_relation_type("rt-gov-001", "gov-link"),
        sel.clone(),
    )
    .unwrap();
    assert_in_boundary(&store, "relation-types", "relationTypes");

    create_lifecycle(&store, make_lifecycle("gov-lifecycle"), sel).unwrap();
    assert_in_boundary(&store, "lifecycles", "lifecycles");
}

#[test]
fn all_definition_creates_reject_path_traversal_selector() {
    let store = store_with_boundary();
    let evil = Some("packages/../evil".to_string());

    assert!(create_field_in_package(
        &store,
        make_field("00000000-0000-0000-0000-00000000000a", "evil-field"),
        evil.clone()
    )
    .is_err());
    assert!(create_type_in_package(
        &store,
        make_type("00000000-0000-0000-0000-00000000000b", "evil-type"),
        evil.clone()
    )
    .is_err());
    assert!(create_view(&store, make_view("evil-view"), evil.clone()).is_err());
    assert!(create_document_view(&store, make_document_view("evil-dv"), evil.clone()).is_err());
    assert!(create_theme(&store, make_theme("evil-theme"), evil.clone()).is_err());
    assert!(create_blueprint(&store, make_blueprint("evil-bp"), evil.clone()).is_err());
    assert!(create_protocol(
        &store,
        make_protocol_value("proto-evil-001", "evil-proto"),
        evil.clone()
    )
    .is_err());
    assert!(create_relation_type(
        &store,
        make_relation_type("rt-evil-001", "evil-link"),
        evil.clone()
    )
    .is_err());
    assert!(create_lifecycle(&store, make_lifecycle("evil-lifecycle"), evil).is_err());
}

#[test]
fn all_definition_creates_reject_unregistered_boundary() {
    // Selector form is valid but no such boundary is registered — every create
    // must fail with PackageNotFound rather than writing into a phantom directory.
    let store = MemoryStore::default();
    let missing = Some("packages/nonexistent".to_string());

    assert!(create_field_in_package(
        &store,
        make_field("00000000-0000-0000-0000-00000000000c", "orphan-field"),
        missing.clone()
    )
    .is_err());
    assert!(create_blueprint(&store, make_blueprint("orphan-bp"), missing.clone()).is_err());
    assert!(create_relation_type(
        &store,
        make_relation_type("rt-orphan-001", "orphan-link"),
        missing.clone()
    )
    .is_err());
    assert!(create_lifecycle(&store, make_lifecycle("orphan-lifecycle"), missing).is_err());
}
