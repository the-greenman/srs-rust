# Plan: Validate definition writes against their declared JSON Schema

## Summary

`srs composition create` (and, by inspection, `view create` / `type create`) writes a
definition file and reports `ok: true` without validating it against the JSON Schema the
file declares via `$schema`. The next command that loads the catalog (`repo validate`,
`render`, `find`, ...) fails with `SRS038-R7-SCHEMA-VALIDATION`, and the whole repository is
unusable until the bad file is removed by hand. `create_note`/`update_note` (services.rs),
`create_container` (container_service.rs), and `create_field_normalized` (package_service.rs)
already guard against this — they call `SchemaRegistry::global().validate_by_id(...)` on the
serialized value *before* their own semantic validator runs, and map a failure to
`RepositoryError::SchemaValidation`. `create_composition`/`create_view`/`create_type_in_package`
(and their `update_*` siblings) never do this. Root cause: some Rust definition types (e.g.
`SectionSource`, which still carries the `FixedInstances`/`RelationQuery` variants removed from
`composition.json` by rfc-decision-4f1e12e5 in srs#444) are a superset of their published JSON
Schema, so a value that deserializes fine into the Rust struct can still fail the schema the
loader enforces.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | (this session) |
| Repository Worker | (this session) |
| Verification | (this session) |

## Architecture Decisions

No new architectural decisions — this plan extends the write-time schema-validation pattern
already established by `create_note`/`update_note` (services.rs) and `create_container`
(container_service.rs) to the definition-creating service functions that lack it. Same
`RepositoryError::SchemaValidation` variant, same `SchemaRegistry::global().validate_by_id`
call, same placement (schema validation before the entity's own semantic validator).

**Plan review round 1 findings, resolved:**
- Helper takes `&serde_json::Value`, not a `<T: Serialize>` generic — matches the existing
  precedent (`create_note`/`create_container` already serialize-then-validate inline) and is
  required anyway for Phase 2's protocol handling (see below): `pub(crate) fn
  validate_definition_write_schema(schema_id: &'static str, value: &serde_json::Value,
  context: &std::path::Path) -> Result<(), RepositoryError>` in `validation.rs`, with a doc
  comment distinguishing it from the existing `validate_value_against_schema` (read-path,
  whole-repo diagnostic scanning — different purpose, different signature, keep both).
- **Id-mint-before-schema-validate applies to `create_view` and `create_theme` too, not just
  `create_composition`** — all three currently run their semantic validator (`validate_view`/
  `validate_theme`/`validate_composition`) *before* minting an empty `id`, and `view.json`/
  `theme.json`/`composition.json` all require `id: {type: string, format: uuid}`. Each Phase 2
  task below now says explicitly: mint id first, then schema-validate, then semantic-validate.
  `create_type_in_package` and `create_relation_type` already mint before any validation, so
  no reorder needed there.
- **`create_protocol`/`update_protocol` must validate the raw `value: serde_json::Value` they
  already hold, not a re-serialized `Protocol` struct** — the function deliberately writes
  `value` verbatim to preserve stage fields the typed `Protocol` struct doesn't model (see its
  own comment: "Store the value verbatim..."). Validating a reserialized typed struct here
  would validate something other than what's written — exactly the Rust/schema mismatch this
  bug is about. Since the helper takes `&Value` directly, this is just "pass `&value`", no
  extra serialization step.
- Leaving `create_note`/`create_container`/`create_field_normalized` un-refactored to use the
  new helper is an accepted, deliberate scope cut (avoid unrelated refactor risk in a bug-fix
  PR) — noted, not fixed, per ADR-048 rule 3 (minimal footprint).

## Contracts

### CLI output contract (ADR-011)

No new/changed command output shapes — existing `create`/`update` commands for composition,
view, and type already return `Err` through `output::err(...)` on a `RepositoryError`; this
plan adds one more error variant they can return (`SchemaValidation`, which already has a
payload-safe `Display` impl and is already surfaced by other commands today). No payload
struct changes.

### Entity schema sync (check-schema-sync.sh)

No — this plan does not touch any JSON Schema file, only the Rust write path.

## Scope

- Add schema-validation-before-write to:
  - `create_composition` / `update_composition` (`view_service.rs`) — the exact issue repro.
  - `create_view` / `update_view` (`view_service.rs`) — named in the issue.
  - `create_type_in_package` / `update_type` (`package_service.rs`) — named in the issue.
  - `create_theme` / `update_theme` (`theme_service.rs`)
  - `create_relation_type` / `update_relation_type` (`package_service.rs`)
  - `create_blueprint` / `update_blueprint` (`blueprint_service.rs`)
  - `create_protocol` / `update_protocol` (`protocol_service.rs`)
  - `create_vocabulary` (`vocabulary_service.rs`)
- One regression test per create path proving: (a) a definition that satisfies the Rust type
  but violates the JSON Schema is now rejected at create-time with `SchemaValidation`, and
  (b) `repo validate` never sees the bad file (nothing was written).
- The exact issue repro as its own test: a composition section with
  `"source": {"type": "fixed-instances", "instanceIds": [...]}` is refused by
  `create_composition`.

**Out of scope:**
- `create_lifecycle` (`lifecycle_service.rs`) — its create path has no semantic validator
  call at all yet (`validate_lifecycle` is only wired into `update_lifecycle`), so adding
  schema validation there touches a wider pre-existing gap than this issue's scope. Filed as
  a follow-up (see below) rather than folded in here.
- Reviving `SectionSource::FixedInstances`/`RelationQuery` in the published schema — that is
  a separate, disputed question tracked on srs#832 (an explicit accepted decision,
  rfc-decision-4f1e12e5, removed them; reviving them is a spec governance call, not this
  bug). This plan only makes `fixed-instances` fail fast at create time instead of bricking
  the repo — it does not change what is or isn't valid.

## Phases

### Phase 1: Shared validation helper

**Goal:** One place implements "validate a serialized value against schema_id → map to
`RepositoryError::SchemaValidation`", used by every call site in this plan.

**Agent:** Repository Worker

#### Tasks

- [ ] Add `pub(crate) fn validate_definition_write_schema(schema_id: &'static str, value: &serde_json::Value, context: &std::path::Path) -> Result<(), RepositoryError>` to `crates/srs-repository/src/validation.rs` (already imports `SchemaRegistry`). Body: call `SchemaRegistry::global().validate_by_id(schema_id, value)`, map failure to `RepositoryError::SchemaValidation { path: context.to_path_buf(), message: e.to_string() }`. Add a doc comment noting this is the write-path fail-fast twin of the existing read-path `validate_value_against_schema` in the same file (whole-repo diagnostic scanning) — same schema registry, different purpose/signature, both kept.
- [ ] Callers serialize their typed struct to `serde_json::Value` themselves (mapping a serialize failure to `RepositoryError::Serialize { path, source: e }`, which already exists and is used this way at the three precedent call sites) before calling the helper — except `create_protocol`/`update_protocol`, which already hold the raw `Value` they're about to write and pass it directly (see Phase 2).
- [ ] Do not change the three existing inline call sites (`create_note`/`update_note`, `create_container`, `create_field_normalized`) — leave their working code as-is; this helper is for the new call sites only, to avoid an unrelated refactor risk in this bug-fix PR.

#### Acceptance Criteria

- [ ] `cargo build -p srs-repository` succeeds.

#### Testing

```bash
cargo build -p srs-repository
```

#### Milestone gate

`cargo build -p srs-repository` is clean; no behavior changed yet (helper is unused until Phase 2). Commit: `feat(repo): add write-time schema validation helper (#1098)`.

### Phase 2: Wire the helper into composition, view, type (the issue's named commands)

**Goal:** The issue's exact repro (`composition create` with `fixed-instances`) is refused
at create time; `view create` and `type create`/`type_in_package` get the same treatment.

**Agent:** Repository Worker

#### Tasks

- [ ] `view_service.rs::create_composition`: reorder so `if composition.id.is_empty() { composition.id = new_instance_id(); }` runs **first**, then serialize `&composition` to `raw` (map serialize error to `RepositoryError::Serialize`), call `validate_definition_write_schema(srs_schema::COMPOSITION_SCHEMA_ID, &raw, Path::new(&format!("{boundary_path}/compositions")))`, **then** the existing `validate_composition(&composition)` call.
- [ ] `view_service.rs::update_composition`: `composition.id` is already set from `composition_id` before validation — just insert serialize+`validate_definition_write_schema` (path `"package/compositions"`) before the existing `validate_composition` call, no reorder needed.
- [ ] `view_service.rs::create_view`: reorder so `if view.id.is_empty() { view.id = new_instance_id(); }` runs **first** (currently it runs *after* `validate_view` — move it above), then serialize+validate against `srs_schema::VIEW_SCHEMA_ID`, **then** `validate_view`.
- [ ] `view_service.rs::update_view`: insert serialize+`validate_definition_write_schema` (path `"package/views"`) before the existing `validate_view` call — `view.id` here comes from the caller's JSON body (this function does not mint or reassign it), so no reorder needed, just insert.
- [ ] `package_service.rs::create_type_in_package`: insert serialize+`validate_definition_write_schema` with `srs_schema::TYPE_SCHEMA_ID` right after `record_type.id` is minted (there is no existing semantic validator call in this function beyond the dangling-fieldId check, which runs first and is unaffected), before `store.save_type`.
- [ ] `package_service.rs::update_type`: insert the same call before `store.update_type_file`.
- [ ] Import `srs_schema::{COMPOSITION_SCHEMA_ID, VIEW_SCHEMA_ID, TYPE_SCHEMA_ID}` and `crate::validation::validate_definition_write_schema` where needed.

#### Acceptance Criteria

- [ ] A composition create with a `fixed-instances` section source returns `SchemaValidation` and writes no file.
- [ ] A view/type create with a schema-violating shape (test with a value that satisfies the Rust struct but not the schema, or a hand-built raw JSON with an invalid enum value not covered by the Rust enum's `deny_unknown_fields`/`oneOf`) is refused the same way.
- [ ] Existing valid create/update calls for composition, view, and type still succeed (no false positives) — run the full existing test suites for `view_service.rs` and the type tests in `package_service.rs`.

#### Testing

```bash
cargo test -p srs-repository view_service
cargo test -p srs-repository package_service
```

New tests (in each file's existing `#[cfg(test)] mod tests`):
- `create_composition_rejects_fixed_instances_section_source` — build a composition JSON with a `fixed-instances` section, call `create_composition_normalized`, assert `Err(RepositoryError::SchemaValidation { .. })`, and assert nothing was written (e.g. `list_compositions` returns empty / the compositions dir was never created, matching how `create_composition_fails_with_empty_sections` already asserts on validation failure).
- `create_view_rejects_schema_violation` — same shape of test for View, using a field/shape the schema forbids.
- `create_type_rejects_schema_violation` — same shape of test for Type.

#### Milestone gate

`cargo test -p srs-repository` (full crate, not just the two modules) is green, the three new tests pass, and no pre-existing `view_service`/`package_service` test changed behavior. Mark Phase 2 checkboxes `[x]`. Commit: `fix(repo): validate composition/view/type against declared schema at write time (#1098)`.

### Phase 3: Extend the same pattern to theme, relation-type, blueprint, protocol, vocabulary

**Goal:** Close the identical gap in the remaining definition kinds the issue's "definitions
should get the same treatment" covers, using the same helper.

**Agent:** Repository Worker

**Risk note (architecture review):** `create_relation_type`/`update_relation_type` currently
run *zero* semantic or schema validation — this phase's change is the first gate ever applied
there. Real corpus fixtures (e.g. `srs/packages/com.mudemocracy.governance/.../relation-types/
supersedes-*.json`) already include `$schema` and conform, but confirm via the full test run
below before trusting this phase; if enabling it breaks existing tests or looks likely to
reject real installed packages, drop relation-type from this PR and file it alone with the
failure evidence rather than forcing it through.

#### Tasks

- [ ] `theme_service.rs::create_theme`: reorder so `if theme.id.is_empty() { theme.id = new_instance_id(); }` runs **first** (currently after `validate_theme` — move it above, same hazard as `create_view`), then serialize+`validate_definition_write_schema(THEME_SCHEMA_ID, ...)`, **then** `validate_theme`.
- [ ] `theme_service.rs::update_theme`: insert serialize+`validate_definition_write_schema` before the existing `validate_theme` call, no reorder (id comes from caller's body, unchanged here).
- [ ] `package_service.rs::create_relation_type`: id/createdAt already minted before any write — insert serialize+`validate_definition_write_schema(RELATION_TYPE_SCHEMA_ID, ...)` right after minting, before `store.save_relation_type_definition`.
- [ ] `package_service.rs::update_relation_type`: insert the same call before `store.save_relation_type_definition`.
- [ ] `blueprint_service.rs::create_blueprint`: read the function fully first to find where `id` (if any) is minted and whether `validate_blueprint` is already called there; insert serialize+`validate_definition_write_schema(BLUEPRINT_SCHEMA_ID, ...)` after id-minting (reorder if the existing validator runs first, mirroring the `create_view` fix) and before the write.
- [ ] `blueprint_service.rs::update_blueprint`: same call before the write.
- [ ] `protocol_service.rs::create_protocol`: insert `validate_definition_write_schema(PROTOCOL_SCHEMA_ID, &value, Path::new(&full_path))` on the raw `value` (not a re-serialized `Protocol`) — right after `let protocol = protocol_from_value(&value)?; check_protocol(&protocol)?;` and before `store.save_instance_json(&full_path, &value)`.
- [ ] `protocol_service.rs::update_protocol`: same call on its `value` (after the `createdAt` preservation splice, since that mutates `value` and must be part of what's validated) before the write.
- [ ] `vocabulary_service.rs::create_vocabulary`: insert serialize+`validate_definition_write_schema(VOCABULARY_SCHEMA_ID, ...)` after id-minting, before the write. Note `body_for_definition_schema` in `catalog.rs` strips `$schema` before validating vocabulary/lifecycle bodies (srs-rust#1058) — if `Vocabulary.schema` is `Some(..)` when serialized here, either strip it the same way before calling the helper, or confirm `vocabulary.json` tolerates it present (check the schema file directly; don't assume).
- [ ] Import the new `*_SCHEMA_ID` constants and `crate::validation::validate_definition_write_schema` where needed in each file.

#### Acceptance Criteria

- [ ] One regression test per entity kind proving a schema-violating-but-Rust-valid value is refused at create time: `create_theme_rejects_schema_violation`, `create_relation_type_rejects_schema_violation`, `create_blueprint_rejects_schema_violation`, `create_protocol_rejects_schema_violation`, `create_vocabulary_rejects_schema_violation`.
- [ ] Full existing test suites for each touched file still pass — this is the load-bearing check for the relation-type risk note above.

#### Testing

```bash
cargo test -p srs-repository theme_service
cargo test -p srs-repository package_service
cargo test -p srs-repository blueprint_service
cargo test -p srs-repository protocol_service
cargo test -p srs-repository vocabulary_service
```

#### Milestone gate

`cargo test -p srs-repository` green, all five new tests pass, zero pre-existing test broken.
If relation-type (or any other kind) fails the risk check above, drop it from this phase,
note why in the plan and the PR, and file it as its own follow-up issue instead of forcing a
fix under time pressure. Commit per surviving entity kind or as one commit covering whatever
subset landed: `fix(repo): validate theme/relation-type/blueprint/protocol/vocabulary against declared schema at write time (#1098)`.

### Phase 4: Full workspace gate + docs

**Goal:** Everything green; docs mention the guarantee where relevant.

#### Tasks

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] File a follow-up issue for `create_lifecycle` (out of scope, see above). Before filing, check whether issue #1098 itself has a parent epic/story (`gh issue view 1098` sub-issue-of field, or `node /tmp/gh-project.mjs` lookup if available); if it does, parent the follow-up under the same parent with `gh-project link`; if #1098 has no parent, leave the follow-up unparented and say so explicitly in its body and in the PR — don't guess a parent.
- [ ] No CLAUDE.md/ADR text currently claims "no write-time validation for definitions" that this would contradict — check `srs-rust/CLAUDE.md`'s Service Function Contract section (ADR-010: "Validation: all validation in the service, not in the CLI handler") already covers this; no wording change needed. State this explicitly rather than editing unnecessarily.

#### Acceptance Criteria

- [ ] `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` all pass with zero failures/warnings.

#### Testing

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

#### Milestone gate

All three workspace-wide commands exit 0. The lifecycle follow-up issue is filed (parented
or explicitly noted as unparented). Commit any final doc/cleanup diffs, if any: `chore: final gate for #1098`.

## Post-implementation: scope reductions (evidence-based)

Full-workspace testing (`cargo test --workspace --no-fail-fast`) surfaced two cases where
write-time schema validation, once wired in, broke *real, legitimate* behavior — not just
test-fixture artifacts. Both were dropped from this PR per the Phase 3 risk note's own
contingency ("if enabling it breaks existing tests or looks likely to reject real installed
packages, drop it from this PR and file it alone with the failure evidence").

- **`create_blueprint`/`update_blueprint` — dropped entirely.** `blueprint.json`'s
  `RelationSpec.cardinality` is a closed enum (`one-to-one`/`one-to-many`/`many-to-one`/
  `many-to-many`), but `srs-repository`'s own `blueprint_schema_service::parse_cardinality`
  (with its own dedicated unit test, `blueprint_schema_cardinality_min_max`) accepts a second,
  actively-used numeric-range grammar (`"1..*"`, `"0..3"`, `"1"`, ...) that the schema's enum
  doesn't declare. A CLI integration test (`blueprint_schema_emits_nested_draft07`) exercises
  exactly this grammar end-to-end. Enforcing the schema at write time would have silently
  broken that supported feature, not fixed a bug — the schema is itself behind the
  implementation here, the same shape of drift as `fixed-instances`, but in the *opposite*
  direction (implementation right, schema stale) and on a feature still in active use. Filed
  as its own srs issue (schema/implementation reconciliation needed before write-time
  validation is safe for Blueprint) rather than bundled here.
- **`create_relation_type`/`update_relation_type` — dropped entirely.** `field_type_migration_service.rs`'s
  `migrate_substrate_properties_to_meta` (the srs-rust#894 `properties`→`meta` data-model
  migration) calls `update_relation_type` internally to rewrite legacy relation-type files
  that predate the current `$schema`-required convention. Enforcing schema validation there
  made a real upgrade path — migrating an old repository forward — fail outright on exactly
  the kind of pre-existing, non-conforming data the migration exists to fix. This is a
  chicken-and-egg problem specific to relation-type's migration story, not present for
  composition/view/type/theme/vocabulary. Filed as its own follow-up (migration call sites
  need a lower-level write path that bypasses full-schema enforcement, or the migration
  needs to stamp `$schema` before/as part of its own rewrite) rather than bundled here.

Both reversions are clean: the shared `validate_definition_write_schema` helper, and the
composition/view/type/theme/vocabulary wiring, are unaffected. `create_type_in_package`/
`update_type` needed no such reversion — Type's Rust representation had no comparable
implementation-ahead-of-schema drift.

## Final Acceptance

- [ ] `cargo build --workspace` — 0 errors.
- [ ] `cargo test --workspace` — 0 failures.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 warnings.
- [ ] The exact issue repro (`fixed-instances` composition section) is refused by
      `create_composition` with `RepositoryError::SchemaValidation`, and no file is written.
- [ ] `view create`/`type create` get the same treatment (issue's explicit ask).
- [ ] Every entity kind actually wired in Phase 3 has a passing regression test; any kind
      dropped per its risk note is filed as its own issue instead, not silently skipped.
- [ ] No payload struct changes (`cargo run --bin generate-schemas` not needed — confirm no
      diff in `crates/srs-cli/schemas/payload/` after the change).
- [ ] No JSON Schema files touched (`bash scripts/check-schema-sync.sh` not applicable — confirm no diff under `crates/srs-schema/schemas/`).
- [ ] `create_lifecycle` follow-up issue filed per Phase 4.
- [ ] PR body cites the precedent this plan extends (`create_note`/`create_container`/
      `create_field_normalized`) and states this is reinforcing an existing ADR-010 contract,
      not new architecture (no `gate:owner-merge` needed on that basis alone — merge-rights
      classification still follows the repo's own door/mode rules).
