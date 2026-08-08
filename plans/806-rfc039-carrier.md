# Plan: RFC-039 revision-2 carrier — srs-rust implementation (srs#242 Phase B unit 1, srs-rust#806)

> Implements the Accepted RFC-039 (Rev 6) name-keyed `fieldValues` carrier in the Rust stack.
> This is **unit 1 of the #242 Phase B / #297 release train** — the critical-path unit nothing
> else in the train can precede. Units 2–3 (the `srs` data cutover, mirrors, srs-web,
> srs-vscode, muSrs) are sequenced under "Train choreography" below and are **out of this
> plan's executable scope**.

## Summary

RFC-039 (Accepted Rev 6) replaces `Record.fieldValues` — an array of `{fieldId, value}` pairs —
with an object keyed by `Field.name` verbatim, over one recursive value rule identical to
RFC-032 `projectField`'s instance space. It retires `FieldGroup`/`groupValues`,
`ext:repeatable-fields` `entries`, and the `FieldAssignment.{repeatable,minItems,maxItems}` trio,
adds a parallel `fieldMeta` provenance sibling, and re-keys the DocumentView projection and
Tier-1 value vocabulary (`TypedField.valueType` → inline `fieldType`, [R8]). The `srs` spec
repo's data cannot migrate until a released `srs` binary reads the new shape (CI pins
`SRS_RUST_CLI_TAG`), so the Rust implementation lands first. This plan delivers it: core types
+ one shared value-shape rule, validation ([R1]–[R9], [R14], [R16]–[R19]), loading ([R9]
fail-closed generation discrimination), rendering (RFC-036 composite re-base, Change I
sunsets), schema projection ([R2a], `x-srs-field-id` retirement), the deterministic Phase 0/1/2
migration transform (registered as data-model migration #2), all adapter surfaces (CLI, WASM,
MCP, gov), and the full fixture migration (~294 files carrying `"fieldValues"`).

## Train choreography (context — decided at the Stage-2 checkpoint, srs#242 decision comment)

The circular dependency and its resolution, per `srs-rust/CLAUDE.md:125-128` (master): mirror
drift jobs always compare against `srs` **master**, so "there is no merge order that avoids a
red CI window, and none is required" — the spec change and its mirror are **"one landing
executed close together"**, pre-staging the mirror while the spec PR is open and accepting the
red-by-construction drift job.

