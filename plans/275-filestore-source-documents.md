# Plan: FileStore loads source-documents/ + .meta.json sidecars (#275)

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

`FileStore` has no capability to enumerate or load `source-documents/` content or `.meta.json` sidecar metadata. This plan adds: (1) a `SourceDocumentMeta` type in `srs-core` mirroring the `source-document-meta.json` schema; (2) a `list_source_document_sidecar_paths()` default method on `RepositoryStore` that scans the directory recursively; and (3) a `source_document_service::list_source_documents()` service function returning parsed sidecar entries. This infrastructure is Gate A prerequisite for issues #276, #279, and #280 in the attachments epic.

No spec change is required — `source-document-meta.json` already exists and Rev 2 changes (subdirectory `contentPath`) are already in the mirror.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Lead Integrator |
| Core Model Worker | Core Model Worker |
| Repository Service Worker | Repository Service Worker |
| Verification | Verification Agent |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service returns typed struct `Vec<SourceDocumentEntry>`; filter struct `ListSourceDocumentsFilter` even when empty; all business logic in `srs-repository` | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | No CLI command in this issue — that is #279. No payload struct changes; `payload_contracts` test unchanged. | accepted |
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | FileStore layout is an adapter detail. `"source-documents"` path string is owned by the store via a default trait method, not hard-coded in service logic. | accepted |

No new ADRs needed — all decisions follow existing accepted ADRs.

---

## Contracts

### CLI output contract (ADR-011)

No new CLI command. No new payload struct. `cargo test --test payload_contracts` must pass unchanged.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON schema files. `bash scripts/check-schema-sync.sh` must exit 0 unchanged.

---

## Scope

- `SourceDocumentMeta` struct (and nested `SourceDocumentExcerpt`, `SourceAnchor`) in `crates/srs-core/src/types/source_document_meta.rs`, mirroring all fields from `source-document-meta.json`
- `SourceDocumentEntry` and `ListSourceDocumentsFilter` structs + `list_source_documents()` function added to **`crates/srs-repository/src/attachment_service.rs`** (existing home for all `source-documents/` operations — no new module; avoids DRY violation per architecture review)
- `list_source_documents()` loads manifest, resolves `src_docs_base = manifest.source_documents_path.as_deref().unwrap_or("source-documents")`, calls `store.list_files_recursive(&src_docs_base)`, filters `.meta.json`, parses sidecars, strips `src_docs_base/` prefix so `sidecar_path` in `SourceDocumentEntry` is source-documents-relative (matching `SourceDocumentIndexEntry.sidecar_path` convention)
- `SourceDocumentMetaLoad` error variant in `crates/srs-repository/src/error.rs`
- Tests in `attachment_service.rs` covering MemoryStore roundtrip and FileStore loading against `tests/fixtures/spec-repo/` fixture (4 sidecars across 2 subdirectories)

**Out of scope:**

