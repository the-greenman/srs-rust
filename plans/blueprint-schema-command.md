# Plan: `blueprint schema` command — nested JSON Schema for a whole multi-record document

> Tracks issue #61. Save location: `plans/blueprint-schema-command.md`.
> Depends on #60 (`type schema`, closed) — reuses `type_schema_service::type_schema`.

## Summary

Add `srs blueprint schema <blueprintId>`, a CLI command that resolves a Blueprint and emits a **single nested draft-07 JSON Schema describing the entire multi-record document** it declares. The schema has a `definitions` section (one entry per member Type projected via the existing `type_schema_service::type_schema`), a `root` property for root types, and one child-collection array property per distinct `relationType` in `blueprint.structure`. This enables a form generator to render a whole document without re-implementing the blueprint→schema mapping. It is a pure projection over already-loaded Blueprint + Type + Field data — no new data model, no write path, no new file I/O primitives.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Lead Integrator (`agents.md#lead-integrator`) |
| Service logic | Repository Service Worker (`agents.md#repository-service-worker`) |
| CLI handler + payload | CLI Worker (`agents.md#cli-worker`) |
| Verification | Verification Agent (`agents.md#verification-agent`) |

See [agents.md](agents.md) for role definitions. No new agent role required — the work falls within existing crate-scoped roles.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service takes a typed input struct, does all logic, returns a typed result struct | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Output is a named payload struct in `payload.rs`; golden JSON Schema committed | accepted |
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Projection logic lives in `srs-repository`, not the CLI | accepted |
| [ADR-014](../docs/adr/014-composite-schema-property-naming.md) | Child-collection property keys in composite schemas are derived from `relationType` converted to lowerCamelCase | proposed |

**New ADR required:** ADR-014 establishes that when a CLI command projects a composite schema with named child-collection properties, the property keys are the `relationType` string converted to lowerCamelCase (e.g., `section-sequence` → `sectionSequence`). This convention applies to any future "schema" command that projects Blueprint-level or multi-type schemas. ADR-014 is written in Phase 3 of this plan.

---

## Contracts

### CLI output contract (ADR-011)

**New command added.** Actions required:
- Add `BlueprintSchemaPayload { schema: serde_json::Value, diagnostics: Vec<String> }` to `crates/srs-cli/src/payload.rs` after `BlueprintStructurePayload` (line ~671) in the blueprint payloads section. The `schema` field uses `#[schemars(with = "serde_json::Value")]`; the `diagnostics` field is a plain `Vec<String>`. Diagnostics are embedded in the payload (matching `BlueprintListPayload`), not at envelope level.
- Add `BlueprintCommand::Schema { id: String }` variant to `BlueprintCommand` in `crates/srs-cli/src/commands/mod.rs`.
- Add `cmd_blueprint_schema` handler in `crates/srs-cli/src/commands/blueprint.rs`; wire into `dispatch`.
- Add `write_schema!("blueprint-schema", BlueprintSchemaPayload);` to the blueprint payloads section in `crates/srs-cli/src/bin/generate-schemas.rs`.
- Run `cargo run --bin generate-schemas`; commit new `crates/srs-cli/schemas/payload/blueprint-schema.json`.
- Add `fn blueprint_schema() { check::<BlueprintSchemaPayload>("blueprint-schema"); }` to `crates/srs-cli/tests/payload_contracts.rs`.

Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

**No.** This plan adds no files under `srs/docs/schema/2.0/`. The emitted draft-07 schema is runtime output, not an SRS entity schema.

---

## Scope

In scope:

- New service module `crates/srs-repository/src/blueprint_schema_service.rs` with:
  - `BlueprintSchemaInput { blueprint_id: String }`
  - `BlueprintSchemaResult { schema: serde_json::Value, diagnostics: Vec<String> }`
  - `pub fn blueprint_schema(store: &dyn RepositoryStore, input: BlueprintSchemaInput) -> Result<BlueprintSchemaResult, RepositoryError>`