1. **This PR (srs-rust#806)**: carrier implementation + fixtures migrated + **schema mirror
   pre-staged** from the `srs` schema branch. All CI green **except** `Schema Drift` —
   red-by-construction until the `srs` PR (unit 2) merges. **`Schema Drift` is a required
   status check on `master`, so merging this PR requires the owner's admin override** — a
   deliberate, one-time consequence of the atomic cutover, stated in the PR body. The PR is
   `epic-256:owner-merge` class regardless, so the owner is the merger either way.
2. **`srs` cutover PR** (unit 2, next session): the 8 schema-file changes (authored on the
   `srs` branch this session creates, so the mirror pre-stage has a source), record fold-in
   (retire I-22–I-27, author new invariant records, amend RFC-012/019/031 records + RFC-033
   OQ2 re-booking per D3, `projection-rules.md` [R2a]), data migration of the five trees via
   the released unit-1 binary (`srs migrate`), the 13 `srs/scripts/**` updates, `srs-usage.md`
   rewrite, `SRS_RUST_CLI_TAG` bump, re-render, #317, deletion of the one live `null`.
   Structurally red until unit 1 merges and cuts a release; goes green on the tag bump. Unit
   1's merge turns `srs-rust` `master`'s own `Schema Drift` red for the window until unit 2
   merges — the documented expected state.
3. **Unit 3**: srs-vscode mirror resync (own pipeline), srs-web enumeration + adoption
   (srs-web is unenumerated — RFC-039 records this gap), muSrs migration (muDemocracy.org —
   RFC-032 definition pass + eight-item corpus audit + carrier + tables off JSON-in-text +
   `ext:themes-l1` declaration; runs the same released binary — no JS transform, per D2).

**Change-I removal gate (RFC-032 Change I / RFC-039 "Rev-7 cardinality-removal gate"):** five
conditions must be evidenced in unit 2's cutover PR. This PR supplies and links, as a 1:1
table in its body: condition 1 (final-schema rejection of every legacy carrier — [R7]/[R9]
tests), condition 4 (I-94, `[T-9]`, `[N+1]`, `is_text_searchable` switched to
cardinality-only, this PR), condition 5 (transition-conflict tests retained + scalar/list
acceptance tests added). Conditions 2 (zero-result corpus scan) and 3 (lossless migration +
round trip over the real corpus) are unit 2's, produced by running this PR's transform.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator + all worker roles | session main agent (single-session execution; Lead Integrator shepherds all phases and owns plan checkboxes) |
| Architecture Reviewer | subagent (sonnet) — plan review (pipeline Stage 3) and diff review (Stage 7) |
| Plan Reviewer | subagent (haiku) — plan review (Stage 3) |
| Verification Agent | subagent (haiku) — diff review (Stage 7) |

"Stage N" in this plan refers to the `/ship` pipeline stages, not plan phases; plan phases
gate themselves via the Milestone gate in Coordination Rules.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-043](../docs/adr/043-rfc039-carrier-representation.md) (new) | Revision-2 carrier: `field_values` as an order-preserving `FieldValues` map (IndexMap-backed) with `serde_json` `preserve_order` enabled workspace-wide + an explicit canonical-serialization discipline amending ADR-017's mechanism (determinism by canonical types, not by BTreeMap re-sorting); no legacy runtime reader ([R9] rejection); migration operates on raw JSON **through `RepositoryStore` generic-JSON methods inside the ADR-021 batch seam** — never through `Vfs` directly (ADR-041 G1/G6); one shared value-shape rule consumed by validation, projection conformance, and migration verification | proposed |
| [ADR-017](../docs/adr/017-deterministic-srsj-serialization.md) | **Amended by ADR-043**: determinism mechanism changes from implicit BTreeMap-backed `Value` sorting to `preserve_order` + canonical map types (`extra` maps become `BTreeMap`, carrier maps carry [R18] order) **+ an explicit canonicalize-on-write step in the `.srsj` writer** (recursively sort every object except `fieldValues`/`fieldMeta` subtrees, whose order is data); byte-churn on re-serialized goldens absorbed in this train | governs (amended) |
| [ADR-036](../docs/adr/036-srs-is-default-working-format.md) / [ADR-037](../docs/adr/037-mcp-adapter-surface.md) | **Both amended by ADR-043**: each states "`preserve_order` disabled" as a load-bearing invariant (ADR-036:22 for `.srsj` ordering; ADR-037:12 + its rmcp dependency audit). Cargo feature-unification propagates the flip to `srs-mcp`/rmcp, so the amendment must land with it: `.srsj` determinism is now guaranteed by the canonicalize-on-write step (stronger, path-independent), and MCP JSON-RPC output is key-order-insensitive by protocol — verified by re-running the MCP snapshot tests under the flag. The stale `srs-projection/src/json_schema.rs:14-17` comment is updated in the same commit | governs (amended) |
| [ADR-002](../docs/adr/002-tier2-generic-record-operations.md) | Generic record operations continue over the new carrier; accessors become name-keyed + Type-mediated fieldId recovery | governs |
| [ADR-004](../docs/adr/004-schemas-embedded-at-compile-time.md) | Embedded mirror validates instances → mirror must be pre-staged in this PR (choreography above) | governs |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) / [ADR-011](../docs/adr/011-cli-output-contract.md) | Typed service structs; payload structs + regenerated goldens record the contract break | governs |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) / [ADR-037](../docs/adr/037-mcp-adapter-surface.md) | Bindings/MCP stay thin; tool schemas updated to the object carrier | governs |
| [ADR-021](../docs/adr/021-jsonstore-batch-write-mode.md) / [ADR-024](../docs/adr/024-best-effort-rollback-multi-write-services.md) | The migration is a multi-write operation: `begin_batch` → transform → `commit_batch`, `abort_batch` on any abort disposition — no half-migrated store state ([R13]) | governs |
| [ADR-032](../docs/adr/032-migration-registry-fn-pointer-pattern.md) | The carrier transform registers as data-model migration #2 | governs |
| [ADR-041](../docs/adr/041-storage-backend-guardrails.md) / [ADR-042](../docs/adr/042-logical-id-instance-persistence.md) | Migration I/O: reads of unmigrated documents and writes of migrated documents route through the store's generic-JSON methods (`load_instance_json`/`save_instance_json` — ADR-042's sanctioned transitional surface; definition files analogously via store raw-JSON access) — **not** `Vfs`, not typed entities (the new typed layer would silently drop the trio serde-side, destroying E.2's `repeatable → cardinality` input) | governs |

Interop register (`srs/docs/research/alignment-opportunities.md`): item 1 (MCP server, weight
100) — `record_create`/`record_update` tool schemas change to the object carrier, improving
direct consumability; item 3 (agent-index/Skills packaging) and item 5 (JSON-LD export) both
benefit from name-keyed instances. No register entry is contradicted; the carrier is the
enabling move for the epic's "Type → standard JSON Schema by direct implementation"
requirement.

## Contracts

### CLI output contract (ADR-011)

**Existing command payloads change** (breaking, deliberate, rides the train): every payload
embedding `Record.fieldValues` (record get/list/create/update, context, container views,
document-view render projection, find). `payload.rs` structs update; `cargo run --bin
generate-schemas`; commit regenerated `schemas/payload/*.json` — the diff is the contract
record. `CreateRecordInput` takes `fieldValues` as an object + optional `fieldMeta`;
`groupValues` input is **removed**. `cargo test --test payload_contracts` must pass.

### Entity schema sync

**Yes — via pre-stage.** The 8 schema-file changes from RFC-039's Schema-changes table are
authored on `srs` branch `feat/242-phase-b-schemas` (this session) and copied into
`crates/srs-schema/schemas/2.0/` by `sync-schemas-from-spec.sh` against that local sibling
tree (Phase 5 task). `check-schema-sync.sh` passes locally against that tree; the CI
`Schema Drift` job (which compares against `srs` **master**) stays red-by-construction until
unit 2 merges — accepted, documented in the PR body, and requiring owner admin-merge (see
choreography).

## Scope

- `srs` branch `feat/242-phase-b-schemas`: the 8 schema files only (record, type,
  typed-record, package-bundle `$defs.Type` tighten, document-view-output, theme; field.json
  + manifest.json are no-ops; the three "until #242 Phase B" description clauses removed in
  type/document-view/theme). Pushed, no PR (unit 2 builds on it).
- srs-rust: everything under Phases 1–6 below, on `feat/806-rfc039-carrier`.

**Out of scope** (sequenced, not dropped): unit 2 (srs record fold-in + data migration +
scripts + srs-usage.md + tag bump + re-render + Change-I conditions 2–3 evidence + #317 +
the fixture-repo `null` deletion); unit 3 (srs-vscode mirror, srs-web adoption, muSrs
migration incl. the eight-item corpus audit); #308 guard (separate owner-merge issue);
metamodel round-trip (D3: re-booked to #272/#273 via unit 2's RFC-033 OQ2 amendment); Tier-1
object-map question (RFC-039 OQ6 — explicitly not settled by the RFC).

---

## Phases

### Phase 1: srs-core — carrier types, one value rule, validation, serialization discipline

**Goal:** the crate compiles with the revision-2 carrier as the only Record shape; one shared
value-shape rule; native validation implements the steady-state conformance rules;
deterministic serialization holds with [R18] order.

#### Tasks

- [x] Workspace: add `indexmap` (features `serde`); enable `serde_json/preserve_order`;
  update the `Cargo.toml:44` audit comment (it currently documents the *absence* of
  `preserve_order` as deliberate — ADR-043 changes the policy). **Feature-unification
  fallout (ADR-036/037 amendment):** verify MCP output is key-order-insensitive by protocol
  and that the `srs-mcp` snapshot tests pass under the flag; add the canonicalize-on-write
  step to the `.srsj` writer (recursively sort all object keys **except**
  `fieldValues`/`fieldMeta` subtrees) so ADR-036's determinism claim becomes
  construction-path-independent instead of relying on the disabled flag; update the stale
  `crates/srs-projection/src/json_schema.rs:14-17` comment
- [x] `crates/srs-core/src/types/record.rs`: `field_values: FieldValues` where
  `pub struct FieldValues(IndexMap<String, serde_json::Value>)` (newtype; serializes as a
  JSON object in map order; deserializes preserving file order); add
  `field_meta: Option<IndexMap<String, FieldMeta>>` with
  `FieldMeta {source?, edited_at?, source_refs?}` reusing `SourceReference`; **delete**
  `FieldValue`, `FieldValueEntry`, `FieldGroupValue`, `FieldGroupEntry`, `group_values`;
  new accessors `fn value(&self, name: &str)`, `fn field_id_for(name, effective_set)`
  (Type-mediated recovery)
- [x] Canonical-serialization audit (ADR-017 amendment): `Record.extra` and every other
  serialized entity map currently typed `HashMap` becomes `BTreeMap` (deterministic without
  the old BTreeMap-Value re-sort); enumerate every `serde_json::to_value(` call site in
  `crates/` and confirm each either feeds a canonical type or is order-insensitive; fix
  `json_store.rs` snapshot writing if it re-canonicalizes
- [x] `crates/srs-core/src/types/record_type.rs`: delete `Type.field_groups`, `FieldGroup`,
  `FieldAssignment.{repeatable, min_items, max_items}`, and the `effective-single`
  repeatable-conjunct helpers
- [x] **One value rule (shared primitive):** `crates/srs-core/src/validation/value_shape.rs`
  — `fn validate_value(field_type: &FieldType, value: &serde_json::Value, resolver: &dyn
  RangeTypeResolver) -> Result<(), ValueShapeError>` implementing Change B's single-case
  table + [R16] uniform list wrap + recursive `ref`/`inline` descent ([R3]). This is the
  **only** implementation of the value grammar: instance validation calls it; the migration
  verifies its output with it; a conformance test asserts `type_schema_service`'s emitted
  schema accepts exactly what it accepts over the fixture corpus (guards the
  RFC's "same rule read in two directions" property)
- [x] `crates/srs-core/src/validation/record.rs`: [R1] keys resolve in the effective field
  set + unknown-key rejection; [R2b] verbatim keys (no transform anywhere); [R5]
  required ⇒ key present, `null` rejected; [R6] `fieldMeta` keys ⊆ `fieldValues` keys, no
  `fieldMeta` inside composites; [R14] reference-mode integrity (target in `instanceIndex`,
  rangeType/typeVersion match)
- [x] `crates/srs-core/src/validation/record_type.rs`: [R4] effective-set `Field.name`
  uniqueness (definition-time diagnostic); [R7] removed-construct rejection at
  `dataModelRevision ≥ 2` (revision resolved from the enclosing manifest, plumbed by
  caller); [R15] a revision ≥ 2 manifest declaring `ext:field-groups` or
  `ext:repeatable-fields` is an **error**, not ignored (declaration-rejection half, distinct
  from the construct-deletion half)
- [x] [R9] generation discrimination at deserialization: array `fieldValues` ⇒ typed error
  naming the document and expected `dataModelRevision`; Tier-1 documents: `TypedField`
  without `fieldType` ⇒ same, in the Tier-1 validation path (see Phase 2 — Tier 1 has no
  typed struct; the check lives where Tier-1 JSON is read)
- [x] Cardinality-only predicate switch (**Change-I condition 4**; the five-condition gate is
  RFC-039 "Migration plan → Ordering", inherited from RFC-032 Change I): remove the
  transition-only `effective FieldAssignment.repeatable != true` conjunct from
  `is_text_searchable` (`field_type.rs`), `[T-9]` theme eligibility, `[N+1]`
  titleFieldId eligibility, and I-94 predicate-field checks (`validation/record_type.rs` +
  repository call sites). Keep the constructed negative tests from srs-rust#794; add
  scalar/list acceptance tests post-removal (condition 5)

#### Acceptance Criteria

- [x] `cargo test -p srs-core` green; no symbol `FieldValue`, `FieldGroupValue`,
  `FieldGroupEntry`, `group_values`, `repeatable` (assignment-level) remains in srs-core
- [x] A revision-1 record JSON fails deserialization with the [R9] diagnostic
- [x] A composite value validates recursively to depth ≥ 3 through `validate_value`
- [x] A Type with duplicate effective `Field.name`s is rejected at definition time ([R4])
- [x] Serializing a `Record` through `serde_json::to_value` **and** direct-to-writer both
  preserve `FieldValues` insertion order (the ADR-017-amendment property)

#### Testing

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

- `record::tests::rev1_array_field_values_rejected_with_r9_diagnostic`
- `record::tests::field_values_order_survives_to_value_roundtrip`
- `value_shape::tests::inline_composite_validates_depth_three`
- `value_shape::tests::list_wrap_uniform_for_map_and_dependent` ([R16])
- `record_type::tests::duplicate_effective_field_name_rejected` ([R4])
- `record::tests::field_meta_key_not_in_field_values_rejected` ([R6])
- `record::tests::null_value_rejected_key_absence_is_unset` ([R5])
- `record_type::tests::rev2_manifest_declaring_retired_extension_rejected` ([R15])
- predicate switch: existing negative tests from #794 pass unmodified; new
  `text_search::tests::list_cardinality_field_searchable_without_repeatable_flag`

#### Milestone gate

Per Coordination Rules. Commit message: `feat(core): RFC-039 revision-2 carrier types + value rule (#806)`.

### Phase 2: srs-repository — loading, services, rendering, projection

**Goal:** all repository services compile and behave over the new carrier; rendering derives
from structured values only; Tier-1 surfaces read `fieldType`.

#### Tasks

- [ ] `json_store.rs` / `file store` load paths: [R9] rejection with document-naming
  diagnostics; manifest/package `dataModelRevision` plumbing for [R7]
- [ ] `render_service.rs`: re-base `render_composite_table` + RFC-036 baseline renderer onto
  recursive name-keyed values; **delete `coerce_to_array`'s string branch** (`:2173`, no
  JSON-in-text); stop emitting unprefixed `field-label`/`field-value` aliases
  ([FR-037-14]); heading/body paths use name-keyed access; **[R5a] preserved**: structural
  presence (key present — validity) and rendering presence (RFC-001 Step 2 — `""` is
  absent, no row) stay distinct; a `required` field valued `""` validates and emits no row
- [ ] `text_projection.rs` + `discovery_service.rs`: Tier-2 text projection iterates
  `FieldAssignment.order` over the object carrier (RFC-012 amendment reading side, [R18]);
  Tier-1 handling reads inline `fieldType` in place of `valueType` ([R8]) and applies the
  Tier-1 [R9] structural test
- [ ] `type_schema_service.rs`: assert domain Types emit `Field.name` verbatim ([R2a] —
  existing behaviour, add the conformance test); **remove `x-srs-field-id`**
- [ ] Document-view projection ([R11]): `ProjectedRecord.fields`/`orderedFieldKeys` keyed by
  `Field.name`; composites recurse under their own key; `fieldGroups` projection +
  `ProjectedFieldGroup`/`ProjectedGroupEntry` removed
- [ ] Remaining call sites (record_service, context_query_service, container_view_service,
  agent_index_service, tag_service, view_service, lifecycle, repository_portability):
  name-keyed access via the Phase-1 accessors
- [ ] Theme handling: runtime support for `groupFieldRowTemplates` removed ([R12]'s
  carry-over is migration-only, Phase 3)

#### Acceptance Criteria

- [ ] `cargo test -p srs-repository` green (fixture-dependent tests may be red until
  Phase 5 — tracked, not silently skipped: the gate for this phase is compile + non-fixture
  unit tests; the 12 composite/table tests re-base here and go green against Phase-2-local
  in-memory stores)
- [ ] `composite_table_no_raw_json_in_output` trivially true; `render_composite_table`
  consumes only structured values
- [ ] No `group_values`/`x-srs-field-id`/`groupFieldRowTemplates` symbol in the crate outside
  the migration module

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

- The 12 composite/table tests in `render_service.rs` (`render_table_markdown_*`,
  `render_table_html_*`, `composite_table_*`) re-based onto structured input
- `type_schema_service::tests::domain_type_keys_are_field_name_verbatim` ([R2a])
- `type_schema_service::tests::no_x_srs_field_id_emitted`
- `type_schema_service::tests::projected_schema_covers_field_values_only_not_envelope` ([R17])
- `render_service::tests::required_empty_string_validates_and_emits_no_row` ([R5a])
- `document_view::tests::projected_fields_keyed_by_name_and_recurse` ([R11])
- `text_projection::tests::tier2_iterates_assignment_order` ([R18] read side)
- `discovery::tests::tier1_field_type_read_and_rev1_rejected` ([R8]/[R9])

#### Milestone gate

Commit: `feat(repository): name-keyed carrier services + RFC-036 composite re-base (#806)`.

### Phase 3: the migration transform (data-model migration #2)

**Goal:** a deterministic, total, abort-not-skip transform any first-party tree (and later
muSrs) runs through the released binary — store-routed, batch-wrapped, idempotent.

#### Tasks

- [ ] New `crates/srs-repository/src/rfc039_carrier_migration_service.rs` operating on **raw
  `serde_json::Value` documents obtained and written through `RepositoryStore`'s
  generic-JSON methods** — `load_instance_json`/`save_instance_json`, used for definition
  documents too (existing precedent: `package_service.rs` tests; there is no separate raw
  definition method) — never `Vfs` (ADR-041 G1), never typed entities (the new typed layer
  silently drops the trio on deserialize, which would destroy E.2's `G.repeatable →
  cardinality: "list"` input). **This is a named, argued exception to the CLAUDE.md/ADR-042
  doctrine that instance persistence goes through typed logical-id methods**: a format
  migration is the one consumer that must see documents the typed layer refuses; ADR-043 §3
  records it against #725/#726, and the migration service is added to #725's exemption list
  by a comment when this lands (it is migration-only, bounded, and does not grow the
  general-purpose caller pile #726 exists to shrink). The whole run is wrapped
  `begin_batch` → … → `commit_batch`, with `abort_batch` on every abort disposition
  (ADR-021/ADR-041 G6) so no store is ever half-migrated ([R13])
- [ ] Phase 0 (definitions): E.2 FieldGroup→composite minting (fresh UUIDs logged to the
  migration log; `<T.name>_<G.groupId>` snake_case range Type; `required = minItems ≥ 1`),
  strip the trio from every FieldAssignment, Tier-1 `valueType`/`selectOptions` → inline
  `fieldType`, Type version bumps + the zero-referent deletion marker
- [ ] Phase 1 (instances, enumerated from `instanceIndex` [R13]): typeVersion re-pin;
  pair→key transform ([R10] abort on unresolvable fieldId); [R20] value/entries agreement;
  recursive `groupValues` descent (steps 2–3 at every depth, step 5 at depth 0 only);
  per-value provenance → `fieldMeta`
- [ ] Phase 2 (repository): theme `groupFieldRowTemplates` → `compositeFieldRowTemplates`
  carry-over ([R12] abort on unmatched key); `dataModelRevision: 2` stamping (repo manifest,
  package manifests, `.srsj` envelopes); [R13] count assertion; zero-referent Type version
  deletion
- [ ] The full 11-row totality disposition table from RFC-039 — every branch implemented and
  tested (aborts produce named diagnostics; the two logged-notice rows log and count)
- [ ] [R18] output ordering: `FieldAssignment.order` at every depth via the `FieldValues`
  carrier; re-run byte-idempotence
- [ ] Round-trip verifier: old → new → logical comparison modulo the two declared
  non-round-trippable classes (valueless pairs; dual-written value/entries), emitting a
  counted audit log — the evidence artifact unit 2 cites (Change-I condition 3 input)
- [ ] Post-transform verification: every migrated Record re-validated through Phase 1's
  `validate_value` + [R1]–[R6] — the migration does **not** reimplement the value grammar
- [ ] Register as migration #2 in `migration_registry_service.rs` (`status_fn` = [R9]
  structural test over the store); exposed via the existing `srs migrate` surface — **no new
  public upgrade command** (RFC-038 constraint)
- [ ] Explicit-path mode for manifest-less trees (`tests/rfc-032/` reconciliation:
  `values` → `fieldValues`), used by unit 2

#### Acceptance Criteria

- [ ] Transform over a fixture copy of a revision-1 repository yields a repository that
  loads clean, validates with 0 errors, and re-runs byte-identically
- [ ] Every abort disposition covered by a test; an abort leaves the store untouched
  (batch rollback verified)
- [ ] Round-trip audit log counts match the fixture's known population

#### Testing

```bash
cargo test -p srs-repository rfc039
cargo clippy -p srs-repository -- -D warnings
```

- `rfc039_migration::tests::field_group_minted_as_composite_with_version_bump` (E.2)
- `rfc039_migration::tests::trio_stripped_from_types_without_groups` (0b)
- `rfc039_migration::tests::dual_written_entries_taken_from_value_and_asserted` ([R20])
- `rfc039_migration::tests::divergent_entries_abort` ([R20])
- `rfc039_migration::tests::unresolvable_field_id_aborts_and_rolls_back` ([R10] + ADR-021)
- `rfc039_migration::tests::nested_group_values_recurse_depth` (step 4 recursion)
- `rfc039_migration::tests::valueless_pair_omitted_and_logged`
- `rfc039_migration::tests::rerun_is_byte_idempotent` ([R18])
- `rfc039_migration::tests::instance_count_asserted_against_index` ([R13])
- `rfc039_migration::tests::migrated_output_passes_value_shape_validation` (shared rule)

#### Milestone gate

Commit: `feat(repository): RFC-039 carrier migration as data-model migration #2 (#806)`.

### Phase 4: adapters — CLI, bindings, MCP, gov

**Goal:** every surface speaks the object carrier; no adapter carries logic.

#### Tasks

- [ ] `srs-cli`: `payload.rs` structs (object `fieldValues` + optional `fieldMeta`;
  `groupValues` removed from `CreateRecordInput` and all outputs); regenerate goldens
  (`cargo run --bin generate-schemas`); handlers otherwise untouched
- [ ] `srs-bindings` (5 files): shapes follow the services; no logic
- [ ] `srs-mcp` (2 files): `record_create`/`record_update`/`note_graduate` tool input
  schemas + shadow structs re-documented to the object carrier
- [ ] `srs-gov` (3 files): name-keyed access; `x-srs-field-id` consumers removed;
  `governance-seed.srsj` regenerated through the Phase-3 transform

#### Acceptance Criteria

- [ ] `cargo test --test payload_contracts` green after regeneration
- [ ] `cargo test -p srs-cli -p srs-bindings -p srs-mcp -p srs-gov` green except
  fixture-dependent tests pending Phase 5 (tracked list, not silence)

#### Testing

```bash
cargo run --bin generate-schemas && git diff --stat crates/srs-cli/schemas/payload/
cargo test --test payload_contracts
cargo test -p srs-cli -p srs-bindings -p srs-mcp -p srs-gov
```

- Golden schema diffs reviewed as the contract-change record (ADR-011)
- MCP tool-schema snapshot tests updated in `crates/srs-mcp/tests/`

#### Milestone gate

Commit: `feat(cli,bindings,mcp,gov): object carrier across adapter surfaces (#806)`.

### Phase 5: fixtures + srs schema branch + mirror pre-stage

**Goal:** the workspace is self-consistent on revision 2; CI green except `Schema Drift`.

#### Tasks

- [x] Author the 8 schema-file changes (RFC-039 Schema-changes table) on `srs` branch
  `feat/242-phase-b-schemas`; validate each against its meta-schema; push the branch
  *(done during Phase 1 — the schema-contract tests in every crate validate against
  the embedded mirror, so the pre-stage had to precede the Phase-1 gate)*
- [x] Pre-stage the mirror: run `scripts/sync-schemas-from-spec.sh` against the local
  sibling checked out on `feat/242-phase-b-schemas` *(done during Phase 1; SHA256SUMS
  regenerated by the sync script)*
- [ ] Migrate every fixture repository under `crates/*/tests/fixtures/**` through the
  Phase-3 transform (dogfood); rewrite inline test JSON by hand; re-pack `core-bundle.srsj`,
  `governance-seed.srsj`, `gallery.srsj` deterministically (ADR-017 as amended)
- [ ] Verified pre-condition (checked 2026-08-08): no srs-rust fixture carries the dangling
  `f1a2b3c4-…4c5c` fieldId (#307's class) — re-grep after fixture migration as a guard
- [ ] Keep two revision-1 fixtures as **named** `legacy-rev1-*` [R9] rejection-test inputs
  (not loadable data)

#### Acceptance Criteria

- [ ] Full `cargo test` green; `cargo clippy -- -D warnings` green
- [ ] `bash scripts/check-schema-sync.sh` green against the sibling schema branch
- [ ] `git grep -c '"fieldValues": \['` over `crates/` returns matches only in
  `legacy-rev1-*` fixtures and migration test inputs

#### Testing

```bash
cargo test
cargo clippy -- -D warnings
bash scripts/check-schema-sync.sh
git grep -n '"groupValues"' -- crates/ | grep -v legacy-rev1 | grep -v rfc039_migration
```

#### Milestone gate

Commit: `test(fixtures): migrate fixture corpus to revision-2 carrier + pre-stage schema mirror (#806)`.

### Phase 6: docs, dogfood, PR

**Goal:** docs match the code; dogfood scenario proves the carrier end-to-end; PR open.

#### Tasks

- [ ] ADR-043 status → `accepted`; ADR-017 amendment note cross-referenced
- [ ] `srs-rust/CLAUDE.md`: add `srs-gov` row to the Crate Authority table (gap RFC-039
  flags); caveat on the "Working with the Spec Repo" `repo validate` line (a revision-2
  binary rejects the pre-cutover spec repo until unit 2 lands — expected [R9] behaviour);
  update the `Cargo.toml` `preserve_order` policy note reference
- [ ] `docs/dogfooding.md`: new scenario — repo create → author a Record with a composite
  list value via `srs record create` (object `fieldValues`) → render → validate; negative:
  array-shape `fieldValues` rejected with the [R9] diagnostic; update the Coverage matrix
- [ ] PR per pipeline Stage 8: body carries the choreography, the required-check
  admin-merge note, and the Change-I five-condition 1:1 mapping table

#### Acceptance Criteria

- [ ] ADR-043 `accepted`; CLAUDE.md and dogfooding.md updates committed; every touched doc
  command block actually runs
- [ ] Dogfood scenario passes end-to-end on a fresh `/tmp` repo with the branch binary
- [ ] PR open with the three required body elements above

#### Testing

```bash
cargo build --bin srs
# dogfood per docs/dogfooding.md scenario steps (happy + negative)
```

#### Milestone gate

Commit: `docs: RFC-039 carrier docs + dogfood scenario (#806)`.

---

## Final Acceptance

- [ ] `cargo test` + `cargo clippy -- -D warnings` green
- [ ] `cargo test --test payload_contracts` green (goldens regenerated)
- [ ] `bash scripts/check-schema-sync.sh` green against the pre-staged mirror / sibling
  `feat/242-phase-b-schemas` tree; CI `Schema Drift` red-by-construction documented in the
  PR body together with the required-check admin-merge note
- [ ] Migration idempotence + rollback + round-trip audit tests green
- [ ] Round-trip fixture evidence produced (unit 2 re-runs it over the real spec corpus —
  Change-I conditions 2–3 are unit 2's)
- [ ] No symbol `FieldValue`, `FieldGroupValue`, `FieldGroupEntry`, `group_values`,
  `x-srs-field-id`, `groupFieldRowTemplates` outside the migration module and
  `legacy-rev1-*` [R9] tests
- [ ] `srs` branch `feat/242-phase-b-schemas` pushed and named in the PR body

## Coordination Rules

Single-session execution; the main agent is Lead Integrator and sole worker; reviewer
subagents are read-only. **Milestone gate, every phase:**

1. Verify all acceptance criteria — check each checkbox in this file.
2. Confirm every test named in the phase's Testing section exists and passes.
3. `cargo test -p <crate>` && `cargo clippy -p <crate> -- -D warnings` (workspace-wide at
   Phases 5–6).
4. Update this plan file: mark task and acceptance checkboxes `[x]`.
5. Commit with the phase's stated message. Do not start the next phase before the gate
   passes.

Decisions D1–D3 (session scope, Rust-only transform, OQ1 re-booking) recorded at srs#242
comment `epic-256:decision-ack:phase-b-unit-1`.

## Assumptions

- **RFC-039 OQ2 is settled** (owner, 2026-08-08, reversal recorded in the epic ledger): the
  33 kebab-case Fields were renamed to snake_case in place — srs#358 + srs-rust#802, both
  merged — so the corpus keys snake_case verbatim and no grandfather list exists. #308
  (fail-closed guard) rides separately.
- The dangling-fieldId blocker (#307) is fixed on `srs` master (PR #362); srs-rust fixtures
  verified clean of the UUID.
- The `srs` schema branch is authored by this session; its PR (unit 2) is a follow-up
  session; nothing in unit 1 merges spec data.
- muSrs corpus audit (RFC-039's eight measurements) happens at unit 3 before the muSrs
  transform runs; the transform's abort rules make a surprise non-zero result safe.
- `Schema Drift` being a required check means unit 1's merge needs the owner's admin
  override — a consequence of the D1 decision, named in the PR body, not a new decision.
