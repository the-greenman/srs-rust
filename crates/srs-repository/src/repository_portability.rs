use crate::container_service::{create_container, get_container, list_containers};
use crate::error::RepositoryError;
use crate::index::InstanceIndexEntry;
use crate::relation_service::load_relations;
use crate::repository_lifecycle::{
    InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use crate::store::RepositoryStore;
use crate::writer::slugify_instance_name;
use srs_core::types::blueprint::Blueprint;
use srs_core::types::container::Container;
use srs_core::types::field::Field;
use srs_core::types::record_type::RecordType;
use srs_core::types::relation::Relation;
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::types::theme::Theme;
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
pub struct PackageBoundarySnapshot {
    /// None => primary package at `package/`; Some(path) => sub-package path from manifest packageRefs.
    pub boundary_path: Option<String>,
    pub metadata: PrimaryPackageMetadata,
    pub fields: Vec<Field>,
    pub record_types: Vec<RecordType>,
    pub relation_type_definitions: Vec<RelationTypeDefinition>,
    pub views: Vec<View>,
    pub document_views: Vec<DocumentView>,
    #[serde(default)]
    pub blueprints: Vec<Blueprint>,
    #[serde(default)]
    pub themes: Vec<Theme>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySnapshot {
    pub repository: RepositoryMetadata,
    pub declared_extensions: Vec<String>,
    pub packages: Vec<PackageBoundarySnapshot>,
    pub instances: Vec<SnapshotInstance>,
    pub containers: Vec<Container>,
    pub relations: Vec<Relation>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPackageMetadata {
    id: String,
    namespace: String,
    name: String,
    version: String,
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default)]
    types: Vec<String>,
    #[serde(default)]
    relation_types: Vec<String>,
    #[serde(default)]
    views: Vec<String>,
    #[serde(default)]
    document_views: Vec<String>,
    #[serde(default)]
    blueprints: Vec<String>,
    #[serde(default)]
    themes: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPackageRef {
    mode: String,
    path: String,
}

pub fn export_repository_snapshot(
    source: &dyn RepositoryStore,
) -> Result<RepositorySnapshot, RepositoryError> {
    let manifest = source.load_manifest()?;

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

    let mut package_boundaries: Vec<Option<String>> = vec![None];
    let refs: Vec<RawPackageRef> = manifest
        .extra
        .get("packageRefs")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    package_boundaries.extend(
        refs.into_iter()
            .filter(|r| r.mode == "local")
            .map(|r| Some(r.path)),
    );

    let mut packages = Vec::new();
    for boundary in package_boundaries {
        packages.push(export_package_boundary(source, boundary)?);
    }

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
            title: manifest
                .extra
                .get("title")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            description: manifest
                .extra
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        },
        declared_extensions,
        packages,
        instances,
        containers,
        relations: load_relations(source)?,
    })
}

