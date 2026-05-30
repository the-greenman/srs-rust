use crate::container_service::{create_container, get_container, list_containers};
use crate::error::RepositoryError;
use crate::index::InstanceIndexEntry;
use crate::relation_service::load_relations;
use crate::repository_lifecycle::{
    InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use crate::store::RepositoryStore;
use srs_core::types::container::Container;
use srs_core::types::field::Field;
use srs_core::types::record_type::RecordType;
use srs_core::types::relation::Relation;
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::types::view::{DocumentView, View};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInstance {
    pub instance_id: String,
    pub tier: u8,
    pub title: Option<serde_json::Value>,
    pub tags: Option<Vec<String>>,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageSnapshot {
    pub metadata: PrimaryPackageMetadata,
    pub fields: Vec<Field>,
    pub record_types: Vec<RecordType>,
    pub relation_type_definitions: Vec<RelationTypeDefinition>,
    pub views: Vec<View>,
    pub document_views: Vec<DocumentView>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySnapshot {
    pub repository: RepositoryMetadata,
    pub declared_extensions: Vec<String>,
    pub package: PackageSnapshot,
    pub instances: Vec<SnapshotInstance>,
    pub containers: Vec<Container>,
    pub relations: Vec<Relation>,
}

pub fn export_repository_snapshot(
    source: &dyn RepositoryStore,
) -> Result<RepositorySnapshot, RepositoryError> {
    let manifest = source.load_manifest()?;
    let package = source.load_package()?;

    let mut instances = Vec::new();
    for entry in &manifest.instance_index {
        let value = source.load_instance_json(entry.path())?;
        instances.push(SnapshotInstance {
            instance_id: entry.instance_id.clone(),
            tier: entry.tier,
            title: entry.title.clone(),
            tags: entry.tags.clone(),
            value,
        });
    }

    let mut containers = Vec::new();
    for summary in list_containers(source, None, None, None)? {
        containers.push(get_container(source, &summary.container_id)?);
    }

    let declared_extensions = manifest
        .extra
        .get("declaredExtensions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(RepositorySnapshot {
        repository: RepositoryMetadata {
            repository_id: manifest
                .extra
                .get("repositoryId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            namespace: manifest
                .extra
                .get("namespace")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            srs_version: manifest
                .extra
                .get("srsVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2.0-draft")
                .to_string(),
        },
        declared_extensions,
        package: PackageSnapshot {
            metadata: PrimaryPackageMetadata {
                id: package.id,
                namespace: package.namespace,
                name: package.name,
                version: package.version,
            },
            fields: package.fields,
            record_types: package.record_types,
            relation_type_definitions: package.relation_type_definitions,
            views: package.views,
            document_views: package.document_views,
        },
        instances,
        containers,
        relations: load_relations(source)?,
    })
}

pub fn import_repository_snapshot(
    target: &dyn RepositoryStore,
    snapshot: &RepositorySnapshot,
) -> Result<(), RepositoryError> {
    if target.repository_exists()? {
        return Err(RepositoryError::RepositoryNotEmpty {
            path: target.repository_root(),
        });
    }

    target.initialize_repository(&InitializeRepositoryInput {
        repository: snapshot.repository.clone(),
        primary_package: snapshot.package.metadata.clone(),
    })?;

    import_package(target, &snapshot.package)?;

    let mut manifest = target.load_manifest()?;
    if !snapshot.declared_extensions.is_empty() {
        manifest.extra.insert(
            "declaredExtensions".to_string(),
            serde_json::Value::Array(
                snapshot
                    .declared_extensions
                    .iter()
                    .map(|e| serde_json::Value::String(e.clone()))
                    .collect(),
            ),
        );
    }

    manifest.instance_index = Vec::new();
    for instance in &snapshot.instances {
        let rel_path = canonical_instance_path(instance.tier, &instance.instance_id);
        ensure_instance_parent(target, &rel_path)?;
        target.save_instance_json(&rel_path, &instance.value)?;
        manifest.instance_index.push(InstanceIndexEntry {
            instance_id: instance.instance_id.clone(),
            tier: instance.tier,
            path: rel_path,
            title: instance.title.clone(),
            tags: instance.tags.clone(),
        });
    }
    target.save_manifest(&manifest)?;

    for container in &snapshot.containers {
        create_container(target, container.clone())?;
    }

    if !snapshot.relations.is_empty() {
        target.ensure_relations_dir("relations")?;
        let value = serde_json::to_value(serde_json::json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
            "relations": snapshot.relations
        }))
        .map_err(|source| RepositoryError::Serialize {
            path: std::path::PathBuf::from("relations/relations-collection.json"),
            source,
        })?;
        target.save_relations_json("relations/relations-collection.json", &value)?;
    }

    Ok(())
}

pub fn copy_repository(
    source: &dyn RepositoryStore,
    target: &dyn RepositoryStore,
) -> Result<(), RepositoryError> {
    let snapshot = export_repository_snapshot(source)?;
    import_repository_snapshot(target, &snapshot)
}

fn import_package(
    target: &dyn RepositoryStore,
    package: &PackageSnapshot,
) -> Result<(), RepositoryError> {
    let mut package_json = target.load_package_json()?;

    target.ensure_fields_dir()?;
    let mut fields = Vec::new();
    for field in &package.fields {
        let path = format!("fields/{}-{}.json", slugify(&field.name), &field.id[..8]);
        target.save_field(&path, field)?;
        fields.push(serde_json::Value::String(path));
    }
    package_json["fields"] = serde_json::Value::Array(fields);

    target.ensure_types_dir()?;
    let mut types = Vec::new();
    for record_type in &package.record_types {
        let path = format!(
            "types/{}-{}.json",
            slugify(&record_type.name),
            &record_type.id[..8]
        );
        target.save_type(&path, record_type)?;
        types.push(serde_json::Value::String(path));
    }
    package_json["types"] = serde_json::Value::Array(types);

    target.ensure_relation_types_dir()?;
    let mut relation_types = Vec::new();
    for relation_type in &package.relation_type_definitions {
        let path = format!(
            "relation-types/{}-{}.json",
            slugify(&relation_type.relation_type),
            &relation_type.id[..8]
        );
        target.save_relation_type_definition(&path, relation_type)?;
        relation_types.push(serde_json::Value::String(path));
    }
    package_json["relationTypes"] = serde_json::Value::Array(relation_types);

    target.ensure_views_dir()?;
    let mut views = Vec::new();
    for view in &package.views {
        let path = format!("views/{}-{}.json", slugify(&view.name), &view.id[..8]);
        target.save_view(&path, view)?;
        views.push(serde_json::Value::String(path));
    }
    package_json["views"] = serde_json::Value::Array(views);

    target.ensure_document_views_dir()?;
    let mut doc_views = Vec::new();
    for view in &package.document_views {
        let path = format!(
            "document-views/{}-{}.json",
            slugify(&view.name),
            &view.id[..8]
        );
        target.save_document_view(&path, view)?;
        doc_views.push(serde_json::Value::String(path));
    }
    package_json["documentViews"] = serde_json::Value::Array(doc_views);

    target.save_package_json(&package_json)?;
    Ok(())
}

fn ensure_instance_parent(
    target: &dyn RepositoryStore,
    rel_path: &str,
) -> Result<(), RepositoryError> {
    let parent = rel_path
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .unwrap_or("records");
    target.ensure_instance_dir(parent)
}

fn canonical_instance_path(tier: u8, instance_id: &str) -> String {
    match tier {
        0 => format!("records/notes/{instance_id}.json"),
        3 => format!("records/tag-definitions/{instance_id}.json"),
        _ => format!("records/tier-{tier}/{instance_id}.json"),
    }
}

fn slugify(name: &str) -> String {
    let slug = name
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != ' ', "")
        .replace(' ', "-");
    if slug.is_empty() {
        "item".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use crate::store::FileStore;
    use tempfile::TempDir;

    fn make_input() -> InitializeRepositoryInput {
        InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "repo-copy".to_string(),
                namespace: "com.semanticops.copy".to_string(),
                srs_version: "2.0-draft".to_string(),
            },
            primary_package: PrimaryPackageMetadata {
                id: "pkg-copy".to_string(),
                namespace: "com.semanticops.copy".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        }
    }

    #[test]
    fn copy_memory_repo_to_filestore_preserves_manifest_and_extensions() {
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut manifest = source.load_manifest().unwrap();
        manifest.extra.insert(
            "declaredExtensions".to_string(),
            serde_json::json!(["ext:repository"]),
        );
        source.save_manifest(&manifest).unwrap();

        let temp = TempDir::new().unwrap();
        let target = FileStore::new(temp.path());
        copy_repository(&source, &target).unwrap();

        let copied = target.load_manifest().unwrap();
        let exts = copied
            .extra
            .get("declaredExtensions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            exts,
            vec![serde_json::Value::String("ext:repository".into())]
        );
    }

    #[test]
    fn copy_repository_rejects_non_empty_target() {
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();

        let target = MemoryStore::uninitialized();
        target.initialize_repository(&make_input()).unwrap();

        let err = copy_repository(&source, &target).unwrap_err();
        assert!(matches!(err, RepositoryError::RepositoryNotEmpty { .. }));
    }

    #[test]
    fn repository_snapshot_contains_no_paths() {
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let snapshot = export_repository_snapshot(&source).unwrap();
        let json = serde_json::to_value(snapshot).unwrap();
        let text = serde_json::to_string(&json).unwrap();
        assert!(!text.contains("\"path\""));
        assert!(!text.contains("package/"));
        assert!(!text.contains("records/"));
    }
}
