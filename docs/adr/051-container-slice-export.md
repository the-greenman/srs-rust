# ADR-051: A container slice is a dedicated filtered tree, not a mode of `archive_pack`

- **Status:** accepted
- **Date:** 2026-10-04
- **Supersedes:** —
- **Superseded by:** —
- **Related:** ADR-033/039 (`.srs` is a deterministic ZIP of the repository tree), ADR-045
  (ordinary operations read the checked catalog), ADR-050 (`.srspkg`: package export is not a
  slice, RFC-026 [R10]).

## Context

srs-rust#631 (folds in #656, supersedes #633): export one container as a standalone, valid `.srs`
per RFC-026 (Revision 8, amended by RFC-038 [R25] and RFC-043). Consumers: `srs slice export` and
the WASM `export_slice(containerId)` for srs-web's Export snapshot (srs-web#417). The July plan
made the slice an `ExportSnapshotOptions` mode of `archive_pack`; that predates RFC-038 (no
`instanceIndex`) and RFC-043 (container entries).

RFC-026 Rev 8 leaves three choices open; the owner ruled them on 2026-10-04 (issue #631), and an
RFC-026 Revision 9 amendment is drafted in parallel to match:

- **D1 = P3.** Change C copies only Types and Fields into one merged `package/`. Packages now
  also hold lifecycles, vocabularies, relation types, compositions and more (an unresolved
  `lifecycleRef` is an error), and a merged or pruned package would keep or invent an identity
  that is not true.
- **D2.** The boundary becomes the root container, whose identity entry RFC-043 requires to be
  a depth-0 entry without descendants; a non-root container's need not be.
- **D3.** Change C step 6 ("all entry ids within the included set") is vacuously true for an
  empty container. The RFC-026 Rev 9 draft (srs#886) then found that steps 2 and 6 were
  replaced on 2026-09-06 by the accepted RFC-034 [R9] (Invariant I-151) and never marked. The
  owner ruled on 2026-10-04 that container closure follows RFC-034 [R9] / I-151, which makes D3
  moot.

## Decision

One core service, `srs_repository::slice_service::export_container_slice(store,
ExportSliceInput) -> SliceExport { bytes, summary }`, with two thin adapters (CLI `srs slice
export --container <id> <out.srs>`, WASM `SrsRepository::export_slice`).

1. **A filter over the faithful enumeration.** The service takes `archive::tree_entries` (the
   one store→tree enumeration) and keeps only the slice's paths at their source locations,
   bytes unchanged; the manifest is the only rewritten file. The ZIP is written by
   `archive::pack_tree`, split out of `archive_pack` so there is one deterministic writer.
   `archive_pack` keeps its signature and stays the whole-repository snapshot.
2. **Checked catalog (ADR-045).** Membership comes from `store.catalog()`; an incoherent
   repository (a dangling entry, say) is refused, never silently cut. The boundary's entries
   still pass through `arrangement::retain_promoting` (RFC-043 [R18]).
3. **Closure (RFC-034 [R9], I-151).** The carried containers are exactly the boundary and its
   declared `childContainerIds` descendants, transitively. Included instances = the entry ids of
   every one of them (`effective(boundary)`). A declared child with no entries is still carried;
   an undeclared container is never carried, even when all its entries are included.
   Membership is declared, never derived (RFC-034, `rfc-decision-0750c62f`): carrying a
   container because its entries happen to fall inside the slice would derive structure the
   source never declared, and the subset test was vacuously true for every empty container.
   `childContainerIds` are copied as-is, and every id they name is carried. A child id naming no
   container (`slice-child-container-missing`) or a cycle (`slice-child-container-cycle`)
   refuses the export: [R7] leaves `effective(C)` undefined over a broken graph and forbids a
   partial result, and a slice must not copy a dangling id. Relations with both endpoints
   included are carried; with exactly one, recorded in `slice.externalRelationRefs` (sorted by
   `relationId`); with none, omitted. Source documents
   cited by included instances **and included relations** are carried (sidecar, plus content
   unless tombstoned).
4. **Packages (D1 = P3).** Every package the slice uses is carried whole and unchanged at its
   source path. "Use" is closed at package granularity: the packages holding the included
   records' Types and the included relations' RelationTypes, then every package a carried
   package's definitions reference (the RFC-003 PINNED/LINEAGE reference sites) or name in
   `packageDependencies` (RFC-044). The primary `package/` is always carried. `packageRefs`
   to packages not carried are dropped from the manifest.
5. **Manifest.** Source properties are kept except: a new `repositoryId` ([R3]), the boundary as
   `container` ([C] step 1; its `containers/` file is not written), `ext:slices` appended to
   `declaredExtensions` ([R4]), the `slice` block, and the `packageRefs` filter above. The
   source root container, when it is a declared descendant, is written to
   `containers/<id>.json`. `.srs/` carries only the marker.
6. **Refusals** (`RepositoryError::SliceRefused { code }`): `slice-root-identity-invalid` when
   the boundary's identity entry is absent, below depth 0 or has descendants (D2: the outline
   is never rewritten); `slice-exported-at-invalid`; `slice-repository-id-reused`;
   `slice-root-level-package-unsupported` (a package rooted at the repository root);
   `slice-child-container-missing` and `slice-child-container-cycle` (RFC-034 [R7]);
   `slice-definition-identity-conflict` (two carried packages hold different definitions under
   one `id` and `version`; rfc-decision-cce3c00e; compared by the package export's PD3 check)
   and `slice-package-outside-repository` (a carried local package path resolving outside the
   repository root, which an archive cannot hold, RFC-017) (RFC-026 Rev 9 Q6, #1263).
   Both refuse before anything is written.
7. **Validator (Change E).** With a `slice` block: `spec.type` not `container` is an error
   ([R10]), `spec.id` not the root container is an error ([R12]), a reused `repositoryId` is an
   error ([R3]), undeclared `ext:slices` is a warning ([R4]), and one info diagnostic counts the
   cut edges (Change E 1). An absent Container named by a Composition section, and an
   unresolved Composition `rootTypeRefs` entry, drop from warning to info (Change E 2/3).
   I-81 (root identity not a `purpose` record) is info inside a slice, still a warning outside
   one (Rev 9 Q4, [R17], #1263).

## Consequences

**Positive:** a slice is always self-contained and every carried package keeps its true
identity; the same bytes from CLI and WASM; deterministic given `exportedAt` and the slice id.
Dogfooded over every container of a muSrs copy: 0 errors each.

**Negative / trade-offs:** carrying whole packages ships definitions no included record uses
(Rev 9 changes RFC-026's "what is excluded" list). The boundary becomes a repository root, so
RFC-013/RFC-018 root rules apply to it in full ([R12]): slicing a content container whose
identity record is not a `purpose` is I-81 at info (Rev 9 Q4), and one with carried sub-containers can report
root members that anchor no container (I-82; under the subset rule this fired 153 times on
muSrs "The case", whose undeclared sub-containers I-151 no longer carries). Inside a slice I-82 is
info too, like I-81 (RFC-026 Rev 10, srs#894; `slice_expected_severity`), and a warning outside one.
A composition in a package the slice does not use is not
carried even if it could render the slice's records (references are followed forward only).

Manifest
properties other than `packageRefs` (e.g. a legacy `changelogPath`) are kept even if their
target is not carried. A Type held only by a package no `packageRefs` names is not carried; the
validator reports the slice, as it would the source.

**Neutral:** the CLI and WASM adapters take no `exportedAt`/`repositoryId` override, so their
output differs per run by those two values only (RFC-026 [R3] requires the fresh id); the
service is byte-deterministic given both. No import, merge or reintegration; no MCP tool; record-level closure stays deferred
([R10]). `srs package slice-create` (an alias of `package create`, #656) is removed.

## Implementation charter (ADR-048)

- [x] **Spec-first** — RFC-026 Rev 8 Changes A–E, [R1]–[R14], amended by RFC-038 [R25] and
      RFC-043 [R18]; container closure per RFC-034 [R9] / [R7] (I-151), which replaced Change C
      steps 2 and 6; D1/D2 owner rulings on #631, RFC-026 Rev 9 pending (srs#886; shipped ahead
      of the amendment, as #1210 shipped ahead of RFC-003 Rev 10).
- [x] **Layer test** — core service `slice_service`; CLI `srs slice export` and WASM
      `export_slice` are adapters only.
- [x] **One way per goal** — `tree_entries` is the one enumeration, `pack_tree` the one ZIP
      writer, `retain_promoting` the one promoting removal, `reference_sites` the one reference
      table.
- [x] **Parity and mirror obligations** — `SliceExportPayload` golden
      (`schemas/payload/slice-export.json`); `slice` schema defs already mirrored; no
      `dataModelRevision` bump.
- [x] **Decision mode** — complicated (spec-guided, owner-ruled forks).
