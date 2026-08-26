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
    /// The repository identity node, or `None` when the root container names no
    /// `identityInstanceId` — a state RFC-029 explicitly permits. Never inferred from an
    /// unrelated record; see ADR-044.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<NavigationNode>,
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
            identity: None,
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
    // The identity comes from identityInstanceId and from nothing else. RFC-029 (line 104) makes a
    // root container with no identityInstanceId valid, so its absence is reported, never inferred
    // from the first root — inferring it would present an ordinary section as the repository's
    // identity and simultaneously drop it from `sections` (ADR-044, srs-rust#838).
    let identity_id = container_ref.identity_instance_id.clone();

    let (field_name_index, identity_field_index) = record_label::build_label_indexes(store)?;
    let mut diagnostics = Vec::new();

    let identity = match &identity_id {
        None => {
            let container_id = &container_ref.container_id;
            diagnostics.push(format!(
                "repository-navigation: root container {container_id} has no identityInstanceId; \
                 no repository identity node (RFC-029 permits this) - set one with \
                 `repo set-root-container`"
            ));
            None
        }
        Some(identity_id) => {
            let note_entry = store
                .catalog()?
                .instances
                .into_iter()
                .find(|e| &e.id == identity_id && e.tier == Some(0));
            Some(if let Some(entry) = note_entry {
                // Transitional grace for un-migrated repos whose identityInstanceId points to a
                // Tier-0 note. Surface a diagnostic and use the catalog-derived title as the
                // display label so the repo remains openable. Remove once all repos are
                // migrated to a Tier-2 purpose record (tracked in epic #262 via issues #424/#426).
                let label = crate::store::catalog_instance_ref(store, &entry)?
                    .title
                    .unwrap_or_else(|| identity_id.clone());
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
                let identity_record = record_store::get_record_by_id(store, identity_id)?
                    .ok_or_else(|| RepositoryError::NotFound {
                        path: PathBuf::from(format!("instance/{identity_id}")),
                    })?;
                node_for_record(
                    &identity_record,
                    &identity_field_index,
                    &field_name_index,
                    None,
                )
            })
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
    // Sections are resolved tier-aware (srs-rust#842). RFC-013 puts no tier
    // constraint on the non-identity members that are the navigation sections,
    // and RFC-029 Change B explicitly permits `repo create` to scaffold a Tier-0
    // root note as one. Reading every member through the Tier-2-only
    // `get_record_by_id` made a legitimate Tier-0 or Tier-1 member throw a
    // `missing field typeId` parse error that took down the *whole* navigation
    // payload — of a repository `repo validate` reports as healthy.
    let instances = store.catalog()?.instances;
    let section_containers = section_containers_by_root(store)?;
    let mut section_members = Vec::new();
    for id in &member_ids {
        // With no identity, nothing is excluded — every root stays in `sections`.
        if identity_id.as_deref() == Some(id.as_str()) {
            continue;
        }
        let Some(entry) = instances.iter().find(|e| &e.id == id) else {
            diagnostics.push(format!(
                "repository-navigation: root container member {id} does not resolve"
            ));
            continue;
        };
        let section_container_id = section_containers.get(id).cloned();
        section_members.push(if entry.tier == Some(1) {
            // Tier-1 is the one shape no loader can return: the catalog admits
            // `typed-record.json`, but `srs-core` has no TypedRecord type, so
            // `get_instance_by_id` would route it to `load_record_by_id` and
            // fail on the missing `typeId`. Fall back to the catalog-derived
            // title — the same projection the identity slot above uses. The type
            // keys stay at their defaults rather than being invented (ADR-044).
            //
            // `created_at` is unavailable here because the projection does not
            // carry one. That affects only the fallback tiebreak for a member no
            // `precedes` edge reaches — `precedes` is the ordering mechanism
            // (Rule [N+12]) — and the instance_id component keeps it a total
            // order. Closing this properly means a TypedRecord core type.
            let title = crate::store::catalog_instance_ref(store, entry)?.title;
            SectionMember {
                created_at: None,
                node: NavigationNode {
                    instance_id: id.clone(),
                    display_label: title.unwrap_or_else(|| id.clone()),
                    section_container_id,
                    ..Default::default()
                },
            }
        } else {
            // Tier 0 and Tier 2 both load through the tier-aware seam, whose own
            // doc comment names container members and roots as its reason to
            // exist. Going through it (rather than the catalog projection) keeps
            // a Tier-0 note's real `createdAt`, so it takes its rightful place in
            // the fallback ordering instead of sorting ahead of every timestamped
            // section.
            let instance = record_store::get_instance_by_id(store, id)?.ok_or_else(|| {
                RepositoryError::NotFound {
                    path: PathBuf::from(format!("instance/{id}")),
                }
            })?;
            let created_at = instance.created_at().map(str::to_string);
            SectionMember {
                created_at,
                node: match instance {
                    record_store::LoadedInstance::Record(record) => node_for_record(
                        &record,
                        &identity_field_index,
                        &field_name_index,
                        section_container_id,
                    ),
                    // A Note has no type binding and no identity field, so its
                    // own title is the only label there is.
                    record_store::LoadedInstance::Note(note) => NavigationNode {
                        display_label: note
                            .title
                            .clone()
                            .unwrap_or_else(|| note.instance_id.clone()),
                        instance_id: note.instance_id,
                        section_container_id,
                        ..Default::default()
                    },
                },
            }
        });
    }

    let relations = relation_service::load_relations(store)?;
    let sections = relation_graph::sort_by_precedes_chain(section_members, &relations)
        .into_iter()
        .map(|m| m.node)
        .collect();

    Ok(RepositoryNavigation {
        root_container_id: container_ref.container_id.clone(),
        identity,
        sections,
        diagnostics,
    })
}

/// A resolved section node awaiting `precedes` ordering. Carries `created_at`
/// separately because a section member need not be a Record (srs-rust#842).
#[derive(Clone)]
struct SectionMember {
    node: NavigationNode,
    created_at: Option<String>,
}

impl relation_graph::PrecedesSortable for SectionMember {
    fn precedes_instance_id(&self) -> &str {
        &self.node.instance_id
    }
    fn precedes_created_at(&self) -> Option<&str> {
        self.created_at.as_deref()
    }
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
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::store::memory::MemoryStore;
    use crate::store::RepositoryStore;
    use srs_core::types::container::Container;
    use srs_core::types::field::{AiGuidance, Field, FieldType};
    use srs_core::types::record::{FieldValues, Record};
    use std::path::PathBuf;

    fn empty_package() -> Package {
        Package {
            id: "pkg-nav".to_string(),
            namespace: "com.test".to_string(),
            name: "nav".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![Field {
                schema: None,
                id: "00000000-0000-4000-8000-00000000f100".to_string(),
                namespace: "governance".to_string(),
                name: "title".to_string(),
                version: 1,
                description: "Title".to_string(),
                instructions: None,
                ai_guidance: Some(AiGuidance {
                    purpose: "Test guidance".to_string(),
                    ..Default::default()
                }),
                field_type: FieldType::string(),
                editor_hint: None,
                tags: None,
                lineage: None,
                provenance: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
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
            field_meta: None,
            instance_id: id.to_string(),
            type_id: format!("type-{id}"),
            type_version: 1,
            type_namespace: "governance".to_string(),
            type_name: "section".to_string(),
            field_values: {
                let mut fv = FieldValues::new();
                fv.insert("title", serde_json::Value::String(title.to_string()));
                fv
            },
            lifecycle_state: None,
            tags: None,
            created_at: Some(created_at.to_string()),
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn add_record(store: MemoryStore, record: Record, path: &str) -> MemoryStore {
        let manifest = store.load_manifest().unwrap();
        store.save_manifest(&manifest).unwrap();
        let raw = serde_json::to_value(record).unwrap();
        store.with_data(path, raw)
    }

    fn add_precedes(store: &MemoryStore, source: &str, target: &str) {
        let raw = serde_json::json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
            "relations": [{
                "relationId": format!(
                    "eeeeeeee-{}-4000-8000-{}",
                    &source[source.len() - 4..],
                    &target[target.len() - 12..]
                ),
                "relationType": "precedes",
                "sourceInstanceId": source,
                "targetInstanceId": target,
                "createdAt": "2026-01-01T00:00:00Z"
            }]
        });
        crate::store::write_relations_standalone_for_test(store, &raw);
    }

    /// The identity node, which these fixtures always set. Absence is asserted explicitly
    /// by the `navigation_absent_identity_*` tests rather than unwrapped here.
    fn identity_of(nav: &super::RepositoryNavigation) -> &super::NavigationNode {
        nav.identity.as_ref().expect("identity present")
    }

    fn nav_store() -> MemoryStore {
        nav_store_with_identity(Some("00000000-0000-4000-8000-00000000a100".to_string()))
    }

    /// `nav_store()` with the root container's `identityInstanceId` under test control.
    /// Passing `None` builds the RFC-029-valid identity-less shape (srs-rust#838).
    ///
    /// The identity must be set on **both** the manifest embed and the materialised root
    /// container: `create_container` syncs a file-backed root back into `manifest.container`
    /// (`save_container_syncing_embed`), so an embed-only value is overwritten by the
    /// container write below. Before srs-rust#838 this fixture set it on the embed alone —
    /// the container write nulled it, and the happy-path assertions passed only because the
    /// first-root fallback re-promoted a100. That is exactly the fabrication this change removes.
    fn nav_store_with_identity(identity: Option<String>) -> MemoryStore {
        let manifest = Manifest {
            container: Some(Container {
                container_id: "00000000-0000-4000-8000-00000000a000".to_string(),
                title: String::new(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: identity.clone(),
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            }),
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
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
                identity_instance_id: identity,
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
                extra: std::collections::BTreeMap::new(),
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
                extra: std::collections::BTreeMap::new(),
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
                extra: std::collections::BTreeMap::new(),
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
            identity_of(&nav).instance_id,
            "00000000-0000-4000-8000-00000000a100"
        );
        assert_eq!(identity_of(&nav).display_label, "Example Governance");

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

    /// srs-rust#842: a Tier-0 note section must take its place in the fallback
    /// ordering by its own `createdAt`, like any other section.
    ///
    /// The fixture's sections carry no `precedes` edge between them here, so the
    /// canonical `(created_at, instance_id)` tiebreak decides. Resolving a Note
    /// through the catalog's title projection would drop its timestamp, and
    /// `sort_by_precedes_chain` keys a missing one on `""` — which sorts before
    /// every ISO timestamp, silently pinning every Tier-0 section to the front.
    #[test]
    fn tier_0_section_orders_by_its_own_created_at() {
        let store = nav_store_with_identity(None);
        // a100 = 2026-01-01, a200 = 2026-01-02 (which `precedes` a300); this
        // note is timestamped between a100 and a200, so that is where it belongs.
        let note_id = "00000000-0000-4000-8000-00000000a250";
        store
            .save_note(&srs_core::types::note::Note {
                instance_id: note_id.to_string(),
                title: Some("Middle Note".to_string()),
                tags: None,
                sections: vec![],
                graduated_at: None,
                source_refs: None,
                created_at: Some("2026-01-01T12:00:00Z".to_string()),
                updated_at: None,
                meta: None,
            })
            .unwrap();
        container_service::add_member(&store, "00000000-0000-4000-8000-00000000a000", note_id)
            .unwrap();

        let nav = super::repository_navigation(&store).unwrap();
        let labels: Vec<&str> = nav
            .sections
            .iter()
            .map(|s| s.display_label.as_str())
            .collect();
        assert_eq!(
            labels,
            vec![
                "Example Governance",
                "Middle Note",
                "Articles",
                "Decision Log"
            ],
            "the Tier-0 note must sort by its own createdAt, not ahead of everything"
        );
    }

    #[test]
    fn repository_navigation_resolves_embed_only_root_container() {
        // Root container exists ONLY as the manifest.container embed — no container file,
        // no containerIndex entry. This is the shape written by `repo set-root-container`
        // and by RFC-013 migrations of pre-container repos (e.g. the spec repo, srs#165).
        let manifest = Manifest {
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
                extra: std::collections::BTreeMap::new(),
            }),
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
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
            identity_of(&nav).instance_id,
            "00000000-0000-4000-8000-00000000a100"
        );
        assert_eq!(identity_of(&nav).display_label, "Embed Governance");
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
        assert!(nav.identity.is_none());
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
            // Embed-only root ([R1]): a containers/*.json file sharing the
            // embed's id is a fatal SRS038-R12-DUPLICATE-ID under the catalog.
            container: Some(Container {
                container_id: "00000000-0000-4000-8000-00000000a000".to_string(),
                title: "Test Repo".to_string(),
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
                extra: std::collections::BTreeMap::new(),
            }),
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        let store = MemoryStore::new(manifest, empty_package());

        // The identity note must exist as a real instance in the tree
        // (RFC-038 [R13]); its display title comes from the body.
        let mut note = serde_json::json!({
            "instanceId": note_id,
            "sections": []
        });
        if let Some(t) = note_title {
            note["title"] = serde_json::Value::String(t.to_string());
        }
        store
            .save_instance_json("records/notes/intent.json", &note)
            .unwrap();

        store
    }

    #[test]
    fn navigation_tier0_note_identity_returns_diagnostic() {
        let store = tier0_note_store(Some("Test Governance"));
        let nav = super::repository_navigation(&store).unwrap();

        assert_eq!(
            identity_of(&nav).instance_id,
            "00000000-0000-4000-8000-00000000d100"
        );
        assert_eq!(identity_of(&nav).display_label, "Test Governance");
        assert_eq!(nav.diagnostics.len(), 1);
        assert!(nav.diagnostics[0].contains("Tier-0"));
        assert!(nav.sections.is_empty());
    }

    #[test]
    fn navigation_tier0_note_identity_no_title_falls_back_to_id() {
        let store = tier0_note_store(None);
        let nav = super::repository_navigation(&store).unwrap();

        assert_eq!(
            identity_of(&nav).instance_id,
            "00000000-0000-4000-8000-00000000d100"
        );
        assert_eq!(
            identity_of(&nav).display_label,
            "00000000-0000-4000-8000-00000000d100"
        );
        assert_eq!(nav.diagnostics.len(), 1);
    }

    // navigation_tier0_identity_and_missing_member_accumulates_both_diagnostics
    // retired by RFC-038 Phase 3 (srs-rust#783): its "ghost member" premise — a
    // root-container member id with no backing instance — is now a fatal
    // SRS038-R13-DANGLING-REFERENCE at catalog build ([R24]), so
    // repository_navigation can never reach its own member-does-not-resolve
    // diagnostic branch through storage. The Tier-0-identity diagnostic half is
    // still covered by navigation_tier0_note_identity_returns_diagnostic.

    #[test]
    fn navigation_absent_identity_keeps_all_roots_as_sections() {
        // Regression for srs-rust#838: with no identityInstanceId, navigation used to promote the
        // first rootInstanceIds entry to the identity node and then exclude it from `sections` —
        // presenting an ordinary section as the repository's identity and silently dropping it
        // from navigation. RFC-029 (line 104) makes this state valid, so it must be reported,
        // not inferred (ADR-044).
        let store = nav_store_with_identity(None);

        let nav = super::repository_navigation(&store).unwrap();

        assert!(
            nav.identity.is_none(),
            "identity must be absent, not inferred from the first root"
        );

        // All three members survive as sections — including a100, which the old fallback ate.
        assert_eq!(nav.sections.len(), 3);
        let ids: Vec<&str> = nav
            .sections
            .iter()
            .map(|s| s.instance_id.as_str())
            .collect();
        assert!(
            ids.contains(&"00000000-0000-4000-8000-00000000a100"),
            "the first root must remain a section, got {ids:?}"
        );

        assert_eq!(nav.diagnostics.len(), 1, "got {:?}", nav.diagnostics);
        assert!(
            nav.diagnostics[0].contains("has no identityInstanceId"),
            "got {:?}",
            nav.diagnostics[0]
        );
        assert!(
            nav.diagnostics[0].contains("00000000-0000-4000-8000-00000000a000"),
            "diagnostic must name the root container, got {:?}",
            nav.diagnostics[0]
        );
    }

    #[test]
    fn navigation_absent_identity_omits_identity_key_in_json() {
        // ADR-044: absence is an omitted key, never an empty-string node. Locks the wire shape
        // that clients (srs-web, srs-gov) branch on.
        let store = nav_store_with_identity(None);
        let nav = super::repository_navigation(&store).unwrap();

        let json = serde_json::to_value(&nav).unwrap();
        assert!(
            json.get("identity").is_none(),
            "identity key must be omitted entirely, got {json}"
        );

        // And present when there is one, so the skip_serializing_if is not simply always-on.
        let present = super::repository_navigation(&nav_store()).unwrap();
        let present_json = serde_json::to_value(&present).unwrap();
        assert!(present_json.get("identity").is_some());
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
                extra: std::collections::BTreeMap::new(),
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
                extra: std::collections::BTreeMap::new(),
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
            container: Some(Container {
                container_id: "00000000-0000-4000-8000-00000000e000".to_string(),
                title: "Root IDs Only".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: Some("00000000-0000-4000-8000-00000000e100".to_string()),
                root_instance_ids: Some(vec![
                    "00000000-0000-4000-8000-00000000e200".to_string(),
                    "00000000-0000-4000-8000-00000000e300".to_string(),
                ]),
                member_instance_ids: Some(vec!["00000000-0000-4000-8000-00000000e100".to_string()]),
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            }),
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
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

        // Sections declared via rootInstanceIds only (in the embed itself —
        // RFC-038 [R1]: a containers/*.json file sharing the embed's id is a
        // fatal SRS038-R12-DUPLICATE-ID, so the root is embed-only).

        let nav = super::repository_navigation(&store).unwrap();

        assert_eq!(
            identity_of(&nav).instance_id,
            "00000000-0000-4000-8000-00000000e100"
        );
        assert_eq!(
            nav.sections.len(),
            2,
            "both rootInstanceIds sections must appear"
        );
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
            container: Some(Container {
                container_id: "00000000-0000-4000-8000-00000000f000".to_string(),
                title: "Dedup Test".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: Some("00000000-0000-4000-8000-00000000f100".to_string()),
                // Section ID appears in BOTH arrays (the dedup scenario), in the
                // embed itself — RFC-038 [R1]: the root container is embed-only.
                root_instance_ids: Some(vec!["00000000-0000-4000-8000-00000000f200".to_string()]),
                member_instance_ids: Some(vec![
                    "00000000-0000-4000-8000-00000000f100".to_string(),
                    "00000000-0000-4000-8000-00000000f200".to_string(),
                ]),
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            }),
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
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

        let nav = super::repository_navigation(&store).unwrap();

        assert_eq!(nav.sections.len(), 1, "duplicate ID must appear only once");
        assert_eq!(
            nav.sections[0].instance_id,
            "00000000-0000-4000-8000-00000000f200"
        );
        assert!(nav.diagnostics.is_empty());
    }
}
