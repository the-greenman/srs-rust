# Plan: ZipStore / archive_pack + archive_unpack (srs-rust#276)

## Summary

RFC-017 (srs#101, merged 2026-07-06) defines the `.srs` archive format: a deterministic ZIP containing the full repository file tree (`manifest.json` at root, `package/`, `records/`, `relations/`, `source-documents/`), with sorted entries, zeroed timestamps, and Deflate-or-Store compression. This plan implements `archive_pack` and `archive_unpack` as standalone service functions in `crates/srs-repository/src/archive.rs`, using the existing `RepositoryStore` trait and `export_repository_snapshot_with_options` / `import_repository_snapshot` for data transport. The `zip = "2"` workspace dependency was added in plan #273 (stub in `archive.rs`); `base64 = "0.22"` was added in plan #274.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | Full-repository portability via RepositorySnapshot | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | srs-repository must compile to wasm32 — `zip` dep added with `default-features = false` | accepted |
| [ADR-031](../docs/adr/031-source-doc-blob-portability.md) | `.srs` archive producer uses `export_repository_snapshot_with_options(include_content_blobs: true)` | accepted |
| [ADR-033](../docs/adr/033-srs-archive-format.md) | `.srs` uses file-tree ZIP (not snapshot-JSON-in-ZIP); `archive_pack/unpack` not `ZipStore` | proposed |

No new ADRs beyond ADR-033.

---

## Contracts

### CLI output contract (ADR-011)

No CLI commands added or changed in this plan. `archive_pack` and `archive_unpack` are library functions only. A CLI handler (`srs archive pack` / `srs archive unpack`) is a follow-up tracked in a separate issue. No payload structs modified; `cargo run --bin generate-schemas` is not required.

Verification: `cargo test --test payload_contracts` must still pass — no changes expected.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files under `srs/docs/schema/2.0/`. No action required.

---

## Scope

- Replace the comment-only stub in `crates/srs-repository/src/archive.rs` with working `archive_pack` and `archive_unpack` functions
- Add `RepositoryError::InvalidArchive { message: String }` variant to `crates/srs-repository/src/error.rs`
- Implement `archive_pack(source: &dyn RepositoryStore, writer: impl Write + Seek) -> Result<(), RepositoryError>`
- Implement `archive_unpack(reader: impl Read + Seek, target: &dyn RepositoryStore) -> Result<(), RepositoryError>`
- Make both functions `pub` (exported from `crates/srs-repository`)
- Write roundtrip and determinism tests

**Out of scope:**

- CLI commands `srs archive pack` / `srs archive unpack` — payload struct + handler come in a follow-up issue
- WASM binding for archive functions — follow-up
- `.srs` extension file-type registration in srs-vscode — follow-up
- Streaming/chunked pack for large repositories — not required by RFC-017
- Encryption or compression selection — RFC-017 mandates Deflate or Store only

---

## Phases

### Phase 1: Error variant + archive_pack

**Goal:** `archive_pack` writes a deterministic `.srs` ZIP from any `RepositoryStore`; `RepositoryError::InvalidArchive` is wired into `From<zip::result::ZipError>`.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/error.rs`: add the variant below immediately after `InvalidSnapshotData`:
  ```rust
  #[error("invalid archive: {message}")]
  InvalidArchive { message: String },
  ```
- [ ] In `crates/srs-repository/src/error.rs`: add `From<zip::result::ZipError>` impl:
  ```rust
  impl From<zip::result::ZipError> for RepositoryError {
      fn from(e: zip::result::ZipError) -> Self {
          RepositoryError::InvalidArchive { message: e.to_string() }
      }
  }
  ```
- [ ] Replace the comment-only stub in `crates/srs-repository/src/archive.rs` with the full implementation of `archive_pack`. Complete algorithm (use this; no alternatives):
  1. Call `export_repository_snapshot_with_options(source, ExportSnapshotOptions { include_content_blobs: true })` to obtain the snapshot (used for instances and source-document data).
  2. Build a `Vec<(String, Vec<u8>)>` of `(zip_path, bytes)` pairs using the following rules for each file type:
     - **`manifest.json`** (required): bytes = `source.load_text_file("manifest.json")?.into_bytes()`. Do NOT serialize `snapshot.repository` — `RepositoryMetadata` lacks `instanceIndex` and extra fields; raw load preserves them exactly.
     - **`package/package.json`** (required): bytes = `source.load_text_file("package/package.json")?.into_bytes()`.
     - **`relations/relations.json`** (optional): bytes = `source.load_text_file("relations/relations.json")?.into_bytes()`. Skip this entry if the file returns a not-found error (`err.is_not_found()` on `RepositoryError`).
     - **Each instance** (from `snapshot.instances`): `zip_path = canonical_instance_path(&instance, source)?`; bytes = `serde_json::to_vec_pretty(&instance.value).map_err(|e| RepositoryError::InvalidSnapshotData { message: e.to_string() })?`.
     - **Source doc sidecars** (from `snapshot.source_documents`): `zip_path = format!("{}/{}", source_docs_dir, doc.sidecar_path)` where `source_docs_dir = snapshot.source_documents_path.as_deref().unwrap_or("source-documents")`; bytes = `serde_json::to_vec_pretty(&doc.sidecar).map_err(|e| RepositoryError::InvalidSnapshotData { message: e.to_string() })?`.
     - **Source doc content** (from `snapshot.source_documents`, if `content_base64` is `Some`): `zip_path = format!("{}/{}", source_docs_dir, doc.content_path)`; bytes = `base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| RepositoryError::InvalidArchive { message: e.to_string() })?`.
  3. Sort all `(zip_path, bytes)` pairs lexicographically by `zip_path` (ascending).
  4. Create a `zip::ZipWriter::new(writer)`. For each pair, call:
     ```
     let options = zip::write::FileOptions::default()
         .compression_method(zip::CompressionMethod::Deflated)
         .last_modified_time(zip::DateTime::default());
     zip_writer.start_file(&zip_path, options)?;
     zip_writer.write_all(&bytes)?;
     ```
  5. `zip_writer.finish()?;` (consumes the writer and flushes).

- [ ] Ensure `archive_pack` and `archive_unpack` are `pub` in `archive.rs` and re-exported from `crates/srs-repository/src/lib.rs` as `pub use archive::{archive_pack, archive_unpack}`.

#### Acceptance Criteria

- [ ] `archive_pack` compiles with no warnings
- [ ] ZIP entries are sorted lexicographically by path
- [ ] ZIP entries use `DateTime::default()` (epoch / zeroed timestamp)
- [ ] ZIP entries use `CompressionMethod::Deflated`
- [ ] `RepositoryError::InvalidArchive` is wired — zip errors map cleanly to `RepositoryError`
- [ ] Source document binary content is included in the ZIP (decoded from base64)

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

No tests yet at this phase — tests come in Phase 3.

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

3. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
4. Commit:

```bash
git commit -m "feat(archive): archive_pack + InvalidArchive error variant (#276)"
```

---

### Phase 2: archive_unpack

**Goal:** `archive_unpack` reads a `.srs` ZIP, reconstructs a `RepositorySnapshot`, and imports it into an initialized target store.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/archive.rs`, implement `archive_unpack` using the `import_repository_snapshot` path (chosen to leverage existing import logic, error handling, and tests). Algorithm:
  1. Open `zip::ZipArchive::new(reader)?` — on error return `Err(RepositoryError::InvalidArchive { message: e.to_string() })`.
  2. Build a `HashMap<String, Vec<u8>>` from all ZIP entries. Skip any entries with names ending in `/` (directory markers — not expected in deterministic archives but defensive handling is required).
  3. Extract `manifest.json` bytes (required). If absent: `return Err(RepositoryError::InvalidArchive { message: "missing manifest.json".into() })`. Parse as `serde_json::Value` to extract `repositoryId`, `namespace`, `srsVersion`, `title`, `description`, and `instanceIndex` (as an array of `{ instanceId, tier, path }` objects).
  4. Build `snapshot.repository: RepositoryMetadata` from the parsed manifest fields.
  5. Build `snapshot.instances: Vec<SnapshotInstance>` — for each entry in `instanceIndex`: look up the entry's `path` in the bytes map (required; error if missing), parse bytes as `serde_json::Value` → extract `tier`, `id`, `title`, `tags` from the JSON to build `SnapshotInstance { instance_id, tier, title, tags, value }`.
  6. Build `snapshot.relations: Vec<Relation>` — if `relations/relations.json` exists in the map: deserialize the value array from `{ "relations": [...] }` using `serde_json::from_slice`. Treat missing file as empty relations.
  7. Build `snapshot.packages: Vec<PackageBoundarySnapshot>` — `package/package.json` is required. Parse bytes as `PackageBoundarySnapshot` via `serde_json::from_slice(&bytes).map_err(|e| RepositoryError::InvalidArchive { message: e.to_string() })?`. Wrap in a `Vec` with `boundary_path: None` (primary package).
  8. Build `snapshot.containers` — for now set to empty `Vec::new()` (containers are stored in manifest.extra; `import_repository_snapshot` will handle them through the normal manifest write path). `root_container: None`, `container_index: None`.
  9. Build `snapshot.source_documents` — detect source-documents path from manifest extra field `sourceDocumentsPath` (or default `"source-documents"`). For each `.meta.json` file under that prefix: parse sidecar JSON; look for a corresponding content file (same name without `.meta.json`) and base64-encode its bytes: `base64::engine::general_purpose::STANDARD.encode(&bytes)`. Build `SourceDocumentSnapshot { document_id, sidecar_path, content_path, sidecar, content_base64: Some(b64) }`. If content file absent: `content_base64: None` (tombstone).
  10. Call `import_repository_snapshot(target, &snapshot)?` — propagates `RepositoryNotEmpty` if target is non-empty (ADR-008), no extra guard needed.

  **Error handling:** All JSON parse errors → `RepositoryError::InvalidArchive { message }`. Missing required files (manifest.json, package/package.json, instance paths in instanceIndex) → `InvalidArchive`. Missing optional files (relations.json, source-doc content) → treat as empty/tombstone.

