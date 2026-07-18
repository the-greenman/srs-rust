# ADR-035: Flat Export Bundle — Gate C Decision Export Format

- **Status:** proposed
- **Date:** 2026-07-18
- **Supersedes:** —
- **Superseded by:** —

## Context

Gate C of the attachments epic (#271) requires `srs-gov export-decision <id>` to produce a
shareable bundle: the decision's rendered document view plus its linked attachment files, packaged
in a ZIP that can be opened by anyone without SRS tooling.

Two bundle formats were considered:

**Option A — Flat ZIP**: `decision.md` (rendered markdown) plus `attachments/<filename>` entries
for each linked source-document. Simple, human-readable, no SRS knowledge required to open it.
Cannot be re-imported into SRS as a repository.

**Option B — `.srs` subset**: a valid `.srs` archive (per ADR-033) containing only the exported
record and its attachments. Re-importable into SRS. More complex — requires the
`archive_pack`/`archive_unpack` path and a format decision about whether partial repo archives
are valid (they are not described by RFC-017 as of Gate C scope).

The gate C acceptance criterion is "is the exported bundle what a user needs to share a decision
off-platform?" — the primary audience is people who do not have SRS installed. Re-importability
is a Gate D or future concern, not a Gate C requirement.

Issue #287 (3.2) flagged "Decide: valid `.srs` subset vs flat export" as an open question. This
ADR settles it.

## Decision

Use **Option A (flat ZIP)** for Gate C:

- `decision.md` — the rendered markdown from `render_service` (via `RenderDocumentViewOptions { instance_id_filter: Some(&id) }`).
- `attachments/<basename>` — raw bytes for each source-document with `sourceRole: attaches`, where `<basename>` is the last path component of the `content_path` from the manifest source-document index.
- Entries are sorted lexicographically (same determinism invariant as ADR-033).
- Timestamps zeroed (`zip::DateTime::default()`), Deflate compression.
- File extension `.zip` (not `.srs` — the format is distinct from ADR-033's archive).

The service function signature mirrors ADR-033's `archive_pack` pattern:
```rust
export_record_bundle(
    store: &dyn RepositoryStore,
    input: ExportBundleInput,
    writer: impl Write + Seek,
) -> Result<ExportBundleMetadata, RepositoryError>
```

## Consequences

**Positive:**
- Immediately useful for sharing decisions off-platform with non-SRS users.
- Simple to implement — no partial-archive semantics to define.
- Deterministic output (same record + attachments → identical bytes across runs).

**Negative / trade-offs:**
- Cannot be re-imported into SRS as a repository. This is an intentional Gate C scope limit.
- A `.srs` subset format (for round-tripping) would require a spec amendment (RFC-017 only defines full-repository archives). That work is deferred to a future RFC and Gate D or later.

**Neutral:**
- The flat ZIP uses the same `zip` crate dependency as `archive.rs`.
- `export_service.rs` is a new file in `srs-repository`; it does not modify `archive.rs`.
