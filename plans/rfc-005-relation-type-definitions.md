# Plan: RFC-005 — Core Relation Type Definitions

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

RFC-005 promotes `RelationTypeDefinition` from optional package metadata to a required, validated, first-class package component. Every `Relation.relationType` string must resolve to an installed definition before a relation is accepted. This plan implements that RFC in Rust: updating the embedded schema artifact, adding a `RelationTypeDefinition` core type, adding validation invariants (E1–E4), adding a package-level loader for relation type definitions, wiring validation into the repository write path, shipping the seven canonical SRS definitions in the spec package, and exposing `srs relation-type` CLI commands.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Core Model Worker | — |
| Repository Service Worker | — |
| CLI Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs — this plan implements RFC-005, which was reviewed and approved independently.

| ADR | Decision | Status |
|---|---|---|
| [ADR-002](../docs/adr/002-tier2-generic-record-operations.md) | Tier 2 record operations are generic | accepted |
| [ADR-004](../docs/adr/004-schemas-embedded-at-compile-time.md) | Schemas embedded as pinned artifact | accepted |

---

## Scope

- Update `srs/docs/schema/2.0/relation-type.json` with new required fields (`id`, `version`, `createdAt`) and optional fields (`irreflexive`, `allowedSourceTypes`, `allowedTargetTypes`, `requireSameSemanticObjectType`, `status`, `updatedAt`); expand `category` enum; update `relationType` description
- Update `srs/docs/schema/2.0/package-bundle.json` — `Reference.definitionType` gains `"relation-type"`
- Sync the updated schemas into `crates/srs-schema/schemas/2.0/` and regenerate `SHA256SUMS`
- Add `RelationTypeDefinition` struct to `srs-core`
- Add `validate_relation_type_definition` to `srs-core`
- Add RFC-005 Relation validation invariants (E1–E4) to `srs-core`
- Add relation type definition loader to `srs-repository` package loading path
- Wire E1–E4 validation into `srs-repository` relation write and load paths
- Ship seven canonical definition files in `srs/srs/package/relation-types/` and update `srs/srs/package/package.json`
- Add `srs relation-type list/get` CLI commands

**Out of scope:**

- `srs relation-type create/update/delete` (deferred — no write service for package definitions yet; covered by the CLI command structure plan)
- Transitive/symmetric query inference, cardinality enforcement, lifecycle constraints (deferred per RFC-005)
- `relationTypeVersion` on `Relation` (deferred per RFC-005)
- `ext:federation` external endpoint deferral

**`ext:import-tracking` prerequisite:** RFC-005 requires `"relation-type"` in the `Reference.definitionType` enum of `package-bundle.json`. This is handled in Phase 1 as a required schema edit (not deferred). The `ext:import-tracking` vocabulary for cross-repo relation type references is a separate concern and remains out of scope.

---

## Phases

### Phase 1: Update Spec Schemas and Sync Rust Artifact

**Goal:** The canonical JSON schemas reflect the RFC-005 shape and the Rust schema artifact is in sync.

**Agent:** Lead Integrator

**Write scope:** `srs/docs/schema/2.0/`, `srs/srs/package/`, `srs/srs/relations/`, `crates/srs-schema/`

#### Tasks

- [ ] Edit `srs/docs/schema/2.0/relation-type.json`:
  - Add `id` to `required[]` (UUID format)
  - Add `version` to `required[]` (integer, minimum 1)
  - Add `createdAt` to `required[]` (string, date-time format)
  - Update `relationType` description to: "Canonical bare string (e.g. `supersedes`) or custom `namespace/name` string (e.g. `org.example/my_type`). Must be globally unique across the effective installed package set."
  - Expand `category` enum to: `["composition", "refinement", "dependency", "sequence", "derivation", "evidence", "governance", "association", "lifecycle", "provenance", "other"]`
  - Add optional properties: `irreflexive` (boolean), `allowedSourceTypes` (array of strings), `allowedTargetTypes` (array of strings), `requireSameSemanticObjectType` (boolean), `status` (enum: `"active"`, `"deprecated"`, `"tombstone"`, `"retired"`), `updatedAt` (string, date-time format)
  - Keep `additionalProperties: false`