#### Acceptance Criteria

- [ ] `archive_unpack` compiles with no warnings
- [ ] `archive_unpack` reconstructs a `RepositorySnapshot` from ZIP bytes
- [ ] `archive_unpack` calls `import_repository_snapshot` to write to the target
- [ ] `archive_unpack` returns `Err(RepositoryError::InvalidArchive)` on missing `manifest.json` or parse failure
- [ ] `archive_unpack` returns `Err(RepositoryError::RepositoryNotEmpty)` (propagated from `import_repository_snapshot`) if target is non-empty

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

No new passing tests yet at this phase — tests come in Phase 3.

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

3. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
4. Commit:

```bash
git commit -m "feat(archive): archive_unpack (#276)"
```

---

### Phase 3: Tests

**Goal:** Roundtrip and determinism tests pass; `cargo test` is fully green.

**Agent:** Repository Service Worker + Verification Agent

#### Tasks

- [ ] In `crates/srs-repository/src/archive.rs` (or `tests/archive_tests.rs` in the same crate), write:

  **Test 1 — `test_archive_roundtrip`**: Use `MemoryStore` (behind `#[cfg(test)]`) as both source and target:
  1. Initialize a `MemoryStore`, create a few instances (note, typed-record, record), add relations, add a source document with binary content.
  2. Call `archive_pack(&source_store, Cursor::new(&mut zip_bytes))?`.
  3. Create a fresh `MemoryStore` as target. Call `archive_unpack(Cursor::new(&zip_bytes), &mut target_store)?`.
  4. Assert: all instances present in target, relations present, source document sidecar and content round-trip correctly.

  **Test 2 — `test_archive_determinism`**: Pack the same store twice; assert the two byte vectors are identical.

  **Test 3 — `test_archive_zip_entry_order`**: Open the ZIP with `zip::ZipArchive`, read all entry names, assert they are sorted in lexicographic order.

  **Test 4 — `test_archive_zip_timestamps`**: Open the ZIP, check that every entry's `last_modified()` is `zip::DateTime::default()`.

  **Test 5 — `test_archive_unpack_missing_manifest`**: Build a ZIP with no `manifest.json`; assert `archive_unpack` returns `Err(RepositoryError::InvalidArchive { .. })`.

