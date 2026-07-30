use crate::container_service;
use crate::error::RepositoryError;
use crate::record_label;
use crate::record_store::{self, LoadedInstance};
use crate::relation_graph;
use crate::relation_service;
use crate::store::RepositoryStore;
use crate::writer::slugify_instance_name;

#[derive(Debug)]
pub struct OkfExportInput {
    pub container_id: String,
}

#[derive(Debug)]
pub struct OkfEntry {
    pub path: String,
    pub display_label: String,
    pub instance_id: String,
    pub type_label: String,
    pub field_pairs: Vec<(String, String)>,
    pub note_text: Option<String>,
}

#[derive(Debug)]
pub struct OkfBundle {
    pub container_title: String,
    pub entries: Vec<OkfEntry>,
    pub diagnostics: Vec<String>,
}

pub fn export_okf_bundle(
    store: &dyn RepositoryStore,
    input: OkfExportInput,
) -> Result<OkfBundle, RepositoryError> {
    let container = container_service::get_container(store, &input.container_id)?;
    let member_ids = container_service::list_container_members(store, &input.container_id)?;

    let (fni, ifi) = record_label::build_label_indexes(store)?;
    let all_relations = relation_service::load_relations(store)?;

    let mut instances: Vec<LoadedInstance> = Vec::new();
    let mut diagnostics: Vec<String> = Vec::new();

    for id in &member_ids {
        match record_store::get_instance_by_id(store, id)? {
            Some(inst) => instances.push(inst),
            None => diagnostics.push(format!("instance not found: {id}")),
        }
    }

    let sorted = relation_graph::sort_by_precedes_chain(instances, &all_relations);

    let entries = sorted
        .iter()
        .map(|inst| okf_entry_from_instance(inst, &fni, &ifi))
        .collect();

    Ok(OkfBundle {
        container_title: container.title,
        entries,
        diagnostics,
    })
}

