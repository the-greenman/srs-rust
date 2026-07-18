# ADR-033: .srs Archive Format — File-Tree ZIP with archive_pack/archive_unpack

- **Status:** accepted
- **Date:** 2026-07-17
- **Supersedes:** —
- **Superseded by:** —

## Context

RFC-017 Rev 3 (srs#101, merged 2026-07-06) defines the `.srs` binary archive format. The format must:
- Be a deterministic ZIP (sorted entries, zeroed timestamps, Deflate or Store compression, no host metadata).
- Contain `manifest.json` at the ZIP root (the SRS repository manifest).
- Contain all instances, relations, local package, source-document sidecars, and source-document binary content.

Two implementation approaches were evaluated:

**Option A — `ZipStore` implementing `RepositoryStore`**: a struct that wraps an open ZIP file and exposes the full `RepositoryStore` trait surface. Reads would decompress on demand; writes would re-serialize the entire ZIP on every mutation (ZIP format does not support in-place entry mutation).

**Option B — `archive_pack` / `archive_unpack` standalone functions**: pack exports the repository via `export_repository_snapshot_with_options(include_content_blobs: true)` and writes a one-shot deterministic ZIP; unpack reads a ZIP, reconstructs a `RepositorySnapshot`, and calls `import_repository_snapshot`. No ZipStore struct; no RepositoryStore impl.

Two ZIP content models were also evaluated:

**Model 1 — Snapshot JSON in ZIP**: the ZIP contains a single `snapshot.json` file (the full `RepositorySnapshot` serialized as JSON, including base64-encoded binary blobs).

**Model 2 — File-tree ZIP**: the ZIP mirrors the on-disk repository layout — `manifest.json`, `package/`, `records/`, `relations/`, `source-documents/` — binary content as raw bytes (no base64 wrapping in the ZIP entries themselves, though the snapshot layer uses base64 for transport through JSON).

RFC-017 explicitly describes `manifest.json` at the ZIP root and names the file-tree structure, ruling out Model 1.

## Decision

1. Use **Option B** (`archive_pack` / `archive_unpack` standalone functions) rather than a `ZipStore` implementing `RepositoryStore`.
2. Use **Model 2** (file-tree ZIP) — `manifest.json` at ZIP root, `package/package.json`, `records/`, `relations/relations.json`, `source-documents/`.
3. Determinism requirements per RFC-017 Change D:
   - Entries sorted lexicographically by path.
   - Timestamps zeroed via `zip::DateTime::default()`.
   - Compression: `CompressionMethod::Deflated` throughout.
   - No host metadata (no Unix extra fields, no extended timestamps).
4. `archive_pack(source: &dyn RepositoryStore, writer: impl Write + Seek) -> Result<(), RepositoryError>` is the pack API.
5. `archive_unpack(reader: impl Read + Seek, target: &dyn RepositoryStore) -> Result<(), RepositoryError>` is the unpack API.
6. Both functions are `pub` in `srs-repository` (re-exported from `lib.rs`).
7. A new `RepositoryError::InvalidArchive { message: String }` variant handles ZIP read/write errors.
8. The pack function loads `manifest.json` and `package/package.json` as raw bytes from the store via dedicated `RepositoryStore` trait methods (`load_manifest_raw_text`, `load_primary_package_raw_text`, `load_relations_raw_text`) to avoid serialization drift and to keep path strings inside the storage-adapter layer (CLAUDE.md path-string rule). Instance files are read via the manifest `instanceIndex` paths (actual storage layout is mirrored faithfully; canonical paths are not recomputed). Relations are loaded via `load_relations_raw_text`; if absent, the ZIP entry is omitted.
9. **Implementation deviation:** the ZIP also contains `package/package.snapshot.json` — the serialized `PackageBoundarySnapshot` from `export_repository_snapshot_with_options`. This is required because `package/package.json` uses a path-index format (fields and types referenced by relative path, not inlined) that cannot be deserialized as `PackageBoundarySnapshot` directly; the snapshot file enables faithful unpack without re-loading the source store's files. `archive_unpack` reads `package/package.snapshot.json` to reconstruct the primary `PackageBoundarySnapshot` and populate the import snapshot. ZIP entry names for the archive format (e.g. `"manifest.json"`, `"package/package.snapshot.json"`) appear as literals in `archive.rs` for the unpack read path; only the storage adapter paths (for the source store during pack) are hidden behind trait methods.
10. `archive_unpack` extracts `container` and `containerIndex` from `manifest.json` and passes them to `import_repository_snapshot` as `root_container` and `container_index`, preserving the source repository's root container identity across roundtrips.

## Consequences

**Positive:**
- Consistent with ADR-031's stated intent: "The `.srs` archive producer (a future caller) will use `export_repository_snapshot_with_options(..., include_content_blobs: true)`."
- `ZipStore` write mode (re-serializing entire ZIP on every mutation) is avoided — write-once pack is a much simpler contract.
- File-tree layout makes `.srs` archives human-inspectable with any ZIP tool — individual files are readable without a special parser.
- Determinism guarantee is straightforward: sort, zero timestamps, Deflate.
- No new `RepositoryStore` impl to maintain; existing FileStore/MemoryStore/JsonStore are unaffected.

**Negative / trade-offs:**
- A `ZipStore` would allow opening an archive and calling arbitrary service functions directly on it (e.g. `record_list`, `record_get`) without first unpacking. This read-side capability is deferred — a read-only ZipStore can be added later without changing the archive format.
- Binary source-document content passes through base64 encoding in the snapshot layer and is decoded back to raw bytes before being written to the ZIP. This is ~33% overhead in intermediate memory during pack; it is not stored base64 in the ZIP itself.
- `archive_unpack` must reconstruct `SnapshotInstance` from raw JSON files in the ZIP (reading `tier`, `id`, `title`, `tags` from instance JSON), then call `import_repository_snapshot`. If the on-disk instance JSON format changes, `archive_unpack` must be updated in sync.

**Neutral:**
- The `zip` workspace dependency (`zip = { version = "2", default-features = false, features = ["deflate"] }`) was already added in plan #273. The `deflate` feature routes through `flate2` with the `miniz_oxide` backend (pure Rust, WASM-safe); the CI `cargo build --target wasm32-unknown-unknown -p srs-repository` job verifies this remains true.
- A future `srs archive pack` / `srs archive unpack` CLI handler will be thin wrappers over these library functions; no new architectural decisions are needed for the CLI layer.
- The WASM binding for archive functions uses `Vec<u8>` I/O as predicted: `SrsRepository::load_archive(bytes: &[u8])` and `SrsRepository::export_archive() -> Uint8Array`, implemented in #290 via `JsonStore::from_archive` and `archive_to_vec`. The `<impl Write + Seek>` / `<impl Read + Seek>` signatures on the library functions are preserved — `archive_to_vec` is the bridge for callers that need `Vec<u8>`.
- `srs-vscode` and `srs-web` are not affected by this decision.