#### Acceptance Criteria

- [ ] `test_archive_roundtrip` passes
- [ ] `test_archive_determinism` passes
- [ ] `test_archive_zip_entry_order` passes (entries sorted lexicographically)
- [ ] `test_archive_zip_timestamps` passes (all entries use epoch/zeroed timestamp)
- [ ] `test_archive_unpack_missing_manifest` passes (returns `InvalidArchive`)
- [ ] `cargo test -p srs-repository` is fully green
- [ ] `cargo test --test payload_contracts` passes (no payload changes expected)

#### Testing

```bash
cargo test -p srs-repository
cargo test -p srs-repository archive
cargo test --test payload_contracts
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Run:

```bash
cargo test -p srs-repository
cargo test --test payload_contracts
cargo clippy -p srs-repository -- -D warnings
```

3. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
4. Commit:

```bash
git commit -m "test(archive): roundtrip, determinism, and entry-order tests (#276)"
```

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `archive_pack` + `archive_unpack` are `pub` and exported from `srs-repository`
- [ ] `RepositoryError::InvalidArchive` variant exists and is reachable from zip errors
- [ ] Roundtrip test passes end-to-end (pack then unpack produces same logical repository)
- [ ] Determinism test confirms byte-identical output for identical input
- [ ] ZIP entry order test confirms lexicographic sort
- [ ] ZIP timestamp test confirms zeroed (epoch) timestamps on all entries

## Coordination Rules

- Lead Integrator owns the public API shape (`archive_pack`, `archive_unpack` signatures, error mapping).
- Repository Service Worker writes implementation and tests.
- Do not edit `crates/srs-schema/schemas/2.0/` — no schema changes in this plan.
- `MemoryStore` is `#[cfg(test)]` — do not use it outside test code.
- `canonical_instance_path` is `pub(crate)` in `repository_portability.rs` — callable from `archive.rs` in the same crate.
- Verification Agent runs `cargo test` + `cargo clippy` after all phases and reports any failures.