- [ ] Edit `srs/docs/schema/2.0/package-bundle.json`:
  - Add `"relation-type"` to `$defs/Reference/properties/definitionType/enum`
  - Update `relationTypes[]` array description from "ext:recommended-relations relation type metadata" to "Relation type definitions included in this bundle"
- [ ] Create `srs/srs/package/relation-types/` directory with seven canonical definition files (filenames: `contains.json`, `depends-on.json`, `supersedes.json`, `refines.json`, `derived-from.json`, `evidences.json`, `precedes.json`) using the exact JSON from RFC-005 section "Canonical Relation Type Definitions". Each file must include the new required fields (`id`, `version`, `createdAt`).
- [ ] Update the four existing spec RFC relation type files — add required fields and namespace the `relationType` values (bare strings are reserved for the seven public canonical types; all other types must use `namespace/name` format):
  - `srs/srs/package/spec-rfc-process/relation-types/rfc-targets-section.json` → `relationType: "com.semanticops.spec/rfc-targets-section"`
  - `srs/srs/package/spec-rfc-process/relation-types/rfc-change-sequence.json` → `relationType: "com.semanticops.spec/rfc-change-sequence"`
  - `srs/srs/package/spec-rfc-process/relation-types/rfc-revision-sequence.json` → `relationType: "com.semanticops.spec/rfc-revision-sequence"`
  - `srs/srs/package/spec-rfc-process/relation-types/rfc-proposed-artifact-sequence.json` → `relationType: "com.semanticops.spec/rfc-proposed-artifact-sequence"`
  - Assign each a new UUID4 `id`, `"version": 1`, and a `createdAt` timestamp. Keep `namespace: "com.semanticops.spec"`.
  - The `namespace` in each of these four files is `com.semanticops.spec` (not `com.semanticops.srs`).
- [ ] Create `srs/srs/package/spec-authoring-core/relation-types/` directory with definition files for the five SRS-specific relation types used in `srs/srs/relations/relations.json` that have no existing definition. These types use `namespace/name` format for `relationType` (per the schema description that bare strings are reserved for the seven public canonical types; SRS-internal types must be namespaced):
  - `section-sequence.json` — `relationType: "com.semanticops.srs/section-sequence"`, `category: "sequence"`, `irreflexive: true`
  - `subsection-sequence.json` — `relationType: "com.semanticops.srs/subsection-sequence"`, `category: "sequence"`, `irreflexive: true`
  - `design-note-sequence.json` — `relationType: "com.semanticops.srs/design-note-sequence"`, `category: "sequence"`, `irreflexive: true`
  - `extension-dependency.json` — `relationType: "com.semanticops.srs/extension-dependency"`, `category: "dependency"`, `irreflexive: true`
  - `explains.json` — `relationType: "com.semanticops.srs/explains"`, `category: "evidence"`, `irreflexive: true`
  - Each file must include `id` (new UUID4), `version: 1`, `createdAt`, `relationType`, `namespace: "com.semanticops.srs"`, `label`, `description`, `category`.
  - **Also update** the `"type"` values in `srs/srs/relations/relations.json` for these five types to the namespaced form as part of the relations.json migration in the same task.