- Service algorithm (details in Phase 1):
  1. Bind `let blueprint_id = input.blueprint_id;` at top. Load blueprint via `blueprint_service::get_blueprint_by_id(store, &blueprint_id)`; `NotFound` → `Err(RepositoryError::BlueprintNotFound { blueprint_id })`.
  2. Collect unique TypeRefs to project: `blueprint.root_types` + all `target_type` from `blueprint.structure`. **Do not include `source_type` TypeRefs** — source types that are not also root types or target types would produce unreachable `definitions` entries with no `$ref` pointing to them. Deduplicate by `type_id` (first-encountered version wins; see Assumptions).
  3. For each unique TypeRef, call `type_schema_service::type_schema(store, TypeSchemaInput { type_id, type_version })`. On `Err`, push a plain diagnostic `"type '<typeId>' could not be projected: <error>; omitted from definitions"` (no `[WARN]` prefix — matches the plain-string diagnostic convention in `type_schema_service`). Collect `Ok` results into `definitions: IndexMap<String, serde_json::Value>` (or `BTreeMap` for deterministic order) keyed by `type_id`. Forward each `TypeSchemaResult::diagnostics` into result diagnostics prefixed with `"<typeId>: "`.
  4. Build the `root` property:
     - One root type: `json!({ "$ref": format!("#/definitions/{}", type_id) })`
     - Multiple: `json!({ "oneOf": root_types.iter().map(|tr| json!({ "$ref": format!("#/definitions/{}", tr.type_id) })).collect::<Vec<_>>() })`
  5. Implement `fn relation_type_to_property_key(s: &str) -> String` converting kebab/snake_case to lowerCamelCase (e.g., `section-sequence` → `sectionSequence`, `depends-on` → `dependsOn`, `precedes` → `precedes`, `contains` → `contains`). Algorithm: split on `-` and `_`; first segment lowercased as-is; subsequent segments title-cased. **This function is private to this module.**
  6. Group `blueprint.structure` RelationSpecs by `relation_type`. For each group (iterate in sorted-by-key order for determinism):
     - Collect unique `target_type.type_id` values.
     - Build items: bare `$ref` when exactly one target type; `oneOf` array when multiple.
     - Parse cardinality string from the first spec in the group:
       - `"N..*"` → `minItems: N` (omit when N=0)
       - `"N..M"` → `minItems: N` (omit when N=0), `maxItems: M`
       - `"N"` (any positive integer) → `minItems: N, maxItems: N`
       - `None` or unparseable → no constraints; push plain diagnostic `"cardinality '<value>' on relation '<relationType>' could not be parsed; minItems/maxItems omitted"`
     - Set `"x-srs-ordered-by"` to the raw `relation_type` string.
     - Add camelCase property key to `required[]` if **any** spec in the group has `required: Some(true)`.
  7. Assemble final schema:
     ```json
     {
       "$schema": "http://json-schema.org/draft-07/schema#",
       "type": "object",
       "properties": { "root": …, "<childKey>": … },
       "required": [<camelCase keys of required relation groups>],
       "definitions": { "<typeId>": <per-type sub-schema> }
     }
     ```
- Export: add `pub mod blueprint_schema_service;` to `crates/srs-repository/src/lib.rs`.
- CLI: `BlueprintCommand::Schema { id: String }` + `cmd_blueprint_schema` handler + `BlueprintSchemaPayload` in `payload.rs` + `generate-schemas.rs` entry + `payload_contracts.rs` test + committed golden schema.
- MemoryStore roundtrip test in `blueprint_schema_service.rs`.
- CLI integration test with explicit UUIDs and relation type string.
- New ADR at `docs/adr/014-composite-schema-property-naming.md` (relative to `srs-rust/` repo root).

**Out of scope:**

