use crate::container_service::{get_container, list_containers, ContainerListFilter};
use crate::error::RepositoryError;
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
    #[serde(deserialize_with = "crate::field_json::deserialize_fields_compat")]
    pub fields: Vec<Field>,
    #[serde(deserialize_with = "crate::type_json::deserialize_types_compat")]
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
    /// `manifest.dataModelRevision` (RFC-033/[R21]) — the data-model generation
    /// is repository identity; dropping it on a copy would silently demote a
    /// rev-2 repository to the rev-0 compatibility path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_model_revision: Option<serde_json::Value>,
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

    // RFC-038 Phase 3: enumerate instances from one catalog snapshot — the
    // manifest index is no longer read. Snapshot `title`/`tags` are derived
    // from the entity bodies.
    let catalog = source.catalog()?;
    let mut instances = Vec::new();
    for entry in &catalog.instances {
        let locator = entry.locator.as_deref().unwrap_or_default();
        let value =
            source
                .load_instance_json(locator)
                .map_err(|e| RepositoryError::InstanceLoad {
                    instance_id: entry.id.clone(),
                    path: std::path::PathBuf::from(locator),
                    source: Box::new(e) as Box<dyn std::error::Error + Send + Sync>,
                })?;
        let r =
            crate::store::instance_ref_from_body(entry.id.clone(), entry.tier.unwrap_or(2), &value);
        instances.push(SnapshotInstance {
            instance_id: entry.id.clone(),
            tier: entry.tier.unwrap_or(2),
            title: r.title.map(serde_json::Value::String),
            tags: if r.tags.is_empty() {
                None
            } else {
                Some(r.tags)
            },
            value,
        });
    }

    // Identify the embed-only root container ID, if any. An embed-only root exists in
    // `manifest.container` but has no file-backed container — it is already
    // preserved in `root_container` below, so including it in `containers` would
    // double-capture it and cause `create_container` UUID-validation failures on import
    // for repositories whose id pre-dates the UUID requirement (e.g. legacy test repos).
    let embed_only_root_id: Option<String> = manifest.container.as_ref().and_then(|mc| {
        let file_backed = catalog.containers.iter().any(|e| {
            e.id == mc.container_id
                && e.locator.as_deref() != Some(crate::catalog::ROOT_CONTAINER_LOCATOR)
        });
        if file_backed {
            None
        } else {
            Some(mc.container_id.clone())
        }
    });

    let mut containers = Vec::new();
    for summary in list_containers(source, &ContainerListFilter::default())? {
        if embed_only_root_id.as_deref() == Some(summary.container_id.as_str()) {
            continue;
        }
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

    // Collect source documents (RFC-017; ADR-031). RFC-038 [R25]: resolved via
    // sidecar discovery — the catalog's source-document set — not
    // `manifest.source_document_index`; tombstone behavior (an absent content
    // file) is keyed to the sidecar itself, never an index entry.
    let source_documents_path = manifest.source_documents_path.clone();
    let src_docs_base = source_documents_path
        .as_deref()
        .unwrap_or("source-documents");

    let mut source_documents = Vec::new();
    for entry in &catalog.source_documents {
        let sidecar_full = entry.locator.as_deref().unwrap_or_default();
        let sidecar_str = match source.load_text_file(sidecar_full) {
            Ok(s) => s,
            Err(ref e) if e.is_not_found() => continue, // vanished between catalog build and read
            Err(e) => return Err(e),
        };
        let sidecar: serde_json::Value = serde_json::from_str(&sidecar_str).map_err(|e| {
            RepositoryError::InvalidSnapshotData {
                message: format!("malformed sidecar '{sidecar_full}': {e}"),
            }
        })?;
        let sidecar_path = sidecar_full
            .strip_prefix(&format!("{src_docs_base}/"))
            .unwrap_or(sidecar_full)
            .to_string();
        let content_path = sidecar
            .get("contentPath")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RepositoryError::InvalidSnapshotData {
                message: format!("sidecar '{sidecar_full}' has no contentPath"),
            })?
            .to_string();
        let title = sidecar
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let content_base64 = if options.include_content_blobs {
            let content_full = format!("{src_docs_base}/{content_path}");
            match source.load_binary_file(&content_full) {
                Ok(bytes) => Some(BASE64.encode(&bytes)),
                Err(ref e) if e.is_not_found() => None, // tombstone: RFC-017 [R15], keyed to the sidecar
                Err(e) => return Err(e),
            }
        } else {
            None
        };

        source_documents.push(SourceDocumentSnapshot {
            document_id: entry.id.clone(),
            sidecar_path,
            content_path,
            sidecar,
            content_base64,
            title,
            sidecar_checksum: None,
            content_checksum: None,
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
        container_index: None,
        // Retired by RFC-038 Change K — `containers` above (catalog-backed) is
        // the real data; this field is never populated or read any more.
        relations: load_relations(source)?,
        source_documents_path: if source_documents.is_empty() {
            None
        } else {
            Some(src_docs_base.to_string())
        },
        source_documents,
        upstream_package: manifest.upstream_package.clone(),
        meta: manifest.extra.get("meta").cloned(),
        data_model_revision: manifest.extra.get("dataModelRevision").cloned(),
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
    // RFC-038 [R1]/[R22]: writes only the entity files — membership comes from the tree,
    // never `manifest.instance_index` (retired by Change K).
    let instance_paths = collision_safe_instance_paths(&snapshot.instances, target)?;
    let mut used_paths: HashMap<&str, &str> = HashMap::with_capacity(snapshot.instances.len());
    for (instance, rel_path) in snapshot.instances.iter().zip(&instance_paths) {
        // After widening, an identical path can only mean a genuine duplicate instance id.
        if let Some(first_id) = used_paths.insert(rel_path.as_str(), instance.instance_id.as_str())
        {
            return Err(RepositoryError::InvalidSnapshotData {
                message: format!(
                    "duplicate instance id — '{first_id}' and '{}' both map to the same path '{rel_path}'",
                    instance.instance_id
                ),
            });
        }
        ensure_instance_parent(target, rel_path)?;
        target.save_instance_json(rel_path, &instance.value)?;
    }
    // Only override the placeholder `initialize_repository` assigned when the source
    // actually declared a root container — some in-memory test sources predate RFC-013
    // and carry no `manifest.container` at all, in which case the target's freshly
    // initialized default (which does satisfy the required-container invariant) should
    // stand rather than being clobbered to `None`. `manifest.container` is a sanctioned
    // write ([R1] — the manifest is authoritative for the inline root container);
    // `containerIndex` is retired (Change K) and is never written.
    if let Some(root_container) = &snapshot.root_container {
        manifest.container = Some(root_container.clone());
    }

    // Materialize source documents (RFC-017 R3/R12). RFC-038 [R25]: no
    // `sourceDocumentIndex` is written — the sidecar file itself is the
    // identity and the tombstone marker (an absent content file), so writing
    // the sidecar/content is the complete operation.
    if !snapshot.source_documents.is_empty() {
        let src_docs_base = snapshot
            .source_documents_path
            .as_deref()
            .unwrap_or("source-documents");
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
        }
        manifest.source_documents_path = Some(src_docs_base.to_string());
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
    if let Some(rev) = &snapshot.data_model_revision {
        manifest
            .extra
            .insert("dataModelRevision".to_string(), rev.clone());
    }

    target.save_manifest(&manifest)?;

    for container in &snapshot.containers {
        // Bulk materialization writes the container file directly. The
        // catalog-backed `create_container` would require the just-written
        // target tree to already be catalog-valid, defeating the #688
        // migration ramp: a legacy (pre-migration) archive must LOAD, with
        // `repo validate` reporting its defects afterwards — import shares
        // `repo validate`'s [R24] disposition, it does not enforce it.
        import_container_file(target, container)?;
    }

    if !snapshot.relations.is_empty() {
        // [R11], since the Phase-6 flip: relations import as one standalone
        // object per relation — a collection file would be denied by the
        // catalog on the next load of the target.
        //
        // Pass 1 — validate everything before the first write (no store can
        // roll a partial import back): every id canonical (it becomes a
        // filename) and unique ([R12] — a duplicate would silently last-win
        // as a file overwrite).
        let mut seen = std::collections::BTreeSet::new();
        for relation in &snapshot.relations {
            crate::store::require_canonical_relation_id(&relation.relation_id)?;
            if !seen.insert(relation.relation_id.as_str()) {
                return Err(RepositoryError::DuplicateRelationId {
                    relation_id: relation.relation_id.clone(),
                    locators: vec![format!(
                        "snapshot relations (relationId {} appears more than once)",
                        relation.relation_id
                    )],
                });
            }
        }
        // Pass 2 — write.
        target.ensure_relations_dir("relations")?;
        for relation in &snapshot.relations {
            target.save_relation(relation)?;
        }
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
        write_repo_json(
            target,
            &base_prefix,
            &path,
            field,
            Some(srs_schema::FIELD_SCHEMA_ID),
        )?;
        field_paths.push(path);
    }

    let mut type_paths = Vec::new();
    for record_type in &package.record_types {
        let path = format!(
            "types/{}-{}.json",
            slugify(&record_type.name),
            id_prefix(&record_type.id)?
        );
        write_repo_json(
            target,
            &base_prefix,
            &path,
            record_type,
            Some(srs_schema::TYPE_SCHEMA_ID),
        )?;
        type_paths.push(path);
    }

    let mut relation_type_paths = Vec::new();
    for relation_type in &package.relation_type_definitions {
        let path = format!(
            "relation-types/{}-{}.json",
            slugify(&relation_type.key),
            id_prefix(&relation_type.id)?
        );
        write_repo_json(
            target,
            &base_prefix,
            &path,
            relation_type,
            Some(srs_schema::RELATION_TYPE_SCHEMA_ID),
        )?;
        relation_type_paths.push(path);
    }

    let mut view_paths = Vec::new();
    for view in &package.views {
        let path = format!(
            "views/{}-{}.json",
            slugify(&view.name),
            id_prefix(&view.id)?
        );
        write_repo_json(
            target,
            &base_prefix,
            &path,
            view,
            Some(srs_schema::VIEW_SCHEMA_ID),
        )?;
        view_paths.push(path);
    }

    let mut doc_view_paths = Vec::new();
    for view in &package.document_views {
        let path = format!(
            "document-views/{}-{}.json",
            slugify(&view.name),
            id_prefix(&view.id)?
        );
        write_repo_json(
            target,
            &base_prefix,
            &path,
            view,
            Some(srs_schema::DOCUMENT_VIEW_SCHEMA_ID),
        )?;
        doc_view_paths.push(path);
    }

    let mut blueprint_paths = Vec::new();
    for blueprint in &package.blueprints {
        let path = format!(
            "blueprints/{}-{}.json",
            slugify(&blueprint.name),
            id_prefix(&blueprint.id)?
        );
        write_repo_json(
            target,
            &base_prefix,
            &path,
            blueprint,
            Some(srs_schema::BLUEPRINT_SCHEMA_ID),
        )?;
        blueprint_paths.push(path);
    }

    let mut theme_paths = Vec::new();
    for theme in &package.themes {
        let path = format!(
            "themes/{}-{}.json",
            slugify(&theme.name),
            id_prefix(&theme.id)?
        );
        write_repo_json(
            target,
            &base_prefix,
            &path,
            theme,
            Some(srs_schema::THEME_SCHEMA_ID),
        )?;
        theme_paths.push(path);
    }

    let mut vocabulary_paths = Vec::new();
    for vocab in &package.vocabularies {
        let path = format!(
            "vocabularies/{}-{}.json",
            slugify(&vocab.name),
            id_prefix(&vocab.id)?
        );
        write_repo_json(
            target,
            &base_prefix,
            &path,
            vocab,
            None, // vocabulary.json denies $schema (additionalProperties: false)
        )?;
        vocabulary_paths.push(path);
    }

    let mut lifecycle_paths = Vec::new();
    for lc in &package.lifecycles {
        let path = format!(
            "lifecycles/{}-{}.json",
            slugify(&lc.name),
            id_prefix(&lc.id)?
        );
        write_repo_json(
            target,
            &base_prefix,
            &path,
            lc,
            None, // lifecycle.json denies $schema (additionalProperties: false)
        )?;
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

/// Write one definition object under a package boundary, injecting `$schema`
/// when the serialised value does not already carry one.
///
/// RFC-038 [R7]/[R8]: every definition candidate under a reserved package
/// location is now classified by the catalog on every subsequent load —
/// unlike the pre-catalog reader, it will not tolerate a definition with no
/// declared `$schema` that also fails shape classification (e.g. `RecordType`
/// carries no `schema` field in `srs-core` at all, so a plain `to_value` never
/// produced one). `or_insert` preserves an already-present `$schema` (e.g. a
/// `Field` round-tripped from a real repo keeps the one it loaded with, per
/// its own doc comment) and only supplies the default for freshly-typed data.
fn write_repo_json<T: serde::Serialize>(
    target: &dyn RepositoryStore,
    base_prefix: &str,
    rel_path: &str,
    value: &T,
    schema_id: Option<&str>,
) -> Result<(), RepositoryError> {
    let full = format!("{base_prefix}/{rel_path}");
    if let Some((dir, _)) = full.rsplit_once('/') {
        ensure_repo_dir(target, dir)?;
    }
    let mut json = serde_json::to_value(value).map_err(|source| RepositoryError::Serialize {
        path: std::path::PathBuf::from(&full),
        source,
    })?;
    // Some definition kinds' schemas don't declare `$schema` as a property at
    // all (additionalProperties: false) — vocabulary.json and lifecycle.json
    // as of RFC-038's catalog validation. Injecting it there would make the
    // freshly-written file fail its own schema. `None` skips injection;
    // `or_insert` still preserves an already-present `$schema` otherwise.
    if let (Some(schema_id), serde_json::Value::Object(ref mut obj)) = (schema_id, &mut json) {
        obj.entry("$schema")
            .or_insert_with(|| serde_json::Value::String(schema_id.to_string()));
    }
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
    instance_id: String,
    from_path: String,
    to_path: String,
    value: serde_json::Value,
    sidecar_value: Option<serde_json::Value>,
}

/// Catalog-backed (RFC-038 [R1]): membership and locators come from one
/// catalog snapshot, never `manifest.instance_index`. There is no manifest
/// bookkeeping to update after a rename — the tree is the only authority.
fn collect_planned_renames(
    store: &dyn RepositoryStore,
) -> Result<Vec<PlannedRename>, RepositoryError> {
    let cat = store.catalog()?;
    // Load every instance first so canonical paths can be derived over the whole set at once
    // (srs-rust#696): id8-colliding siblings normalise to their full-id form — order-independent,
    // never a collision error — so path normalization stays applicable to valid repositories
    // with prefix-colliding UUIDs.
    let instances: Vec<SnapshotInstance> = cat
        .instances
        .iter()
        .map(|entry| {
            let locator = entry.locator.clone().unwrap_or_default();
            let value = store.load_instance_json(&locator)?;
            let r = crate::store::instance_ref_from_body(
                entry.id.clone(),
                entry.tier.unwrap_or(2),
                &value,
            );
            Ok(SnapshotInstance {
                instance_id: entry.id.clone(),
                tier: entry.tier.unwrap_or(2),
                title: r.title.map(serde_json::Value::String),
                tags: if r.tags.is_empty() {
                    None
                } else {
                    Some(r.tags)
                },
                value,
            })
        })
        .collect::<Result<_, RepositoryError>>()?;
    let canonical_paths = collision_safe_instance_paths(&instances, store)?;

    // After widening, two identical canonical paths can only mean a genuine duplicate instance
    // id (ADR-040) — a corrupt repository. Reject it rather than silently planning a rename that
    // would clobber one file with another in `upgrade_repository_paths`.
    let mut seen: HashSet<&str> = HashSet::with_capacity(canonical_paths.len());
    for (entry, canonical) in cat.instances.iter().zip(&canonical_paths) {
        if !seen.insert(canonical.as_str()) {
            return Err(RepositoryError::InvalidSnapshotData {
                message: format!(
                    "duplicate instance id '{}' — two catalog entries normalise to the same path '{canonical}'",
                    entry.id
                ),
            });
        }
    }

    let mut planned: Vec<PlannedRename> = Vec::new();
    for (entry, (instance, canonical)) in cat
        .instances
        .iter()
        .zip(instances.iter().zip(&canonical_paths))
    {
        let current_path = entry.locator.as_deref().unwrap_or_default();
        if current_path != canonical {
            let old_sidecar = sidecar_path_for(current_path);
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
                instance_id: entry.id.clone(),
                from_path: current_path.to_string(),
                to_path: canonical.clone(),
                value: instance.value.clone(),
                sidecar_value,
            });
        }
    }
    Ok(planned)
}

/// Returns `true` if any instance file path differs from its canonical
/// slug-id8 form (i.e. `upgrade_repository_paths` would rename at least one
/// file). Reads the catalog but performs no writes.
pub fn check_path_upgrade_needed(store: &dyn RepositoryStore) -> Result<bool, RepositoryError> {
    let planned = collect_planned_renames(store)?;
    Ok(!planned.is_empty())
}

pub fn upgrade_repository_paths(
    store: &dyn RepositoryStore,
) -> Result<UpgradeRepositoryPathsResult, RepositoryError> {
    let total_instances = store.catalog()?.instances.len();

    let planned = collect_planned_renames(store)?;

    if planned.is_empty() {
        return Ok(UpgradeRepositoryPathsResult {
            renames: vec![],
            already_canonical_count: total_instances,
            total_instances,
        });
    }

    // Phase 2: apply — write canonical instance files (and sidecars). No
    // manifest write follows: membership and locators come from the tree
    // ([R1]/[R22]) — there is no index entry to repoint.
    for rename in &planned {
        ensure_instance_parent(store, &rename.to_path)?;
        store.save_instance_json(&rename.to_path, &rename.value)?;
        if let Some(sidecar_value) = &rename.sidecar_value {
            let new_sidecar = sidecar_path_for(&rename.to_path);
            ensure_instance_parent(store, &new_sidecar)?;
            store.save_instance_json(&new_sidecar, sidecar_value)?;
        }
    }

    // Phase 3: cleanup — delete old files (best-effort; orphans are harmless per ADR-007)
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
        2 => instance
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
        2 => store.record_tier_dir(RecordTier::Tier2),
        // Tier 1 (TypedRecord) is retired (srs#448/rfc-decision-53635966,
        // srs-rust#888) — it falls into this same "unknown tier" refusal as
        // any other value: a snapshot import can no longer place Tier-1
        // content on disk at any revision.
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
/// Materialise one snapshot container as a file (or the root embed) without a
/// catalog read — import is a faithful bulk write, judged by `repo validate`
/// afterwards ([R24]'s reporting disposition; see call site in `do_import`).
/// Mirrors `RepositoryStore::save_container`'s placement rules: same id as the
/// root embed updates `manifest.container` ([R1], never a duplicate file —
/// [R12]); otherwise a collision-safe `containers/{slug}-{id8}.json` filename.
fn import_container_file(
    target: &dyn RepositoryStore,
    container: &srs_core::types::container::Container,
) -> Result<(), RepositoryError> {
    let id = &container.container_id;
    let mut manifest = target.load_manifest()?;
    if manifest
        .container
        .as_ref()
        .is_some_and(|c| &c.container_id == id)
    {
        manifest.container = Some(container.clone());
        return target.save_manifest(&manifest);
    }
    let val = serde_json::to_value(container).map_err(|source| RepositoryError::Serialize {
        path: std::path::PathBuf::from("containers"),
        source,
    })?;
    let slug = container
        .title
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    let prefix = &id[..id.len().min(8)];
    let mut filename = format!("containers/{slug}-{prefix}.json");
    if !matches!(target.load_instance_json(&filename), Err(ref e) if e.is_not_found()) {
        filename = format!("containers/{slug}-{id}.json");
    }
    target.ensure_instance_dir("containers")?;
    target.save_instance_json(&filename, &val)
}

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
    use crate::store::memory::MemoryStore;
    use crate::store::{FileStore, RepositoryStore};
    use crate::validation::validate_repository;
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
            schema: None,
            id: "short".to_string(),
            namespace: "com.semanticops.copy".to_string(),
            name: "bad".to_string(),
            version: 1,
            field_type: srs_core::types::field::FieldType::string(),
            description: "".to_string(),
            instructions: None,
            ai_guidance: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "".to_string(),
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
        // RFC-038 [R13]: the member note must be a real instance in the source
        // tree BEFORE export — a manifest.container referencing a nonexistent
        // member is a fatal dangling reference at catalog build, so the old
        // pattern of pushing the instance into the exported snapshot after the
        // fact no longer works.
        source
            .save_instance_json(
                "records/notes/n.json",
                &serde_json::json!({
                    "instanceId": "11111111-1111-4111-8111-111111111111",
                    "title": "n",
                    "sections": [{"name":"body","content":"hello"}]
                }),
            )
            .unwrap();
        let mut manifest = source.load_manifest().unwrap();
        manifest.container = Some(Container {
            container_id: "99999999-9999-4999-8999-999999999999".to_string(),
            title: "Root".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
            anchor_instance_id: None,
            member_instance_ids: Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]),
            root_instance_ids: Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]),
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        });
        source.save_manifest(&manifest).unwrap();

        let mut snapshot = export_repository_snapshot(&source).unwrap();
        snapshot.containers.push(Container {
            container_id: "99999999-9999-4999-8999-999999999999".to_string(),
            title: "Root".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
            anchor_instance_id: None,
            member_instance_ids: Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]),
            root_instance_ids: Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]),
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        });

        // Import into a .srsj FileStore bundle — this is the exact `srs repo copy
        // --to *.srsj` path used to regenerate single-file snapshots.
        let target = crate::tree_session::new_tree_session();
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
            schema: None,
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
        let target = crate::tree_session::new_tree_session();
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
            anchor_instance_id: None,
            member_instance_ids: Some(vec!["11111111-1111-4111-8111-111111111111".to_string()]),
            root_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        });
        // Second note so the relation can join two real instances — a containerId
        // must never be a relation endpoint (core invariant), and RFC-038 [R13]
        // now fatally rejects a relation endpoint that isn't in the instance set.
        snapshot.instances.push(SnapshotInstance {
            instance_id: "44444444-4444-4444-8444-444444444444".to_string(),
            tier: 0,
            title: Some(serde_json::Value::String("m".to_string())),
            tags: None,
            value: serde_json::json!({
                "instanceId": "44444444-4444-4444-8444-444444444444",
                "title": "m",
                "sections": []
            }),
        });
        snapshot.relations.push(Relation {
            relation_id: "33333333-3333-4333-8333-333333333333".to_string(),
            relation_type: "contains".to_string(),
            source_instance_id: "44444444-4444-4444-8444-444444444444".to_string(),
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

        assert_eq!(target.catalog().unwrap().instances.len(), 2);
        let summaries = list_containers(&target, &ContainerListFilter::default()).unwrap();
        // 2 = root container (embed-only, from manifest.container) + explicitly added container.
        assert_eq!(summaries.len(), 2);
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

        let json_store = crate::tree_session::new_tree_session();
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
                "title": "Round Trip",
                "sections": [{"name":"body","content":"round trip"}]
            }),
        });

        let json_store = crate::tree_session::new_tree_session();
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
    fn import_refuses_tier1_typed_record_instance() {
        // Tier 1 (TypedRecord) is retired (srs#448/rfc-decision-53635966,
        // srs-rust#888): a snapshot carrying a `tier: 1` instance — the old
        // shape had named fields but no type binding — can no longer be
        // placed on disk at any revision. `instance_path_with_id_fragment`
        // folds it into the same "unknown tier" refusal as any other
        // out-of-range value, so the whole import fails loudly rather than
        // silently writing to the retired `records/tier-1/` location.
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
        let err = import_repository_snapshot(&target, &snapshot).unwrap_err();
        assert!(
            err.to_string().contains("unknown tier 1"),
            "expected an unknown-tier refusal naming tier 1, got: {err}"
        );
        assert!(
            !temp.path().join("records/tier-1").exists(),
            "no records/tier-1 directory must be created for retired Tier-1 content"
        );
    }

    // export_fails_with_instance_load_error_when_record_missing retired by
    // RFC-038 Phase 3 (srs-rust#783): its scenario — a manifest index entry
    // pointing at a file with no data — cannot occur under the catalog model.
    // Membership comes from the tree ([R1]); an instance is only ever
    // discovered if its file exists, so there is no more "ghost" index entry
    // to inject. `InstanceLoad`'s error-context contract is still exercised
    // by `catalog_require_instance_locator`'s callers elsewhere.

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
                "instanceId": "aaaaaaaa-0000-4000-8000-000000000001",
                "title": "same title",
                "sections": []
            }),
        });
        snapshot.instances.push(SnapshotInstance {
            instance_id: "aaaaaaaa-0000-4000-8000-000000000002".to_string(),
            tier: 0,
            title: Some(serde_json::json!("same title")),
            tags: None,
            value: serde_json::json!({
                "instanceId": "aaaaaaaa-0000-4000-8000-000000000002",
                "title": "same title",
                "sections": []
            }),
        });

        let target = MemoryStore::uninitialized();
        import_repository_snapshot(&target, &snapshot)
            .expect("prefix-colliding instances must import, not error (srs-rust#696)");

        let cat = target.catalog().unwrap();
        let path_of = |id: &str| -> String {
            cat.instances
                .iter()
                .find(|e| e.id == id)
                .unwrap_or_else(|| panic!("instance {id} missing from catalog"))
                .locator
                .clone()
                .unwrap_or_default()
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
                "instanceId": format!("aaaaaaaa-0000-4000-8000-00000000000{suffix}"),
                "title": "same title",
                "sections": []
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
            .catalog()
            .unwrap()
            .instances
            .iter()
            .filter(|e| e.id.starts_with("aaaaaaaa-0000-4000-8000-0000000000"))
            .map(|e| e.locator.clone().unwrap_or_default())
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
    fn srsj_partial_import_is_never_projected_to_the_file() {
        // Two valid instances reach the session tree before the 3rd fails. The
        // `.srsj` file must never carry that partial state: a session projects
        // only on success, which is exactly what `with_store` does in the CLI.
        let tmp = TempDir::new().unwrap();
        let srsj_path = tmp.path().join("target.srsj");
        let session = crate::srsj::SrsjSession::create(&srsj_path).unwrap();
        let target = session.store();

        let mut snapshot = {
            let source = MemoryStore::uninitialized();
            source.initialize_repository(&make_input()).unwrap();
            export_repository_snapshot(&source).unwrap()
        };

        // Instance 3 has an unknown tier, which cannot be mapped to a storage path and
        // triggers InvalidSnapshotData after instances 1 and 2 have been saved in the
        // session tree. (A plain id8 path collision is no longer an error — see
        // srs-rust#696 — so this uses a genuine per-instance failure.)
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

        let result = import_repository_snapshot(target, &snapshot);
        assert!(
            matches!(result, Err(RepositoryError::InvalidSnapshotData { .. })),
            "expected InvalidSnapshotData from unknown tier, got: {result:?}"
        );

        // The failure short-circuits before any flush — exactly `with_store`'s
        // `f(...)?; session.flush()?` ordering — so the file never appears.
        assert!(
            !srsj_path.exists(),
            "a failed import must never be projected to the .srsj file"
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

    /// A minimal record.json (Tier 2)-shaped body. `type_name` feeds both the
    /// (schema-unconstrained) `typeName` string and the canonical-path slug
    /// derivation in `instance_path_with_id_fragment`.
    fn tier2_record_value(id: &str, type_name: &str) -> serde_json::Value {
        serde_json::json!({
            "instanceId": id,
            "typeId": "00000000-0000-4000-8000-0000000000ab",
            "typeVersion": 1,
            "typeNamespace": "com.example",
            "typeName": type_name,
            "fieldValues": {}
        })
    }

    /// A minimal note.json (Tier 0)-shaped body.
    fn note_value(id: &str, title: &str) -> serde_json::Value {
        serde_json::json!({
            "instanceId": id,
            "title": title,
            "sections": []
        })
    }

    /// Writes the instance file only — membership comes from the tree ([R1]);
    /// `_tier` is unused (the file's own shape/content determines its tier
    /// once discovered via the catalog), kept for call-site clarity.
    fn inject_non_canonical_instance(
        store: &dyn RepositoryStore,
        _instance_id: &str,
        _tier: u8,
        path: &str,
        value: serde_json::Value,
    ) {
        store
            .ensure_instance_dir(path.rsplit_once('/').map(|(d, _)| d).unwrap_or("records"))
            .unwrap();
        store.save_instance_json(path, &value).unwrap();
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
        let value = tier2_record_value(id, "com.example/my-type");
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

        // Discoverable at the canonical path via the catalog (no manifest write).
        let cat = store.catalog().unwrap();
        assert_eq!(
            cat.instances[0].locator.as_deref(),
            Some("records/tier-2/com-example-my-type-aabbccdd.json")
        );
    }

    #[test]
    fn upgrade_renames_non_canonical_note_path() {
        let store = MemoryStore::uninitialized();
        store.initialize_repository(&make_upgrade_input()).unwrap();

        let id = "11223344-0000-4000-8000-000000000000";
        // Title lives directly in the body — slug derivation is catalog/body-backed now,
        // no manifest-index title patch needed.
        let value = note_value(id, "My Note");
        inject_non_canonical_instance(&store, id, 0, "records/notes/raw-note.json", value.clone());

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
        let value = tier2_record_value(id, "com.example/my-type");
        inject_non_canonical_instance(&store, id, 2, "records/tier-2/old-name.json", value);

        let first = upgrade_repository_paths(&store).unwrap();
        assert_eq!(first.renames.len(), 1);

        let second = upgrade_repository_paths(&store).unwrap();
        assert_eq!(second.renames.len(), 0, "second run should be a no-op");
        assert_eq!(second.total_instances, 1);
    }

    #[test]
    fn upgrade_widens_id8_colliding_instances() {
        // srs-rust#696: repo-upgrade must normalise a repository whose deterministic UUIDs collide
        // in their first 8 hex chars WITHOUT erroring — both siblings widen to their full-id form,
        // land on distinct canonical files, and a second pass is a no-op (order-independent,
        // ADR-040). Exercises `collect_planned_renames`/`upgrade_repository_paths` directly, the
        // path the `repo-upgrade` migration and `srs repo upgrade` run.
        let store = MemoryStore::uninitialized();
        store.initialize_repository(&make_upgrade_input()).unwrap();

        let id1 = "00000000-0000-4000-8000-000000005801";
        let id2 = "00000000-0000-4000-8000-000000005802";
        inject_non_canonical_instance(
            &store,
            id1,
            2,
            &format!("records/tier-2/{id1}.json"),
            tier2_record_value(id1, "com.example/decision"),
        );
        inject_non_canonical_instance(
            &store,
            id2,
            2,
            &format!("records/tier-2/{id2}.json"),
            tier2_record_value(id2, "com.example/decision"),
        );

        let result =
            upgrade_repository_paths(&store).expect("upgrade must not fail on an id8 collision");
        assert_eq!(result.renames.len(), 2, "both colliding instances renamed");

        let cat = store.catalog().unwrap();
        let path_of = |id: &str| -> String {
            cat.instances
                .iter()
                .find(|e| e.id == id)
                .unwrap()
                .locator
                .clone()
                .unwrap_or_default()
        };
        let p1 = path_of(id1);
        let p2 = path_of(id2);
        // Both widened to their full id (order-independent), on distinct files that exist.
        assert!(p1.ends_with(&format!("-{id1}.json")), "id1 widened: {p1}");
        assert!(p2.ends_with(&format!("-{id2}.json")), "id2 widened: {p2}");
        assert_ne!(p1, p2);
        assert!(store.load_instance_json(&p1).is_ok());
        assert!(store.load_instance_json(&p2).is_ok());

        let second = upgrade_repository_paths(&store).unwrap();
        assert_eq!(
            second.renames.len(),
            0,
            "second upgrade pass must be a no-op (idempotent, clobber-free)"
        );
    }

    #[test]
    fn upgrade_rejects_duplicate_instance_id() {
        // A corrupt manifest with two index entries sharing an instance id must be rejected by the
        // rename planner (ADR-040) rather than silently clobbering one file with the other during
        // `upgrade_repository_paths`.
        let store = MemoryStore::uninitialized();
        store.initialize_repository(&make_upgrade_input()).unwrap();

        let id = "dddddddd-0000-4000-8000-000000000009";
        let value = tier2_record_value(id, "com.example/decision");
        inject_non_canonical_instance(&store, id, 2, "records/tier-2/a.json", value.clone());
        inject_non_canonical_instance(&store, id, 2, "records/tier-2/b.json", value);

        let err = upgrade_repository_paths(&store).unwrap_err();
        // RFC-038: the duplicate is now caught upstream by the catalog's [R12]
        // duplicate-id check (fatal CatalogLoad) before upgrade's own
        // InvalidSnapshotData guard runs — still a hard rejection, earlier.
        assert!(
            matches!(err, RepositoryError::CatalogLoad { .. }),
            "duplicate instance id must be rejected, got: {err:?}"
        );
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

        // Inject a non-canonical tier-2 file directly to disk via the store.
        // (Tier 1 / TypedRecord is retired — srs-rust#888 — so a
        // slug-bearing fixture must be Tier 2.)
        let id = "ddccbbaa-1234-5678-90ab-cdef01234567";
        let value = tier2_record_value(id, "com.example/section");
        inject_non_canonical_instance(&store, id, 2, "records/tier-2/old-section.json", value);

        let canonical_path = "records/tier-2/com-example-section-ddccbbaa.json";

        let result = upgrade_repository_paths(&store).unwrap();
        assert_eq!(result.renames.len(), 1);
        assert_eq!(result.renames[0].to_path, canonical_path);

        // Verify filesystem state.
        assert!(
            !temp.path().join("records/tier-2/old-section.json").exists(),
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

        let value = tier2_record_value(id, "com.example/my-type");
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

    /// RFC-038 [R25]: source documents resolve via sidecar discovery — the
    /// catalog's source-document set — not `manifest.source_document_index`
    /// (retired, Change K). This only needs to confirm the (already-default)
    /// `sourceDocumentsPath`; the sidecar file itself is the real fixture.
    fn make_source_doc_manifest(store: &dyn RepositoryStore) {
        let mut manifest = store.load_manifest().unwrap();
        manifest.source_documents_path = Some("source-documents".to_string());
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

        // Discoverable via the catalog's source-document set (RFC-038 [R25]) —
        // no `sourceDocumentIndex` is written any more.
        let cat = target.catalog().unwrap();
        assert_eq!(cat.source_documents.len(), 1);
        assert_eq!(
            cat.source_documents[0].id,
            "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
        );
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
    fn copy_preserves_source_doc_title_from_sidecar_body() {
        // RFC-038 [R25]: title comes from the sidecar body itself, not from a
        // `sourceDocumentIndex` side-channel (retired, Change K — checksums
        // were only ever carried there and are not reconstructed).
        let source = MemoryStore::uninitialized();
        source.initialize_repository(&make_input()).unwrap();
        let sidecar_with_title = r#"{
            "documentId": "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
            "contentPath": "my-doc.pdf",
            "contentType": "application/pdf",
            "title": "My Test Doc",
            "createdAt": "2026-01-01T00:00:00Z"
        }"#;
        source
            .save_text_file("source-documents/my-doc.meta.json", sidecar_with_title)
            .unwrap();
        source
            .save_binary_file("source-documents/my-doc.pdf", b"pdf bytes")
            .unwrap();

        let temp = TempDir::new().unwrap();
        let target = FileStore::new(temp.path());
        copy_repository(&source, &target).unwrap();

        let cat = target.catalog().unwrap();
        assert_eq!(cat.source_documents.len(), 1);
        assert_eq!(
            cat.source_documents[0].id,
            "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
        );
        let sidecar_str = target
            .load_text_file("source-documents/my-doc.meta.json")
            .unwrap();
        let sidecar: serde_json::Value = serde_json::from_str(&sidecar_str).unwrap();
        assert_eq!(sidecar["title"], "My Test Doc");
    }
}

#[cfg(test)]
mod rfc038_import_guard_tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use crate::store::RepositoryStore;

    fn snapshot_with_relations(
        relations: Vec<srs_core::types::relation::Relation>,
    ) -> RepositorySnapshot {
        let source = MemoryStore::empty();
        let mut snapshot = export_repository_snapshot(&source).expect("empty snapshot builds");
        snapshot.relations = relations;
        snapshot
    }

    fn relation(id: &str) -> srs_core::types::relation::Relation {
        serde_json::from_value(serde_json::json!({
            "relationId": id,
            "relationType": "contains",
            "sourceInstanceId": "aaaaaaaa-0000-4000-8000-000000000001",
            "targetInstanceId": "aaaaaaaa-0000-4000-8000-000000000002",
            "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }

    /// Import pass 1 (validate before any write): a non-canonical relationId
    /// fails the whole import before a single relation file exists.
    #[test]
    fn import_refuses_a_non_canonical_relation_id_before_writing() {
        let snapshot = snapshot_with_relations(vec![relation("r1")]);
        let target = MemoryStore::uninitialized();
        let err = import_repository_snapshot(&target, &snapshot).unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidRelationId { ref relation_id } if relation_id == "r1"),
            "got {err:?}"
        );
        assert!(
            target.load_instance_json("relations/r1.json").is_err(),
            "nothing may be written for a refused import"
        );
    }

    /// Import pass 1: a duplicate relationId fails loudly instead of silently
    /// last-winning as a file overwrite ([R12]).
    #[test]
    fn import_refuses_duplicate_relation_ids() {
        let dup = "eeeeeeee-0000-4000-8000-00000000dead";
        let snapshot = snapshot_with_relations(vec![relation(dup), relation(dup)]);
        let target = MemoryStore::uninitialized();
        let err = import_repository_snapshot(&target, &snapshot).unwrap_err();
        assert!(
            matches!(err, RepositoryError::DuplicateRelationId { ref relation_id, .. } if relation_id == dup),
            "got {err:?}"
        );
    }
}