pub fn import_repository_snapshot(
    target: &dyn RepositoryStore,
    snapshot: &RepositorySnapshot,
) -> Result<(), RepositoryError> {
    ensure_target_empty(target)?;

    let primary = snapshot
        .packages
        .iter()
        .find(|p| p.boundary_path.is_none())
        .ok_or_else(|| RepositoryError::InvalidSnapshotData {
            message: "snapshot missing primary package boundary".to_string(),
        })?;

    target.initialize_repository(&InitializeRepositoryInput {
        repository: snapshot.repository.clone(),
        primary_package: primary.metadata.clone(),
    })?;

    import_package_boundary(target, primary)?;

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

    let mut package_refs = Vec::new();
    for package in snapshot
        .packages
        .iter()
        .filter(|p| p.boundary_path.is_some())
    {
        import_package_boundary(target, package)?;
        if let Some(path) = &package.boundary_path {
            package_refs.push(serde_json::json!({ "mode": "local", "path": path }));
        }
    }
    if !package_refs.is_empty() {
        manifest.extra.insert(
            "packageRefs".to_string(),
            serde_json::Value::Array(package_refs),
        );
    }

    manifest.instance_index = Vec::new();
    for instance in &snapshot.instances {
        let rel_path = canonical_instance_path(instance);
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

fn export_package_boundary(
    source: &dyn RepositoryStore,
    boundary_path: Option<String>,
) -> Result<PackageBoundarySnapshot, RepositoryError> {
    if boundary_path.is_none() {
        let pkg = source.load_package()?;
        return Ok(PackageBoundarySnapshot {
            boundary_path: None,
            metadata: PrimaryPackageMetadata {
                id: pkg.id,
                namespace: pkg.namespace,
                name: pkg.name,
                version: pkg.version,
            },
            fields: pkg.fields,
            record_types: pkg.record_types,
            relation_type_definitions: pkg.relation_type_definitions,
            views: pkg.views,
            document_views: pkg.document_views,
            blueprints: pkg.blueprints,
            themes: pkg.themes,
        });
    }

    let package_prefix = match &boundary_path {
        Some(p) => p.clone(),
        None => "package".to_string(),
    };
    let package_json_path = format!("{package_prefix}/package.json");
    let package_json = source.load_instance_json(&package_json_path)?;
    let metadata: RawPackageMetadata =
        serde_json::from_value(package_json).map_err(|source| RepositoryError::PackageLoad {
            path: std::path::PathBuf::from(&package_json_path),
            source,
        })?;

    let fields = metadata
        .fields
        .iter()
        .map(|p| load_typed_json::<Field>(source, &package_prefix, p))
        .collect::<Result<Vec<_>, _>>()?;
    let record_types = metadata
        .types
        .iter()
        .map(|p| load_typed_json::<RecordType>(source, &package_prefix, p))
        .collect::<Result<Vec<_>, _>>()?;
    let relation_type_definitions = metadata
        .relation_types
        .iter()
        .map(|p| load_typed_json::<RelationTypeDefinition>(source, &package_prefix, p))
        .collect::<Result<Vec<_>, _>>()?;
    let views = metadata
        .views
        .iter()
        .map(|p| load_typed_json::<View>(source, &package_prefix, p))
        .collect::<Result<Vec<_>, _>>()?;
    let document_views = metadata
        .document_views
        .iter()
        .map(|p| load_typed_json::<DocumentView>(source, &package_prefix, p))
        .collect::<Result<Vec<_>, _>>()?;
    let blueprints = metadata
        .blueprints
        .iter()
        .map(|p| load_typed_json::<Blueprint>(source, &package_prefix, p))
        .collect::<Result<Vec<_>, _>>()?;
    let themes = metadata
        .themes
        .iter()
        .map(|p| load_typed_json::<Theme>(source, &package_prefix, p))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(PackageBoundarySnapshot {
        boundary_path,
        metadata: PrimaryPackageMetadata {
            id: metadata.id,
            namespace: metadata.namespace,
            name: metadata.name,
            version: metadata.version,
        },
        fields,
        record_types,
        relation_type_definitions,
        views,
        document_views,
        blueprints,
        themes,
    })
}

fn import_package_boundary(
    target: &dyn RepositoryStore,
    package: &PackageBoundarySnapshot,
) -> Result<(), RepositoryError> {
    let base_prefix = package
        .boundary_path
        .as_ref()
        .map(|p| p.to_string())
        .unwrap_or_else(|| "package".to_string());

    ensure_repo_dir(target, &base_prefix)?;

    let mut field_paths = Vec::new();
    for field in &package.fields {
        let path = format!(
            "fields/{}-{}.json",
            slugify(&field.name),
            id_prefix(&field.id)?
        );
        write_repo_json(target, &base_prefix, &path, field)?;
        field_paths.push(path);
    }

    let mut type_paths = Vec::new();
    for record_type in &package.record_types {
        let path = format!(
            "types/{}-{}.json",
            slugify(&record_type.name),
            id_prefix(&record_type.id)?
        );
        write_repo_json(target, &base_prefix, &path, record_type)?;
        type_paths.push(path);
    }

    let mut relation_type_paths = Vec::new();
    for relation_type in &package.relation_type_definitions {
        let path = format!(
            "relation-types/{}-{}.json",
            slugify(&relation_type.key),
            id_prefix(&relation_type.id)?
        );
        write_repo_json(target, &base_prefix, &path, relation_type)?;
        relation_type_paths.push(path);
    }

    let mut view_paths = Vec::new();
    for view in &package.views {
        let path = format!(
            "views/{}-{}.json",
            slugify(&view.name),
            id_prefix(&view.id)?
        );
        write_repo_json(target, &base_prefix, &path, view)?;
        view_paths.push(path);
    }

    let mut doc_view_paths = Vec::new();
    for view in &package.document_views {
        let path = format!(
            "document-views/{}-{}.json",
            slugify(&view.name),
            id_prefix(&view.id)?
        );
        write_repo_json(target, &base_prefix, &path, view)?;
        doc_view_paths.push(path);
    }

    let mut blueprint_paths = Vec::new();
    for blueprint in &package.blueprints {
        let path = format!(
            "blueprints/{}-{}.json",
            slugify(&blueprint.name),
            id_prefix(&blueprint.id)?
        );
        write_repo_json(target, &base_prefix, &path, blueprint)?;
        blueprint_paths.push(path);
    }

    let mut theme_paths = Vec::new();
    for theme in &package.themes {
        let path = format!(
            "themes/{}-{}.json",
            slugify(&theme.name),
            id_prefix(&theme.id)?
        );
        write_repo_json(target, &base_prefix, &path, theme)?;
        theme_paths.push(path);
    }

    let package_json = serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
        "id": package.metadata.id,
        "namespace": package.metadata.namespace,
        "name": package.metadata.name,
        "version": package.metadata.version,
        "title": package.metadata.name,
        "description": "",
        "status": "active",
        "createdAt": "2026-01-01T00:00:00Z",
        "fields": field_paths,
        "types": type_paths,
        "relationTypes": relation_type_paths,
        "views": view_paths,
        "documentViews": doc_view_paths,
        "blueprints": blueprint_paths,
        "themes": theme_paths
    });
    target.save_instance_json(&format!("{base_prefix}/package.json"), &package_json)?;
    Ok(())
}

