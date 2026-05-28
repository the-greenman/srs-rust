# Plan: Storage-Agnostic SRS Rust Library

## Summary

Refactor SRS Rust so reusable SRS logic lives in library crates and the CLI is only an interface. The immediate priority is concrete and synchronous: move note operations out of `srs-cli` into `srs-repository`, so the same logic can be called by CLI, Python bindings, and future applications without duplication. Long-term direction is storage-agnostic SRS support for file-backed, database-backed, and embedded applications — but speculative complexity is deferred.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| CLI Worker | — |
| Core Model Worker | — |
| Bindings Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Scope

- Move note service logic from CLI into `srs-repository`
- Add canonical Rust types for remaining SRS data to `srs-core`
- Define a synchronous repository boundary in `srs-repository`
- Expose a JSON-first Python binding surface over services

**Out of scope:**

- Async traits (deferred until a concrete async consumer exists)
- `srs-file-repository` crate extraction (deferred until a second adapter exists)
- Database-backed adapter implementation

---

## Phases

### Phase 1: Move Note Services To The Library

**Status:** `complete`

**Goal:** Note CRUD and tag operations live in `srs-repository`; CLI handlers are thin wrappers.

**Agent:** Repository Service Worker + CLI Worker

#### Tasks

- [x] Add `list_notes(repo_root, filter)` to `srs-repository`
- [x] Add `get_note_by_id(repo_root, id)` to `srs-repository`
- [x] Add `create_note(repo_root, note)` to `srs-repository`
- [x] Add `add_note_tag(repo_root, id, tag)` to `srs-repository`
- [x] Move `slugify_title` to `srs-repository` (library-owned)
- [x] Avoid double-loading manifest in tag/update flows
- [x] Thin CLI handlers — parse args/stdin, call service, wrap output
- [x] Remove duplicated note logic from `srs-cli`

#### Acceptance Criteria

- [x] `srs note list/get/create/tag` CLI behaviour unchanged
- [x] CLI handlers contain no business logic
- [x] Service tests cover list, get, create, tag, slugging, missing IDs, non-note IDs, manifest updates

#### Testing

```bash
cargo test -p srs-repository
cargo test -p srs-cli
cargo test --test integration_tests
```

---

### Phase 2: Add Remaining Core In-Memory Types

**Status:** `open`

**Goal:** `srs-core` has canonical Rust structs for all SRS data (fields, types, records, packages, relations).

**Agent:** Core Model Worker

#### Tasks

- [ ] Add `Field` struct (`id`, `namespace`, `name`, `version`, `valueType`, `aiGuidance`, ...)
- [ ] Add `FieldAssignment` struct (`fieldId`, `order`, `required`, `displayLabel`)
- [ ] Add `Type` struct (`id`, `namespace`, `name`, `version`, `fields: Vec<FieldAssignment>`)
- [ ] Add `FieldValue` struct (`fieldId`, `value: serde_json::Value`)
- [ ] Add `Record` struct (Tier 2: `typeId`, `typeVersion`, `typeNamespace`, `typeName`, `fieldValues`)
- [ ] Add `TypedRecord` struct (Tier 1: named fields, no type binding)
- [ ] Add `Package` struct (`id`, `namespace`, `name`, `version`, `fields[]`, `types[]`, `views[]`)
- [ ] Add `Relation` struct and canonical relation type vocabulary
- [ ] Add validation: Record field values match their Field's `valueType`
- [ ] Extend `srs-core/src/types/mod.rs` with new modules
- [ ] Extend `srs-core/src/validation/mod.rs` with record validation

#### Acceptance Criteria

- [ ] All new structs roundtrip representative schema-compatible JSON
- [ ] Existing note serialization remains compatible
- [ ] Record validation rejects mismatched field value types
- [ ] No filesystem dependencies introduced to `srs-core`

#### Testing

```bash
cargo test -p srs-core
```

Specific tests to write:

- `field_roundtrips_json` — construct Field, serialize → deserialize, assert equal
- `type_roundtrips_json` — Type with FieldAssignments roundtrips
- `record_roundtrips_json` — Tier 2 Record with fieldValues roundtrips
- `record_validation_rejects_wrong_value_type` — fieldValue type mismatch → validation error
- `package_roundtrips_json` — Package with fields + types roundtrips

---

### Phase 3: Synchronous Repository Boundary

**Status:** `open`

**Goal:** Services depend on a repository boundary trait; the file-backed implementation satisfies it; tests can use an in-memory fake.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Define synchronous repository boundary (trait or equivalent) in `srs-repository`:
  - manifest load/save
  - note load/save
  - generic instance JSON load/save
  - relations load/save
  - package JSON load
  - source document discovery
