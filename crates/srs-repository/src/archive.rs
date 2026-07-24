//! `.srs` archive pack/unpack — a deterministic ZIP of the exploded tree
//! (ADR-039; determinism requirements inherited from ADR-033).
//!
//! Pack writes the repository's file tree verbatim: tree-backed sessions dump
//! their `MemVfs` snapshot; other stores are enumerated from the manifest and
//! every package boundary (per-definition files included — the pre-ADR-039
//! pack omitted them). `package/package.snapshot.json` is never written.
//!
//! Unpack prefers the native tree layout (layout-faithful, no
//! re-canonicalization). Archives carrying `package/package.snapshot.json`
//! without per-definition files take the legacy snapshot-import path — a
//! migration ramp only, kept so pre-ADR-039 archives can be opened in order
//! to be re-saved in the new format. Remove after ecosystem archives are
//! migrated (#688).

use crate::error::RepositoryError;
use crate::repository_lifecycle::RepositoryMetadata;
use crate::repository_portability::{
    export_repository_snapshot_with_options, import_repository_snapshot, ExportSnapshotOptions,
    PackageBoundarySnapshot, RepositorySnapshot, SnapshotInstance, SourceDocumentSnapshot,
};
use crate::store::{FileStore, RepositoryStore};
use crate::tree_session::open_tree;
use crate::vfs::{vfs_join, SRS_MARKER_DIR};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use srs_core::types::container::{Container, ContainerIndexEntry};
use srs_core::types::relation::Relation;
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, Write};
use zip::write::SimpleFileOptions;

/// `package.json` array keys naming per-definition files (all definition kinds).
const DEFINITION_KEYS: [&str; 10] = [
    "fields",
    "types",
    "relationTypes",
    "views",
    "documentViews",
    "themes",
    "blueprints",
    "protocols",
    "vocabularies",
    "lifecycles",
];

/// Enumerate the repository as a path→bytes tree.
///
/// Tree-backed sessions return their snapshot verbatim (unknown files — README,
/// CI config — ride along: the archive is a faithful snapshot of the session
/// tree). Other stores are enumerated from the manifest: raw manifest, every
/// package boundary's `package.json` plus every per-definition file it
/// references, relations (real filename preserved), instanceIndex files,
/// containerIndex files, and the whole `source-documents/` subtree.
/// Deliberately not a blind directory sweep, so a git working tree's `.git/`
/// is never archived from disk.
///
/// This is the single authoritative faithful store→tree enumeration, reused by both
/// `archive_pack` (ADR-039) and `tree_session::materialize_tree` (ADR-040), so a `.srsj`
/// load reproduces the source's real paths instead of re-canonicalizing them (srs-rust#696).
pub(crate) fn tree_entries(
    source: &dyn RepositoryStore,
) -> Result<BTreeMap<String, Vec<u8>>, RepositoryError> {
    if let Some(map) = source.as_tree_snapshot() {
        return Ok(map);
    }

    let manifest = source.load_manifest()?;
    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();

    entries.insert(
        "manifest.json".to_string(),
        source.load_manifest_raw_text()?.into_bytes(),
    );

    // Primary package via the raw-text trait method (works on every store),
    // then any sub-package boundaries by their manifest packageRefs path.
    let mut package_prefixes: Vec<(String, String)> = vec![(
        "package".to_string(),
        source.load_primary_package_raw_text()?,
    )];
    for boundary in source.list_package_boundaries()? {
        if let Some(prefix) = boundary.selector.clone() {
            let pkg_rel = vfs_join(&prefix, "package.json");
            package_prefixes.push((prefix, source.load_text_file(&pkg_rel)?));
        }
    }
    for (prefix, pkg_text) in package_prefixes {
        let pkg_rel = vfs_join(&prefix, "package.json");
        let pkg_val: serde_json::Value =
            serde_json::from_str(&pkg_text).map_err(|e| RepositoryError::InvalidArchive {
                message: format!("invalid {pkg_rel}: {e}"),
            })?;
        entries.insert(pkg_rel.clone(), pkg_text.into_bytes());

        for key in DEFINITION_KEYS {
            let Some(arr) = pkg_val.get(key).and_then(|v| v.as_array()) else {
                continue;
            };
            for rel in arr.iter().filter_map(|v| v.as_str()) {
                let full = vfs_join(&prefix, rel);
                // A dangling reference is a hard error naming the missing
                // path — never a silent skip (ADR-039).
                let text = source.load_text_file(&full).map_err(|e| {
                    if e.is_not_found() {
                        RepositoryError::InvalidArchive {
                            message: format!(
                                "package file missing: {full} (referenced by {pkg_rel})"
                            ),
                        }
                    } else {
                        e
                    }
                })?;
                entries.insert(full, text.into_bytes());
            }
        }
    }

    // Preserve the real relations filename (canonical first, then legacy).
    for name in [
        "relations/relations-collection.json",
        "relations/relations.json",
    ] {
        match source.load_text_file(name) {
            Ok(text) => {
                entries.insert(name.to_string(), text.into_bytes());
                break;
            }
            Err(e) if e.is_not_found() => {}
            Err(e) => return Err(e),
        }
    }

    // Instance files at their real storage paths (may predate canonicalization).
    for entry in &manifest.instance_index {
        let content = source.load_text_file(&entry.path)?;
        entries.insert(entry.path.clone(), content.into_bytes());
    }

    // Container JSON files — referenced by containerIndex paths, not instanceIndex.
    if let Some(index) = &manifest.container_index {
        for entry in index {
            if let Some(path) = &entry.path {
                let content = source.load_text_file(path)?;
                entries.insert(path.clone(), content.into_bytes());
            }
        }
    }

    // The whole source-documents subtree (sidecars + binary content).
    let src_docs_dir = manifest
        .source_documents_path
        .as_deref()
        .unwrap_or("source-documents");
    for path in source.list_files_recursive(src_docs_dir) {
        let bytes = match source.load_binary_file(&path) {
            Ok(b) => b,
            // Stores that keep text and binary content separately (MemoryStore,
            // JsonStore) serve sidecars through the text path.
            Err(e) if e.is_not_found() => source.load_text_file(&path)?.into_bytes(),
            Err(e) => return Err(e),
        };
        entries.insert(path, bytes);
    }

    Ok(entries)
}

