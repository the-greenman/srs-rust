use srs_repository::manifest_service::{
    add_rendered_presentation, list_rendered_presentations, remove_rendered_presentation,
    AddRenderedPresentationInput,
};
use srs_repository::view_service::create_composition;
use srs_repository::FileStore;
use srs_core::types::view::{Composition, DocumentSection, SectionSource};
use tempfile::TempDir;

fn create_minimal_repo_with_package(dir: &std::path::Path) {
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"srsVersion":"2.0-draft","repositoryId":"test-repo","dataModelRevision":2}"#,
    )
    .unwrap();
    let pkg_dir = dir.join("package");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(
        pkg_dir.join("package.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "id": "test-pkg",
            "namespace": "com.test",
            "name": "test-package",
            "version": "1.0.0",
            "fields": [],
            "types": []
        }))
        .unwrap(),
    )
    .unwrap();
}

fn create_test_composition(store: &FileStore, name: &str) -> String {
    let composition = Composition {
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
        description: "test composition".to_string(),
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
        export_config: None,
        depth_offset: None,
        theme_ref: None,
        theme_variants: None,
        tags: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
    };
    create_composition(store, composition, None).unwrap().composition.id
}

#[test]
fn presentation_add_list_remove_roundtrips_through_manifest() {
    let tmp = TempDir::new().unwrap();
    create_minimal_repo_with_package(tmp.path());
    let store = FileStore::new(tmp.path());
    let composition_id = create_test_composition(&store, "doc-a");

    let added = add_rendered_presentation(
        &store,
        AddRenderedPresentationInput {
            composition_id: composition_id.clone(),
            output_path: "../docs/spec/doc-a.md".to_string(),
            format: Some("markdown".to_string()),
            is_default: Some(true),
        },
    )
    .unwrap();
    assert_eq!(added.len(), 1);

    let listed = list_rendered_presentations(&store).unwrap();
    assert_eq!(listed, added);

    let manifest_str = std::fs::read_to_string(tmp.path().join("manifest.json")).unwrap();
    let manifest_val: serde_json::Value = serde_json::from_str(&manifest_str).unwrap();
    assert_eq!(
        manifest_val["renderedPresentations"][0]["compositionId"].as_str(),
        Some(composition_id.as_str())
    );
    assert_eq!(
        manifest_val["renderedPresentations"][0]["outputPath"].as_str(),
        Some("../docs/spec/doc-a.md")
    );
    assert_eq!(
        manifest_val["renderedPresentations"][0]["isDefault"].as_bool(),
        Some(true)
    );

    let remaining = remove_rendered_presentation(&store, &composition_id).unwrap();
    assert!(remaining.is_empty());

    let manifest_str = std::fs::read_to_string(tmp.path().join("manifest.json")).unwrap();
    let manifest_val: serde_json::Value = serde_json::from_str(&manifest_str).unwrap();
    assert!(manifest_val.get("renderedPresentations").is_none());
}

#[test]
fn presentation_add_rejects_unresolvable_composition() {
    let tmp = TempDir::new().unwrap();
    create_minimal_repo_with_package(tmp.path());
    let store = FileStore::new(tmp.path());

    let result = add_rendered_presentation(
        &store,
        AddRenderedPresentationInput {
            composition_id: "00000000-0000-4000-8000-000000000000".to_string(),
            output_path: "../docs/spec/missing.md".to_string(),
            format: None,
            is_default: None,
        },
    );
    assert!(result.is_err());
}
