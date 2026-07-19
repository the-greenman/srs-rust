use crate::error::RepositoryError;
use crate::index::InstanceIndexEntry;
use crate::record_store;
use crate::store::RepositoryStore;
use crate::writer::write_manifest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use srs_core::types::source_document::SourceDocumentIndexEntry;
use srs_core::types::source_document_meta::SourceDocumentMeta;
use srs_core::types::source_reference::{SourceReference, SourceRole, SourceType};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentEntry {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_checksum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidecar_checksum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListAttachmentsResult {
    pub source_documents_path: String,
    pub entries: Vec<AttachmentEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListAttachmentsFilter {}

/// List source document attachments by walking `source_documents_path` recursively.
///
/// Sidecar files (`.meta.json`) are excluded from the listing; their metadata is
/// surfaced through the index fields (`document_id`, `title`, etc.) on the content entry.
/// Files not present in `manifest.sourceDocumentIndex` appear with only `path` populated.
pub fn list_attachments(
    store: &dyn RepositoryStore,
    _filter: ListAttachmentsFilter,
) -> Result<ListAttachmentsResult, RepositoryError> {
    let manifest = store.load_manifest()?;

    let src_docs_base = manifest
        .source_documents_path
        .as_deref()
        .unwrap_or("source-documents")
        .to_string();

    // Build index map keyed on content_path (relative to src_docs_base).
    let index_entries = manifest.source_document_index.as_deref().unwrap_or(&[]);
    let index_map: HashMap<&str, _> = index_entries
        .iter()
        .map(|e| (e.content_path.as_str(), e))
        .collect();

    // Walk the filesystem. list_files_recursive returns paths relative to repo root,
    // prefixed with src_docs_base + "/".
    let prefix = format!("{}/", src_docs_base);
    let all_files = store.list_files_recursive(&src_docs_base);

    let mut entries: Vec<AttachmentEntry> = all_files
        .into_iter()
        // Capture the repo-relative path together with the stripped relative path.
        .filter_map(|repo_rel| {
            let rel = repo_rel.strip_prefix(&prefix)?.to_string();
            Some((repo_rel, rel))
        })
        // Exclude .meta.json sidecars.
        .filter(|(_, rel)| !rel.ends_with(".meta.json"))
        .map(|(full_rel, rel)| {
            // Best-effort: FileStore returns the real byte count via fs::metadata;
            // MemoryStore returns None (binary_data is separate from text data).
            let size_bytes = store.file_byte_len(&full_rel).ok();
            if let Some(idx) = index_map.get(rel.as_str()) {
                AttachmentEntry {
                    path: rel,
                    document_id: Some(idx.document_id.clone()).filter(|s| !s.is_empty()),
                    title: idx.title.clone(),
                    content_checksum: idx.content_checksum.clone(),
                    sidecar_checksum: idx.sidecar_checksum.clone(),
                    size_bytes,
                }
            } else {
                AttachmentEntry {
                    path: rel,
                    document_id: None,
                    title: None,
                    content_checksum: None,
                    sidecar_checksum: None,
                    size_bytes,
                }
            }
        })
        .collect();

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(ListAttachmentsResult {
        source_documents_path: src_docs_base,
        entries,
    })
}

// ── add_attachment ─────────────────────────────────────────────────────────────

/// Input for `add_attachment`. The CLI reads the source file from disk and passes
/// its bytes here; the service never touches the local filesystem directly.
pub struct AddAttachmentInput {
    /// Original filename (e.g. `"brief.pdf"`), used to derive `content_path` and
    /// `sidecar_path`. Must not contain path separators or `..`.
    pub file_name: String,
    /// Raw bytes of the file to store.
    pub content: Vec<u8>,
    /// Optional subdirectory within `source-documents/` (e.g. `"phase-1"`).
    pub subdir: Option<String>,
    /// Optional human-readable title stored in the sidecar and manifest index.
    pub title: Option<String>,
    /// MIME type (e.g. `"application/pdf"`). Auto-detected from file extension if `None`.
    pub content_type: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddAttachmentResult {
    /// Generated UUID for this document.
    pub document_id: String,
    /// Path of the content file relative to `source-documents/`.
    pub content_path: String,
    /// Path of the sidecar file relative to `source-documents/`.
    pub sidecar_path: String,
    /// Resolved `source-documents/` base dir (from manifest or default).
    pub source_documents_path: String,
    /// `sha256:<hex>` checksum of the content file.
    pub content_checksum: String,
    /// `sha256:<hex>` checksum of the sidecar JSON bytes.
    pub sidecar_checksum: String,
}

fn infer_content_type(file_name: &str) -> &'static str {
    match file_name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "html" | "htm" => "text/html",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "zip" => "application/zip",
        "json" => "application/json",
        "csv" => "text/csv",
        _ => "application/octet-stream",
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    format!("sha256:{}", hex::encode(hash))
}

/// Store `input.content` under `source-documents/<[subdir/]file_name>`, write its
/// `.meta.json` sidecar, and append a `SourceDocumentIndexEntry` to the manifest.
///
/// Write order (ADR-007 file-before-index for create):
///   1. Write content file
///   2. Write sidecar file
///   3. Update and save manifest index
///
/// Returns `RepositoryError::InvalidInput` if the content file already exists.
pub fn add_attachment(
    store: &dyn RepositoryStore,
    input: AddAttachmentInput,
) -> Result<AddAttachmentResult, RepositoryError> {
    let file_name = input.file_name.trim().to_string();
    if file_name.is_empty() {
        return Err(RepositoryError::InvalidInput {
            message: "file_name must not be empty".to_string(),
        });
    }
    if file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
        return Err(RepositoryError::InvalidInput {
            message: format!("file_name must not contain path separators: {file_name}"),
        });
    }
    if let Some(ref sub) = input.subdir {
        let sub = sub.trim();
        if sub.starts_with('/') || sub.starts_with('\\') || sub.contains("..") {
            return Err(RepositoryError::InvalidInput {
                message: format!("subdir must be a relative path without '..': {sub}"),
            });
        }
    }

    let manifest = store.load_manifest()?;
    let src_docs_base = manifest
        .source_documents_path
        .as_deref()
        .unwrap_or("source-documents")
        .to_string();

    let rel_content_path = match &input.subdir {
        Some(sub) => {
            let sub = sub.trim().trim_matches('/');
            if sub.is_empty() {
                file_name.clone()
            } else {
                format!("{sub}/{file_name}")
            }
        }
        None => file_name.clone(),
    };

    let sidecar_name = {
        let stem = file_name
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or(&file_name);
        format!("{stem}.meta.json")
    };
    let rel_sidecar_path = match &input.subdir {
        Some(sub) => {
            let sub = sub.trim().trim_matches('/');
            if sub.is_empty() {
                sidecar_name.clone()
            } else {
                format!("{sub}/{sidecar_name}")
            }
        }
        None => sidecar_name.clone(),
    };

    let full_content_path = format!("{src_docs_base}/{rel_content_path}");
    let full_sidecar_path = format!("{src_docs_base}/{rel_sidecar_path}");

    // Reject duplicates by checking the manifest index (the authoritative membership record).
    let existing_index = manifest.source_document_index.as_deref().unwrap_or(&[]);
    if existing_index
        .iter()
        .any(|e| e.content_path == rel_content_path)
    {
        return Err(RepositoryError::InvalidInput {
            message: format!("file already exists in repository: {full_content_path}"),
        });
    }

    let content_checksum = sha256_hex(&input.content);

    let content_type = input
        .content_type
        .as_deref()
        .map(|s| s.to_string())
        .unwrap_or_else(|| infer_content_type(&file_name).to_string());

    let document_id = uuid::Uuid::new_v4().to_string();
    let sidecar_value = serde_json::json!({
        "documentId": document_id,
        "contentPath": rel_content_path,
        "contentType": content_type,
        "encoding": "binary",
        "checksum": content_checksum,
    });
    let sidecar_bytes =
        serde_json::to_vec_pretty(&sidecar_value).map_err(|e| RepositoryError::InvalidInput {
            message: format!("failed to serialize sidecar: {e}"),
        })?;
    let sidecar_str =
        String::from_utf8(sidecar_bytes.clone()).map_err(|e| RepositoryError::InvalidInput {
            message: format!("sidecar is not valid UTF-8: {e}"),
        })?;
    let sidecar_checksum = sha256_hex(&sidecar_bytes);

    // ADR-007: write content file first, then sidecar, then update index.
    store.save_binary_file(&full_content_path, &input.content)?;
    store.save_text_file(&full_sidecar_path, &sidecar_str)?;

    let mut manifest = store.load_manifest()?;
    let new_entry = SourceDocumentIndexEntry {
        document_id: document_id.clone(),
        sidecar_path: rel_sidecar_path.clone(),
        content_path: rel_content_path.clone(),
        title: input.title,
        sidecar_checksum: Some(sidecar_checksum.clone()),
        content_checksum: Some(content_checksum.clone()),
    };
    let mut index = manifest.source_document_index.take().unwrap_or_default();
    index.push(new_entry);
    manifest.source_document_index = Some(index);
    if manifest.source_documents_path.is_none() {
        manifest.source_documents_path = Some(src_docs_base.clone());
    }
    write_manifest(store, &manifest)?;

    Ok(AddAttachmentResult {
        document_id,
        content_path: rel_content_path,
        sidecar_path: rel_sidecar_path,
        source_documents_path: src_docs_base,
        content_checksum,
        sidecar_checksum,
    })
}

// ── link_attachment ─────────────────────────────────────────────────────────────

/// Input for `link_attachment`.
pub struct LinkAttachmentInput {
    pub instance_id: String,
    pub document_id: String,
}

/// Result for `link_attachment`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkAttachmentResult {
    pub instance_id: String,
    pub document_id: String,
    /// Total number of sourceRefs on the record after the link was added.
    pub source_refs_count: usize,
}