- [ ] Edit `srs/srs/package/package.json` — add a `"relationTypes"` array listing the seven canonical paths plus the five new spec-authoring-core paths plus the four RFC-process paths (all definitions in the effective root package set must be declared at the root). The four RFC-process definitions remain on disk under `spec-rfc-process/relation-types/` but are referenced by path from the root `package.json`.
- [ ] Run `scripts/sync-schemas-from-spec.sh` to copy updated schemas into `crates/srs-schema/schemas/2.0/`
- [ ] Verify `scripts/check-schema-drift.sh` passes
- [ ] Migrate `srs/srs/relations/relations.json` from the legacy grouped shape to the `relations-collection.json` flat shape using these exact rules per legacy record type. Drop `id` and `description` from every legacy record (no flat schema equivalent). Drop `label` from every legacy record **except** flat `from`/`to` records, where `label` maps to `notes` (see rule below). Set `"createdAt": "2026-05-29T00:00:00Z"` on every emitted relation.

  - **`from` + `members[]` grouped records** (`subsection-sequence`, `rfc-change-sequence`, `rfc-revision-sequence`, `rfc-proposed-artifact-sequence`): These encode "parent owns these ordered children." Emit two kinds of flat relations — do **not** emit a relation using the sequence type name itself:
    1. One `contains` relation per member: `sourceInstanceId: from`, `targetInstanceId: member[i]`, `relationType: "contains"`, new UUID4 `relationId`.
    2. One `precedes` relation per adjacent pair `(members[i], members[i+1])`: `sourceInstanceId: members[i]`, `targetInstanceId: members[i+1]`, `relationType: "precedes"`, new UUID4 `relationId`. (No `precedes` for a single-member list.)

  - **`members[]`-only grouped records without `from`** (`section-sequence`, `design-note-sequence`): Global ordered list, no parent. Emit only `precedes` relations per adjacent pair: `sourceInstanceId: members[i]`, `targetInstanceId: members[i+1]`, `relationType: "precedes"`, new UUID4 `relationId`. No `contains` (no `from`).

  - **Existing flat `from`/`to` records** (`rfc-targets-section`): Field rename and namespace:
    - Drop `id` (was a slug, not a UUID).
    - `type` → `relationType` in namespaced form: `"rfc-targets-section"` → `"com.semanticops.spec/rfc-targets-section"`.
    - `from` → `sourceInstanceId`, `to` → `targetInstanceId`.
    - `label` → `notes`.
    - Assign a new UUID4 `relationId`.

  - Update the `$schema` reference to `relations-collection.json`.

- [ ] Update all view files that reference relation types as machine values (not prose descriptions). Since grouping relations now expand to `precedes`, sequence-type views must use `"precedes"`:
  - `srs/srs/package/views/srs-spec-document-view.json`: `"section-sequence"` → `"precedes"`
  - `srs/srs/package/spec-authoring-core/views/spec-document-view.json`: `"section-sequence"` → `"precedes"`
  - `srs/srs/package/spec-authoring-core/views/rationale-document-view.json`: `"design-note-sequence"` → `"precedes"`
  - `srs/srs/package/spec-authoring-core/views/unified-document-view.json`: `"section-sequence"` → `"precedes"`
  - Do not modify prose descriptions in type or field files — those are human-readable text, not machine values.

**Note on relations.json migration:** `relations-collection.json` requires the flat `Relation` shape (`additionalProperties: false`). The grouped shape is a legacy format. Migration must complete in Phase 1 — the spec repo fails schema validation otherwise.

**Status of installed-but-unused sequence definitions:** The nine sequence type definitions (`com.semanticops.srs/section-sequence`, `com.semanticops.srs/subsection-sequence`, `com.semanticops.srs/design-note-sequence`, `com.semanticops.srs/extension-dependency`, `com.semanticops.srs/explains`, `com.semanticops.spec/rfc-change-sequence`, `com.semanticops.spec/rfc-revision-sequence`, `com.semanticops.spec/rfc-proposed-artifact-sequence`, `com.semanticops.spec/rfc-targets-section`) remain installed with `status: "deprecated"`. The `deprecated` status means: existing stored relations using them would still resolve for reads, but new writes using these type names are rejected. Since the migrated `relations.json` no longer uses them, this is the correct signal — they are preserved for historical context and in case other repositories reference them, but this repository does not create new relations of these types. Phase 1 must set `"status": "deprecated"` in each of these nine definition files. The Phase 4 list test must account for this: all 16 definitions are returned by `relation-type list` regardless of status.

#### Acceptance Criteria

