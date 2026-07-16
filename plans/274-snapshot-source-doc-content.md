# Plan: RepositorySnapshot carries source-document content + sidecar metadata (#274)

## Summary

`RepositorySnapshot` — the path-free DTO used by `export_repository_snapshot`, `import_repository_snapshot`, and `copy_repository` — currently omits source documents entirely. This means a repo copy drops all attachments silently. This plan extends `RepositorySnapshot` to carry `sourceDocumentIndex` entries as documentId-keyed sidecar snapshots with optional binary content blobs. Binary is skipped for `.srsj` (JSON Store) callers and included for `.srs` archive / `copy_repository` callers, per RFC-017 Rev 3 (srs#101). The existing path-free-snapshot guard test must continue to pass.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Repository Service Worker |
| Repository Service Worker | — |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-007](../docs/adr/007-file-before-index-write-ordering.md) | Files are written to store before the manifest index is updated (source doc sidecars + binaries written before `save_manifest`) | accepted |
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | `RepositorySnapshot` is path-free; callers must not embed storage paths | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions take typed input structs | accepted |
| [ADR-021](../docs/adr/021-batch-write-mode.md) | Source-doc writes in `do_import` are inside the `begin_batch`/`commit_batch` bracket — atomicity guarantee inherited | accepted |
| [ADR-031](../docs/adr/031-source-doc-blob-portability.md) | Binary blobs encoded as base64 strings in `SourceDocumentSnapshot`; export options control inclusion | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. `RepositorySnapshot` is an internal DTO, not a CLI payload struct. No `payload.rs` changes; no schema regeneration needed.

### Entity schema sync (check-schema-sync.sh)

No entity schema changes. `source-document-meta.json` already exists in the spec. No mirror sync needed.

---

## Scope

- Add `load_binary_file` / `save_binary_file` to `RepositoryStore` trait, implemented in `FileStore` and `MemoryStore`.
- Add `SourceDocumentSnapshot` struct and `source_documents: Vec<SourceDocumentSnapshot>` field to `RepositorySnapshot` in `repository_portability.rs`.
- Add `ExportSnapshotOptions { include_content_blobs: bool }` and `export_repository_snapshot_with_options(source, &ExportSnapshotOptions)`.
- Update `export_repository_snapshot` to delegate (backward-compat, no blobs).
- Update `copy_repository` to use `include_content_blobs: true`.
- Import: materialize source-document sidecars and optional blobs from snapshot into target store; reconstruct `sourceDocumentIndex` in manifest.
- Tests: roundtrip with blobs, tombstone roundtrip, guard test still passes.
- Add `base64` to workspace dependencies.