fn okf_entry_from_instance(
    instance: &LoadedInstance,
    fni: &record_label::FieldNameIndex,
    ifi: &record_label::IdentityFieldIndex,
) -> OkfEntry {
    match instance {
        LoadedInstance::Record(r) => {
            let display_label = record_label::record_display_label(r, ifi, fni);
            let slug = slugify_instance_name(&display_label);
            let id8 = &r.instance_id[..r.instance_id.len().min(8)];
            let path = if slug.is_empty() {
                format!("{id8}.md")
            } else {
                format!("{slug}-{id8}.md")
            };
            let type_label = format!("{}/{}", r.type_namespace, r.type_name);
            let field_pairs = r
                .field_values
                .iter()
                .filter_map(|fv| {
                    fni.get(&fv.field_id)
                        .map(|name| (name.clone(), fv.value.to_string()))
                })
                .collect();
            OkfEntry {
                path,
                display_label,
                instance_id: r.instance_id.clone(),
                type_label,
                field_pairs,
                note_text: None,
            }
        }
        LoadedInstance::Note(n) => {
            let display_label = n
                .title
                .as_deref()
                .unwrap_or_else(|| &n.instance_id[..n.instance_id.len().min(8)])
                .to_string();
            let slug = slugify_instance_name(&display_label);
            let id8 = &n.instance_id[..n.instance_id.len().min(8)];
            let path = if slug.is_empty() {
                format!("{id8}.md")
            } else {
                format!("{slug}-{id8}.md")
            };
            let note_text = if n.sections.is_empty() {
                None
            } else {
                Some(
                    n.sections
                        .iter()
                        .map(|s| s.content.as_str())
                        .collect::<Vec<_>>()
                        .join("\n\n"),
                )
            };
            OkfEntry {
                path,
                display_label,
                instance_id: n.instance_id.clone(),
                type_label: "note".to_string(),
                field_pairs: vec![],
                note_text,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_service::{add_member, create_container};
    use crate::store::memory::MemoryStore;
    use srs_core::types::container::Container;
    use srs_core::types::note::{Note, NoteSection};
    use srs_core::types::record::Record;
    use srs_core::types::relation::Relation;
    use std::collections::HashMap;

    fn make_store() -> MemoryStore {
        MemoryStore::default()
    }

    fn minimal_container(id: &str, title: &str) -> Container {
        Container {
            container_id: id.to_string(),
            title: title.to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: HashMap::new(),
        }
    }

    fn minimal_record(id: &str, created_at: Option<&str>) -> Record {
        Record {
            instance_id: id.to_string(),
            type_id: "t-test-0001".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "item".to_string(),
            field_values: vec![],
            group_values: None,
            lifecycle_state: None,
            tags: None,
            created_at: created_at.map(|s| s.to_string()),
            updated_at: None,
            extra: HashMap::new(),
        }
    }

    fn minimal_note(id: &str, title: Option<&str>, sections: Vec<NoteSection>) -> Note {
        Note {
            instance_id: id.to_string(),
            title: title.map(|s| s.to_string()),
            tags: None,
            sections,
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        }
    }

    fn make_section(name: &str, content: &str) -> NoteSection {
        NoteSection {
            name: name.to_string(),
            label: None,
            content: content.to_string(),
            content_hint: None,
            tags: None,
        }
    }

    fn make_precedes_relation(id: &str, src: &str, tgt: &str) -> Relation {
        Relation {
            relation_id: id.to_string(),
            relation_type: "precedes".to_string(),
            source_instance_id: src.to_string(),
            target_instance_id: tgt.to_string(),
            asserted_by: None,
            confidence: None,
            created_at: None,
            created_by: None,
            status: None,
            valid_from: None,
            valid_until: None,
            notes: None,
            source_refs: None,
            meta: None,
            source_repository_id: None,
            target_repository_id: None,
        }
    }

    #[test]
    fn empty_container_returns_empty_bundle() {
        let store = make_store();
        let c = create_container(&store, minimal_container("", "Empty")).unwrap();
        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();
        assert_eq!(bundle.container_title, "Empty");
        assert!(bundle.entries.is_empty());
        assert!(bundle.diagnostics.is_empty());
    }

    #[test]
    fn record_member_produces_entry_with_correct_path_and_type_label() {
        let store = make_store();
        let r = minimal_record("rec-0001-aabb-ccdd-eeff", Some("2026-01-01T00:00:00Z"));
        store.save_record(&r).unwrap();
        let c = create_container(&store, minimal_container("", "Sprint")).unwrap();
        add_member(&store, &c.container_id, &r.instance_id).unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 1);
        let entry = &bundle.entries[0];
        assert_eq!(entry.instance_id, r.instance_id);
        assert_eq!(entry.type_label, "com.test/item");
        assert!(entry.path.ends_with(".md"));
        assert!(bundle.diagnostics.is_empty());
    }

    #[test]
    fn note_member_produces_note_text_from_sections() {
        let store = make_store();
        let sections = vec![
            make_section("intro", "First paragraph"),
            make_section("body", "Second paragraph"),
        ];
        let n = minimal_note("note-0001-aabb-ccdd-eeff", Some("My Note"), sections);
        store.save_note(&n).unwrap();
        let c = create_container(&store, minimal_container("", "Docs")).unwrap();
        add_member(&store, &c.container_id, &n.instance_id).unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 1);
        let entry = &bundle.entries[0];
        assert_eq!(entry.display_label, "My Note");
        assert_eq!(entry.type_label, "note");
        let text = entry.note_text.as_deref().unwrap();
        assert!(text.contains("First paragraph"));
        assert!(text.contains("Second paragraph"));
    }

    #[test]
    fn missing_instance_produces_diagnostic_not_error() {
        let store = make_store();
        let c = create_container(&store, minimal_container("", "Partial")).unwrap();
        add_member(&store, &c.container_id, "does-not-exist").unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert!(bundle.entries.is_empty());
        assert_eq!(bundle.diagnostics.len(), 1);
        assert!(bundle.diagnostics[0].contains("does-not-exist"));
    }

    #[test]
    fn nonexistent_container_returns_error() {
        let store = make_store();
        let err = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: "no-such-container".to_string(),
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::ContainerNotFound { container_id } if container_id == "no-such-container"
        ));
    }

    #[test]
    fn note_with_no_title_falls_back_to_id_prefix() {
        let store = make_store();
        let note_id = "abcdefgh-1111-4111-8111-111111111111";
        let n = minimal_note(note_id, None, vec![make_section("main", "Some text")]);
        store.save_note(&n).unwrap();
        let c = create_container(&store, minimal_container("", "Fallback")).unwrap();
        add_member(&store, &c.container_id, note_id).unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 1);
        let entry = &bundle.entries[0];
        // display_label falls back to first 8 chars of instance_id
        assert_eq!(entry.display_label, &note_id[..8]);
        assert!(entry.path.ends_with(".md"));
    }

    #[test]
    fn precedes_relation_orders_members() {
        let store = make_store();
        let r1 = minimal_record("r1-aaaa-0001-0000-0000", Some("2026-01-01T00:00:00Z"));
        let r2 = minimal_record("r2-aaaa-0002-0000-0000", Some("2026-01-02T00:00:00Z"));
        let r3 = minimal_record("r3-aaaa-0003-0000-0000", Some("2026-01-03T00:00:00Z"));
        store.save_record(&r1).unwrap();
        store.save_record(&r2).unwrap();
        store.save_record(&r3).unwrap();

        let c = create_container(&store, minimal_container("", "Ordered")).unwrap();
        // Add in reverse order to verify sort overrides insertion order
        add_member(&store, &c.container_id, &r3.instance_id).unwrap();
        add_member(&store, &c.container_id, &r1.instance_id).unwrap();
        add_member(&store, &c.container_id, &r2.instance_id).unwrap();

        let relations = [
            make_precedes_relation("rel-1", &r1.instance_id, &r2.instance_id),
            make_precedes_relation("rel-2", &r2.instance_id, &r3.instance_id),
        ];
        let rel_json = serde_json::json!({
            "relations": relations
                .iter()
                .map(|r| serde_json::json!({
                    "relationId": r.relation_id,
                    "relationType": r.relation_type,
                    "sourceInstanceId": r.source_instance_id,
                    "targetInstanceId": r.target_instance_id,
                }))
                .collect::<Vec<_>>()
        });
        store
            .save_relations_json("relations/relations-collection.json", &rel_json)
            .unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 3);
        assert_eq!(bundle.entries[0].instance_id, r1.instance_id);
        assert_eq!(bundle.entries[1].instance_id, r2.instance_id);
        assert_eq!(bundle.entries[2].instance_id, r3.instance_id);
    }

    #[test]
    fn record_with_field_values_produces_field_pairs() {
        use crate::manifest::Manifest;
        use crate::package::Package;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record::FieldValue;
        use std::path::PathBuf;

        let field_id = "f-title-0001-0000-0000-0000000000001".to_string();
        let field = Field {
            schema: None,
            id: field_id.clone(),
            namespace: "com.test".to_string(),
            name: "title".to_string(),
            version: 1,
            description: String::new(),
            instructions: None,
            ai_guidance: AiGuidance::default(),
            field_type: FieldType::string(),
            default_value: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            deprecated_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let manifest = Manifest {
            instance_index: vec![],
            container: None,
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: HashMap::new(),
            source_documents_path: None,
            source_document_index: None,
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![field],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = MemoryStore::new(manifest, package);

        let mut r = minimal_record("rec-field-0001-0000-0000", Some("2026-01-01T00:00:00Z"));
        r.field_values = vec![FieldValue {
            field_id: field_id.clone(),
            value: serde_json::json!("My Title"),
            entries: None,
            source: None,
            edited_at: None,
        }];
        store.save_record(&r).unwrap();
        let c = create_container(&store, minimal_container("", "Fields")).unwrap();
        add_member(&store, &c.container_id, &r.instance_id).unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 1);
        let entry = &bundle.entries[0];
        assert_eq!(entry.field_pairs.len(), 1);
        assert_eq!(entry.field_pairs[0].0, "title");
        // serde_json stringifies strings with surrounding quotes (valid JSON/YAML scalar)
        assert_eq!(entry.field_pairs[0].1, "\"My Title\"");
    }

    #[test]
    fn mixed_record_and_note_precedes_ordering_respected() {
        let store = make_store();
        let note_id = "note-mix-0001-0000-0000-000000000001";
        let rec_id = "rec--mix-0002-0000-0000-000000000001";
        let n = minimal_note(note_id, Some("First"), vec![make_section("s", "text")]);
        let r = minimal_record(rec_id, Some("2026-01-02T00:00:00Z"));
        store.save_note(&n).unwrap();
        store.save_record(&r).unwrap();

        let c = create_container(&store, minimal_container("", "Mixed")).unwrap();
        // Add record first, note second — precedes should reverse the order
        add_member(&store, &c.container_id, &r.instance_id).unwrap();
        add_member(&store, &c.container_id, &n.instance_id).unwrap();

        let rel_json = serde_json::json!({
            "relations": [{
                "relationId": "rel-mix-1",
                "relationType": "precedes",
                "sourceInstanceId": note_id,
                "targetInstanceId": rec_id,
            }]
        });
        store
            .save_relations_json("relations/relations-collection.json", &rel_json)
            .unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 2);
        assert_eq!(bundle.entries[0].instance_id, note_id);
        assert_eq!(bundle.entries[0].type_label, "note");
        assert_eq!(bundle.entries[1].instance_id, rec_id);
        assert_eq!(bundle.entries[1].type_label, "com.test/item");
    }

    #[test]
    fn display_label_that_slugifies_to_empty_uses_bare_id_path() {
        let store = make_store();
        let note_id = "abcdefgh-2222-4222-8222-222222222222";
        // Title consisting only of non-alphanumeric chars → slug is empty
        let n = minimal_note(note_id, Some("!!!"), vec![]);
        store.save_note(&n).unwrap();
        let c = create_container(&store, minimal_container("", "Symbols")).unwrap();
        add_member(&store, &c.container_id, note_id).unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 1);
        let entry = &bundle.entries[0];
        assert_eq!(entry.display_label, "!!!");
        // path should be just id8.md when slug is empty
        let id8 = &note_id[..8];
        assert_eq!(entry.path, format!("{id8}.md"));
    }
}
