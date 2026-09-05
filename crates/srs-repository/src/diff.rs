use crate::error::RepositoryError;
use crate::repository_portability::{
    export_repository_snapshot, PackageBoundarySnapshot, RepositorySnapshot,
};
use crate::store::RepositoryStore;
use std::collections::HashMap;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestDiff {
    pub namespace_changed: bool,
    pub srs_version_changed: bool,
    pub extensions_added: Vec<String>,
    pub extensions_removed: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceAdded {
    pub instance_id: String,
    pub tier: u8,
    pub value: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceRemoved {
    pub instance_id: String,
    pub tier: u8,
    pub value: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceModified {
    pub instance_id: String,
    pub tier: u8,
    pub from_value: serde_json::Value,
    pub to_value: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffInstances {
    pub added: Vec<InstanceAdded>,
    pub removed: Vec<InstanceRemoved>,
    pub modified: Vec<InstanceModified>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationAdded {
    pub relation_id: String,
    pub value: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationRemoved {
    pub relation_id: String,
    pub value: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationModified {
    pub relation_id: String,
    pub from_value: serde_json::Value,
    pub to_value: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffRelations {
    pub added: Vec<RelationAdded>,
    pub removed: Vec<RelationRemoved>,
    pub modified: Vec<RelationModified>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageItemAdded {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub value: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageItemRemoved {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub value: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageItemModified {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub from_value: serde_json::Value,
    pub to_value: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffPackageCategory {
    pub added: Vec<PackageItemAdded>,
    pub removed: Vec<PackageItemRemoved>,
    pub modified: Vec<PackageItemModified>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffPackage {
    pub fields: DiffPackageCategory,
    pub record_types: DiffPackageCategory,
    pub blueprints: DiffPackageCategory,
    pub compositions: DiffPackageCategory,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffSummary {
    pub instances_added: usize,
    pub instances_removed: usize,
    pub instances_modified: usize,
    pub relations_added: usize,
    pub relations_removed: usize,
    pub relations_modified: usize,
    pub fields_added: usize,
    pub fields_removed: usize,
    pub fields_modified: usize,
    pub record_types_added: usize,
    pub record_types_removed: usize,
    pub record_types_modified: usize,
    pub blueprints_added: usize,
    pub blueprints_removed: usize,
    pub blueprints_modified: usize,
    pub compositions_added: usize,
    pub compositions_removed: usize,
    pub compositions_modified: usize,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiff {
    pub summary: DiffSummary,
    pub manifest: ManifestDiff,
    pub instances: DiffInstances,
    pub relations: DiffRelations,
    pub package: DiffPackage,
}

pub fn diff_repositories(
    from: &dyn RepositoryStore,
    to: &dyn RepositoryStore,
) -> Result<RepoDiff, RepositoryError> {
    let snap_from = export_repository_snapshot(from)?;
    let snap_to = export_repository_snapshot(to)?;
    Ok(compute_diff(&snap_from, &snap_to))
}

fn compute_diff(snap_from: &RepositorySnapshot, snap_to: &RepositorySnapshot) -> RepoDiff {
    // Manifest diff
    let namespace_changed = snap_from.repository.namespace != snap_to.repository.namespace;
    let srs_version_changed = snap_from.repository.srs_version != snap_to.repository.srs_version;
    let ext_from: std::collections::HashSet<&str> = snap_from
        .declared_extensions
        .iter()
        .map(|s| s.as_str())
        .collect();
    let ext_to: std::collections::HashSet<&str> = snap_to
        .declared_extensions
        .iter()
        .map(|s| s.as_str())
        .collect();
    let extensions_added: Vec<String> = ext_to
        .difference(&ext_from)
        .map(|s| s.to_string())
        .collect();
    let extensions_removed: Vec<String> = ext_from
        .difference(&ext_to)
        .map(|s| s.to_string())
        .collect();

    // Instance diff — keyed by instance_id
    let map_from: HashMap<&str, &crate::repository_portability::SnapshotInstance> = snap_from
        .instances
        .iter()
        .map(|i| (i.instance_id.as_str(), i))
        .collect();
    let map_to: HashMap<&str, &crate::repository_portability::SnapshotInstance> = snap_to
        .instances
        .iter()
        .map(|i| (i.instance_id.as_str(), i))
        .collect();

    let mut instances_added = Vec::new();
    let mut instances_removed = Vec::new();
    let mut instances_modified = Vec::new();

    for (id, inst_to) in &map_to {
        if let Some(inst_from) = map_from.get(id) {
            if inst_from.value != inst_to.value {
                instances_modified.push(InstanceModified {
                    instance_id: id.to_string(),
                    tier: inst_to.tier,
                    from_value: inst_from.value.clone(),
                    to_value: inst_to.value.clone(),
                });
            }
        } else {
            instances_added.push(InstanceAdded {
                instance_id: id.to_string(),
                tier: inst_to.tier,
                value: inst_to.value.clone(),
            });
        }
    }

    for (id, inst_from) in &map_from {
        if !map_to.contains_key(id) {
            instances_removed.push(InstanceRemoved {
                instance_id: id.to_string(),
                tier: inst_from.tier,
                value: inst_from.value.clone(),
            });
        }
    }

    // Relation diff — keyed by relation_id
    let rel_from: HashMap<&str, &srs_core::types::relation::Relation> = snap_from
        .relations
        .iter()
        .map(|r| (r.relation_id.as_str(), r))
        .collect();
    let rel_to: HashMap<&str, &srs_core::types::relation::Relation> = snap_to
        .relations
        .iter()
        .map(|r| (r.relation_id.as_str(), r))
        .collect();

    let mut relations_added = Vec::new();
    let mut relations_removed = Vec::new();
    let mut relations_modified = Vec::new();

    for (id, rel) in &rel_to {
        if let Some(from_rel) = rel_from.get(id) {
            let from_val = serde_json::to_value(from_rel).unwrap_or(serde_json::Value::Null);
            let to_val = serde_json::to_value(rel).unwrap_or(serde_json::Value::Null);
            if from_val != to_val {
                relations_modified.push(RelationModified {
                    relation_id: id.to_string(),
                    from_value: from_val,
                    to_value: to_val,
                });
            }
        } else {
            relations_added.push(RelationAdded {
                relation_id: id.to_string(),
                value: serde_json::to_value(rel).unwrap_or(serde_json::Value::Null),
            });
        }
    }

    for (id, rel) in &rel_from {
        if !rel_to.contains_key(id) {
            relations_removed.push(RelationRemoved {
                relation_id: id.to_string(),
                value: serde_json::to_value(rel).unwrap_or(serde_json::Value::Null),
            });
        }
    }

    // Package diff — flatten all packages from each side by item ID
    let package = diff_packages(&snap_from.packages, &snap_to.packages);

    let summary = DiffSummary {
        instances_added: instances_added.len(),
        instances_removed: instances_removed.len(),
        instances_modified: instances_modified.len(),
        relations_added: relations_added.len(),
        relations_removed: relations_removed.len(),
        relations_modified: relations_modified.len(),
        fields_added: package.fields.added.len(),
        fields_removed: package.fields.removed.len(),
        fields_modified: package.fields.modified.len(),
        record_types_added: package.record_types.added.len(),
        record_types_removed: package.record_types.removed.len(),
        record_types_modified: package.record_types.modified.len(),
        blueprints_added: package.blueprints.added.len(),
        blueprints_removed: package.blueprints.removed.len(),
        blueprints_modified: package.blueprints.modified.len(),
        compositions_added: package.compositions.added.len(),
        compositions_removed: package.compositions.removed.len(),
        compositions_modified: package.compositions.modified.len(),
    };

    RepoDiff {
        summary,
        manifest: ManifestDiff {
            namespace_changed,
            srs_version_changed,
            extensions_added,
            extensions_removed,
        },
        instances: DiffInstances {
            added: instances_added,
            removed: instances_removed,
            modified: instances_modified,
        },
        relations: DiffRelations {
            added: relations_added,
            removed: relations_removed,
            modified: relations_modified,
        },
        package,
    }
}

/// Flatten all packages from each snapshot by item ID and diff each category.
/// IDs are globally unique (UUID4) so flattening across package boundaries is safe.
fn diff_packages(
    from_pkgs: &[PackageBoundarySnapshot],
    to_pkgs: &[PackageBoundarySnapshot],
) -> DiffPackage {
    let mut fields_from: HashMap<&str, &srs_core::types::field::Field> = HashMap::new();
    let mut fields_to: HashMap<&str, &srs_core::types::field::Field> = HashMap::new();
    let mut types_from: HashMap<&str, &srs_core::types::record_type::RecordType> = HashMap::new();
    let mut types_to: HashMap<&str, &srs_core::types::record_type::RecordType> = HashMap::new();
    let mut blueprints_from: HashMap<&str, &srs_core::types::blueprint::Blueprint> = HashMap::new();
    let mut blueprints_to: HashMap<&str, &srs_core::types::blueprint::Blueprint> = HashMap::new();
    let mut doc_views_from: HashMap<&str, &srs_core::types::view::Composition> = HashMap::new();
    let mut doc_views_to: HashMap<&str, &srs_core::types::view::Composition> = HashMap::new();

    for pkg in from_pkgs {
        for f in &pkg.fields {
            fields_from.insert(f.id.as_str(), f);
        }
        for t in &pkg.record_types {
            types_from.insert(t.id.as_str(), t);
        }
        for b in &pkg.blueprints {
            blueprints_from.insert(b.id.as_str(), b);
        }
        for dv in &pkg.compositions {
            doc_views_from.insert(dv.id.as_str(), dv);
        }
    }

    for pkg in to_pkgs {
        for f in &pkg.fields {
            fields_to.insert(f.id.as_str(), f);
        }
        for t in &pkg.record_types {
            types_to.insert(t.id.as_str(), t);
        }
        for b in &pkg.blueprints {
            blueprints_to.insert(b.id.as_str(), b);
        }
        for dv in &pkg.compositions {
            doc_views_to.insert(dv.id.as_str(), dv);
        }
    }

    DiffPackage {
        fields: diff_package_items(&fields_from, &fields_to, |f| {
            (f.namespace.as_str(), f.name.as_str(), f.version)
        }),
        record_types: diff_package_items(&types_from, &types_to, |t| {
            (t.namespace.as_str(), t.name.as_str(), t.version)
        }),
        blueprints: diff_package_items(&blueprints_from, &blueprints_to, |b| {
            (b.namespace.as_str(), b.name.as_str(), b.version)
        }),
        compositions: diff_package_items(&doc_views_from, &doc_views_to, |dv| {
            (dv.namespace.as_str(), dv.name.as_str(), dv.version)
        }),
    }
}

fn diff_package_items<T, F>(
    from_map: &HashMap<&str, &T>,
    to_map: &HashMap<&str, &T>,
    meta: F,
) -> DiffPackageCategory
where
    T: serde::Serialize,
    F: Fn(&T) -> (&str, &str, u32),
{
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();

    for (id, item_to) in to_map {
        let to_val = serde_json::to_value(item_to).unwrap_or(serde_json::Value::Null);
        if let Some(item_from) = from_map.get(id) {
            let from_val = serde_json::to_value(item_from).unwrap_or(serde_json::Value::Null);
            if from_val != to_val {
                let (namespace, name, _) = meta(item_to);
                modified.push(PackageItemModified {
                    id: id.to_string(),
                    namespace: namespace.to_string(),
                    name: name.to_string(),
                    from_value: from_val,
                    to_value: to_val,
                });
            }
        } else {
            let (namespace, name, version) = meta(item_to);
            added.push(PackageItemAdded {
                id: id.to_string(),
                namespace: namespace.to_string(),
                name: name.to_string(),
                version,
                value: to_val,
            });
        }
    }

    for (id, item_from) in from_map {
        if !to_map.contains_key(id) {
            let from_val = serde_json::to_value(item_from).unwrap_or(serde_json::Value::Null);
            let (namespace, name, version) = meta(item_from);
            removed.push(PackageItemRemoved {
                id: id.to_string(),
                namespace: namespace.to_string(),
                name: name.to_string(),
                version,
                value: from_val,
            });
        }
    }

    DiffPackageCategory {
        added,
        removed,
        modified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_lifecycle::{PrimaryPackageMetadata, RepositoryMetadata};
    use crate::repository_portability::{
        PackageBoundarySnapshot, RepositorySnapshot, SnapshotInstance,
    };

    fn make_snapshot(
        namespace: &str,
        srs_version: &str,
        extensions: Vec<&str>,
        instances: Vec<SnapshotInstance>,
        relations: Vec<srs_core::types::relation::Relation>,
    ) -> RepositorySnapshot {
        RepositorySnapshot {
            repository: RepositoryMetadata {
                repository_id: "test-repo-id".to_string(),
                namespace: namespace.to_string(),
                srs_version: srs_version.to_string(),
                title: None,
                description: None,
            },
            declared_extensions: extensions.into_iter().map(|s| s.to_string()).collect(),
            packages: vec![],
            instances,
            containers: vec![],
            root_container: None,
            container_index: None,
            relations,
            source_documents_path: None,
            source_documents: vec![],
            upstream_package: None,
            meta: None,
            data_model_revision: None,
        }
    }

    fn make_instance(id: &str, tier: u8, value: serde_json::Value) -> SnapshotInstance {
        SnapshotInstance {
            instance_id: id.to_string(),
            tier,
            title: None,
            tags: None,
            value,
        }
    }

    fn make_field(id: &str, name: &str, version: u32) -> srs_core::types::field::Field {
        srs_core::types::field::Field {
            schema: None,
            id: id.to_string(),
            namespace: "com.example".to_string(),
            name: name.to_string(),
            version,
            description: String::new(),
            instructions: None,
            ai_guidance: Some(srs_core::types::field::AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            field_type: srs_core::types::field::FieldType::string(),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    fn make_blueprint(id: &str, name: &str, version: u32) -> srs_core::types::blueprint::Blueprint {
        srs_core::types::blueprint::Blueprint {
            schema: None,
            id: id.to_string(),
            namespace: "com.example".to_string(),
            name: name.to_string(),
            version,
            description: String::new(),
            root_types: vec![],
            structure: vec![],
            required_types: vec![],
            ai_guidance: None,
            tags: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        }
    }

    fn empty_pkg_meta() -> PrimaryPackageMetadata {
        PrimaryPackageMetadata {
            id: "pkg-id".to_string(),
            namespace: "com.example".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
        }
    }

    fn snap_with_packages(packages: Vec<PackageBoundarySnapshot>) -> RepositorySnapshot {
        RepositorySnapshot {
            repository: RepositoryMetadata {
                repository_id: "test-repo-id".to_string(),
                namespace: "com.example".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
            },
            declared_extensions: vec![],
            packages,
            instances: vec![],
            containers: vec![],
            root_container: None,
            container_index: None,
            relations: vec![],
            source_documents_path: None,
            source_documents: vec![],
            upstream_package: None,
            meta: None,
            data_model_revision: None,
        }
    }

    fn pkg(
        fields: Vec<srs_core::types::field::Field>,
        blueprints: Vec<srs_core::types::blueprint::Blueprint>,
    ) -> PackageBoundarySnapshot {
        PackageBoundarySnapshot {
            boundary_path: None,
            metadata: empty_pkg_meta(),
            fields,
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            compositions: vec![],
            blueprints,
            themes: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
            protocols: vec![],
            raw_package_json: None,
        }
    }

    #[test]
    fn test_diff_identical_repos() {
        let inst = make_instance("id-1", 2, serde_json::json!({"title": "hello"}));
        let snap = make_snapshot(
            "com.example",
            "2.0-draft",
            vec!["ext:lifecycle"],
            vec![inst],
            vec![],
        );
        let diff = compute_diff(&snap, &snap);
        assert_eq!(diff.summary.instances_added, 0);
        assert_eq!(diff.summary.instances_removed, 0);
        assert_eq!(diff.summary.instances_modified, 0);
        assert_eq!(diff.summary.relations_added, 0);
        assert_eq!(diff.summary.relations_removed, 0);
        assert_eq!(diff.summary.relations_modified, 0);
        assert_eq!(diff.summary.fields_added, 0);
        assert_eq!(diff.summary.blueprints_added, 0);
        assert!(!diff.manifest.namespace_changed);
        assert!(!diff.manifest.srs_version_changed);
    }

    #[test]
    fn test_diff_instance_added() {
        let snap_from = make_snapshot("com.example", "2.0-draft", vec![], vec![], vec![]);
        let inst = make_instance("id-new", 1, serde_json::json!({"title": "new"}));
        let snap_to = make_snapshot("com.example", "2.0-draft", vec![], vec![inst], vec![]);
        let diff = compute_diff(&snap_from, &snap_to);
        assert_eq!(diff.summary.instances_added, 1);
        assert_eq!(diff.summary.instances_removed, 0);
        assert_eq!(diff.instances.added[0].instance_id, "id-new");
    }

    #[test]
    fn test_diff_instance_removed() {
        let inst = make_instance("id-gone", 1, serde_json::json!({"title": "gone"}));
        let snap_from = make_snapshot("com.example", "2.0-draft", vec![], vec![inst], vec![]);
        let snap_to = make_snapshot("com.example", "2.0-draft", vec![], vec![], vec![]);
        let diff = compute_diff(&snap_from, &snap_to);
        assert_eq!(diff.summary.instances_removed, 1);
        assert_eq!(diff.summary.instances_added, 0);
        assert_eq!(diff.instances.removed[0].instance_id, "id-gone");
    }

    #[test]
    fn test_diff_instance_modified() {
        let inst_from = make_instance("id-1", 2, serde_json::json!({"title": "before"}));
        let inst_to = make_instance("id-1", 2, serde_json::json!({"title": "after"}));
        let snap_from = make_snapshot("com.example", "2.0-draft", vec![], vec![inst_from], vec![]);
        let snap_to = make_snapshot("com.example", "2.0-draft", vec![], vec![inst_to], vec![]);
        let diff = compute_diff(&snap_from, &snap_to);
        assert_eq!(diff.summary.instances_modified, 1);
        assert_eq!(diff.summary.instances_added, 0);
        assert_eq!(diff.summary.instances_removed, 0);
        let m = &diff.instances.modified[0];
        assert_eq!(m.instance_id, "id-1");
        assert_eq!(m.from_value, serde_json::json!({"title": "before"}));
        assert_eq!(m.to_value, serde_json::json!({"title": "after"}));
    }

    #[test]
    fn test_diff_manifest_namespace_changed() {
        let snap_from = make_snapshot("com.example.a", "2.0-draft", vec![], vec![], vec![]);
        let snap_to = make_snapshot("com.example.b", "2.0-draft", vec![], vec![], vec![]);
        let diff = compute_diff(&snap_from, &snap_to);
        assert!(diff.manifest.namespace_changed);
        assert!(!diff.manifest.srs_version_changed);
    }

    #[test]
    fn test_package_diff_field_added() {
        let field = make_field("field-uuid-1", "title", 1);
        let snap_from = snap_with_packages(vec![pkg(vec![], vec![])]);
        let snap_to = snap_with_packages(vec![pkg(vec![field], vec![])]);
        let diff = compute_diff(&snap_from, &snap_to);
        assert_eq!(diff.summary.fields_added, 1);
        assert_eq!(diff.summary.fields_removed, 0);
        assert_eq!(diff.summary.fields_modified, 0);
        assert_eq!(diff.package.fields.added[0].id, "field-uuid-1");
        assert_eq!(diff.package.fields.added[0].name, "title");
    }

    #[test]
    fn test_package_diff_field_removed() {
        let field = make_field("field-uuid-1", "title", 1);
        let snap_from = snap_with_packages(vec![pkg(vec![field], vec![])]);
        let snap_to = snap_with_packages(vec![pkg(vec![], vec![])]);
        let diff = compute_diff(&snap_from, &snap_to);
        assert_eq!(diff.summary.fields_removed, 1);
        assert_eq!(diff.summary.fields_added, 0);
        assert_eq!(diff.package.fields.removed[0].id, "field-uuid-1");
    }

    #[test]
    fn test_package_diff_field_modified() {
        let field_v1 = make_field("field-uuid-1", "title", 1);
        let field_v2 = make_field("field-uuid-1", "title", 2);
        let snap_from = snap_with_packages(vec![pkg(vec![field_v1], vec![])]);
        let snap_to = snap_with_packages(vec![pkg(vec![field_v2], vec![])]);
        let diff = compute_diff(&snap_from, &snap_to);
        assert_eq!(diff.summary.fields_modified, 1);
        assert_eq!(diff.summary.fields_added, 0);
        assert_eq!(diff.summary.fields_removed, 0);
        assert_eq!(diff.package.fields.modified[0].id, "field-uuid-1");
    }

    #[test]
    fn test_package_diff_blueprint_added() {
        let bp = make_blueprint("bp-uuid-1", "my-blueprint", 1);
        let snap_from = snap_with_packages(vec![pkg(vec![], vec![])]);
        let snap_to = snap_with_packages(vec![pkg(vec![], vec![bp])]);
        let diff = compute_diff(&snap_from, &snap_to);
        assert_eq!(diff.summary.blueprints_added, 1);
        assert_eq!(diff.summary.blueprints_removed, 0);
        assert_eq!(diff.package.blueprints.added[0].id, "bp-uuid-1");
        assert_eq!(diff.package.blueprints.added[0].name, "my-blueprint");
    }

    #[test]
    fn test_package_diff_identical_no_changes() {
        let field = make_field("field-uuid-1", "title", 1);
        let bp = make_blueprint("bp-uuid-1", "my-blueprint", 1);
        let snap = snap_with_packages(vec![pkg(vec![field], vec![bp])]);
        let diff = compute_diff(&snap, &snap);
        assert_eq!(diff.summary.fields_added, 0);
        assert_eq!(diff.summary.fields_removed, 0);
        assert_eq!(diff.summary.fields_modified, 0);
        assert_eq!(diff.summary.blueprints_added, 0);
        assert_eq!(diff.summary.blueprints_removed, 0);
        assert_eq!(diff.summary.blueprints_modified, 0);
    }
}
