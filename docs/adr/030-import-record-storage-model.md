# ADR-030: Import Record Storage Model

- **Status:** proposed
- **Date:** 2026-07-16
- **Supersedes:** —
- **Superseded by:** —

## Context

RFC-014 defines `ImportRecord` (per-definition provenance) and `ImportSummary` (aggregate view).
These records must be stored durably so that `srs package imports` can reconstruct them without
re-scanning definition files, and so that divergence detection can compare the current state of
each definition against the version that was present at install time.

Two questions needed decisions:

1. **Where are import records stored?** Options: (a) per-boundary file, (b) single repo-wide file,
   (c) embedded in each definition file.

2. **How is divergence detection implemented?** Options: (a) store reference JSON copies, (b) store
   content hashes, (c) always re-fetch from upstream.

The issue comment for #246 explicitly rules out (b): "never a stored hash". Option (c) requires
`ext:registry` which is out of scope for this epic. That leaves (a) for divergence: storing
reference JSON copies.

For storage location, per-boundary (option a) keeps each boundary self-describing and aligns with
the existing package boundary model (ADR-009): a boundary directory contains everything needed to
understand that boundary without reading other parts of the repository.

## Decision

1. Import records are stored in **`<boundary-path>/.srs-import/import-records.json`** as a
   serialized `ImportSummary` struct. The boundary path is derived from the `PackageSelector`
   exactly as in the rest of `package_service.rs` (via `store.list_package_boundaries`).

2. Reference JSON copies of each installed definition are stored at
   **`<boundary-path>/.srs-import/refs/<kind>/<file>.json`** where `<kind>/<file>` mirrors the
   relative path of the definition within the boundary (e.g. `fields/title.json`).

3. Both are written via the existing `store.save_instance_json` / `store.load_instance_json` store
   methods. No new store trait methods are introduced — the `.srs-import/` path convention is owned
   by `package_service.rs`, not by the store adapter.

4. Divergence detection in `list_package_imports` compares `current_json == reference_json` using
   `serde_json::Value` structural equality. Match → `Clean`; mismatch → `LocalAhead`.
   `UpstreamAhead` and `Diverged` states require registry access and are deferred.

5. Boundaries that pre-date this feature (no `.srs-import/` directory) are silently skipped by
   `list_package_imports` — they contribute no records to the summary.

## Consequences

**Positive:**
- Boundary directories are self-describing: all import provenance lives alongside the definitions.
- No new store trait methods needed; MemoryStore and FileStore work without changes.
- Divergence detection is pure content comparison — no hashing, no registry call.
- Backward compatible: old boundaries without `.srs-import/` are silently skipped.

**Negative / trade-offs:**
- Reference copies double the on-disk storage for installed definitions inside the boundary.
- `list_package_imports` loads `import-records.json` and then potentially many reference files per
  boundary — O(n) I/O where n is the number of installed definitions. Acceptable for current sizes.
- The `.srs-import/` path is a convention in service code, not an explicit store abstraction. A
  future SQL store would need to handle this path as a special case or add dedicated methods.

**Neutral:**
- `package import` (local directory import) does not store reference copies because there is no
  canonical "upstream" to compare against — divergence detection is not applicable.
- `update_available` is always `None` until `ext:registry` is implemented (deferred to epic #243).