fn load_typed_json<T: serde::de::DeserializeOwned>(
    source: &dyn RepositoryStore,
    base_prefix: &str,
    rel_path: &str,
) -> Result<T, RepositoryError> {
    let full = format!("{base_prefix}/{rel_path}");
    let value = source.load_instance_json(&full)?;
    serde_json::from_value(value).map_err(|source| RepositoryError::PackageLoad {
        path: std::path::PathBuf::from(full),
        source,
    })
}

fn write_repo_json<T: serde::Serialize>(
    target: &dyn RepositoryStore,
    base_prefix: &str,
    rel_path: &str,
    value: &T,
) -> Result<(), RepositoryError> {
    let full = format!("{base_prefix}/{rel_path}");
    if let Some((dir, _)) = full.rsplit_once('/') {
        ensure_repo_dir(target, dir)?;
    }
    let json = serde_json::to_value(value).map_err(|source| RepositoryError::Serialize {
        path: std::path::PathBuf::from(&full),
        source,
    })?;
    target.save_instance_json(&full, &json)
}

fn ensure_repo_dir(target: &dyn RepositoryStore, rel_dir: &str) -> Result<(), RepositoryError> {
    target.ensure_instance_dir(rel_dir)
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

fn ensure_target_empty(target: &dyn RepositoryStore) -> Result<(), RepositoryError> {
    let files = target.list_files_recursive("");
    if !files.is_empty() {
        return Err(RepositoryError::RepositoryNotEmpty {
            path: target.repository_root(),
        });
    }
    if target.repository_exists()? {
        return Err(RepositoryError::RepositoryNotEmpty {
            path: target.repository_root(),
        });
    }
    Ok(())
}

fn canonical_instance_path(instance: &SnapshotInstance) -> String {
    let id8 = &instance.instance_id[..8];
    let slug = match instance.tier {
        0 => instance
            .title
            .as_ref()
            .and_then(|v| v.as_str())
            .map(slugify_instance_name)
            .unwrap_or_default(),
        1 | 2 => instance
            .value
            .get("typeName")
            .and_then(|v| v.as_str())
            .map(slugify_instance_name)
            .unwrap_or_default(),
        _ => String::new(),
    };
    let filename = if slug.is_empty() {
        format!("{id8}.json")
    } else {
        format!("{slug}-{id8}.json")
    };
    match instance.tier {
        0 => format!("records/notes/{filename}"),
        1 => format!("records/tier-1/{filename}"),
        2 => format!("records/tier-2/{filename}"),
        tier => format!("records/tier-{tier}/{filename}"),
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

fn id_prefix(id: &str) -> Result<&str, RepositoryError> {
    if id.len() < 8 {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!("identifier '{id}' must be at least 8 characters"),
        });
    }
    Ok(&id[..8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_store::JsonStore;
    use crate::store::memory::MemoryStore;
    use crate::store::{FileStore, RepositoryStore};
    use crate::validation::validate_repository;
    use tempfile::TempDir;

    fn make_input() -> InitializeRepositoryInput {
        InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "repo-copy".to_string(),
                namespace: "com.semanticops.copy".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
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

        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("already-there.txt"), "x").unwrap();
        let target = FileStore::new(temp.path());

        let err = copy_repository(&source, &target).unwrap_err();
        assert!(matches!(err, RepositoryError::RepositoryNotEmpty { .. }));
    }

    #[test]
    // The snapshot DTO must not serialize the file-backed `path` field from
    // `InstanceIndexEntry` — paths are a FileStore adapter concern, not part
    // of the logical snapshot. This guards against accidental `#[serde(flatten)]`
    // or field leakage that would couple the snapshot format to storage layout.
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

    #[test]
    fn import_repository_snapshot_rejects_short_identifiers() {
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut snapshot = export_repository_snapshot(&source).unwrap();
        snapshot.packages[0].fields.push(Field {
            id: "short".to_string(),
            namespace: "com.semanticops.copy".to_string(),
            name: "bad".to_string(),
            version: 1,
            value_type: srs_core::types::field::ValueType::String,
            description: "".to_string(),
            ai_guidance: serde_json::Value::Null,
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "".to_string(),
            extra: std::collections::HashMap::new(),
        });

        let target = MemoryStore::uninitialized();
        let result = import_repository_snapshot(&target, &snapshot);
        assert!(matches!(
            result,
            Err(RepositoryError::InvalidSnapshotData { .. })
        ));
    }

    #[test]
    fn copy_round_trips_package_blueprints() {
        use crate::blueprint_service::{get_blueprint_by_id, GetBlueprintResult};
        use srs_core::types::blueprint::{Blueprint, TypeRef};

        // Source repo with a blueprint in its primary package.
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut snapshot = export_repository_snapshot(&source).unwrap();
        snapshot.packages[0].blueprints.push(Blueprint {
            id: "7bfa600b-f7b2-4a0e-82d4-34c02d9d6770".to_string(),
            namespace: "com.semanticops.copy".to_string(),
            name: "guide".to_string(),
            version: 1,
            description: "Guide blueprint".to_string(),
            root_types: vec![TypeRef {
                type_id: "8f138dd6-11d2-42a5-99ec-3d6e23bed54f".to_string(),
                type_version: None,
            }],
            structure: vec![],
            required_types: vec![],
            ai_guidance: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        });

        // Import into a JSON store (the .srsj bundle backend) and confirm the
        // blueprint survives: get_blueprint_by_id is exactly the path the
        // blueprint-schema service (and the web guides editor) consult.
        let tmp = TempDir::new().unwrap();
        let target = JsonStore::create(tmp.path().join("repo.srsj")).unwrap();
        import_repository_snapshot(&target, &snapshot).unwrap();

        // package.json must index the blueprint.
        let pkg_json = target.load_instance_json("package/package.json").unwrap();
        let blueprints = pkg_json
            .get("blueprints")
            .and_then(|v| v.as_array())
            .expect("package.json must carry a blueprints array");
        assert_eq!(
            blueprints.len(),
            1,
            "one blueprint expected in package.json"
        );

        // And the blueprint must resolve by id through the real consumer path.
        match get_blueprint_by_id(&target, "7bfa600b-f7b2-4a0e-82d4-34c02d9d6770").unwrap() {
            GetBlueprintResult::Found(bp) => {
                assert_eq!(bp.name, "guide");
                assert_eq!(bp.root_types.len(), 1);
            }
            GetBlueprintResult::NotFound => panic!("blueprint lost during copy"),
        }
    }

    #[test]
    fn copy_memory_repo_to_filestore_preserves_packages() {
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut snapshot = export_repository_snapshot(&source).unwrap();
        snapshot.packages.push(PackageBoundarySnapshot {
            boundary_path: Some("package/subpkg".to_string()),
            metadata: PrimaryPackageMetadata {
                id: "pkg-sub".to_string(),
                namespace: "com.semanticops.copy".to_string(),
                name: "subpkg".to_string(),
                version: "1.0.0".to_string(),
            },
            fields: vec![],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            blueprints: vec![],
            themes: vec![],
        });

        let temp = TempDir::new().unwrap();
        let target = FileStore::new(temp.path());
        import_repository_snapshot(&target, &snapshot).unwrap();

        let manifest = target.load_manifest().unwrap();
        let refs = manifest
            .extra
            .get("packageRefs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0]["path"], "package/subpkg");
    }

    #[test]
    fn copy_memory_repo_to_filestore_preserves_records_and_containers() {
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut snapshot = export_repository_snapshot(&source).unwrap();
        snapshot.instances.push(SnapshotInstance {
            instance_id: "11111111-1111-4111-8111-111111111111".to_string(),
            tier: 0,
            title: Some(serde_json::Value::String("n".to_string())),
            tags: None,
            value: serde_json::json!({
                "instanceId": "11111111-1111-4111-8111-111111111111",
                "sections": [{"name":"body","content":"hello"}]
            }),
        });
        snapshot.containers.push(Container {
            container_id: "22222222-2222-4222-8222-222222222222".to_string(),
            title: "C".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            member_instance_ids: Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]),
            root_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::HashMap::new(),
        });
        snapshot.relations.push(Relation {
            relation_id: "33333333-3333-4333-8333-333333333333".to_string(),
            relation_type: "contains".to_string(),
            source_instance_id: "22222222-2222-4222-8222-222222222222".to_string(),
            target_instance_id: "11111111-1111-4111-8111-111111111111".to_string(),
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
        });

        let temp = TempDir::new().unwrap();
        let target = FileStore::new(temp.path());
        import_repository_snapshot(&target, &snapshot).unwrap();

        let copied = target.load_manifest().unwrap();
        assert_eq!(copied.instance_index.len(), 1);
        let summaries = list_containers(&target, None, None, None).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(load_relations(&target).unwrap().len(), 1);
    }

    #[test]
    fn copied_repository_validates() {
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut snapshot = export_repository_snapshot(&source).unwrap();
        snapshot.instances.push(SnapshotInstance {
            instance_id: "44444444-4444-4444-8444-444444444444".to_string(),
            tier: 0,
            title: None,
            tags: None,
            value: serde_json::json!({
                "instanceId": "44444444-4444-4444-8444-444444444444",
                "sections": [{"name":"body","content":"ok"}]
            }),
        });

        let temp = TempDir::new().unwrap();
        let target = FileStore::new(temp.path());
        import_repository_snapshot(&target, &snapshot).unwrap();

        let report = validate_repository(&target).unwrap();
        assert!(report.is_ok(), "{:?}", report.diagnostics);
    }

    #[test]
    fn memory_to_json_to_file_roundtrip_validates() {
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut snapshot = export_repository_snapshot(&source).unwrap();
        snapshot.instances.push(SnapshotInstance {
            instance_id: "55555555-5555-4555-8555-555555555555".to_string(),
            tier: 0,
            title: None,
            tags: None,
            value: serde_json::json!({
                "instanceId": "55555555-5555-4555-8555-555555555555",
                "sections": [{"name":"body","content":"json hop"}]
            }),
        });

        let tmp = TempDir::new().unwrap();
        let json_path = tmp.path().join("repo.srsj");
        let json_store = JsonStore::create(&json_path).unwrap();
        import_repository_snapshot(&json_store, &snapshot).unwrap();

        let out = TempDir::new().unwrap();
        let file_store = FileStore::new(out.path());
        copy_repository(&json_store, &file_store).unwrap();

        let report = validate_repository(&file_store).unwrap();
        assert!(report.is_ok(), "{:?}", report.diagnostics);
    }

    #[test]
    fn copy_file_to_file_produces_slug_id_filename() {
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut snapshot = export_repository_snapshot(&source).unwrap();
        snapshot.instances.push(SnapshotInstance {
            instance_id: "11111111-1111-4111-8111-111111111111".to_string(),
            tier: 0,
            title: Some(serde_json::Value::String("My Note".to_string())),
            tags: None,
            value: serde_json::json!({
                "instanceId": "11111111-1111-4111-8111-111111111111",
                "sections": [{"name":"body","content":"hello"}]
            }),
        });

        let temp = TempDir::new().unwrap();
        let target = FileStore::new(temp.path());
        import_repository_snapshot(&target, &snapshot).unwrap();

        assert!(
            temp.path()
                .join("records/notes/my-note-11111111.json")
                .exists(),
            "expected records/notes/my-note-11111111.json"
        );
    }

    #[test]
    fn copy_file_to_file_no_title_produces_id_only_filename() {
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut snapshot = export_repository_snapshot(&source).unwrap();
        snapshot.instances.push(SnapshotInstance {
            instance_id: "22222222-2222-4222-8222-222222222222".to_string(),
            tier: 0,
            title: None,
            tags: None,
            value: serde_json::json!({
                "instanceId": "22222222-2222-4222-8222-222222222222",
                "sections": [{"name":"body","content":"no title"}]
            }),
        });

        let temp = TempDir::new().unwrap();
        let target = FileStore::new(temp.path());
        import_repository_snapshot(&target, &snapshot).unwrap();

        assert!(
            temp.path().join("records/notes/22222222.json").exists(),
            "expected records/notes/22222222.json (id-only, no title)"
        );
    }

    #[test]
    fn file_json_file_roundtrip_produces_slug_id_filename() {
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut snapshot = export_repository_snapshot(&source).unwrap();
        snapshot.instances.push(SnapshotInstance {
            instance_id: "33333333-3333-4333-8333-333333333333".to_string(),
            tier: 0,
            title: Some(serde_json::Value::String("Round Trip".to_string())),
            tags: None,
            value: serde_json::json!({
                "instanceId": "33333333-3333-4333-8333-333333333333",
                "sections": [{"name":"body","content":"round trip"}]
            }),
        });

        let tmp = TempDir::new().unwrap();
        let json_path = tmp.path().join("repo.srsj");
        let json_store = JsonStore::create(&json_path).unwrap();
        import_repository_snapshot(&json_store, &snapshot).unwrap();

        let out = TempDir::new().unwrap();
        let file_store = FileStore::new(out.path());
        copy_repository(&json_store, &file_store).unwrap();

        assert!(
            out.path()
                .join("records/notes/round-trip-33333333.json")
                .exists(),
            "expected records/notes/round-trip-33333333.json after file→json→file round-trip"
        );
    }

    #[test]
    fn copy_tier2_record_uses_type_slug_id_filename() {
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut snapshot = export_repository_snapshot(&source).unwrap();
        snapshot.instances.push(SnapshotInstance {
            instance_id: "44444444-4444-4444-a444-444444444444".to_string(),
            tier: 2,
            title: None,
            tags: None,
            value: serde_json::json!({
                "instanceId": "44444444-4444-4444-a444-444444444444",
                "typeId": "some-type-id",
                "typeName": "section",
                "typeNamespace": "com.example",
                "typeVersion": 1,
                "fieldValues": []
            }),
        });

        let temp = TempDir::new().unwrap();
        let target = FileStore::new(temp.path());
        import_repository_snapshot(&target, &snapshot).unwrap();

        assert!(
            temp.path()
                .join("records/tier-2/section-44444444.json")
                .exists(),
            "expected records/tier-2/section-44444444.json"
        );
    }
}
