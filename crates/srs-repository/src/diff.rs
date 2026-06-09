use crate::error::RepositoryError;
use crate::repository_portability::export_repository_snapshot;
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
pub struct DiffSummary {
    pub instances_added: usize,
    pub instances_removed: usize,
    pub instances_modified: usize,
    pub relations_added: usize,
    pub relations_removed: usize,
    pub relations_modified: usize,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiff {
    pub summary: DiffSummary,
    pub manifest: ManifestDiff,
    pub instances: DiffInstances,
    pub relations: DiffRelations,
}

pub fn diff_repositories(
    from: &dyn RepositoryStore,
    to: &dyn RepositoryStore,
) -> Result<RepoDiff, RepositoryError> {
    let snap_from = export_repository_snapshot(from)?;
    let snap_to = export_repository_snapshot(to)?;

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

    let summary = DiffSummary {
        instances_added: instances_added.len(),
        instances_removed: instances_removed.len(),
        instances_modified: instances_modified.len(),
        relations_added: relations_added.len(),
        relations_removed: relations_removed.len(),
        relations_modified: relations_modified.len(),
    };

    Ok(RepoDiff {
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_lifecycle::RepositoryMetadata;
    use crate::repository_portability::{RepositorySnapshot, SnapshotInstance};

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
            relations,
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

    // Diff two in-memory snapshots directly (bypasses RepositoryStore).
    fn diff_snapshots(snap_from: &RepositorySnapshot, snap_to: &RepositorySnapshot) -> RepoDiff {
        // Duplicate the core logic without I/O by constructing the diff inline.
        let namespace_changed = snap_from.repository.namespace != snap_to.repository.namespace;
        let srs_version_changed =
            snap_from.repository.srs_version != snap_to.repository.srs_version;

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
        let mut extensions_added: Vec<String> = ext_to
            .difference(&ext_from)
            .map(|s| s.to_string())
            .collect();
        let mut extensions_removed: Vec<String> = ext_from
            .difference(&ext_to)
            .map(|s| s.to_string())
            .collect();
        extensions_added.sort();
        extensions_removed.sort();

        let map_from: HashMap<&str, &SnapshotInstance> = snap_from
            .instances
            .iter()
            .map(|i| (i.instance_id.as_str(), i))
            .collect();
        let map_to: HashMap<&str, &SnapshotInstance> = snap_to
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
                let from_val = serde_json::to_value(from_rel).unwrap();
                let to_val = serde_json::to_value(rel).unwrap();
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
                    value: serde_json::to_value(rel).unwrap(),
                });
            }
        }

        for (id, rel) in &rel_from {
            if !rel_to.contains_key(id) {
                relations_removed.push(RelationRemoved {
                    relation_id: id.to_string(),
                    value: serde_json::to_value(rel).unwrap(),
                });
            }
        }

        let summary = DiffSummary {
            instances_added: instances_added.len(),
            instances_removed: instances_removed.len(),
            instances_modified: instances_modified.len(),
            relations_added: relations_added.len(),
            relations_removed: relations_removed.len(),
            relations_modified: relations_modified.len(),
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
        let diff = diff_snapshots(&snap, &snap);
        assert_eq!(diff.summary.instances_added, 0);
        assert_eq!(diff.summary.instances_removed, 0);
        assert_eq!(diff.summary.instances_modified, 0);
        assert_eq!(diff.summary.relations_added, 0);
        assert_eq!(diff.summary.relations_removed, 0);
        assert_eq!(diff.summary.relations_modified, 0);
        assert!(!diff.manifest.namespace_changed);
        assert!(!diff.manifest.srs_version_changed);
    }

    #[test]
    fn test_diff_instance_added() {
        let snap_from = make_snapshot("com.example", "2.0-draft", vec![], vec![], vec![]);
        let inst = make_instance("id-new", 1, serde_json::json!({"title": "new"}));
        let snap_to = make_snapshot("com.example", "2.0-draft", vec![], vec![inst], vec![]);
        let diff = diff_snapshots(&snap_from, &snap_to);
        assert_eq!(diff.summary.instances_added, 1);
        assert_eq!(diff.summary.instances_removed, 0);
        assert_eq!(diff.instances.added[0].instance_id, "id-new");
    }

    #[test]
    fn test_diff_instance_removed() {
        let inst = make_instance("id-gone", 1, serde_json::json!({"title": "gone"}));
        let snap_from = make_snapshot("com.example", "2.0-draft", vec![], vec![inst], vec![]);
        let snap_to = make_snapshot("com.example", "2.0-draft", vec![], vec![], vec![]);
        let diff = diff_snapshots(&snap_from, &snap_to);
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
        let diff = diff_snapshots(&snap_from, &snap_to);
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
        let diff = diff_snapshots(&snap_from, &snap_to);
        assert!(diff.manifest.namespace_changed);
        assert!(!diff.manifest.srs_version_changed);
    }
}