- A new `source_document_service.rs` module — function lives in `attachment_service.rs` per DRY finding
- `list_source_document_sidecar_paths()` store default method — path resolution requires manifest, so it belongs in the service, not the store trait
- Binary content loading (`load_source_document_bytes`) — needed for #276 archive pack; deferred
- `sourceDocumentIndex`-based lookup optimization — directory scan is the primary approach; index is out of scope
- CLI command `srs attachment list` — that is #279
- WASM binding — deferred to the binding phase of the epic (#290+)
- Write operations (add/remove source documents) — those are #280
- Lenient parsing mode (silently skipping malformed sidecars) — parse errors propagate like other record-loading errors

---

## Phases

### Phase 1: `SourceDocumentMeta` type in `srs-core`

**Goal:** A `SourceDocumentMeta` Rust type exists in `srs-core` with a passing roundtrip test against the four fixture sidecars.

**Agent:** Core Model Worker

#### Tasks

- [ ] Create `crates/srs-core/src/types/source_document_meta.rs`:
  - Derive `Debug, Clone, PartialEq, Serialize, Deserialize` on all structs
  - `#[serde(rename_all = "camelCase")]` on `SourceDocumentMeta`
  - Add doc comment `// RFC-017 core type; belongs in types/, not extensions/ (see ADR-028)` to `SourceDocumentMeta`
  - Do **not** add `#[serde(deny_unknown_fields)]` — forward-compatible with future Rev 3 additions
  - All optional fields: `Option<T>` with `#[serde(skip_serializing_if = "Option::is_none")]`
  - Include a `$schema` field as `schema` (use `#[serde(rename = "$schema")]`): `pub schema: Option<String>`
  - Required fields: `document_id: String`, `content_path: String`, `content_type: String`, `created_at: String`
  - Optional fields: `encoding`, `language`, `title`, `description`, `processing_note`, `excerpt`, `date`, `tags`, `imported_at`, `meta` (as `Option<serde_json::Value>`)
  - Nested `SourceDocumentExcerpt` struct (camelCase serde): `source_document_id: String` (required), `anchor: Option<SourceAnchor>`, `captured_at: Option<String>`, `captured_by: Option<String>`, `source_checksum_at_capture: Option<String>`
  - Nested `SourceAnchor` struct (camelCase serde): `kind: String` (required), `value: String` (required), `note: Option<String>`
  - Tests use inline JSON strings only — `srs-core` has a hard constraint of no file I/O
- [ ] Register in `crates/srs-core/src/types/mod.rs`: add `pub mod source_document_meta;`

#### Acceptance Criteria

- [ ] `SourceDocumentMeta` has all fields: `schema`, `document_id`, `content_path`, `content_type`, `encoding`, `language`, `title`, `description`, `processing_note`, `excerpt`, `date`, `tags`, `created_at`, `imported_at`, `meta`
- [ ] `SourceDocumentExcerpt` has: `source_document_id`, `anchor`, `captured_at`, `captured_by`, `source_checksum_at_capture`
- [ ] `SourceAnchor` has: `kind`, `value`, `note`
- [ ] `cargo test -p srs-core source_document_meta` passes with roundtrip assertions

#### Testing

```bash
cargo test -p srs-core source_document_meta
cargo clippy -p srs-core -- -D warnings
```

Specific tests to write or verify:

- `source_document_meta_roundtrip_spec` — use raw string (`r#"{...}"#`) with spec fixture JSON; asserts `document_id == "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d"`, roundtrips via `serde_json::to_value`/`from_value`
- `source_document_meta_roundtrip_ai_session` — use raw string (`r#"{...}"#`) with ai-session fixture JSON; asserts `content_type == "text/markdown"`, roundtrips

#### Milestone gate

1. `grep "pub struct SourceDocumentMeta" crates/srs-core/src/types/source_document_meta.rs` succeeds
2. `grep "source_document_meta" crates/srs-core/src/types/mod.rs` succeeds
3. `cargo test -p srs-core source_document_meta` — both roundtrip tests pass
4. `cargo clippy -p srs-core -- -D warnings` — 0 warnings
5. Mark task checkboxes `[x]`
6. Commit: `git commit -m "feat(core): add SourceDocumentMeta type (#275)"`

---

### Phase 2: `list_source_documents()` in `attachment_service.rs`

**Goal:** `attachment_service::list_source_documents(store, filter)` returns a parsed `Vec<SourceDocumentEntry>` for all `.meta.json` sidecars under the manifest-configured source-documents path.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add to `crates/srs-repository/src/error.rs` — insert `SourceDocumentMetaLoad` variant after `ThemeLoad` (line 154):
  ```rust
  #[error("failed to load source document metadata at {path:?}: {source}")]
  SourceDocumentMetaLoad {
      path: PathBuf,
      source: serde_json::Error,
  },
  ```
  And add `PartialEq` arm in `impl PartialEq for RepositoryError` after the `ThemeLoad` arm:
  ```rust
  (
      RepositoryError::SourceDocumentMetaLoad { path: a, source: sa },
      RepositoryError::SourceDocumentMetaLoad { path: b, source: sb },
  ) => a == b && sa.to_string() == sb.to_string(),
  ```

- [ ] In `crates/srs-repository/src/attachment_service.rs`, add the following (use `srs_core::types::source_document_meta::SourceDocumentMeta` in the import block):
  ```rust
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
      let sidecar_paths: Vec<String> = store
          .list_files_recursive(&src_docs_base)
          .into_iter()
          .filter(|p| p.ends_with(".meta.json"))
          .collect();
      let mut entries = Vec::with_capacity(sidecar_paths.len());
      for repo_relative_path in sidecar_paths {
          let json_str = store.load_text_file(&repo_relative_path)?;
          let meta = serde_json::from_str::<SourceDocumentMeta>(&json_str)
              .map_err(|source| RepositoryError::SourceDocumentMetaLoad {
                  path: std::path::PathBuf::from(&repo_relative_path),
                  source,
              })?;
          let sidecar_path = repo_relative_path
              .strip_prefix(&prefix)
              .unwrap_or(&repo_relative_path)
              .to_string();
          entries.push(SourceDocumentEntry { sidecar_path, meta });
      }
      Ok(entries)
  }
  ```

- [ ] Tests appended to the `#[cfg(test)] mod tests` block already in `attachment_service.rs`:
  - `list_source_documents_empty` — fresh MemoryStore returns empty vec (no manifest `source_documents_path`, no files)
  - `list_source_documents_single` — single `.meta.json` added to MemoryStore; result has 1 entry with source-documents-relative path
  - `list_source_documents_subdirectory` — two `.meta.json` in subdirectory; result has 2 entries
  - `list_source_documents_malformed_returns_err` — invalid JSON in sidecar; `Err(SourceDocumentMetaLoad)` returned
  - `file_store_list_source_documents_spec_repo` — loads `tests/fixtures/spec-repo/`; expects exactly 4 entries with non-empty `document_id` and `content_type`

  Note: MemoryStore tests need a minimal manifest written to `"manifest.json"` so `load_manifest()` succeeds. Write: `{"instanceId":"test","namespace":"com.example","schema":"https://srs.semanticops.com/schema/2.0/manifest.json"}`.

#### Acceptance Criteria

- [ ] `SourceDocumentEntry` and `ListSourceDocumentsFilter` defined in `attachment_service.rs`
- [ ] `list_source_documents()` defined in `attachment_service.rs`, manifest-aware (no hardcoded path in logic)
- [ ] Returns empty vec (no error) when `source-documents/` directory does not exist
- [ ] `sidecar_path` in `SourceDocumentEntry` is source-documents-relative (not repo-relative)
- [ ] MemoryStore: all 4 memory tests pass
- [ ] FileStore: `file_store_list_source_documents_spec_repo` returns exactly 4 entries
- [ ] No changes to `store.rs` (no new store default method)
- [ ] No new module in `lib.rs`

#### Testing

```bash
cargo test -p srs-repository list_source_documents
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `list_source_documents_empty` — fresh store, empty result
- `list_source_documents_single` — single sidecar, source-documents-relative path in result
- `list_source_documents_subdirectory` — two sidecars in subdirectory
- `list_source_documents_malformed_returns_err` — parse error propagates as `SourceDocumentMetaLoad`
- `file_store_list_source_documents_spec_repo` — 4 entries from fixture

#### Milestone gate

1. All acceptance criteria met
2. `cargo test -p srs-repository` passes (full suite — no regressions)
3. `cargo clippy -p srs-repository -- -D warnings` passes
4. Mark task checkboxes `[x]`
5. Commit: `git commit -m "feat(repository): add list_source_documents to attachment_service (#275)"`

---

## Final Acceptance

- [ ] `cargo test` passes (full workspace, no failures)
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] 4 sidecar entries returned from `tests/fixtures/spec-repo/` via FileStore
- [ ] `sidecar_path` in results is source-documents-relative (not repo-relative)
- [ ] MemoryStore roundtrip tests all pass
- [ ] `srs repo validate --repo tests/fixtures/spec-repo` still exits 0 (no regressions)

## Coordination Rules

- Core Model Worker writes only to `crates/srs-core/`.
- Repository Service Worker writes only to `crates/srs-repository/` and its tests.
- No changes to `srs-cli`, `srs-bindings`, or entity schemas.
- Phase 2 must not start until Phase 1 milestone gate passes.

## Assumptions

- `sourceDocumentsPath` manifest override is not in scope; `"source-documents"` is the default
- Binary content loading is out of scope; needed for archive operations (#276)
- The `.meta.json` sidecar naming convention is stable
- Parse errors are propagated strictly, consistent with how other record loading works
- The spec-repo fixture has exactly 4 `.meta.json` sidecars
