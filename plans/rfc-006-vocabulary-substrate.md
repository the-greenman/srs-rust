# Plan: RFC-006 Vocabulary Substrate — Rust Implementation

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

RFC-006 (Vocabulary Substrate, Accepted Rev 8) defined a unified substrate — `VocabularyEntry` — shared by `Term`, `LifecycleState`, and `RelationTypeDefinition`. The spec-records pass (srs PR #9, merged) delivered the authoritative spec text. This plan implements RFC-006 in the Rust stack: new `Term`, `Vocabulary`, and `Lifecycle` core types; field renames across `TagDefinition`→`Term`, `LifecycleState.name`→`key`, `RelationTypeDefinition.relation_type`→`key`; `LifecycleTransition.id`/`properties` addition; `lifecycleRef` on `RecordType`; `vocabularyRef` on `Field`; package loading for `vocabularies[]` and `lifecycles[]`; validation invariants V1–V10; new JSON Schema files; new CLI commands; and schema sync. ADR-003 is superseded by ADR-012.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Core Model Worker | — |
| Repository Service Worker | — |
| CLI Worker | — |
| Verification Agent | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-003](../docs/adr/003-tagdefinition-is-core.md) | TagDefinition is a core native type (tier 3 instance) | **Superseded by ADR-012** |
| ADR-012 (create at `docs/adr/012-vocabulary-substrate.md`) | `Term`/`Vocabulary`/`Lifecycle` are package-level definitions, not instance-index entries. `TagDefinition` (tier 3) is retired; tags now resolve via `Term` entries in a local open `Vocabulary` in the package. `tier: 3` deprecated. `RelationTypeDefinition.relation_type` → `key`; `LifecycleState.name` → `key`. One forward-compat policy: `properties` bag, unknown top-level fields rejected. | proposed |

---

## Contracts

### CLI output contract (ADR-011)

The following CLI payload structs in `crates/srs-cli/src/payload.rs` change:

- `TagListPayload` — replace `tag_definitions: Vec<TagDefinition>` with `terms: Vec<Term>` (field `tag_key` → `key`)
- `TagPayload` — replace `TagDefinition` with `Term`
- `RelationTypeGetPayload` / `RelationTypeListPayload` — embedded struct reflects `key` rename (verify serde rename covers this transparently)
- **New payloads:** `VocabularyListPayload`, `VocabularyGetPayload`, `VocabularyCreatePayload`, `VocabularyUpdatePayload`, `LifecycleListPayload`, `LifecycleGetPayload`

After any payload struct change: `cargo run --bin generate-schemas` → commit updated `crates/srs-cli/schemas/payload/<name>.json`.

**Sequencing for TagListPayload:** update the struct first → run `cargo run --bin generate-schemas` → commit the updated golden files → then run `cargo test --test payload_contracts`. Running in the wrong order will fail the contract test against stale golden files.

Verification: `cargo test --test payload_contracts` must pass after all payload changes.

### Entity schema sync

This plan adds and modifies JSON Schema files. Canonical source: `crates/srs-schema/schemas/2.0/` (confirmed exists). After each schema change, sync to `../../srs-vscode/schemas/2.0/` and verify `bash scripts/check-schema-sync.sh` exits 0.

**Note:** The srs-schema crate is the source; the srs-vscode extension is a downstream mirror. The srs/ repo `docs/schema/2.0/` is also a mirror that should be kept in sync.

**New schema files (create in `crates/srs-schema/schemas/2.0/`):**
- `term.json`
- `vocabulary.json`
- `lifecycle.json`

**Modified schema files:**
- `field.json` — add `vocabularyRef` (optional string, mutually exclusive with `allowedValues`)
- `type.json` — rename `lifecycle.states[].name` → `key`; add `lifecycle.transitions[].id`/`properties`; add top-level `lifecycleRef` (optional string)
- `relation-type.json` — rename `relationType` → `key`; add optional `properties` object; remove `additionalProperties: false`
- `package-manifest.json` — add optional `vocabularies` and `lifecycles` string arrays

---

## Scope