- [ ] `scripts/check-schema-drift.sh` exits 0
- [ ] `cargo test -p srs-schema` passes (schema self-validation tests)
- [ ] `node scripts/validate-all.mjs` passes from `srs/`

#### Testing

```bash
scripts/check-schema-drift.sh
cargo test -p srs-schema
cd ../srs && node scripts/validate-all.mjs
```

#### Milestone gate

```bash
cargo test -p srs-schema
cargo clippy -p srs-schema -- -D warnings
git commit
```

---

### Phase 2: `RelationTypeDefinition` Core Type and Validation

**Goal:** `srs-core` has a typed `RelationTypeDefinition` struct with all RFC-005 fields and a validation function enforcing the RFC invariants.

**Agent:** Core Model Worker

**Write scope:** `crates/srs-core/src/`

#### Files to create/modify

| File | Action |
|---|---|
| `crates/srs-core/src/types/relation_type_definition.rs` | Create |
| `crates/srs-core/src/types/mod.rs` | Add `pub mod relation_type_definition;` |
| `crates/srs-core/src/validation/relation_type_definition.rs` | Create |
| `crates/srs-core/src/validation/mod.rs` | Add `pub mod relation_type_definition;` |
| `crates/srs-core/src/error.rs` | Add `InvalidRelationTypeId`, `EmptyRelationType`, `InvalidRelationTypeStatus` variants |

#### `RelationTypeDefinition` struct shape

```rust
// crates/srs-core/src/types/relation_type_definition.rs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelationTypeDefinition {
    pub id: String,                          // UUID
    pub version: u32,                        // min 1
    pub relation_type: String,               // bare string or namespace/name
    pub namespace: String,
    pub label: String,
    pub description: String,
    pub category: RelationTypeCategory,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inverse_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub irreflexive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_source_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_target_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_same_semantic_object_type: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<RelationTypeStatus>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelationTypeCategory {
    Composition, Refinement, Dependency, Sequence, Derivation,
    Evidence, Governance, Association, Lifecycle, Provenance, Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelationTypeStatus {
    Active, Deprecated, Tombstone, Retired,
}

impl RelationTypeDefinition {
    /// Returns true if this definition resolves for new relation writes.
    /// deprecated resolves but writes are rejected; tombstone and retired do not accept writes.
    pub fn accepts_new_relations(&self) -> bool {
        matches!(self.status, None | Some(RelationTypeStatus::Active))
    }

    /// Returns true if this definition resolves for historical reads (existing stored relations).
    pub fn resolves_for_reads(&self) -> bool {
        !matches!(self.status, Some(RelationTypeStatus::Retired))
    }
}
```

#### Validation function

```rust
// crates/srs-core/src/validation/relation_type_definition.rs
pub fn validate_relation_type_definition(rtd: &RelationTypeDefinition) -> Result<(), CoreError>
```

Checks:
- `id` is non-empty → `Err(CoreError::InvalidRelationTypeId)`
- `version` >= 1 (enforced by u32 in struct + serde min)
- `relation_type` is non-empty → `Err(CoreError::EmptyRelationType)`
- `relation_type` containing `/` must be in `namespace/name` format (exactly one `/`, both parts non-empty)
- `created_at` is non-empty

#### RFC invariants as a separate validation function

```rust
// crates/srs-core/src/validation/relation.rs  (new file)
pub struct RelationValidationContext<'a> {
    pub definitions: &'a [RelationTypeDefinition],
    pub known_instance_ids: &'a std::collections::HashSet<String>,
    /// Maps instanceId → semanticObjectType (if present on the instance). Used for E4.
    /// If an instance is not in this map, E4 type-constraint checks are skipped for that endpoint.
    pub instance_semantic_types: &'a std::collections::HashMap<String, String>,
}

pub fn validate_relation(
    relation: &Relation,
    ctx: &RelationValidationContext,
    is_write: bool,
) -> Result<(), Vec<RelationValidationError>>
```

