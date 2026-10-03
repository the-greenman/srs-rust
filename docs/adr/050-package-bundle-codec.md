# ADR-050: The `.srspkg` Package Bundle is a boundary codec with one reader and one writer

- **Status:** accepted
- **Date:** 2026-10-03
- **Supersedes:** —
- **Superseded by:** —
- **Related:** ADR-033/039 (`.srs`: a ZIP of a repository tree), ADR-013/038 (`.srsj`: a
  repository in one JSON envelope), ADR-035 (flat decision-export ZIP). A `.srspkg` is none of
  these: it carries package definitions only, is not a repository (RFC-003 [C2]), and is not a
  ZIP. ADR-025 (implicit core package) underpins the deviation below.

## Context

srs-rust#632 (export), #690 (bytes input) and #663 (WASM install) land together (owner decision,
2026-10-03): `srs package export` emits a Package Bundle (`docs/schema/2.0/package-bundle.json`,
`.srspkg`, RFC-003 Rev 5 Change C), `srs package install --bundle` and the WASM binding consume it,
and the round trip export -> install into a fresh repository -> identical definitions is the
acceptance test. srs-web pins `.srspkg` files by sha256 at build time (srs-web#339) and installs
them in the browser (srs-web#340), so the bytes must be reproducible and the reader must run
without a filesystem.

Until now the only install input was a package *directory* (`load_package_source_dir`, std::fs).
`rfc043_container_entries_migration_service::migrate_package_bundle_value` (the RFC-043 [R17]
pre-load transformer for bundles) exists with no non-test caller. The workspace has serde_json
`preserve_order` on (ADR-043), so key order is not sorted for free.

Forces: capability once in core (capability-layering), thin adapters (ADR-010/011/013), one way
per goal (ADR-048 rule 3), bundle forms stay out of the migration registry (ADR-032 scope rule),
honest refusal over silent coercion (RFC-038 [R21], ADR-044).

## Decision

1. **One codec module, bytes in, bytes out.** `crates/srs-repository/src/package_bundle.rs` owns
   `read_package_bundle(&[u8]) -> ReadPackageBundle` (the bundle plus non-fatal notes) and
   `export_package_bundle(&dyn RepositoryStore, ExportPackageInput) -> PackageBundleExport`.
   The reader never touches a store; the writer only reads one, through the single boundary
   loader `load_boundary_definitions` that install also uses. The CLI reads and writes the file at
   the process boundary (the `archive pack` precedent); the WASM binding passes the JSON text.
   Both install entry points (directory, bundle) end in the one `install_package_bundle` core.
   Inlined definitions get their boundary filename from the one `definition_rel_path` helper.
2. **Reader pipeline, in this order:** parse JSON -> refuse a `readme` (RFC-045 support is
   srs-rust#1164; carrying it silently would drop it) -> refuse a `dataModelRevision` greater than
   `CURRENT_DATA_MODEL_REVISION` (never silently downgrade) -> always run
   `migrate_package_bundle_value` (idempotent, never lowers a stamp above 8; its diagnostics
   become the install result's `notes`) -> validate against the embedded `package-bundle.json`
   (ADR-004) -> validate every definition with the loader's own strictness. There is **no numeric
   revision floor**: the floor is the content shape, and a failure on a bundle older than current
   names its revision and says to re-export it. Refusals are `InvalidPackageBundle { code,
   message }`, coded like `ActorProvenance` so clients can branch on the reason. Codes:
   reader `bundle-not-json`, `bundle-readme-unsupported`, `bundle-revision-too-new`,
   `bundle-migration-refused` (an RFC-043 transformer refusal, inner code kept in the message),
   `bundle-schema-invalid`, `bundle-definition-invalid`; writer `bundle-published-at-invalid`,
   `bundle-boundary-unreadable` (the target boundary's `package.json` cannot be loaded, so the
   writer never emits a silently empty bundle), `bundle-schema-invalid`.
3. **Writer determinism** ([C5]): definitions sorted by `id` within each kind array, optional kind
   arrays omitted when empty, every object key sorted by the `.srsj` writer's canonicalize step
   (ADR-043, reused, not copied), pretty-printed with a trailing newline. Definitions are carried
   verbatim in the [C4] sense: value-equal to the stored definition, with keys canonicalized.
   `publishedAt` is part of the bytes and is an input (default now), so **a reproducible sha256
   requires a fixed `publishedAt`** (`--published-at`); a consumer pinning hashes must pass it.
   The writer validates its own output against the schema and refuses to emit an invalid bundle.
4. **Hash format.** The reported checksum is `sha256:<64 lowercase hex>` over the file bytes, the
   format `attachment_service::sha256_hex` already produces. One format, not a second bare-hex one.
5. **Closure** ([C1], `mode: "bundled"`): starting from the boundary's definitions, any string
   value equal to the `id` of a definition in another package boundary of the same repository is
   inlined, to a fixpoint. `dependencyRefs` is `[]`.
6. **Identity** ([C3]/[C4]): whole-boundary export only; `packageId`/`packageVersion` are the
   boundary's `id`/`version`.

**Deviation (owner ruling 2026-10-03):** the embedded `com.semanticops.core` definitions are
**omitted** from bundles. RFC-003 [C1] and the schema's `bundled` description say a bundled bundle
inlines *all* referenced definitions; core definitions referenced by bundle content are neither
inlined nor listed in `dependencyRefs`. The bundle is therefore complete for any conforming SRS
repository (ADR-025 merges the core package into every one), but not self-contained in the literal
[C1] sense. An amendment to RFC-003 [C1] and the schema text is filed in `srs`; until it lands
this ADR is the record of the deviation.

Rejected: a directory-tree bytes input (`BTreeMap<String, Vec<u8>>`, #690's first sketch; owner
decision: the artifact is the spec'd bundle); a per-edge typed reference walker for the closure (a
second, partial copy of reference knowledge the loader already has; the id scan catches every edge
kind, including future ones); inlining the core package (owner ruling above); inferring the format
from the `.srspkg` suffix (the explicit `--bundle` flag is one way, the suffix would be a second);
a bare-hex checksum (a second format); a `standalone` mode and subset export (no consumer).

## Consequences

**Positive:** one reader serves CLI, WASM and any future MCP tool (srs-rust#1153); a pinned
sha256 stays valid across rebuilds when `publishedAt` is fixed; the RFC-043 bundle transformer
finally has its caller; install and export share one boundary loader, the single ADR-042 shim
migration point.

**Negative / trade-offs:** the id-scan closure would also inline a definition whose UUID appears
in a non-reference string (no such field exists today; a `ponytail:` comment names the upgrade to a
typed walker). Core-package references rely on ADR-025 rather than `dependencyRefs` (deviation
above). A bundle with a `readme` is refused until #1164. `collect_existing` now refuses a boundary
whose index lists an unloadable file instead of skipping it (such a repository already fails
`load_package()`), so one corrupt boundary now blocks every install into that repository.

**Neutral:** `migrate_package_bundle_value` no longer lowers a stamp above 8. Install upgrade
semantics (boundary version, import-summary merge) are unchanged and stay with srs-rust#1152.

## Implementation charter (ADR-048)

- [x] **Spec-first** — `package-bundle.json` (normative schema), RFC-003 Rev 5 Change C [C1]-[C5]
      (RFC still Draft; shipped under `gate:owner-merge` by owner ruling), RFC-043 [R17], RFC-044
      [R5]/[R8], RFC-045 (deferred, refused). Deviation from [C1] recorded above, amendment filed.
- [x] **Layer test** — core (`srs-repository::package_bundle`); CLI and WASM expose only.
- [x] **One way per goal** — one reader, one writer, one install core, one boundary loader, one
      filename helper (lifecycle/vocabulary creators still diverge: srs-rust#1209), one
      canonicalize, one checksum format.
- [x] **Parity and mirror obligations** — CLI payloads `package-export` (new) and
      `package-install` (`notes` added) regenerated; WASM returns the same service structs; no
      entity schema change; srs-web pin bump after release (srs-web#339/#340).
- [x] **Decision mode** — complicated.
