use crate::error::RepositoryError;
use crate::repository_lifecycle::RepositoryMetadata;
use crate::repository_portability::{
    export_repository_snapshot, import_repository_snapshot, PackageBoundarySnapshot,
    RepositorySnapshot, SnapshotInstance,
};
use crate::store::RepositoryStore;
use srs_core::types::container::{Container, ContainerIndexEntry};
use srs_core::types::relation::Relation;
use std::collections::HashMap;
use std::io::{Read, Seek, Write};
use zip::write::SimpleFileOptions;

/// Pack a repository into a deterministic `.srs` ZIP archive (SRSzip, ADR-033).
///
/// The archive is byte-identical across calls on the same repository state:
/// entries are sorted lexicographically, timestamps are zeroed, and all fields
/// that could introduce non-determinism (host metadata, etc.) are suppressed.
/// HashMap keys are ordered through `serde_json::to_value` — requires that
/// `preserve_order` remains disabled in the `serde_json` dependency (ADR-017).
///
/// # Arguments
/// * `source` — the repository store to pack
/// * `writer` — any `Write + Seek` target (file, `Cursor<Vec<u8>>`, etc.)
pub fn archive_pack(
    source: &dyn RepositoryStore,
    writer: impl Write + Seek,
) -> Result<(), RepositoryError> {
    let snapshot = export_repository_snapshot(source)?;
    let manifest = source.load_manifest()?;

    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();

    // manifest.json — route through to_value() so serde_json::Map (BTreeMap-backed) sorts all
    // HashMap<String,Value> fields, making the archive byte-stable across process runs.
    // See ADR-017: preserve_order must remain disabled or this guarantee breaks.
    let manifest_value =
        serde_json::to_value(&manifest).map_err(|e| RepositoryError::InvalidSnapshotData {
            message: e.to_string(),
        })?;
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest_value).map_err(|e| {
            RepositoryError::InvalidSnapshotData {
                message: e.to_string(),
            }
        })?;
    entries.push(("manifest.json".to_string(), manifest_bytes));

    // package/package.json
    let pkg_json = source.load_package_json()?;
    let pkg_json_sorted =
        serde_json::to_value(pkg_json).map_err(|e| RepositoryError::InvalidSnapshotData {
            message: e.to_string(),
        })?;
    let pkg_bytes = serde_json::to_vec_pretty(&pkg_json_sorted).map_err(|e| {
        RepositoryError::InvalidSnapshotData {
            message: e.to_string(),
        }
    })?;
    entries.push(("package/package.json".to_string(), pkg_bytes));

    // package/package.snapshot.json — primary package boundary snapshot
    if let Some(pkg) = snapshot.packages.iter().find(|p| p.boundary_path.is_none()) {
        let pkg_snap_value =
            serde_json::to_value(pkg).map_err(|e| RepositoryError::InvalidSnapshotData {
                message: e.to_string(),
            })?;
        let pkg_snap_bytes =
            serde_json::to_vec_pretty(&pkg_snap_value).map_err(|e| {
                RepositoryError::InvalidSnapshotData {
                    message: e.to_string(),
                }
            })?;
        entries.push(("package/package.snapshot.json".to_string(), pkg_snap_bytes));
    }

    // relations/relations-collection.json (only when relations exist)
    if !snapshot.relations.is_empty() {
        let relations_val = serde_json::to_value(serde_json::json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
            "relations": snapshot.relations
        }))
        .map_err(|e| RepositoryError::InvalidSnapshotData {
            message: e.to_string(),
        })?;
        let relations_bytes =
            serde_json::to_vec_pretty(&relations_val).map_err(|e| {
                RepositoryError::InvalidSnapshotData {
                    message: e.to_string(),
                }
            })?;
        entries.push((
            "relations/relations-collection.json".to_string(),
            relations_bytes,
        ));
    }

    // instance files — load and re-serialize for deterministic key ordering
    for entry in &manifest.instance_index {
        let value = source.load_instance_json(entry.path())?;
        let sorted_value =
            serde_json::to_value(value).map_err(|e| RepositoryError::InvalidSnapshotData {
                message: e.to_string(),
            })?;
        let bytes =
            serde_json::to_vec_pretty(&sorted_value).map_err(|e| {
                RepositoryError::InvalidSnapshotData {
                    message: e.to_string(),
                }
            })?;
        entries.push((entry.path().to_string(), bytes));
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut zip = zip::ZipWriter::new(writer);
    for (path, bytes) in &entries {
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        zip.start_file(path, options)
            .map_err(|e| RepositoryError::InvalidArchive {
                message: e.to_string(),
            })?;
        zip.write_all(bytes)
            .map_err(|e| RepositoryError::InvalidArchive {
                message: e.to_string(),
            })?;
    }
    let _ = zip.finish().map_err(|e| RepositoryError::InvalidArchive {
        message: e.to_string(),
    })?;

    Ok(())
}

