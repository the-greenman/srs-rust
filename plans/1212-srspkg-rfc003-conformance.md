# Plan: conform the `.srspkg` exporter and reader to RFC-003 Revision 10 (srs-rust#1212)

> Closes #1212. Parent: the-greenman/muDemocracy.org#242. Builds on srs-rust#1210 (#632/#690/#663,
> plan `plans/663-wasm-package-install.md`, ADR-050). Spec: RFC-003 Revision 10, srs PR #874
> (branch `rfc/857-rfc003-whole-package-export`, acceptance issue srs#857; open, owner-merge
> pending). Must land before srs-web#339 pins a published bundle (the bytes change).

## Summary

srs-rust#1210 shipped `srs package export` and the `.srspkg` reader while RFC-003 was Draft. Against
the accepted text (Revision 10) the exporter finds references by scanning every string leaf, writes
`dependencyRefs: []`, lets the first package boundary win on a duplicate id, stamps the binary's
revision instead of the repository's, writes `schemaVersion` from `srsVersion`, sorts definition
arrays by id only and drops `description`; the reader has no [C6] floor (it runs the 7-to-8
transformer on anything and reads the result). This plan replaces the id scan with one typed
reference-site table (PINNED and LINEAGE followed; KEYED, LOCATOR and non-reference sites not),
computes `dependencyRefs` in both modes (own, inlined and core definitions, core listed but never
carried), makes identity conflicts and unresolved followed references fatal, fixes the four
property rules, and gives the reader a step-wise [C6] chain derived from the existing migration
registry. It also adds `mode: "standalone"` (owner ruling O4) across service, CLI and WASM input. The round trip (export, install into a fresh repository, zero validation errors) stays
the acceptance test, now with `dependencyRefs` checked against the six Package invariants.

## Spec gate (Stage 1.5)

No spec change in this repository. Governing text, read from the srs PR #874 branch (not yet on
srs `master`):

