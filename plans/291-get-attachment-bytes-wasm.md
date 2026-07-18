# Plan: WASM `get_attachment_bytes(documentId) → Uint8Array` (#291)

## Summary

The WASM surface for attachments (RFC-017 Gate D) is missing a method to retrieve the raw bytes of a source-document attachment for browser-side download. The existing `resolve_document_view_attachments()` binding resolves metadata (path, checksum, title) but does not return file content. This plan adds a `get_attachment_bytes(documentId)` service function in `srs-repository` and a matching WASM binding on `SrsRepository`, completing the browser-download flow for archive-loaded repositories. A prerequisite fix is also included: `JsonStore::save_binary_file` is currently a silent no-op and `load_binary_file` always returns not-found, so binary content is discarded when loading a `.srs` archive into an in-memory `JsonStore`. That makes the new binding unreachable for WASM callers; fixing the in-memory store to actually hold binary files (while continuing to exclude them from `.srsj` serialization) is required for the binding to be useful.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Bindings Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | New method on `SrsRepository`; one service call; no business logic in bindings | accepted |
| [ADR-031](../docs/adr/031-source-doc-blob-portability.md) | `JsonStore` gains an in-memory binary-file map; `.srsj` serialisation excludes it; amendment required | accepted (amendment) |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `get_attachment_bytes` takes a typed input, validates in the service, returns a typed result | accepted |
| [ADR-015](../docs/adr/015-wasm-write-and-export.md) | Binary bytes are held only in `JsonStoreState::binary_files`; `to_srsj_string()` serialises only the `data` map | accepted |

No new ADRs required — this plan implements within existing architectural decisions. ADR-031 requires an amendment (see Phase 1).

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. No payload struct changes. No schema regeneration needed.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files added or modified. No schema sync needed.

---

## Scope

In scope:
- Add `binary_files: HashMap<String, Vec<u8>>` to `JsonStoreState`; implement `save_binary_file` / `load_binary_file` on `JsonStore` to use it (Phase 1)
- Add `get_attachment_bytes(store, GetAttachmentBytesInput) -> Result<GetAttachmentBytesResult, RepositoryError>` to `crates/srs-repository/src/attachment_service.rs` (Phase 2)
- Add `get_attachment_bytes(&self, document_id: &str) -> Result<js_sys::Uint8Array, JsValue>` method on `SrsRepository` in `crates/srs-bindings/src/lib.rs` (Phase 3)
- Add ADR-031 amendment documenting the `JsonStore` binary-file change (Phase 1)
- Add tests in `crates/srs-bindings/tests/attachment_bytes.rs` (Phase 3)

**Out of scope:**
- CLI exposure of raw attachment bytes (the CLI uses `FileStore` which already serves bytes from disk)
- Streaming download or chunking for large files
- Content-type detection
- Checksum verification against `sourceDocumentIndex` on read
- `.srsj` binary embedding (RFC-017 explicitly excludes this)

---

## Phases

### Phase 1: Fix `JsonStore` binary-file storage

**Goal:** After this phase, calling `JsonStore::save_binary_file(path, bytes)` followed by `load_binary_file(path)` returns the saved bytes, and `to_srsj_string()` still omits them.

**Agent:** Repository Service Worker

#### Tasks

- [x] Add `binary_files: HashMap<String, Vec<u8>>` field to the `JsonStoreState` struct in `crates/srs-repository/src/json_store.rs`
- [x] Initialize `binary_files: HashMap::new()` in all three `JsonStoreState { ... }` constructions in `json_store.rs` (search for `JsonStoreState {` to find each site)
- [x] Update `JsonStore::save_binary_file` to insert into `self.state.borrow_mut().binary_files` and return `Ok(())`
- [x] Update `JsonStore::load_binary_file` to look up `self.state.borrow().binary_files.get(relative_path)`, returning a cloned `Vec<u8>` or `Self::not_found(relative_path)`
- [x] Update doc comments on both methods to reflect the new behaviour
- [x] Add amendment to `docs/adr/031-source-doc-blob-portability.md`: "JsonStore now stores binary files in an in-memory map (`binary_files`). `save_binary_file` inserts; `load_binary_file` looks up (not-found if absent). `to_srsj_string()` continues to serialise only the `data` JSON map — binary blobs are still excluded from `.srsj` output. This enables archive-loaded WASM repositories to serve attachment bytes."

#### Acceptance Criteria

- [x] `JsonStore::save_binary_file(path, bytes)` followed by `load_binary_file(path)` returns identical bytes
- [x] `JsonStore::load_binary_file` on an unknown path returns a not-found error
- [x] `JsonStore::to_srsj_string()` does NOT include binary files in its output
- [x] `JsonStore::from_archive(archive_bytes)` followed by `load_binary_file(path)` returns the attachment bytes from the archive

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write (add to `crates/srs-repository/src/json_store.rs` test module):

- `json_store_binary_file_save_and_load` — save then load returns same bytes
- `json_store_binary_file_load_absent` — load unknown path returns not-found error
- `json_store_srsj_excludes_binary` — after saving a binary file, `to_srsj_string()` output does not contain the path
- `json_store_from_archive_binary_available` — pack a `MemoryStore` with a binary file into a ZIP, call `JsonStore::from_archive`, verify `load_binary_file` returns the bytes

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm the four named tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Mark task checkboxes `[x]`.
5. Commit: `fix(json-store): add in-memory binary-file storage for archive-loaded WASM repos (#291)`

---

### Phase 2: Add `get_attachment_bytes` service function

