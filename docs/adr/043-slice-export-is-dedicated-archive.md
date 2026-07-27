# ADR-043: Container Slices Are Dedicated Export Archives, Not a Snapshot-Filter Mode

- **Status:** proposed
- **Date:** 2026-07-27
- **Supersedes:** —
- **Superseded by:** —

## Context

RFC-026 (`ext:slices`) defines a **container-membership closure** export that produces a valid `.srs`
archive. The original issue description (srs-rust#631) suggested extending `ExportSnapshotOptions`
with a `slice: Option<SliceSpec>` — i.e., treating slice export as a filter applied inside the
existing `export_repository_snapshot_with_options` path.

Two implementation approaches were evaluated:

**Option A — Snapshot filter**: extend `ExportSnapshotOptions` with `slice: Option<SliceSpec>`;
`export_repository_snapshot_with_options` conditionally filters its output. `archive_pack` then
passes the filtered snapshot to the ZIP writer.

**Option B — Dedicated service function**: a new `export_container_slice(store, ContainerSliceInput, writer)` function in `slice_service.rs` that:
1. Calls `archive::tree_entries(source)` to get the full tree.
2. Filters that tree (manifest, instance files, relations) to the container's closure.
3. Writes the filtered tree as a deterministic ZIP directly.

Several forces push toward Option B:

- **Snapshot is a codec, not the operational model.** ADR-038/ADR-039 established that the
  archive format is a file tree, not a snapshot round-trip. Filtering through the snapshot
  layer would re-canonicalize instance paths (exactly the ADR-039 regression), losing the
  "layout-faithful" property.
- **The slice manifest differs structurally from the source manifest** — it has a new
  `repositoryId`, a `slice` block, a filtered `instanceIndex`, and a filtered `containerIndex`.
  These mutations are easier to express against a `serde_json::Value` parsed from the
  existing `manifest.json` bytes than wired through the snapshot DTO.
- **`ExportSnapshotOptions` is the `.srsj` / portability bridge** (ADR-008/036). Adding a
  slice filter there confuses its scope: the snapshot is the RFC-014 portability engine;
  slices are RFC-026 subset archives. Keeping them separate prevents cross-contamination.
- **Scope clarity**: the issue comments (2026-07-21) split package export (RFC-003) from
  container slices (RFC-026) — `PackageBoundary` is removed from the slice type; only
  `Container` closures are in scope. A dedicated function makes this split concrete.

## Decision

1. **Container slice export is a dedicated service function**, not a mode on
   `export_repository_snapshot_with_options`. The function is:
   ```rust
   pub fn export_container_slice(
       source: &dyn RepositoryStore,
       input: ContainerSliceInput,
       writer: impl Write + Seek,
   ) -> Result<ContainerSliceResult, RepositoryError>
   ```
   Located in `crates/srs-repository/src/slice_service.rs`.

2. **Implementation reuses `archive::tree_entries`** (the single authoritative faithful
   store→tree enumeration, ADR-040) to get the full file tree, then filters it. This avoids
   duplicating the enumeration logic and benefits from any future improvements to
   `tree_entries` automatically.

3. **`ExportSnapshotOptions` is not extended.** It retains its current single field
   (`include_content_blobs: bool`) and its role as the portability/`.srsj` codec bridge.
   Future snapshot-level options are unaffected.

4. **CLI surface is `srs slice export --container <id> <output.srs>`** — a new top-level
   subcommand group, not an extension of `srs archive pack`. This separates full-repo archive
   from subset export in the CLI UX and leaves room for future `srs slice import` etc.

5. **`SliceSpec`, `SliceExternalRef`, `Slice`, `SliceOrigin`** are canonical spec types and
   live in `srs-core/src/types/slice.rs` (no schemars, no file I/O — the srs-core constraints
   are satisfied).

## Consequences

**Positive:**
- Archive layout remains layout-faithful: no snapshot round-trip, no path re-canonicalization.
- Separation of concerns: snapshot/portability codec is unchanged; slices are a new export path.
- Service function is independently testable against MemoryStore.
- CLI UX is clean: `srs slice export` is a named, distinct operation; `srs archive pack` continues
  to mean "full repo archive".
- Future `srs slice import` or `srs slice list-refs` commands can live under the same subcommand
  group.

**Negative / trade-offs:**
- `slice_service.rs` cannot share the `write_zip_from_tree` logic with `archive_pack` without
  extracting it. A helper `write_zip_entries(tree, writer)` should be extracted from `archive_pack`
  into a `pub(crate)` function in `archive.rs` to avoid duplication. This is a small internal
  refactor (ADR-010 DRY principle).
- Two code paths for producing `.srs` archives (`archive_pack` and `export_container_slice`)
  must stay consistent on ZIP determinism settings (sorted, zeroed timestamps, Deflated).
  Mitigated by the shared `write_zip_entries` helper.

**Neutral:**
- `ExportSnapshotOptions` is not deprecated; it continues to serve its existing callers.
- The snapshot machinery remains the codec bridge for `.srsj` and full-snapshot portability.
- CLI payload struct (`SliceExportPayload`) follows ADR-011 (named struct in `payload.rs`).
- Record closure and `PackageBoundary` slice types remain explicitly deferred (no code changes
  for them implied or staged by this ADR).
