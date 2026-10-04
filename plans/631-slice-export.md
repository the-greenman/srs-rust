# Plan: RFC-026 container slice export (srs-rust#631)

> Closes #631 (folds in #656; supersedes #633). Spec: RFC-026 Revision 8 (`srs/rfcs/rfc-026-ext-slices-subset-export.md`,
> amended by RFC-038 [R25] and RFC-043), `docs/schema/2.0/manifest.json` `$defs.Slice`/`SliceSpec`/`SliceExternalRef`
> (already in the srs-rust mirror). Consumer: srs-web Export snapshot (the-greenman/srs-web#417), driving use case the
> essay snapshot bundle (the-greenman/muDemocracy.org#276). Background only: the July plan on
> `feat/631-container-slice-export` (pre-RFC-038/043, 534 commits stale).
>
> **STATUS: IMPLEMENTED (2026-10-04).** Owner rulings on #631: D1 = P3 (every package the slice uses, carried
> whole, closed at package granularity; RFC-026 Rev 9 drafted in parallel), D2 = refuse, D3 = non-empty only;
> defaults accepted; ADR-051 accepted. Phases 1-4 done.
>
> **AMENDED (2026-10-04, owner ruling on PR #1259):** container closure follows the accepted RFC-034 [R9] /
> Invariant I-151, not Rev 8's subset rule (Change C steps 2 and 6 were replaced at RFC-034's acceptance,
> found by the Rev 9 draft, srs#886). The slice carries the boundary and exactly its declared
> `childContainerIds` descendants, transitively, and includes every entry id of each. A declared memberless
> child is carried; an undeclared container never is. A missing child or a cycle refuses the export
> (RFC-034 [R7]). D3 and F1/F2 are moot.

## Summary

`archive_pack` exports the whole repository; there is no way to export one container as a standalone,
valid `.srs`. This plan adds one core service, `slice_service::export_container_slice`, that computes the
RFC-026 container closure over the tree-authoritative catalog, assembles the slice as a path-to-bytes tree
(reusing `archive::tree_entries`), rewrites the manifest (new `repositoryId`, the boundary container as
`manifest.container`, `ext:slices`, the `slice` block with `externalRelationRefs`) and zips it through the
same deterministic writer as `archive_pack`. Two thin adapters: `srs slice export --container <id> <out.srs>`
(new `srs slice` group; the bogus `srs package slice-create` alias is deleted) and the WASM
`export_slice(containerId)` returning the `.srs` bytes. The validator gains the RFC-026 slice-block checks
and the Change E relaxations. No import, merge or reintegration.

## Spec gate (Stage 1.5)

No new RFC is needed **if** D1 is ruled to the RFC-literal option (P2). Options P1/P3 deviate from Change C's
text and would need an RFC-026 Revision 9 amendment (Door 3) before this implementation merges.

### Spec findings (made while planning)

- **F1. Rev-8 step 2 fixpoint is vacuous.** "For each Container whose entry ids are a subset of the
  included set, include its entry ids" adds nothing (they are already included). At revision >= 8 the
  included instance set is exactly the boundary container's entry ids. Harmless; implementation follows it.
- **F2. Empty containers satisfy step 6 vacuously.** A container with no entries has "all its entry ids
  within the included set", so every slice would carry it (muSrs "Problem grid" has 0 entries). See D3.
- **F3. The closure names only Types and Fields.** Current packages also carry relation types, lifecycles,
  vocabularies, compositions, views, themes, blueprints and protocols. A Type's `lifecycleRef` and a Field's
  `vocabularyRef` are errors when unresolved (validation V8/V2), so a types-and-fields-only closure produces
  invalid slices whenever those kinds are used. See D1.
- **F4. "Copied into the slice archive's `package/` directory ... even if sourced from multiple packages."**
  The RFC flattens multi-package sources into one `package/` but does not say what the flattened
  `package.json` identity is (`id`, `namespace`, `name`, `version`), and RFC-044 `packageDependencies` and
  RFC-014 `upstreamPackage` of the sub-packages have no home after flattening. muSrs has 7 package
  boundaries (`package/` + 6 `packages/*`). See D1.
- **F5. Root identity rule.** RFC-043: the root container's identity entry must be depth 0 with no
  descendants. A non-root boundary container's identity entry need not be. See D2.
- **F6.** Source documents referenced only by an included **relation's** `sourceRefs` are not covered
  (Change C step 5 says "included instances"). Default below: include them.
- **F7.** RFC-034 `childContainerIds` of an included container may name containers outside the slice.
  Not covered; the validator does not error on it today. Default below: copy verbatim (Change E item 3).

### Prototype evidence (python, against a temp copy of muSrs, validated with this branch's binary)

| Boundary container | Instances | Relations kept | Cut (`externalRelationRefs`) | Errors |
|---|---|---|---|---|
| Tensions (identity + anchor) | 16 | 15 | 48 | 0 |
| Guides (identity, `childContainerIds`) | 6 | 8 | 22 | 0 |
| Decision Log (no identity) | 15 | 1 | 48 | 0 |

Packages carried whole, and again with unreferenced Types pruned: 0 errors either way. Warnings are the
expected absences (compositions naming containers/types outside the slice) — Change E items 2/3.

## Design decisions (owner input needed)

### D1. Package shape of the slice (F3, F4) — long-term artifact format

- **P1 Preserve boundaries, prune.** Keep `package/` and every `packageRefs` boundary, each rewritten to drop
  the Types and Fields outside the closure (closure = included records' `typeId@typeVersion`, followed through
  the RFC-003 reference-site table `reference_sites::followed_references`, so supertypes, composite-range
  Types, lifecycles and vocabularies resolve); other kinds carried whole. Cost: a pruned package keeps its
  source `id@version` with different content (a false identity claim); needs RFC-026 Rev 9.
- **P2 Flatten into `package/` (RFC-literal).** One `package/package.json`, every definition from every
  boundary merged in, Types/Fields pruned as in P1, `packageRefs` removed. Open sub-choices: the flattened
  package's identity (source primary package's, or a fresh UUID like the slice's `repositoryId`), what happens
  to sub-package `packageDependencies`/`upstreamPackage` (dropped), file collisions (dedupe by `id@version`,
  refuse on an RFC-003 identity conflict as `export_package_bundle` does).
- **P3 Carry every package boundary whole.** No pruning, no flattening: the package set is the source's,
  byte-identical. Simplest (reuses `tree_entries`' package enumeration), always self-contained, package
  identity honest, validated 0 errors on all three prototypes. Cost: violates Change C's "What is excluded"
  bullet for unreferenced Types/Fields and the single-`package/` wording; needs RFC-026 Rev 9 ("a slice
  carries the source's package set unchanged").

**Recommendation: P3 + an RFC-026 Revision 9 amendment.** A slice is a snapshot; a pruned or merged package
that keeps (or invents) a package identity is the one thing in it that would not be true, and the RFC's
types-and-fields closure is already incomplete for today's definition kinds (F3). If the owner prefers the
RFC-literal path, P2 with a fresh package UUID and the reference-site closure is the fallback.

### D2. Boundary container whose identity entry is not a depth-0 leaf (F5)

Recommendation: **refuse the export** with `slice-root-identity-invalid` naming the container and entry
(no silent outline rewrite, no dropped identity). Reversible later; no muSrs container hits it (all depth 0).

### D3. Empty containers (F2) — moot, superseded by RFC-034 [R9] (see the amendment above)

Recommendation: a sub-container is included only when it has **at least one** entry and all its entry ids are
inside. File the vacuous-truth reading as a spec finding on RFC-026.

### Defaults taken unless overruled (no owner input requested)

- Source documents: sidecars (and content, unless tombstoned) referenced by included instances **and included
  relations** (F6).
- `childContainerIds` copied verbatim (F7).
- Every container in the slice is passed through `arrangement::retain_promoting` against the included
  instance set (RFC-043 [R18]/[R7]); a no-op unless the source has dangling entries (F1).
- Manifest: every source property kept except the RFC-mandated rewrites (`repositoryId`, `container`,
  `declaredExtensions` += `ext:slices`, `slice`); `dataModelRevision` stays the source's. The boundary
  container's own `containers/*.json` file is not written (it is inline). If the source root container is
  a declared descendant of the boundary, it is written to `containers/<id>.json`.
- `exportedAt` defaults to now; the service input takes an optional override (deterministic tests).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | main loop |
| Repository Worker | main loop (srs-repository) |
| CLI Worker | main loop (srs-cli) |
| Bindings Worker | main loop (srs-bindings) |
| Architecture Reviewer / Plan Reviewer / Verification | subagents (Stages 3, 7) |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| ADR-010 / ADR-011 | Service in `srs-repository`, typed in/out; `SliceExportPayload` named struct, golden regenerated | accepted |
| ADR-013 | WASM binding is a thin wrapper over the same service | accepted |
| ADR-038 / ADR-039 | Slice assembled as a path-to-bytes tree from `tree_entries`, zipped by the shared deterministic writer | accepted |
| ADR-045 | Ordinary operation: uses the checked `store.catalog()`, never the unchecked seam | accepted |
| ADR-048 | Spec-first (D1 deviation needs Rev 9), layer test (core service, thin adapters), one way per goal (one closure, one zip writer) | accepted |
| **ADR-051** | A container slice is a dedicated filtered tree, not a mode of `archive_pack`; records D1-D3 rulings | accepted |

## Contracts

### CLI output contract (ADR-011)

`srs slice export --container <id> <output.srs>` (global `--container` flag, already defined) →
`SliceExportPayload { output_path, file_size_bytes, container_id, slice_repository_id, origin_repository_id,
exported_at, instance_count, relation_count, container_count, source_document_count,
package_count, external_relation_ref_count }`. Regenerate `crates/srs-cli/schemas/payload/` with `cargo run --bin generate-schemas`.
Removed: `srs package slice-create` and its golden/test (breaking, sanctioned by #656).

### Service

```rust
pub struct ExportSliceInput { pub container_id: String, pub exported_at: Option<String>, pub repository_id: Option<String> }
pub struct SliceExport { pub bytes: Vec<u8>, pub summary: SliceExportSummary }
pub fn export_container_slice(store: &dyn RepositoryStore, input: ExportSliceInput) -> Result<SliceExport, RepositoryError>;
```
Errors: unknown container (`ContainerNotFound`), a checked-catalog refusal (ADR-045), `SliceRefused { code }`
(`slice-root-identity-invalid`, `slice-exported-at-invalid`, `slice-repository-id-reused`,
`slice-child-container-missing`, `slice-child-container-cycle`). `archive_pack` keeps its signature; its ZIP loop
moves to a shared `pack_tree(entries, writer)`.

### Entity schema sync

None: the `slice` defs are already mirrored.

## Phases

### Phase 1: core service (`srs-repository/src/slice_service.rs`)
- Closure (RFC-034 [R9]): carried containers = the boundary and its declared `childContainerIds`
  descendants; included = their entry ids ∩ catalog instance set; relations split (both in / exactly one in /
  none); source documents per step 5 + F6; packages per D1.
- Manifest rewrite on the raw JSON (unknown keys preserved); `slice` block; new UUID.
- Tests (MemoryStore + tree session): closure, cut edges recorded with `relationId`, both-outside omitted,
  declared descendants carried (memberless included) and undeclared never, missing child / cycle refused,
  promoting removal on a dangling entry, D2 refusal, determinism (fixed `exported_at`
  and a seeded id → identical bytes), slice validates with 0 errors after `archive_to_tree`.

### Phase 2: validator (Change E + slice-block checks) in `validation.rs`
- When `manifest.slice` is present: `spec.type != "container"` → error (R10); `spec.id` ≠
  `manifest.container.containerId` → error (R12); `repositoryId == slice.origin.repositoryId` → error (R3);
  `ext:slices` undeclared → warning (R4); one info diagnostic with the `externalRelationRefs` count (Change E 1).
- Composition-references-absent-container and rootTypeRefs-unresolved warnings become info in a slice
  (Change E 2/3). Tombstones already skip (Change E 4).

### Phase 3: adapters
- CLI `SliceCommand::Export { output }`, handler = one service call + write bytes; delete `SliceCreate`.
- WASM `SrsRepository::export_slice(container_id) -> Uint8Array`.

### Phase 4: docs + dogfood
- `srs/srs-usage.md` §5h command reference (srs branch), README CLI table, `docs/dogfooding.md` scenario
  (slice a muSrs container from a temp copy, validate the output), ADR-051 accepted.

## Final Acceptance

`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`,
`cargo test --test payload_contracts` all exit 0; a muSrs container slice validates with 0 errors.

## Assumptions

- Revision >= 8 corpora only (the binary refuses a revision-7 container shape, `Rfc043MigrationNeeded`).
- Out of scope: import, merge, reintegration, record closure, an MCP tool (file as follow-up if wanted).
