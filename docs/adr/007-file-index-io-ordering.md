# ADR-007: File-Before-Index Ordering for File-Backed Entity CRUD

- **Status:** accepted
- **Date:** 2026-05-29
- **Supersedes:** —
- **Superseded by:** —

## Context

Several SRS repository entities are stored as individual JSON files on disk and tracked by an index — currently `manifest.json → instanceIndex` for records, and `manifest.json → extra.containerIndex` for containers. Any CRUD operation that modifies both a file and its index entry must choose an ordering, and that ordering determines which failure mode is possible when the process is interrupted between the two writes.

There are two options:

**Option A — File first, then index (write file, then update index)**

- Create: write the entity file, then append to index. If the index write fails, the file exists but is not indexed — it is invisible to all read operations.
- Delete: remove the file, then remove from index. If the index write fails, the index references a missing file — every subsequent `list` or `get` that encounters the entry will fail to load it.

**Option B — Index first, then file (update index, then write file)**

- Create: append to index, then write the entity file. If the file write fails, the index references a file that does not yet exist — every subsequent `list` or `get` that encounters the entry will fail to load it.
- Delete: remove from index, then remove the entity file. If the file removal fails, the file exists on disk but is not indexed — it is invisible (orphaned), but no read operation will encounter it.

The key asymmetry:

| Scenario | File-first (A) | Index-first (B) |
|---|---|---|
| Create interrupted after file write | Orphaned file — invisible, harmless | Dangling index entry — every list/get errors |
| Delete interrupted after index update | Dangling index entry — every list/get errors | Orphaned file — invisible, harmless |

The worst failure mode in both options is a **dangling index entry**: the index points to a file that does not exist, causing load errors on every subsequent read of that entity. Orphaned files are harmless to readers and can be found by scanning the directory against the index.

No atomic cross-file transaction mechanism is available in the target environment (plain filesystem, no SQLite, no WAL).

## Decision

Use **file-first ordering for create** and **index-first ordering for delete**, chosen to avoid dangling index entries in both cases:

- **Create:** write the entity file first, then update and persist the index. An interrupted create leaves an orphaned file (unindexed, invisible). The index remains consistent.
- **Delete:** update and persist the index first (removing the entry), then remove the entity file. An interrupted delete leaves an orphaned file (present on disk, not indexed). The index remains consistent.

In both cases the index is the authoritative membership record. A file on disk that is not in the index is not a member of the repository. Orphaned files are recoverable by a future `srs repo repair` scan (compare `containers/` directory against `containerIndex`); dangling index entries require manual surgery and cause visible errors until fixed.

This principle applies to all file-backed indexed entities in `srs-repository`:
- Container files vs. `manifest.extra.containerIndex`
- Instance record files vs. `manifest.instanceIndex`
- Any future file-backed index (source documents, views, etc.)

## Consequences

**Positive:**
- The index is always internally consistent — a `list` or `get` operation will never encounter an index entry that fails to load due to a missing file caused by an interrupted CRUD operation in this codebase.
- Orphaned files are safe and detectable. A directory scan of `containers/` vs. `containerIndex` reveals any orphans without requiring log analysis.
- The rule is simple and uniform — implementors do not need to reason about per-operation ordering.

**Negative / trade-offs:**
- Create interrupted between file write and index update leaves a file on disk that wastes space and may confuse manual inspection. The orphan is harmless to program correctness but requires a repair scan to identify.
- Delete interrupted between index update and file removal leaves a file on disk indefinitely. It will not be garbage-collected automatically.
- This is not a substitute for transactional semantics. A process crash during the index write itself (mid-`write_manifest`) can still corrupt `manifest.json`. That risk is accepted — it is the same risk present in all current manifest writes and requires a broader journalling solution outside this ADR's scope.

**Neutral:**
- A `srs repo repair` command (future work) should scan all indexed directories against their index entries and report orphaned files and dangling entries. This ADR's ordering guarantees that `repair` will only find orphaned files (safe), never dangling entries (errors), for operations following this rule.
- Callers that observe a failed create or delete should not assume the repository state is inconsistent — the index will be coherent, though a file may be orphaned.
