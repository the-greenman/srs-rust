//! Repository structural navigation service.
//!
//! Derives root identity and section navigation from the repository's root
//! container. This is the Layer-1 contract consumed by CLI/TUI/WASM clients.

use crate::container_service::{self, ContainerListFilter};
use crate::error::RepositoryError;
use crate::record_label;
use crate::record_store;
use crate::store::RepositoryStore;
use serde::{Deserialize, Serialize};
use srs_core::types::record::Record;
use std::collections::HashMap;
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
    /// RFC-043 [R12]: the entry's `depth` in the root container's outline (0 = a navigation
    /// section; deeper entries nest under the nearest preceding shallower entry). Omitted at 0.
    /// A client may show the nesting or filter on depth 0 — the spec does not choose.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub depth: u32,
    /// The part-of tree below this node: `contains` targets, ordered by the
    /// `precedes` chain among siblings (rfc-decision-0750c62f consequence 3 —
    /// navigation below the root container follows the part-of tree, not the
    /// container tree). Empty for a leaf, and omitted from the payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<NavigationNode>,
    /// Carried through from `TreeNode::already_expanded` (srs-rust#1117):
    /// true when this node was already expanded once elsewhere in the same
    /// navigation tree and this occurrence is a leaf stub, not the primary.
    /// Always `false` for a record built directly from a `Record`
    /// (`node_for_record`) — only `node_for_tree_node` can set it, matching
    /// how `cycle_pruned` is scoped to `tree_service`.
    #[serde(default)]
    pub already_expanded: bool,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
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

/// Navigation with the part-of tree expanded to its full depth.
pub fn repository_navigation(
    store: &dyn RepositoryStore,
) -> Result<RepositoryNavigation, RepositoryError> {
    repository_navigation_with_depth(store, None)
}

/// `max_depth` bounds the `contains` descent below each section: `Some(0)` is the
/// pre-#573 flat section list, `None` is unlimited. Cycles are pruned by
/// `tree_service`, which owns the one traversal.
pub fn repository_navigation_with_depth(
    store: &dyn RepositoryStore,
    max_depth: Option<u32>,
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
                .iter()
                .find(|e| &e.id == identity_id && e.tier == Some(0))
                .cloned();
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

    // RFC-043 [R12]: navigation order IS the root container's entry order (identity excluded);
    // `precedes` and Rule [N+12] are never consulted.
    let entries = root_container
        .member_instance_ids
        .clone()
        .unwrap_or_default();
    // Sections are resolved tier-aware (srs-rust#842). RFC-013 puts no tier
    // constraint on the non-identity members that are the navigation sections,
    // and RFC-029 Change B explicitly permits `repo create` to scaffold a Tier-0
    // root note as one. Reading every member through the Tier-2-only
    // `get_record_by_id` made a legitimate Tier-0 member throw a `missing field
    // typeId` parse error that took down the *whole* navigation payload — of a
    // repository `repo validate` reports as healthy.
    //
    // Tier 1 (TypedRecord) was retired (srs#448/rfc-decision-53635966,
    // srs-rust#888) and its raw-JSON shape no longer classifies at catalog
    // build, so `catalog()` — used only for existence-checking here — can no
    // longer surface a Tier-1 member for this loop to special-case: every
    // resolvable member is now Tier 0 or Tier 2, both of which load through
    // the tier-aware `get_instance_by_id` seam below.
    let cat = store.catalog()?;
    let instances = &cat.instances;
    let (section_containers, link_diagnostics) = section_containers_by_root(store)?;
    diagnostics.extend(link_diagnostics);
    let mut sections: Vec<NavigationNode> = Vec::new();
    for entry in &entries {
        let id = &entry.instance_id;
        let depth = entry.depth();
        // With no identity, nothing is excluded — every root stays in `sections`.
        if identity_id.as_deref() == Some(id.as_str()) {
            continue;
        }
        if instances.iter().all(|e| &e.id != id) {
            diagnostics.push(format!(
                "repository-navigation: root container member {id} does not resolve"
            ));
            continue;
        }
        let section_container_id = section_containers.get(id).cloned();
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
        sections.push(match instance {
            record_store::LoadedInstance::Record(record) => {
                let mut node = node_for_record(
                    &record,
                    &identity_field_index,
                    &field_name_index,
                    section_container_id,
                );
                node.depth = depth;
                node
            }
            // A Note has no type binding and no identity field, so its
            // own title is the only label there is.
            record_store::LoadedInstance::Note(note) => NavigationNode {
                display_label: note
                    .title
                    .clone()
                    .unwrap_or_else(|| note.instance_id.clone()),
                instance_id: note.instance_id,
                section_container_id,
                depth,
                ..Default::default()
            },
        });
    }

    // rfc-decision-0750c62f consequence 3: below the root container, navigation is the
    // part-of tree. One traversal, one home — `tree_service` already walks `contains`
    // with sibling `precedes` ordering, a depth bound and cycle pruning.
    if max_depth != Some(0) && !sections.is_empty() {
        let tree = crate::tree_service::build_tree(
            store,
            crate::tree_service::TreeOptions {
                root_ids: Some(sections.iter().map(|s| s.instance_id.clone()).collect()),
                max_depth,
                ..Default::default()
            },
        )?;
        diagnostics.extend(tree.diagnostics);
        let mut by_id: HashMap<String, Vec<NavigationNode>> = tree
            .roots
            .into_iter()
            .map(|root| {
                (
                    root.instance_id,
                    root.children
                        .into_iter()
                        .map(|c| node_for_tree_node(c, &section_containers))
                        .collect(),
                )
            })
            .collect();
        for section in &mut sections {
            section.children = by_id.remove(&section.instance_id).unwrap_or_default();
        }
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
        depth: 0,
        children: Vec::new(),
        already_expanded: false,
    }
}