## Assumptions

- `zip::DateTime::default()` yields the epoch (zeroed) timestamp — confirmed in zip 2.x docs. RFC-017 Change D requires zeroed timestamps for deterministic output.
- `zip` dep already in workspace with `default-features = false, features = ["deflate"]` (plan #273); no Cargo.toml changes needed.
- `base64 = "0.22"` already in workspace (plan #274); `base64::engine::general_purpose::STANDARD` available.
- `source.load_text_file("manifest.json")`, `source.load_text_file("package/package.json")`, and `source.load_text_file("relations/relations.json")` return raw text (`String`); call `.into_bytes()` to convert. The `load_file` method does not exist on `RepositoryStore` — always use `load_text_file` for text and `load_binary_file` for binary.
- `RepositoryMetadata` (in `snapshot.repository`) does NOT include `instanceIndex` — the ZIP's `manifest.json` entry must be loaded raw from `source.load_text_file("manifest.json")`, not serialized from the snapshot.
- Instance JSON files are self-describing — `tier`, `id`, `title`, `tags` fields are present in the stored JSON (needed for `archive_unpack` to reconstruct `SnapshotInstance`).
- `MemoryStore` in `#[cfg(test)]` implements the full `RepositoryStore` trait including `load_text_file` and `load_binary_file` (needed for roundtrip tests).
- `save_text_file(path, content)` and `save_binary_file(path, content)` are on the `RepositoryStore` trait and are the correct methods for writing files during `archive_unpack`.
