# ADR-050: The `.srspkg` Package Bundle is a boundary codec with one reader and one writer

- **Status:** accepted
- **Date:** 2026-10-03
- **Supersedes:** —
- **Superseded by:** —
- **Amended by:** srs-rust#1212 (RFC-003 Rev 10)
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
2. > Amended by srs-rust#1212: the no-floor rule is replaced by the RFC-003 [C6] step chain (see the Amendment).

   **Reader pipeline, in this order:** parse JSON -> refuse a `readme` (RFC-045 support is
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
5. > Amended by srs-rust#1212: the id scan is replaced by the typed reference-site table (see the Amendment).

   **Closure** ([C1], `mode: "bundled"`): starting from the boundary's definitions, any string
   value equal to the `id` of a definition in another package boundary of the same repository is
   inlined, to a fixpoint. `dependencyRefs` is `[]`.
6. **Identity** ([C3]/[C4]): whole-boundary export only; `packageId`/`packageVersion` are the
   boundary's `id`/`version`.

> Amended by srs-rust#1212: resolved by RFC-003 Rev 10 and `rfc-decision-a8dcbfe5` (core listed in `dependencyRefs`, never carried).

**Deviation (owner ruling 2026-10-03):** the embedded `com.semanticops.core` definitions are
**omitted** from bundles. RFC-003 [C1] and the schema's `bundled` description say a bundled bundle
inlines *all* referenced definitions; core definitions referenced by bundle content are neither
inlined nor listed in `dependencyRefs`. The bundle is therefore complete for any conforming SRS
repository (ADR-025 merges the core package into every one), but not self-contained in the literal
[C1] sense. An amendment to RFC-003 [C1] and the schema text is filed in `srs`; until it lands
this ADR is the record of the deviation.

Rejected: a directory-tree bytes input (`BTreeMap<String, Vec<u8>>`, #690's first sketch; owner
decision: the artifact is the spec'd bundle); inlining the core package (owner ruling above); inferring the format
from the `.srspkg` suffix (the explicit `--bundle` flag is one way, the suffix would be a second);
a bare-hex checksum (a second format); subset export (RFC-047, Draft).

## Consequences

**Positive:** one reader serves CLI, WASM and any future MCP tool (srs-rust#1153); a pinned
sha256 stays valid across rebuilds when `publishedAt` is fixed; the RFC-043 bundle transformer
finally has its caller; install and export share one boundary loader, the single ADR-042 shim
migration point.

**Negative / trade-offs:** (the id-scan and core-omission trade-offs below are resolved by the srs-rust#1212 Amendment) the id-scan closure would also inline a definition whose UUID appears
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

## Amendment (srs-rust#1212): RFC-003 Revision 10

RFC-003 Revision 10 (srs PR #874, acceptance srs#857) states the rules this codec shipped ahead
of. The codec decisions (one reader, one writer, bytes in/out, store-free reader, hash format,
thin adapters) stand; the rules it applies change as follows.

- **PD1 Reference-site table.** `crates/srs-repository/src/reference_sites.rs` holds one const
  table of every UUID-valued property of the ten definition schemas with its RFC-003 Change C
  strength (`mechanism-64469ada`). The closure follows PINNED (exact version) and LINEAGE (every
  version held) sites only; KEYED sites hold strings and are never rows; LOCATOR and non-reference
  rows produce nothing. A unit test walks the embedded schemas and fails on a missing or stale
  row. Deriving the table from schema annotations is proposed on srs#873 (spec-side guard).
- **PD2 [C6] chain from the registry.** Each data-model revision entry of the migration registry
  declares its Package Bundle form (ADR-032 amendment). The reader walks the steps from the
  bundle's stamp (absent = 0): `Restamp` sets the stamp, `Transform` rewrites (7-8 RFC-043),
  `Unspecified` refuses with `bundle-migration-step-missing` naming the step and the reader floor
  (7 today). The stamp is written after every step; a revision-9 bundle no longer runs through
  the RFC-043 transformer.
- **PD3/PD6 Identity.** The effective package set is every boundary plus the embedded core's
  Fields and Types (`core_package()`; core RelationTypes are KEYED, never reached). An `(id,
  version)` the export carries or lists with two different holders is `bundle-identity-conflict`:
  boundary vs boundary compares canonical content, core vs boundary uses ADR-025's
  namespace/name rule. This includes the source package's own definition.
- **PD4** A PINNED site whose optional version is absent resolves as LINEAGE.
- **PD5 `standalone` mode** (owner ruling O4): carries the source package's own definitions only;
  closure, resolution, failures and `dependencyRefs` are mode-independent; a non-own definition
  is listed and its own references are not followed ([C1] "made by a carried definition";
  finding F4). Install is unchanged and does not check `dependencyRefs`: a standalone bundle
  installed without its dependencies leaves dangling references that make the destination's
  checked catalog fatal (spec-door question D4).
- **`dependencyRefs`** lists every reached definition (own, inlined, core), sorted by `id`,
  `version`, `definitionType`. Core definitions are listed, never carried unless the source
  package lists them (`rfc-decision-a8dcbfe5`); this resolves the Deviation above.
- **Properties.** `dataModelRevision` is the repository's own stamp; `schemaVersion` is `"2.0"`;
  definition arrays sort by `id` then numeric `version`; `description` comes from the target
  `package.json` (PD7); `homepage` is caller-supplied (PD12). An export from a repository below
  the reader floor succeeds with a `bundle-below-reader-floor` note (PD11; refusing it is owner
  question O5).
- **PD8** Every carried definition is checked with the reader's `validate_source_definition`.
- **PD10** The gallery fixture's dangling `lifecycleRef`s now refuse its export; bindings tests
  use the `install-package` fixture.
- **New codes.** Writer: `bundle-reference-unresolved`, `bundle-identity-conflict`,
  `bundle-definition-invalid`. Reader: `bundle-migration-step-missing`.

Consequence: every `.srspkg` written before this amendment differs from the new output and must
be re-exported (none is committed or pinned; srs-web#339 re-pins after release). A repository
with a dangling PINNED/LINEAGE reference can no longer export the affected package.

### Implementation charter (ADR-048)

- [x] **Spec-first** — RFC-003 Rev 10 [C1]-[C6], mechanism records `mechanism-b63bc07f`,
      `mechanism-64469ada`, `mechanism-a2602d0e`, `mechanism-b18c44c4`, `mechanism-012a2148`,
      `rfc-decision-a8dcbfe5`; Invariants 8/15/35/36/37/43 checked by the round-trip test.
- [x] **Layer test** — core only; the CLI gains `--mode`/`--homepage` and three payload fields;
      WASM code unchanged (the input JSON carries `mode`).
- [x] **One way per goal** — one reference-site table (replaces the id scan), one revision-step
      list (the registry), one definition strictness, one canonicalize, one boundary loader.
- [x] **Parity and mirror obligations** — golden `package-export.json` regenerated; no entity
      schema change; srs-usage.md §5g follows in `srs`.
- [x] **Decision mode** — complicated; Door 1; `gate:owner-merge` (released bytes change, merges
      after srs PR #874).