/// A `contains`-tree node in navigation shape. A Tier-0 note never appears here:
/// `tree_service` walks Tier-2 records and diagnoses anything else.
fn node_for_tree_node(
    node: crate::tree_service::TreeNode,
    section_containers: &HashMap<String, String>,
) -> NavigationNode {
    NavigationNode {
        section_container_id: section_containers.get(&node.instance_id).cloned(),
        depth: 0,
        children: node
            .children
            .into_iter()
            .map(|c| node_for_tree_node(c, section_containers))
            .collect(),
        instance_id: node.instance_id,
        type_id: node.type_id,
        type_version: node.type_version,
        type_namespace: node.type_namespace,
        type_name: node.type_name,
        display_label: node.label,
        already_expanded: node.already_expanded,
    }
}

fn display_label(
    record: &Record,
    identity_field_index: &HashMap<(String, u32), String>,
    field_name_index: &HashMap<String, String>,
) -> String {
    record_label::record_display_label(record, identity_field_index, field_name_index)
}

/// `anchorInstanceId -> containerId` — RFC-043 [R19]: the section container of a record is
/// the Container whose `anchorInstanceId` equals it. The descent hook shared by navigation
/// sections and container members (srs-rust#949). Where more than one container names the same
/// anchor the link is ambiguous: it is omitted and a `section-container-ambiguous` diagnostic
/// is returned — never resolved by position or storage order.
pub(crate) fn section_containers_by_root(
    store: &dyn RepositoryStore,
) -> Result<(HashMap<String, String>, Vec<String>), RepositoryError> {
    let mut by_anchor: HashMap<String, Vec<String>> = HashMap::new();
    for summary in container_service::list_containers(store, &ContainerListFilter::default())? {
        let Ok(container) = container_service::get_container(store, &summary.container_id) else {
            continue;
        };
        if let Some(anchor) = container.anchor_instance_id {
            by_anchor
                .entry(anchor)
                .or_default()
                .push(summary.container_id);
        }
    }
    let mut map = HashMap::new();
    let mut diagnostics = Vec::new();
    let mut anchors: Vec<_> = by_anchor.into_iter().collect();
    anchors.sort();
    for (anchor, mut containers) in anchors {
        if containers.len() == 1 {
            map.insert(anchor, containers.remove(0));
        } else {
            containers.sort();
            diagnostics.push(format!(
                "section-container-ambiguous: anchor {anchor} is named by {} containers ({}); no section-container link is chosen (RFC-043 [R19])",
                containers.len(),
                containers.join(", ")
            ));
        }
    }
    Ok((map, diagnostics))
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
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            package_dependencies: vec![],
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
                anchor_instance_id: None,
                member_instance_ids: None,
                child_container_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            }),
            upstream_package: None,
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
                member_instance_ids: Some(srs_core::types::container::entries(vec![
                    "00000000-0000-4000-8000-00000000a100".to_string(),
                    "00000000-0000-4000-8000-00000000a300".to_string(),
                    "00000000-0000-4000-8000-00000000a200".to_string(),
                ])),
                anchor_instance_id: Some("00000000-0000-4000-8000-00000000a100".to_string()),
                child_container_ids: None,
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
                child_container_ids: None,
                anchor_instance_id: Some("00000000-0000-4000-8000-00000000a200".to_string()),
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
                child_container_ids: None,
                anchor_instance_id: Some("00000000-0000-4000-8000-00000000a300".to_string()),
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
    fn repository_navigation_returns_identity_and_entry_ordered_sections() {
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
        // RFC-043 [R12]: navigation order IS the root container's entry order (Decision Log is
        // listed before Articles); the `precedes` edge Articles -> Decision Log is ignored.
        assert_eq!(labels, vec!["Decision Log", "Articles"]);

        assert_eq!(
            nav.sections[0].section_container_id.as_deref(),
            Some("00000000-0000-4000-8000-00000000c000")
        );
        assert_eq!(
            nav.sections[1].section_container_id.as_deref(),
            Some("00000000-0000-4000-8000-00000000b000")
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
    fn tier_0_section_takes_its_entry_position() {
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
        container_service::add_member(
            &store,
            "00000000-0000-4000-8000-00000000a000",
            note_id,
            None,
            None,
        )
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
                "Decision Log",
                "Articles",
                "Middle Note"
            ],
            "the Tier-0 note sits at its entry position (appended), not sorted by createdAt"
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
                anchor_instance_id: None,
                member_instance_ids: Some(srs_core::types::container::entries(vec![
                    "00000000-0000-4000-8000-00000000a100".to_string(),
                    "00000000-0000-4000-8000-00000000a200".to_string(),
                ])),
                child_container_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            }),
            upstream_package: None,
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
                anchor_instance_id: None,
                member_instance_ids: Some(srs_core::types::container::entries(vec![
                    note_id.clone()
                ])),
                child_container_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            }),
            upstream_package: None,
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
                member_instance_ids: Some(srs_core::types::container::entries(vec![
                    "00000000-0000-4000-8000-00000000a200".to_string(),
                ])),
                anchor_instance_id: Some("00000000-0000-4000-8000-00000000a200".to_string()),
                child_container_ids: None,
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
                member_instance_ids: Some(srs_core::types::container::entries(vec![
                    "00000000-0000-4000-8000-00000000a300".to_string(),
                ])),
                anchor_instance_id: Some("00000000-0000-4000-8000-00000000a300".to_string()),
                child_container_ids: None,
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
            Some("00000000-0000-4000-8000-00000000c000")
        );
        assert_eq!(
            nav.sections[1].section_container_id.as_deref(),
            Some("00000000-0000-4000-8000-00000000b000")
        );
        assert!(nav.diagnostics.is_empty());
    }

    #[test]
    fn repository_navigation_yields_every_non_identity_entry_in_order() {
        // RFC-043 [R12]: every non-identity entry of the root container's outline is a
        // navigation entry, in entry order.
        let manifest = Manifest {
            container: Some(Container {
                container_id: "00000000-0000-4000-8000-00000000e000".to_string(),
                title: "Root IDs Only".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: Some("00000000-0000-4000-8000-00000000e100".to_string()),
                anchor_instance_id: Some("00000000-0000-4000-8000-00000000e200".to_string()),
                member_instance_ids: Some(srs_core::types::container::entries(vec![
                    "00000000-0000-4000-8000-00000000e100".to_string(),
                    "00000000-0000-4000-8000-00000000e200".to_string(),
                    "00000000-0000-4000-8000-00000000e300".to_string(),
                ])),
                child_container_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            }),
            upstream_package: None,
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
        assert_eq!(nav.sections.len(), 2, "both section entries must appear");
        let section_ids: std::collections::HashSet<&str> = nav
            .sections
            .iter()
            .map(|s| s.instance_id.as_str())
            .collect();
        assert!(section_ids.contains("00000000-0000-4000-8000-00000000e200"));
        assert!(section_ids.contains("00000000-0000-4000-8000-00000000e300"));
        assert!(nav.diagnostics.is_empty());
    }

    /// RFC-043 [R19]: two containers anchored on one record make the link ambiguous — reported,
    /// and no link is chosen by position or storage order.
    #[test]
    fn shared_anchor_reports_section_container_ambiguous_and_links_nothing() {
        let store = nav_store();
        container_service::create_container(
            &store,
            Container {
                container_id: "00000000-0000-4000-8000-00000000b001".to_string(),
                title: "Articles again".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                anchor_instance_id: Some("00000000-0000-4000-8000-00000000a200".to_string()),
                member_instance_ids: None,
                child_container_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            },
        )
        .unwrap();
        let nav = super::repository_navigation(&store).unwrap();
        let articles = nav
            .sections
            .iter()
            .find(|s| s.display_label == "Articles")
            .unwrap();
        assert!(articles.section_container_id.is_none());
        assert!(
            nav.diagnostics
                .iter()
                .any(|d| d.starts_with("section-container-ambiguous")),
            "{:?}",
            nav.diagnostics
        );
    }

    /// RFC-043 [R12]: the payload carries each entry's `depth`, in entry order.
    #[test]
    fn navigation_carries_entry_depth_in_order() {
        let store = nav_store();
        let root = "00000000-0000-4000-8000-00000000a000";
        // [identity a100, decision log a300, articles a200] -> nest articles under decision log.
        container_service::move_member(
            &store,
            root,
            "00000000-0000-4000-8000-00000000a200",
            None,
            Some(1),
        )
        .unwrap();
        let nav = super::repository_navigation(&store).unwrap();
        let shape: Vec<(&str, u32)> = nav
            .sections
            .iter()
            .map(|s| (s.display_label.as_str(), s.depth))
            .collect();
        assert_eq!(shape, vec![("Decision Log", 0), ("Articles", 1)]);
        let json = serde_json::to_value(&nav).unwrap();
        assert_eq!(json["sections"][1]["depth"], 1);
        assert!(
            json["sections"][0].get("depth").is_none(),
            "depth 0 is omitted"
        );
    }

    #[test]
    fn repository_navigation_lists_a_single_entry_section_once() {
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
                anchor_instance_id: Some("00000000-0000-4000-8000-00000000f200".to_string()),
                member_instance_ids: Some(srs_core::types::container::entries(vec![
                    "00000000-0000-4000-8000-00000000f100".to_string(),
                    "00000000-0000-4000-8000-00000000f200".to_string(),
                ])),
                child_container_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            }),
            upstream_package: None,
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

    fn add_relation(store: &MemoryStore, relation_type: &str, source: &str, target: &str) {
        crate::store::write_relations_standalone_for_test(
            store,
            &serde_json::json!({ "relations": [{
                "relationId": format!(
                    "dddddddd-{}-4000-8000-{}",
                    &source[source.len() - 4..],
                    &target[target.len() - 12..]
                ),
                "relationType": relation_type,
                "sourceInstanceId": source,
                "targetInstanceId": target,
                "createdAt": "2026-01-01T00:00:00Z"
            }]}),
        );
    }

    /// `nav_store()` plus a two-level `contains` tree under the Articles section
    /// (a200), with `precedes` putting the later-created child first.
    fn nav_store_with_part_of_tree() -> MemoryStore {
        let store = nav_store();
        let store = add_record(
            store,
            record(
                "00000000-0000-4000-8000-00000000a210",
                "Article One",
                "2026-01-04T00:00:00Z",
            ),
            "records/article-one.json",
        );
        let store = add_record(
            store,
            record(
                "00000000-0000-4000-8000-00000000a220",
                "Article Two",
                "2026-01-05T00:00:00Z",
            ),
            "records/article-two.json",
        );
        let store = add_record(
            store,
            record(
                "00000000-0000-4000-8000-00000000a221",
                "Clause 2.1",
                "2026-01-06T00:00:00Z",
            ),
            "records/clause-two-one.json",
        );
        add_relation(
            &store,
            "contains",
            "00000000-0000-4000-8000-00000000a200",
            "00000000-0000-4000-8000-00000000a210",
        );
        add_relation(
            &store,
            "contains",
            "00000000-0000-4000-8000-00000000a200",
            "00000000-0000-4000-8000-00000000a220",
        );
        add_relation(
            &store,
            "contains",
            "00000000-0000-4000-8000-00000000a220",
            "00000000-0000-4000-8000-00000000a221",
        );
        // a220 before a210, against createdAt order — proves the sibling order is
        // the `precedes` chain and not the timestamp fallback.
        add_relation(
            &store,
            "precedes",
            "00000000-0000-4000-8000-00000000a220",
            "00000000-0000-4000-8000-00000000a210",
        );
        store
    }

    /// rfc-decision-0750c62f consequence 3: below the root container, navigation
    /// descends the part-of tree, siblings in `precedes` order, at full depth.
    #[test]
    fn navigation_descends_the_contains_tree_below_each_section() {
        let store = nav_store_with_part_of_tree();
        let nav = super::repository_navigation(&store).unwrap();

        let articles = nav
            .sections
            .iter()
            .find(|s| s.instance_id.ends_with("a200"))
            .expect("Articles section");
        let labels: Vec<&str> = articles
            .children
            .iter()
            .map(|c| c.display_label.as_str())
            .collect();
        assert_eq!(labels, vec!["Article Two", "Article One"]);
        assert_eq!(
            articles.children[0]
                .children
                .iter()
                .map(|c| c.display_label.as_str())
                .collect::<Vec<_>>(),
            vec!["Clause 2.1"],
            "the descent is recursive, not one extra level"
        );
        assert_eq!(articles.children[0].type_name, "section");

        let decision_log = nav
            .sections
            .iter()
            .find(|s| s.instance_id.ends_with("a300"))
            .expect("Decision Log section");
        assert!(decision_log.children.is_empty(), "a leaf stays a leaf");
    }

    /// The container is a named scope over the one tree, not a second tree: the
    /// descent hook rides every node, at any depth.
    #[test]
    fn navigation_children_carry_the_section_container_hook() {
        let store = nav_store_with_part_of_tree();
        // b000 roots a200, which is now a child-bearing section; scope a220 too.
        container_service::create_container(
            &store,
            srs_core::types::container::Container {
                container_id: "00000000-0000-4000-8000-00000000c000".to_string(),
                title: "Article Two".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                member_instance_ids: None,
                child_container_ids: None,
                anchor_instance_id: Some("00000000-0000-4000-8000-00000000a220".to_string()),
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            },
        )
        .unwrap();

        let nav = super::repository_navigation(&store).unwrap();
        let articles = nav
            .sections
            .iter()
            .find(|s| s.instance_id.ends_with("a200"))
            .expect("Articles section");
        assert_eq!(
            articles.children[0].section_container_id.as_deref(),
            Some("00000000-0000-4000-8000-00000000c000")
        );
        assert!(articles.children[1].section_container_id.is_none());
    }

    /// `Some(0)` is the pre-#573 flat list — the escape hatch for a consumer that
    /// depends on the old shape.
    #[test]
    fn navigation_depth_zero_is_the_flat_section_list() {
        let store = nav_store_with_part_of_tree();
        let nav = super::repository_navigation_with_depth(&store, Some(0)).unwrap();
        assert!(nav.sections.iter().all(|s| s.children.is_empty()));
        // The wire shape every adapter (CLI payload, MCP resource, WASM) serves:
        // `children` is camelCase and absent, not null, when empty.
        let flat = serde_json::to_string(&nav).unwrap();
        assert!(!flat.contains("children"), "{flat}");
        let deep = serde_json::to_string(&super::repository_navigation(&store).unwrap()).unwrap();
        assert!(deep.contains("\"children\":["), "{deep}");

        let one = super::repository_navigation_with_depth(&store, Some(1)).unwrap();
        let articles = one
            .sections
            .iter()
            .find(|s| s.instance_id.ends_with("a200"))
            .expect("Articles section");
        assert_eq!(articles.children.len(), 2);
        assert!(
            articles.children[0].children.is_empty(),
            "depth 1 stops one level below the section"
        );
    }
}