Implements:
- **E1** — resolves `relationType` against `ctx.definitions`; errors on missing, conflict (same `relationType`, different `id`/`version`/content), or `status: "retired"`; errors when `is_write: true` and `status` is `"deprecated"` or `"tombstone"`
- **E2** — checks both endpoint IDs exist in `ctx.known_instance_ids`
- **E3** — if definition has `irreflexive: true`, rejects `sourceInstanceId == targetInstanceId`
- **E4** — if definition has `allowedSourceTypes`/`allowedTargetTypes` or `requireSameSemanticObjectType: true`, looks up each endpoint in `ctx.instance_semantic_types`; if the endpoint's type is present and violates a constraint, returns an error; if the endpoint's type is absent from the map, the check is skipped (the instance has no `semanticObjectType` — not a validation error)

#### Tests (inline `#[cfg(test)]`)

- `relation_type_definition_roundtrips_json` — full struct serialize → deserialize → assert equal
- `relation_type_definition_minimal_roundtrips` — only required fields present → Ok
- `relation_type_definition_unknown_field_fails_deserialization` — JSON with unknown key fails to deserialize (schema has `additionalProperties: false`; the Rust struct uses `#[serde(deny_unknown_fields)]`)
- `validate_rtd_passes_minimal` — valid minimal definition → Ok
- `validate_rtd_empty_relation_type_fails` — `relation_type: ""` → EmptyRelationType
- `validate_rtd_empty_id_fails` — `id: ""` → InvalidRelationTypeId
- `validate_rtd_invalid_namespaced_type_fails` — `relation_type: "bad/format/extra"` → error
- `accepts_new_relations_active` — status None or Active → true
- `accepts_new_relations_deprecated` — status Deprecated → false
- `resolves_for_reads_tombstone` — status Tombstone → true (reads resolve)
- `resolves_for_reads_retired` — status Retired → false
- `e1_missing_definition_is_error` — relation with unknown relationType → error
- `e1_retired_definition_is_error` — relationType resolves to retired def → error
- `e1_deprecated_write_is_error` — deprecated def, is_write=true → error
- `e1_deprecated_read_is_ok` — deprecated def, is_write=false → ok
- `e1_tombstone_write_is_error` — tombstone def, is_write=true → error
- `e1_tombstone_read_is_ok` — tombstone def, is_write=false → ok
- `e1_conflict_same_relation_type_different_id` — two defs same relationType different id → conflict error
- `e1_coalesce_identical_definitions` — two defs identical id+version+content → resolves as one
- `e2_unknown_source_endpoint_is_error` — sourceInstanceId not in known_ids → error
- `e3_irreflexive_self_relation_is_error` — source == target, irreflexive:true → error
- `e3_irreflexive_false_self_relation_is_ok` — source == target, irreflexive:false → ok
- `e4_allowed_source_type_rejected` — source semanticObjectType not in allowedSourceTypes → error
- `e4_require_same_type_mismatch` — different semanticObjectTypes, requireSame:true → error

#### Milestone gate

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
git commit
```

---

### Phase 3: Package Relation Type Loading and Repository Validation Wiring

**Goal:** `srs-repository` loads relation type definitions from the package and validates relations against them on write and on `validate_repository`.

**Agent:** Repository Service Worker

**Write scope:** `crates/srs-repository/src/`

#### Files to create/modify

| File | Action |
|---|---|
| `crates/srs-repository/src/package.rs` | Add `load_relation_type_definitions`, expose via `Package` struct |
| `crates/srs-repository/src/loader.rs` | Add `load_relation_type_definition(path)` |
| `crates/srs-repository/src/error.rs` | Add `RelationTypeDefinitionLoad`, `RelationTypeDefinitionConflict`, `RelationValidation` variants |
| `crates/srs-repository/src/validation.rs` | Wire E1–E4 into `validate_repository` |
| `crates/srs-repository/src/writer.rs` | Call `validate_relation` (with `is_write: true`) before writing a new relation |

#### `Package` struct changes

Add to the existing `Package` struct in `package.rs`:
```rust
pub relation_type_definitions: Vec<RelationTypeDefinition>,
```

`load_package` must read `package.json`'s `relationTypes[]` paths, load each file via `load_relation_type_definition`, and populate the field.

#### New error variants

```rust
#[error("failed to load relation type definition at {path:?}: {source}")]
RelationTypeDefinitionLoad { path: PathBuf, source: serde_json::Error },

