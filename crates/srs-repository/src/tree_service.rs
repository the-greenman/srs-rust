use crate::container_service;
use crate::error::RepositoryError;
use crate::record_label;
use crate::record_store::{get_instance_by_id, get_record_by_id, LoadedInstance};
use crate::relation_graph;
use crate::relation_service::load_relations;
use crate::store::RepositoryStore;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

pub struct TreeOptions {
    /// Explicit root instance IDs. `None` = auto-detect (records not targeted by any
    /// `relation_type` edge are roots).
    pub root_ids: Option<Vec<String>>,
    /// Scope to this container's `rootInstanceIds`.  Mutually exclusive with `root_ids`.
    pub container_id: Option<String>,
    /// Edge type to follow for parent → child traversal (default: "contains").
    pub relation_type: String,
    /// Stop recursing beyond this depth (0 = roots only, `None` = unlimited).
    pub max_depth: Option<u32>,
    /// Only include nodes whose `type_namespace/type_name` matches this string.
    pub type_filter: Option<String>,
}

impl Default for TreeOptions {
    fn default() -> Self {
        Self {
            root_ids: None,
            container_id: None,
            relation_type: "contains".to_string(),
            max_depth: None,
            type_filter: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    pub instance_id: String,
    pub label: String,
    pub type_id: String,
    pub type_version: u32,
    pub type_namespace: String,
    pub type_name: String,
    pub lifecycle_state: Option<String>,
    pub depth: u32,
    pub children: Vec<TreeNode>,
    /// True when this node was not expanded because its ID appeared in the ancestor path.
    pub cycle_pruned: bool,
    /// True when this node was already expanded once elsewhere in this same
    /// `build_tree` call (srs-rust#1117 — "expand once per tree"). Its first
    /// occurrence, decided purely by walk order, carries the real subtree;
    /// every later occurrence is a leaf stub and is never re-walked.
    pub already_expanded: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeResult {
    pub roots: Vec<TreeNode>,
    pub diagnostics: Vec<String>,
}

/// The subset of an instance's data a tree node needs to render itself and
/// sort its siblings — everything `build_node`/`child_ids` used to re-derive
/// by fully re-parsing the instance on every visit (srs-rust#1113). Built
/// once per `build_tree` call, over every catalog instance, never per visit.
struct NodeHeader {
    label: String,
    type_id: String,
    type_version: u32,
    type_namespace: String,
    type_name: String,
    lifecycle_state: Option<String>,
    created_at: Option<String>,
}

/// One pass over every instance in the catalog, computed exactly once
/// regardless of how many times (or via how many paths) the tree walk
/// visits it. `record_label::record_display_label` needs a Record's
/// `fieldValues` (for the identity-field label) and `LoadedInstance` doesn't
/// carry `createdAt` at the catalog-entry level, so this still loads each
/// instance once via `get_instance_by_id` — but once, not once per visit.
fn build_node_headers(
    store: &dyn RepositoryStore,
    identity_field_index: &HashMap<(String, u32), String>,
    field_name_index: &HashMap<String, String>,
) -> Result<HashMap<String, NodeHeader>, RepositoryError> {
    let cat = store.catalog()?;
    let mut headers = HashMap::with_capacity(cat.instances.len());
    for entry in &cat.instances {
        let Some(instance) = get_instance_by_id(store, &entry.id)? else {
            continue;
        };
        let header = match &instance {
            LoadedInstance::Record(record) => NodeHeader {
                label: record_label::record_display_label(
                    record,
                    identity_field_index,
                    field_name_index,
                ),
                type_id: record.type_id.clone(),
                type_version: record.type_version,
                type_namespace: record.type_namespace.clone(),
                type_name: record.type_name.clone(),
                lifecycle_state: record.lifecycle_state.clone(),
                created_at: instance.created_at().map(str::to_string),
            },
            // A Note has no type binding, so its title is the only label there is.
            LoadedInstance::Note(note) => NodeHeader {
                label: note.title.clone().unwrap_or_else(|| entry.id.clone()),
                type_id: String::new(),
                type_version: 0,
                type_namespace: String::new(),
                type_name: String::new(),
                lifecycle_state: None,
                created_at: instance.created_at().map(str::to_string),
            },
        };
        headers.insert(entry.id.clone(), header);
    }
    Ok(headers)
}

pub fn build_tree(
    store: &dyn RepositoryStore,
    options: TreeOptions,
) -> Result<TreeResult, RepositoryError> {
    if options.root_ids.is_some() && options.container_id.is_some() {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: "TreeOptions: root_ids and container_id are mutually exclusive".to_string(),
        });
    }

    let relations = load_relations(store)?;
    let (field_name_index, identity_field_index) = record_label::build_label_indexes(store)?;
    // RFC-034 [R1] direct membership as extra part-of children (srs-rust#1096) —
    // only meaningful for the "contains" part-of tree, never an arbitrary
    // `--relation-type` traversal.
    let container_children = if options.relation_type == "contains" {
        container_service::direct_children_by_root(store)?
    } else {
        HashMap::new()
    };
    // Single pass, once per `build_tree` call (srs-rust#1113) — the walk
    // below touches no instance files at all, however many times a shared
    // node is visited.
    let headers = build_node_headers(store, &identity_field_index, &field_name_index)?;

    let root_ids = resolve_roots(store, &options, &relations)?;

    let mut diagnostics = Vec::new();
    let mut roots = Vec::new();
    // Global to the whole `build_tree` call (srs-rust#1117), not per-root: a
    // node reached under one root and again under another (or again later
    // under the same root, off the current ancestor path) is expanded only
    // once, at its first occurrence in walk order.
    let mut expanded: HashSet<String> = HashSet::new();

    for id in &root_ids {
        let mut ancestors = HashSet::new();
        if let Some(node) = build_node(
            id,
            &relations,
            &headers,
            &options,
            0,
            &mut ancestors,
            &mut expanded,
            &mut diagnostics,
            &container_children,
        )? {
            roots.push(node);
        }
    }

    Ok(TreeResult { roots, diagnostics })
}

fn resolve_roots(
    store: &dyn RepositoryStore,
    options: &TreeOptions,
    relations: &[srs_core::types::relation::Relation],
) -> Result<Vec<String>, RepositoryError> {
    if let Some(ids) = &options.root_ids {
        return Ok(ids.clone());
    }

    if let Some(container_id) = &options.container_id {
        return container_service::list_roots(store, container_id);
    }

    // Auto-detect: records not appearing as a target of the traversal relation type.
    let target_ids: HashSet<&str> = relations
        .iter()
        .filter(|r| r.relation_type == options.relation_type)
        .map(|r| r.target_instance_id.as_str())
        .collect();

    // Walk the catalog's instance set to find all tier-2 records, filter by type
    // if requested (RFC-038: no manifest.instanceIndex).
    let cat = store.catalog()?;
    let mut root_ids = Vec::new();
    for entry in &cat.instances {
        if entry.tier != Some(2) {
            continue;
        }
        if target_ids.contains(entry.id.as_str()) {
            continue;
        }
        if let Some(filter) = &options.type_filter {
            if let Some(record) = get_record_by_id(store, &entry.id)? {
                let qualified = format!("{}/{}", record.type_namespace, record.type_name);
                if &qualified != filter {
                    continue;
                }
            } else {
                continue;
            }
        }
        root_ids.push(entry.id.clone());
    }

    Ok(root_ids)
}

#[allow(clippy::too_many_arguments)]
fn build_node(
    instance_id: &str,
    relations: &[srs_core::types::relation::Relation],
    headers: &HashMap<String, NodeHeader>,
    options: &TreeOptions,
    depth: u32,
    ancestors: &mut HashSet<String>,
    expanded: &mut HashSet<String>,
    diagnostics: &mut Vec<String>,
    container_children: &HashMap<String, Vec<String>>,
) -> Result<Option<TreeNode>, RepositoryError> {
    // Tier-aware: a Tier-0 note is a legal member of the part-of tree (RFC-013
    // scaffolds one as a section) — its header (built once in
    // `build_node_headers`) carries an empty type and no lifecycle, exactly
    // as a fresh `get_instance_by_id` parse would.
    let Some(header) = headers.get(instance_id) else {
        diagnostics.push(format!(
            "tree: instance {instance_id} does not resolve — skipped"
        ));
        return Ok(None);
    };

    // Apply type filter when visiting non-root nodes. An untyped Note never matches.
    if let Some(filter) = &options.type_filter {
        if &format!("{}/{}", header.type_namespace, header.type_name) != filter {
            return Ok(None);
        }
    }

    // Cycle check must precede everything else: a node at exactly max_depth
    // or already expanded elsewhere that is ALSO a back-edge on the current
    // path is still a cycle first (srs-rust#1117: "cycle_pruned wins").
    let node = |children: Vec<TreeNode>, cycle_pruned: bool, already_expanded: bool| TreeNode {
        instance_id: instance_id.to_string(),
        label: header.label.clone(),
        type_id: header.type_id.clone(),
        type_version: header.type_version,
        type_namespace: header.type_namespace.clone(),
        type_name: header.type_name.clone(),
        lifecycle_state: header.lifecycle_state.clone(),
        depth,
        children,
        cycle_pruned,
        already_expanded,
    };

    if ancestors.contains(instance_id) {
        return Ok(Some(node(vec![], true, false)));
    }
    // Already-expanded check precedes max_depth (srs-rust#1117): a node
    // expanded once anywhere in this tree is never re-walked, regardless of
    // how deep this second occurrence sits. Checking max_depth first would
    // silently swallow the fact that this is a duplicate (it would emit an
    // indistinguishable plain truncation stub instead) and — because a
    // max_depth truncation must NOT itself count as an expansion — would
    // also require extra bookkeeping to avoid re-truncating instead of fully
    // expanding a shallower path to the same node reached later. Checking
    // the expanded set first keeps both rules simple: only a node that is
    // actually walked to completion here is inserted into `expanded`.
    if expanded.contains(instance_id) {
        return Ok(Some(node(vec![], false, true)));
    }
    if options.max_depth.is_some_and(|max| depth >= max) {
        return Ok(Some(node(vec![], false, false)));
    }

    ancestors.insert(instance_id.to_string());
    expanded.insert(instance_id.to_string());
    let mut child_nodes = Vec::new();
    for child_id in child_ids_from_headers(
        headers,
        instance_id,
        &options.relation_type,
        relations,
        container_children,
    ) {
        if let Some(child) = build_node(
            &child_id,
            relations,
            headers,
            options,
            depth + 1,
            ancestors,
            expanded,
            diagnostics,
            container_children,
        )? {
            child_nodes.push(child);
        }
    }
    ancestors.remove(instance_id);

    Ok(Some(node(child_nodes, false, false)))
}

/// Outgoing `relation_type` targets of `source_id`, plus (when `relation_type`
/// is "contains") the direct members of any Container rooted at `source_id`
/// that a `contains` Relation doesn't already name (RFC-034 [R1], srs-rust#1096)
/// — the part-of tree's two membership sources, merged and deduplicated.
/// Ordered by the `precedes` chain among them. Resolves ids only — unlike
/// `relation_graph::children_by_relation_type` this never parses a target as a
/// Tier-2 record, so a Tier-0 note child is ordered and walked like any other
/// node.
#[derive(Clone)]
struct Child {
    id: String,
    created_at: Option<String>,
}
impl relation_graph::PrecedesSortable for Child {
    fn precedes_instance_id(&self) -> &str {
        &self.id
    }
    fn precedes_created_at(&self) -> Option<&str> {
        self.created_at.as_deref()
    }
}

/// The candidate child ids of `source_id`: outgoing `relation_type` targets
/// plus (for `contains`) direct container membership, deduplicated — the
/// part-of tree's two membership sources, merged, before either candidate
/// is checked for existence. Shared by both `child_ids` (store-backed, for
/// callers outside the header-mapped tree walk) and `child_ids_from_headers`.
fn child_candidate_ids(
    source_id: &str,
    relation_type: &str,
    relations: &[srs_core::types::relation::Relation],
    container_children: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut ids = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for rel in relations
        .iter()
        .filter(|r| r.relation_type == relation_type && r.source_instance_id == source_id)
    {
        if seen.insert(rel.target_instance_id.clone()) {
            ids.push(rel.target_instance_id.clone());
        }
    }
    for id in container_children.get(source_id).into_iter().flatten() {
        if seen.insert(id.clone()) {
            ids.push(id.clone());
        }
    }
    ids
}

/// Outgoing `relation_type` targets of `source_id`, plus (when `relation_type`
/// is "contains") the direct members of any Container rooted at `source_id`
/// that a `contains` Relation doesn't already name (RFC-034 [R1], srs-rust#1096)
/// — the part-of tree's two membership sources, merged and deduplicated.
/// Ordered by the `precedes` chain among them. Resolves ids only — unlike
/// `relation_graph::children_by_relation_type` this never parses a target as a
/// Tier-2 record, so a Tier-0 note child is ordered and walked like any other
/// node.
///
/// Store-backed: used by callers (e.g. `okf_export_service`) that don't
/// already hold a `NodeHeader` map. `tree_service::build_tree`'s own walk
/// uses `child_ids_from_headers` instead so it never re-parses an instance
/// per visit (srs-rust#1113).
pub(crate) fn child_ids(
    store: &dyn RepositoryStore,
    source_id: &str,
    relation_type: &str,
    relations: &[srs_core::types::relation::Relation],
    container_children: &HashMap<String, Vec<String>>,
) -> Result<Vec<String>, RepositoryError> {
    let mut children = Vec::new();
    for id in child_candidate_ids(source_id, relation_type, relations, container_children) {
        if let Some(instance) = get_instance_by_id(store, &id)? {
            children.push(Child {
                created_at: instance.created_at().map(str::to_string),
                id,
            });
        }
    }
    Ok(relation_graph::sort_by_precedes_chain(children, relations)
        .into_iter()
        .map(|c| c.id)
        .collect())
}

/// `child_ids`, resolved from the pre-built header map instead of the store —
/// touches no instance files. See `child_ids`'s doc comment.
fn child_ids_from_headers(
    headers: &HashMap<String, NodeHeader>,
    source_id: &str,
    relation_type: &str,
    relations: &[srs_core::types::relation::Relation],
    container_children: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut children = Vec::new();
    for id in child_candidate_ids(source_id, relation_type, relations, container_children) {
        if let Some(header) = headers.get(&id) {
            children.push(Child {
                created_at: header.created_at.clone(),
                id,
            });
        }
    }
    relation_graph::sort_by_precedes_chain(children, relations)
        .into_iter()
        .map(|c| c.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Package;
    use crate::record_store::create_record;
    use crate::relation_service::create_relation_auto;
    use crate::store::memory::MemoryStore;
    use srs_core::types::field::{AiGuidance, Field, FieldType};
    use srs_core::types::record::FieldValues;
    use srs_core::types::record_type::{FieldAssignment, RecordType};
    use srs_core::types::relation::Relation;
    use srs_core::types::relation_type_definition::{RelationTypeCategory, RelationTypeDefinition};

    fn make_field(id: &str, name: &str) -> Field {
        Field {
            schema: None,
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: String::new(),
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

    fn make_type(id: &str, name: &str, field_ids: &[&str]) -> RecordType {
        RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: String::new(),
            fields: field_ids
                .iter()
                .enumerate()
                .map(|(i, fid)| FieldAssignment {
                    field_id: fid.to_string(),
                    order: i as u32,
                    required: false,
                    display_label: None,
                    description: None,
                })
                .collect(),
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

    fn make_store(fields: Vec<Field>, types: Vec<RecordType>) -> MemoryStore {
        let manifest = crate::manifest::Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: std::path::PathBuf::from("/memory"),
        };
        let package = Package {
            id: "pkg-test".to_string(),
            namespace: "com.test".to_string(),
            name: "test-package".to_string(),
            version: "1.0.0".to_string(),
            fields,
            record_types: types,
            relation_type_definitions: vec![RelationTypeDefinition {
                schema: None,
                id: "00000000-0000-4000-8000-000000000rt1".to_string(),
                namespace: "com.test".to_string(),
                key: "contains".to_string(),
                label: "Contains".to_string(),
                description: "Containment relation".to_string(),
                category: RelationTypeCategory::Composition,
                canonical_direction: None,
                irreflexive: Some(true),
                inverse_type: None,
                version: 1,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                require_same_type: None,
                status: None,
                updated_at: None,
                meta: None,
            }],
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        MemoryStore::new(manifest, package)
    }

    fn add_record(store: &MemoryStore, type_id: &str, field_name: &str, title: &str) -> String {
        let mut fv = FieldValues::new();
        fv.insert(field_name, serde_json::json!(title));
        create_record(store, type_id, 1, fv, None, None)
            .unwrap()
            .instance_id
    }

    fn make_relation(relation_type: &str, from: &str, to: &str) -> Relation {
        Relation {
            relation_id: uuid::Uuid::new_v4().to_string(),
            relation_type: relation_type.to_string(),
            source_instance_id: from.to_string(),
            target_instance_id: to.to_string(),
            created_at: None,
            notes: None,
            source_refs: None,
            meta: None,
        }
    }

    #[test]
    fn build_tree_auto_detects_roots_and_children() {
        let store = make_store(
            vec![make_field("f-title", "title")],
            vec![make_type("t-node", "node", &["f-title"])],
        );
        let root_id = add_record(&store, "t-node", "title", "Root");
        let child_id = add_record(&store, "t-node", "title", "Child");
        create_relation_auto(&store, make_relation("contains", &root_id, &child_id)).unwrap();

        let result = build_tree(&store, TreeOptions::default()).unwrap();

        assert_eq!(result.roots.len(), 1);
        assert_eq!(result.roots[0].label, "Root");
        assert_eq!(result.roots[0].children.len(), 1);
        assert_eq!(result.roots[0].children[0].label, "Child");
        assert_eq!(result.roots[0].children[0].depth, 1);
    }

    #[test]
    fn build_tree_respects_max_depth() {
        let store = make_store(
            vec![make_field("f-title", "title")],
            vec![make_type("t-node", "node", &["f-title"])],
        );
        let root_id = add_record(&store, "t-node", "title", "Root");
        let child_id = add_record(&store, "t-node", "title", "Child");
        let grandchild_id = add_record(&store, "t-node", "title", "Grandchild");
        create_relation_auto(&store, make_relation("contains", &root_id, &child_id)).unwrap();
        create_relation_auto(&store, make_relation("contains", &child_id, &grandchild_id)).unwrap();

        let result = build_tree(
            &store,
            TreeOptions {
                max_depth: Some(1),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(result.roots[0].children.len(), 1);
        assert!(
            result.roots[0].children[0].children.is_empty(),
            "depth 1 means children are included but their children are not"
        );
    }

    #[test]
    fn build_tree_cycle_produces_pruned_node() {
        let store = make_store(
            vec![make_field("f-title", "title")],
            vec![make_type("t-node", "node", &["f-title"])],
        );
        let a_id = add_record(&store, "t-node", "title", "A");
        let b_id = add_record(&store, "t-node", "title", "B");
        create_relation_auto(&store, make_relation("contains", &a_id, &b_id)).unwrap();
        create_relation_auto(&store, make_relation("contains", &b_id, &a_id)).unwrap();

        let result = build_tree(
            &store,
            TreeOptions {
                root_ids: Some(vec![a_id.clone()]),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(result.roots.len(), 1, "should have root A");
        let b_node = &result.roots[0].children[0];
        assert_eq!(b_node.instance_id, b_id);
        assert!(
            b_node.children[0].cycle_pruned,
            "A reachable from B should be pruned"
        );
    }

    #[test]
    fn build_tree_cycle_at_max_depth_is_flagged_not_silently_truncated() {
        // Regression: cycle check must precede max_depth check. With max_depth=2 and
        // A→B→A, A is revisited at depth=2 while also being in ancestors. The old
        // code fired the max_depth arm first, returning cycle_pruned:false; the fix
        // ensures cycle_pruned:true is returned instead.
        let store = make_store(
            vec![make_field("f-title", "title")],
            vec![make_type("t-node", "node", &["f-title"])],
        );
        let a_id = add_record(&store, "t-node", "title", "A");
        let b_id = add_record(&store, "t-node", "title", "B");
        create_relation_auto(&store, make_relation("contains", &a_id, &b_id)).unwrap();
        create_relation_auto(&store, make_relation("contains", &b_id, &a_id)).unwrap();

        // max_depth=2: A(0)→B(1)→A(2). At depth=2 A is both at max_depth AND an ancestor.
        let result = build_tree(
            &store,
            TreeOptions {
                root_ids: Some(vec![a_id.clone()]),
                max_depth: Some(2),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(result.roots.len(), 1);
        let b_node = &result.roots[0].children[0];
        assert_eq!(b_node.instance_id, b_id);
        // A at depth=2 is both at max_depth and a cycle ancestor — must be cycle_pruned.
        assert_eq!(b_node.children.len(), 1, "B should have A as a cycle child");
        assert!(
            b_node.children[0].cycle_pruned,
            "A at max_depth must be flagged cycle_pruned, not silently truncated"
        );
        assert_eq!(b_node.children[0].instance_id, a_id);
    }

    #[test]
    fn build_tree_mutually_exclusive_options_returns_error() {
        let store = make_store(
            vec![make_field("f-title", "title")],
            vec![make_type("t-node", "node", &["f-title"])],
        );
        let err = build_tree(
            &store,
            TreeOptions {
                root_ids: Some(vec!["r1".to_string()]),
                container_id: Some("c1".to_string()),
                ..Default::default()
            },
        );
        assert!(err.is_err());
    }

    #[test]
    fn build_tree_type_filter_excludes_other_types() {
        let store = make_store(
            vec![make_field("f-title", "title")],
            vec![
                make_type("t-section", "section", &["f-title"]),
                make_type("t-note", "note", &["f-title"]),
            ],
        );

        let sec_id = add_record(&store, "t-section", "title", "A Section");
        let _note_id = add_record(&store, "t-note", "title", "A Note");

        let result = build_tree(
            &store,
            TreeOptions {
                type_filter: Some("com.test/section".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(result.roots.len(), 1);
        assert_eq!(result.roots[0].instance_id, sec_id);
    }

    /// Reproduction for srs-rust#1096: a section whose Container carries its
    /// members in `memberInstanceIds` (RFC-034 [R1] direct membership), with
    /// no `contains` relation at all, must still surface those members as
    /// part-of children — not render as an empty leaf.
    #[test]
    fn build_tree_surfaces_container_direct_membership_with_no_contains_relations() {
        let store = make_store(
            vec![make_field("f-title", "title")],
            vec![make_type("t-node", "node", &["f-title"])],
        );
        let section_id = add_record(&store, "t-node", "title", "The Case");
        let member_1 = add_record(&store, "t-node", "title", "Evidence Item 1");
        let member_2 = add_record(&store, "t-node", "title", "Evidence Item 2");

        // No `contains` relation anywhere — membership is declared only via the
        // Container's `rootInstanceIds`/`memberInstanceIds`, per container.json.
        crate::container_service::create_container(
            &store,
            srs_core::types::container::Container {
                container_id: "00000000-0000-4000-8000-00000000c100".to_string(),
                title: "The Case".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                anchor_instance_id: None,
                root_instance_ids: Some(vec![section_id.clone()]),
                member_instance_ids: Some(vec![member_1.clone(), member_2.clone()]),
                child_container_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            },
        )
        .unwrap();

        let result = build_tree(
            &store,
            TreeOptions {
                root_ids: Some(vec![section_id.clone()]),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(result.roots.len(), 1);
        let children: Vec<&str> = result.roots[0]
            .children
            .iter()
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(
            children,
            vec!["Evidence Item 1", "Evidence Item 2"],
            "container direct membership must surface as part-of children, got {children:?}"
        );
    }

    /// srs-rust#1117: a node reachable from two distinct roots (a DAG, not a
    /// tree) is expanded once, at its first occurrence in walk order; every
    /// later occurrence is a leaf stub flagged `already_expanded`.
    #[test]
    fn build_tree_dag_node_expanded_once_second_occurrence_is_stub() {
        let store = make_store(
            vec![make_field("f-title", "title")],
            vec![make_type("t-node", "node", &["f-title"])],
        );
        let p1_id = add_record(&store, "t-node", "title", "P1");
        let p2_id = add_record(&store, "t-node", "title", "P2");
        let c_id = add_record(&store, "t-node", "title", "C");
        let d_id = add_record(&store, "t-node", "title", "D");
        create_relation_auto(&store, make_relation("contains", &p1_id, &c_id)).unwrap();
        create_relation_auto(&store, make_relation("contains", &p2_id, &c_id)).unwrap();
        create_relation_auto(&store, make_relation("contains", &c_id, &d_id)).unwrap();

        // Explicit root order: P1 walked before P2, so P1's C is primary.
        let result = build_tree(
            &store,
            TreeOptions {
                root_ids: Some(vec![p1_id.clone(), p2_id.clone()]),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(result.roots.len(), 2);
        let c_under_p1 = &result.roots[0].children[0];
        assert_eq!(c_under_p1.instance_id, c_id);
        assert!(
            !c_under_p1.already_expanded,
            "first occurrence (under P1) must be the primary expansion"
        );
        assert_eq!(c_under_p1.children.len(), 1, "primary C keeps its child D");
        assert_eq!(c_under_p1.children[0].instance_id, d_id);

        let c_under_p2 = &result.roots[1].children[0];
        assert_eq!(c_under_p2.instance_id, c_id);
        assert!(
            c_under_p2.already_expanded,
            "second occurrence (under P2) must be flagged already_expanded"
        );
        assert!(
            c_under_p2.children.is_empty(),
            "an already-expanded stub is never re-walked"
        );
        assert!(!c_under_p2.cycle_pruned, "not a back-edge, just a dup");
    }

    /// srs-rust#1117: container membership can make the part-of graph cyclic
    /// (container A's members include container B's root and vice versa).
    /// The walk must terminate, bounded by the instance count plus stubs —
    /// not blow up per-path the way it did on muSrs (srs-rust#1113 measured
    /// 584 real nodes exploding to 936k at depth 5).
    #[test]
    fn build_tree_container_membership_cycle_terminates_and_cycle_pruned_wins() {
        let store = make_store(
            vec![make_field("f-title", "title")],
            vec![make_type("t-node", "node", &["f-title"])],
        );
        let a_id = add_record(&store, "t-node", "title", "A");
        let b_id = add_record(&store, "t-node", "title", "B");

        // No `contains` relations at all — the cycle comes entirely from
        // container membership, per srs-rust#1097.
        crate::container_service::create_container(
            &store,
            srs_core::types::container::Container {
                container_id: "00000000-0000-4000-8000-00000000ca00".to_string(),
                title: "Container A".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                anchor_instance_id: None,
                root_instance_ids: Some(vec![a_id.clone()]),
                member_instance_ids: Some(vec![b_id.clone()]),
                child_container_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            },
        )
        .unwrap();
        crate::container_service::create_container(
            &store,
            srs_core::types::container::Container {
                container_id: "00000000-0000-4000-8000-00000000cb00".to_string(),
                title: "Container B".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                anchor_instance_id: None,
                root_instance_ids: Some(vec![b_id.clone()]),
                member_instance_ids: Some(vec![a_id.clone()]),
                child_container_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            },
        )
        .unwrap();

        let result = build_tree(
            &store,
            TreeOptions {
                root_ids: Some(vec![a_id.clone()]),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(result.roots.len(), 1, "walk must terminate");
        let a_node = &result.roots[0];
        assert_eq!(a_node.children.len(), 1, "A's only child is B");
        let b_node = &a_node.children[0];
        assert_eq!(b_node.instance_id, b_id);
        assert_eq!(b_node.children.len(), 1, "B's only child is a back to A");
        let a_again = &b_node.children[0];
        assert_eq!(a_again.instance_id, a_id);
        assert!(
            a_again.cycle_pruned,
            "A is both an ancestor and already expanded here — cycle_pruned must win"
        );
        assert!(
            !a_again.already_expanded,
            "cycle_pruned wins over already_expanded when both are true"
        );
    }
}