**Out of scope:**
- Source-document import CLI command (future: file the placeholder as a follow-up).
- Soft size-warning diagnostics (RFC-017 Change E — separate issue).
- `attaches` sourceRole validation (requires srs-core Rust field rename from `relation_type` to `source_role`, separate from #274).
- `.srs` ZIP archive producer (RFC-017 Change D) — that caller will use `export_repository_snapshot_with_options` once implemented.
- srsj-gzip retirement validation (RFC-017 Change F) — separate issue.

---

## Phases

### Phase 1: Binary file I/O on RepositoryStore

**Goal:** `RepositoryStore` trait has `load_binary_file` and `save_binary_file`; both `FileStore` and `MemoryStore` implement them.

**Agent:** Repository Service Worker

#### Tasks

- [x] Add `load_binary_file(&self, relative_path: &str) -> Result<Vec<u8>, RepositoryError>` to the `RepositoryStore` trait in `crates/srs-repository/src/store.rs` (between the `load_text_file`/`save_text_file` section and `validate_package_ref_path`).
- [x] Add `save_binary_file(&self, relative_path: &str, content: &[u8]) -> Result<(), RepositoryError>` to the trait in the same section.
- [x] Implement `load_binary_file` on `FileStore`: `std::fs::read(self.abs(relative_path))` mapped to `RepositoryError::Io`.
- [x] Implement `save_binary_file` on `FileStore`: create parent dirs then `std::fs::write`.
- [x] In `crates/srs-repository/src/store.rs` (where `MemoryStore` is defined in `#[cfg(test)] pub mod memory`): add a `binary_data: RefCell<HashMap<String, Vec<u8>>>` field (parallel to the `data` map, consistent with all other `MemoryStore` fields which use `RefCell`) and implement both methods.
- [x] Add `load_binary_file`/`save_binary_file` to `JsonStore` in `json_store.rs`: `load` always returns not-found (callers treat absent blobs as tombstones), `save` is a silent no-op. Per RFC-017 Change F, `.srsj` is text-only and must never store binary blobs.

#### Acceptance Criteria

- [ ] `cargo build -p srs-repository` compiles without warnings.
- [ ] Both `FileStore` and `MemoryStore` satisfy the trait without `todo!()` stubs.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write:
- `binary_file_roundtrip_memory` — write bytes to MemoryStore, read back, assert equal.
- `binary_file_roundtrip_file` — write bytes to FileStore (tempdir), read back, assert equal.

#### Milestone gate

1. All acceptance criteria checked.
2. Two new tests exist and pass.
3. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
4. Mark checkboxes `[x]`, commit:
```bash
git commit -m "feat(repository): add load_binary_file/save_binary_file to RepositoryStore (#274)"
```

---

### Phase 2: SourceDocumentSnapshot type + RepositorySnapshot field

**Goal:** `RepositorySnapshot` carries a `source_documents` field (empty by default, backward-compatible); serialization still passes the path-free guard test.

**Agent:** Repository Service Worker

#### Tasks

- [x] Add `base64 = "0.22"` to `[workspace.dependencies]` in `srs-rust/Cargo.toml`.
- [x] Add `base64 = { workspace = true }` to `[dependencies]` in `crates/srs-repository/Cargo.toml`.
- [x] In `crates/srs-repository/src/repository_portability.rs`, add `SourceDocumentSnapshot` struct above `RepositorySnapshot`:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDocumentSnapshot {
    pub document_id: String,
    /// sidecarPath relative to sourceDocumentsPath (e.g. "doc.pdf.meta.json").
    pub sidecar_path: String,
    /// contentPath relative to sourceDocumentsPath (e.g. "doc.pdf" or "audio/meeting.mp3").
    pub content_path: String,
    /// Full parsed sidecar JSON (source-document-meta.json shape).
    pub sidecar: serde_json::Value,
    /// Base64-encoded binary content. None = tombstone or metadata-only export (srsj).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
}
```

- [x] Add to `RepositorySnapshot` (two fields — `source_documents_path` carries the manifest value for import reconstruction; `source_documents` carries the sidecar entries):
```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub source_documents_path: Option<String>,
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub source_documents: Vec<SourceDocumentSnapshot>,
```
- [x] Add `ExportSnapshotOptions` struct with `#[derive(Debug, Clone, Copy)]`:
```rust
#[derive(Debug, Clone, Copy)]
pub struct ExportSnapshotOptions {
    pub include_content_blobs: bool,
}
```

#### Acceptance Criteria

- [ ] `cargo build -p srs-repository` compiles without warnings.
- [ ] Existing `repository_snapshot_contains_no_paths` test still passes (no new path keys in JSON).
- [ ] `RepositorySnapshot` serializes with `source_documents` omitted when empty.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:
- Existing `repository_snapshot_contains_no_paths` — must still pass unchanged.
- `source_document_snapshot_has_no_path_key` — creates a `SourceDocumentSnapshot` with a populated sidecar, serializes to JSON, asserts `!text.contains("\"path\"")`.

#### Milestone gate

1. All acceptance criteria checked.
2. Both tests pass.
3. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
4. Mark checkboxes `[x]`, commit:
```bash
git commit -m "feat(repository): add SourceDocumentSnapshot to RepositorySnapshot (#274)"
```

---

### Phase 3: Export — populate source_documents from store

**Goal:** `export_repository_snapshot_with_options` reads `sourceDocumentIndex` from the manifest, loads sidecars (always) and binary content (when `include_content_blobs: true`), and populates `snapshot.source_documents`. `export_repository_snapshot` delegates with `include_content_blobs: false`. `copy_repository` uses `include_content_blobs: true`.

**Agent:** Repository Service Worker

#### Tasks

- [x] Add `pub fn export_repository_snapshot_with_options(source: &dyn RepositoryStore, options: ExportSnapshotOptions) -> Result<RepositorySnapshot, RepositoryError>` in `repository_portability.rs`. Move the implementation body from `export_repository_snapshot` into this new function and add source-doc collection at the end.
- [x] In `export_repository_snapshot_with_options`, after building the base snapshot, collect source documents:
  1. Read `manifest.extra["sourceDocumentsPath"]` as a `&str` (default `"source-documents"` if absent).
  2. Read `manifest.extra["sourceDocumentIndex"]` as a `Vec<serde_json::Value>` (empty if absent).
  3. For each index entry: extract `documentId`, `sidecarPath`, `contentPath` (all required strings — return `InvalidSnapshotData` if missing).
  4. Compute `sidecar_full_rel = format!("{}/{}", source_docs_path, sidecar_path)`.
  5. Load sidecar: `source.load_text_file(&sidecar_full_rel)` → `serde_json::from_str` → `serde_json::Value`. On `Io` error with `NotFound` kind, treat as tombstone (sidecar absent — include the index metadata but with `sidecar: Value::Null`). On other errors, propagate.
  6. If `options.include_content_blobs`:
     - Compute `content_full_rel = format!("{}/{}", source_docs_path, content_path)`.
     - Call `source.load_binary_file(&content_full_rel)`.
     - On `Io` error with `NotFound` kind (tombstone state), set `content_base64 = None`.
     - On other errors, propagate.
     - On success, base64-encode: `use base64::{engine::general_purpose::STANDARD, Engine as _}; STANDARD.encode(&bytes)`.
  7. Push `SourceDocumentSnapshot { document_id, sidecar_path, content_path, sidecar, content_base64 }`.
- [x] Update `export_repository_snapshot` to delegate: `export_repository_snapshot_with_options(source, ExportSnapshotOptions { include_content_blobs: false })`.
- [x] In `copy_repository`: change to `export_repository_snapshot_with_options(source, ExportSnapshotOptions { include_content_blobs: true })?`.
- [x] Add `is_not_found(err: &RepositoryError) -> bool` helper that handles both `RepositoryError::NotFound` (MemoryStore) and `RepositoryError::Io { source }` where `source.kind() == NotFound` (FileStore/JsonStore), for use in tombstone detection.
- [x] Carry `sourceDocumentsPath` as `snapshot.source_documents_path: Option<String>` (set to the actual value from manifest, or `None` when no source docs exist).

#### Acceptance Criteria

- [ ] `export_repository_snapshot` (no options) returns a snapshot with `source_documents` populated (sidecars, no blobs) for a repo with source docs.
- [ ] `export_repository_snapshot_with_options(..., include_content_blobs: true)` returns a snapshot with `content_base64` populated.
- [ ] A tombstone entry (sidecar present, content file absent) exports without error, with `content_base64: None`.
- [ ] All existing callers (`diff.rs`, `validation.rs`, `json_store.rs` tests) compile unchanged.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write:
- `export_snapshot_includes_source_doc_sidecar` — set up MemoryStore with manifest `sourceDocumentIndex` + a sidecar via `save_text_file`, export without blobs, assert `snapshot.source_documents[0].sidecar` is correct JSON.
- `export_snapshot_with_blobs_includes_content` — set up MemoryStore with binary content via `save_binary_file`, export with blobs, decode base64, assert bytes match.
- `export_snapshot_tombstone_does_not_error` — set up index entry with sidecar but no content file, export with blobs option, assert no error and `content_base64` is None.

#### Milestone gate

1. All acceptance criteria checked.
2. Three new tests exist and pass, all existing tests pass.
3. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
4. Mark checkboxes `[x]`, commit:
```bash
git commit -m "feat(repository): export source-doc sidecars + optional blobs in snapshot (#274)"
```

---

### Phase 4: Import — materialize source_documents into target store

**Goal:** `import_repository_snapshot` writes sidecar files and optional content blobs to the target store, and reconstructs `sourceDocumentIndex` in the manifest.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `do_import` in `repository_portability.rs`, after handling instances and before `target.save_manifest`, add source-document materialization (ADR-007: files before index; ADR-021: inside batch bracket):
  1. Determine `source_docs_path`: use `snapshot.source_documents_path.as_deref().unwrap_or("source-documents")` — the snapshot carries the original `sourceDocumentsPath` value from the source manifest (or `None` if there were no source docs, in which case the default is fine).
  2. Build a new `sourceDocumentIndex` array as `Vec<serde_json::Value>`.
  3. For each `entry` in `snapshot.source_documents`:
     - Write sidecar: `target.save_text_file(&sidecar_full_rel, &serde_json::to_string_pretty(&entry.sidecar).map_err(|e| RepositoryError::Serialize { path: ..., source: e })?)`.
     - If `entry.content_base64` is `Some(b64)`: decode with `STANDARD.decode(b64).map_err(|e| RepositoryError::InvalidSnapshotData { message: ... })?`, write with `save_binary_file`.
     - Append index entry: `json!({ "documentId": ..., "sidecarPath": ..., "contentPath": ... })`.
  4. If `!snapshot.source_documents.is_empty()`:
     - `manifest.extra.insert("sourceDocumentsPath", json!(source_docs_path))`.
     - `manifest.extra.insert("sourceDocumentIndex", json!(source_doc_index))`.
- [x] Base64 decode errors map to `RepositoryError::InvalidSnapshotData` with a descriptive message including the `content_path`.

#### Acceptance Criteria

- [ ] A roundtrip (export with blobs → import) preserves sidecar JSON content exactly.
- [ ] A roundtrip (export with blobs → import) preserves binary content exactly (byte-for-byte).
- [ ] A tombstone roundtrip (sidecar only, no content) imports the sidecar but writes no content file; `sourceDocumentIndex` is reconstructed.
- [ ] `sourceDocumentsPath` appears in manifest after import.
- [ ] Existing import tests still pass.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write:
- `roundtrip_source_doc_with_blob_memory_to_memory` — MemoryStore source with doc + blob → export with blobs → import to fresh MemoryStore → verify sidecar and content present.
- `roundtrip_source_doc_tombstone_memory_to_memory` — MemoryStore source with index + sidecar, no content → export → import → verify sidecar present, no content written, `sourceDocumentIndex` has entry.
- `copy_repository_preserves_source_docs` — call `copy_repository` (which uses blobs), verify target has sidecar + content.

#### Milestone gate

1. All acceptance criteria checked.
2. Three new tests exist and pass, all existing tests pass.
3. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
4. Mark checkboxes `[x]`, commit:
```bash
git commit -m "feat(repository): materialize source-doc sidecars + blobs on snapshot import (#274)"
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] Existing `repository_snapshot_contains_no_paths` test passes unchanged
- [ ] `SourceDocumentSnapshot` serialized JSON contains no `"path"` key (verified by `source_document_snapshot_has_no_path_key` test)
- [ ] Roundtrip test (binary content) passes: bytes preserved exactly
- [ ] Tombstone roundtrip: no error, `sourceDocumentIndex` entry preserved
- [ ] `copy_repository` test verifies source docs are preserved end-to-end

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `MemoryStore` currently has a text-file map (used by `load_text_file`/`save_text_file`). A parallel binary-file map follows the same pattern.
- `base64 = "0.22"` is available on crates.io and has a stable API under the `Engine` trait introduced in 0.21.
- Source documents always use forward-slash path separators in `sidecarPath`/`contentPath` (RFC-017 R9).
- The sidecar file's `contentPath` is relative to `sourceDocumentsPath` (per RFC-017).
- `sourceDocumentsPath` defaults to `"source-documents"` when absent from manifest (matching the analysis fixture).