#[error("relation type conflict for '{relation_type}': definitions from {path_a:?} and {path_b:?} differ")]
RelationTypeDefinitionConflict { relation_type: String, path_a: PathBuf, path_b: PathBuf },

#[error("relation validation failed for relation {relation_id}: {message}")]
RelationValidation { relation_id: String, message: String },
```

#### Validation service changes (`validation.rs`)

`validate_repository` already loads instances and runs schema validation. Extend it to:
1. Load package relation type definitions via `load_package`
2. Collect all known instance IDs from the manifest index
3. Build `instance_semantic_types` map: for each loaded instance, if it has a `semanticObjectType` field (Tier 2 records may carry this), add `instanceId → semanticObjectType` to the map. If the field is absent, do not add an entry.
4. For each relation in `relations.json`, call `validate_relation` with `is_write: false` and add any errors to diagnostics

#### Writer changes (`writer.rs`)

Before writing a new relation (when a `relation create` service exists), call `validate_relation(relation, ctx, is_write: true)`. For now, add the hook as a function stub `validate_relation_before_write(relation, repo_root)` that loads the package, collects known IDs and `instance_semantic_types`, constructs the context, runs validation, and returns `Result<(), RepositoryError>`.

#### Tests (inline `#[cfg(test)]`)

- `load_package_includes_relation_type_definitions` — temp repo with a canonical relation type file → `package.relation_type_definitions` is non-empty
- `load_relation_type_definitions_empty_when_none_declared` — package with no `relationTypes[]` key → empty vec, no error
- `relation_type_conflict_detected_on_load` — two definition files with same `relationType` different `id` → `RelationTypeDefinitionConflict` error
- `relation_type_coalesces_identical_definitions` — two files identical content → loads as one definition
- `validate_repository_reports_missing_definition` — relation with unknown `relationType` in a repo with no definitions → diagnostic in report
- `validate_repository_reports_irreflexive_violation` — self-relation for irreflexive type → diagnostic
- `validate_repository_passes_valid_canonical_relation` — relation of type `contains` in repo with canonical definitions loaded → no diagnostics

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
git commit
```

---

### Phase 4: CLI `relation-type` Commands

**Goal:** `srs relation-type list` and `srs relation-type get <id>` are available and delegate entirely to the package loader.

**Agent:** CLI Worker

**Write scope:** `crates/srs-cli/src/`

#### Files to create/modify

| File | Action |
|---|---|
| `crates/srs-cli/src/commands/relation_type.rs` | Create |
| `crates/srs-cli/src/commands/mod.rs` | Add `pub mod relation_type;`, add `RelationType(RelationTypeCommand)` variant to `Commands`, add dispatch arm |

#### Command surface

```
srs relation-type list [--repo <path>]
srs relation-type get <id> [--repo <path>]
```

`list` loads the package via `load_package` and returns all `relation_type_definitions` as a JSON array.

`get` loads the package, finds the definition by `id` field, returns it or `ok: false` with a not-found diagnostic.

Output envelopes:
```json
// list
{ "ok": true, "command": "relation-type list", "version": "...", "relationTypeDefinitions": [...] }

// get (found)
{ "ok": true, "command": "relation-type get", "version": "...", "relationTypeDefinition": {...} }