**In scope:**
- New `srs-core` types: `Term`, `Vocabulary`, `Lifecycle` (as installable referenceable container)
- `LifecycleState`/`LifecycleTransition` moved from `record_type.rs` to `lifecycle.rs` (re-exported from `record_type.rs` for backward compat)
- `TypeLifecycle` stays in `record_type.rs` — it is the inline block on `RecordType`; `Lifecycle` is the *separate* installable container; they both use `LifecycleState`/`LifecycleTransition` from `lifecycle.rs`
- Field renames: `TagDefinition.tag_key` → `Term.key`; `LifecycleState.name` → `key` (serde alias "name" for compat); `RelationTypeDefinition.relation_type` → `key` (serde alias "relationType" for compat)
- `LifecycleTransition.name` is the transition's **display label** — it is NOT renamed; only `LifecycleState.name` → `key` (the state's machine key)
- `LifecycleTransition`: add `id: Option<String>` (UUID) and `properties: Option<HashMap<String, Value>>`
- `LifecycleState`: add substrate header fields: `id`, `version`, `namespace`, `aliases`, `status`, `properties`
- `RecordType`: add `lifecycle_ref: Option<String>` (mutually exclusive with inline `lifecycle`); V7 validation
- `Field`: add `vocabulary_ref: Option<String>` (mutually exclusive with `select_options`/`allowed_values`); V3 validation
- `Package` struct + `PackageMetadata`: add `vocabularies: Vec<Vocabulary>` and `lifecycles: Vec<Lifecycle>`; loading logic for path arrays in `package.json`
- Validation: V1 (closed-vocab resolution via `resolves_for_reads()` on all entries), V2 (open-vocab with deterministic tie-break: key>alias, then lowest `id`), V3 (field exclusivity/closedness), V4 (vocabularyRef resolution), V5 (effective entry set; retired excluded before uniqueness; `extends*Version` enforced hard), V7 (lifecycle exclusivity), V8 (lifecycleRef resolution), V9 (single active initial state; isFinal no outgoing transitions; transition ids unique), V10 (promotion window pre-flight)
- CLI: `srs tag list/get` returns `Vec<Term>` with `key` field; `srs tag create/update/delete` returns a clear descriptive error; new `srs vocabulary list/get` command group; new `srs lifecycle list/get` command group
- New JSON Schema files in `crates/srs-schema/schemas/2.0/` + sync to srs-vscode
- ADR-012: supersede ADR-003
- `tag_service.rs` write functions (`create/update/delete_tag_definition*`) are **kept compiling** but marked `#[deprecated]` — not removed (clean removal is a follow-on)
- `Vocabulary::effective_terms()` implements RFC-006 V2 tie-break (key match beats alias match; ties broken by lexicographically smallest `id`)
- `Term.properties["mergedFrom"]` — intentionally deferred (alias-merge is a follow-on CLI command); the field exists in the schema but no merge logic is implemented here

**Out of scope:**
- Alias-merge / normalize CLI operations (future)
- `ext:federation` cross-repo vocabulary linking
- Ontology layer (term-to-term relations, id-as-value)
- VS Code extension UI changes beyond schema sync
- Updating existing `tier: 3` tag-definition JSON files in `srs/srs/` (no applied uses per RFC-006)
- Full removal of `tag_service` write paths (marked deprecated; removal is follow-on)

---

## Phases

### Phase 1: Core types — Term, Vocabulary, Lifecycle substrate

**Goal:** `srs-core` contains new types, all field renames applied, all existing tests pass.

**Agent:** Core Model Worker

#### Tasks

- [ ] Create `crates/srs-core/src/types/term.rs`:
  - `VocabularyEntryStatus` enum: `Active`, `Deprecated`, `Tombstone`, `Retired` — `#[serde(rename_all = "kebab-case")]`
  - `impl Default for VocabularyEntryStatus` → `Active`
  - `VocabularyEntryStatus::resolves_for_reads(&self) -> bool` → true if Active/Deprecated/Tombstone
  - `VocabularyEntryStatus::accepts_new_writes(&self) -> bool` → true if Active only
  - `VocabularyEntryStatus::is_retired(&self) -> bool`
  - `Term` struct: `id: String`, `version: u32`, `namespace: String`, `key: String`, `label: Option<String>`, `description: Option<String>`, `aliases: Option<Vec<String>>`, `roles: Option<Vec<String>>`, `status: Option<VocabularyEntryStatus>`, `properties: Option<HashMap<String, serde_json::Value>>`, `created_at: Option<String>`, `updated_at: Option<String>` — `#[serde(rename_all = "camelCase", deny_unknown_fields)]`

