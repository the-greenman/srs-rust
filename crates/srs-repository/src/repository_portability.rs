use crate::container_service::{
    create_container, get_container, list_containers, ContainerListFilter,
};
use crate::error::RepositoryError;
use crate::index::InstanceIndexEntry;
use crate::relation_service::load_relations;
use crate::repository_lifecycle::{
    InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use crate::revision_service::sidecar_path_for;
use crate::store::{RecordTier, RepositoryStore};
use crate::writer::slugify_instance_name;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use srs_core::extensions::import_tracking::UpstreamPackage;
use srs_core::types::blueprint::Blueprint;
use srs_core::types::container::{Container, ContainerIndexEntry};
use srs_core::types::field::Field;
use srs_core::types::lifecycle::Lifecycle;
use srs_core::types::record_type::RecordType;
use srs_core::types::relation::Relation;
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::types::source_document::SourceDocumentIndexEntry;
use srs_core::types::theme::Theme;
use srs_core::types::view::{DocumentView, View};
use srs_core::types::vocabulary::Vocabulary;
use std::collections::{HashMap, HashSet};

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
    #[serde(default)]
    pub vocabularies: Vec<Vocabulary>,
    #[serde(default)]
    pub lifecycles: Vec<Lifecycle>,
}

/// In-flight snapshot of a single source document: sidecar metadata + optional binary blob.
///
/// Distinct from `srs_core::types::source_document::SourceDocumentIndexEntry`, which is the
/// manifest-persisted index shape (no blob, serialised to disk). `SourceDocumentSnapshot` is
/// ephemeral: it carries the blob across export/import and is never written to disk as-is.
///
/// `content_base64` is `None` when the blob was excluded (text-only export) or the
/// content file was absent in the source (tombstone — RFC-017 R12). Both cases are
/// valid; import always reconstructs the index entry but writes the binary only when
/// `content_base64` is `Some`.
///
/// `sidecar_path` and `content_path` are relative to `sourceDocumentsPath`
/// (e.g. `"my-doc.meta.json"`, `"my-doc.pdf"`), never full repo-relative paths.
/// The guard test `repository_snapshot_contains_no_paths` therefore still passes:
/// no key named `"path"` appears, and no `"records/"` / `"package/"` prefix.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDocumentSnapshot {
    pub document_id: String,
    pub sidecar_path: String,
    pub content_path: String,
    pub sidecar: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar_checksum: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_checksum: Option<String>,
}

