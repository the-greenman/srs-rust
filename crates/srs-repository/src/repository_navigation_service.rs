//! Repository structural navigation service.
//!
//! Derives root identity and section navigation from the repository's root
//! container. This is the Layer-1 contract consumed by CLI/TUI/WASM clients.

use crate::container_service::{self, ContainerListFilter};
use crate::error::RepositoryError;
use crate::record_label;
use crate::record_store;
use crate::relation_graph;
use crate::relation_service;
use crate::store::RepositoryStore;
use serde::{Deserialize, Serialize};
use srs_core::types::record::Record;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationNode {
    pub instance_id: String,
    pub type_id: String,
    pub type_version: u32,
    pub type_namespace: String,
    pub type_name: String,
    pub display_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_container_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryNavigation {
    pub root_container_id: String,
    pub identity: NavigationNode,
    pub sections: Vec<NavigationNode>,
    pub diagnostics: Vec<String>,
}

pub fn repository_navigation(
    store: &dyn RepositoryStore,
) -> Result<RepositoryNavigation, RepositoryError> {
    let manifest = store.load_manifest()?;
    let Some(container_ref) = &manifest.container else {
        return Ok(RepositoryNavigation {
            root_container_id: String::new(),
            identity: NavigationNode::default(),
            sections: Vec::new(),
            diagnostics: vec![
                "repository-navigation: manifest.container is absent; repo predates RFC-013 root container (epic #95)"
                    .to_string(),
            ],
        });
    };

    // Prefer the materialised container; fall back to the manifest.container embed for
    // embed-only roots (the embed is the canonical repository-identity source, RFC-013).
    let root_container = container_service::resolve_root_container(store, &manifest)?
        .expect("manifest.container presence checked above");
    let identity_id = container_ref
        .identity_instance_id
        .clone()
        .or_else(|| {
            root_container
                .root_instance_ids
                .as_ref()
                .and_then(|ids| ids.first().cloned())
        })
        .ok_or_else(|| RepositoryError::NotFound {
            path: PathBuf::from("manifest.container.identityInstanceId"),
        })?;

    let (field_name_index, identity_field_index) = record_label::build_label_indexes(store)?;
    let mut diagnostics = Vec::new();

    let identity =
        {
            let note_entry = manifest
                .instance_index
                .iter()
                .find(|e| e.instance_id() == identity_id && e.is_note());
            if let Some(entry) = note_entry {
                // Transitional grace for un-migrated repos whose identityInstanceId points to a
                // Tier-0 note. Surface a diagnostic and use the index title as the display label
                // so the repo remains openable. Remove once all repos are migrated to a Tier-2
                // purpose record (tracked in epic #262 via issues #424 and #426).
                let label = entry.title().unwrap_or_else(|| identity_id.clone());
                diagnostics.push(format!(
                "repository-navigation: identity {identity_id} is a Tier-0 note (un-migrated); \
                 run identity migration to upgrade to a Tier-2 purpose record - see #426"
            ));
                NavigationNode {
                    instance_id: identity_id.clone(),
                    display_label: label,
                    ..Default::default()
                }
            } else {
                let identity_record = record_store::get_record_by_id(store, &identity_id)?
                    .ok_or_else(|| RepositoryError::NotFound {
                        path: PathBuf::from(format!("instance/{identity_id}")),
                    })?;
                node_for_record(
                    &identity_record,
                    &identity_field_index,
                    &field_name_index,
                    None,
                )
            }
        };

    // RFC-013 I-80/R2: root-container membership = memberInstanceIds ∪ rootInstanceIds.
    let member_ids: Vec<String> = {
        let mut ids: HashSet<String> = root_container
            .member_instance_ids
            .unwrap_or_default()
            .into_iter()
            .collect();
        ids.extend(root_container.root_instance_ids.unwrap_or_default());
        ids.into_iter().collect()
    };
    let mut section_records = Vec::new();
    for id in &member_ids {
        if id == &identity_id {
            continue;
        }
        match record_store::get_record_by_id(store, id)? {
            Some(record) => section_records.push(record),
            None => diagnostics.push(format!(
                "repository-navigation: root container member {id} does not resolve"
            )),
        }
    }

    let relations = relation_service::load_relations(store)?;
    let ordered_records = relation_graph::sort_by_precedes_chain(section_records, &relations);
    let section_containers = section_containers_by_root(store)?;

    let mut sections = Vec::new();
    for record in ordered_records {
        let section_container_id = section_containers.get(&record.instance_id).cloned();
        sections.push(node_for_record(
            &record,
            &identity_field_index,
            &field_name_index,
            section_container_id,
        ));
    }

    Ok(RepositoryNavigation {
        root_container_id: container_ref.container_id.clone(),
        identity,
        sections,
        diagnostics,
    })
}

fn node_for_record(
    record: &Record,
    identity_field_index: &HashMap<(String, u32), String>,
    field_name_index: &HashMap<String, String>,
    section_container_id: Option<String>,
) -> NavigationNode {
    NavigationNode {
        instance_id: record.instance_id.clone(),
        type_id: record.type_id.clone(),
        type_version: record.type_version,
        type_namespace: record.type_namespace.clone(),
        type_name: record.type_name.clone(),
        display_label: display_label(record, identity_field_index, field_name_index),
        section_container_id,
    }
}

fn display_label(
    record: &Record,
    identity_field_index: &HashMap<(String, u32), String>,
    field_name_index: &HashMap<String, String>,
) -> String {
    record_label::record_display_label(record, identity_field_index, field_name_index)
}

fn section_containers_by_root(
    store: &dyn RepositoryStore,
) -> Result<HashMap<String, String>, RepositoryError> {
    let containers = container_service::list_containers(store, &ContainerListFilter::default())?;
    Ok(containers
        .into_iter()
        .filter_map(|summary| {
            let container = container_service::get_container(store, &summary.container_id).ok()?;
            let roots = container.root_instance_ids?;
            Some((summary.container_id, roots))
        })
        .flat_map(|(container_id, roots)| {
            roots
                .into_iter()
                .map(move |root_id| (root_id, container_id.clone()))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use crate::container_service;
    use crate::index::InstanceIndexEntry;
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::store::memory::MemoryStore;
    use crate::store::RepositoryStore;
    use srs_core::types::container::Container;
    use srs_core::types::field::{Field, ValueType};
    use srs_core::types::record::{FieldValue, Record};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn empty_package() -> Package {
        Package {
            id: "pkg-nav".to_string(),
            namespace: "com.test".to_string(),
            name: "nav".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![Field {
                id: "00000000-0000-4000-8000-00000000f100".to_string(),
                namespace: "governance".to_string(),
                name: "title".to_string(),
                version: 1,
                description: "Title".to_string(),
                instructions: None,
                ai_guidance: serde_json::json!({}),
                value_type: ValueType::String,
                allowed_values: None,
                vocabulary_ref: None,
                default_value: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                extra: HashMap::new(),
            }],
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
        }
    }

    fn record(id: &str, title: &str, created_at: &str) -> Record {
        Record {
            instance_id: id.to_string(),
            type_id: format!("type-{id}"),
            type_version: 1,
            type_namespace: "governance".to_string(),
            type_name: "section".to_string(),
            field_values: vec![FieldValue {
                field_id: "00000000-0000-4000-8000-00000000f100".to_string(),
                value: serde_json::Value::String(title.to_string()),
                entries: None,
                source: None,
                edited_at: None,
            }],
            group_values: None,
            lifecycle_state: None,
            tags: None,
            created_at: Some(created_at.to_string()),
            updated_at: None,
            extra: HashMap::new(),
        }
    }

    fn add_record(store: MemoryStore, record: Record, path: &str) -> MemoryStore {
        let mut manifest = store.load_manifest().unwrap();
        manifest.instance_index.push(InstanceIndexEntry {
            instance_id: record.instance_id.clone(),
            tier: 2,
            path: path.to_string(),
            title: None,
            tags: None,
        });
        store.save_manifest(&manifest).unwrap();
        let raw = serde_json::to_value(record).unwrap();
        store.with_data(path, raw)
    }

    fn add_precedes(store: &MemoryStore, source: &str, target: &str) {
        let raw = serde_json::json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
            "relations": [{
                "relationId": format!("rel-{source}-{target}"),
                "relationType": "precedes",
                "sourceInstanceId": source,
                "targetInstanceId": target,
                "createdAt": "2026-01-01T00:00:00Z"
            }]
        });
        store
            .save_relations_json("relations/relations-collection.json", &raw)
            .unwrap();
    }

    fn nav_store() -> MemoryStore {
        let manifest = Manifest {
            instance_index: vec![],
            container: Some(Container {
                container_id: "00000000-0000-4000-8000-00000000a000".to_string(),
                title: String::new(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: Some("00000000-0000-4000-8000-00000000a100".to_string()),
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            }),
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: HashMap::new(),
            source_documents_path: None,
            source_document_index: None,
            root: PathBuf::from("/memory"),
        };
        let store = MemoryStore::new(manifest, empty_package());
        let store = add_record(
            store,
            record(
                "00000000-0000-4000-8000-00000000a100",
                "Example Governance",
                "2026-01-01T00:00:00Z",
            ),
            "records/identity.json",
        );
        let store = add_record(
            store,
            record(
                "00000000-0000-4000-8000-00000000a200",
                "Articles",
                "2026-01-02T00:00:00Z",
            ),
            "records/articles-root.json",
        );
        let store = add_record(
            store,
            record(
                "00000000-0000-4000-8000-00000000a300",
                "Decision Log",
                "2026-01-03T00:00:00Z",
            ),
            "records/decision-log-root.json",
        );

        container_service::create_container(
            &store,
            Container {
                container_id: "00000000-0000-4000-8000-00000000a000".to_string(),
                title: "Example Governance".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                member_instance_ids: Some(vec![
                    "00000000-0000-4000-8000-00000000a100".to_string(),
                    "00000000-0000-4000-8000-00000000a300".to_string(),
                    "00000000-0000-4000-8000-00000000a200".to_string(),
                ]),
                root_instance_ids: Some(vec!["00000000-0000-4000-8000-00000000a100".to_string()]),
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            },
        )
        .unwrap();

        container_service::create_container(
            &store,
            Container {
                container_id: "00000000-0000-4000-8000-00000000b000".to_string(),
                title: "Articles".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: Some("stale-hint-is-not-a-key".to_string()),
                identity_instance_id: None,
                member_instance_ids: None,
                root_instance_ids: Some(vec!["00000000-0000-4000-8000-00000000a200".to_string()]),
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            },
        )
        .unwrap();

        container_service::create_container(
            &store,
            Container {
                container_id: "00000000-0000-4000-8000-00000000c000".to_string(),
                title: "Decision Log".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: Some("another-stale-hint".to_string()),
                identity_instance_id: None,
                member_instance_ids: None,
                root_instance_ids: Some(vec!["00000000-0000-4000-8000-00000000a300".to_string()]),
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            },
        )
        .unwrap();

        add_precedes(
            &store,
            "00000000-0000-4000-8000-00000000a200",
            "00000000-0000-4000-8000-00000000a300",
        );

        store
    }

    #[test]
    fn repository_navigation_returns_identity_and_precedes_ordered_sections() {
        let store = nav_store();
        let nav = super::repository_navigation(&store).unwrap();

        assert_eq!(
            nav.identity.instance_id,
            "00000000-0000-4000-8000-00000000a100"
        );
        assert_eq!(nav.identity.display_label, "Example Governance");

        let labels: Vec<&str> = nav
            .sections
            .iter()
            .map(|section| section.display_label.as_str())
            .collect();
        assert_eq!(labels, vec!["Articles", "Decision Log"]);

        assert_eq!(
            nav.sections[0].section_container_id.as_deref(),
            Some("00000000-0000-4000-8000-00000000b000")
        );
        assert_eq!(
            nav.sections[1].section_container_id.as_deref(),
            Some("00000000-0000-4000-8000-00000000c000")
        );
        assert!(nav.diagnostics.is_empty());
    }

    #[test]
    fn repository_navigation_resolves_embed_only_root_container() {
        // Root container exists ONLY as the manifest.container embed — no container file,
        // no containerIndex entry. This is the shape written by `repo set-root-container`
        // and by RFC-013 migrations of pre-container repos (e.g. the spec repo, srs#165).
        let manifest = Manifest {
            instance_index: vec![],
            container: Some(Container {
                container_id: "00000000-0000-4000-8000-00000000a000".to_string(),
                title: "Embed Only".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: Some("00000000-0000-4000-8000-00000000a100".to_string()),
                root_instance_ids: None,
                member_instance_ids: Some(vec![
                    "00000000-0000-4000-8000-00000000a100".to_string(),
                    "00000000-0000-4000-8000-00000000a200".to_string(),
                ]),
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            }),
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: HashMap::new(),
            source_documents_path: None,
            source_document_index: None,
            root: PathBuf::from("/memory"),
        };
        let store = MemoryStore::new(manifest, empty_package());
        let store = add_record(
            store,
            record(
                "00000000-0000-4000-8000-00000000a100",
                "Embed Governance",
                "2026-01-01T00:00:00Z",
            ),
            "records/identity.json",
        );
        let store = add_record(
            store,
            record(
                "00000000-0000-4000-8000-00000000a200",
                "Articles",
                "2026-01-02T00:00:00Z",
            ),
            "records/articles-root.json",
        );

        let nav = super::repository_navigation(&store).unwrap();

        assert_eq!(
            nav.root_container_id,
            "00000000-0000-4000-8000-00000000a000"
        );
        assert_eq!(
            nav.identity.instance_id,
            "00000000-0000-4000-8000-00000000a100"
        );
        assert_eq!(nav.identity.display_label, "Embed Governance");
        assert_eq!(nav.sections.len(), 1);
        assert_eq!(nav.sections[0].display_label, "Articles");
        assert!(nav.diagnostics.is_empty(), "{:?}", nav.diagnostics);
    }

    #[test]
    fn repository_navigation_prefers_materialised_container_over_embed() {
        // When both the embed and a container file exist, the file wins — it may carry a
        // richer member list than the embed (e.g. srs-gov scaffolds).
        let store = nav_store();
        let nav = super::repository_navigation(&store).unwrap();
        // nav_store's embed has no members; the container FILE provides the sections.
        assert_eq!(nav.sections.len(), 2, "sections must come from the file");
    }

    #[test]
    fn repository_navigation_missing_manifest_container_returns_empty_with_diagnostic() {
        let store = MemoryStore::default();
        let nav = super::repository_navigation(&store).unwrap();

        assert_eq!(nav.root_container_id, "");
        assert_eq!(nav.identity.instance_id, "");
        assert!(nav.sections.is_empty());
        assert_eq!(
            nav.diagnostics,
            vec![
                "repository-navigation: manifest.container is absent; repo predates RFC-013 root container (epic #95)"
                    .to_string()
            ]
        );
    }

    fn tier0_note_store(note_title: Option<&str>) -> MemoryStore {
        let note_id = "00000000-0000-4000-8000-00000000d100".to_string();
        let manifest = Manifest {
            instance_index: vec![InstanceIndexEntry {
                instance_id: note_id.clone(),
                tier: 0,
                path: "records/notes/intent.json".to_string(),
                title: note_title.map(|t| serde_json::Value::String(t.to_string())),
                tags: None,
            }],
            container: Some(Container {
                container_id: "00000000-0000-4000-8000-00000000a000".to_string(),
                title: String::new(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: Some(note_id.clone()),
                root_instance_ids: None,
                member_instance_ids: Some(vec![note_id.clone()]),
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            }),
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: HashMap::new(),
            source_documents_path: None,
            source_document_index: None,
            root: PathBuf::from("/memory"),
        };
        let store = MemoryStore::new(manifest, empty_package());

        container_service::create_container(
            &store,
            Container {
                container_id: "00000000-0000-4000-8000-00000000a000".to_string(),
                title: "Test Repo".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                member_instance_ids: Some(vec![note_id]),
                root_instance_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            },
        )
        .unwrap();

        store
    }

    #[test]
    fn navigation_tier0_note_identity_returns_diagnostic() {
        let store = tier0_note_store(Some("Test Governance"));
        let nav = super::repository_navigation(&store).unwrap();

        assert_eq!(
            nav.identity.instance_id,
            "00000000-0000-4000-8000-00000000d100"
        );
        assert_eq!(nav.identity.display_label, "Test Governance");
        assert_eq!(nav.diagnostics.len(), 1);
        assert!(nav.diagnostics[0].contains("Tier-0"));
        assert!(nav.sections.is_empty());
    }

    #[test]
    fn navigation_tier0_note_identity_no_title_falls_back_to_id() {
        let store = tier0_note_store(None);
        let nav = super::repository_navigation(&store).unwrap();

        assert_eq!(
            nav.identity.instance_id,
            "00000000-0000-4000-8000-00000000d100"
        );
        assert_eq!(
            nav.identity.display_label,
            "00000000-0000-4000-8000-00000000d100"
        );
        assert_eq!(nav.diagnostics.len(), 1);
    }

    #[test]
    fn navigation_tier0_identity_and_missing_member_accumulates_both_diagnostics() {
        // Verify that diagnostics from the identity branch and the sections loop both
        // accumulate into the same vec — guards against accidentally re-declaring `diagnostics`
        // between the two push sites.
        let note_id = "00000000-0000-4000-8000-00000000d100".to_string();
        let ghost_id = "00000000-0000-4000-8000-00000000ffff".to_string();
        let manifest = Manifest {
            instance_index: vec![InstanceIndexEntry {
                instance_id: note_id.clone(),
                tier: 0,
                path: "records/notes/intent.json".to_string(),
                title: Some(serde_json::Value::String("Test Gov".to_string())),
                tags: None,
            }],
            container: Some(Container {
                container_id: "00000000-0000-4000-8000-00000000a000".to_string(),
                title: String::new(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: Some(note_id.clone()),
                root_instance_ids: None,
                member_instance_ids: Some(vec![note_id.clone(), ghost_id.clone()]),
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            }),
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: HashMap::new(),
            source_documents_path: None,
            source_document_index: None,
            root: PathBuf::from("/memory"),
        };
        let store = MemoryStore::new(manifest, empty_package());
        container_service::create_container(
            &store,
            Container {
                container_id: "00000000-0000-4000-8000-00000000a000".to_string(),
                title: "Test Repo".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                member_instance_ids: Some(vec![note_id, ghost_id]),
                root_instance_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            },
        )
        .unwrap();

        let nav = super::repository_navigation(&store).unwrap();
        assert_eq!(nav.diagnostics.len(), 2);
        assert!(nav.diagnostics.iter().any(|d| d.contains("Tier-0")));
        assert!(nav
            .diagnostics
            .iter()
            .any(|d| d.contains("does not resolve")));
    }

    #[test]
    fn repository_navigation_root_is_member_of_its_own_sub_container() {
        // Regression for: section_containers_by_root previously excluded root→container
        // mappings when the root record also appeared in member_instance_ids, silently
        // producing sectionContainerId: null for all sections in real governance repos.
        let store = nav_store();

        // Replace sub-containers b000 and c000 with variants where each root record
        // is also listed as a member of its own container (the "root is also a member" shape).
        // create_container overwrites an existing container when the container_id matches.
        container_service::create_container(
            &store,
            Container {
                container_id: "00000000-0000-4000-8000-00000000b000".to_string(),
                title: "Articles".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                member_instance_ids: Some(vec!["00000000-0000-4000-8000-00000000a200".to_string()]),
                root_instance_ids: Some(vec!["00000000-0000-4000-8000-00000000a200".to_string()]),
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            },
        )
        .unwrap();

        container_service::create_container(
            &store,
            Container {
                container_id: "00000000-0000-4000-8000-00000000c000".to_string(),
                title: "Decision Log".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                member_instance_ids: Some(vec!["00000000-0000-4000-8000-00000000a300".to_string()]),
                root_instance_ids: Some(vec!["00000000-0000-4000-8000-00000000a300".to_string()]),
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            },
        )
        .unwrap();

        let nav = super::repository_navigation(&store).unwrap();

        assert_eq!(nav.sections.len(), 2);
        assert_eq!(
            nav.sections[0].section_container_id.as_deref(),
            Some("00000000-0000-4000-8000-00000000b000")
        );
        assert_eq!(
            nav.sections[1].section_container_id.as_deref(),
            Some("00000000-0000-4000-8000-00000000c000")
        );
        assert!(nav.diagnostics.is_empty());
    }

    #[test]
    fn repository_navigation_root_instance_ids_only_yields_same_sections() {
        // Regression for: rootInstanceIds was effectively dead for the root container.
        // RFC-013 I-80/R2: membership = memberInstanceIds ∪ rootInstanceIds.
        // This test constructs a root container where all section IDs are in
        // rootInstanceIds only (memberInstanceIds contains only the identity), and
        // asserts that navigation returns the same sections as if they were in memberInstanceIds.
        let manifest = Manifest {
            instance_index: vec![],
            container: Some(Container {
                container_id: "00000000-0000-4000-8000-00000000e000".to_string(),
                title: "Root IDs Only".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: Some("00000000-0000-4000-8000-00000000e100".to_string()),
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            }),
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: HashMap::new(),
            source_documents_path: None,
            source_document_index: None,
            root: PathBuf::from("/memory"),
        };
        let store = MemoryStore::new(manifest, empty_package());
        let store = add_record(
            store,
            record(
                "00000000-0000-4000-8000-00000000e100",
                "Root IDs Governance",
                "2026-01-01T00:00:00Z",
            ),
            "records/identity.json",
        );
        let store = add_record(
            store,
            record(
                "00000000-0000-4000-8000-00000000e200",
                "Section Alpha",
                "2026-01-02T00:00:00Z",
            ),
            "records/section-alpha.json",
        );
        let store = add_record(
            store,
            record(
                "00000000-0000-4000-8000-00000000e300",
                "Section Beta",
                "2026-01-03T00:00:00Z",
            ),
            "records/section-beta.json",
        );

        // Sections declared via rootInstanceIds only; memberInstanceIds has only the identity.
        container_service::create_container(
            &store,
            Container {
                container_id: "00000000-0000-4000-8000-00000000e000".to_string(),
                title: "Root IDs Only".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                member_instance_ids: Some(vec![
                    "00000000-0000-4000-8000-00000000e100".to_string(),
                ]),
                root_instance_ids: Some(vec![
                    "00000000-0000-4000-8000-00000000e200".to_string(),
                    "00000000-0000-4000-8000-00000000e300".to_string(),
                ]),
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            },
        )
        .unwrap();

        let nav = super::repository_navigation(&store).unwrap();

        assert_eq!(
            nav.identity.instance_id,
            "00000000-0000-4000-8000-00000000e100"
        );
        assert_eq!(nav.sections.len(), 2, "both rootInstanceIds sections must appear");
        let section_ids: std::collections::HashSet<&str> = nav
            .sections
            .iter()
            .map(|s| s.instance_id.as_str())
            .collect();
        assert!(section_ids.contains("00000000-0000-4000-8000-00000000e200"));
        assert!(section_ids.contains("00000000-0000-4000-8000-00000000e300"));
        assert!(nav.diagnostics.is_empty());
    }

    #[test]
    fn repository_navigation_union_deduplicates_ids_in_both_arrays() {
        // When an ID appears in both rootInstanceIds and memberInstanceIds, it must appear
        // as a section exactly once (no duplicate NavigationNode).
        let manifest = Manifest {
            instance_index: vec![],
            container: Some(Container {
                container_id: "00000000-0000-4000-8000-00000000f000".to_string(),
                title: "Dedup Test".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: Some("00000000-0000-4000-8000-00000000f100".to_string()),
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            }),
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: HashMap::new(),
            source_documents_path: None,
            source_document_index: None,
            root: PathBuf::from("/memory"),
        };
        let store = MemoryStore::new(manifest, empty_package());
        let store = add_record(
            store,
            record(
                "00000000-0000-4000-8000-00000000f100",
                "Dedup Governance",
                "2026-01-01T00:00:00Z",
            ),
            "records/identity.json",
        );
        let store = add_record(
            store,
            record(
                "00000000-0000-4000-8000-00000000f200",
                "Section Gamma",
                "2026-01-02T00:00:00Z",
            ),
            "records/section-gamma.json",
        );

        // Section ID appears in BOTH arrays.
        container_service::create_container(
            &store,
            Container {
                container_id: "00000000-0000-4000-8000-00000000f000".to_string(),
                title: "Dedup Test".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                member_instance_ids: Some(vec![
                    "00000000-0000-4000-8000-00000000f100".to_string(),
                    "00000000-0000-4000-8000-00000000f200".to_string(),
                ]),
                root_instance_ids: Some(vec![
                    "00000000-0000-4000-8000-00000000f200".to_string(),
                ]),
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            },
        )
        .unwrap();

        let nav = super::repository_navigation(&store).unwrap();

        assert_eq!(nav.sections.len(), 1, "duplicate ID must appear only once");
        assert_eq!(
            nav.sections[0].instance_id,
            "00000000-0000-4000-8000-00000000f200"
        );
        assert!(nav.diagnostics.is_empty());
    }
}