- [ ] Create `crates/srs-core/src/types/vocabulary.rs`:
  - `VocabularyMode` enum: `Open`, `Closed` — `#[serde(rename_all = "kebab-case")]`
  - `PromotionWindow` struct: `until: String` — `#[serde(rename_all = "camelCase", deny_unknown_fields)]`
  - `Vocabulary` struct: `id: String`, `version: u32`, `namespace: String`, `name: String`, `mode: VocabularyMode`, `terms: Vec<Term>`, `extends_vocabulary_id: Option<String>`, `extends_vocabulary_version: Option<u32>`, `promotion_window: Option<PromotionWindow>`, `description: Option<String>`, `created_at: String` — `#[serde(rename_all = "camelCase", deny_unknown_fields)]`
  - `Vocabulary::effective_terms(&self) -> Vec<&Term>` → returns terms where `status.unwrap_or(Active) != Retired`
  - `Vocabulary::resolve_term_by_key(&self, key: &str) -> Option<&Term>` → checks `term.key == key` first, then checks any alias; skips retired terms; implements V2 tie-break: key match takes priority over alias match; among alias matches, lowest `term.id` (lexicographic) wins

- [ ] Create `crates/srs-core/src/types/lifecycle.rs`:
  - Move `LifecycleState`, `LifecycleTransition` from `record_type.rs` to this file
  - In `LifecycleState`: rename `name: String` → `key: String` with `#[serde(alias = "name")]` for backward compat; add `id: Option<String>`, `version: Option<u32>`, `namespace: Option<String>`, `aliases: Option<Vec<String>>`, `status: Option<VocabularyEntryStatus>`, `properties: Option<HashMap<String, serde_json::Value>>`
  - In `LifecycleTransition`: `name: String` stays as-is (it is the transition's display label, NOT the key); add `id: Option<String>`, `properties: Option<HashMap<String, serde_json::Value>>`
  - `Lifecycle` struct: `id: String`, `version: u32`, `namespace: String`, `name: String`, `states: Vec<LifecycleState>`, `transitions: Vec<LifecycleTransition>`, `initial_state: String`, `extends_lifecycle_id: Option<String>`, `extends_lifecycle_version: Option<u32>`, `description: Option<String>`, `created_at: String` — `#[serde(rename_all = "camelCase", deny_unknown_fields)]`

- [ ] Edit `crates/srs-core/src/types/record_type.rs`:
  - Remove `LifecycleState` and `LifecycleTransition` definitions; add `pub use crate::types::lifecycle::{LifecycleState, LifecycleTransition};`
  - `TypeLifecycle` stays in `record_type.rs` (it is the inline lifecycle block on a `RecordType`; it uses `LifecycleState`/`LifecycleTransition` from `lifecycle.rs`)
  - Add `lifecycle_ref: Option<String>` to `RecordType` — `#[serde(skip_serializing_if = "Option::is_none")]`

- [ ] Edit `crates/srs-core/src/types/relation_type_definition.rs`:
  - Rename `relation_type: String` → `key: String` with `#[serde(rename = "relationType", alias = "key")]` (serialize as `relationType` for now to preserve existing JSON; accept both on deserialization)
  - Add `properties: Option<HashMap<String, serde_json::Value>>` — `#[serde(skip_serializing_if = "Option::is_none")]`
  - Remove `deny_unknown_fields` (substrate policy: unknown fields rejected → use `properties` instead; `deny_unknown_fields` was the old policy)

- [ ] Edit `crates/srs-core/src/types/field.rs` (confirm location first):
  - Add `vocabulary_ref: Option<String>` — `#[serde(skip_serializing_if = "Option::is_none")]`

- [ ] Edit `crates/srs-core/src/types/tag_definition.rs`:
  - Add `#[serde(alias = "key")]` to `tag_key` field so the struct can deserialize both `"tagKey"` and `"key"` (forward compat during transition)
  - No other changes — `TagDefinition` is retained as a transitional shim

- [ ] Export new modules from `crates/srs-core/src/types/mod.rs`:
  - `pub mod term;`, `pub mod vocabulary;`, `pub mod lifecycle;`

#### Acceptance Criteria

- [ ] `cargo build -p srs-core` succeeds
- [ ] `cargo test -p srs-core` passes (all existing tests green)
- [ ] `LifecycleState` deserializes JSON with `"name"` and with `"key"` (alias works)
- [ ] `RelationTypeDefinition` deserializes JSON with `"relationType"` and with `"key"` (alias works)
- [ ] `RelationTypeDefinition` serializes with `"relationType"` (existing format preserved)
- [ ] `Term` roundtrips through serde
- [ ] `Vocabulary::effective_terms()` excludes `status: retired` entries
- [ ] `Vocabulary::resolve_term_by_key` returns None for retired entries; key match beats alias match
- [ ] `LifecycleTransition.name` is unchanged (not renamed to `key`)

#### Testing

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

Specific tests to write:
- `term_roundtrips_json` — all optional fields present and absent
- `vocabulary_mode_serde` — `"open"` and `"closed"` round-trip
- `lifecycle_state_accepts_name_alias` — JSON with `"name"` deserializes to `key`
- `lifecycle_state_accepts_key` — JSON with `"key"` deserializes correctly
- `relation_type_accepts_relation_type_alias` — JSON with `"relationType"` works
- `relation_type_serializes_as_relation_type` — serialize produces `"relationType"` not `"key"`
- `vocabulary_effective_terms_excludes_retired`
- `vocabulary_resolve_by_alias_secondary_to_key` — two terms, one with key="foo", one with alias="foo"; key match wins
- `lifecycle_transition_name_unchanged` — `LifecycleTransition.name` still called `name`

#### Milestone gate

1. All acceptance criteria checked.
2. All listed tests exist and pass.
3. `cargo test -p srs-core && cargo clippy -p srs-core -- -D warnings`
4. Update plan checkboxes to `[x]`.
5. Commit: `feat(core): RFC-006 vocabulary substrate types and field renames`

---

### Phase 2: Package loading — Vocabulary and Lifecycle

**Goal:** `Package` loads `vocabularies[]` and `lifecycles[]` from `package.json`, resolves Terms by key/alias, and resolves Lifecycles.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Edit `crates/srs-repository/src/package.rs`:
  - Add `vocabularies: Vec<Vocabulary>` and `lifecycles: Vec<Lifecycle>` to `Package` struct
  - Add `vocabularies: Vec<String>` and `lifecycles: Vec<String>` (`#[serde(default)]`) to `PackageMetadata`
  - Add loading loop for vocabularies: for each path in `metadata.vocabularies`, read file, deserialize into `Vocabulary`, push to `Package.vocabularies`
  - Add loading loop for lifecycles: same pattern for `metadata.lifecycles` → `Package.lifecycles`
  - Add `Package::resolve_vocabulary(&self, id: &str) -> Option<&Vocabulary>`
  - Add `Package::resolve_lifecycle(&self, id: &str) -> Option<&Lifecycle>`
  - Add `Package::resolve_lifecycle_by_name(&self, namespace: &str, name: &str) -> Option<&Lifecycle>`
  - Add `Package::resolve_term_by_key(&self, vocabulary_id: &str, key: &str) -> Option<&Term>` — delegates to `Vocabulary::resolve_term_by_key`

- [ ] Create `crates/srs-repository/src/vocabulary_service.rs`:
  - `list_vocabularies(store: &impl Store) -> Result<Vec<Vocabulary>, RepositoryError>` — loads package, returns `package.vocabularies.clone()`
  - `get_vocabulary_by_id(store: &impl Store, id: &str) -> Result<Option<Vocabulary>, RepositoryError>`

- [ ] Create `crates/srs-repository/src/lifecycle_service.rs`:
  - `list_lifecycles(store: &impl Store) -> Result<Vec<Lifecycle>, RepositoryError>`
  - `get_lifecycle_by_id(store: &impl Store, id: &str) -> Result<Option<Lifecycle>, RepositoryError>`

- [ ] Edit `crates/srs-repository/src/tag_service.rs`:
  - Add `list_terms(store: &impl Store) -> Result<Vec<Term>, RepositoryError>` — collects all `Term`s from all vocabularies in the package
  - Add `get_term_by_id(store: &impl Store, id: &str) -> Result<Option<Term>, RepositoryError>` — searches all vocabularies
  - Mark existing `create_tag_definition`, `update_tag_definition`, `delete_tag_definition`, `create_tag_definition_in_context`, `update_tag_definition_validated`, `delete_tag_definition_in_context` with `#[deprecated(note = "Tag terms are now package definitions. Edit the vocabulary JSON file directly.")]`; leave them compiling

#### Acceptance Criteria

- [ ] `Package` loads successfully when `vocabularies` and `lifecycles` are absent in `package.json` (backward compat)
- [ ] `Package` loads a vocabulary from a path in `vocabularies[]`
- [ ] `Package::resolve_term_by_key` finds a term by primary key
- [ ] `Package::resolve_term_by_key` finds a term by alias
- [ ] `Package::resolve_term_by_key` returns None for retired terms
- [ ] `list_terms` returns empty vec when no vocabularies are loaded

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `package_loads_vocabularies_from_paths` — fixture package.json with `"vocabularies": ["vocabularies/tags.json"]`; confirm `Package.vocabularies` has one entry
- `package_loads_empty_vocabularies_when_absent` — package.json without `vocabularies` still loads
- `resolve_term_by_key_matches_primary_key`
- `resolve_term_by_alias`
- `resolve_term_excludes_retired`
- `list_terms_empty_when_no_vocabularies`

#### Milestone gate

1. All acceptance criteria checked.
2. All listed tests exist and pass.
3. `cargo test -p srs-repository && cargo clippy -p srs-repository -- -D warnings`
4. Update plan checkboxes.
5. Commit: `feat(repository): load vocabularies and lifecycles from package`

---

### Phase 3: Validation — V1, V3, V5, V7, V9

**Goal:** RFC-006 normative invariants are enforced in `srs-core` validation and surfaced as diagnostics.

**Agent:** Core Model Worker

**Note on V1:** V1 (closed-vocab resolution) is already *partially* covered by the existing `resolves_for_reads()` / `accepts_new_relations()` methods on `RelationTypeDefinition`. This phase adds V1 enforcement for the *new* vocabulary types and wires the resolution path; the relation-type path is confirmed working and tested already.

#### Tasks

- [ ] Create `crates/srs-core/src/validation/vocabulary.rs`:
  - `validate_vocabulary(vocab: &Vocabulary) -> Vec<ValidationDiagnostic>`:
    - V5: collect all `key`s and all `aliases` in effective terms (non-retired); flag any key/key, key/alias, alias/alias collision
    - Closed vocab: collisions are errors
    - Open vocab: collisions are warnings (not errors)
    - Require `extends_vocabulary_version` is present when `extends_vocabulary_id` is present (if either is missing while the other is set: error)

- [ ] Create `crates/srs-core/src/validation/lifecycle.rs`:
  - `validate_lifecycle(lc: &Lifecycle) -> Vec<ValidationDiagnostic>`:
    - V9a: count states with `is_initial: true` in effective state set; must be exactly 1 (0 or 2+ is error)
    - V9b: the initial state's effective status must be Active (absent = Active); Deprecated/Tombstone/Retired is error
    - V9c: every `transition.from` and `transition.to` references a state `key` in the effective state set; unknown key is error
    - V9d: no state with `is_final: true` appears as `transition.from`; violation is error
    - V9e: transition `id` values must be unique within the lifecycle; duplicate is error
    - V5: no duplicate state `id`s (when set)
    - Require `extends_lifecycle_version` when `extends_lifecycle_id` is present (same rule as vocabularies)

- [ ] Edit validation for `RecordType` (locate the appropriate file — likely `crates/srs-core/src/validation/record_type.rs`):
  - V7: if both `lifecycle` and `lifecycle_ref` are `Some`, emit an error diagnostic

- [ ] Edit validation for `Field` (locate file — likely `crates/srs-core/src/validation/field.rs` or inline):
  - V3: if `value_type` is `select` or `multiselect` AND both `allowed_values`/`select_options` and `vocabulary_ref` are `Some`: error
  - V3: if `value_type` is `select` or `multiselect` AND neither is set: error

- [ ] Export new modules from `crates/srs-core/src/validation/mod.rs`

#### Acceptance Criteria

- [ ] Closed vocab with two terms sharing a key → error
- [ ] Open vocab with two terms sharing a key → warning (not error)
- [ ] Lifecycle with zero `isInitial` states → error
- [ ] Lifecycle with two `isInitial` states → error
- [ ] Lifecycle with initial state `status: deprecated` → error
- [ ] Lifecycle with `isFinal` state as transition `from` → error
- [ ] RecordType with both `lifecycle` and `lifecycleRef` → error diagnostic
- [ ] Field `valueType: select` with both `allowedValues` and `vocabularyRef` → error
- [ ] Field `valueType: select` with neither → error

#### Testing

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

Specific tests:
- `closed_vocab_duplicate_key_is_error`
- `open_vocab_duplicate_key_is_warning`
- `lifecycle_zero_initial_is_error`
- `lifecycle_two_initial_is_error`
- `lifecycle_deprecated_initial_is_error`
- `lifecycle_final_state_as_source_is_error`
- `record_type_both_lifecycle_and_ref_is_error`
- `select_field_both_bindings_is_error`
- `select_field_no_binding_is_error`

#### Milestone gate

1. All acceptance criteria checked.
2. All listed tests exist and pass.
3. `cargo test -p srs-core && cargo clippy -p srs-core -- -D warnings`
4. Update plan checkboxes.
5. Commit: `feat(core): RFC-006 validation V1 V3 V5 V7 V9`

---

### Phase 4: JSON Schema files

**Goal:** New schema files exist in `crates/srs-schema/schemas/2.0/`; modified schemas updated; all schema directories in sync.

**Agent:** Core Model Worker (authoring) + Lead Integrator (sync)

#### Tasks

- [ ] Create `crates/srs-schema/schemas/2.0/term.json`:
  - Required: `id` (UUID), `version` (integer ≥1), `namespace` (string), `key` (string)
  - Optional: `label`, `description`, `aliases` (string[]), `roles` (string[]), `status` (enum: active/deprecated/tombstone/retired), `properties` (object with `additionalProperties: true`), `createdAt`, `updatedAt`
  - `additionalProperties: false` at top level (properties bag handles extensibility)

- [ ] Create `crates/srs-schema/schemas/2.0/vocabulary.json`:
  - Required: `id`, `version`, `namespace`, `name`, `mode` (enum: open/closed), `terms` (array of Term inline schema), `createdAt`
  - Optional: `extendsVocabularyId` (string), `extendsVocabularyVersion` (integer), `promotionWindow` (object with required `until: string`), `description`
  - `additionalProperties: false`

- [ ] Create `crates/srs-schema/schemas/2.0/lifecycle.json`:
  - Required: `id`, `version`, `namespace`, `name`, `states`, `transitions`, `initialState`, `createdAt`
  - `LifecycleState` inline: required `key`; optional `id`, `version`, `namespace`, `label`, `description`, `isInitial`, `isFinal`, `aliases`, `status`, `properties`
  - `LifecycleTransition` inline: required `name`, `from`, `to`; optional `id`, `description`, `properties`
  - Optional: `extendsLifecycleId`, `extendsLifecycleVersion`, `description`
  - `additionalProperties: false`

- [ ] Edit `crates/srs-schema/schemas/2.0/field.json`:
  - Add `vocabularyRef` (optional string) to properties

- [ ] Edit `crates/srs-schema/schemas/2.0/type.json`:
  - In `$defs` or inline lifecycle block: rename `LifecycleState.name` → `key`
  - In `LifecycleTransition`: add optional `id` (string), `properties` (object)
  - Add top-level `lifecycleRef` (optional string)

- [ ] Edit `crates/srs-schema/schemas/2.0/relation-type.json`:
  - Rename `relationType` required property → `key`
  - Add optional `properties` (object with `additionalProperties: true`)
  - Remove `additionalProperties: false`

- [ ] Edit `crates/srs-schema/schemas/2.0/package-manifest.json`:
  - Add optional `vocabularies` (string[]) and `lifecycles` (string[]) properties

- [ ] Sync to srs-vscode:
  ```bash
  # From srs-rust/ root:
  for f in term.json vocabulary.json lifecycle.json field.json type.json relation-type.json package-manifest.json; do
    cp crates/srs-schema/schemas/2.0/$f ../../srs-vscode/schemas/2.0/$f
  done
  ```
- [ ] Sync to srs/ docs:
  ```bash
  for f in term.json vocabulary.json lifecycle.json field.json type.json relation-type.json package-manifest.json; do
    cp crates/srs-schema/schemas/2.0/$f ../srs/docs/schema/2.0/$f
  done
  # Then re-run node scripts/publish-spec.mjs to confirm no drift
  ```

#### Acceptance Criteria

- [ ] `crates/srs-schema/schemas/2.0/term.json` exists and is valid JSON
- [ ] `crates/srs-schema/schemas/2.0/vocabulary.json` exists and is valid JSON
- [ ] `crates/srs-schema/schemas/2.0/lifecycle.json` exists and is valid JSON
- [ ] `field.json` contains `vocabularyRef` property
- [ ] `type.json` has `lifecycleRef` property; LifecycleState uses `key` not `name`
- [ ] `relation-type.json` uses `key` as primary field
- [ ] `package-manifest.json` has `vocabularies` and `lifecycles` arrays
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (run from srs-rust/ or srs/)
- [ ] `node scripts/validate-all.mjs` exits 0 (run from srs/)

#### Testing

```bash
bash scripts/check-schema-sync.sh    # from srs/ or confirm script location
node scripts/validate-all.mjs        # from srs/
```

#### Milestone gate

1. All acceptance criteria checked.
2. Schema sync check exits 0.
3. `node scripts/validate-all.mjs` exits 0.
4. Update plan checkboxes.
5. Commit across srs/, srs-rust/, srs-vscode/ together: `feat(schema): RFC-006 vocabulary/term/lifecycle schemas; update field/type/relation-type/package-manifest`

---

### Phase 5: CLI commands and payload structs

**Goal:** `srs vocabulary list/get`, `srs lifecycle list/get`, updated `srs tag list/get`, payload schemas regenerated.

**Agent:** CLI Worker

#### Tasks

- [ ] Edit `crates/srs-cli/src/payload.rs`:
  - Add `VocabularyListPayload { vocabularies: Vec<Vocabulary> }`
  - Add `VocabularyGetPayload` enum: `Found(Box<Vocabulary>)` / `NotFound { id: String }` (mirror `TagPayload` pattern)
  - Add `LifecycleListPayload { lifecycles: Vec<Lifecycle> }`
  - Add `LifecycleGetPayload` enum: `Found(Box<Lifecycle>)` / `NotFound { id: String }`
  - Update `TagListPayload`: change `tag_definitions: Vec<TagDefinition>` → `terms: Vec<Term>`
  - Update `TagPayload`: change embedded `TagDefinition` → `Term`

- [ ] Create `crates/srs-cli/src/commands/vocabulary.rs`:
  ```rust
  pub fn dispatch(ctx: CliContext, cmd: VocabularyCommand) -> Result<String>

  fn cmd_vocabulary_list(ctx: CliContext) -> Result<String>
      // with_store(&ctx, |store| vocabulary_service::list_vocabularies(store))
      // output::ok("vocabulary list", VocabularyListPayload { vocabularies })

  fn cmd_vocabulary_get(ctx: CliContext, id: String) -> Result<String>
      // vocabulary_service::get_vocabulary_by_id(store, &id)
      // VocabularyGetPayload::Found or NotFound
  ```

- [ ] Create `crates/srs-cli/src/commands/lifecycle.rs`:
  ```rust
  fn cmd_lifecycle_list(ctx: CliContext) -> Result<String>
  fn cmd_lifecycle_get(ctx: CliContext, id: String) -> Result<String>
  ```

- [ ] Edit `crates/srs-cli/src/commands/tag.rs`:
  - `cmd_tag_list`: call `vocabulary_service::list_terms` (or `tag_service::list_terms`); return `TagListPayload { terms }` using `Term`
  - `cmd_tag_get`: call `tag_service::get_term_by_id`; return `TagPayload::Found(term)` or `NotFound`
  - `cmd_tag_create` / `cmd_tag_update` / `cmd_tag_delete`: return `Err(RepositoryError::InvalidOperation("Tag terms are now package definitions; edit the vocabulary JSON file directly.".to_string()))` or equivalent

- [ ] Wire `vocabulary` and `lifecycle` into the CLI dispatcher in `crates/srs-cli/src/main.rs` (or wherever subcommands are registered — follow existing `relation-type` pattern)

- [ ] **Regenerate payload schemas** (AFTER struct changes, BEFORE running payload_contracts test):
  ```bash
  cd srs-rust && cargo run --bin generate-schemas
  ```
  Commit all updated `crates/srs-cli/schemas/payload/*.json` files.

#### Acceptance Criteria

- [ ] `srs vocabulary list --repo <path>` returns `{"ok": true, "command": "vocabulary list", "payload": {"vocabularies": [...]}}`
- [ ] `srs lifecycle list --repo <path>` returns well-formed JSON
- [ ] `srs tag list --repo <path>` returns JSON with `"terms"` array; each entry has `"key"` not `"tagKey"`
- [ ] `srs tag create` returns an error with a clear descriptive message
- [ ] `cargo test --test payload_contracts` passes

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
# Smoke tests:
cargo run --bin srs -- vocabulary list --repo ../srs/srs --pretty
cargo run --bin srs -- lifecycle list --repo ../srs/srs --pretty
cargo run --bin srs -- tag list --repo ../srs/srs --pretty
```

Specific tests:
- `vocabulary_list_empty_when_no_vocabularies` — fixture with empty package returns `{"vocabularies": []}`
- `vocabulary_list_returns_installed_vocabularies` — fixture with a vocabulary returns it
- `tag_list_returns_terms_with_key_field` — response payload has `terms[].key` not `tagKey`

#### Milestone gate

1. All acceptance criteria checked.
2. Confirm tests exist and pass; `payload_contracts` passes.
3. `cargo test && cargo clippy -- -D warnings`
4. Update plan checkboxes.
5. Commit: `feat(cli): vocabulary and lifecycle commands; tag returns Term with key field`

---

### Phase 6: ADR and tier-3 cleanup

**Goal:** ADR-003 superseded, ADR-012 written, `tier: 3` / `is_tag_definition()` deprecated cleanly.

**Agent:** Lead Integrator

#### Tasks

- [ ] Create `crates/srs-rust/docs/adr/012-vocabulary-substrate.md` using ADR-TEMPLATE.md format:
  - Status: accepted
  - Supersedes: ADR-003
  - Context: RFC-006 replaces the ADR-003 TagDefinition-as-core-instance model
  - Decision: Term/Vocabulary/Lifecycle are package-level definitions; tags resolve via open Vocabulary; `tier: 3` deprecated
  - Consequences: `is_tag_definition()` deprecated; existing `tier: 3` entries in manifests silently ignored by package loading; write commands now return descriptive errors directing to vocabulary files

- [ ] Edit `docs/adr/003-tagdefinition-is-core.md`: add `Superseded by: ADR-012` to header block

- [ ] Edit `crates/srs-repository/src/index.rs`:
  - Mark `is_tag_definition()` with `#[deprecated(note = "Use Term resolution via Package::resolve_term_by_key instead. tier: 3 is retired per ADR-012.")]`
  - Do not remove the method (callers may exist; removal is a follow-on)
  - Confirm any callers that filter by `tier == 3` are in `tag_service.rs` only and are already deprecated there

#### Acceptance Criteria

- [ ] `docs/adr/012-vocabulary-substrate.md` exists with proper ADR structure
- [ ] `docs/adr/003-tagdefinition-is-core.md` has `Superseded by: ADR-012`
- [ ] `cargo build` emits no new *errors* (deprecation warnings on `is_tag_definition()` are acceptable)

#### Testing

```bash
cargo test
cargo clippy -- -D warnings
```

#### Milestone gate

1. All acceptance criteria checked.
2. `cargo test && cargo clippy -- -D warnings` (treat deprecation warnings as expected)
3. Update plan checkboxes.
4. Commit: `docs(adr): add ADR-012 vocabulary substrate; supersede ADR-003`

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures (all crates)
- [ ] `cargo clippy -- -D warnings` passes (expected deprecation warnings on `is_tag_definition` are acceptable — suppress with `allow(deprecated)` at call sites if needed)
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] `node scripts/validate-all.mjs` exits 0 (from `srs/`)
- [ ] `cargo run --bin srs -- repo validate --repo ../srs/srs --pretty` reports 0 errors
- [ ] `srs vocabulary list`, `srs lifecycle list`, `srs tag list` return valid JSON envelopes
- [ ] `srs tag list` response contains `"terms"` with `"key"` field (not `"tagKey"`)
- [ ] ADR-012 exists; ADR-003 marked superseded
- [ ] All phase milestone gates completed and committed

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after Phase 5 and before Final Acceptance.
- Phase 4 schema sync is a multi-repo commit — coordinate with Lead Integrator before committing.

## Assumptions

- Serde alias approach (`#[serde(alias = "name")]` on `LifecycleState.key`) is sufficient for backward compat; no data migration script needed since `TagDefinition` has no applied uses (confirmed in RFC-006 Rev 8).
- `srs/srs/` has no `tier: 3` tag-definition instance files that would break loading.
- The `srs-vscode` extension needs only the schema sync — no Rust or CLI changes.
- `tag_service` write functions stay compiling (deprecated, not removed); final removal is a follow-on.
- `Term.properties["mergedFrom"]` for alias-merge is intentionally deferred — no merge logic implemented in this plan.
- `Vocabulary::resolve_term_by_key` implements the V2 tie-break inline in `srs-core` (not just in validation) because it is a runtime resolution operation, not just a validation check.