/// Options controlling what `export_repository_snapshot_with_options` includes.
#[derive(Debug, Clone, Copy)]
pub struct ExportSnapshotOptions {
    pub include_content_blobs: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySnapshot {
    pub repository: RepositoryMetadata,
    pub declared_extensions: Vec<String>,
    pub packages: Vec<PackageBoundarySnapshot>,
    pub instances: Vec<SnapshotInstance>,
    pub containers: Vec<Container>,
    /// RFC-013 `manifest.container` root-container pointer, if the source declares one.
    /// Distinct from `containers` (the container definitions themselves): this is the
    /// manifest-level identity/navigation-root marker that `repo navigation` resolves.
    #[serde(default)]
    pub root_container: Option<Container>,
    #[serde(default)]
    pub container_index: Option<Vec<ContainerIndexEntry>>,
    pub relations: Vec<Relation>,
    /// `manifest.source_documents_path` — needed to reconstruct on import.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_documents_path: Option<String>,
    /// One entry per `sourceDocumentIndex` item. Empty when the source has no source docs
    /// or on a text-only export with no index.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_documents: Vec<SourceDocumentSnapshot>,
    /// RFC-014 upstream-package provenance. ADR-008's snapshot is *path*-free, but that
    /// never meant provenance-free: dropping `manifest.upstreamPackage` on a `.srsj` load
    /// breaks `scaffold_new_repository`, which requires the seed to carry upstream
    /// provenance (srs-rust#696 — the create-document / walkthrough flows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_package: Option<UpstreamPackage>,
    /// `manifest.meta` (e.g. `sourceOfTruth`) — repository metadata preserved so a load
    /// round-trip keeps it rather than silently dropping it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
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
    #[serde(default)]
    vocabularies: Vec<String>,
    #[serde(default)]
    lifecycles: Vec<String>,
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
    export_repository_snapshot_with_options(
        source,
        ExportSnapshotOptions {
            include_content_blobs: false,
        },
    )
}

/// Export a full snapshot, optionally including binary source-document blobs.
///
/// With `include_content_blobs: false` (the default / `.srsj` path per RFC-017 Change F):
///   sidecars are included; binary content is never read.
/// With `include_content_blobs: true` (`.srs` archive and `copy_repository`):
///   binary content is base64-encoded and attached to each `SourceDocumentSnapshot`.
///   A missing content file is treated as a tombstone (RFC-017 R12): the snapshot
///   entry is still emitted but `content_base64` is `None`.
pub fn export_repository_snapshot_with_options(
    source: &dyn RepositoryStore,
    options: ExportSnapshotOptions,
) -> Result<RepositorySnapshot, RepositoryError> {
    let manifest = source.load_manifest()?;

    let mut instances = Vec::new();
    for entry in &manifest.instance_index {
        let value =
            source
                .load_instance_json(entry.path())
                .map_err(|e| RepositoryError::InstanceLoad {
                    instance_id: entry.instance_id.clone(),
                    path: std::path::PathBuf::from(entry.path()),
                    source: Box::new(e) as Box<dyn std::error::Error + Send + Sync>,
                })?;
        instances.push(SnapshotInstance {
            instance_id: entry.instance_id.clone(),
            tier: entry.tier,
            title: entry.title.clone(),
            tags: entry.tags.clone(),
            value,
        });
    }

    let mut containers = Vec::new();
    for summary in list_containers(source, &ContainerListFilter::default())? {
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
    let refs: Vec<RawPackageRef> = match manifest.extra.get("packageRefs") {
        None => Vec::new(),
        Some(v) => {
            serde_json::from_value(v.clone()).map_err(|e| RepositoryError::InvalidSnapshotData {
                message: format!("malformed packageRefs in manifest: {e}"),
            })?
        }
    };
    package_boundaries.extend(
        refs.into_iter()
            .filter(|r| r.mode == "local")
            .map(|r| Some(r.path)),
    );

    let mut packages = Vec::new();
    for boundary in package_boundaries {
        packages.push(export_package_boundary(source, boundary)?);
    }

    // Collect source documents (RFC-017; ADR-031).
    let source_documents_path = manifest.source_documents_path.clone();
    let src_docs_base = source_documents_path
        .as_deref()
        .unwrap_or("source-documents");
    let index_entries = manifest
        .source_document_index
        .as_deref()
        .unwrap_or_default();

    let mut source_documents = Vec::new();
    for entry in index_entries {
        let document_id = Some(entry.document_id.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RepositoryError::InvalidSnapshotData {
                message: format!(
                    "sourceDocumentIndex entry has empty 'documentId' (sidecarPath: {:?})",
                    entry.sidecar_path
                ),
            })?
            .to_string();
        let sidecar_path = Some(entry.sidecar_path.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RepositoryError::InvalidSnapshotData {
                message: format!(
                    "sourceDocumentIndex entry has empty 'sidecarPath' (documentId: {:?})",
                    entry.document_id
                ),
            })?
            .to_string();
        let content_path = Some(entry.content_path.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RepositoryError::InvalidSnapshotData {
                message: format!(
                    "sourceDocumentIndex entry has empty 'contentPath' (documentId: {:?})",
                    entry.document_id
                ),
            })?
            .to_string();

        let sidecar_full = format!("{src_docs_base}/{sidecar_path}");
        let sidecar_str = match source.load_text_file(&sidecar_full) {
            Ok(s) => s,
            Err(ref e) if e.is_not_found() => continue, // tombstone: skip this entry
            Err(e) => return Err(e),
        };
        let sidecar: serde_json::Value = serde_json::from_str(&sidecar_str).map_err(|e| {
            RepositoryError::InvalidSnapshotData {
                message: format!("malformed sidecar '{}': {e}", sidecar_full),
            }
        })?;

        let content_base64 = if options.include_content_blobs {
            let content_full = format!("{src_docs_base}/{content_path}");
            match source.load_binary_file(&content_full) {
                Ok(bytes) => Some(BASE64.encode(&bytes)),
                Err(ref e) if e.is_not_found() => None, // tombstone: RFC-017 R12
                Err(e) => return Err(e),
            }
        } else {
            None
        };

        source_documents.push(SourceDocumentSnapshot {
            document_id,
            sidecar_path,
            content_path,
            sidecar,
            content_base64,
            title: entry.title.clone(),
            sidecar_checksum: entry.sidecar_checksum.clone(),
            content_checksum: entry.content_checksum.clone(),
        });
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
        root_container: manifest.container.clone(),
        container_index: manifest.container_index.clone(),
        relations: load_relations(source)?,
        source_documents_path: if source_documents.is_empty() {
            None
        } else {
            Some(src_docs_base.to_string())
        },
        source_documents,
        upstream_package: manifest.upstream_package.clone(),
        meta: manifest.extra.get("meta").cloned(),
    })
}

pub fn import_repository_snapshot(
    target: &dyn RepositoryStore,
    snapshot: &RepositorySnapshot,
) -> Result<(), RepositoryError> {
    target.begin_batch();
    let result = do_import(target, snapshot);
    match result {
        Ok(()) => target.commit_batch(),
        Err(e) => {
            target.abort_batch();
            Err(e)
        }
    }
}

fn do_import(
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

    // Widen id8 → full id for any instances that share a short canonical path (srs-rust#696),
    // so a valid repository with prefix-colliding UUIDs still materializes to distinct files.
    let instance_paths = collision_safe_instance_paths(&snapshot.instances, target)?;
    let mut used_paths: HashSet<&str> = HashSet::with_capacity(snapshot.instances.len());
    manifest.instance_index = Vec::new();
    for (instance, rel_path) in snapshot.instances.iter().zip(&instance_paths) {
        // After widening, an identical path can only mean a genuine duplicate instance id.
        if !used_paths.insert(rel_path.as_str()) {
            return Err(RepositoryError::InvalidSnapshotData {
                message: format!(
                    "duplicate instance id '{}' — two instances map to the same path '{rel_path}'",
                    instance.instance_id
                ),
            });
        }
        ensure_instance_parent(target, rel_path)?;
        target.save_instance_json(rel_path, &instance.value)?;
        manifest.instance_index.push(InstanceIndexEntry {
            instance_id: instance.instance_id.clone(),
            tier: instance.tier,
            path: rel_path.clone(),
            title: instance.title.clone(),
            tags: instance.tags.clone(),
        });
    }
    // Only override the placeholder `initialize_repository` assigned when the source
    // actually declared a root container — some in-memory test sources predate RFC-013
    // and carry no `manifest.container` at all, in which case the target's freshly
    // initialized default (which does satisfy the required-container invariant) should
    // stand rather than being clobbered to `None`.
    if let Some(root_container) = &snapshot.root_container {
        manifest.container = Some(root_container.clone());
    }
    if let Some(container_index) = &snapshot.container_index {
        manifest.container_index = Some(container_index.clone());
    }

    // Materialize source documents (RFC-017 R3/R12; ADR-007: files before index;
    // ADR-021: writes happen inside the begin_batch/commit_batch bracket above).
    if !snapshot.source_documents.is_empty() {
        let src_docs_base = snapshot
            .source_documents_path
            .as_deref()
            .unwrap_or("source-documents");
        let mut source_doc_index: Vec<SourceDocumentIndexEntry> =
            Vec::with_capacity(snapshot.source_documents.len());
        for entry in &snapshot.source_documents {
            let sidecar_full = format!("{src_docs_base}/{}", entry.sidecar_path);
            let sidecar_str = serde_json::to_string_pretty(&entry.sidecar).map_err(|e| {
                RepositoryError::Serialize {
                    path: std::path::PathBuf::from(&sidecar_full),
                    source: e,
                }
            })?;
            target.save_text_file(&sidecar_full, &sidecar_str)?;
            if let Some(b64) = &entry.content_base64 {
                let bytes =
                    BASE64
                        .decode(b64)
                        .map_err(|e| RepositoryError::InvalidSnapshotData {
                            message: format!(
                                "base64 decode failed for '{}': {e}",
                                entry.content_path
                            ),
                        })?;
                let content_full = format!("{src_docs_base}/{}", entry.content_path);
                target.save_binary_file(&content_full, &bytes)?;
            }
            source_doc_index.push(SourceDocumentIndexEntry {
                document_id: entry.document_id.clone(),
                sidecar_path: entry.sidecar_path.clone(),
                content_path: entry.content_path.clone(),
                title: entry.title.clone(),
                sidecar_checksum: entry.sidecar_checksum.clone(),
                content_checksum: entry.content_checksum.clone(),
            });
        }
        manifest.source_documents_path = Some(src_docs_base.to_string());
        manifest.source_document_index = Some(source_doc_index);
    }

    // Restore repository-level provenance the snapshot carries (srs-rust#696): the
    // path-free RepositorySnapshot still preserves upstreamPackage + meta so a `.srsj`
    // load → scaffold keeps the seed's upstream provenance instead of dropping it.
    if snapshot.upstream_package.is_some() {
        manifest.upstream_package = snapshot.upstream_package.clone();
    }
    if let Some(meta) = &snapshot.meta {
        manifest.extra.insert("meta".to_string(), meta.clone());
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
    let snapshot = export_repository_snapshot_with_options(
        source,
        ExportSnapshotOptions {
            include_content_blobs: true,
        },
    )?;
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
            blueprints: pkg.blueprints.into_iter().map(|lb| lb.blueprint).collect(),
            themes: pkg.themes,
            vocabularies: pkg.vocabularies,
            lifecycles: pkg.lifecycles,
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
    let vocabularies = metadata
        .vocabularies
        .iter()
        .map(|p| load_typed_json::<Vocabulary>(source, &package_prefix, p))
        .collect::<Result<Vec<_>, _>>()?;
    let lifecycles = metadata
        .lifecycles
        .iter()
        .map(|p| load_typed_json::<Lifecycle>(source, &package_prefix, p))
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
        vocabularies,
        lifecycles,
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

    let mut vocabulary_paths = Vec::new();
    for vocab in &package.vocabularies {
        let path = format!(
            "vocabularies/{}-{}.json",
            slugify(&vocab.name),
            id_prefix(&vocab.id)?
        );
        write_repo_json(target, &base_prefix, &path, vocab)?;
        vocabulary_paths.push(path);
    }

    let mut lifecycle_paths = Vec::new();
    for lc in &package.lifecycles {
        let path = format!(
            "lifecycles/{}-{}.json",
            slugify(&lc.name),
            id_prefix(&lc.id)?
        );
        write_repo_json(target, &base_prefix, &path, lc)?;
        lifecycle_paths.push(path);
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
        "themes": theme_paths,
        "vocabularies": vocabulary_paths,
        "lifecycles": lifecycle_paths
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

pub(crate) fn ensure_target_empty(target: &dyn RepositoryStore) -> Result<(), RepositoryError> {
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

// ---------------------------------------------------------------------------
// upgrade_repository_paths
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstancePathRename {
    pub instance_id: String,
    pub from_path: String,
    pub to_path: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeRepositoryPathsResult {
    pub renames: Vec<InstancePathRename>,
    pub total_instances: usize,
    pub already_canonical_count: usize,
}

struct PlannedRename {
    manifest_index: usize,
    instance_id: String,
    from_path: String,
    to_path: String,
    value: serde_json::Value,
    sidecar_value: Option<serde_json::Value>,
}

fn collect_planned_renames(
    store: &dyn RepositoryStore,
    manifest: &crate::manifest::Manifest,
) -> Result<Vec<PlannedRename>, RepositoryError> {
    // Load every instance first so canonical paths can be derived over the whole set at once
    // (srs-rust#696): id8-colliding siblings normalise to their full-id form — order-independent,
    // never a collision error — so path normalization stays applicable to valid repositories
    // with prefix-colliding UUIDs.
    let instances: Vec<SnapshotInstance> = manifest
        .instance_index
        .iter()
        .map(|entry| {
            Ok(SnapshotInstance {
                instance_id: entry.instance_id.clone(),
                tier: entry.tier,
                title: entry.title.clone(),
                tags: entry.tags.clone(),
                value: store.load_instance_json(entry.path())?,
            })
        })
        .collect::<Result<_, RepositoryError>>()?;
    let canonical_paths = collision_safe_instance_paths(&instances, store)?;

    let mut planned: Vec<PlannedRename> = Vec::new();
    for ((idx, entry), (instance, canonical)) in manifest
        .instance_index
        .iter()
        .enumerate()
        .zip(instances.iter().zip(&canonical_paths))
    {
        if entry.path() != canonical {
            let old_sidecar = sidecar_path_for(entry.path());
            let sidecar_value = match store.load_instance_json(&old_sidecar) {
                Ok(v) => Some(v),
                Err(RepositoryError::NotFound { .. }) => None,
                Err(RepositoryError::Io { source, .. })
                    if source.kind() == std::io::ErrorKind::NotFound =>
                {
                    None
                }
                Err(e) => return Err(e),
            };
            planned.push(PlannedRename {
                manifest_index: idx,
                instance_id: entry.instance_id.clone(),
                from_path: entry.path().to_string(),
                to_path: canonical.clone(),
                value: instance.value.clone(),
                sidecar_value,
            });
        }
    }
    Ok(planned)
}

/// Returns `true` if any instance file path in the manifest index differs from its
/// canonical slug-id8 form (i.e. `upgrade_repository_paths` would rename at least one file).
/// Reads the manifest but performs no writes.
pub fn check_path_upgrade_needed(store: &dyn RepositoryStore) -> Result<bool, RepositoryError> {
    let manifest = store.load_manifest()?;
    let planned = collect_planned_renames(store, &manifest)?;
    Ok(!planned.is_empty())
}

pub fn upgrade_repository_paths(
    store: &dyn RepositoryStore,
) -> Result<UpgradeRepositoryPathsResult, RepositoryError> {
    let mut manifest = store.load_manifest()?;
    let total_instances = manifest.instance_index.len();

    let planned = collect_planned_renames(store, &manifest)?;

    if planned.is_empty() {
        return Ok(UpgradeRepositoryPathsResult {
            renames: vec![],
            already_canonical_count: total_instances,
            total_instances,
        });
    }

    // Phase 2: apply — write canonical instance files (and sidecars)
    for rename in &planned {
        ensure_instance_parent(store, &rename.to_path)?;
        store.save_instance_json(&rename.to_path, &rename.value)?;
        if let Some(sidecar_value) = &rename.sidecar_value {
            let new_sidecar = sidecar_path_for(&rename.to_path);
            ensure_instance_parent(store, &new_sidecar)?;
            store.save_instance_json(&new_sidecar, sidecar_value)?;
        }
    }

    // Phase 3: manifest update — persist index before any deletes (ADR-007)
    for rename in &planned {
        manifest.instance_index[rename.manifest_index].path = rename.to_path.clone();
    }
    store.save_manifest(&manifest)?;

    // Phase 4: cleanup — delete old files (best-effort; orphans are harmless per ADR-007)
    for rename in &planned {
        let _ = store.delete_instance_file(&rename.from_path);
        if rename.sidecar_value.is_some() {
            let _ = store.delete_instance_file(&sidecar_path_for(&rename.from_path));
        }
    }

    let renames: Vec<InstancePathRename> = planned
        .into_iter()
        .map(|r| InstancePathRename {
            instance_id: r.instance_id,
            from_path: r.from_path,
            to_path: r.to_path,
        })
        .collect();

    let already_canonical_count = total_instances - renames.len();
    Ok(UpgradeRepositoryPathsResult {
        renames,
        total_instances,
        already_canonical_count,
    })
}

pub(crate) fn canonical_instance_path(
    instance: &SnapshotInstance,
    store: &dyn RepositoryStore,
) -> Result<String, RepositoryError> {
    let id = &instance.instance_id;
    if id.len() < 8 {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!("instance_id '{id}' must be at least 8 characters"),
        });
    }
    instance_path_with_id_fragment(instance, store, &id[..8])
}

/// Storage path for an instance whose id fragment is `id_fragment` (a prefix of, or the
/// whole, `instance_id`). Factored out of [`canonical_instance_path`] so a colliding short
/// form can be widened to the full id without duplicating slug/tier-dir logic.
fn instance_path_with_id_fragment(
    instance: &SnapshotInstance,
    store: &dyn RepositoryStore,
    id_fragment: &str,
) -> Result<String, RepositoryError> {
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
        format!("{id_fragment}.json")
    } else {
        format!("{slug}-{id_fragment}.json")
    };
    let dir = match instance.tier {
        0 => store.record_tier_dir(RecordTier::Note),
        1 => store.record_tier_dir(RecordTier::Tier1),
        2 => store.record_tier_dir(RecordTier::Tier2),
        tier => {
            return Err(RepositoryError::InvalidSnapshotData {
                message: format!(
                    "instance '{}' has unknown tier {tier} — cannot map to a storage path",
                    instance.instance_id
                ),
            })
        }
    };
    Ok(format!("{dir}/{filename}"))
}

/// Repository-unique storage paths for `instances`, returned in the same order.
///
/// Each instance keeps the readable `slug-id8` short form (see [`canonical_instance_path`])
/// unless two or more instances in the set map to the same short form; **every** instance in
/// such a colliding group instead uses its full instance id (`slug-<full-uuid>.json`), which
/// is unique within a repository by construction.
///
/// The widening decision is a pure function of the whole instance set, so it is independent
/// of iteration order: the same repository always yields the same paths regardless of how the
/// instance index happens to be ordered. That order-independence is what keeps
/// `upgrade_repository_paths` idempotent and free of write-before-delete clobbering across
/// repeated passes.
///
/// This fixes srs-rust#696 (see ADR-040): two distinct, legitimately-valid instances can share
/// their first 8 hex characters — e.g. deterministic UUID5s like gallery.srsj's decision
/// instances `…5801`/`…5802`, both of which start `00000000` — and the id8-only scheme mapped
/// them to the same file, making an otherwise valid repository fail to load or copy.
fn collision_safe_instance_paths(
    instances: &[SnapshotInstance],
    store: &dyn RepositoryStore,
) -> Result<Vec<String>, RepositoryError> {
    let shorts: Vec<String> = instances
        .iter()
        .map(|instance| canonical_instance_path(instance, store))
        .collect::<Result<_, _>>()?;

    let mut short_counts: HashMap<&str, usize> = HashMap::with_capacity(shorts.len());
    for short in &shorts {
        *short_counts.entry(short.as_str()).or_default() += 1;
    }

    instances
        .iter()
        .zip(&shorts)
        .map(|(instance, short)| {
            if short_counts[short.as_str()] > 1 {
                instance_path_with_id_fragment(instance, store, &instance.instance_id)
            } else {
                Ok(short.clone())
            }
        })
        .collect()
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
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn make_input() -> InitializeRepositoryInput {
        InitializeRepositoryInput {
            repository: RepositoryMetadata {
                // Must be a UUID: the root-container embed inherits this id, and
                // validation now checks the embed when no container file exists.
                repository_id: "c0c0c0c0-0000-4000-8000-c0c0c0c0c0c0".to_string(),
                namespace: "com.semanticops.copy".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: Some("Copy Test".to_string()),
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
            instructions: None,
            ai_guidance: serde_json::Value::Null,
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "".to_string(),
            extra: HashMap::new(),
        });

        let target = MemoryStore::uninitialized();
        let result = import_repository_snapshot(&target, &snapshot);
        assert!(matches!(
            result,
            Err(RepositoryError::InvalidSnapshotData { .. })
        ));
    }

    #[test]
    fn copy_preserves_rfc013_root_container_pointer() {
        // Source repo with a note (the future root container's identity/member) and
        // manifest.container pointing at a real container — not the auto-generated
        // placeholder that initialize_repository assigns (which keys off repositoryId).
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut manifest = source.load_manifest().unwrap();
        manifest.container = Some(Container {
            container_id: "99999999-9999-4999-8999-999999999999".to_string(),
            title: "Root".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
            member_instance_ids: Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]),
            root_instance_ids: Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]),
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: HashMap::new(),
        });
        source.save_manifest(&manifest).unwrap();

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
            container_id: "99999999-9999-4999-8999-999999999999".to_string(),
            title: "Root".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
            member_instance_ids: Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]),
            root_instance_ids: Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]),
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: HashMap::new(),
        });

        // Import into a .srsj JsonStore bundle — this is the exact `srs repo copy
        // --to *.srsj` path used to regenerate single-file snapshots.
        let tmp = TempDir::new().unwrap();
        let target = JsonStore::create(tmp.path().join("repo.srsj")).unwrap();
        import_repository_snapshot(&target, &snapshot).unwrap();

        let copied = target.load_manifest().unwrap();
        let container = copied
            .container
            .as_ref()
            .expect("manifest.container must survive copy");
        assert_eq!(
            container.container_id, "99999999-9999-4999-8999-999999999999",
            "manifest.container must point at the real root container, not the repositoryId placeholder"
        );

        // And `repo navigation` — what srs-gov and srs-web's GovernanceShell call —
        // must resolve manifest.container's id instead of failing with
        // "container not found: <repositoryId>" (the bug: the placeholder container
        // initialize_repository assigns keys off repositoryId, and previously survived
        // the copy uncontested since manifest.container/containerIndex weren't in the
        // snapshot at all).
        let container = get_container(&target, "99999999-9999-4999-8999-999999999999")
            .expect("manifest.container's id must resolve to a real container post-copy");
        assert_eq!(
            container.container_id,
            "99999999-9999-4999-8999-999999999999"
        );
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
            vocabularies: vec![],
            lifecycles: vec![],
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
            identity_instance_id: None,
            member_instance_ids: Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]),
            root_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: HashMap::new(),
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
        let summaries = list_containers(&target, &ContainerListFilter::default()).unwrap();
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

    #[test]
    fn copy_tier1_record_no_type_name_produces_id_only_filename() {
        // Tier-1 TypedRecords have named fields but no type binding — they
        // carry no `typeName` field, so the slug falls back to id-only.
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut snapshot = export_repository_snapshot(&source).unwrap();
        snapshot.instances.push(SnapshotInstance {
            instance_id: "55555555-5555-4555-b555-555555555555".to_string(),
            tier: 1,
            title: None,
            tags: None,
            value: serde_json::json!({
                "instanceId": "55555555-5555-4555-b555-555555555555",
                "fields": [{"name": "description", "value": "some text"}]
            }),
        });

        let temp = TempDir::new().unwrap();
        let target = FileStore::new(temp.path());
        import_repository_snapshot(&target, &snapshot).unwrap();

        assert!(
            temp.path().join("records/tier-1/55555555.json").exists(),
            "expected records/tier-1/55555555.json (id-only — tier-1 has no typeName)"
        );
    }

    #[test]
    fn export_fails_with_instance_load_error_when_record_missing() {
        // A manifest entry pointing to a path with no data should surface
        // InstanceLoad with the instance_id and path in the error, not a
        // generic IO error with no identifying context.
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();

        // Inject a manifest entry whose path has no corresponding data entry.
        let mut manifest = source.load_manifest().unwrap();
        manifest
            .instance_index
            .push(crate::index::InstanceIndexEntry {
                instance_id: "deadbeef-dead-4ead-8ead-deadbeefcafe".to_string(),
                tier: 0,
                path: "records/notes/ghost.json".to_string(),
                title: None,
                tags: None,
            });
        source.save_manifest(&manifest).unwrap();

        let result = export_repository_snapshot(&source);

        match result {
            Err(RepositoryError::InstanceLoad {
                ref instance_id,
                ref path,
                ..
            }) => {
                assert_eq!(instance_id, "deadbeef-dead-4ead-8ead-deadbeefcafe");
                assert_eq!(path.to_str().unwrap(), "records/notes/ghost.json");
            }
            other => panic!("expected InstanceLoad error, got: {other:?}"),
        }
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("deadbeef-dead-4ead-8ead-deadbeefcafe"),
            "error message must contain instance_id: {err_msg}"
        );
        assert!(
            err_msg.contains("records/notes/ghost.json"),
            "error message must contain source path: {err_msg}"
        );
    }

    #[test]
    fn import_widens_path_on_id8_collision() {
        // srs-rust#696: two tier-0 instances with the same slug AND the same first 8 UUID
        // characters both want "records/notes/same-title-aaaaaaaa.json". A valid repository
        // may legitimately contain such instances (e.g. deterministic UUID5s), so the import
        // must NOT fail. Widening is order-independent (ADR-040): every instance in a colliding
        // group takes its full-id form, so neither instance is dropped or silently overwritten.
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut snapshot = export_repository_snapshot(&source).unwrap();

        snapshot.instances.push(SnapshotInstance {
            instance_id: "aaaaaaaa-0000-4000-8000-000000000001".to_string(),
            tier: 0,
            title: Some(serde_json::json!("same title")),
            tags: None,
            value: serde_json::json!({
                "instanceId": "aaaaaaaa-0000-4000-8000-000000000001"
            }),
        });
        snapshot.instances.push(SnapshotInstance {
            instance_id: "aaaaaaaa-0000-4000-8000-000000000002".to_string(),
            tier: 0,
            title: Some(serde_json::json!("same title")),
            tags: None,
            value: serde_json::json!({
                "instanceId": "aaaaaaaa-0000-4000-8000-000000000002"
            }),
        });

        let target = MemoryStore::uninitialized();
        import_repository_snapshot(&target, &snapshot)
            .expect("prefix-colliding instances must import, not error (srs-rust#696)");

        let manifest = target.load_manifest().unwrap();
        let path_of = |id: &str| -> String {
            manifest
                .instance_index
                .iter()
                .find(|e| e.instance_id == id)
                .unwrap_or_else(|| panic!("instance {id} missing from index"))
                .path()
                .to_string()
        };
        let p1 = path_of("aaaaaaaa-0000-4000-8000-000000000001");
        let p2 = path_of("aaaaaaaa-0000-4000-8000-000000000002");

        // Order-independent widening: BOTH colliding instances take their full-id form, so the
        // result does not depend on index order (ADR-040). Distinct files, neither dropped.
        assert_eq!(
            p1, "records/notes/same-title-aaaaaaaa-0000-4000-8000-000000000001.json",
            "first widens to full id"
        );
        assert_eq!(
            p2, "records/notes/same-title-aaaaaaaa-0000-4000-8000-000000000002.json",
            "second widens to full id"
        );
        assert_ne!(p1, p2, "colliding instances must land on distinct paths");

        // Both files are materialized and carry the right instance.
        assert_eq!(
            target.load_instance_json(&p1).unwrap()["instanceId"],
            serde_json::json!("aaaaaaaa-0000-4000-8000-000000000001")
        );
        assert_eq!(
            target.load_instance_json(&p2).unwrap()["instanceId"],
            serde_json::json!("aaaaaaaa-0000-4000-8000-000000000002")
        );
    }

    #[test]
    fn copy_repository_widens_id8_colliding_instances() {
        // The issue's CLI reproduction — `srs repo copy --from <colliding>.srsj` — must succeed on
        // a repository whose deterministic UUIDs collide in their first 8 hex chars (srs-rust#696).
        // copy_repository = export + import; both colliding instances must land on distinct files,
        // and re-copying the resulting repository is stable (idempotent, order-independent).
        let colliding = |suffix: &str| SnapshotInstance {
            instance_id: format!("aaaaaaaa-0000-4000-8000-00000000000{suffix}"),
            tier: 0,
            title: Some(serde_json::json!("same title")),
            tags: None,
            value: serde_json::json!({
                "instanceId": format!("aaaaaaaa-0000-4000-8000-00000000000{suffix}")
            }),
        };

        let seed = MemoryStore::uninitialized();
        seed.initialize_repository(&make_input()).unwrap();
        let mut snapshot = export_repository_snapshot(&seed).unwrap();
        snapshot.instances.push(colliding("1"));
        snapshot.instances.push(colliding("2"));

        let first = MemoryStore::uninitialized();
        import_repository_snapshot(&first, &snapshot).unwrap();

        // Copy the whole colliding repository — the operation the issue reproduces.
        let second = MemoryStore::uninitialized();
        copy_repository(&first, &second)
            .expect("repo copy must not fail on prefix-colliding UUIDs (srs-rust#696)");

        let paths: Vec<String> = second
            .load_manifest()
            .unwrap()
            .instance_index
            .iter()
            .filter(|e| {
                e.instance_id
                    .starts_with("aaaaaaaa-0000-4000-8000-0000000000")
            })
            .map(|e| e.path().to_string())
            .collect();
        assert_eq!(paths.len(), 2, "both colliding instances must be copied");
        assert_ne!(
            paths[0], paths[1],
            "colliding instances copied to distinct paths"
        );
        // Both widened to full-id form (order-independent, ADR-040).
        assert!(
            paths
                .iter()
                .all(|p| p.ends_with("1.json") || p.ends_with("2.json")),
            "widened paths carry the full instance id: {paths:?}"
        );
    }

    #[test]
    fn snapshot_preserves_upstream_package_and_meta() {
        // srs-rust#696: the path-free RepositorySnapshot must still carry repository
        // provenance, so a `.srsj` load (export snapshot → import into a MemVfs) keeps
        // `upstreamPackage` — required by scaffold_new_repository — and `meta`, rather
        // than dropping them (which broke the create-document / walkthrough flows).
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let mut manifest = source.load_manifest().unwrap();
        manifest.upstream_package = Some(UpstreamPackage {
            package_id: "pkg-123".to_string(),
            namespace: "com.example.seed".to_string(),
            name: "seed".to_string(),
            version: "1.0.0".to_string(),
            installed_at: "2026-01-01T00:00:00Z".to_string(),
        });
        manifest.extra.insert(
            "meta".to_string(),
            serde_json::json!({"sourceOfTruth": "records"}),
        );
        source.save_manifest(&manifest).unwrap();

        let snapshot = export_repository_snapshot(&source).unwrap();
        assert!(
            snapshot.upstream_package.is_some(),
            "export must capture upstreamPackage"
        );
        assert_eq!(
            snapshot.meta,
            Some(serde_json::json!({"sourceOfTruth": "records"}))
        );

        let target = MemoryStore::uninitialized();
        import_repository_snapshot(&target, &snapshot).unwrap();
        let out = target.load_manifest().unwrap();
        let up = out
            .upstream_package
            .expect("import must restore upstreamPackage");
        assert_eq!(up.package_id, "pkg-123");
        assert_eq!(up.namespace, "com.example.seed");
        assert_eq!(
            out.extra.get("meta"),
            Some(&serde_json::json!({"sourceOfTruth": "records"}))
        );
    }

    #[test]
    fn export_fails_on_malformed_package_refs() {
        // When manifest.packageRefs is present but is not a valid
        // Vec<{mode, path}> array, export must return InvalidSnapshotData
        // rather than silently treating sub-packages as absent.
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();

        let mut manifest = source.load_manifest().unwrap();
        manifest
            .extra
            .insert("packageRefs".to_string(), serde_json::json!("not-an-array"));
        source.save_manifest(&manifest).unwrap();

        let result = export_repository_snapshot(&source);

        match result {
            Err(RepositoryError::InvalidSnapshotData { ref message }) => {
                assert!(
                    message.contains("malformed packageRefs"),
                    "error must mention packageRefs: {message}"
                );
            }
            other => panic!("expected InvalidSnapshotData error, got: {other:?}"),
        }
    }

    // --- Batch atomicity test (ADR-021) ---

    #[test]
    fn json_store_partial_import_leaves_file_unchanged() {
        // Two valid instances are written to in-memory state before a path-collision
        // error on the 3rd instance forces abort_batch(). The .srsj file on disk
        // must be unchanged from its pre-import state.
        let tmp = TempDir::new().unwrap();
        let srsj_path = tmp.path().join("target.srsj");
        let target = JsonStore::create(&srsj_path).unwrap();

        let initial_contents = std::fs::read_to_string(&srsj_path).unwrap();

        let mut snapshot = {
            let source = MemoryStore::uninitialized();
            source.initialize_repository(&make_input()).unwrap();
            export_repository_snapshot(&source).unwrap()
        };

        // Instance 3 has an unknown tier, which cannot be mapped to a storage path and
        // triggers InvalidSnapshotData after instances 1 and 2 have been saved in-memory.
        // (A plain id8 path collision is no longer an error — see srs-rust#696 — so this
        // uses a genuine per-instance failure to exercise abort_batch's rollback.)
        snapshot.instances = vec![
            SnapshotInstance {
                instance_id: "aaaaaaaa-0001-0001-0001-000000000001".to_string(),
                tier: 0,
                title: None,
                tags: None,
                value: serde_json::json!({"instanceId":"aaaaaaaa-0001-0001-0001-000000000001"}),
            },
            SnapshotInstance {
                instance_id: "bbbbbbbb-0002-0002-0002-000000000002".to_string(),
                tier: 0,
                title: None,
                tags: None,
                value: serde_json::json!({"instanceId":"bbbbbbbb-0002-0002-0002-000000000002"}),
            },
            SnapshotInstance {
                instance_id: "cccccccc-0003-0003-0003-000000000003".to_string(),
                tier: 9,
                title: None,
                tags: None,
                value: serde_json::json!({"instanceId":"cccccccc-0003-0003-0003-000000000003"}),
            },
        ];

        let result = import_repository_snapshot(&target, &snapshot);
        assert!(
            matches!(result, Err(RepositoryError::InvalidSnapshotData { .. })),
            "expected InvalidSnapshotData from unknown tier, got: {result:?}"
        );

        let final_contents = std::fs::read_to_string(&srsj_path).unwrap();
        assert_eq!(
            final_contents, initial_contents,
            "abort_batch must leave the disk file in its pre-import state (no partial records)"
        );
    }

    // --- upgrade_repository_paths tests ---

    fn make_upgrade_input() -> InitializeRepositoryInput {
        InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "upgrade-test-repo".to_string(),
                namespace: "com.example.upgrade".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "upgrade-test-pkg".to_string(),
                namespace: "com.example.upgrade".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        }
    }

    fn inject_non_canonical_instance(
        store: &dyn RepositoryStore,
        instance_id: &str,
        tier: u8,
        path: &str,
        value: serde_json::Value,
    ) {
        store
            .ensure_instance_dir(path.rsplit_once('/').map(|(d, _)| d).unwrap_or("records"))
            .unwrap();
        store.save_instance_json(path, &value).unwrap();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .instance_index
            .push(crate::index::InstanceIndexEntry {
                instance_id: instance_id.to_string(),
                tier,
                path: path.to_string(),
                title: None,
                tags: None,
            });
        store.save_manifest(&manifest).unwrap();
    }

    #[test]
    fn upgrade_no_op_when_paths_canonical() {
        // A repo initialised via copy_repository already has canonical paths.
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_upgrade_input()).unwrap();
        let temp = TempDir::new().unwrap();
        let store = FileStore::new(temp.path());
        copy_repository(&source, &store).unwrap();

        let result = upgrade_repository_paths(&store).unwrap();
        assert_eq!(
            result.renames.len(),
            0,
            "should be a no-op on canonical repo"
        );
    }

    #[test]
    fn upgrade_renames_non_canonical_tier2_path() {
        let store = MemoryStore::uninitialized();
        store.initialize_repository(&make_upgrade_input()).unwrap();

        // Inject a tier-2 instance at a non-canonical path.
        let id = "aabbccdd-1234-5678-90ab-cdef01234567";
        let value = serde_json::json!({"typeName": "com.example/my-type", "id": id});
        inject_non_canonical_instance(&store, id, 2, "records/tier-2/old-name.json", value);

        let result = upgrade_repository_paths(&store).unwrap();
        assert_eq!(result.renames.len(), 1);
        assert_eq!(result.renames[0].from_path, "records/tier-2/old-name.json");
        assert_eq!(
            result.renames[0].to_path,
            "records/tier-2/com-example-my-type-aabbccdd.json"
        );
        assert_eq!(result.total_instances, 1);

        // Old path gone, new path present in store.
        assert!(
            store
                .load_instance_json("records/tier-2/old-name.json")
                .is_err(),
            "old path should be deleted"
        );
        let canonical =
            store.load_instance_json("records/tier-2/com-example-my-type-aabbccdd.json");
        assert!(canonical.is_ok(), "canonical path should exist");

        // Manifest updated.
        let manifest = store.load_manifest().unwrap();
        assert_eq!(
            manifest.instance_index[0].path,
            "records/tier-2/com-example-my-type-aabbccdd.json"
        );
    }

    #[test]
    fn upgrade_renames_non_canonical_note_path() {
        let store = MemoryStore::uninitialized();
        store.initialize_repository(&make_upgrade_input()).unwrap();

        let id = "11223344-0000-0000-0000-000000000000";
        let value = serde_json::json!({"title": "My Note", "id": id});
        inject_non_canonical_instance(&store, id, 0, "records/notes/raw-note.json", value.clone());
        // Patch title in manifest entry for slug derivation.
        let mut manifest = store.load_manifest().unwrap();
        manifest.instance_index[0].title = Some(serde_json::Value::String("My Note".to_string()));
        store.save_manifest(&manifest).unwrap();

        let result = upgrade_repository_paths(&store).unwrap();
        assert_eq!(result.renames.len(), 1);
        assert_eq!(result.renames[0].from_path, "records/notes/raw-note.json");
        assert_eq!(
            result.renames[0].to_path,
            "records/notes/my-note-11223344.json"
        );
    }

    #[test]
    fn upgrade_is_idempotent() {
        let store = MemoryStore::uninitialized();
        store.initialize_repository(&make_upgrade_input()).unwrap();

        let id = "aabbccdd-1234-5678-90ab-cdef01234567";
        let value = serde_json::json!({"typeName": "com.example/my-type", "id": id});
        inject_non_canonical_instance(&store, id, 2, "records/tier-2/old-name.json", value);

        let first = upgrade_repository_paths(&store).unwrap();
        assert_eq!(first.renames.len(), 1);

        let second = upgrade_repository_paths(&store).unwrap();
        assert_eq!(second.renames.len(), 0, "second run should be a no-op");
        assert_eq!(second.total_instances, 1);
    }

    #[test]
    fn upgrade_does_not_rename_already_canonical_paths() {
        // copy_repository writes canonical paths; upgrade is a no-op afterward.
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_upgrade_input()).unwrap();

        let target = MemoryStore::uninitialized();
        copy_repository(&source, &target).unwrap();

        let result = upgrade_repository_paths(&target).unwrap();
        assert_eq!(result.renames.len(), 0);
    }

    #[test]
    fn upgrade_renames_non_canonical_path_on_filestore() {
        // Cross-store roundtrip: inject a non-canonical file on disk, verify filesystem state.
        let temp = TempDir::new().unwrap();
        let store = FileStore::new(temp.path());

        // Bootstrap with copy_repository so the manifest and package exist.
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_upgrade_input()).unwrap();
        copy_repository(&source, &store).unwrap();

        // Inject a non-canonical tier-1 file directly to disk via the store.
        let id = "ddccbbaa-1234-5678-90ab-cdef01234567";
        let value = serde_json::json!({"typeName": "com.example/section", "id": id});
        inject_non_canonical_instance(&store, id, 1, "records/tier-1/old-section.json", value);

        let canonical_path = "records/tier-1/com-example-section-ddccbbaa.json";

        let result = upgrade_repository_paths(&store).unwrap();
        assert_eq!(result.renames.len(), 1);
        assert_eq!(result.renames[0].to_path, canonical_path);

        // Verify filesystem state.
        assert!(
            !temp.path().join("records/tier-1/old-section.json").exists(),
            "old file must not exist on disk"
        );
        assert!(
            temp.path().join(canonical_path).exists(),
            "canonical file must exist on disk"
        );

        // Filesystem state is the focus here; full schema validation is covered by dogfooding.
    }

    #[test]
    fn upgrade_moves_revision_sidecar() {
        use crate::revision_service::sidecar_path_for;

        let store = MemoryStore::uninitialized();
        store.initialize_repository(&make_upgrade_input()).unwrap();

        let id = "aabbccdd-1234-5678-90ab-cdef01234567";
        let old_path = "records/tier-2/old-name.json";
        let canonical_path = "records/tier-2/com-example-my-type-aabbccdd.json";
        let old_sidecar = sidecar_path_for(old_path);
        let new_sidecar = sidecar_path_for(canonical_path);

        let value = serde_json::json!({"typeName": "com.example/my-type", "id": id});
        inject_non_canonical_instance(&store, id, 2, old_path, value);

        // Write a fake sidecar at the old path.
        let sidecar_value = serde_json::json!({"recordId": id, "revisions": []});
        store
            .save_instance_json(&old_sidecar, &sidecar_value)
            .unwrap();

        upgrade_repository_paths(&store).unwrap();

        // Old sidecar gone, new sidecar present.
        assert!(
            store.load_instance_json(&old_sidecar).is_err(),
            "old sidecar should be deleted"
        );
        assert!(
            store.load_instance_json(&new_sidecar).is_ok(),
            "new sidecar should exist at canonical path"
        );
    }

    // --- Source document snapshot tests (RFC-017, ADR-031) ---

    fn make_source_doc_manifest(store: &dyn RepositoryStore) {
        let mut manifest = store.load_manifest().unwrap();
        manifest.source_documents_path = Some("source-documents".to_string());
        manifest.source_document_index = Some(vec![SourceDocumentIndexEntry {
            document_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_string(),
            sidecar_path: "my-doc.meta.json".to_string(),
            content_path: "my-doc.pdf".to_string(),
            title: None,
            sidecar_checksum: None,
            content_checksum: None,
        }]);
        store.save_manifest(&manifest).unwrap();
    }

    const SIDECAR_JSON: &str = r#"{
        "documentId": "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
        "contentPath": "my-doc.pdf",
        "contentType": "application/pdf",
        "createdAt": "2026-01-01T00:00:00Z"
    }"#;

    #[test]
    fn source_document_binary_roundtrip() {
        let binary_content = b"PDF binary content \x00\x01\x02";

        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        source
            .save_text_file("source-documents/my-doc.meta.json", SIDECAR_JSON)
            .unwrap();
        source
            .save_binary_file("source-documents/my-doc.pdf", binary_content)
            .unwrap();
        make_source_doc_manifest(&source);

        // Export with blobs.
        let snapshot = export_repository_snapshot_with_options(
            &source,
            ExportSnapshotOptions {
                include_content_blobs: true,
            },
        )
        .unwrap();
        assert_eq!(snapshot.source_documents.len(), 1);
        let sd = &snapshot.source_documents[0];
        assert_eq!(sd.document_id, "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee");
        assert_eq!(sd.sidecar_path, "my-doc.meta.json");
        assert_eq!(sd.content_path, "my-doc.pdf");
        assert!(sd.content_base64.is_some(), "blob must be present");

        // Import into a target MemoryStore and verify both files materialise.
        let target = MemoryStore::uninitialized();
        import_repository_snapshot(&target, &snapshot).unwrap();

        let sidecar_str = target
            .load_text_file("source-documents/my-doc.meta.json")
            .unwrap();
        let sidecar: serde_json::Value = serde_json::from_str(&sidecar_str).unwrap();
        assert_eq!(
            sidecar["documentId"],
            "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
        );
        let recovered_bytes = target
            .load_binary_file("source-documents/my-doc.pdf")
            .unwrap();
        assert_eq!(recovered_bytes, binary_content);

        // Manifest must carry sourceDocumentIndex.
        let manifest = target.load_manifest().unwrap();
        let idx = manifest.source_document_index.as_ref().unwrap();
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].document_id, "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee");
    }

    #[test]
    fn source_document_text_only_export_excludes_blob() {
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        source
            .save_text_file("source-documents/my-doc.meta.json", SIDECAR_JSON)
            .unwrap();
        source
            .save_binary_file("source-documents/my-doc.pdf", b"binary")
            .unwrap();
        make_source_doc_manifest(&source);

        // Default export (include_content_blobs: false).
        let snapshot = export_repository_snapshot(&source).unwrap();
        assert_eq!(snapshot.source_documents.len(), 1);
        assert!(
            snapshot.source_documents[0].content_base64.is_none(),
            "text-only export must not include binary blob"
        );
        // Sidecar metadata must still be present.
        assert_eq!(
            snapshot.source_documents[0].sidecar["contentType"],
            "application/pdf"
        );
    }

    #[test]
    fn content_file_tombstone_during_export() {
        // Index entry present, sidecar present, binary absent → tombstone (RFC-017 R12).
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        source
            .save_text_file("source-documents/my-doc.meta.json", SIDECAR_JSON)
            .unwrap();
        // No binary file written.
        make_source_doc_manifest(&source);

        let snapshot = export_repository_snapshot_with_options(
            &source,
            ExportSnapshotOptions {
                include_content_blobs: true,
            },
        )
        .unwrap();
        assert_eq!(
            snapshot.source_documents.len(),
            1,
            "tombstone entry must still appear in snapshot"
        );
        assert!(
            snapshot.source_documents[0].content_base64.is_none(),
            "missing binary must yield content_base64: None"
        );
    }

    #[test]
    fn sidecar_absent_tombstone_during_export() {
        // Index entry present but sidecar file is missing → whole entry skipped gracefully.
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        // Neither sidecar nor binary written.
        make_source_doc_manifest(&source);

        let snapshot = export_repository_snapshot_with_options(
            &source,
            ExportSnapshotOptions {
                include_content_blobs: true,
            },
        )
        .unwrap();
        assert_eq!(
            snapshot.source_documents.len(),
            0,
            "entry with absent sidecar must be skipped"
        );
    }

    #[test]
    fn copy_preserves_source_documents() {
        let binary_content = b"source document bytes";

        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        source
            .save_text_file("source-documents/my-doc.meta.json", SIDECAR_JSON)
            .unwrap();
        source
            .save_binary_file("source-documents/my-doc.pdf", binary_content)
            .unwrap();
        make_source_doc_manifest(&source);

        let temp = TempDir::new().unwrap();
        let target = FileStore::new(temp.path());
        copy_repository(&source, &target).unwrap();

        let recovered = target
            .load_text_file("source-documents/my-doc.meta.json")
            .unwrap();
        let sidecar: serde_json::Value = serde_json::from_str(&recovered).unwrap();
        assert_eq!(
            sidecar["documentId"],
            "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
        );
        let recovered_bytes = target
            .load_binary_file("source-documents/my-doc.pdf")
            .unwrap();
        assert_eq!(recovered_bytes, binary_content);
    }

    #[test]
    fn snapshot_with_source_docs_passes_path_guard() {
        // The path guard must still pass when source_documents is populated.
        // Field names in SourceDocumentSnapshot (sidecarPath, contentPath, documentId)
        // contain "Path" with uppercase P — never the bare lowercase "path" key the
        // guard checks for. Sidecar field names (contentPath, contentType, …) similarly
        // contain no standalone "path" key.
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        source
            .save_text_file("source-documents/my-doc.meta.json", SIDECAR_JSON)
            .unwrap();
        make_source_doc_manifest(&source);

        let snapshot = export_repository_snapshot_with_options(
            &source,
            ExportSnapshotOptions {
                include_content_blobs: false,
            },
        )
        .unwrap();
        assert!(!snapshot.source_documents.is_empty());
        let text = serde_json::to_string(&snapshot).unwrap();
        assert!(
            !text.contains("\"path\""),
            "bare \"path\" key must not appear"
        );
        assert!(
            !text.contains("package/"),
            "package/ prefix must not appear"
        );
        assert!(
            !text.contains("records/"),
            "records/ prefix must not appear"
        );
    }

    #[test]
    fn copy_preserves_source_doc_checksum_metadata() {
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        source
            .save_text_file("source-documents/my-doc.meta.json", SIDECAR_JSON)
            .unwrap();
        source
            .save_binary_file("source-documents/my-doc.pdf", b"pdf bytes")
            .unwrap();

        // Set up manifest with non-None checksum metadata.
        let mut manifest = source.load_manifest().unwrap();
        manifest.source_documents_path = Some("source-documents".to_string());
        manifest.source_document_index = Some(vec![SourceDocumentIndexEntry {
            document_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_string(),
            sidecar_path: "my-doc.meta.json".to_string(),
            content_path: "my-doc.pdf".to_string(),
            title: Some("My Test Doc".to_string()),
            sidecar_checksum: Some("sha256:aaabbb".to_string()),
            content_checksum: Some("sha256:cccddd".to_string()),
        }]);
        source.save_manifest(&manifest).unwrap();

        let temp = TempDir::new().unwrap();
        let target = FileStore::new(temp.path());
        copy_repository(&source, &target).unwrap();

        let target_manifest = target.load_manifest().unwrap();
        let idx = target_manifest
            .source_document_index
            .as_ref()
            .expect("source_document_index must be present");
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].title, Some("My Test Doc".to_string()));
        assert_eq!(idx[0].sidecar_checksum, Some("sha256:aaabbb".to_string()));
        assert_eq!(idx[0].content_checksum, Some("sha256:cccddd".to_string()));
    }
}