- `srs-vscode` guide-editor refactor that consumes this command — tracked as a follow-up.
- Support for blueprints referencing types from external/federated packages — only types resolvable in the current package are projected; unresolvable TypeRefs emit a diagnostic and are omitted from `definitions`.
- Hierarchical child nesting (section → sub-section as nested arrays within the section sub-schema) — the schema is a flat document envelope; all relation types produce top-level properties.
- `ext:type-inheritance` flattening — `type_schema_service` already handles inherited fields via `package.effective_fields`; this plan takes whatever it returns.

---

## Phases

### Phase 1: Composition service in `srs-repository`

**Goal:** `blueprint_schema_service::blueprint_schema` returns a correct nested draft-07 schema for any resolvable Blueprint, fully tested against `MemoryStore`, with zero CLI involvement.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Create `crates/srs-repository/src/blueprint_schema_service.rs` with `BlueprintSchemaInput`, `BlueprintSchemaResult`, and `pub fn blueprint_schema`.
- [ ] Bind `let blueprint_id = input.blueprint_id;` at the top of the function; call `blueprint_service::get_blueprint_by_id(store, &blueprint_id)?`. On `GetBlueprintResult::NotFound`, return `Err(RepositoryError::BlueprintNotFound { blueprint_id })`.
- [ ] Collect unique TypeRefs: `blueprint.root_types` + `structure[].target_type`. **Exclude `structure[].source_type`** (source-only types would produce unreachable definitions entries). Deduplicate by `type_id`; first-encountered version is used.
- [ ] For each unique TypeRef, call `type_schema_service::type_schema(store, TypeSchemaInput { type_id: tr.type_id.clone(), type_version: tr.type_version })`. On `Err`, push plain diagnostic `"type '<typeId>' could not be projected: <error>; omitted from definitions"` and continue. Collect sub-schemas in a `BTreeMap<String, Value>` (keyed by type_id; BTreeMap gives deterministic serialization order). Forward each `TypeSchemaResult::diagnostics` into result diagnostics, prefixed with `"<typeId>: "`.
- [ ] Implement `fn relation_type_to_property_key(s: &str) -> String`: split on `-` and `_`; join with first segment lowercased, remaining segments title-cased. Examples: `section-sequence` → `sectionSequence`, `depends-on` → `dependsOn`, `precedes` → `precedes`, `contains` → `contains`.
- [ ] Build `root` property: single TypeRef → `{ "$ref": "#/definitions/<typeId>" }`; multiple → `{ "oneOf": [...] }`.
- [ ] Group `blueprint.structure` RelationSpecs by `relation_type`. For each group (sorted by `relation_type_to_property_key(relation_type)` for deterministic order):
  - Unique target type_ids in this group.
  - Items: one target → bare `{ "$ref": ... }`; multiple → `{ "oneOf": [...] }`.
  - Parse cardinality of first spec: `"N..*"` → minItems (omit when N=0); `"N..M"` → minItems (omit when N=0), maxItems M; `"N"` → minItems N, maxItems N; `None`/other → omit with diagnostic.
  - Set `"x-srs-ordered-by"`: raw `relation_type` string.
  - Collect property key into `required[]` if any spec in group has `required: Some(true)`.
- [ ] Assemble final `serde_json::json!` schema with `$schema`, `type`, `properties`, `required`, `definitions`.
- [ ] Add `pub mod blueprint_schema_service;` to `crates/srs-repository/src/lib.rs`.

#### Acceptance Criteria

