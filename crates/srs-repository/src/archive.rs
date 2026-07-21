use crate::error::RepositoryError;
use crate::repository_lifecycle::RepositoryMetadata;
use crate::repository_portability::{
    export_repository_snapshot_with_options, import_repository_snapshot, ExportSnapshotOptions,
    PackageBoundarySnapshot, RepositorySnapshot, SnapshotInstance, SourceDocumentSnapshot,
};
use crate::store::RepositoryStore;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use srs_core::types::container::{Container, ContainerIndexEntry};
use srs_core::types::relation::Relation;
use std::collections::HashMap;
use std::io::{Read, Seek, Write};
use zip::write::SimpleFileOptions;

pub fn archive_pack(
    source: &dyn RepositoryStore,
    writer: impl Write + Seek,
) -> Result<(), RepositoryError> {
    let snapshot = export_repository_snapshot_with_options(
        source,
        ExportSnapshotOptions {
            include_content_blobs: true,
        },
    )?;

    // Load the manifest to get the actual storage paths for each instance —
    // these may not be canonical (e.g. the file predates canonicalization),
    // so we mirror the real on-disk layout rather than re-deriving paths.
    let manifest = source.load_manifest()?;

    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();

    entries.push((
        "manifest.json".to_string(),
        source.load_manifest_raw_text()?.into_bytes(),
    ));

    entries.push((
        "package/package.json".to_string(),
        source.load_primary_package_raw_text()?.into_bytes(),
    ));

    if let Some(pkg) = snapshot.packages.iter().find(|p| p.boundary_path.is_none()) {
        // Route through to_value() so serde_json::Map (BTreeMap-backed) sorts all
        // HashMap<String,Value> fields, making the snapshot byte-stable across process runs.
        // See ADR-017: preserve_order must remain disabled or this guarantee breaks.
        let pkg_value =
            serde_json::to_value(pkg).map_err(|e| RepositoryError::InvalidSnapshotData {
                message: e.to_string(),
            })?;
        let pkg_bytes = serde_json::to_vec_pretty(&pkg_value).map_err(|e| {
            RepositoryError::InvalidSnapshotData {
                message: e.to_string(),
            }
        })?;
        entries.push(("package/package.snapshot.json".to_string(), pkg_bytes));
    }

    if let Some(text) = source.load_relations_raw_text()? {
        entries.push((
            "relations/relations-collection.json".to_string(),
            text.into_bytes(),
        ));
    }

    for entry in &manifest.instance_index {
        let content = source.load_text_file(&entry.path)?;
        entries.push((entry.path.clone(), content.into_bytes()));
    }

    let src_docs_dir = snapshot
        .source_documents_path
        .as_deref()
        .unwrap_or("source-documents");
    for doc in &snapshot.source_documents {
        let sidecar_bytes = serde_json::to_vec_pretty(&doc.sidecar).map_err(|e| {
            RepositoryError::InvalidSnapshotData {
                message: e.to_string(),
            }
        })?;
        entries.push((
            format!("{}/{}", src_docs_dir, doc.sidecar_path),
            sidecar_bytes,
        ));
        if let Some(b64) = &doc.content_base64 {
            let content_bytes =
                BASE64
                    .decode(b64)
                    .map_err(|e| RepositoryError::InvalidSnapshotData {
                        message: e.to_string(),
                    })?;
            entries.push((
                format!("{}/{}", src_docs_dir, doc.content_path),
                content_bytes,
            ));
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

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

pub fn archive_unpack(
    reader: impl Read + Seek,
    target: &dyn RepositoryStore,
) -> Result<(), RepositoryError> {
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

    let snapshot = RepositorySnapshot {
        repository: repo_meta,
        declared_extensions,
        packages: vec![primary_pkg],
        instances,
        containers: Vec::new(),
        root_container,
        container_index,
        relations,
        source_documents_path,
        source_documents,
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
}
