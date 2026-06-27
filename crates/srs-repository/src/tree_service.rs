use crate::container_service;
use crate::error::RepositoryError;
use crate::record_label;
use crate::record_store::get_record_by_id;
use crate::relation_graph;
use crate::relation_service::load_relations;
use crate::store::RepositoryStore;
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

pub struct TreeNode {
    pub instance_id: String,
    pub label: String,
    pub type_namespace: String,
    pub type_name: String,
    pub lifecycle_state: Option<String>,
    pub depth: u32,
    pub children: Vec<TreeNode>,
    /// True when this node was not expanded because its ID appeared in the ancestor path.
    pub cycle_pruned: bool,
}

pub struct TreeResult {
    pub roots: Vec<TreeNode>,
    pub diagnostics: Vec<String>,
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
    let field_name_index = record_label::build_field_name_index(store)?;

    let root_ids = resolve_roots(store, &options, &relations)?;

    let mut diagnostics = Vec::new();
    let mut roots = Vec::new();

    for id in &root_ids {
        let mut ancestors = HashSet::new();
        if let Some(node) = build_node(
            store,
            id,
            &relations,
            &field_name_index,
            &options,
            0,
            &mut ancestors,
            &mut diagnostics,
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

    // Walk instanceIndex to find all tier-2 records, filter by type if requested.
    let manifest = store.load_manifest()?;
    let mut root_ids = Vec::new();
    for entry in &manifest.instance_index {
        if entry.tier != 2 {
            continue;
        }
        if target_ids.contains(entry.instance_id.as_str()) {
            continue;
        }
        if let Some(filter) = &options.type_filter {
            if let Some(record) = get_record_by_id(store, &entry.instance_id)? {
                let qualified = format!("{}/{}", record.type_namespace, record.type_name);
                if &qualified != filter {
                    continue;
                }
            } else {
                continue;
            }
        }
        root_ids.push(entry.instance_id.clone());
    }

    Ok(root_ids)
}

#[allow(clippy::too_many_arguments)]
fn build_node(
    store: &dyn RepositoryStore,
    instance_id: &str,
    relations: &[srs_core::types::relation::Relation],
    field_name_index: &HashMap<String, String>,
    options: &TreeOptions,
    depth: u32,
    ancestors: &mut HashSet<String>,
    diagnostics: &mut Vec<String>,
) -> Result<Option<TreeNode>, RepositoryError> {
    let record = match get_record_by_id(store, instance_id)? {
        Some(r) => r,
        None => {
            diagnostics.push(format!(
                "tree: instance {instance_id} not found as a Tier 2 record — skipped"
            ));
            return Ok(None);
        }
    };

    // Apply type filter when visiting non-root nodes.
    if let Some(filter) = &options.type_filter {
        let qualified = format!("{}/{}", record.type_namespace, record.type_name);
        if &qualified != filter {
            return Ok(None);
        }
    }

    let label = record_label::record_display_label(&record, field_name_index);

    // Cycle check must precede max_depth: a node at exactly max_depth that is also
    // an ancestor is a back-edge and must be flagged cycle_pruned, not silently truncated.
    let children = if ancestors.contains(instance_id) {
        return Ok(Some(TreeNode {
            instance_id: instance_id.to_string(),
            label,
            type_namespace: record.type_namespace,
            type_name: record.type_name,
            lifecycle_state: record.lifecycle_state,
            depth,
            children: vec![],
            cycle_pruned: true,
        }));
    } else if options.max_depth.is_some_and(|max| depth >= max) {
        vec![]
    } else {
        ancestors.insert(instance_id.to_string());
        let child_records = relation_graph::children_by_relation_type(
            instance_id,
            &options.relation_type,
            relations,
            store,
        )?;
        let mut child_nodes = Vec::new();
        for child in child_records {
            if let Some(node) = build_node(
                store,
                &child.instance_id,
                relations,
                field_name_index,
                options,
                depth + 1,
                ancestors,
                diagnostics,
            )? {
                child_nodes.push(node);
            }
        }
        ancestors.remove(instance_id);
        child_nodes
    };

    Ok(Some(TreeNode {
        instance_id: instance_id.to_string(),
        label,
        type_namespace: record.type_namespace,
        type_name: record.type_name,
        lifecycle_state: record.lifecycle_state,
        depth,
        children,
        cycle_pruned: false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Package;
    use crate::record_store::create_record;
    use crate::relation_service::create_relation_auto;
    use crate::store::memory::MemoryStore;
    use srs_core::types::field::{Field, ValueType};
    use srs_core::types::record::FieldValue;
    use srs_core::types::record_type::{FieldAssignment, RecordType};
    use srs_core::types::relation::Relation;
    use srs_core::types::relation_type_definition::{RelationTypeCategory, RelationTypeDefinition};
    use std::collections::HashMap;

    fn make_field(id: &str, name: &str) -> Field {
        Field {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            value_type: ValueType::String,
            description: String::new(),
            ai_guidance: serde_json::json!(null),
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn make_type(id: &str, name: &str, field_ids: &[&str]) -> RecordType {
        RecordType {
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
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                })
                .collect(),
            field_groups: None,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn make_store(fields: Vec<Field>, types: Vec<RecordType>) -> MemoryStore {
        let manifest = crate::manifest::Manifest {
            instance_index: vec![],
            extra: HashMap::new(),
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
                allowed_source_types: None,
                allowed_target_types: None,
                require_same_semantic_object_type: None,
                status: None,
                updated_at: None,
                properties: None,
            }],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        MemoryStore::new(manifest, package)
    }

    fn add_record(store: &MemoryStore, type_id: &str, field_id: &str, title: &str) -> String {
        let fv = vec![FieldValue {
            field_id: field_id.to_string(),
            value: serde_json::json!(title),
            entries: None,
            source: None,
            edited_at: None,
        }];
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
    fn build_tree_auto_detects_roots_and_children() {
        let store = make_store(
            vec![make_field("f-title", "title")],
            vec![make_type("t-node", "node", &["f-title"])],
        );
        let root_id = add_record(&store, "t-node", "f-title", "Root");
        let child_id = add_record(&store, "t-node", "f-title", "Child");
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
        let root_id = add_record(&store, "t-node", "f-title", "Root");
        let child_id = add_record(&store, "t-node", "f-title", "Child");
        let grandchild_id = add_record(&store, "t-node", "f-title", "Grandchild");
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
        let a_id = add_record(&store, "t-node", "f-title", "A");
        let b_id = add_record(&store, "t-node", "f-title", "B");
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
        let a_id = add_record(&store, "t-node", "f-title", "A");
        let b_id = add_record(&store, "t-node", "f-title", "B");
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

        let sec_id = add_record(&store, "t-section", "f-title", "A Section");
        let _note_id = add_record(&store, "t-note", "f-title", "A Note");

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
}