/// Unpack a `.srs` ZIP archive into a repository store (SRSzip, ADR-033).
///
/// The target store must be empty (no existing repository). A new repository
/// is created at the target with the same identity and content as the source.
pub fn archive_unpack(
    reader: impl Read + Seek,
    target: &dyn RepositoryStore,
) -> Result<(), RepositoryError> {
    let mut zip =
        zip::ZipArchive::new(reader).map_err(|e| RepositoryError::InvalidArchive {
            message: e.to_string(),
        })?;

    let file_count = zip.len();
    let mut bytes_map: HashMap<String, Vec<u8>> = HashMap::with_capacity(file_count);
    for i in 0..file_count {
        let mut entry = zip.by_index(i).map_err(|e| RepositoryError::InvalidArchive {
            message: e.to_string(),
        })?;
        if entry.name().ends_with('/') {
            continue;
        }
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| RepositoryError::InvalidArchive {
                message: e.to_string(),
            })?;
        bytes_map.insert(name, buf);
    }

    let manifest_bytes = bytes_map
        .get("manifest.json")
        .ok_or_else(|| RepositoryError::InvalidArchive {
            message: "missing manifest.json".to_string(),
        })?;
    let mut manifest_val: serde_json::Value =
        serde_json::from_slice(manifest_bytes).map_err(|e| RepositoryError::InvalidArchive {
            message: e.to_string(),
        })?;
    crate::manifest::migrate_upstream_package(&mut manifest_val);

    let repo_meta = RepositoryMetadata {
        repository_id: manifest_val
            .get("repositoryId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        namespace: manifest_val
            .get("namespace")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        srs_version: manifest_val
            .get("srsVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("2.0-draft")
            .to_string(),
        title: manifest_val
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        description: manifest_val
            .get("description")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    };

    let declared_extensions: Vec<String> = manifest_val
        .get("declaredExtensions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let instance_index = manifest_val
        .get("instanceIndex")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let pkg_bytes = bytes_map
        .get("package/package.snapshot.json")
        .ok_or_else(|| RepositoryError::InvalidArchive {
            message: "missing package/package.snapshot.json".to_string(),
        })?;
    let primary_pkg: PackageBoundarySnapshot =
        serde_json::from_slice(pkg_bytes).map_err(|e| RepositoryError::InvalidArchive {
            message: e.to_string(),
        })?;

    let mut instances = Vec::with_capacity(instance_index.len());
    for entry in &instance_index {
        let instance_id = entry
            .get("instanceId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tier: u8 = entry.get("tier").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
        let path = entry
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let title = entry.get("title").cloned();
        let tags: Option<Vec<String>> =
            entry.get("tags").and_then(|v| v.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            });

        let inst_bytes =
            bytes_map
                .get(&path)
                .ok_or_else(|| RepositoryError::InvalidArchive {
                    message: format!(
                        "instance '{}' referenced in instanceIndex not found at '{}'",
                        instance_id, path
                    ),
                })?;
        let value: serde_json::Value =
            serde_json::from_slice(inst_bytes).map_err(|e| RepositoryError::InvalidArchive {
                message: e.to_string(),
            })?;

        instances.push(SnapshotInstance {
            instance_id,
            tier,
            title,
            tags,
            value,
        });
    }

    let relations: Vec<Relation> =
        if let Some(rel_bytes) = bytes_map.get("relations/relations-collection.json") {
            let val: serde_json::Value = serde_json::from_slice(rel_bytes).map_err(|e| {
                RepositoryError::InvalidArchive {
                    message: e.to_string(),
                }
            })?;
            let arr = val
                .get("relations")
                .cloned()
                .unwrap_or(serde_json::Value::Array(vec![]));
            serde_json::from_value(arr).map_err(|e| RepositoryError::InvalidArchive {
                message: e.to_string(),
            })?
        } else {
            Vec::new()
        };

    let root_container: Option<Container> = manifest_val
        .get("container")
        .and_then(|v| serde_json::from_value::<Container>(v.clone()).ok());

    let container_index: Option<Vec<ContainerIndexEntry>> = manifest_val
        .get("containerIndex")
        .and_then(|v| serde_json::from_value::<Vec<ContainerIndexEntry>>(v.clone()).ok());

    let snapshot = RepositorySnapshot {
        repository: repo_meta,
        declared_extensions,
        packages: vec![primary_pkg],
        instances,
        containers: Vec::new(),
        root_container,
        container_index,
        relations,
    };

    import_repository_snapshot(target, &snapshot)?;

    Ok(())
}

/// Pack a repository into a `.srs` binary archive and return the bytes.
///
/// Convenience wrapper over [`archive_pack`] for callers that need an in-memory byte buffer
/// (e.g. WASM bindings). Equivalent to calling `archive_pack` with a `Cursor<Vec<u8>>` and
/// extracting the inner `Vec` — provided so binding layers stay thin (ADR-010, ADR-033).
pub fn archive_to_vec(source: &dyn RepositoryStore) -> Result<Vec<u8>, RepositoryError> {
    let mut buf = std::io::Cursor::new(Vec::new());
    archive_pack(source, &mut buf)?;
    Ok(buf.into_inner())
}