- [ ] Blueprint with one root type, two distinct relation types (`section-sequence` and `contains`), and three unique target types produces: `root` as bare `$ref`; properties `sectionSequence` and `contains` as arrays; exactly three entries in `definitions` (root type + two target types, assuming root ≠ any target type).
- [ ] Blueprint with two root types produces `root` as `oneOf`.
- [ ] Cardinality `"1..*"` → `minItems: 1`, no `maxItems`. Cardinality `"0..3"` → `maxItems: 3`, no `minItems` (N=0 is omitted). Cardinality `"1"` → `minItems: 1, maxItems: 1`. Absent/null → no constraints, no diagnostic. Unparseable string → no constraints, diagnostic emitted.
- [ ] `relation_type_to_property_key("section-sequence")` = `"sectionSequence"`. `relation_type_to_property_key("precedes")` = `"precedes"`. `relation_type_to_property_key("depends-on")` = `"dependsOn"`.
- [ ] Single target type in a relation group → bare `$ref` in `items` (not `oneOf`). Multiple targets → `oneOf`.
- [ ] Unresolvable TypeRef → plain diagnostic (no `[WARN]` prefix), that type omitted from definitions; other types still present. Result is still `Ok`.
- [ ] Unknown blueprint_id → `Err(RepositoryError::BlueprintNotFound { .. })`.
- [ ] Each child-array property carries `"x-srs-ordered-by"` equal to the raw relation type string (not camelCase).
- [ ] `required[]` in top-level schema contains the camelCase key for any relation group where any spec has `required: Some(true)`.
- [ ] `source_type` TypeRefs that are not also root types or target types do NOT appear in `definitions`.

#### Testing

```bash
cargo test -p srs-repository blueprint_schema
```

Specific tests to write in `blueprint_schema_service.rs` `#[cfg(test)]` against `MemoryStore`:

- `blueprint_schema_single_root_and_two_relation_types` — one root type (UUID `"00000000-0000-4000-8000-000000000001"`), two RelationSpecs with relation types `"section-sequence"` (target `"00000000-0000-4000-8000-000000000002"`) and `"contains"` (target `"00000000-0000-4000-8000-000000000003"`). Assert: `root` is `$ref`; `sectionSequence` and `contains` are arrays; `definitions` has exactly three entries (the three distinct type IDs).
- `blueprint_schema_multiple_root_types` — two root types → `root` property is `oneOf` with two `$ref`s.
- `blueprint_schema_cardinality_min_max` — four RelationSpecs with cardinalities `"1..*"`, `"0..3"`, `"1"`, and `None`. Assert minItems/maxItems per spec above.
- `blueprint_schema_relation_type_to_camelcase` — unit tests for `relation_type_to_property_key`: `section-sequence`→`sectionSequence`, `precedes`→`precedes`, `contains`→`contains`, `depends-on`→`dependsOn`, `refines`→`refines`.
- `blueprint_schema_single_target_uses_ref_not_oneof` — one target type in a group → bare `$ref` (not `oneOf`) in `items`.
- `blueprint_schema_multiple_targets_uses_oneof` — two target types in same relation group → `oneOf` in `items`.
- `blueprint_schema_unresolvable_type_emits_diagnostic` — one TypeRef that resolves but has no fields → should succeed; OR inject a type_id that truly can't be projected and assert the diagnostic contains the type_id with no `[WARN]` prefix, and the rest of the schema is still returned.
- `blueprint_schema_unknown_blueprint_errors` — unknown id returns `Err(BlueprintNotFound { .. })`.
- `blueprint_schema_required_propagates_from_relation_spec` — RelationSpec with `required: Some(true)` → camelCase property key appears in top-level `required[]`.
- `blueprint_schema_xsrs_ordered_by_is_raw_relation_type` — `x-srs-ordered-by` carries the un-converted relation type string (e.g., `"section-sequence"`, not `"sectionSequence"`).
- `blueprint_schema_source_only_types_not_in_definitions` — a RelationSpec where `source_type.type_id` is different from any root_type or target_type; assert that source_type does NOT appear in `definitions`.
- `blueprint_schema_memory_roundtrip` — build MemoryStore with blueprint + types + fields, project, serialize to string, reparse, spot-check `schema.properties.root`, `schema.definitions`, one child array property.

#### Milestone gate

1. Verify every acceptance criterion above.
2. Confirm all listed tests exist and pass.
3. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Mark checkboxes `[x]` in this file.
5. `git commit` referencing #61.

---

### Phase 2: CLI command + payload contract

**Goal:** `srs blueprint schema <blueprintId>` returns the nested schema in the standard envelope, with a committed golden schema and a passing payload contract test.