**Goal:** After this phase, `srs-repository` exposes `get_attachment_bytes(store, input)` that resolves a `documentId` to its bytes via `load_binary_file`.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/attachment_service.rs`, after the `link_attachment` block, add:
  ```rust
  pub struct GetAttachmentBytesInput {
      pub document_id: String,
  }

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
      let src_docs_base = manifest.source_documents_path.as_deref().unwrap_or("source-documents");
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
  ```

#### Acceptance Criteria

- [x] `get_attachment_bytes` with a valid `documentId` backed by binary content returns the correct bytes
- [x] `get_attachment_bytes` with an unknown `documentId` returns `RepositoryError::InvalidInput`
- [x] `get_attachment_bytes` with a `documentId` in the index but no binary file (tombstone) returns a not-found `RepositoryError::Io`
- [x] Function compiles with `cargo build -p srs-repository`

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write (in `crates/srs-repository/src/attachment_service.rs` test module):

- `get_attachment_bytes_returns_correct_bytes` — `MemoryStore` with a binary file + index entry; service returns matching bytes
- `get_attachment_bytes_unknown_document_id` — service returns `InvalidInput` error
- `get_attachment_bytes_tombstone` — `MemoryStore` with index entry but no binary file; service returns not-found error

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm the three named tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Mark task checkboxes `[x]`.
5. Commit: `feat(attachment-service): add get_attachment_bytes service function (#291)`

---

### Phase 3: WASM binding and tests

**Goal:** After this phase, `SrsRepository.get_attachment_bytes(documentId)` is a callable WASM method returning a `Uint8Array`, and tests cover the full roundtrip from archive load to byte retrieval.

**Agent:** Bindings Worker

#### Tasks

- [x] In `crates/srs-bindings/src/lib.rs`, extend the existing `attachment_service` import to include `GetAttachmentBytesInput`:
  ```rust
  use srs_repository::attachment_service::{
      self as attachment_service, GetAttachmentBytesInput, ResolveDocumentViewAttachmentsInput,
  };
  ```
- [x] Add method to `impl SrsRepository`:
  ```rust
  /// Return the raw bytes of a source-document attachment by `documentId`.
  ///
  /// Requires the repository to have been loaded via `load_archive()` — a `.srsj`-loaded
  /// repository never contains binary content (tombstone per RFC-017) and will return an error.
  ///
  /// Returns the attachment file bytes as a `Uint8Array`, or a JS error string when:
  /// - `documentId` is not in `manifest.sourceDocumentIndex` (not found in index)
  /// - binary content is absent (tombstone state — archive does not contain the file)
  pub fn get_attachment_bytes(&self, document_id: &str) -> Result<js_sys::Uint8Array, JsValue> {
      let result = attachment_service::get_attachment_bytes(
          &self.store,
          GetAttachmentBytesInput {
              document_id: document_id.to_string(),
          },
      )
      .map_err(js_err)?;
      Ok(js_sys::Uint8Array::from(result.bytes.as_slice()))
  }
  ```
- [x] Create `crates/srs-bindings/tests/attachment_bytes.rs` with three tests:
  - `get_attachment_bytes_roundtrip_via_archive` — use `FileStore` + `add_attachment` to build a store with binary content + index entry, pack to archive, `JsonStore::from_archive`, call `get_attachment_bytes`, verify bytes
  - `get_attachment_bytes_unknown_document_id` — verify `InvalidInput` propagated as `Err`
  - `get_attachment_bytes_srsj_tombstone` — `JsonStore::from_srsj` with index entry but no binary; verify error

#### Acceptance Criteria

- [x] `SrsRepository.get_attachment_bytes(documentId)` compiles on `wasm32-unknown-unknown` target
- [x] `get_attachment_bytes_roundtrip_via_archive` test passes (end-to-end: pack → archive → load → get bytes)
- [x] `get_attachment_bytes_unknown_document_id` test passes
- [x] `get_attachment_bytes_srsj_tombstone` test passes

#### Testing

```bash
cargo test -p srs-bindings
cargo build --target wasm32-unknown-unknown -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

Specific tests: see Tasks above.

#### Milestone gate

1. Verify all acceptance criteria.
2. Confirm all three named tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-bindings
   cargo build --target wasm32-unknown-unknown -p srs-bindings
   cargo clippy -p srs-bindings -- -D warnings
   ```
4. Mark task checkboxes `[x]`.
5. Commit: `feat(srs-bindings): add get_attachment_bytes WASM method (#291)`

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [x] `cargo test` passes with no failures (pre-existing unrelated failures in `srs-gov` excluded)
- [x] `cargo clippy -- -D warnings` passes
- [x] `cargo test --test payload_contracts` passes (no payload structs were changed, but run to confirm)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (worktree path resolution issue; no schema files changed in this branch — confirmed via `git diff origin/master -- crates/srs-schema/`)
- [x] `get_attachment_bytes_roundtrip_via_archive` test exists and passes
- [x] `get_attachment_bytes_unknown_document_id` test passes
- [x] `get_attachment_bytes_srsj_tombstone` test passes
- [x] `cargo build --target wasm32-unknown-unknown -p srs-bindings` succeeds

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `MemoryStore` already correctly implements `save_binary_file`/`load_binary_file` (confirmed in existing archive roundtrip tests)
- The `wasm32-unknown-unknown` target is installed in the CI environment (confirmed in ADR-013)
- `js_sys::Uint8Array::from(slice)` is the correct WASM pattern for returning raw bytes (confirmed from `export_archive` in lib.rs)