// get (not found)
{ "ok": false, "command": "relation-type get", "version": "...", "diagnostics": [{"message": "relation type definition not found: <id>"}] }
```

#### Tests (integration, in `crates/srs-cli/tests/integration_tests.rs`)

- `relation_type_list_returns_ok_envelope` — `srs relation-type list` against live `srs/srs/` repo → `ok: true`, `relationTypeDefinitions` array has 16 entries (7 canonical + 5 spec-authoring-core + 4 RFC-process); assert that a definition with `relationType: "contains"` and no `status` field is present (active canonical), and a definition with `relationType: "com.semanticops.srs/section-sequence"` and `status: "deprecated"` is present (validates the two-tier naming design and deprecated status)
- `relation_type_get_finds_contains` — `srs relation-type get 3a1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d` against live repo → `ok: true`, `relationType == "contains"`
- `relation_type_get_not_found` — unknown id → `ok: false`
- `repo_validate_migrated_relations_use_only_canonical_types` — `srs repo validate --repo srs/srs --json` against the live repo after migration → diagnostics contain no E1 errors; assert the loaded relations contain `relationType: "precedes"` and `relationType: "contains"` entries, and no `relationType` value matching any of the nine deprecated sequence type names

#### Milestone gate

```bash
cargo test -p srs-cli
cargo test --test integration_tests
cargo clippy -p srs-cli -- -D warnings
git commit
```

---

### Phase 5: Verification Pass

**Goal:** All tests pass, all RFC-005 invariants are exercised by tests, no regressions.

**Agent:** Verification Agent

#### Tasks

- [ ] Run full workspace test suite
- [ ] Run clippy across all crates with `-D warnings`
- [ ] Confirm `scripts/check-schema-drift.sh` passes
- [ ] Confirm `srs repo validate --repo ../srs/srs --json` reports no new errors
- [ ] Confirm `node scripts/validate-all.mjs` passes from `srs/`
- [ ] Audit that every RFC-005 invariant implemented in this plan (E1–E4, conflict rule, coalescing rule) has at least one test. **Deletion rule (instance deletion → relation status transition) is deferred** — no write service for definition or instance deletion exists yet; it will be covered when the `relation delete` and `record delete` services are added in the CLI command structure plan.

#### Testing

```bash
cargo test
cargo clippy -- -D warnings
scripts/check-schema-drift.sh
cargo run --bin srs -- repo validate --repo ../srs/srs --json
cd ../srs && node scripts/validate-all.mjs
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `scripts/check-schema-drift.sh` passes
- [ ] `srs relation-type list` against the live srs repo returns 16 definitions (7 canonical + 5 spec-authoring-core + 4 RFC-process)
- [ ] `srs repo validate` against a repo with an unknown `relationType` produces a diagnostic
- [ ] `srs repo validate` against a repo with a self-relation on an `irreflexive` type produces a diagnostic
- [ ] `srs repo validate` against a repo using only canonical types and valid relations produces no diagnostics
- [ ] E1 coalescing: two identical canonical definitions in the effective package set produce no conflict
- [ ] E1 conflict: two definitions with same `relationType` but different `id` produce a conflict error
- [ ] CLI integration tests pass
- [ ] `node scripts/validate-all.mjs` passes from `srs/`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- Agents must run the milestone gate (lint + tests + commit) before marking a phase complete.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- The `Relation` struct in `srs-core` uses the flat shape (`relationId`, `relationType`, `sourceInstanceId`, `targetInstanceId`) matching `relations-collection.json`. If a `Relation` struct already exists with compatible fields, Phase 2 extends it; otherwise Phase 2 creates it. No grouped/multi-member shape is needed — Phase 1 migrates `srs/srs/relations/relations.json` to flat.
- Instances in `manifest.json` do not currently carry `semanticObjectType`. E4 validation skips type-constraint checks when `semanticObjectType` is absent from `instance_semantic_types` — this is not an error. No warning is emitted; the check is simply not applicable.
- The existing `load_package` in `srs-repository/src/package.rs` already returns a `Package` struct; Phase 3 extends that struct rather than replacing it.
- The sync script (`scripts/sync-schemas-from-spec.sh`) uses `SRS_SPEC_DIR` defaulting to `../srs`. Phase 1 is run from `srs-rust/`.
- `srs/srs/relations/relations.json` is migrated to flat shape in Phase 1. Phase 3 only needs to handle the flat `relations-collection.json` schema.
