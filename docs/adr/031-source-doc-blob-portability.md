# ADR-031: Source-Document Blob Portability in RepositorySnapshot

- **Status:** accepted
- **Date:** 2026-07-16
- **Supersedes:** —
- **Superseded by:** —

## Context

`RepositorySnapshot` is the path-free DTO used for export, import, and copy operations (ADR-008). It serialises all logical repository content — instances, containers, relations, packages — without embedding storage paths. Before this decision, source documents (files under `source-documents/`, registered in `manifest.extra["sourceDocumentIndex"]`) were silently omitted from the snapshot, so `copy_repository` dropped all attachments.

RFC-017 Rev 3 (srs#101) specifies that:
- `.srsj` (JSON Store) MUST NOT include binary source-document content.
- `.srs` ZIP archives MUST include binary content.
- A source document's content file may be absent (tombstone state) without invalidating the repository.

The snapshot layer sits between the logical repository and both of these callers. The challenge is representing binary content (arbitrary bytes, potentially large) in a struct that is also JSON-serialised for `JsonStore` and tested against a path-free guard.

Two encoding options were evaluated:
- **hex**: uses 2× the original size; no dependency; awkward for large blobs.
- **base64**: uses 1.33× the original size; requires `base64 = "0.22"` (STANDARD engine); widely understood.

A third option — a separate out-of-band `HashMap<documentId, Vec<u8>>` return value alongside the snapshot — was considered but rejected because it splits a logically coupled export into two return values, complicating all call sites.

## Decision

1. Add `SourceDocumentSnapshot` to `repository_portability.rs` with fields `document_id`, `sidecar_path`, `content_path`, `sidecar: serde_json::Value`, and `content_base64: Option<String>` (base64-encoded bytes, absent for tombstones and srsj-mode exports).
2. Add `ExportSnapshotOptions { include_content_blobs: bool }` and `export_repository_snapshot_with_options(source, ExportSnapshotOptions)` as the canonical configurable export function.
3. Keep `export_repository_snapshot(source)` as a backward-compatible wrapper that delegates with `include_content_blobs: false`.
4. `copy_repository` uses `include_content_blobs: true`.
5. Binary blobs are excluded from `.srsj` exports via `JsonStore::save_binary_file` being a silent no-op. Note: `JsonStore::to_srsj_string` is an independent physical serialiser that does NOT call `export_repository_snapshot` — it directly serialises the in-memory `data` map (ADR-015). The two code paths are independent; blobs are excluded from `.srsj` at the adapter level, not by routing through the snapshot layer.
6. Binary files are transported as base64 strings within the JSON-serialisable `SourceDocumentSnapshot`.
7. Add `load_binary_file`/`save_binary_file` to `RepositoryStore` for adapter-neutral binary I/O. `JsonStore` implements `load_binary_file` as always-not-found (tombstone) and `save_binary_file` as a silent no-op.

## Consequences

**Positive:**
- `copy_repository` preserves source documents end-to-end for the first time.
- `RepositorySnapshot` remains path-free (base64 fields carry content, not storage paths).
- Backward-compatible: all existing `export_repository_snapshot` callers (`diff.rs`, `validation.rs`, test code) compile unchanged.
- Tombstone state (sidecar present, content absent) is round-tripped faithfully — the index entry survives even when `content_base64` is None.

**Negative / trade-offs:**
- Adds `base64 = "0.22"` as a workspace dependency.
- Large binary blobs inflate JSON by ~33%; the snapshot is not intended for streaming large archives (that is the `.srs` ZIP archive's job).
- `export_repository_snapshot_with_options` duplicates the function signature surface; callers must choose the right variant.
- **Known limitation — all-tombstone edge case:** if every source-document sidecar file is absent at export time (all entries are tombstones), `source_documents` is empty and `source_documents_path` is set to `None`. On import the `source_documents.is_empty()` guard is false, so neither `sourceDocumentsPath` nor `sourceDocumentIndex` is written to the target manifest. A custom configured path (e.g. `"attachments"`) is silently lost. This edge case is unlikely in practice; a future enhancement could decouple `source_documents_path` emission from list emptiness.

**Neutral:**
- `MemoryStore` gains a binary-file map parallel to its text-file map.
- The `.srs` archive producer (a future caller) will use `export_repository_snapshot_with_options(..., include_content_blobs: true)` once implemented.