/// Append `sourceType:"repository-document"`, `sourceId:<documentId>`,
/// `sourceRole:"attaches"` to a record's `sourceRefs[]`.
///
/// Validation:
/// - `document_id` MUST resolve in `manifest.sourceDocumentIndex` → `InvalidInput` if absent.
/// - Duplicate link (same record + doc with `sourceRole: Attaches`) → `InvalidInput`.
/// - Record with `instance_id` not found → `NotFound`.
///
/// Write order (ADR-007): record body written before no manifest update is needed
/// (sourceRefs are embedded in the record JSON, not the manifest index).
pub fn link_attachment(
    store: &dyn RepositoryStore,
    input: LinkAttachmentInput,
) -> Result<LinkAttachmentResult, RepositoryError> {
    // 1. Validate document_id in source-document index.
    let manifest = store.load_manifest()?;
    let index = manifest.source_document_index.as_deref().unwrap_or(&[]);
    if !index.iter().any(|e| e.document_id == input.document_id) {
        return Err(RepositoryError::InvalidInput {
            message: format!(
                "document '{}' not found in source-document index",
                input.document_id
            ),
        });
    }

    // 2. Append the new SourceReference (path-encapsulated via record_store).
    // NotFound if record absent; duplicate check (same source_type + source_id + source_role)
    // is handled atomically inside append_source_ref to avoid TOCTOU (ADR-034).
    let new_ref = SourceReference {
        source_type: SourceType::RepositoryDocument,
        source_id: input.document_id.clone(),
        source_standard: None,
        stream_id: None,
        source_role: Some(SourceRole::Attaches),
        relation_type: None,
        confidence: None,
        note: None,
    };
    let updated = record_store::append_source_ref(store, &input.instance_id, new_ref, true)?;

    let source_refs_count = updated
        .extra
        .get("sourceRefs")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    Ok(LinkAttachmentResult {
        instance_id: input.instance_id,
        document_id: input.document_id,
        source_refs_count,
    })
}

// ── resolve_document_view_attachments ──────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveDocumentViewAttachmentsInput {
    pub instance_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedAttachment {
    pub document_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidecar_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_checksum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidecar_checksum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordAttachments {
    pub instance_id: String,
    pub attachments: Vec<ResolvedAttachment>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveDocumentViewAttachmentsResult {
    pub source_documents_path: String,
    pub records: Vec<RecordAttachments>,
}

/// Given a list of instance IDs from a rendered document_view, resolve each record's
/// `sourceRefs` with `sourceRole: attaches` and `sourceType: repository-document`
/// to their `SourceDocumentIndexEntry` metadata.
///
/// Resolution is sourceRefs-only per RFC-017 Rev 3 [R1].
/// Records with no qualifying sourceRefs are omitted from the result.
pub fn resolve_document_view_attachments(
    store: &dyn RepositoryStore,
    input: ResolveDocumentViewAttachmentsInput,
) -> Result<ResolveDocumentViewAttachmentsResult, RepositoryError> {
    let manifest = store.load_manifest()?;

    let src_docs_base = manifest
        .source_documents_path
        .as_deref()
        .unwrap_or("source-documents")
        .to_string();

    let index_entries = manifest.source_document_index.as_deref().unwrap_or(&[]);
    let index_map: HashMap<&str, &SourceDocumentIndexEntry> = index_entries
        .iter()
        .map(|e| (e.document_id.as_str(), e))
        .collect();

    let instance_map: HashMap<&str, &InstanceIndexEntry> = manifest
        .instance_index
        .iter()
        .filter(|e| e.tier() == 2)
        .map(|e| (e.instance_id(), e))
        .collect();

    let mut records: Vec<RecordAttachments> = Vec::new();

    for instance_id in &input.instance_ids {
        let Some(entry) = instance_map.get(instance_id.as_str()) else {
            continue;
        };

        let record = crate::record_store::load_record(store, entry.path())?;

        let source_refs: Vec<SourceReference> = match record.extra.get("sourceRefs") {
            None => vec![],
            Some(v) => {
                serde_json::from_value(v.clone()).map_err(|e| RepositoryError::Serialize {
                    path: std::path::PathBuf::from(entry.path()),
                    source: e,
                })?
            }
        };

        let attachments: Vec<ResolvedAttachment> = source_refs
            .into_iter()
            .filter(|r| {
                r.source_role == Some(SourceRole::Attaches)
                    && r.source_type == SourceType::RepositoryDocument
            })
            .map(|r| {
                let idx = index_map.get(r.source_id.as_str());
                ResolvedAttachment {
                    document_id: r.source_id,
                    content_path: idx.map(|e| e.content_path.clone()),
                    sidecar_path: idx.map(|e| e.sidecar_path.clone()),
                    title: idx.and_then(|e| e.title.clone()),
                    content_checksum: idx.and_then(|e| e.content_checksum.clone()),
                    sidecar_checksum: idx.and_then(|e| e.sidecar_checksum.clone()),
                }
            })
            .collect();

        if !attachments.is_empty() {
            records.push(RecordAttachments {
                instance_id: instance_id.clone(),
                attachments,
            });
        }
    }

    Ok(ResolveDocumentViewAttachmentsResult {
        source_documents_path: src_docs_base,
        records,
    })
}

// ── get_record_attachments ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GetRecordAttachmentsInput {
    pub instance_id: String,
}

/// Single-record analog to `RecordAttachments`; carries `source_documents_path` inline
/// because the multi-record path (`ResolveDocumentViewAttachmentsResult`) surfaces it at
/// the top level instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetRecordAttachmentsResult {
    pub instance_id: String,
    pub source_documents_path: String,
    pub attachments: Vec<ResolvedAttachment>,
}

/// Return the `sourceRole: attaches` + `sourceType: repository-document` refs for a
/// single record, resolved against the source document index.
///
/// Returns `Ok(None)` when the instance ID is not in the manifest index.
/// Searches all tiers (not restricted to tier-2).
///
/// sourceRefs are stored in `record.extra["sourceRefs"]` (ADR-034); deserialization
/// failures surface as `RepositoryError::Serialize`.
pub fn get_record_attachments(
    store: &dyn RepositoryStore,
    input: GetRecordAttachmentsInput,
) -> Result<Option<GetRecordAttachmentsResult>, RepositoryError> {
    let manifest = store.load_manifest()?;

    let src_docs_base = manifest
        .source_documents_path
        .as_deref()
        .unwrap_or("source-documents")
        .to_string();

    let index_entries = manifest.source_document_index.as_deref().unwrap_or(&[]);
    let index_map: HashMap<&str, &SourceDocumentIndexEntry> = index_entries
        .iter()
        .map(|e| (e.document_id.as_str(), e))
        .collect();

    let record = match record_store::get_record_by_id(store, &input.instance_id)? {
        Some(r) => r,
        None => return Ok(None),
    };

    let source_refs: Vec<SourceReference> = match record.extra.get("sourceRefs") {
        None => vec![],
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| RepositoryError::Serialize {
            path: std::path::PathBuf::from(&input.instance_id),
            source: e,
        })?,
    };

    let attachments: Vec<ResolvedAttachment> = source_refs
        .into_iter()
        .filter(|r| {
            r.source_role == Some(SourceRole::Attaches)
                && r.source_type == SourceType::RepositoryDocument
        })
        .map(|r| {
            let idx = index_map.get(r.source_id.as_str());
            ResolvedAttachment {
                document_id: r.source_id,
                content_path: idx.map(|e| e.content_path.clone()),
                sidecar_path: idx.map(|e| e.sidecar_path.clone()),
                title: idx.and_then(|e| e.title.clone()),
                content_checksum: idx.and_then(|e| e.content_checksum.clone()),
                sidecar_checksum: idx.and_then(|e| e.sidecar_checksum.clone()),
            }
        })
        .collect();

    Ok(Some(GetRecordAttachmentsResult {
        instance_id: input.instance_id,
        source_documents_path: src_docs_base,
        attachments,
    }))
}