- `rfcs/rfc-003.md` Revision 10: Change C (reference-site table, *When export fails*, property
  table, Determinism, Data-model revision), [C1]-[C6], the core-package exception, the [C6]
  re-stamp rule for shape-neutral steps (RFC-046's 8 to 9).
- Mechanism records created by the fold: *Package export* `mechanism-b63bc07f`, *Package export
  reference sites* `mechanism-64469ada`, *Package export identity* `mechanism-a2602d0e`,
  *Package Bundle serialisation* `mechanism-b18c44c4`, *Package bundle revision gate*
  `mechanism-012a2148`; decision `rfc-decision-a8dcbfe5` (core exception).
- Invariants 8, 15, 35, 36, 37, 43 (bundled-mode clause gains the core exception).
- `docs/schema/2.0/package-bundle.json`: description text only (the `mode` and top-level
  descriptions). No property, `required` or `$def` change, so validation is unaffected.

**Landing precondition:** the implementation PR merges only after srs PR #874 merges (owner). The
`package-bundle.json` mirror refresh is description-only and arrives through the automated
`schema-sync.yml` PR (srs-rust#914); this PR does not depend on it and does not hand-copy it.

**Spec findings made while planning (reported, not acted on here):**

- F1. The reference-site table (RFC-003 Change C, `mechanism-64469ada`) omits a Protocol
  `FieldRef.fieldId` (`protocol.json` `/stages/*/contributesTo/*/fieldId`). The table's own
  derivation rule makes it a site (it holds a Field UUID; `fieldId` is LINEAGE everywhere under
  `rfc-decision-c8704763`), and Invariant 37 requires it in `dependencyRefs`. This plan follows it
  as LINEAGE (derivation rule plus the invariant floor). Ruled O3: the row is folded into srs PR
  #874 before it merges.
- F2. Invariant 15 ("every `typeId` referenced by any View") is vacuous against today's
  `view.json`: a View holds no Type UUID (only the KEYED `compatibleTypes`). Nothing to implement;
  the invariant checker test notes it.
- F3. `extendsTypeVersion`, `extendsVocabularyVersion` and `extendsLifecycleVersion` are optional
  in their schemas, so a PINNED inheritance site can arrive without its version. Plan decision PD4.
- F4. Standalone transitivity is implicit: [C1] follows references "made by a carried definition", so in
  `mode: "standalone"` the references of a listed-but-not-carried definition are not followed and
  its own dependencies are not listed. The plan implements that literal reading; deferred as D5.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | /ship session |
| Repository Worker | Phases 1, 2, 3, 4 |
| CLI Worker | Phase 5 (payload field, golden schema, CLI test) |
| Bindings Worker | Phase 5 (bindings tests only; no binding code changes) |
| Verification | Verification Agent, after Phases 3, 4 and before sign-off |

See [agents.md](agents.md).

## Architecture Decisions

Settled by the RFC (not to relitigate): closure by reference strength; `dependencyRefs` in both
modes incl. own and core; core never carried unless the source package lists it; identity conflict
and unresolved followed reference fatal, nothing written; `dataModelRevision` = repository stamp;
`schemaVersion` = `"2.0"`; arrays by `id` then numeric `version`, `dependencyRefs` by `id`,
`version`, `definitionType`; `description` from the manifest; [C6] absent = 0, step-wise, re-stamp
for shape-neutral steps, refuse on a missing step naming it.

Plan decisions (owner may override at the Stage 4 checkpoint):

| # | Decision |
|---|---|
| PD1 | **One reference-site table** in a new module `crates/srs-repository/src/reference_sites.rs`: a `const REFERENCE_SITES: &[ReferenceSite]` keyed by (definition kind, JSON-pointer pattern), each row carrying a strength (`Pinned { version_key }`, `Lineage`, `Locator`, `NotReference`) and a target kind. Every UUID-valued property of the ten definition schemas has exactly one row, including the not-followed ones, so the table is checkable. A unit test walks the embedded schemas (`srs_schema::schema_source`) and fails on any UUID site without a row or any row without a schema site: the in-repo twin of srs#873's spec-side guard. KEYED sites hold strings, not UUIDs, so they are not rows and are never visited. Schema-annotation derivation is not possible today (strengths are prose in `description`s, not machine-readable); see O2. |
| PD2 | **[C6] step list derived from the migration registry.** `MigrationDefinition` gains `revision_step: Option<RevisionStep>`, `RevisionStep { from: u64, to: u64, bundle: BundleForm }`, `enum BundleForm { Unspecified, Restamp, Transform(fn(&mut Value) -> Result<Vec<String>, RepositoryError>) }`. The nine revision entries (`field-type` 0-1 ... `rfc046-actor-provenance` 8-9) set it from the existing `*_REVISION` constants; the six structural entries set `None`. Only RFC-003's two specified steps get a bundle form (7-8 `Transform` wrapping `migrate_package_bundle_value`, 8-9 `Restamp`); 0-7 are `Unspecified`, so a revision-9 reader refuses below 7 exactly as [C6] states. The field is non-defaulted, so the next revision bump cannot compile without declaring its bundle form. This refines ADR-032's scope rule (bundle forms are still not registry *entries*; each revision entry declares its bundle form). See O1. |
| PD3 | **Identity conflict scope.** Fatal (`bundle-identity-conflict`) when any `(id, version)` the export carries **or lists** has two different definitions in the effective package set. RFC text says "would be carried"; listing an ambiguous `(id, version)` in `dependencyRefs` (namespace/name taken from one of two) is the same precedence the Earth rule forbids. This explicitly includes the source package's **own** definition when another boundary holds a different-content copy of the same `(id, version)`: the export refuses (tested). "Different" is decided by one rule per pair: **boundary vs boundary** = different `srsj::canonicalize` content; **core vs boundary copy** (core Fields and Types only, PD6) = ADR-025's existing rule (`merge_core_into_package`): same `id` and `version` with the same `namespace` and `name` is the same core definition, otherwise a conflict. The raw core bundle is never parsed a second time or compared by content (it is stamped revision 2 and read through compat shims, so content comparison would raise false conflicts). Conflicts nothing carries or lists do not block the export. |
| PD4 | **PINNED site without its version** (F3): resolved as LINEAGE (every version held). The schema admits it, so refusing would block a schema-valid package; a bare UUID is LINEAGE-shaped. Tested. |
| PD5 | **`standalone` mode is built (owner ruling O4).** `ExportPackageInput.mode: BundleMode` with `#[serde(default)]` (`bundled` default, `standalone`). The closure, resolution, failure rules and `dependencyRefs` are identical in both modes; the only difference is that `standalone` carries the source package's own definitions and nothing else, so only own definitions are walked (a non-own definition is listed, never carried, and its own references are not followed: [C1] binds references "made by a carried definition"). Core is listed in both modes. Install is unchanged (RFC-003: readers add nothing beyond [C2]/[C6]); it does not check `dependencyRefs`. Installing a standalone bundle into a repository that lacks its dependencies therefore succeeds and leaves dangling references: a Type `fieldId` that resolves to nothing is `SRS038-R13-DANGLING-REFERENCE`, which is **fatal to the destination's checked catalog** (the same failure srs-rust#1208 shows), so ordinary commands then fail until the dependency is installed. The plan asserts exactly this and files D4 (see Deferred items). |
| PD6 | **Effective package set** = every boundary `list_package_boundaries` returns (primary + local `packageRefs`, the set `load_package` merges) plus the embedded core package, read through the existing typed `core_package::core_package()`: its **Fields and Types only** (`fields`, `record_types`: `id`, `version`, `namespace`, `name`, kind), not from the RFC-038 catalog and not by re-parsing the raw bundle. Core RelationTypes are excluded from the reference index: no reference site targets a RelationType (relation types are KEYED, and ADR-025 compares them by key), so a core RelationType can never be reached, listed or conflict. So export does not depend on srs-rust#1208 (catalog lacks core fields). |
| PD7 | **`description`** written verbatim when the target `package.json` has a string `description` (including `""`, which `package-manifest.json` requires); omitted only when absent. Read from the raw `package.json` the exporter already loads for its unreadable-index check (no `PackageBoundary` change). |
| PD8 | **Carried-definition schema check on export** reuses the reader's `validate_source_definition` (one strictness), code `bundle-definition-invalid`. |
| PD9 | **Payload:** `PackageExportSummary` and `PackageExportPayload` gain `dependencyRefCount: usize` and `mode` (`"bundled"` / `"standalone"`, echoing the bundle); `inlined` keeps its meaning (ids carried from other boundaries; always empty in `standalone`). Regenerate `schemas/payload/package-export.json`. |
| PD10 | **Gallery fixture** carries three dangling `lifecycleRef`s (LINEAGE), so its export now refuses. `crates/srs-bindings/tests/package_bundle.rs` switches its two gallery-based tests to the clean `install-package` fixture; one new bindings test pins the refusal on the gallery. The fixture is not repaired here (shared, and the dangling refs are what other tests exercise). |
| PD11 | **Repository stamped below the reader floor** (arch review #6). A repository at revision 0-6 exports a bundle stamped with its own revision ([C6]), which every revision-9 reader, including this one, refuses. No new refusal rule is added (that needs the owner, O5); the export succeeds, the summary carries a `notes` entry `bundle-below-reader-floor: repository dataModelRevision {n}; readers at revision {current} refuse bundles below {floor}; migrate the repository first (srs repo apply-migration)`, and a test pins the behaviour. `notes` is computed from the same `revision_step_from` chain the reader uses (no second floor constant). |
| PD12 | **`homepage`** (arch review #8): caller-supplied like `publisher` (RFC-003 property table): `ExportPackageInput.homepage`, CLI `--homepage`, omitted when absent. |

| ADR | Decision | Status |
|---|---|---|
| [ADR-050](../docs/adr/050-package-bundle-codec.md) | Amended **in place**: new `## Amendment (srs-rust#1212): RFC-003 Revision 10` section, plus a one-line `> Amended by ...` pointer at the head of §2, §5 and the *Deviation* paragraph; the *Rejected* paragraph is edited to drop the typed-walker and `standalone` rejections. Status stays accepted; header gains `- **Amended by:** srs-rust#1212 (RFC-003 Rev 10)`. Rationale: the codec decisions (one reader, one writer, bytes in/out, hash format, thin adapters, store-free reader) stand; what changes is the RFC rules the codec applies, which is how ADR-025 (#685) and ADR-045 (RFC-043) were amended. A superseding ADR would split one module's rules across two records (ADR-038/040 superseded a mechanism, which is not the case here). | amended in Phase 6 |
| [ADR-032](../docs/adr/032-migration-registry-fn-pointer-pattern.md) | Short `### Amendment (srs-rust#1212)` under the scope rule: revision entries declare `revision_step` with a `BundleForm`; pre-load transformers are still not entries | amended in Phase 6 |
| ADR-004 | Embedded schemas: bundle validation and the PD1 guard read them via `srs_schema` | applies |
| ADR-009 | Boundaries via `list_package_boundaries` / `PackageSelector` | applies |
| ADR-010 / ADR-011 | Typed service in/out; payload change regenerates golden schema | applies |
| ADR-013 / ADR-015 | WASM methods unchanged (same services, same structs) | applies |
| ADR-017 / ADR-043 | Determinism via the explicit `srsj::canonicalize`; String `Ord` is byte order = code-point order ([C5]) | applies |
| ADR-025 | Core merged everywhere; basis of the core exception and PD6 | applies |
| ADR-030 | Install core and import records unchanged | applies |
| ADR-039 | Missing listed file is a hard error naming the path (kept) | applies |
| ADR-042 | Boundary definitions still read through the one shim `load_boundary_definitions` | applies |
| ADR-048 | Charter below | applies |
| ADR-001/002/003/005-008/012/014/016/018-024/026-029/031/033-038/040/041/044-047/049 | Read; no bearing beyond the rows above (no new storage, CLI command, MCP tool, repair seam or write path) | n/a |

### Implementation charter (ADR-048)

- **Spec-first:** RFC-003 Rev 10 [C1]-[C6] and mechanism records above; Invariants 8/15/35/36/37/43; F1-F3 reported to the owner, not resolved impl-side beyond PD4 and the F1 derivation-rule reading.
- **Layer test:** core only (`srs-repository`: `reference_sites.rs`, `package_bundle.rs`, `migration_registry_service.rs`). CLI gains `--mode` and two payload fields; WASM code unchanged (the `mode` input field arrives through the existing `ExportPackageInput` JSON).
- **One way per goal:** one reference-site table (replaces the id scan); one revision-step list (the registry, PD2) instead of a second bundle-step list; one definition strictness (`validate_source_definition`); one canonicalize; one boundary loader.
- **Parity and mirror obligations:** golden `package-export.json` regenerated. Entity schema mirror: description-only, via the automated sync PR. srs-usage.md §5g lives in `srs`: follow-up issue D2. srs-web#339 re-pins after release.
- **Decision mode / door:** complicated; Door 1 (executes accepted RFC-003 Rev 10 and `rfc-decision-a8dcbfe5`) but `gate:owner-merge` because it changes released artifact bytes and amends two ADRs, and the precondition (srs#874 merged) is owner-held.

### Positioning (alignment register, `srs/docs/research/alignment-opportunities.md`)

- **#21 LinkML**: LinkML carries slot semantics (range, inlined) as schema annotations and derives generators from them; the same move would let the reference-site table be derived from the definition schemas (O2). Borrow the discipline, not the toolchain.
- **#6 atproto lexicon publishing**: `dependencyRefs` (id + version + definitionType, sorted) is the bundle's resolved-reference manifest, the same role lexicon `ref`s play; it keeps a future registry able to check completeness without parsing content.
- **#14 Nanopublications** / **#18 Frictionless**: content hashing and `$schema` self-versioning unchanged from ADR-050; the hash of every existing export changes once (What breaks).

---

## Contracts

### CLI output contract (ADR-011)

- **Changed payload** `PackageExportPayload` (`crates/srs-cli/src/payload.rs`) gains `dependency_ref_count: usize` (`dependencyRefCount`), `mode: String` (`"bundled"`/`"standalone"`) and `notes: Vec<String>` (PD11), mapped in `PackageExportPayload::new`.
- **Changed command** `srs package export` gains `--mode <bundled|standalone>` (optional; absent = bundled) and `--homepage <url>` (PD12), mapped onto `ExportPackageInput.mode`; the handler stays one service call. The `inlined` doc comment becomes "Ids carried from other package boundaries by the closure (sorted); core definitions are listed, never carried". Run `cargo run --bin generate-schemas`; commit `crates/srs-cli/schemas/payload/package-export.json`. `cargo test -p srs-cli --test payload_contracts` must pass.
- `PackageInstallPayload` unchanged (reader notes still flow through `notes`).
- New coded refusals reach clients as the existing `InvalidPackageBundle { code, message }` error envelope: writer `bundle-reference-unresolved`, `bundle-identity-conflict`, `bundle-definition-invalid`; reader `bundle-migration-step-missing`.

### Entity schema sync (check-schema-sync.sh)

This PR edits no entity schema and no mirror file, so `bash scripts/check-schema-sync.sh` passes on its own terms (it compares the mirror with its SHA256SUMS and the release, which this PR does not touch). srs#874's description-only change to `package-bundle.json` reaches `crates/srs-schema/schemas/2.0/` separately, through the automated `schema-sync.yml` PR after #874 merges; no test here depends on that text.

---

## Scope

- `crates/srs-repository/src/reference_sites.rs` (new): table, path matcher, `followed_references`.
- `crates/srs-repository/src/package_bundle.rs`: exporter rewrite (closure, `dependencyRefs`, failures, properties, sort, `standalone` mode); reader [C6] chain.
- CLI `srs package export --mode`.
- `crates/srs-repository/src/migration_registry_service.rs`: `revision_step` on every entry; `revision_step_from`.
- `crates/srs-repository/src/lib.rs`: `mod reference_sites;`.
- Tests: unit tests in the files above; `crates/srs-repository/tests/package_bundle_reader.rs`, `tests/package_bundle_roundtrip.rs`; `crates/srs-bindings/tests/package_bundle.rs`; `crates/srs-cli/tests/package_install_cli.rs`.
- CLI payload field + golden schema.
- Docs: ADR-050 and ADR-032 amendments; `docs/dogfooding.md` S49 + coverage row; `CLAUDE.md` crate-authority row for `srs-repository` (mention the reference-site table).

**Out of scope:**

- Subset export (RFC-047, Draft).
- An install-time check of `dependencyRefs` against the destination (D4).
- Writing a bundle `readme` (RFC-045; reader still refuses it): srs-rust#1164.
- Renaming the embedded `core-bundle.srsj`: srs-rust#1213 / srs#872.
- Catalog resolution of core fields: srs-rust#1208 (not needed, PD6).
- Bundle-form transformers below revision 7 (RFC specifies none).
- An install-time invariant check on bundles (RFC: readers add nothing beyond [C2], [C6]).
- srs-usage.md §5g edit (lives in `srs`): D2.

---

## Types and signatures (names final; Lead Integrator owns polish)

`crates/srs-repository/src/reference_sites.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Strength {
    /// {id, version} pair; `version_key` is the sibling key holding the version.
    Pinned { version_key: &'static str },
    Lineage,
    Locator,
    /// UUID-valued but not a definition reference (provenance, container ids, nested own ids).
    NotReference,
}

pub(crate) struct ReferenceSite {
    pub kind: DefinitionKind,            // the referring definition's kind
    pub path: &'static str,              // pointer pattern: "*" = every array item, "{*}" = every object value
    pub target: Option<DefinitionKind>,  // None for Locator/NotReference
    pub strength: Strength,
}

pub(crate) const REFERENCE_SITES: &[ReferenceSite] = &[ /* rows below */ ];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FollowedReference {
    pub path: String,                    // concrete pointer, for diagnostics ("/fields/2/fieldId")
    pub target: DefinitionKind,
    pub id: String,
    pub version: Option<u32>,            // Some for PINNED (with version present), None for LINEAGE
}

/// Every PINNED and LINEAGE reference `value` (a definition of `kind`) makes. Empty strings skipped.
pub(crate) fn followed_references(kind: DefinitionKind, value: &Value) -> Vec<FollowedReference>;
```

Table rows (paths verified against the srs#874 branch schemas with a schema walk; Lead Integrator
re-runs the guard test, which is the authority):

| kind | path | target | strength |
|---|---|---|---|
| Field | `/fieldType/rangeType/typeId` | Type | Pinned `typeVersion` |
| Field | `/fieldType/vocabularyRef` | Vocabulary | Lineage |
| Field, Type | `/lineage/sourceDefinitionId`, `/lineage/forkedFromDefinitionId` | - | NotReference (the other kinds' `lineage` is free-form, with no UUID site) |
| Type | `/extendsTypeId` | Type | Pinned `extendsTypeVersion` |
| Type | `/fields/*/fieldId`, `/fieldAssignmentOverrides/*/fieldId`, `/fieldOrder/*`, `/validationRules/*/fieldIds/*`, `/validationRules/*/predicateFieldId`, `/validationRules/*/targetFieldId`, `/identityFieldId` | Field | Lineage |
| Type | `/lifecycleRef` | Lifecycle | Lineage |
| Type | `/lifecycle/states/*/id`, `/lifecycle/transitions/*/id` | - | NotReference |
| View | `/fieldViews/*/fieldId`, `/fieldViews/*/compositeRenderer/roles/{*}` | Field | Lineage |
| Composition | `/rootTypeRefs/*/typeId` | Type | Pinned `typeVersion` |
| Composition | `/compositeRenderers/*/fieldId`, `/compositeRenderers/*/roles/{*}`, `/sections/*/compositeRenderers/*/fieldId`, `/sections/*/compositeRenderers/*/roles/{*}`, `/sections/*/titleFieldId`, `/sections/*/ordering/fieldId` | Field | Lineage |
| Composition | `/sections/*/renderViewId`, `/sections/*/typeDispatch/{*}` | View | Lineage |
| Composition | `/sections/*/source/query/typeId` | Type | Lineage |
| Composition | `/themeRef/themeId`, `/themeVariants/*/themeRef/themeId` | - | Locator |
| Composition | `/sections/*/source/containerId`, `/sections/*/source/containerIds/*`, `/sections/*/source/query/containerId` | - | NotReference |
| Vocabulary | `/extendsVocabularyId` | Vocabulary | Pinned `extendsVocabularyVersion` |
| Vocabulary | `/terms/*/id` | - | NotReference |
| Lifecycle | `/extendsLifecycleId` | Lifecycle | Pinned `extendsLifecycleVersion` |
| Lifecycle | `/states/*/id`, `/transitions/*/id` | - | NotReference |
| Theme | `/cssClassFields/*` | Field | Lineage |
| Theme | `/elementTemplates/recordWrapperOverrides/*/typeId` | Type | Lineage |
| Blueprint | `/rootTypes/*/typeId`, `/requiredTypes/*/typeId`, `/structure/*/sourceType/typeId`, `/structure/*/targetType/typeId` | Type | Pinned `typeVersion` |
| Protocol | `/targetType`, `/stages/*/outputType`, `/stages/*/contributesTo/*/typeId` | Type | Lineage |
| Protocol | `/stages/*/contributesTo/*/fieldId` | Field | Lineage (F1) |

Every definition's own `/id` is excluded by rule (not a row). RelationType has no UUID site.

`crates/srs-repository/src/migration_registry_service.rs`:

```rust
pub(crate) struct RevisionStep { pub from: u64, pub to: u64, pub bundle: BundleForm }
pub(crate) enum BundleForm {
    /// The spec names no bundle-form transformer for this step: a reader refuses ([C6]).
    Unspecified,
    /// The step changes no shape a Package Bundle carries: set the stamp to `to` ([C6]).
    Restamp,
    /// Rewrite the bundle; returns non-fatal notes. Err = transformer refusal.
    Transform(fn(&mut serde_json::Value) -> Result<Vec<String>, RepositoryError>),
}
// MigrationDefinition gains: revision_step: Option<RevisionStep>
/// The registry entry whose revision step starts at `from` (its id + step).
pub(crate) fn revision_step_from(from: u64) -> Option<(&'static str, &'static RevisionStep)>;
```

`core_package.rs` is unchanged: export reads `core_package()` as-is (PD6).

`crates/srs-repository/src/package_bundle.rs` (public signatures unchanged):

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BundleMode { #[default] Bundled, Standalone }   // + FromStr for the CLI flag
pub struct ExportPackageInput {
    /* existing selector, published_at, publisher */
    #[serde(default)] pub mode: BundleMode,
    /// Optional `homepage` (RFC-003 property table: caller-supplied); omitted when None.
    #[serde(default)] pub homepage: Option<String>,
}
pub struct PackageExportSummary {
    /* existing */
    pub dependency_ref_count: usize,
    pub mode: BundleMode,
    /// Non-fatal export notes; today only `bundle-below-reader-floor` (PD11).
    pub notes: Vec<String>,
}
fn bring_forward(v: &mut Value, rev: u64) -> Result<Vec<String>, RepositoryError>; // private, [C6]
fn definition_type(kind: DefinitionKind) -> &'static str; // schema enum: "relation-type", ...
```

`definition_type` maps to the `package-bundle.json` `Reference.definitionType` enum (`relation-type`
kebab, unlike `kind_label`'s `relationType`); one match, private.

---

## Phases

### Phase 1: Registry revision steps

**Goal:** the migration registry states each revision step and its bundle form; nothing reads it yet.

**Agent:** Repository Worker

#### Tasks

- [x] Add `RevisionStep`, `BundleForm`, `revision_step` field and `revision_step_from` (signatures above) to `migration_registry_service.rs`.
- [x] Set `revision_step` on `field-type` (0-1), `rfc039-carrier` (1-2), `metamodel-v1-1-0` (2-3), `tier1-removal` (3-4), `substrate-properties-to-meta` (4-5), `composition-cutover` (5-6), `discovery-query-cutover` (6-7) with `BundleForm::Unspecified`; `rfc043-container-entries` (7-8) with `Transform(|v| migrate_package_bundle_value(v).map(|r| r.diagnostics))`; `rfc046-actor-provenance` (8-9) with `Restamp`. Use the existing `*_REVISION` constants (`FIELD_TYPE_REVISION`, `CARRIER_REVISION`, `METAMODEL_V1_1_0_REVISION`, ..., `RFC046_ACTOR_PROVENANCE_REVISION`), `to - 1` for `from`. `None` on `graduated-at-cleanup`, `revisions-sidecar-cleanup`, `migrate-identity`, `repo-upgrade`, `rfc038-storage`.
- [x] Doc comment on `BundleForm::Unspecified` citing RFC-003 [C6]; doc comment on the field: a new revision entry must state its bundle form (the reason the field has no default).

#### Acceptance Criteria

- [x] Every data-model revision from 0 to the current one is reachable by exactly one registered step, so a caller can walk any stamp forward without gaps.
- [x] A reader consulting the registry finds a bundle form only for the 7-to-8 and 8-to-9 steps; every earlier step reports none.
- [x] Listing and applying migrations behaves exactly as before.

#### Testing

- `migration_registry_service::tests::revision_steps_form_a_contiguous_chain_to_current` — from 0, each `to == from + 1`, last `to == CURRENT_DATA_MODEL_REVISION`, no duplicate `from`.
- `migration_registry_service::tests::bundle_forms_are_the_ones_rfc003_c6_specifies` — 7-8 Transform, 8-9 Restamp, 0-7 Unspecified.
- Existing registry tests pass unchanged.

#### Milestone gate

```bash
cargo test -p srs-repository migration_registry
cargo clippy -p srs-repository --all-targets -- -D warnings
```
Tick boxes, commit.

---

### Phase 2: Reference-site table

**Goal:** `followed_references` returns exactly the PINNED and LINEAGE references of any definition, and a guard proves the table covers the schemas.

**Agent:** Repository Worker

#### Tasks

- [x] Create `reference_sites.rs` with `Strength`, `ReferenceSite`, `REFERENCE_SITES` (rows above), `FollowedReference`, `followed_references`; register `mod reference_sites;` in `lib.rs`.
- [x] Path matcher: split pattern on `/`; a literal segment indexes an object key; `*` iterates array items; `{*}` iterates object values; collect `(concrete_path, &Value)` leaves. Non-string or empty-string leaves are skipped.
- [x] PINNED: read the sibling `version_key` in the leaf's parent object as `u32`; absent → `version: None` (PD4, LINEAGE fallback). LINEAGE: `version: None`. Locator / NotReference rows never produce output.
- [x] Module doc: the table is RFC-003 Change C's reference-site table (`mechanism-64469ada`), derivation rule quoted, F1 noted, srs#873 named as the spec-side guard.

#### Acceptance Criteria

- [x] Guard passes against the embedded schemas; removing any row or adding a bogus row fails it.
- [x] KEYED strings (relation-type keys, `typeDispatch` keys) are never returned.

#### Testing

- `reference_sites::tests::table_matches_every_uuid_site_in_the_definition_schemas` — walks `srs_schema::schema_source` for the ten definition schemas (`$ref` into `#/$defs`, `properties`, `items`, `additionalProperties`, `patternProperties`, `oneOf`/`anyOf`/`allOf`, `if`/`then`/`else`), collects paths of `format: "uuid"` leaves except the root `/id`, and asserts set equality with the table's (kind, path) rows. Failure message lists missing and stale rows.
- `reference_sites::tests::pinned_site_returns_its_version` — Blueprint `rootTypes` and Field `rangeType`.
- `reference_sites::tests::lineage_site_returns_no_version` — Type `fields/*/fieldId`, Protocol `contributesTo/*/fieldId` (F1).
- `reference_sites::tests::wildcards_match_arrays_and_object_values` — Composition `sections/*/typeDispatch/{*}`, View `compositeRenderer/roles/{*}`.
- `reference_sites::tests::locator_and_non_reference_sites_are_not_followed` — `themeRef.themeId`, `lineage.sourceDefinitionId`, `source.containerId`, lifecycle state ids.
- `reference_sites::tests::keyed_strings_are_not_followed` — Blueprint `structure/*/relationType`, `typeDispatch` keys.
- `reference_sites::tests::empty_value_is_not_a_reference` — Protocol `targetType: ""`.
- `reference_sites::tests::pinned_without_version_falls_back_to_lineage` — `extendsTypeId` without `extendsTypeVersion`.

#### Milestone gate

```bash
cargo test -p srs-repository reference_sites
cargo clippy -p srs-repository --all-targets -- -D warnings
```
Tick boxes, commit.

---

### Phase 3: Exporter

**Goal:** `export_package_bundle` writes a [C1]-[C6]-conformant bundle or refuses with a coded error and writes nothing.

**Agent:** Repository Worker

#### Tasks

- [x] Replace `string_leaves` and the id-scan loop. Build the effective index `BTreeMap<id, BTreeMap<version, Holding>>` from every boundary's `load_boundary_definitions` (target included) plus `core_package()`'s typed fields and record types (core RelationTypes excluded: no reference site targets a RelationType, PD6); `Holding { kind, namespace, name, value: Option<Value> /* None for core */, is_core, conflicting: bool }`. On a duplicate `(id, version)`: boundary vs boundary compares `srsj::canonicalize(..)`; core vs boundary compares `namespace`+`name` (ADR-025 rule); different → `conflicting = true` (PD3). `is_core` = the id is a core id. A core `Reference` takes `namespace`/`name`/`version` from the typed core struct.
- [x] Closure: carried = target's own definitions (all, [C1]); worklist over carried definitions; for each `followed_references` result resolve in the index with matching `target` kind: PINNED with version → that version; LINEAGE (or PD4) → every version held. Nothing found → `bundle-reference-unresolved` (message: referring definition id, kind, concrete path, target kind, target id and version). Each reached `(id, version)`: if `conflicting` → `bundle-identity-conflict` (message names id, version and both locations); insert a `Reference {id, namespace, name, version, definitionType}` into a `BTreeSet` keyed `(id, version, definitionType)`; if not core and not yet carried → carry and enqueue. The target's own carried definitions are also checked for `conflicting` (PD3).
- [x] Mode (PD5): `input.mode`; in `standalone` a reached non-own definition is listed and never carried or enqueued; write `"mode"` from the input.
- [x] Validate each carried definition with `validate_source_definition` → `bundle-definition-invalid` naming kind and id (PD8).
- [x] Properties: `schemaVersion` = `"2.0"` constant; `dataModelRevision` = `field_type_migration_service::data_model_revision(store)?`; `description` from the raw target `package.json` (PD7; keep the value the existing unreadable-index check loads); arrays sorted by `(id, version)` numerically; `dependencyRefs` from the BTreeSet (already in `(id, version, definitionType)` order).
- [x] Summary: `data_model_revision` = repository stamp; `dependency_ref_count`; `mode`; `inlined` = ids carried from boundaries other than the target; `notes` (PD11).
- [x] `homepage` written when given (PD12).
- [x] Update the module and function doc comments (drop "core omitted, OD5", "id-scan", the `ponytail:` comment).
- [x] Adapt the existing unit tests in `package_bundle.rs` exactly as the Testing section of this phase says.

#### Acceptance Criteria

- [x] Every failure in Change C's *When export fails* returns `InvalidPackageBundle` (or the existing `PackageNotFound`/load error) and no text.
- [x] Two exports of the same input are byte-identical; keys sorted at every depth.
- [x] Core definitions reached appear in `dependencyRefs` and in no definition array.

#### Testing

Unit tests in `crates/srs-repository/src/package_bundle.rs` (new unless marked):

- Per strength: `export_follows_pinned_reference_to_exact_version` (Field `rangeType` names Type v2 of a v1/v2 lineage in another boundary: only v2 carried and listed); `export_follows_lineage_reference_to_every_version` (Type `fieldId` to a Field held at v1 and v2 in another boundary: both carried, two `dependencyRefs` entries); `export_does_not_follow_keyed_reference` (Blueprint `RelationSpec.relationType` key of another boundary's relation type: not carried, not listed); `export_does_not_follow_locator_reference` (Composition `themeRef` to another boundary's Theme: not carried, not listed); `export_does_not_follow_lineage_provenance` (Field `lineage.sourceDefinitionId` naming another boundary's Field: not carried; this is the id-scan false positive); `export_follows_references_transitively` (View -> Type -> Field across two boundaries).
- `dependencyRefs`: `export_lists_own_referenced_definitions` (own Type's own Field listed with `definitionType: "field"`); `export_unreferenced_own_definition_is_carried_not_listed`; `export_dependency_refs_sorted_by_id_version_definition_type`.
Labels: **(new)** unless marked **(modified: replaces X)** or listed under **Kept**.

- Core: `export_lists_core_definitions_without_carrying_them` **(modified: replaces `export_omits_core_package_definitions`)**, keeps its install + zero-error check; `export_carries_a_core_definition_the_source_package_lists`; `export_boundary_copy_of_core_field_is_not_a_conflict` (a boundary holds a current-shape copy of a core field, same id/version/namespace/name, reached by a View: export succeeds, one `dependencyRefs` entry, not carried); `export_core_id_with_different_name_is_identity_conflict` (ADR-025 conflict case).
- Failures: `export_refuses_unresolved_lineage_reference`; `export_refuses_unresolved_pinned_version`; `export_refuses_identity_conflict` **(modified: replaces `export_closure_first_boundary_wins_on_duplicate_id`)**; `export_refuses_own_definition_conflicting_with_another_boundary` (PD3: target's own Field and a different-content copy in another boundary); `export_identity_duplicate_with_equal_content_is_one_definition`; `export_refuses_carried_definition_failing_its_schema`. Kept: `export_missing_listed_definition_file_is_hard_error`, `export_refuses_legacy_package_dependencies` ([R11]), `export_refuses_unreadable_target_index`, `export_rejects_invalid_published_at`, `export_unknown_selector_is_package_not_found`.
- Standalone (PD5): `export_standalone_carries_only_own_definitions` (same two-boundary setup as `export_follows_references_transitively`: own definitions only in the arrays, `mode: "standalone"`, `inlined` empty); `export_standalone_lists_every_reached_non_own_and_core_definition` (a cross-boundary Field and a core field both in `dependencyRefs`; the non-own definition's own references are not followed); `export_standalone_refuses_unresolved_reference` (resolution is mode-independent); `export_standalone_is_byte_deterministic`; `export_mode_defaults_to_bundled` (input `{}`); `export_input_json_maps_mode` (`{"mode":"standalone","homepage":"https://x"}`).
- PD11: `export_from_repository_below_reader_floor_writes_bundle_its_reader_refuses` — repository stamped 6: export succeeds, bundle stamped 6, `summary.notes` has `bundle-below-reader-floor`, and `read_package_bundle` on the text refuses with `bundle-migration-step-missing`. A repository at 9 has empty `notes`.
- PD12: `export_writes_homepage_when_given`.
- Properties: `export_stamps_repository_data_model_revision` (replaces `export_stamps_current_data_model_revision`; repository manifest stamped 8 via `stamp_data_model_revision` → bundle 8); `export_schema_version_is_constant_2_0`; `export_definitions_sorted_by_id_then_version` (extends `export_definitions_sorted_by_id_and_empty_kinds_omitted`, incl. versions 2 and 10 to prove numeric order); `export_writes_manifest_description` and `export_omits_description_when_manifest_has_none`.
- Kept unchanged: identity, verbatim definitions, determinism, sha256, sorted keys, schema-valid output, primary-package selector, input JSON mapping. `export_inlines_cross_boundary_references` updated to assert `dependencyRefs` too.

#### Milestone gate

```bash
cargo test -p srs-repository package_bundle
cargo clippy -p srs-repository --all-targets -- -D warnings
```
Verification Agent runs after this phase. Tick boxes, commit.

---

### Phase 4: Reader [C6] chain

**Goal:** the reader brings a bundle forward step by step through the registry's bundle forms, or refuses naming the step.

**Agent:** Repository Worker

#### Tasks

- [x] In `read_package_bundle`, replace the unconditional `migrate_package_bundle_value` call with `bring_forward(&mut v, rev)`: for `r` in `rev..CURRENT_DATA_MODEL_REVISION`, `revision_step_from(r)`; `None` or `Unspecified` → `bundle-migration-step-missing` ("bundle declares dataModelRevision {rev} (absent = 0); data-model step {r} -> {r+1} ({migration id}) has no bundle-form transformer; this srs reads bundles from revision {floor}; re-export it with a current srs", `floor` derived by walking down from current while steps have a form); `Restamp` → write `dataModelRevision = to`; `Transform(f)` → `f(&mut v)` collecting notes, `Err` → `bundle-migration-refused` (inner code kept), then write `dataModelRevision = to`. The stamp is written after **every** step, so the bundle always says which revision it has reached. Order: parse → readme refusal → too-new refusal → chain → schema → definitions (unchanged otherwise).
- [x] Update `ReadPackageBundle`/function docs and the `install_package_bundle_bytes` doc.
- [x] Leave `migrate_package_bundle_value` and its tests unchanged (still the 7-8 transformer; its stamp write is harmless under the chain).

#### Acceptance Criteria

- [x] Stamped 9 → read as is; 8 → re-stamped; 7 → transformer then re-stamp; below 7 or absent → refused naming the first missing step; above 9 → refused (unchanged).
- [x] A refusal installs nothing (boundary count unchanged).

#### Testing

`crates/srs-repository/tests/package_bundle_reader.rs`:

- `read_bundle_absent_stamp_is_revision_0_and_refused` — no `dataModelRevision`; code `bundle-migration-step-missing`; message names `0 -> 1` and `field-type`.
- `read_bundle_below_7_names_the_missing_step` — stamped 6; names `6 -> 7` and `discovery-query-cutover`; replaces `read_bundle_schema_error_on_old_bundle_names_revision`.
- `read_bundle_rev8_is_restamped_without_changing_definitions` — definitions value-equal to input; no notes.
- `read_bundle_migrates_pre_rev8_member_order` (existing) — also assert the read bundle ends at revision 9 through both steps.
- `read_bundle_member_order_conflict_refuses` (existing) — transformer refusal, nothing installed.
- `read_bundle_refuses_newer_revision`, `read_bundle_current_revision_passes_through_unchanged` (existing).
- `read_bundle_rev9_with_member_order_is_not_migrated` **(new)** — a revision-9 bundle carrying `ordering.memberOrder` is no longer silently stripped by an always-run transformer: it is refused with the single code the reader produces for it (pinned to the code observed at implementation; recorded under Deviations if it is not `bundle-schema-invalid`), the message naming `memberOrder`.
- `read_bundle_restamp_writes_each_step_target` **(new)** — lives in the `#[cfg(test)]` module of `crates/srs-repository/src/package_bundle.rs`, not in this integration test file, because it asserts through the private `bring_forward`: a rev-7 bundle's stamp is 8 after the transform step and 9 after the re-stamp.
- `read_bundle_refusal_below_floor_installs_nothing` — `install_package_bundle_bytes` on a rev-6 bundle leaves `list_package_boundaries` unchanged.

#### Milestone gate

```bash
cargo test -p srs-repository --test package_bundle_reader
cargo clippy -p srs-repository --all-targets -- -D warnings
```
Verification Agent runs after this phase. Tick boxes, commit.

---

### Phase 5: Round trip, adapters, payload

**Goal:** the round trip still installs cleanly and its `dependencyRefs` satisfy the Package invariants; CLI and bindings tests reflect the new rules.

**Agent:** Repository Worker (round trip), CLI Worker, Bindings Worker

#### Tasks

- [ ] `tests/package_bundle_roundtrip.rs`: add a test-only `assert_package_invariants(bundle: &Value)` written from the invariant texts, independent of `REFERENCE_SITES`: I-8 (every `field` entry in `dependencyRefs` is in `fields[]` or a core id), I-15 (vacuous: no View Type site; documented), I-35 (`renderViewId` in `views[]` or a core `dependencyRefs` entry, and listed), I-36 (Blueprint type refs listed as `type` and carried), I-37 (Protocol target/output types and `contributesTo` fieldIds listed and carried), I-43 (base-Type closure listed and carried).
- [ ] CLI: `--mode` flag on `package export` (clap `value_parser = ["bundled", "standalone"]`, parsed with `BundleMode::from_str`) and `--homepage`; `PackageExportPayload.dependency_ref_count`, `.mode` and `.notes` + mapping; `cargo run --bin generate-schemas`; update `package_install_cli.rs` expectations that read `dataModelRevision`.
- [ ] Bindings: switch `gallery_bundle()` users (`tree_session_install_advances_write_epoch`, `tree_session_export_does_not_advance_write_epoch`) to the `install-package` fixture (PD10).

#### Acceptance Criteria

- [ ] Round trip installs with zero validation errors and the invariants hold on the exported bundle.
- [ ] Golden schema regenerated and payload contract test green.

#### Testing

- `bundle_roundtrip_dependency_refs_satisfy_package_invariants` — install-fixture export.
- `bundle_roundtrip_cross_boundary_closure_installs_clean` — target boundary whose View uses a Field of a second boundary and a core field: export lists both, carries only the non-core one, installs into a fresh repository, `validate_repository` 0 errors, invariants hold. (Uses a View, not a Type, for the core field because of srs-rust#1208.)
- Existing `bundle_roundtrip_*` (all ten kinds identical, boundary metadata, imports clean, re-export byte-identical, tree session matches disk) pass unchanged.
- `bundle_roundtrip_standalone_into_repo_with_dependencies_has_zero_errors` — dependency boundary exported (bundled) and installed into the fresh repository first, then the standalone bundle; `validate_repository` 0 errors; invariants hold for the standalone bundle (listed, not carried).
- `bundle_roundtrip_standalone_into_repo_missing_dependencies_leaves_dangling_refs` — asserts the unchanged install contract: install succeeds (`installed` = own count, no refusal, no notes), then `validate_repository` reports `SRS038-R13-DANGLING-REFERENCE` naming the missing Field id, and the checked catalog (`store.catalog()`) fails. The test comment cites D4.
- `srs-bindings/tests/package_bundle.rs::tree_session_export_accepts_mode_standalone` — `{"selector":...,"mode":"standalone"}` input JSON round-trips through the service the WASM method calls; summary `mode` is `standalone`.
- `srs-cli/tests/package_install_cli.rs::package_export_cli_mode_standalone` — `--mode standalone` writes `"mode": "standalone"`, payload echoes `mode`; `--mode bogus` is a clap error.
- `srs-bindings/tests/package_bundle.rs::gallery_export_refuses_dangling_lifecycle_ref` — code `bundle-reference-unresolved`.
- `srs-cli/tests/package_install_cli.rs::package_install_cli_bundle_below_floor_is_error_envelope` — rev-6 bundle → `ok: false`, diagnostic contains `bundle-migration-step-missing`.
- `srs-cli/tests/package_install_cli.rs::package_export_cli_writes_srspkg_and_reports_sha256` (existing) also asserts `dependencyRefCount`.
- `srs-cli/tests/package_install_cli.rs::package_export_cli_writes_homepage` **(new)** — `--homepage https://example.org/pkg` writes `"homepage"` into the bundle; absent flag omits it.
- `srs-cli/tests/package_install_cli.rs::package_export_cli_reports_below_floor_notes` **(new)** — a repository stamped 6 (via the existing `stamp_data_model_revision` helper) exports with payload `notes` non-empty and containing `bundle-below-reader-floor`; a current repository's payload `notes` is empty.
- `cargo test -p srs-cli --test payload_contracts`.

#### Milestone gate

```bash
cargo test -p srs-repository --test package_bundle_roundtrip
cargo test -p srs-cli --test package_install_cli --test payload_contracts
cargo test -p srs-bindings --test package_bundle
cargo clippy --workspace --all-targets -- -D warnings
```
Tick boxes, commit.

---

### Phase 6: Docs and dogfood

**Goal:** ADRs, S49 and the crate table say what the code does.

**Agent:** Lead Integrator

#### Tasks

- [ ] ADR-050: header `Amended by` line; one-line pointer notes on §2 (no-floor rule replaced by [C6]), §5 (closure) and *Deviation* (resolved by RFC-003 Rev 10 / `rfc-decision-a8dcbfe5`); edit the *Rejected* paragraph to drop the typed-walker and `standalone` rejections (both now adopted); `## Amendment (srs-rust#1212): RFC-003 Revision 10` section stating PD1-PD10 decisions (incl. `standalone` and its install consequence), the four new codes, and its own ADR-048 charter.
- [ ] ADR-032: append `### Amendment (srs-rust#1212)` under the scope rule (ruled O1), text: "Each revision-step entry declares `revision_step: Some(RevisionStep { from, to, bundle })`; the field has no default, so a new revision cannot be registered without stating its Package Bundle form (`Unspecified` = readers refuse across this step, `Restamp`, or `Transform(fn)`). The scope rule stands for entries: a pre-load bundle transformer is still never a registry entry of its own; it is attached to the step it implements. `revision_step` is orthogonal to `status_fn`/`apply_fn`: it describes the step, it never runs the repository migration and is never consulted by `list_migrations`/`apply_migration`. The `.srs`/`.srsj` pre-load transformers (`migrate_rfc014`, `migrate_srsj_str`) are unchanged."
- [ ] `docs/dogfooding.md` S49: capabilities line (typed closure, `dependencyRefs`, core listed, [C6]); expected outputs (`dependencyRefCount`, `dataModelRevision` = repository stamp); a standalone step: author exports `--mode standalone` (expect `mode: "standalone"`, only the essay's own definitions carried, `dependencyRefCount` not greater than the bundled export's: standalone does not follow references of definitions it does not carry, F4); install it into a fresh writer that already has the bundled install (0 errors), and into a third fresh repository with nothing installed (installs; `repo validate` reports the dangling references, D4); negative cases add a rev-6 bundle (`bundle-migration-step-missing` naming `6 -> 7`) and a rev-8 bundle that installs (re-stamp); run it with the branch binary and record actual outputs. Coverage-matrix `package` row: mention RFC-003 Rev 10.
- [ ] `CLAUDE.md` crate-authority row for `srs-repository`: add "the package-export reference-site table (`reference_sites.rs`, RFC-003)".
- [ ] File D2, D4 and D5 below, linked (`gh-project link`; D5 under muDemocracy.org#242).

#### Acceptance Criteria

- [ ] S49 runs end to end with the branch binary and its recorded outputs match.
- [ ] No doc still claims "core omitted, not listed", "id scan" or "no revision floor".

#### Testing

```bash
cargo build --bin srs
# run S49 from docs/dogfooding.md with SRS=$PWD/target/debug/srs and ESSAY set
grep -rn "id-scan\|no numeric revision floor\|dependencyRefs is \`\[\]\`" docs crates CLAUDE.md
```

#### Milestone gate

Run every item of **Final Acceptance** (below) by exit code, tick the boxes of this phase and of Final Acceptance, commit.

---

## Final Acceptance

- [ ] `cargo build --workspace`, `cargo test --workspace` (zero failures), `cargo clippy --workspace --all-targets -- -D warnings`, all by exit code.
- [ ] `cargo test -p srs-cli --test payload_contracts` passes with regenerated `package-export.json`.
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (this PR edits no schema or mirror; see Contracts).
- [ ] wasm32 build of `srs-bindings` green in CI.
- [ ] Every test named in Phases 1-5 exists and passes.
- [ ] Exports of first-party package boundaries succeed or refuse for a stated reason: `srs/srs` (fresh `origin/master` clone, all boundaries) and the muSrs essay package (S49). For `srs/srs` specifically, confirm no own definition collides with a different-content copy in another boundary (PD3 makes that fatal). Record any refusal (unresolved reference or identity conflict) in the PR body; a real-corpus refusal is a finding to file, not a reason to loosen the rule.
- [ ] PR body states decision mode (complicated), door (Door 1, RFC-003 Rev 10 / `rfc-decision-a8dcbfe5`), `gate:owner-merge`, `Closes #1212`, and that it merges after srs PR #874.

## Deferred items (file during Phase 6, linked)

- D1. F1 (Protocol `FieldRef.fieldId` row): **folded into srs PR #874** (ruled O3; done separately). Not a deferred issue. Phase 6 checks the merged `mechanism-64469ada` names it.
- D2. srs: update `srs-usage.md` §5g (export payload: `dependencyRefCount`, `dependencyRefs` semantics, [C6] refusal codes) once the release carrying this ships.
- D3. srs-web#339: comment that the bundle bytes and sha256 change with this release; re-pin after it.
- D4. Spec-door question (srs, linked to srs#857): should a reader check a bundle's `dependencyRefs` against the destination before install (refuse, or report like RFC-044 [R9])? RFC-003 deliberately adds no such reader rule, and the implementation follows it: today a `standalone` bundle installed without its dependencies succeeds and leaves the destination's checked catalog fatal (Type `fieldId` R13). Door 2 candidate or an RFC-003 revision; not an implementation gap.
- D5. Spec finding F4 (srs, parent muDemocracy.org#242): RFC-003 leaves implicit whether `standalone` `dependencyRefs` include definitions reached transitively through definitions that are listed but not carried. The plan reads [C1] literally ("references made by a carried definition"), so they are not listed. Ask for one sentence in Change C / `mechanism-b63bc07f`.
- D6. Owner question O5 (no issue until ruled): should export refuse a repository stamped below the reader floor instead of only noting it (PD11)?
- Existing, not refiled: srs-rust#1164 (readme), #1208 (catalog core fields), #1213 (core-bundle rename), srs#873 (spec-side table guard).

## What breaks

- Every `.srspkg` written by srs-rust#1210 differs from the new output (dependencyRefs, stamp, schemaVersion, order, description): re-export. None is committed anywhere (RFC-003 What breaks); srs-web#339 has not pinned one.
- A repository with a dangling PINNED/LINEAGE reference (the gallery fixture's `lifecycleRef`s) can no longer export the affected package; it could before. Intended ([C1]).
- A hand-made or pre-RFC bundle stamped below 7 or unstamped is refused on install; it was read before. Intended ([C6]).

## Review resolutions (Stage 3)

| Finding | Resolution |
|---|---|
| Arch #1 (blocking) core compared by raw content | PD3/PD6 rewritten: core holdings from `core_package()`; core vs boundary uses ADR-025's id+version+namespace/name rule; canonical-content comparison only boundary vs boundary; `core_definition_values()` dropped. Tests `export_boundary_copy_of_core_field_is_not_a_conflict`, `export_core_id_with_different_name_is_identity_conflict`. |
| Arch #2 S49 standalone count; F4 | S49 says "not greater than"; F4 recorded under Spec findings, deferred as D5 (parent muDemocracy.org#242). |
| Arch #3 lineage row kinds | Row names Field and Type only. |
| Arch #4 PD3 own-vs-other conflict | PD3 states it blocks; test `export_refuses_own_definition_conflicting_with_another_boundary`; Final Acceptance note for `srs/srs`. |
| Arch #5 Restamp writes `to`; rev-9 memberOrder | Chain writes the stamp after every step; tests `read_bundle_restamp_writes_each_step_target`, `read_bundle_rev9_with_member_order_is_not_migrated`. |
| Arch #6 repository below reader floor | No new refusal (O5 open). PD11: summary `notes` entry plus pinning test `export_from_repository_below_reader_floor_writes_bundle_its_reader_refuses`; payload gains `notes`. |
| Arch #7 `Option<BundleMode>` | `#[serde(default)] mode: BundleMode`. |
| Arch #8 `homepage` | Added (PD12): input field, CLI `--homepage`, test `export_writes_homepage_when_given`. |
| Arch #9 duplicate ADR-032 task; ADR-050 notes | ADR-032 tasks merged into one; ADR-050 pointer notes trimmed to §2, §5, Deviation (Rejected paragraph edited, not annotated). |
| Arch #10 orthogonality | ADR-032 amendment text says `revision_step` is orthogonal to `status_fn`/`apply_fn`. |
| Arch #11 D4 phrasing | D4 rephrased as a spec-door question. |
| Plan #1 (blocking) schema-sync ambiguity | Contracts and Final Acceptance state this PR edits no schema or mirror; srs#874's text arrives via `schema-sync.yml`. |
| Plan #2 Phase 6 gate | Gate now runs every Final Acceptance item. |
| Plan #3 "listed below" | Points at the phase's Testing section. |
| Plan #4 test labels | Phase 3 tests labelled new / modified (replaces X) / Kept; Phase 4 new tests labelled. |
| Plan #5 Phase 1 criteria | Reworded as behaviours. |

## Declined review findings

None declined. Arch #6's stronger option (refusing the export) is deferred to the owner as O5 rather than declined.

## Owner decisions (RULED 2026-10-03)

- **O1 — RULED YES.** The registry carries each revision step's bundle form (PD2 as written); ADR-032 amended per Phase 6.
- **O2 — RULED.** Rust const table plus schema guard now (PD1). The spec-side schema-annotation proposal is posted on srs#873 by the orchestrator; ADR-050's amendment and the `reference_sites.rs` module doc reference srs#873 for it. Nothing further here.
- **O3 — RULED.** The Protocol `FieldRef.fieldId` row is folded into srs PR #874 (separate agent). D1 is closed as "folded into srs#874".
- **O4 — RULED BUILD.** `standalone` mode is built in this PR (PD5 revised): service `mode` input, CLI `--mode`, WASM input field, payload echoes `mode`, golden schema regenerated, ADR-050 drops "rejected: standalone", tests in Phases 3 and 5, S49 step. Consequence stated and asserted: install is unchanged, so a standalone bundle installed without its dependencies leaves a fatal catalog (D4).
- **O5 — OPEN (from arch review #6).** Export from a repository stamped below the reader floor: today note-only (PD11). Refusing would be a new rule. Recommendation: keep note-only; a repository that old cannot load through ordinary commands in most corpora anyway.

## Deviations during implementation

- **Fixture fix (Phase 3).** `crates/srs-repository/tests/fixtures/install-package/protocols/entry-9a1b0c90.json` carried `targetType: "com.example.install/entry"` (a display key at a LINEAGE UUID site, which the schema text forbids; the validator does not assert `format: uuid`). The typed closure reports it as `bundle-reference-unresolved`, so the fixture now holds the entry Type's UUID `9a1b0c30-0003-4aaa-8bbb-000000004001`. No other test read that value. PD10's premise ("the clean `install-package` fixture") now holds.
- **Test repositories are created at the current revision (Phase 3).** The unit-test `fresh()` helper used `store.initialize_repository`, which stamps revision 2; the export now writes the repository's own stamp, so a revision-2 bundle would be below the reader floor. `fresh()` now calls `repository_lifecycle::create_repository` (stamps current), the same path `srs repo create` takes.
- **`export_follows_references_transitively` (Phase 3)** uses Type -> base Type (`extendsTypeId`) -> Field, both in `packages/b`, instead of View -> Type -> Field: a View holds no Type site (F2), so the planned chain cannot exist.
- **`FollowedReference.version` is `Option<u64>`**, not `Option<u32>`: definition versions are read with `as_u64` and the effective index is keyed by `u64`; a `u32` would only add casts.
- **Temporary `#[allow(dead_code)]`** on `RevisionStep`/`BundleForm` between Phases 1 and 4 (and on `reference_sites` between Phases 2 and 3), so each milestone commit passes `clippy -D warnings`; removed when the consumer landed.

- **`read_bundle_rev9_with_member_order_is_not_migrated` pins `bundle-definition-invalid`** (Phase 4): the observed single code. `package-bundle.json` does not constrain the inner shape of a Composition, so the bundle-level schema check passes and the per-definition check (`validate_source_definition`) refuses it; the message names `memberOrder`.
- **Revision-9 assertion for the 7 -> 9 path (Phase 4).** The read result does not expose the stamp, so the "ends at revision 9 through both steps" check is the unit test `read_bundle_restamp_writes_each_step_target` (stamp after each step) plus a sibling integration test `read_bundle_rev7_reaches_current_revision` (the revision-9 schema accepts the carried-forward bundle); `read_bundle_migrates_pre_rev8_member_order` is unchanged. `read_bundle_schema_error_on_old_bundle_names_revision` is replaced by `read_bundle_below_7_names_the_missing_step` as planned.
