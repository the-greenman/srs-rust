# ADR-033: SRSzip — deterministic ZIP as the portable repository format

- **Status:** accepted
- **Date:** 2026-07-21
- **Supersedes:** —
- **Superseded by:** —

## Context

SRS repositories need a portable, single-file interchange format for:
- Transfer between systems (copy, backup, import/export)
- Deterministic content-addressable snapshots (binary diff, cache, integrity checks)
- WASM binding consumers that operate on in-memory byte buffers

Two formats were already in use:
- `.srsj` (SRS JSON): a JSON bundle — a single JSON document embedding all records and package
  definitions. It is lightweight and human-readable, but it is a projection (not a lossless copy),
  it embeds the serializer's indentation choices, and its HashMap-derived key ordering is
  non-deterministic across runs without the `preserve_order` feature explicitly disabled.
- File-tree (directory): the native on-disk layout — canonical but not portable as a single file.

Neither satisfied all three use cases above simultaneously. A third format was needed.

## Decision

The `.srs` file is a ZIP archive (SRSzip) containing a file-tree that mirrors the native on-disk
repository layout. The archive is produced by `archive_pack` and consumed by `archive_unpack` in
`srs-repository::archive`.

**Determinism guarantee:** byte-identical output for the same repository state:
- All ZIP entries are sorted lexicographically by path.
- All ZIP timestamps are zeroed (`zip::DateTime::default()`).
- Compression: Deflate (level default).
- All JSON is serialized via `serde_json::to_value` → `serde_json::to_vec_pretty`, which routes
  through `serde_json::Map` (BTreeMap-backed when `preserve_order` is disabled — ADR-017).
  This sorts all object keys, eliminating HashMap insertion-order variance across process runs.

**Archive contents:**
| Path | Contents |
|---|---|
| `manifest.json` | Full repository manifest (instanceIndex, repositoryId, namespace, …) |
| `package/package.json` | Primary package manifest |
| `package/package.snapshot.json` | `PackageBoundarySnapshot` — types, fields, views, etc. |
| `relations/relations-collection.json` | Relations (omitted when empty) |
| `records/…/*.json` | Instance files at their manifest-indexed paths |

**Restore contract:** `archive_unpack` rebuilds the repository at a new target via
`import_repository_snapshot`, which canonicalises instance paths. The restored repository is
semantically identical to the source (same IDs, same records) but not byte-identical on disk —
`import_repository_snapshot` uses canonical slug-based file names.

**Library shape:** three public functions in `srs-repository`:
- `archive_pack(source: &dyn RepositoryStore, writer: impl Write + Seek) -> Result<(), RepositoryError>`
- `archive_unpack(reader: impl Read + Seek, target: &dyn RepositoryStore) -> Result<(), RepositoryError>`
- `archive_to_vec(source: &dyn RepositoryStore) -> Result<Vec<u8>, RepositoryError>` (WASM-friendly wrapper)

## Consequences

**Positive:**
- Single-file transfer and storage for any repository, regardless of size.
- Deterministic output enables byte-level deduplication and content-addressed caching.
- WASM consumers can call `archive_to_vec` without file I/O.
- Existing ecosystem tooling (unzip, zipinfo) can inspect `.srs` archives.

**Negative / trade-offs:**
- Two serialization passes per record (load JSON, re-serialize for determinism). Acceptable for
  the expected repository sizes; revisit if performance matters at > 100 k records.
- The canonical path used inside the ZIP comes from the manifest `instanceIndex`, not re-derived
  from the record content — so archives from repositories with non-canonical paths preserve those
  non-canonical paths. `archive_unpack` canonicalises on restore.

**Neutral:**
- `.srs` is distinct from `.srsj`. Both are valid single-file forms; they serve different roles
  (see ADR-036).
- The `preserve_order` feature in `serde_json` must remain **disabled** for the determinism
  guarantee to hold (ADR-017).