// ── list_source_documents ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceDocumentEntry {
    /// Path relative to the source-documents directory (e.g. "spec/srs-spec.md.meta.json").
    /// Matches the convention of SourceDocumentIndexEntry.sidecar_path.
    pub sidecar_path: String,
    pub meta: SourceDocumentMeta,
}

#[derive(Debug, Clone, Default)]
pub struct ListSourceDocumentsFilter {}

/// Enumerate all source-document sidecars in the repository.
///
/// Reads manifest to resolve the configured source-documents path
/// (defaults to "source-documents"). Scans recursively for *.meta.json files,
/// parses each, and returns entries with source-documents-relative sidecar paths.
pub fn list_source_documents(
    store: &dyn RepositoryStore,
    _filter: ListSourceDocumentsFilter,
) -> Result<Vec<SourceDocumentEntry>, RepositoryError> {
    let manifest = store.load_manifest()?;
    let src_docs_base = manifest
        .source_documents_path
        .as_deref()
        .unwrap_or("source-documents")
        .to_string();
    let prefix = format!("{}/", src_docs_base);
    let sidecar_paths: Vec<(String, String)> = store
        .list_files_recursive(&src_docs_base)
        .into_iter()
        .filter(|p| p.ends_with(".meta.json"))
        .filter_map(|repo_rel| {
            repo_rel
                .strip_prefix(&prefix)
                .map(|rel| (repo_rel.clone(), rel.to_string()))
        })
        .collect();
    let mut entries = Vec::with_capacity(sidecar_paths.len());
    for (repo_relative_path, sidecar_path) in sidecar_paths {
        let json_str = store.load_text_file(&repo_relative_path)?;
        let meta = serde_json::from_str::<SourceDocumentMeta>(&json_str).map_err(|source| {
            RepositoryError::SourceDocumentMetaLoad {
                path: std::path::PathBuf::from(&repo_relative_path),
                source,
            }
        })?;
        entries.push(SourceDocumentEntry { sidecar_path, meta });
    }
    Ok(entries)
}

// ── get_attachment_bytes ───────────────────────────────────────────────────────

#[derive(Debug)]
pub struct GetAttachmentBytesInput {
    pub document_id: String,
}

#[derive(Debug)]
pub struct GetAttachmentBytesResult {
    pub document_id: String,
    pub content_path: String,
    pub bytes: Vec<u8>,
}

