# ADR-039: `.srs` archive is a pure tree zip (no snapshot file)

- **Status:** proposed
- **Date:** 2026-07-22
- **Supersedes:** ADR-033 items 8–9 (snapshot-driven content model and the
  `package/package.snapshot.json` deviation); determinism requirements of ADR-033 remain in
  force
- **Superseded by:** —

## Context

ADR-033 defined `.srs` as a file-tree ZIP but implemented pack/unpack over the
`RepositorySnapshot` machinery, with two consequences that violate the spec's own definition
of the archive (ext:repository: "an archive is structurally identical to a repository
snapshot"; "a conforming implementation must be able to round-trip between a live repository
and an archive without data loss"):

- Pack **omitted per-definition files** (`package/fields|types/…/*.json`) and instead emitted
  a `package/package.snapshot.json` that only this implementation reads.
- Unpack re-canonicalized instance/definition paths and synthesized `package/package.json`
  (hardcoded metadata), so pack→unpack was not layout-faithful.

With ADR-038 the operational model **is** the file tree, so the archive can be exactly what
the spec says it is. Owner decision (2026-07-22): **no backwards compatibility** — existing
repos and archives are force-updated to the new form on their next save/export.

## Decision

1. **Pack**: a `.srs` archive is a deterministic ZIP of the exploded tree. For a MemVfs-backed
   store this is the `export_tree` map verbatim — including non-SRS files that rode along in
   the session (README, CI config): the archive is a faithful snapshot of the session tree.
   For disk stores the entry list is enumerated from the store: manifest raw, **every package
   boundary's** `package.json` raw plus every per-definition file each references (this
   closes the pre-existing gap where sub-package boundary files were never packed), relations
   raw, instanceIndex files, containerIndex files, source-document sidecars and binaries —
   deliberately *not* a blind directory sweep, so a git working tree's `.git/` or unrelated
   files are never archived from disk. The two enumerations intentionally differ in how they
   treat unknown files; both are complete for all SRS-owned content. A referenced file that
   does not exist is a **hard error naming the missing path** — never a silent skip.
   `package/package.snapshot.json` is **never written again**.
2. **Determinism** (unchanged from ADR-033): entries sorted lexicographically, timestamps
   zeroed, `Deflated` compression, no host metadata.
3. **Unpack**: native tree load first (`open_tree` over the unzipped map — layout-faithful,
   no re-canonicalization). A ZIP carrying `package/package.snapshot.json` without
   per-definition files takes the **legacy snapshot import path** — retained solely as the
   migration ramp so existing archives can be opened in order to be re-saved in the new form.
   The ramp's removal is a filed follow-up; no new code may depend on it.
4. `archive_unpack(reader, target)` keeps its signature for CLI unpack-to-disk;
   `archive_to_tree(reader) -> FileStore` is the in-memory entry point.

## Consequences

**Positive:**
- The archive now matches the spec's definition — a zip any tool can open, unpack, and
  `git init` into a working repository; lossless, layout-faithful round-trip.
- One content model: tree in the zip, tree in memory, tree on disk, tree on the git host.
- Definition-file edits survive archive round-trips (previously silently stranded).

**Negative / trade-offs:**
- Archives written before this change load only through the legacy ramp; once the ramp is
  removed they must have been re-saved. Accepted by owner (no-backcompat decision).
- Third parties that parsed `package.snapshot.json` (none known) lose that file.

**Neutral:**
- CLI payloads (`ArchivePackPayload`/`ArchiveUnpackPayload`) unchanged.
- The snapshot machinery itself is untouched — still the RFC-014 portability engine and the
  `.srsj` codec bridge.