pub fn archive_pack(
    source: &dyn RepositoryStore,
    writer: impl Write + Seek,
) -> Result<(), RepositoryError> {
    let entries = tree_entries(source)?;

    // BTreeMap iteration is already lexicographic — the ADR-033 entry-order
    // and determinism guarantees hold by construction.
    let mut zip = zip::ZipWriter::new(writer);
    for (path, bytes) in &entries {
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        zip.start_file(path, options)?;
        zip.write_all(bytes)
            .map_err(|e| RepositoryError::InvalidArchive {
                message: e.to_string(),
            })?;
    }
    let _ = zip.finish()?;

    Ok(())
}

/// Read a ZIP into a path→bytes map (directory entries skipped).
fn read_zip_to_map(reader: impl Read + Seek) -> Result<HashMap<String, Vec<u8>>, RepositoryError> {
    let mut zip = zip::ZipArchive::new(reader).map_err(|e| RepositoryError::InvalidArchive {
        message: e.to_string(),
    })?;
    let file_count = zip.len();
    let mut bytes_map: HashMap<String, Vec<u8>> = HashMap::with_capacity(file_count);
    for i in 0..file_count {
        let mut entry = zip.by_index(i)?;
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
    Ok(bytes_map)
}

/// Parse and RFC-014-migrate the archive manifest.
fn parse_manifest_value(
    bytes_map: &HashMap<String, Vec<u8>>,
) -> Result<serde_json::Value, RepositoryError> {
    let manifest_bytes =
        bytes_map
            .get("manifest.json")
            .ok_or_else(|| RepositoryError::InvalidArchive {
                message: "missing manifest.json".to_string(),
            })?;
    let mut manifest_val: serde_json::Value =
        serde_json::from_slice(manifest_bytes).map_err(|e| RepositoryError::InvalidArchive {
            message: e.to_string(),
        })?;
    crate::manifest::migrate_upstream_package(&mut manifest_val);
    Ok(manifest_val)
}

/// Referenced definition files (primary package and every local sub-package
/// boundary in the manifest's `packageRefs`) absent from the map.
/// Empty ⇒ the archive is loadable as a native tree.
fn missing_tree_definitions(
    bytes_map: &HashMap<String, Vec<u8>>,
    manifest_val: &serde_json::Value,
) -> Vec<String> {
    let mut prefixes = vec!["package".to_string()];
    if let Some(refs) = manifest_val.get("packageRefs").and_then(|v| v.as_array()) {
        for pkg_ref in refs {
            if pkg_ref.get("mode").and_then(|m| m.as_str()) == Some("local") {
                if let Some(path) = pkg_ref.get("path").and_then(|p| p.as_str()) {
                    prefixes.push(path.to_string());
                }
            }
        }
    }

    let mut missing = Vec::new();
    for prefix in prefixes {
        let pkg_rel = vfs_join(&prefix, "package.json");
        let Some(pkg_bytes) = bytes_map.get(&pkg_rel) else {
            missing.push(pkg_rel);
            continue;
        };
        let Ok(pkg_val) = serde_json::from_slice::<serde_json::Value>(pkg_bytes) else {
            missing.push(pkg_rel);
            continue;
        };
        for key in DEFINITION_KEYS {
            let Some(arr) = pkg_val.get(key).and_then(|v| v.as_array()) else {
                continue;
            };
            for rel in arr.iter().filter_map(|v| v.as_str()) {
                let full = vfs_join(&prefix, rel);
                if !bytes_map.contains_key(&full) {
                    missing.push(full);
                }
            }
        }
    }
    missing
}

/// Unpack a `.srs` ZIP archive into a repository store and return the repository ID.
///
/// Native tree archives are written layout-faithfully: verbatim raw files into
/// file-tree stores (`FileStore`), or via a tree session + snapshot import for
/// other store kinds. Legacy snapshot archives take the migration ramp.
pub fn archive_unpack(
    reader: impl Read + Seek,
    target: &dyn RepositoryStore,
) -> Result<String, RepositoryError> {
    let mut bytes_map = read_zip_to_map(reader)?;
    let manifest_val = parse_manifest_value(&bytes_map)?;
    let repository_id = manifest_val
        .get("repositoryId")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let missing = missing_tree_definitions(&bytes_map, &manifest_val);
    if missing.is_empty() {
        // Native tree archive. A stray legacy snapshot (old archive of an
        // empty package) is metadata, not tree content — drop it.
        bytes_map.remove("package/package.snapshot.json");
        let tree: BTreeMap<String, Vec<u8>> = bytes_map.into_iter().collect();
        if target.is_file_tree_store() {
            // Same emptiness contract as the snapshot import path: never
            // write into a target that already holds content.
            crate::repository_portability::ensure_target_empty(target)?;
            // Layout-faithful: raw files, no re-canonicalization.
            let marker_prefix = format!("{SRS_MARKER_DIR}/");
            let has_marker = tree.keys().any(|k| k.starts_with(&marker_prefix));
            for (path, bytes) in &tree {
                target.save_binary_file(path, bytes)?;
            }
            if !has_marker {
                target.save_binary_file(&format!("{SRS_MARKER_DIR}/.gitkeep"), &[])?;
            }
        } else {
            let session = open_tree(tree)?;
            let snapshot = export_repository_snapshot_with_options(
                &session,
                ExportSnapshotOptions {
                    include_content_blobs: true,
                },
            )?;
            import_repository_snapshot(target, &snapshot)?;
        }
        return Ok(repository_id);
    }

    if bytes_map.contains_key("package/package.snapshot.json") {
        // Legacy migration ramp — remove after ecosystem archives are migrated (#688).
        let snapshot = legacy_snapshot_from_map(&bytes_map, &manifest_val)?;
        let repository_id = snapshot.repository.repository_id.clone();
        import_repository_snapshot(target, &snapshot)?;
        return Ok(repository_id);
    }

    Err(RepositoryError::InvalidArchive {
        message: format!(
            "archive is not a valid tree ({} missing, e.g. {}) and carries no legacy snapshot",
            missing.len(),
            missing[0]
        ),
    })
}

/// Open a `.srs` archive as an in-memory tree session (ADR-038).
///
/// Native archives load layout-faithfully; legacy snapshot archives are
/// materialized through the migration ramp (canonical paths).
pub fn archive_to_tree(reader: impl Read + Seek) -> Result<FileStore, RepositoryError> {
    let mut bytes_map = read_zip_to_map(reader)?;
    let manifest_val = parse_manifest_value(&bytes_map)?;

    let missing = missing_tree_definitions(&bytes_map, &manifest_val);
    if missing.is_empty() {
        bytes_map.remove("package/package.snapshot.json");
        return open_tree(bytes_map.into_iter().collect());
    }

    if bytes_map.contains_key("package/package.snapshot.json") {
        // Legacy migration ramp — remove after ecosystem archives are migrated (#688).
        let snapshot = legacy_snapshot_from_map(&bytes_map, &manifest_val)?;
        let store = FileStore::from_vfs(std::rc::Rc::new(crate::vfs::MemVfs::new()));
        import_repository_snapshot(&store, &snapshot)?;
        return Ok(store);
    }

    Err(RepositoryError::InvalidArchive {
        message: format!(
            "archive is not a valid tree ({} missing, e.g. {}) and carries no legacy snapshot",
            missing.len(),
            missing[0]
        ),
    })
}

/// Legacy (pre-ADR-039) archive: reconstruct a `RepositorySnapshot` from the
/// `package/package.snapshot.json` content model. Migration ramp only (#688).
fn legacy_snapshot_from_map(
    bytes_map: &HashMap<String, Vec<u8>>,
    manifest_val: &serde_json::Value,
) -> Result<RepositorySnapshot, RepositoryError> {
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
        let tags: Option<Vec<String>> = entry.get("tags").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        });

        let inst_bytes = bytes_map
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
            let val: serde_json::Value =
                serde_json::from_slice(rel_bytes).map_err(|e| RepositoryError::InvalidArchive {
                    message: e.to_string(),
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

    let src_docs_base = manifest_val
        .get("sourceDocumentsPath")
        .and_then(|v| v.as_str())
        .unwrap_or("source-documents");
    let source_doc_index = manifest_val
        .get("sourceDocumentIndex")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut source_documents = Vec::new();
    for entry in &source_doc_index {
        let document_id = entry
            .get("documentId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let sidecar_path = entry
            .get("sidecarPath")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let content_path = entry
            .get("contentPath")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let sidecar_full = format!("{}/{}", src_docs_base, sidecar_path);
        let sidecar_bytes = match bytes_map.get(&sidecar_full) {
            Some(b) => b,
            None => continue, // tombstone: sidecar absent
        };
        let sidecar: serde_json::Value =
            serde_json::from_slice(sidecar_bytes).map_err(|e| RepositoryError::InvalidArchive {
                message: e.to_string(),
            })?;

        let content_full = format!("{}/{}", src_docs_base, content_path);
        let content_base64 = bytes_map.get(&content_full).map(|b| BASE64.encode(b));

        // Optional metadata fields are extracted from raw JSON here (camelCase keys from
        // SourceDocumentIndexEntry's #[serde(rename_all = "camelCase")]). This mirrors the
        // typed path in export_repository_snapshot_with_options — any new field added to
        // SourceDocumentIndexEntry must be propagated in both places.
        source_documents.push(SourceDocumentSnapshot {
            document_id,
            sidecar_path,
            content_path,
            sidecar,
            content_base64,
            title: entry
                .get("title")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            sidecar_checksum: entry
                .get("sidecarChecksum")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            content_checksum: entry
                .get("contentChecksum")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        });
    }

    let source_documents_path = if source_documents.is_empty() {
        None
    } else {
        Some(src_docs_base.to_string())
    };

    let root_container: Option<Container> = manifest_val
        .get("container")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let container_index: Option<Vec<ContainerIndexEntry>> = manifest_val
        .get("containerIndex")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    // Restore container JSON files from the archive so import_repository_snapshot
    // can call create_container() for each one.
    let mut containers: Vec<Container> = Vec::new();
    if let Some(index) = &container_index {
        for entry in index {
            if let Some(path) = &entry.path {
                if let Some(bytes) = bytes_map.get(path.as_str()) {
                    let container: Container = serde_json::from_slice(bytes).map_err(|e| {
                        RepositoryError::InvalidArchive {
                            message: format!("invalid container JSON at '{path}': {e}"),
                        }
                    })?;
                    containers.push(container);
                }
            }
        }
    }

    Ok(RepositorySnapshot {
        repository: repo_meta,
        declared_extensions,
        packages: vec![primary_pkg],
        instances,
        containers,
        root_container,
        container_index,
        relations,
        source_documents_path,
        source_documents,
        upstream_package: manifest_val
            .get("upstreamPackage")
            .and_then(|v| serde_json::from_value(v.clone()).ok()),
        meta: manifest_val.get("meta").cloned(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_lifecycle::{InitializeRepositoryInput, PrimaryPackageMetadata};
    use crate::store::memory::MemoryStore;
    use std::io::Cursor;

    fn init_memory_store() -> MemoryStore {
        use crate::repository_lifecycle::RepositoryMetadata;
        let store = MemoryStore::uninitialized();
        store
            .initialize_repository(&InitializeRepositoryInput {
                repository: RepositoryMetadata {
                    repository_id: "test-repo-id".to_string(),
                    namespace: "com.example.test".to_string(),
                    srs_version: "2.0-draft".to_string(),
                    title: Some("Test Repository".to_string()),
                    description: None,
                },
                primary_package: PrimaryPackageMetadata {
                    id: "test-pkg-id".to_string(),
                    namespace: "com.example.test".to_string(),
                    name: "test-package".to_string(),
                    version: "1.0.0".to_string(),
                },
            })
            .expect("initialize_repository failed");
        store
    }

    fn pack_to_bytes(store: &dyn RepositoryStore) -> Vec<u8> {
        let mut buf = Vec::new();
        archive_pack(store, Cursor::new(&mut buf)).expect("archive_pack failed");
        buf
    }

    #[test]
    fn test_archive_roundtrip() {
        use crate::writer::new_instance_id;

        let source = init_memory_store();

        let note_id = new_instance_id();
        let note_value = serde_json::json!({
            "id": note_id,
            "tier": 0,
            "title": "Test Note",
            "sections": [{ "id": "s1", "title": "Intro", "content": "Hello" }]
        });
        source
            .save_instance_json(
                &format!("records/notes/{}.json", &note_id[..8]),
                &note_value,
            )
            .expect("save instance");

        let mut manifest = source.load_manifest().expect("load manifest");
        manifest
            .instance_index
            .push(crate::index::InstanceIndexEntry {
                instance_id: note_id.clone(),
                tier: 0,
                path: format!("records/notes/{}.json", &note_id[..8]),
                title: Some(serde_json::Value::String("Test Note".to_string())),
                tags: None,
            });
        source.save_manifest(&manifest).expect("save manifest");

        let zip_bytes = pack_to_bytes(&source);
        assert!(!zip_bytes.is_empty(), "pack produced no bytes");

        let target = MemoryStore::uninitialized();
        archive_unpack(Cursor::new(&zip_bytes), &target).expect("archive_unpack failed");

        let unpacked = target.load_manifest().expect("load target manifest");
        assert_eq!(unpacked.instance_index.len(), 1);
        assert_eq!(unpacked.instance_index[0].instance_id, note_id);

        // Verify instance body survived roundtrip
        let inst_path = &unpacked.instance_index[0].path;
        let inst_body = target
            .load_instance_json(inst_path)
            .expect("load unpacked instance");
        assert_eq!(inst_body["title"], "Test Note");
        assert_eq!(inst_body["tier"], 0);
        let sections = inst_body["sections"].as_array().expect("sections array");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0]["content"], "Hello");
    }

    #[test]
    fn test_archive_unpack_missing_package_snapshot() {
        use zip::write::SimpleFileOptions;

        let manifest_json = serde_json::json!({
            "repositoryId": "test-id",
            "namespace": "com.example",
            "srsVersion": "2.0-draft",
            "instanceIndex": []
        });
        let mut buf = Vec::new();
        let mut zw = zip::ZipWriter::new(Cursor::new(&mut buf));
        let opts = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        zw.start_file("manifest.json", opts).unwrap();
        zw.write_all(
            serde_json::to_vec_pretty(&manifest_json)
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        let _ = zw.finish().unwrap();

        let target = MemoryStore::uninitialized();
        let result = archive_unpack(Cursor::new(buf), &target);
        assert!(
            matches!(result, Err(RepositoryError::InvalidArchive { .. })),
            "expected InvalidArchive for missing package snapshot, got {:?}",
            result
        );
    }

    #[test]
    fn test_archive_cross_store_roundtrip() {
        use crate::store::FileStore;
        use crate::writer::new_instance_id;
        use tempfile::tempdir;

        // Pack from MemoryStore, unpack into FileStore
        let source = init_memory_store();
        let note_id = new_instance_id();
        let note_value = serde_json::json!({
            "id": note_id,
            "tier": 0,
            "title": "Cross-Store Note",
            "sections": [{ "id": "s1", "title": "Body", "content": "cross-store content" }]
        });
        source
            .save_instance_json(
                &format!("records/notes/{}.json", &note_id[..8]),
                &note_value,
            )
            .expect("save instance to memory");

        let mut manifest = source.load_manifest().expect("load memory manifest");
        manifest
            .instance_index
            .push(crate::index::InstanceIndexEntry {
                instance_id: note_id.clone(),
                tier: 0,
                path: format!("records/notes/{}.json", &note_id[..8]),
                title: Some(serde_json::Value::String("Cross-Store Note".to_string())),
                tags: None,
            });
        source.save_manifest(&manifest).expect("save manifest");

        let zip_bytes = pack_to_bytes(&source);

        let target_dir = tempdir().unwrap();
        let target = FileStore::new(target_dir.path());
        archive_unpack(Cursor::new(&zip_bytes), &target).expect("cross-store unpack failed");

        let unpacked = target.load_manifest().expect("load filestore manifest");
        assert_eq!(unpacked.instance_index.len(), 1);
        assert_eq!(unpacked.instance_index[0].instance_id, note_id);

        let inst_path = &unpacked.instance_index[0].path;
        let inst_body = target
            .load_instance_json(inst_path)
            .expect("load cross-store instance");
        assert_eq!(inst_body["title"], "Cross-Store Note");
        assert_eq!(inst_body["sections"][0]["content"], "cross-store content");
    }

    #[test]
    fn test_archive_determinism() {
        let store = init_memory_store();
        let bytes1 = pack_to_bytes(&store);
        let bytes2 = pack_to_bytes(&store);
        assert_eq!(bytes1, bytes2, "archive_pack is not deterministic");
    }

    fn extract_zip_entry(zip_bytes: &[u8], entry_name: &str) -> Vec<u8> {
        let mut zip = zip::ZipArchive::new(Cursor::new(zip_bytes)).expect("open zip");
        let mut entry = zip.by_name(entry_name).expect("entry not found");
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buf).expect("read entry");
        buf
    }

    #[test]
    fn test_archive_determinism_from_jsonstore() {
        // Build a .srsj whose extra keys arrive in non-alphabetical order ("zzz" before "aaa").
        // Without the to_value fix, load_text_file("manifest.json") emits them in HashMap
        // iteration order (non-deterministic across process runs). With the fix, to_value
        // normalises all keys — typed fields and extra — into BTreeMap (sorted) order (ADR-017).
        let srsj = r#"{"srsj":"1","manifest":{"instanceIndex":[],"repositoryId":"det-test-id","namespace":"com.example.det","srsVersion":"2.0-draft","title":"Det Test","zzz":"last","aaa":"first","createdAt":"2026-01-01T00:00:00Z"},"data":{"package/package.json":{"id":"p","namespace":"com.example.det","name":"n","version":"1","fields":[],"types":[],"relationTypes":[],"views":[],"documentViews":[]}}}"#;

        let store = crate::JsonStore::from_srsj(srsj).unwrap();
        let archive_bytes = pack_to_bytes(&store);

        // Read the raw (unreparsed) manifest.json bytes from the archive.
        let raw = extract_zip_entry(&archive_bytes, "manifest.json");
        let manifest_text = std::str::from_utf8(&raw).expect("manifest.json is UTF-8");

        let pos_aaa = manifest_text
            .find("\"aaa\"")
            .expect("\"aaa\" key must be present");
        let pos_zzz = manifest_text
            .find("\"zzz\"")
            .expect("\"zzz\" key must be present");
        assert!(
            pos_aaa < pos_zzz,
            "manifest.json keys not sorted: \"zzz\" appears before \"aaa\" (issue #654)"
        );
    }

    #[test]
    fn test_archive_manifest_bytes_identical_filestore_vs_jsonstore() {
        use crate::repository_lifecycle::RepositoryMetadata;
        use crate::store::FileStore;
        use tempfile::TempDir;

        let init_input = InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "cross-store-determinism-00000000000000000000000000".to_string(),
                namespace: "com.example.test".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: Some("Cross-Store Determinism Test".to_string()),
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "cross-store-pkg-00000000-0000-0000-0000-000000000001".to_string(),
                namespace: "com.example.test".to_string(),
                name: "test-package".to_string(),
                version: "1.0.0".to_string(),
            },
        };

        // Initialize FileStore and pin createdAt to a fixed value.
        let file_dir = TempDir::new().unwrap();
        let file_store = FileStore::new(file_dir.path());
        file_store
            .initialize_repository(&init_input)
            .expect("initialize file store");
        let mut manifest = file_store.load_manifest().unwrap();
        manifest.extra.insert(
            "createdAt".to_string(),
            serde_json::json!("2026-01-01T00:00:00Z"),
        );
        file_store.save_manifest(&manifest).unwrap();

        // Initialize JsonStore with the same input and pin the same createdAt.
        let json_dir = TempDir::new().unwrap();
        let json_path = json_dir.path().join("repo.srsj");
        let json_store = crate::JsonStore::create(&json_path).unwrap();
        json_store
            .initialize_repository(&init_input)
            .expect("initialize json store");
        let mut json_manifest = json_store.load_manifest().unwrap();
        json_manifest.extra.insert(
            "createdAt".to_string(),
            serde_json::json!("2026-01-01T00:00:00Z"),
        );
        json_store.save_manifest(&json_manifest).unwrap();

        let file_bytes = pack_to_bytes(&file_store);
        let json_bytes = pack_to_bytes(&json_store);

        let manifest_from_file = extract_zip_entry(&file_bytes, "manifest.json");
        let manifest_from_json = extract_zip_entry(&json_bytes, "manifest.json");
        assert_eq!(
            manifest_from_file, manifest_from_json,
            "manifest.json differs between FileStore and JsonStore packs (issue #654)"
        );
    }

    #[test]
    fn test_archive_zip_entry_order() {
        let store = init_memory_store();
        let bytes = pack_to_bytes(&store);

        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("open zip");
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "ZIP entries are not in lexicographic order");
    }

    #[test]
    fn test_archive_zip_timestamps() {
        let store = init_memory_store();
        let bytes = pack_to_bytes(&store);

        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("open zip");
        let default_dt = zip::DateTime::default();
        for i in 0..zip.len() {
            let entry = zip.by_index(i).unwrap();
            if let Some(dt) = entry.last_modified() {
                assert_eq!(
                    dt,
                    default_dt,
                    "entry '{}' has non-default timestamp",
                    entry.name()
                );
            }
        }
    }

    #[test]
    fn test_archive_unpack_missing_manifest() {
        use zip::write::SimpleFileOptions;
        let mut buf = Vec::new();
        let mut zw = zip::ZipWriter::new(Cursor::new(&mut buf));
        let opts = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        zw.start_file("some-other-file.txt", opts).unwrap();
        zw.write_all(b"content").unwrap();
        let _ = zw.finish().unwrap();

        let target = MemoryStore::empty();
        let result = archive_unpack(Cursor::new(buf), &target);
        assert!(
            matches!(result, Err(RepositoryError::InvalidArchive { .. })),
            "expected InvalidArchive, got {:?}",
            result
        );
    }

    #[test]
    fn test_archive_roundtrip_filestore() {
        use crate::repository_lifecycle::{InitializeRepositoryInput, RepositoryMetadata};
        use crate::store::FileStore;
        use crate::writer::new_instance_id;
        use tempfile::tempdir;

        let source_dir = tempdir().unwrap();
        let source = FileStore::new(source_dir.path());
        source
            .initialize_repository(&InitializeRepositoryInput {
                repository: RepositoryMetadata {
                    repository_id: "filestore-test-id".to_string(),
                    namespace: "com.example.filetest".to_string(),
                    srs_version: "2.0-draft".to_string(),
                    title: Some("FileStore Test".to_string()),
                    description: None,
                },
                primary_package: PrimaryPackageMetadata {
                    id: "filestore-pkg-id".to_string(),
                    namespace: "com.example.filetest".to_string(),
                    name: "filestore-package".to_string(),
                    version: "1.0.0".to_string(),
                },
            })
            .expect("initialize source FileStore");

        let note_id = new_instance_id();
        let note_value = serde_json::json!({
            "id": note_id,
            "tier": 0,
            "title": "FileStore Note",
            "sections": []
        });
        source
            .ensure_instance_dir("records/notes")
            .expect("ensure records/notes dir");
        source
            .save_instance_json(
                &format!("records/notes/{}.json", &note_id[..8]),
                &note_value,
            )
            .expect("save instance to FileStore");

        let mut manifest = source.load_manifest().expect("load FileStore manifest");
        manifest
            .instance_index
            .push(crate::index::InstanceIndexEntry {
                instance_id: note_id.clone(),
                tier: 0,
                path: format!("records/notes/{}.json", &note_id[..8]),
                title: Some(serde_json::Value::String("FileStore Note".to_string())),
                tags: None,
            });
        source.save_manifest(&manifest).expect("save manifest");

        let zip_dir = tempdir().unwrap();
        let zip_path = zip_dir.path().join("test.srs");
        let mut zip_file = std::fs::File::create(&zip_path).expect("create zip file");
        archive_pack(&source, &mut zip_file).expect("archive_pack FileStore");
        drop(zip_file);

        let target_dir = tempdir().unwrap();
        let target = FileStore::new(target_dir.path());
        let zip_file2 = std::fs::File::open(&zip_path).expect("open zip file");
        archive_unpack(zip_file2, &target).expect("archive_unpack FileStore");

        let unpacked = target
            .load_manifest()
            .expect("load target FileStore manifest");
        assert_eq!(unpacked.instance_index.len(), 1);
        assert_eq!(unpacked.instance_index[0].instance_id, note_id);
    }

    #[test]
    fn test_archive_roundtrip_with_source_documents() {
        const SIDECAR_JSON: &str = r#"{"documentId":"test-doc-aaaa","contentPath":"my-doc.pdf","contentType":"application/pdf"}"#;
        const BINARY_CONTENT: &[u8] = b"\x00\x01\x02\x03 binary pdf content";

        let source = init_memory_store();
        source
            .save_text_file("source-documents/my-doc.meta.json", SIDECAR_JSON)
            .expect("save sidecar");
        source
            .save_binary_file("source-documents/my-doc.pdf", BINARY_CONTENT)
            .expect("save binary");

        let mut manifest = source.load_manifest().expect("load manifest");
        manifest.source_documents_path = Some("source-documents".to_string());
        manifest.source_document_index = Some(vec![
            srs_core::types::source_document::SourceDocumentIndexEntry {
                document_id: "test-doc-aaaa".to_string(),
                sidecar_path: "my-doc.meta.json".to_string(),
                content_path: "my-doc.pdf".to_string(),
                title: None,
                sidecar_checksum: None,
                content_checksum: None,
            },
        ]);
        source.save_manifest(&manifest).expect("save manifest");

        let zip_bytes = pack_to_bytes(&source);

        let target = MemoryStore::uninitialized();
        archive_unpack(Cursor::new(&zip_bytes), &target).expect("unpack failed");

        let restored = target.load_manifest().expect("load restored manifest");
        let idx = restored
            .source_document_index
            .as_ref()
            .expect("source_document_index missing");
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].document_id, "test-doc-aaaa");

        let restored_bytes = target
            .load_binary_file("source-documents/my-doc.pdf")
            .expect("load binary content");
        assert_eq!(restored_bytes.as_slice(), BINARY_CONTENT);

        let restored_sidecar = target
            .load_text_file("source-documents/my-doc.meta.json")
            .expect("load sidecar");
        let sidecar_val: serde_json::Value =
            serde_json::from_str(&restored_sidecar).expect("parse sidecar");
        assert_eq!(sidecar_val["documentId"], "test-doc-aaaa");
    }

    #[test]
    fn test_archive_roundtrip_with_source_documents_subdir() {
        const SIDECAR_JSON: &str = r#"{"documentId":"subdir-doc-bbbb","contentPath":"reports/2026/analysis.pdf","contentType":"application/pdf"}"#;
        const BINARY_CONTENT: &[u8] = b"subdir pdf bytes";

        let source = init_memory_store();
        source
            .save_text_file(
                "source-documents/reports/2026/analysis.meta.json",
                SIDECAR_JSON,
            )
            .expect("save sidecar");
        source
            .save_binary_file("source-documents/reports/2026/analysis.pdf", BINARY_CONTENT)
            .expect("save binary");

        let mut manifest = source.load_manifest().expect("load manifest");
        manifest.source_documents_path = Some("source-documents".to_string());
        manifest.source_document_index = Some(vec![
            srs_core::types::source_document::SourceDocumentIndexEntry {
                document_id: "subdir-doc-bbbb".to_string(),
                sidecar_path: "reports/2026/analysis.meta.json".to_string(),
                content_path: "reports/2026/analysis.pdf".to_string(),
                title: None,
                sidecar_checksum: None,
                content_checksum: None,
            },
        ]);
        source.save_manifest(&manifest).expect("save manifest");

        let zip_bytes = pack_to_bytes(&source);

        let target = MemoryStore::uninitialized();
        archive_unpack(Cursor::new(&zip_bytes), &target).expect("unpack failed");

        let restored = target.load_manifest().expect("load restored manifest");
        let idx = restored
            .source_document_index
            .as_ref()
            .expect("source_document_index missing");
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].content_path, "reports/2026/analysis.pdf");

        let restored_bytes = target
            .load_binary_file("source-documents/reports/2026/analysis.pdf")
            .expect("load subdir binary content");
        assert_eq!(restored_bytes.as_slice(), BINARY_CONTENT);
    }

    #[test]
    fn test_archive_tombstone_roundtrip() {
        // Tombstone: sidecar present, content file absent — index entry survives pack→unpack.
        // Verifies ADR-031: tombstone state is valid and must not be lost in archive roundtrip.
        const SIDECAR_JSON: &str = r#"{"documentId":"tombstone-doc-dddd","contentPath":"gone.pdf","contentType":"application/pdf"}"#;

        let source = init_memory_store();
        source
            .save_text_file("source-documents/gone.meta.json", SIDECAR_JSON)
            .expect("save tombstone sidecar");
        // Intentionally no save_binary_file("source-documents/gone.pdf") — tombstone state.

        let mut manifest = source.load_manifest().expect("load manifest");
        manifest.source_documents_path = Some("source-documents".to_string());
        manifest.source_document_index = Some(vec![
            srs_core::types::source_document::SourceDocumentIndexEntry {
                document_id: "tombstone-doc-dddd".to_string(),
                sidecar_path: "gone.meta.json".to_string(),
                content_path: "gone.pdf".to_string(),
                title: None,
                sidecar_checksum: None,
                content_checksum: None,
            },
        ]);
        source.save_manifest(&manifest).expect("save manifest");

        let zip_bytes = pack_to_bytes(&source);

        let target = MemoryStore::uninitialized();
        archive_unpack(Cursor::new(&zip_bytes), &target).expect("unpack failed");

        // Index entry must survive in the restored manifest.
        let restored = target.load_manifest().expect("load manifest");
        let idx = restored
            .source_document_index
            .as_ref()
            .expect("sourceDocumentIndex must survive tombstone roundtrip");
        assert_eq!(idx.len(), 1, "tombstone index entry count");
        assert_eq!(idx[0].document_id, "tombstone-doc-dddd");

        // Sidecar must be present after roundtrip.
        let sidecar = target
            .load_text_file("source-documents/gone.meta.json")
            .expect("tombstone sidecar must survive roundtrip");
        let sidecar_val: serde_json::Value =
            serde_json::from_str(&sidecar).expect("parse sidecar");
        assert_eq!(sidecar_val["documentId"], "tombstone-doc-dddd");

        // Content file must remain absent — tombstone state preserved.
        let content_result = target.load_binary_file("source-documents/gone.pdf");
        assert!(
            content_result.is_err(),
            "tombstone content file must remain absent after roundtrip"
        );
    }

    #[test]
    fn test_archive_roundtrip_preserves_checksum_metadata() {
        use crate::store::FileStore;
        use tempfile::tempdir;

        const SIDECAR_JSON: &str = r#"{"documentId":"checksum-doc-cccc","contentPath":"doc.pdf","contentType":"application/pdf"}"#;

        let source = init_memory_store();
        source
            .save_text_file("source-documents/doc.meta.json", SIDECAR_JSON)
            .expect("save sidecar");
        source
            .save_binary_file("source-documents/doc.pdf", b"doc bytes")
            .expect("save binary");

        let mut manifest = source.load_manifest().expect("load manifest");
        manifest.source_documents_path = Some("source-documents".to_string());
        manifest.source_document_index = Some(vec![
            srs_core::types::source_document::SourceDocumentIndexEntry {
                document_id: "checksum-doc-cccc".to_string(),
                sidecar_path: "doc.meta.json".to_string(),
                content_path: "doc.pdf".to_string(),
                title: Some("Checksum Doc".to_string()),
                sidecar_checksum: Some("sha256:aaa111".to_string()),
                content_checksum: Some("sha256:bbb222".to_string()),
            },
        ]);
        source.save_manifest(&manifest).expect("save manifest");

        let zip_bytes = pack_to_bytes(&source);

        let target_dir = tempdir().unwrap();
        let target = FileStore::new(target_dir.path());
        archive_unpack(Cursor::new(&zip_bytes), &target).expect("unpack failed");

        let restored = target.load_manifest().expect("load restored manifest");
        let idx = restored
            .source_document_index
            .as_ref()
            .expect("source_document_index missing");
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].document_id, "checksum-doc-cccc");
        assert_eq!(idx[0].title, Some("Checksum Doc".to_string()));
        assert_eq!(idx[0].sidecar_checksum, Some("sha256:aaa111".to_string()));
        assert_eq!(idx[0].content_checksum, Some("sha256:bbb222".to_string()));
    }

    #[test]
    fn test_archive_roundtrip_filestore_with_source_docs() {
        use crate::repository_lifecycle::{InitializeRepositoryInput, RepositoryMetadata};
        use crate::store::FileStore;
        use tempfile::tempdir;

        const SIDECAR_JSON: &str = r#"{"documentId":"filestore-doc-dddd","contentPath":"report.pdf","contentType":"application/pdf"}"#;
        const BINARY_CONTENT: &[u8] = b"binary report content\x00\x01\x02";

        let source_dir = tempdir().unwrap();
        let source = FileStore::new(source_dir.path());
        source
            .initialize_repository(&InitializeRepositoryInput {
                repository: RepositoryMetadata {
                    repository_id: "source-repo-id".to_string(),
                    namespace: "com.example.srcdoc".to_string(),
                    srs_version: "2.0-draft".to_string(),
                    title: Some("Source Doc Test".to_string()),
                    description: None,
                },
                primary_package: PrimaryPackageMetadata {
                    id: "src-pkg-id".to_string(),
                    namespace: "com.example.srcdoc".to_string(),
                    name: "src-package".to_string(),
                    version: "1.0.0".to_string(),
                },
            })
            .expect("initialize source FileStore");

        source
            .save_text_file("source-documents/report.meta.json", SIDECAR_JSON)
            .expect("save sidecar to FileStore");
        source
            .save_binary_file("source-documents/report.pdf", BINARY_CONTENT)
            .expect("save binary to FileStore");

        let mut manifest = source.load_manifest().expect("load FileStore manifest");
        manifest.source_documents_path = Some("source-documents".to_string());
        manifest.source_document_index = Some(vec![
            srs_core::types::source_document::SourceDocumentIndexEntry {
                document_id: "filestore-doc-dddd".to_string(),
                sidecar_path: "report.meta.json".to_string(),
                content_path: "report.pdf".to_string(),
                title: None,
                sidecar_checksum: None,
                content_checksum: None,
            },
        ]);
        source.save_manifest(&manifest).expect("save manifest");

        let zip_dir = tempdir().unwrap();
        let zip_path = zip_dir.path().join("repo.srs");
        let mut zip_file = std::fs::File::create(&zip_path).expect("create zip");
        archive_pack(&source, &mut zip_file).expect("archive_pack");
        drop(zip_file);

        let target_dir = tempdir().unwrap();
        let target = FileStore::new(target_dir.path());
        let zip_file2 = std::fs::File::open(&zip_path).expect("open zip");
        archive_unpack(zip_file2, &target).expect("archive_unpack");

        let restored = target.load_manifest().expect("load target manifest");
        let idx = restored
            .source_document_index
            .as_ref()
            .expect("source_document_index missing");
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].document_id, "filestore-doc-dddd");

        let content_path = target_dir
            .path()
            .join("source-documents")
            .join("report.pdf");
        assert!(
            content_path.exists(),
            "content file should exist at source-documents/report.pdf"
        );

        let sidecar_path = target_dir
            .path()
            .join("source-documents")
            .join("report.meta.json");
        assert!(
            sidecar_path.exists(),
            "sidecar file should exist at source-documents/report.meta.json"
        );

        let restored_bytes = std::fs::read(&content_path).expect("read content file");
        assert_eq!(restored_bytes.as_slice(), BINARY_CONTENT);
    }

    #[test]
    fn test_load_from_archive_roundtrip() {
        use crate::services::{list_notes, ListNotesFilter};
        use crate::writer::new_instance_id;

        let source = init_memory_store();

        let note_id = new_instance_id();
        let note_value = serde_json::json!({
            "id": note_id,
            "tier": 0,
            "title": "Archive Service Note",
            "sections": [{ "id": "s1", "title": "Body", "content": "test" }]
        });
        source
            .save_instance_json(
                &format!("records/notes/{}.json", &note_id[..8]),
                &note_value,
            )
            .expect("save instance");
        let mut manifest = source.load_manifest().expect("load manifest");
        manifest
            .instance_index
            .push(crate::index::InstanceIndexEntry {
                instance_id: note_id.clone(),
                tier: 0,
                path: format!("records/notes/{}.json", &note_id[..8]),
                title: Some(serde_json::Value::String(
                    "Archive Service Note".to_string(),
                )),
                tags: None,
            });
        source.save_manifest(&manifest).expect("save manifest");

        let bytes = pack_to_bytes(&source);

        let store = crate::JsonStore::from_archive(&bytes).expect("from_archive should succeed");
        let result =
            list_notes(&store, ListNotesFilter::default()).expect("list_notes on reloaded store");
        assert_eq!(
            result.notes.len(),
            1,
            "should have exactly one note after roundtrip"
        );
        assert_eq!(result.notes[0].instance_id, note_id);
    }

    #[test]
    fn test_load_from_archive_rejects_invalid_bytes() {
        assert!(
            crate::JsonStore::from_archive(b"not a zip").is_err(),
            "from_archive must fail on invalid bytes"
        );
    }

    #[test]
    fn test_archive_no_extra_fields_and_deflated() {
        let store = init_memory_store();
        let bytes = pack_to_bytes(&store);
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("open zip");
        for i in 0..zip.len() {
            let entry = zip.by_index(i).unwrap();
            let extra = entry.extra_data().unwrap_or(&[]);
            assert!(
                extra.is_empty(),
                "entry '{}' has non-empty extra_data (host metadata present): {:?}",
                entry.name(),
                extra
            );
            assert_eq!(
                entry.compression(),
                zip::CompressionMethod::Deflated,
                "entry '{}' uses {:?} instead of Deflated",
                entry.name(),
                entry.compression()
            );
        }
    }

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/exploded-basic");
    const LEGACY: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/legacy-snapshot.srs"
    );

    #[test]
    fn pack_contains_definition_files_no_snapshot() {
        use crate::store::FileStore;
        let store = FileStore::new(FIXTURE);
        let bytes = pack_to_bytes(&store);
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("open zip");
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(names.contains(&"package/fields/title-22222222.json".to_string()));
        assert!(names.contains(&"package/fields/approved-22222222.json".to_string()));
        assert!(names.contains(&"package/types/decision-33333333.json".to_string()));
        assert!(names.contains(&"package/relation-types/precedes-66666666.json".to_string()));
        assert!(
            !names.iter().any(|n| n.contains("package.snapshot.json")),
            "snapshot file must never be written again (ADR-039): {names:?}"
        );
    }

    #[test]
    fn pack_unpack_tree_roundtrip_byte_faithful() {
        use crate::store::FileStore;
        use tempfile::tempdir;

        let source = FileStore::new(FIXTURE);
        let bytes = pack_to_bytes(&source);

        // Every packed entry unpacks byte-identically at its original path —
        // no re-canonicalization (ADR-039).
        let target_dir = tempdir().unwrap();
        let target = FileStore::new(target_dir.path());
        archive_unpack(Cursor::new(&bytes), &target).expect("native unpack");

        let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).expect("open zip");
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).unwrap();
            let name = entry.name().to_string();
            let mut packed = Vec::new();
            entry.read_to_end(&mut packed).unwrap();
            let unpacked = std::fs::read(target_dir.path().join(&name))
                .unwrap_or_else(|e| panic!("missing unpacked file {name}: {e}"));
            assert_eq!(packed, unpacked, "byte drift at {name}");
        }
        assert!(
            !target_dir
                .path()
                .join("package/package.snapshot.json")
                .exists(),
            "no snapshot file may appear on unpack"
        );
        // The re-materialized repository is a working repo.
        assert!(target.repository_exists().unwrap());
        let report = crate::validation::validate_repository(&target).expect("validate");
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| format!("{d:?}").contains("Error"))
            .collect();
        assert!(errors.is_empty(), "unpacked repo has errors: {errors:?}");
    }

    #[test]
    fn legacy_snapshot_archive_still_loads() {
        // Migration ramp (#688): the committed fixture was packed by the
        // pre-ADR-039 code (package.snapshot.json, no per-definition files).
        let bytes = std::fs::read(LEGACY).expect("legacy fixture committed?");
        let tree = crate::archive::archive_to_tree(Cursor::new(bytes)).expect("legacy load");
        let report = crate::validation::validate_repository(&tree).expect("validate");
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| format!("{d:?}").contains("Error"))
            .collect();
        assert!(errors.is_empty(), "legacy archive invalid: {errors:?}");
    }

    #[test]
    fn pack_missing_definition_file_errors() {
        let store = init_memory_store();
        let mut pkg = store.load_package_json().expect("pkg json");
        pkg["fields"]
            .as_array_mut()
            .expect("fields array")
            .push(serde_json::json!("fields/ghost.json"));
        store.save_package_json(&pkg).expect("save pkg json");

        let mut buf = Vec::new();
        let err = archive_pack(&store, Cursor::new(&mut buf)).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("package/fields/ghost.json"),
            "error must name the missing path: {msg}"
        );
    }

    #[test]
    fn native_unpack_rejects_non_empty_target() {
        use crate::store::FileStore;
        use tempfile::tempdir;

        let source = FileStore::new(FIXTURE);
        let bytes = pack_to_bytes(&source);

        let target_dir = tempdir().unwrap();
        let target = FileStore::new(target_dir.path());
        archive_unpack(Cursor::new(&bytes), &target).expect("first unpack");
        let err = archive_unpack(Cursor::new(&bytes), &target).unwrap_err();
        assert!(
            matches!(err, RepositoryError::RepositoryNotEmpty { .. }),
            "second unpack into a populated target must fail, got {err:?}"
        );
    }
}