/// Return the raw bytes of a source-document attachment by `document_id`.
///
/// Errors:
/// - `InvalidInput` if `document_id` is not in `manifest.sourceDocumentIndex`
/// - `Io(NotFound)` if the binary file is absent (tombstone state per RFC-017)
pub fn get_attachment_bytes(
    store: &dyn RepositoryStore,
    input: GetAttachmentBytesInput,
) -> Result<GetAttachmentBytesResult, RepositoryError> {
    let manifest = store.load_manifest()?;
    let src_docs_base = manifest
        .source_documents_path
        .as_deref()
        .unwrap_or("source-documents");
    let index = manifest.source_document_index.as_deref().unwrap_or(&[]);

    let entry = index
        .iter()
        .find(|e| e.document_id == input.document_id)
        .ok_or_else(|| RepositoryError::InvalidInput {
            message: format!(
                "document '{}' not found in source-document index",
                input.document_id
            ),
        })?;

    let full_path = format!("{}/{}", src_docs_base, entry.content_path);
    let bytes = store.load_binary_file(&full_path)?;

    Ok(GetAttachmentBytesResult {
        document_id: input.document_id,
        content_path: entry.content_path.clone(),
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::store::memory::MemoryStore;
    use std::path::PathBuf;

    fn test_package() -> Package {
        Package {
            id: "test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        }
    }

    fn store_with_manifest(manifest: Manifest) -> MemoryStore {
        MemoryStore::new(manifest, test_package())
    }

    fn manifest_with_index(
        source_documents_path: Option<String>,
        entries: Vec<SourceDocumentIndexEntry>,
    ) -> Manifest {
        Manifest {
            source_documents_path,
            source_document_index: Some(entries),
            ..Manifest::default()
        }
    }

    // MemoryStore::list_files_recursive only scans the text-data store.
    // In tests, use save_text_file (empty string suffices) to register a file path.
    fn touch(store: &MemoryStore, path: &str) {
        store.save_text_file(path, "").unwrap();
    }

    #[test]
    fn list_attachments_empty_store() {
        let store = store_with_manifest(Manifest::default());
        let result = list_attachments(&store, ListAttachmentsFilter::default()).unwrap();
        assert_eq!(result.source_documents_path, "source-documents");
        assert!(result.entries.is_empty());
    }

    #[test]
    fn list_attachments_indexed_file() {
        let store = store_with_manifest(manifest_with_index(
            None,
            vec![SourceDocumentIndexEntry {
                document_id: "doc-uuid-1".to_string(),
                sidecar_path: "my-doc.meta.json".to_string(),
                content_path: "my-doc.pdf".to_string(),
                title: Some("My Document".to_string()),
                sidecar_checksum: Some("sha256:aaa".to_string()),
                content_checksum: Some("sha256:bbb".to_string()),
            }],
        ));
        touch(&store, "source-documents/my-doc.pdf");
        touch(&store, "source-documents/my-doc.meta.json");

        let result = list_attachments(&store, ListAttachmentsFilter::default()).unwrap();
        assert_eq!(result.entries.len(), 1);
        let e = &result.entries[0];
        assert_eq!(e.path, "my-doc.pdf");
        assert_eq!(e.document_id.as_deref(), Some("doc-uuid-1"));
        assert_eq!(e.title.as_deref(), Some("My Document"));
        assert_eq!(e.content_checksum.as_deref(), Some("sha256:bbb"));
        assert_eq!(e.sidecar_checksum.as_deref(), Some("sha256:aaa"));
        // MemoryStore: touch writes to `data`, load_binary_file reads `binary_data` → None
        assert!(e.size_bytes.is_none());
    }

    #[test]
    fn list_attachments_unindexed_file() {
        let store = store_with_manifest(Manifest::default());
        touch(&store, "source-documents/unknown.docx");

        let result = list_attachments(&store, ListAttachmentsFilter::default()).unwrap();
        assert_eq!(result.entries.len(), 1);
        let e = &result.entries[0];
        assert_eq!(e.path, "unknown.docx");
        assert!(e.document_id.is_none());
        assert!(e.title.is_none());
        assert!(e.content_checksum.is_none());
        // MemoryStore: touch writes to `data`, load_binary_file reads `binary_data` → None
        assert!(e.size_bytes.is_none());
    }

    #[test]
    fn list_attachments_walks_subdirs() {
        let store = store_with_manifest(Manifest::default());
        touch(&store, "source-documents/subdir/nested.pdf");
        touch(&store, "source-documents/top.pdf");

        let result = list_attachments(&store, ListAttachmentsFilter::default()).unwrap();
        assert_eq!(result.entries.len(), 2);
        let paths: Vec<&str> = result.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(
            paths.contains(&"subdir/nested.pdf"),
            "expected subdir/nested.pdf, got {paths:?}"
        );
        assert!(
            paths.contains(&"top.pdf"),
            "expected top.pdf, got {paths:?}"
        );
    }

    #[test]
    fn list_attachments_excludes_sidecars() {
        let store = store_with_manifest(Manifest::default());
        touch(&store, "source-documents/doc.pdf");
        touch(&store, "source-documents/doc.meta.json");

        let result = list_attachments(&store, ListAttachmentsFilter::default()).unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].path, "doc.pdf");
    }

    #[test]
    fn list_attachments_custom_path() {
        let manifest = Manifest {
            source_documents_path: Some("attachments".to_string()),
            ..Manifest::default()
        };
        let store = store_with_manifest(manifest);
        touch(&store, "attachments/report.pdf");

        let result = list_attachments(&store, ListAttachmentsFilter::default()).unwrap();
        assert_eq!(result.source_documents_path, "attachments");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].path, "report.pdf");
    }

    // Cross-store roundtrip: MemoryStore setup → FileStore exercised (CLAUDE.md requirement).
    #[test]
    fn list_attachments_filestore_roundtrip() {
        use crate::store::FileStore;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Minimal repo scaffold.
        std::fs::create_dir_all(root.join(".srs")).unwrap();
        let manifest_json = serde_json::json!({
            "instanceIndex": [],
            "sourceDocumentsPath": "source-documents",
            "sourceDocumentIndex": [
                {
                    "documentId": "roundtrip-uuid",
                    "sidecarPath": "brief.meta.json",
                    "contentPath": "brief.pdf",
                    "title": "Roundtrip Brief",
                    "contentChecksum": "sha256:ccc"
                }
            ]
        });
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string(&manifest_json).unwrap(),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("package")).unwrap();
        std::fs::write(
            root.join("package/package.json"),
            serde_json::json!({
                "id": "rt-pkg", "namespace": "com.test", "name": "test",
                "version": "1.0.0", "fields": [], "types": []
            })
            .to_string(),
        )
        .unwrap();
        // Write a real file (content) and sidecar.
        std::fs::create_dir_all(root.join("source-documents")).unwrap();
        std::fs::write(root.join("source-documents/brief.pdf"), b"pdf bytes").unwrap();
        std::fs::write(
            root.join("source-documents/brief.meta.json"),
            r#"{"documentId":"roundtrip-uuid"}"#,
        )
        .unwrap();
        // Write a file in a subdirectory (not in index).
        std::fs::create_dir_all(root.join("source-documents/annexes")).unwrap();
        std::fs::write(root.join("source-documents/annexes/annex-a.pdf"), b"annex").unwrap();

        let store = FileStore::new(root);
        let result = list_attachments(&store, ListAttachmentsFilter::default()).unwrap();

        assert_eq!(result.source_documents_path, "source-documents");
        let paths: Vec<&str> = result.entries.iter().map(|e| e.path.as_str()).collect();
        // content file is indexed → metadata populated
        let brief = result
            .entries
            .iter()
            .find(|e| e.path == "brief.pdf")
            .unwrap();
        assert_eq!(brief.document_id.as_deref(), Some("roundtrip-uuid"));
        assert_eq!(brief.title.as_deref(), Some("Roundtrip Brief"));
        // FileStore: size_bytes from fs::metadata ("pdf bytes" = 9 bytes)
        assert_eq!(brief.size_bytes, Some(9));
        // subdirectory file is present but not indexed
        assert!(
            paths.contains(&"annexes/annex-a.pdf"),
            "expected annexes/annex-a.pdf, got {paths:?}"
        );
        let annex = result
            .entries
            .iter()
            .find(|e| e.path == "annexes/annex-a.pdf")
            .unwrap();
        assert!(annex.document_id.is_none());
        // FileStore: size_bytes from fs::metadata ("annex" = 5 bytes)
        assert_eq!(annex.size_bytes, Some(5));
        // sidecar excluded
        assert!(!paths.contains(&"brief.meta.json"));
    }

    // ── add_attachment tests ───────────────────────────────────────────────────

    fn empty_store() -> MemoryStore {
        store_with_manifest(Manifest::default())
    }

    #[test]
    fn add_attachment_happy_path() {
        let store = empty_store();
        let result = add_attachment(
            &store,
            AddAttachmentInput {
                file_name: "report.pdf".to_string(),
                content: b"PDF content".to_vec(),
                subdir: None,
                title: Some("Annual Report".to_string()),
                content_type: None,
            },
        )
        .unwrap();

        assert!(!result.document_id.is_empty());
        assert_eq!(result.content_path, "report.pdf");
        assert_eq!(result.sidecar_path, "report.meta.json");
        assert_eq!(result.source_documents_path, "source-documents");
        assert!(result.content_checksum.starts_with("sha256:"));
        assert!(result.sidecar_checksum.starts_with("sha256:"));
        assert_ne!(result.content_checksum, result.sidecar_checksum);
    }

    #[test]
    fn add_attachment_sets_manifest_index() {
        let store = empty_store();
        let result = add_attachment(
            &store,
            AddAttachmentInput {
                file_name: "brief.pdf".to_string(),
                content: b"bytes".to_vec(),
                subdir: None,
                title: Some("Brief".to_string()),
                content_type: None,
            },
        )
        .unwrap();

        let manifest = store.load_manifest().unwrap();
        let index = manifest.source_document_index.as_deref().unwrap();
        assert_eq!(index.len(), 1);
        let entry = &index[0];
        assert_eq!(entry.document_id, result.document_id);
        assert_eq!(entry.content_path, "brief.pdf");
        assert_eq!(entry.sidecar_path, "brief.meta.json");
        assert_eq!(entry.title.as_deref(), Some("Brief"));
        assert_eq!(entry.content_checksum, Some(result.content_checksum));
        assert_eq!(entry.sidecar_checksum, Some(result.sidecar_checksum));
    }

    #[test]
    fn add_attachment_with_subdir() {
        let store = empty_store();
        let result = add_attachment(
            &store,
            AddAttachmentInput {
                file_name: "annex.pdf".to_string(),
                content: b"annex bytes".to_vec(),
                subdir: Some("annexes".to_string()),
                title: None,
                content_type: None,
            },
        )
        .unwrap();

        assert_eq!(result.content_path, "annexes/annex.pdf");
        assert_eq!(result.sidecar_path, "annexes/annex.meta.json");

        // Confirm the manifest index was updated with the correct paths.
        let manifest = store.load_manifest().unwrap();
        let index = manifest.source_document_index.as_deref().unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].content_path, "annexes/annex.pdf");
        assert_eq!(index[0].sidecar_path, "annexes/annex.meta.json");
    }

    #[test]
    fn add_attachment_duplicate_rejected() {
        let store = empty_store();
        let input = || AddAttachmentInput {
            file_name: "doc.pdf".to_string(),
            content: b"data".to_vec(),
            subdir: None,
            title: None,
            content_type: None,
        };
        add_attachment(&store, input()).unwrap();
        let err = add_attachment(&store, input()).unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidInput { .. }),
            "expected InvalidInput, got: {err:?}"
        );
    }

    #[test]
    fn add_attachment_infers_content_type_pdf() {
        let store = empty_store();
        add_attachment(
            &store,
            AddAttachmentInput {
                file_name: "doc.pdf".to_string(),
                content: b"pdf".to_vec(),
                subdir: None,
                title: None,
                content_type: None,
            },
        )
        .unwrap();

        // Check the sidecar contains the correct contentType.
        let sidecar_str = store
            .load_text_file("source-documents/doc.meta.json")
            .unwrap();
        let sidecar: serde_json::Value = serde_json::from_str(&sidecar_str).unwrap();
        assert_eq!(
            sidecar["contentType"].as_str(),
            Some("application/pdf"),
            "expected PDF MIME type"
        );
    }

    #[test]
    fn add_attachment_explicit_content_type() {
        let store = empty_store();
        add_attachment(
            &store,
            AddAttachmentInput {
                file_name: "data.bin".to_string(),
                content: b"raw".to_vec(),
                subdir: None,
                title: None,
                content_type: Some("application/custom".to_string()),
            },
        )
        .unwrap();

        let sidecar_str = store
            .load_text_file("source-documents/data.meta.json")
            .unwrap();
        let sidecar: serde_json::Value = serde_json::from_str(&sidecar_str).unwrap();
        assert_eq!(
            sidecar["contentType"].as_str(),
            Some("application/custom"),
            "explicit content_type should not be overridden"
        );
    }

    // ── link_attachment tests ───────────────────────────────────────────────────

    use crate::index::InstanceIndexEntry;

    /// Build a MemoryStore that contains one source-document index entry and one
    /// tier-2 record at "records/tier-2/test-record-aaaabbbb.json".
    fn store_with_doc_and_record(doc_id: &str, record_id: &str) -> MemoryStore {
        let manifest = Manifest {
            source_document_index: Some(vec![SourceDocumentIndexEntry {
                document_id: doc_id.to_string(),
                sidecar_path: "brief.meta.json".to_string(),
                content_path: "brief.pdf".to_string(),
                title: None,
                sidecar_checksum: None,
                content_checksum: None,
            }]),
            instance_index: vec![InstanceIndexEntry {
                instance_id: record_id.to_string(),
                tier: 2,
                path: format!("records/tier-2/test-record-{}.json", &record_id[..8]),
                title: None,
                tags: None,
            }],
            ..Manifest::default()
        };
        let store = store_with_manifest(manifest);
        let record_path = format!("records/tier-2/test-record-{}.json", &record_id[..8]);
        store
            .save_instance_json(
                &record_path,
                &serde_json::json!({
                    "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                    "instanceId": record_id,
                    "typeId": "type-test-001",
                    "typeVersion": 1,
                    "typeNamespace": "com.test",
                    "typeName": "test-type",
                    "fieldValues": []
                }),
            )
            .unwrap();
        store
    }

    #[test]
    fn link_attachment_happy_path() {
        let doc_id = "doc-aaa-111";
        let record_id = "aaaabbbb-0000-4000-8000-000000000001";
        let store = store_with_doc_and_record(doc_id, record_id);

        let result = link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: record_id.to_string(),
                document_id: doc_id.to_string(),
            },
        )
        .unwrap();

        assert_eq!(result.instance_id, record_id);
        assert_eq!(result.document_id, doc_id);
        assert_eq!(result.source_refs_count, 1);

        // Verify the record JSON on disk has sourceRefs[0].sourceRole == "attaches"
        let record_path = format!("records/tier-2/test-record-{}.json", &record_id[..8]);
        let val = store.load_instance_json(&record_path).unwrap();
        assert_eq!(
            val["sourceRefs"][0]["sourceRole"],
            serde_json::json!("attaches")
        );
        assert_eq!(val["sourceRefs"][0]["sourceId"], serde_json::json!(doc_id));
        assert_eq!(
            val["sourceRefs"][0]["sourceType"],
            serde_json::json!("repository-document")
        );
        assert!(
            val["sourceRefs"][0].get("relationType").is_none(),
            "relationType must not be emitted (RFC-023)"
        );
    }

    #[test]
    fn link_attachment_duplicate_rejected() {
        let doc_id = "doc-bbb-222";
        let record_id = "bbbbcccc-0000-4000-8000-000000000002";
        let store = store_with_doc_and_record(doc_id, record_id);

        link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: record_id.to_string(),
                document_id: doc_id.to_string(),
            },
        )
        .unwrap();

        let err = link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: record_id.to_string(),
                document_id: doc_id.to_string(),
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidInput { .. }),
            "expected InvalidInput for duplicate, got: {err:?}"
        );
    }

    #[test]
    fn link_attachment_unknown_doc_rejected() {
        let record_id = "ccccdddd-0000-4000-8000-000000000003";
        let store = store_with_doc_and_record("real-doc", record_id);

        let err = link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: record_id.to_string(),
                document_id: "nonexistent-doc".to_string(),
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidInput { .. }),
            "expected InvalidInput for unknown doc, got: {err:?}"
        );
    }

    #[test]
    fn link_attachment_unknown_record_rejected() {
        let doc_id = "doc-ccc-333";
        // Store has the doc in the index but no record with this ID
        let manifest = Manifest {
            source_document_index: Some(vec![SourceDocumentIndexEntry {
                document_id: doc_id.to_string(),
                sidecar_path: "f.meta.json".to_string(),
                content_path: "f.pdf".to_string(),
                title: None,
                sidecar_checksum: None,
                content_checksum: None,
            }]),
            ..Manifest::default()
        };
        let store = store_with_manifest(manifest);

        let err = link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: "no-such-record-00000000".to_string(),
                document_id: doc_id.to_string(),
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::NotFound { .. }),
            "expected NotFound for unknown record, got: {err:?}"
        );
    }

    #[test]
    fn link_attachment_preserves_existing_refs() {
        let doc_id_1 = "doc-first-111";
        let doc_id_2 = "doc-second-222";
        let record_id = "ddddeeee-0000-4000-8000-000000000004";

        let manifest = Manifest {
            source_document_index: Some(vec![
                SourceDocumentIndexEntry {
                    document_id: doc_id_1.to_string(),
                    sidecar_path: "a.meta.json".to_string(),
                    content_path: "a.pdf".to_string(),
                    title: None,
                    sidecar_checksum: None,
                    content_checksum: None,
                },
                SourceDocumentIndexEntry {
                    document_id: doc_id_2.to_string(),
                    sidecar_path: "b.meta.json".to_string(),
                    content_path: "b.pdf".to_string(),
                    title: None,
                    sidecar_checksum: None,
                    content_checksum: None,
                },
            ]),
            instance_index: vec![InstanceIndexEntry {
                instance_id: record_id.to_string(),
                tier: 2,
                path: format!("records/tier-2/test-record-{}.json", &record_id[..8]),
                title: None,
                tags: None,
            }],
            ..Manifest::default()
        };
        let store = store_with_manifest(manifest);
        let record_path = format!("records/tier-2/test-record-{}.json", &record_id[..8]);
        store
            .save_instance_json(
                &record_path,
                &serde_json::json!({
                    "instanceId": record_id,
                    "typeId": "type-test-001",
                    "typeVersion": 1,
                    "typeNamespace": "com.test",
                    "typeName": "test-type",
                    "fieldValues": []
                }),
            )
            .unwrap();

        link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: record_id.to_string(),
                document_id: doc_id_1.to_string(),
            },
        )
        .unwrap();

        let result = link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: record_id.to_string(),
                document_id: doc_id_2.to_string(),
            },
        )
        .unwrap();

        assert_eq!(result.source_refs_count, 2, "both refs should be present");
    }

    #[test]
    fn link_attachment_filestore_roundtrip() {
        use crate::store::FileStore;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let root = temp.path();

        std::fs::create_dir_all(root.join(".srs")).unwrap();
        let doc_id = "doc-roundtrip-001";
        let manifest_json = serde_json::json!({
            "instanceIndex": [{
                "instanceId": "ffffffff-0000-4000-8000-000000000001",
                "tier": 2,
                "path": "records/tier-2/test-type-ffffffff.json"
            }],
            "sourceDocumentIndex": [{
                "documentId": doc_id,
                "sidecarPath": "brief.meta.json",
                "contentPath": "brief.pdf"
            }]
        });
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string(&manifest_json).unwrap(),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("package")).unwrap();
        std::fs::write(
            root.join("package/package.json"),
            serde_json::json!({
                "id": "link-rt-pkg", "namespace": "com.test", "name": "test",
                "version": "1.0.0", "fields": [], "types": []
            })
            .to_string(),
        )
        .unwrap();

        std::fs::create_dir_all(root.join("records/tier-2")).unwrap();
        let record_id = "ffffffff-0000-4000-8000-000000000001";
        std::fs::write(
            root.join("records/tier-2/test-type-ffffffff.json"),
            serde_json::json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": record_id,
                "typeId": "type-test-001",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "test-type",
                "fieldValues": []
            })
            .to_string(),
        )
        .unwrap();

        let store = FileStore::new(root);
        let result = link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: record_id.to_string(),
                document_id: doc_id.to_string(),
            },
        )
        .unwrap();

        assert_eq!(result.source_refs_count, 1);

        // Reload and verify sourceRefs[0].sourceRole == "attaches"
        let val =
            std::fs::read_to_string(root.join("records/tier-2/test-type-ffffffff.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&val).unwrap();
        assert_eq!(
            parsed["sourceRefs"][0]["sourceRole"],
            serde_json::json!("attaches")
        );
        assert_eq!(
            parsed["sourceRefs"][0]["sourceId"],
            serde_json::json!(doc_id)
        );
        assert!(
            parsed["sourceRefs"][0].get("relationType").is_none(),
            "relationType must not be emitted (RFC-023)"
        );
    }

    #[test]
    fn add_attachment_filestore_roundtrip() {
        use crate::store::FileStore;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let root = temp.path();

        std::fs::create_dir_all(root.join(".srs")).unwrap();
        std::fs::write(
            root.join("manifest.json"),
            serde_json::json!({"instanceIndex": []}).to_string(),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("package")).unwrap();
        std::fs::write(
            root.join("package/package.json"),
            serde_json::json!({
                "id": "rt2-pkg", "namespace": "com.test", "name": "test",
                "version": "1.0.0", "fields": [], "types": []
            })
            .to_string(),
        )
        .unwrap();

        let store = FileStore::new(root);
        let result = add_attachment(
            &store,
            AddAttachmentInput {
                file_name: "evidence.pdf".to_string(),
                content: b"evidence content".to_vec(),
                subdir: Some("decisions".to_string()),
                title: Some("Key Evidence".to_string()),
                content_type: None,
            },
        )
        .unwrap();

        assert_eq!(result.content_path, "decisions/evidence.pdf");
        assert_eq!(result.sidecar_path, "decisions/evidence.meta.json");

        // Content file is on disk.
        let on_disk = std::fs::read(root.join("source-documents/decisions/evidence.pdf")).unwrap();
        assert_eq!(on_disk, b"evidence content");

        // Sidecar is on disk and parseable.
        let sidecar_str =
            std::fs::read_to_string(root.join("source-documents/decisions/evidence.meta.json"))
                .unwrap();
        let sidecar: serde_json::Value = serde_json::from_str(&sidecar_str).unwrap();
        assert_eq!(sidecar["contentType"].as_str(), Some("application/pdf"));
        assert_eq!(sidecar["encoding"].as_str(), Some("binary"));

        // Manifest index is updated.
        let manifest = store.load_manifest().unwrap();
        let index = manifest.source_document_index.as_deref().unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].document_id, result.document_id);

        // list_attachments confirms the file appears.
        let list = list_attachments(&store, ListAttachmentsFilter::default()).unwrap();
        assert_eq!(list.entries.len(), 1);
        assert_eq!(list.entries[0].path, "decisions/evidence.pdf");
        assert_eq!(
            list.entries[0].document_id.as_deref(),
            Some(result.document_id.as_str())
        );
    }

    // ── list_source_documents tests ───────────────────────────────────────────

    #[test]
    fn list_source_documents_empty() {
        let store = store_with_manifest(Manifest::default());
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn list_source_documents_single() {
        let store = store_with_manifest(Manifest::default());
        store
            .save_text_file(
                "source-documents/my-doc.meta.json",
                r#"{"documentId":"aaaaaaaa-0000-4000-8000-000000000001","contentPath":"my-doc.pdf","contentType":"text/plain"}"#,
            )
            .unwrap();
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].sidecar_path, "my-doc.meta.json");
        assert_eq!(
            result[0].meta.document_id,
            "aaaaaaaa-0000-4000-8000-000000000001"
        );
    }

    #[test]
    fn list_source_documents_subdirectory() {
        let store = store_with_manifest(Manifest::default());
        store
            .save_text_file(
                "source-documents/sub/a.meta.json",
                r#"{"documentId":"aaaaaaaa-0000-4000-8000-000000000002","contentPath":"sub/a.pdf","contentType":"text/plain"}"#,
            )
            .unwrap();
        store
            .save_text_file(
                "source-documents/sub/b.meta.json",
                r#"{"documentId":"aaaaaaaa-0000-4000-8000-000000000003","contentPath":"sub/b.pdf","contentType":"text/plain"}"#,
            )
            .unwrap();
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert_eq!(result.len(), 2);
        let paths: Vec<&str> = result.iter().map(|e| e.sidecar_path.as_str()).collect();
        assert!(
            paths.contains(&"sub/a.meta.json"),
            "expected sub/a.meta.json, got {paths:?}"
        );
        assert!(
            paths.contains(&"sub/b.meta.json"),
            "expected sub/b.meta.json, got {paths:?}"
        );
    }

    #[test]
    fn list_source_documents_malformed_returns_err() {
        let store = store_with_manifest(Manifest::default());
        store
            .save_text_file("source-documents/bad.meta.json", "not valid json")
            .unwrap();
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default());
        assert!(
            matches!(result, Err(RepositoryError::SourceDocumentMetaLoad { .. })),
            "expected SourceDocumentMetaLoad error, got {result:?}"
        );
    }

    #[test]
    fn file_store_list_source_documents_spec_repo() {
        use crate::store::FileStore;
        let repo_root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/spec-repo");
        let store = FileStore::new(&repo_root);
        let result = list_source_documents(&store, ListSourceDocumentsFilter::default()).unwrap();
        assert_eq!(
            result.len(),
            4,
            "expected 4 sidecars, got {:?}",
            result.iter().map(|e| &e.sidecar_path).collect::<Vec<_>>()
        );
        for entry in &result {
            assert!(
                !entry.meta.document_id.is_empty(),
                "document_id missing in {}",
                entry.sidecar_path
            );
            assert!(
                !entry.meta.content_type.is_empty(),
                "content_type missing in {}",
                entry.sidecar_path
            );
        }
    }

    // ── resolve_document_view_attachments tests ───────────────────────────────────

    #[test]
    fn resolve_document_view_attachments_empty_ids() {
        let store = store_with_manifest(Manifest::default());
        let result = resolve_document_view_attachments(
            &store,
            ResolveDocumentViewAttachmentsInput {
                instance_ids: vec![],
            },
        )
        .unwrap();
        assert!(result.records.is_empty());
        assert_eq!(result.source_documents_path, "source-documents");
    }

    #[test]
    fn resolve_document_view_attachments_no_source_refs() {
        let doc_id = "doc-no-refs-001";
        let record_id = "aaaa0001-0000-4000-8000-000000000001";
        let store = store_with_doc_and_record(doc_id, record_id);
        let result = resolve_document_view_attachments(
            &store,
            ResolveDocumentViewAttachmentsInput {
                instance_ids: vec![record_id.to_string()],
            },
        )
        .unwrap();
        assert!(
            result.records.is_empty(),
            "record with no sourceRefs must not appear"
        );
    }

    #[test]
    fn resolve_document_view_attachments_wrong_role() {
        let doc_id = "doc-wrong-role-002";
        let record_id = "aaaa0002-0000-4000-8000-000000000002";
        let store = store_with_doc_and_record(doc_id, record_id);
        let record_path = format!("records/tier-2/test-record-{}.json", &record_id[..8]);
        store
            .save_instance_json(
                &record_path,
                &serde_json::json!({
                    "instanceId": record_id,
                    "typeId": "type-test-001",
                    "typeVersion": 1,
                    "typeNamespace": "com.test",
                    "typeName": "test-type",
                    "fieldValues": [],
                    "sourceRefs": [{
                        "sourceType": "repository-document",
                        "sourceId": doc_id,
                        "sourceRole": "evidence"
                    }]
                }),
            )
            .unwrap();
        let result = resolve_document_view_attachments(
            &store,
            ResolveDocumentViewAttachmentsInput {
                instance_ids: vec![record_id.to_string()],
            },
        )
        .unwrap();
        assert!(
            result.records.is_empty(),
            "sourceRole:evidence must not be included"
        );
    }

    #[test]
    fn resolve_document_view_attachments_happy_path() {
        let doc_id = "doc-happy-003";
        let record_id = "aaaa0003-0000-4000-8000-000000000003";
        let manifest = Manifest {
            source_document_index: Some(vec![SourceDocumentIndexEntry {
                document_id: doc_id.to_string(),
                sidecar_path: "brief.meta.json".to_string(),
                content_path: "brief.pdf".to_string(),
                title: Some("Brief Title".to_string()),
                sidecar_checksum: Some("sha256:sidecar".to_string()),
                content_checksum: Some("sha256:content".to_string()),
            }]),
            instance_index: vec![InstanceIndexEntry {
                instance_id: record_id.to_string(),
                tier: 2,
                path: format!("records/tier-2/test-record-{}.json", &record_id[..8]),
                title: None,
                tags: None,
            }],
            ..Manifest::default()
        };
        let store = store_with_manifest(manifest);
        let record_path = format!("records/tier-2/test-record-{}.json", &record_id[..8]);
        store
            .save_instance_json(
                &record_path,
                &serde_json::json!({
                    "instanceId": record_id,
                    "typeId": "type-test-001",
                    "typeVersion": 1,
                    "typeNamespace": "com.test",
                    "typeName": "test-type",
                    "fieldValues": []
                }),
            )
            .unwrap();
        link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: record_id.to_string(),
                document_id: doc_id.to_string(),
            },
        )
        .unwrap();

        let result = resolve_document_view_attachments(
            &store,
            ResolveDocumentViewAttachmentsInput {
                instance_ids: vec![record_id.to_string()],
            },
        )
        .unwrap();

        assert_eq!(result.records.len(), 1);
        let rec = &result.records[0];
        assert_eq!(rec.instance_id, record_id);
        assert_eq!(rec.attachments.len(), 1);
        let att = &rec.attachments[0];
        assert_eq!(att.document_id, doc_id);
        assert_eq!(att.content_path.as_deref(), Some("brief.pdf"));
        assert_eq!(att.sidecar_path.as_deref(), Some("brief.meta.json"));
        assert_eq!(att.title.as_deref(), Some("Brief Title"));
        assert_eq!(att.content_checksum.as_deref(), Some("sha256:content"));
        assert_eq!(att.sidecar_checksum.as_deref(), Some("sha256:sidecar"));
    }

    #[test]
    fn resolve_document_view_attachments_unindexed_doc() {
        let record_id = "aaaa0004-0000-4000-8000-000000000004";
        let manifest = Manifest {
            source_document_index: Some(vec![]),
            instance_index: vec![InstanceIndexEntry {
                instance_id: record_id.to_string(),
                tier: 2,
                path: format!("records/tier-2/test-record-{}.json", &record_id[..8]),
                title: None,
                tags: None,
            }],
            ..Manifest::default()
        };
        let store = store_with_manifest(manifest);
        let record_path = format!("records/tier-2/test-record-{}.json", &record_id[..8]);
        store
            .save_instance_json(
                &record_path,
                &serde_json::json!({
                    "instanceId": record_id,
                    "typeId": "type-test-001",
                    "typeVersion": 1,
                    "typeNamespace": "com.test",
                    "typeName": "test-type",
                    "fieldValues": [],
                    "sourceRefs": [{
                        "sourceType": "repository-document",
                        "sourceId": "nonexistent-doc-id",
                        "sourceRole": "attaches"
                    }]
                }),
            )
            .unwrap();

        let result = resolve_document_view_attachments(
            &store,
            ResolveDocumentViewAttachmentsInput {
                instance_ids: vec![record_id.to_string()],
            },
        )
        .unwrap();

        assert_eq!(
            result.records.len(),
            1,
            "record with unindexed ref must still appear"
        );
        let att = &result.records[0].attachments[0];
        assert_eq!(att.document_id, "nonexistent-doc-id");
        assert!(
            att.content_path.is_none(),
            "unindexed doc should have no content_path"
        );
        assert!(
            att.sidecar_path.is_none(),
            "unindexed doc should have no sidecar_path"
        );
        assert!(att.title.is_none());
        assert!(att.content_checksum.is_none());
        assert!(att.sidecar_checksum.is_none());
    }

    #[test]
    fn resolve_document_view_attachments_skips_missing_instance() {
        let doc_id = "doc-skip-005";
        let record_id = "aaaa0005-0000-4000-8000-000000000005";
        let store = store_with_doc_and_record(doc_id, record_id);
        link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: record_id.to_string(),
                document_id: doc_id.to_string(),
            },
        )
        .unwrap();

        let result = resolve_document_view_attachments(
            &store,
            ResolveDocumentViewAttachmentsInput {
                instance_ids: vec![
                    "nonexistent-instance-000000000000".to_string(),
                    record_id.to_string(),
                ],
            },
        )
        .unwrap();

        assert_eq!(
            result.records.len(),
            1,
            "missing instance must be silently skipped"
        );
        assert_eq!(result.records[0].instance_id, record_id);
    }

    #[test]
    fn resolve_document_view_attachments_filestore_roundtrip() {
        use crate::store::FileStore;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let root = temp.path();

        std::fs::create_dir_all(root.join(".srs")).unwrap();
        let doc_id = "doc-resolve-roundtrip-006";
        let record_id = "aaaa0006-0000-4000-8000-000000000006";
        let manifest_json = serde_json::json!({
            "instanceIndex": [{
                "instanceId": record_id,
                "tier": 2,
                "path": "records/tier-2/test-type-aaaa0006.json"
            }],
            "sourceDocumentIndex": [{
                "documentId": doc_id,
                "sidecarPath": "brief.meta.json",
                "contentPath": "brief.pdf",
                "title": "Roundtrip Brief",
                "contentChecksum": "sha256:abc"
            }]
        });
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string(&manifest_json).unwrap(),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("package")).unwrap();
        std::fs::write(
            root.join("package/package.json"),
            serde_json::json!({
                "id": "resolve-rt-pkg", "namespace": "com.test", "name": "test",
                "version": "1.0.0", "fields": [], "types": []
            })
            .to_string(),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("records/tier-2")).unwrap();
        std::fs::write(
            root.join("records/tier-2/test-type-aaaa0006.json"),
            serde_json::json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": record_id,
                "typeId": "type-test-001",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "test-type",
                "fieldValues": []
            })
            .to_string(),
        )
        .unwrap();

        let store = FileStore::new(root);
        link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: record_id.to_string(),
                document_id: doc_id.to_string(),
            },
        )
        .unwrap();

        let result = resolve_document_view_attachments(
            &store,
            ResolveDocumentViewAttachmentsInput {
                instance_ids: vec![record_id.to_string()],
            },
        )
        .unwrap();

        assert_eq!(result.records.len(), 1);
        let rec = &result.records[0];
        assert_eq!(rec.instance_id, record_id);
        assert_eq!(rec.attachments.len(), 1);
        let att = &rec.attachments[0];
        assert_eq!(att.document_id, doc_id);
        assert_eq!(att.content_path.as_deref(), Some("brief.pdf"));
        assert_eq!(att.sidecar_path.as_deref(), Some("brief.meta.json"));
        assert_eq!(att.title.as_deref(), Some("Roundtrip Brief"));
        assert_eq!(att.content_checksum.as_deref(), Some("sha256:abc"));
    }

    // ── get_attachment_bytes tests ────────────────────────────────────────────

    fn store_with_binary_attachment(doc_id: &str, content_path: &str, bytes: &[u8]) -> MemoryStore {
        let manifest = manifest_with_index(
            None,
            vec![SourceDocumentIndexEntry {
                document_id: doc_id.to_string(),
                sidecar_path: format!("{}.meta.json", content_path.trim_end_matches(".pdf")),
                content_path: content_path.to_string(),
                title: None,
                sidecar_checksum: None,
                content_checksum: None,
            }],
        );
        let store = store_with_manifest(manifest);
        store
            .save_binary_file(&format!("source-documents/{}", content_path), bytes)
            .unwrap();
        store
    }

    #[test]
    fn get_attachment_bytes_returns_correct_bytes() {
        const BYTES: &[u8] = b"\x89PNG test content";
        let store = store_with_binary_attachment("doc-001", "figure.png", BYTES);
        let result = get_attachment_bytes(
            &store,
            GetAttachmentBytesInput {
                document_id: "doc-001".to_string(),
            },
        )
        .unwrap();
        assert_eq!(result.document_id, "doc-001");
        assert_eq!(result.content_path, "figure.png");
        assert_eq!(result.bytes, BYTES);
    }

    #[test]
    fn get_attachment_bytes_unknown_document_id() {
        let store = store_with_manifest(Manifest::default());
        let err = get_attachment_bytes(
            &store,
            GetAttachmentBytesInput {
                document_id: "no-such-doc".to_string(),
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidInput { .. }),
            "unknown documentId must return InvalidInput, got: {err:?}"
        );
    }

    #[test]
    fn get_attachment_bytes_tombstone() {
        let manifest = manifest_with_index(
            None,
            vec![SourceDocumentIndexEntry {
                document_id: "tomb-doc".to_string(),
                sidecar_path: "tombstone.meta.json".to_string(),
                content_path: "tombstone.pdf".to_string(),
                title: None,
                sidecar_checksum: None,
                content_checksum: None,
            }],
        );
        // Index entry exists but no binary file stored (tombstone per RFC-017).
        let store = store_with_manifest(manifest);
        let err = get_attachment_bytes(
            &store,
            GetAttachmentBytesInput {
                document_id: "tomb-doc".to_string(),
            },
        )
        .unwrap_err();
        assert!(
            err.is_not_found(),
            "tombstone must return not-found error, got: {err:?}"
        );
    }

    #[test]
    fn get_attachment_bytes_filestore_roundtrip() {
        use crate::store::FileStore;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Minimal FileStore setup.
        std::fs::create_dir_all(root.join(".srs")).unwrap();
        std::fs::create_dir_all(root.join("source-documents")).unwrap();
        std::fs::create_dir_all(root.join("package")).unwrap();

        const BYTES: &[u8] = b"PDF binary content for filestore roundtrip";
        std::fs::write(root.join("source-documents/spec.pdf"), BYTES).unwrap();

        let doc_id = "fs-doc-001";
        let manifest_json = serde_json::json!({
            "instanceIndex": [],
            "sourceDocumentsPath": "source-documents",
            "sourceDocumentIndex": [{
                "documentId": doc_id,
                "sidecarPath": "spec.meta.json",
                "contentPath": "spec.pdf"
            }]
        });
        std::fs::write(root.join("manifest.json"), manifest_json.to_string()).unwrap();
        std::fs::write(
            root.join("package/package.json"),
            serde_json::json!({
                "id": "fs-pkg", "namespace": "com.test", "name": "test",
                "version": "1.0.0", "fields": [], "types": []
            })
            .to_string(),
        )
        .unwrap();

        let store = FileStore::new(root);
        let result = get_attachment_bytes(
            &store,
            GetAttachmentBytesInput {
                document_id: doc_id.to_string(),
            },
        )
        .unwrap();
        assert_eq!(result.document_id, doc_id);
        assert_eq!(result.content_path, "spec.pdf");
        assert_eq!(result.bytes, BYTES);
    }

    // ── get_record_attachments tests ──────────────────────────────────────────

    #[test]
    fn get_record_attachments_returns_none_for_missing_record() {
        let store = store_with_manifest(Manifest::default());
        let result = get_record_attachments(
            &store,
            GetRecordAttachmentsInput {
                instance_id: "nonexistent-id-000000000000".to_string(),
            },
        )
        .unwrap();
        assert!(result.is_none(), "missing record must return Ok(None)");
    }

    #[test]
    fn get_record_attachments_empty_when_no_source_refs() {
        let doc_id = "doc-gra-001";
        let record_id = "bbbb0001-0000-4000-8000-000000000001";
        let store = store_with_doc_and_record(doc_id, record_id);
        let result = get_record_attachments(
            &store,
            GetRecordAttachmentsInput {
                instance_id: record_id.to_string(),
            },
        )
        .unwrap()
        .expect("record exists, must return Some");
        assert_eq!(result.instance_id, record_id);
        assert!(
            result.attachments.is_empty(),
            "record with no sourceRefs must return empty attachments"
        );
        assert_eq!(result.source_documents_path, "source-documents");
    }

    #[test]
    fn get_record_attachments_filters_by_attaches_role() {
        let doc_id = "doc-gra-002";
        let record_id = "bbbb0002-0000-4000-8000-000000000002";
        let store = store_with_doc_and_record(doc_id, record_id);
        let record_path = format!("records/tier-2/test-record-{}.json", &record_id[..8]);
        store
            .save_instance_json(
                &record_path,
                &serde_json::json!({
                    "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                    "instanceId": record_id,
                    "typeId": "type-test-001",
                    "typeVersion": 1,
                    "typeNamespace": "com.test",
                    "typeName": "test-type",
                    "fieldValues": [],
                    "sourceRefs": [
                        {
                            "sourceType": "repository-document",
                            "sourceId": doc_id,
                            "sourceRole": "attaches"
                        },
                        {
                            "sourceType": "repository-document",
                            "sourceId": "other-doc-id",
                            "sourceRole": "evidence"
                        }
                    ]
                }),
            )
            .unwrap();
        let result = get_record_attachments(
            &store,
            GetRecordAttachmentsInput {
                instance_id: record_id.to_string(),
            },
        )
        .unwrap()
        .expect("record exists");
        assert_eq!(
            result.attachments.len(),
            1,
            "only sourceRole:attaches must be included"
        );
        assert_eq!(result.attachments[0].document_id, doc_id);
    }

    #[test]
    fn get_record_attachments_resolves_indexed_document() {
        let doc_id = "doc-gra-003";
        let record_id = "bbbb0003-0000-4000-8000-000000000003";
        let manifest = Manifest {
            source_document_index: Some(vec![SourceDocumentIndexEntry {
                document_id: doc_id.to_string(),
                sidecar_path: "report.meta.json".to_string(),
                content_path: "report.pdf".to_string(),
                title: Some("Annual Report".to_string()),
                sidecar_checksum: Some("sha256:sid".to_string()),
                content_checksum: Some("sha256:cnt".to_string()),
            }]),
            instance_index: vec![InstanceIndexEntry {
                instance_id: record_id.to_string(),
                tier: 2,
                path: format!("records/tier-2/test-record-{}.json", &record_id[..8]),
                title: None,
                tags: None,
            }],
            ..Manifest::default()
        };
        let store = store_with_manifest(manifest);
        let record_path = format!("records/tier-2/test-record-{}.json", &record_id[..8]);
        store
            .save_instance_json(
                &record_path,
                &serde_json::json!({
                    "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                    "instanceId": record_id,
                    "typeId": "type-test-001",
                    "typeVersion": 1,
                    "typeNamespace": "com.test",
                    "typeName": "test-type",
                    "fieldValues": []
                }),
            )
            .unwrap();
        link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: record_id.to_string(),
                document_id: doc_id.to_string(),
            },
        )
        .unwrap();
        let result = get_record_attachments(
            &store,
            GetRecordAttachmentsInput {
                instance_id: record_id.to_string(),
            },
        )
        .unwrap()
        .expect("record exists");
        assert_eq!(result.attachments.len(), 1);
        let att = &result.attachments[0];
        assert_eq!(att.document_id, doc_id);
        assert_eq!(att.content_path.as_deref(), Some("report.pdf"));
        assert_eq!(att.sidecar_path.as_deref(), Some("report.meta.json"));
        assert_eq!(att.title.as_deref(), Some("Annual Report"));
        assert_eq!(att.content_checksum.as_deref(), Some("sha256:cnt"));
        assert_eq!(att.sidecar_checksum.as_deref(), Some("sha256:sid"));
    }

    #[test]
    fn get_record_attachments_filestore_roundtrip() {
        use crate::store::FileStore;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let root = temp.path();

        std::fs::create_dir_all(root.join(".srs")).unwrap();
        let doc_id = "doc-gra-roundtrip-004";
        let record_id = "bbbb0004-0000-4000-8000-000000000004";
        let manifest_json = serde_json::json!({
            "instanceIndex": [{
                "instanceId": record_id,
                "tier": 2,
                "path": "records/tier-2/test-type-bbbb0004.json"
            }],
            "sourceDocumentIndex": [{
                "documentId": doc_id,
                "sidecarPath": "slides.meta.json",
                "contentPath": "slides.pdf",
                "title": "Conference Slides",
                "contentChecksum": "sha256:xyz"
            }]
        });
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string(&manifest_json).unwrap(),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("package")).unwrap();
        std::fs::write(
            root.join("package/package.json"),
            serde_json::json!({
                "id": "gra-rt-pkg", "namespace": "com.test", "name": "test",
                "version": "1.0.0", "fields": [], "types": []
            })
            .to_string(),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("records/tier-2")).unwrap();
        std::fs::write(
            root.join("records/tier-2/test-type-bbbb0004.json"),
            serde_json::json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": record_id,
                "typeId": "type-test-001",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "test-type",
                "fieldValues": []
            })
            .to_string(),
        )
        .unwrap();

        let store = FileStore::new(root);
        link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: record_id.to_string(),
                document_id: doc_id.to_string(),
            },
        )
        .unwrap();

        let result = get_record_attachments(
            &store,
            GetRecordAttachmentsInput {
                instance_id: record_id.to_string(),
            },
        )
        .unwrap()
        .expect("record exists on disk");

        assert_eq!(result.instance_id, record_id);
        assert_eq!(result.source_documents_path, "source-documents");
        assert_eq!(result.attachments.len(), 1);
        let att = &result.attachments[0];
        assert_eq!(att.document_id, doc_id);
        assert_eq!(att.content_path.as_deref(), Some("slides.pdf"));
        assert_eq!(att.sidecar_path.as_deref(), Some("slides.meta.json"));
        assert_eq!(att.title.as_deref(), Some("Conference Slides"));
        assert_eq!(att.content_checksum.as_deref(), Some("sha256:xyz"));
    }
}