- [ ] Refactor existing file-backed code to implement the boundary
- [ ] Migrate service functions to depend on the boundary, not raw paths
- [ ] Write an in-memory fake store for tests

#### Acceptance Criteria

- [ ] Services can run against in-memory fake in unit tests
- [ ] File-backed behaviour remains compatible
- [ ] No async runtime, async-trait, or pinning introduced
- [ ] CLI integration tests still pass

#### Testing

```bash
cargo test -p srs-repository
cargo test --test integration_tests
```

Specific tests to write:

- `list_notes_against_fake_store` — fake store with two notes → list returns both
- `create_note_against_fake_store` — create via fake → note present in fake, manifest updated

---

### Phase 4: Generic Record Operations (Tier 2 Foundation)

**Status:** `open`

**Goal:** Library can load, list, validate, and create any Tier 2 Record against its Type from the package — no type-specific code required.

**Agent:** Repository Service Worker + Core Model Worker

**Depends on:** Phase 2 (core types), Phase 3 (storage boundary)

#### Tasks

- [ ] Add `load_package(repo_root) -> Result<Package, RepositoryError>` to `srs-repository`
- [ ] Add `list_records_by_type(repo_root, type_namespace, type_name, type_version) -> Result<Vec<Record>, RepositoryError>`
- [ ] Add `get_record_by_id(repo_root, id) -> Result<Option<Record>, RepositoryError>`
- [ ] Add `create_record(repo_root, type_ref, field_values) -> Result<Record, RepositoryError>`
- [ ] Validate Record field values against resolved Type on load and create
- [ ] Expose `list_records_by_type` and `create_record` from `srs-repository` public API

#### Acceptance Criteria

- [ ] `list_records_by_type` returns all Tier 2 Records of a given type from a live repo
- [ ] Records are validated against their Type on load (field value type checking)
- [ ] No `TagDefinition`-specific code in `srs-repository` — tags are just one type
- [ ] Creating a Record with a missing required field returns a validation error

#### Testing

```bash
cargo test -p srs-repository
```

Specific tests to write:

- `list_records_by_type_returns_matching_records` — temp repo with 2 types, filter by one
- `record_validation_rejects_missing_required_field` — create with missing required field → error
- `load_package_from_live_repo` — load package from `srs/`, assert field/type counts > 0

---

### Phase 5: CLI and Python Bindings Over Services

**Status:** `open`

**Goal:** CLI handlers delegate entirely to library services. Python bindings expose the same services over JSON.

**Agent:** CLI Worker + Bindings Worker

**Depends on:** Phase 1 (complete), Phase 4 for Tier 2 commands

#### Tasks

CLI:
- [ ] Verify all handlers: parse → call service → wrap output (no inline logic)
- [ ] Add `srs record list --type <namespace/name>` command using `list_records_by_type`
- [ ] Add `srs record get <id>` command using `get_record_by_id`

Python bindings:
- [ ] Scaffold `crates/srs-bindings/` with PyO3
- [ ] Expose `repo_map_json(repo_path: str) -> str`
- [ ] Expose `note_list_json(repo_path: str, tag: Option<str>) -> str`
- [ ] Expose `note_get_json(repo_path: str, id: str) -> str`
- [ ] Expose `note_create_json(repo_path: str, note_json: str) -> str`
- [ ] Expose `note_tag_json(repo_path: str, id: str, tag: str) -> str`
- [ ] Expose `note_audit_tags_json(repo_path: str) -> str`
- [ ] Expose `note_foundations_json(repo_path: str) -> str`
- [ ] Expose `migration_packet_json(repo_path: str, profile: str) -> str`

#### Acceptance Criteria

- [ ] All Python binding functions return parseable JSON
- [ ] Python bindings call the same services as CLI (zero duplicated logic)
- [ ] `srs record list` returns all Tier 2 records of the given type
- [ ] CLI behaviour remains compatible

#### Testing

```bash
cargo test -p srs-bindings
cargo test --test integration_tests
```

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI integration tests pass (output format unchanged)
- [ ] Note logic is reusable without calling CLI code
- [ ] Core data structures are usable in any application (no I/O in `srs-core`)
- [ ] Tier 2 Records can be loaded and validated generically — no type-specific library code
- [ ] Python bindings call Rust services, not duplicated logic
- [ ] Database-backed implementations can be added by implementing the storage boundary

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- Synchronous APIs come first. Async is deferred until there is a concrete async consumer.
- Python bindings are JSON-first initially.
- Database adapter implementation is out of scope for this pass.
- `srs-file-repository` crate extraction is deferred until a second adapter justifies the split.
- The CLI may own workflow-facing profile policy; reusable logic belongs in library crates.