**Agent:** CLI Worker

#### Tasks

- [ ] Add `/// Emit a nested draft-07 JSON Schema for a whole multi-record document declared by this Blueprint` doc comment + `Schema { id: String }` variant to `BlueprintCommand` in `crates/srs-cli/src/commands/mod.rs`, after the existing `Structure` variant.
- [ ] Add `BlueprintSchemaPayload` to `crates/srs-cli/src/payload.rs` **after `BlueprintStructurePayload`** in the blueprint payloads section:
  ```rust
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct BlueprintSchemaPayload {
      /// A nested draft-07 JSON Schema for the whole multi-record document.
      #[schemars(with = "serde_json::Value")]
      pub schema: serde_json::Value,
      /// Non-fatal projection diagnostics (unresolvable types, unparseable cardinality, etc.).
      #[serde(skip_serializing_if = "Vec::is_empty")]
      pub diagnostics: Vec<String>,
  }
  ```
- [ ] Add `use srs_repository::blueprint_schema_service::{self, BlueprintSchemaInput};` to the imports in `crates/srs-cli/src/commands/blueprint.rs`.
- [ ] Add `cmd_blueprint_schema(ctx: CliContext, id: String) -> Result<String>` to `blueprint.rs`:
  ```rust
  fn cmd_blueprint_schema(ctx: CliContext, id: String) -> Result<String> {
      match with_store(&ctx, |store| {
          Ok(blueprint_schema_service::blueprint_schema(
              store,
              BlueprintSchemaInput { blueprint_id: id.clone() },
          )?)
      }) {
          Ok(result) => output::serialize(
              "blueprint schema",
              BlueprintSchemaPayload {
                  schema: result.schema,
                  diagnostics: result.diagnostics,
              },
          ),
          Err(e) => {
              if let Some(RepositoryError::BlueprintNotFound { .. }) =
                  e.downcast_ref::<RepositoryError>()
              {
                  return Ok(output::err(
                      "blueprint schema",
                      vec![format!("Blueprint '{id}' not found")],
                  ));
              }
              Err(e)
          }
      }
  }
  ```
- [ ] Add `BlueprintCommand::Schema { id } => cmd_blueprint_schema(ctx, id)` arm to `dispatch` in `blueprint.rs`.
- [ ] Add `write_schema!("blueprint-schema", BlueprintSchemaPayload);` to the blueprint payloads section in `crates/srs-cli/src/bin/generate-schemas.rs`. (Find the section by searching for `write_schema!("blueprint-structure"` and insert after it.)
- [ ] Run `cargo run --bin generate-schemas`; stage and include `crates/srs-cli/schemas/payload/blueprint-schema.json` in the commit.
- [ ] Add `#[test] fn blueprint_schema() { check::<BlueprintSchemaPayload>("blueprint-schema"); }` to `crates/srs-cli/tests/payload_contracts.rs` in the blueprint payloads section.

#### Acceptance Criteria

