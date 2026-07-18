use crate::attachment_service::{
    resolve_document_view_attachments, ResolveDocumentViewAttachmentsInput,
};
use crate::error::RepositoryError;
use crate::render_service::{render_document_view, RenderDocumentViewOptions};
use crate::store::RepositoryStore;
use std::collections::HashSet;
use std::io::{Seek, Write};
use zip::write::SimpleFileOptions;

pub struct ExportBundleInput {
    pub instance_id: String,
    pub view_id: String,
    pub format: Option<String>,
}

pub struct ExportBundleMetadata {
    pub rendered_filename: String,
    pub attachment_count: usize,
    pub diagnostics: Vec<String>,
}

pub fn export_record_bundle(
    store: &dyn RepositoryStore,
    input: ExportBundleInput,
    writer: impl Write + Seek,
) -> Result<ExportBundleMetadata, RepositoryError> {
    let render_result = render_document_view(RenderDocumentViewOptions {
        store,
        view_id: &input.view_id,
        format: input.format.as_deref(),
        theme_variant: None,
        container_id: None,
        instance_id_filter: Some(&input.instance_id),
    })?;

    let attach_result = resolve_document_view_attachments(
        store,
        ResolveDocumentViewAttachmentsInput {
            instance_ids: vec![input.instance_id.clone()],
        },
    )?;

    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    entries.push((
        "decision.md".to_string(),
        render_result.rendered.into_bytes(),
    ));

    let mut used_keys: HashSet<String> = HashSet::new();
    used_keys.insert("decision.md".to_string());

    for record in &attach_result.records {
        for attachment in &record.attachments {
            let Some(content_path) = &attachment.content_path else {
                continue;
            };
            let basename = content_path
                .rsplit('/')
                .next()
                .unwrap_or(content_path.as_str());
            let candidate_key = format!("attachments/{}", basename);
            let entry_key = if used_keys.contains(&candidate_key) {
                // Two attachments share a basename: keep the bundle flat by
                // appending the first 8 chars of the document ID as a suffix.
                let id_prefix = &attachment.document_id[..8.min(attachment.document_id.len())];
                format!("attachments/{}_{}", basename, id_prefix)
            } else {
                candidate_key
            };
            used_keys.insert(entry_key.clone());
            let full_path = format!("{}/{}", attach_result.source_documents_path, content_path);
            let bytes = store.load_binary_file(&full_path)?;
            entries.push((entry_key, bytes));
        }
    }

    let attachment_count = entries.len().saturating_sub(1);

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut zip = zip::ZipWriter::new(writer);
    for (path, bytes) in &entries {
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        zip.start_file(path, options)
            .map_err(|e| RepositoryError::InvalidExportBundle {
                message: format!("failed to start ZIP entry '{}': {}", path, e),
            })?;
        zip.write_all(bytes)
            .map_err(|e| RepositoryError::InvalidExportBundle {
                message: format!("failed to write ZIP entry '{}': {}", path, e),
            })?;
    }
    zip.finish()
        .map_err(|e| RepositoryError::InvalidExportBundle {
            message: format!("failed to finalize ZIP: {}", e),
        })?;

    Ok(ExportBundleMetadata {
        rendered_filename: "decision.md".to_string(),
        attachment_count,
        diagnostics: render_result.diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachment_service::{link_attachment, LinkAttachmentInput};
    use crate::index::InstanceIndexEntry;
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::store::memory::MemoryStore;
    use srs_core::types::source_document::SourceDocumentIndexEntry;
    use srs_core::types::view::{DocumentSection, DocumentView, EmptyBehavior, SectionSource};
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::path::PathBuf;
    use zip::ZipArchive;

    /// Build a minimal Package with a DocumentView for export tests.
    /// Uses a TypeQuery section with a non-existent semantic type — produces an
    /// empty rendered output (section hidden per EmptyBehavior::Hide) without erroring.
    fn minimal_package_with_view(view_id: &str) -> Package {
        let doc_view = DocumentView {
            id: view_id.to_string(),
            namespace: "test".to_string(),
            name: "export-test-view".to_string(),
            version: 1,
            description: "Minimal test view".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
                section_id: "content".to_string(),
                title: None,
                description: None,
                order: 0,
                source: SectionSource::TypeQuery {
                    semantic_object_type: "test/placeholder".to_string(),
                    lifecycle_state: None,
                    container_ids: None,
                    lifecycle_states: None,
                    exclude_lifecycle_states: None,
                    container_scope: None,
                },
                render_view_id: None,
                type_dispatch: None,
                title_field_id: None,
                ordering: None,
                required: None,
                empty_behavior: Some(EmptyBehavior::Hide),
            }],
            navigation_links: None,
            preamble: None,
            format: Some("markdown".to_string()),
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        Package {
            id: "test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![doc_view],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        }
    }

    #[test]
    fn test_export_bundle_no_attachments() {
        let instance_id = "test-noatt-0000-4000-8000-000000000002".to_string();
        let view_id = "view-noatt-0000-4000-8000-000000000002".to_string();

        let manifest = Manifest {
            instance_index: vec![InstanceIndexEntry {
                instance_id: instance_id.clone(),
                tier: 2,
                path: format!("records/tier-2/dec-{}.json", &instance_id[..8]),
                title: None,
                tags: None,
            }],
            ..Manifest::default()
        };
        let package = minimal_package_with_view(&view_id);
        let store = MemoryStore::new(manifest, package);
        store
            .save_instance_json(
                &format!("records/tier-2/dec-{}.json", &instance_id[..8]),
                &serde_json::json!({
                    "instanceId": instance_id,
                    "typeId": "type-dec-001",
                    "typeVersion": 1,
                    "typeNamespace": "com.test",
                    "typeName": "decision",
                    "fieldValues": []
                }),
            )
            .unwrap();

        let mut buf = Cursor::new(Vec::new());
        let meta = export_record_bundle(
            &store,
            ExportBundleInput {
                instance_id: instance_id.clone(),
                view_id: view_id.clone(),
                format: None,
            },
            &mut buf,
        )
        .expect("export_record_bundle should succeed with no attachments");

        assert_eq!(meta.rendered_filename, "decision.md");
        assert_eq!(meta.attachment_count, 0);

        buf.set_position(0);
        let mut zip = ZipArchive::new(buf).expect("should be a valid zip");
        assert_eq!(zip.len(), 1);
        let entry_name = zip.by_index(0).unwrap().name().to_string();
        assert_eq!(entry_name, "decision.md");
    }

    #[test]
    fn test_export_bundle_with_attachments() {
        let instance_id = "test-att01-0000-4000-8000-000000000003".to_string();
        let view_id = "view-att01-0000-4000-8000-000000000003".to_string();
        let doc_id = "doc-att01-001";
        let content_path = "report.pdf";
        let pdf_bytes: &[u8] = b"fake pdf content";

        let manifest = Manifest {
            source_document_index: Some(vec![SourceDocumentIndexEntry {
                document_id: doc_id.to_string(),
                sidecar_path: "report.meta.json".to_string(),
                content_path: content_path.to_string(),
                title: Some("Report".to_string()),
                sidecar_checksum: None,
                content_checksum: None,
            }]),
            instance_index: vec![InstanceIndexEntry {
                instance_id: instance_id.clone(),
                tier: 2,
                path: format!("records/tier-2/dec-{}.json", &instance_id[..8]),
                title: None,
                tags: None,
            }],
            ..Manifest::default()
        };
        let package = minimal_package_with_view(&view_id);
        let store = MemoryStore::new(manifest, package);
        store
            .save_instance_json(
                &format!("records/tier-2/dec-{}.json", &instance_id[..8]),
                &serde_json::json!({
                    "instanceId": instance_id,
                    "typeId": "type-dec-001",
                    "typeVersion": 1,
                    "typeNamespace": "com.test",
                    "typeName": "decision",
                    "fieldValues": []
                }),
            )
            .unwrap();
        link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: instance_id.clone(),
                document_id: doc_id.to_string(),
            },
        )
        .unwrap();
        store
            .save_binary_file(&format!("source-documents/{}", content_path), pdf_bytes)
            .unwrap();

        let mut buf = Cursor::new(Vec::new());
        let meta = export_record_bundle(
            &store,
            ExportBundleInput {
                instance_id: instance_id.clone(),
                view_id: view_id.clone(),
                format: None,
            },
            &mut buf,
        )
        .expect("export_record_bundle should succeed with one attachment");

        assert_eq!(meta.rendered_filename, "decision.md");
        assert_eq!(meta.attachment_count, 1);

        buf.set_position(0);
        let mut zip = ZipArchive::new(buf).expect("should be a valid zip");
        assert_eq!(
            zip.len(),
            2,
            "ZIP should contain decision.md + one attachment"
        );

        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(names.contains(&"decision.md".to_string()));
        assert!(names.contains(&"attachments/report.pdf".to_string()));

        let mut entry = zip.by_name("attachments/report.pdf").unwrap();
        let mut content = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut content).unwrap();
        assert_eq!(
            content, pdf_bytes,
            "attachment bytes must match source bytes"
        );
    }

    #[test]
    fn test_export_bundle_cross_store_roundtrip() {
        use tempfile::NamedTempFile;

        let instance_id = "test-xst01-0000-4000-8000-000000000004".to_string();
        let view_id = "view-xst01-0000-4000-8000-000000000004".to_string();
        let doc_id = "doc-xst01-001";
        let content_path = "evidence.pdf";
        let pdf_bytes: &[u8] = b"cross-store evidence bytes";

        let manifest = Manifest {
            source_document_index: Some(vec![SourceDocumentIndexEntry {
                document_id: doc_id.to_string(),
                sidecar_path: "evidence.meta.json".to_string(),
                content_path: content_path.to_string(),
                title: Some("Evidence".to_string()),
                sidecar_checksum: None,
                content_checksum: None,
            }]),
            instance_index: vec![InstanceIndexEntry {
                instance_id: instance_id.clone(),
                tier: 2,
                path: format!("records/tier-2/dec-{}.json", &instance_id[..8]),
                title: None,
                tags: None,
            }],
            ..Manifest::default()
        };
        let package = minimal_package_with_view(&view_id);
        let store = MemoryStore::new(manifest, package);
        store
            .save_instance_json(
                &format!("records/tier-2/dec-{}.json", &instance_id[..8]),
                &serde_json::json!({
                    "instanceId": instance_id,
                    "typeId": "type-dec-001",
                    "typeVersion": 1,
                    "typeNamespace": "com.test",
                    "typeName": "decision",
                    "fieldValues": []
                }),
            )
            .unwrap();
        link_attachment(
            &store,
            LinkAttachmentInput {
                instance_id: instance_id.clone(),
                document_id: doc_id.to_string(),
            },
        )
        .unwrap();
        store
            .save_binary_file(&format!("source-documents/{}", content_path), pdf_bytes)
            .unwrap();

        // Pack to a real temp file (File implements Write + Seek — cross-store requirement)
        let mut tmp = NamedTempFile::new().expect("create temp file");
        export_record_bundle(
            &store,
            ExportBundleInput {
                instance_id: instance_id.clone(),
                view_id: view_id.clone(),
                format: None,
            },
            &mut tmp,
        )
        .expect("cross-store export should succeed");

        // Re-open and verify via zip reader on the real file
        let tmp_path = tmp.path().to_path_buf();
        let f = std::fs::File::open(&tmp_path).expect("open temp file");
        let mut zip = ZipArchive::new(f).expect("should be a valid zip");
        assert_eq!(zip.len(), 2, "ZIP should have decision.md + one attachment");

        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(
            names.contains(&"decision.md".to_string()),
            "ZIP must contain decision.md"
        );
        assert!(
            names.contains(&"attachments/evidence.pdf".to_string()),
            "ZIP must contain attachments/evidence.pdf"
        );

        let f2 = std::fs::File::open(&tmp_path).expect("re-open temp file");
        let mut zip2 = ZipArchive::new(f2).expect("reopen zip");
        let mut entry = zip2.by_name("attachments/evidence.pdf").unwrap();
        let mut content = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut content).unwrap();
        assert_eq!(
            content, pdf_bytes,
            "attachment bytes must be byte-equal to source"
        );
    }
}