- [ ] `cargo run --bin srs -- blueprint schema <id> --repo <fixture>` emits `{ "ok": true, "command": "blueprint schema", "payload": { "schema": { … }, "diagnostics": [] } }`.
- [ ] Unknown id → `{ "ok": false, "diagnostics": ["Blueprint '<id>' not found"] }`, exit code 0 (ADR-011).
- [ ] `crates/srs-cli/schemas/payload/blueprint-schema.json` exists and is committed.
- [ ] Handler body stays within ADR-010 shape (no projection logic in CLI).
- [ ] `diagnostics` key is omitted from payload JSON when empty (due to `skip_serializing_if`).

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
```

Specific tests:

- `blueprint_schema_emits_nested_draft07` — CLI integration test against a JSON-backed fixture repo. Create:
  - Field `"00000000-0000-4000-8000-0000000061a1"`, name `"title"`, valueType `"string"`.
  - Root type `"00000000-0000-4000-8000-0000000061b1"` with that field.
  - Section type `"00000000-0000-4000-8000-0000000061b2"` with that field.
  - Blueprint `id` to be captured from `blueprint create` response. The blueprint has `root_types: [{ "typeId": "...61b1" }]` and `structure: [{ "relationType": "section-sequence", "sourceType": { "typeId": "...61b1" }, "targetType": { "typeId": "...61b2" }, "required": true }]`.
  - Run `blueprint schema <id>` and assert: `ok: true`, `command: "blueprint schema"`, `schema.$schema` is draft-07, `schema.properties.root.$ref` ends with `61b1`, `schema.properties.sectionSequence.type` = `"array"`, `schema.properties.sectionSequence.x-srs-ordered-by` = `"section-sequence"`, `schema.definitions` has entries for both type IDs.
- `blueprint_schema_unknown_blueprint_returns_error_envelope` — unknown id → `ok: false`, error message present.
- `payload_contracts` golden test passes for `blueprint-schema.json`.

#### Milestone gate

1. Verify acceptance criteria.
2. Confirm tests exist and pass.
3. Run:

```bash
cargo test
cargo clippy -- -D warnings
cargo test --test payload_contracts
```

4. Mark checkboxes `[x]`.
5. `git commit` referencing #61.

---

### Phase 3: ADR-014

**Goal:** The composite schema property naming convention is documented and committed.

**Agent:** Lead Integrator

#### Tasks

- [ ] Write `docs/adr/014-composite-schema-property-naming.md` (absolute path: `/home/greenman/dev/semanticops/srs-rust/docs/adr/014-composite-schema-property-naming.md`) using the ADR-TEMPLATE, status `proposed`. Set `Date:` to today's date at authoring time. Content: the decision that child-collection property keys in composite/blueprint-level schema projections are the raw `relationType` string converted to lowerCamelCase; the rejected alternative (verbatim hyphenated keys and their JS ergonomics trade-off); consequences.
- [ ] After Phase 2 milestone gate passes, flip ADR-014 status from `proposed` to `accepted` and commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output envelope unchanged for existing commands (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (new `blueprint-schema.json` golden committed; `blueprint_schema()` test function present in `payload_contracts.rs`)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] Blueprint with one root type and two distinct relation types returns a valid nested draft-07 schema
- [ ] `definitions` entries match `type schema` output for each member type (no divergence in field projection)
- [ ] Cardinality maps correctly to `minItems`/`maxItems` (N=0 → omit minItems)
- [ ] `x-srs-ordered-by` carries the raw relation type string on each child array property
- [ ] Source-only types do not appear in `definitions`
- [ ] ADR-014 committed with status `accepted`

## Coordination Rules

- Repository Service Worker writes only `crates/srs-repository/**`; CLI Worker only `crates/srs-cli/**`.
- Phase 2 does not start until Phase 1's milestone gate passes (handler depends on service signature).
- No projection logic in the CLI handler — if `cmd_blueprint_schema` exceeds the handler shape, the excess moves to the service.
- Workers return changed file paths + a one-line behaviour summary.
- Verification Agent runs after Phase 2 and before final sign-off.

## Assumptions

- `type_schema_service::type_schema` is the authoritative per-type projection. Any type that fails to project produces a plain diagnostic (no severity prefix) and is omitted from `definitions`; the overall function still returns `Ok`.
- Deduplication by `type_id` assumes a well-formed blueprint does not reference the same type at two different versions. If it does, the first-encountered version is used and no diagnostic is emitted.
- `Blueprint.required_types` is a subset of the type universe already collected via `root_types` and `structure[].target_type`; it is not collected separately and does not affect the schema.
- A RelationSpec with `required: None` is treated as not required (default false).
- All relation types in `blueprint.structure` become top-level child array properties regardless of whether their source type is a root type. Hierarchical nesting (children of children) is out of scope.
- The `diagnostics` field in `BlueprintSchemaPayload` is `skip_serializing_if = "Vec::is_empty"` so the JSON key is absent when there are no diagnostics, matching the convention on other SRS payload structs.
