# Plan: Core ext:registry Types — Registry + RegistryEntry

## Summary

The `ext:registry` extension is specified in `srs/srs/records/extensions/ext-registry.json` and defines a package catalog format (`Registry` + `RegistryEntry`). No Rust types for these exist yet. This plan adds them to `srs-core/src/extensions/registry.rs` so the service layer (#244) and bindings can be built against a stable, spec-aligned in-memory representation.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (current session) |
| Core Model Worker | Claude (current session) |
| Verification | Claude (current session) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-028](../docs/adr/028-extension-catalog-types-in-srs-core.md) | Extension catalog file types (external data format files defined by an extension) get native Rust structs in `srs-core/extensions/`. Distinct from ADR-005 (extension *definition records*). Reverses ADR-005's "may be removed" note on `extensions/mod.rs`. | accepted (new, this plan) |
| [ADR-002](../docs/adr/002-tier2-generic-record-operations.md) | `Registry`/`RegistryEntry` are not SRS Tier 2 instance records — they are external catalog files consumed by the service layer. ADR-002 (generic record operations for SRS instances) does not apply. | accepted (does not apply) |
| [ADR-005](../docs/adr/005-extension-definitions-are-tier2-records.md) | Extension *definition records* remain generic tier 2 records. This plan adds extension *data format types* — a distinct class governed by ADR-028. `extensions/mod.rs` is not removed. | accepted (unchanged; scope clarified by ADR-028) |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | No service logic here — types only. Service and CLI live in #244. | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No CLI commands are added or changed. `Registry`/`RegistryEntry` are not yet referenced by any payload struct. No schema regeneration needed.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files in `srs/docs/schema/2.0/` are added or changed. The types live only in Rust — no file-level schema sync is required. `bash scripts/check-schema-sync.sh` should exit 0 unchanged.

---

## Scope

- Add `RegistryEntry` struct to `crates/srs-core/src/extensions/registry.rs`
- Add `Registry` struct to the same file
- Export both from `crates/srs-core/src/extensions/mod.rs` and re-export via `crates/srs-core/src/lib.rs`
- Unit tests: JSON roundtrip, optional-field omission, unknown-field tolerance (forward compat)

**Out of scope:**
- Service layer (`registry_service`) — that is #244
- CLI command (`srs registry`) — that is #244
- WASM binding — that is #244
- JSON Schema file for registry catalog format — deferred; no file-level validation needed yet
- Schema contract test (`registry_catalog_passes_schema_contract`) — deferred pending a `registry-catalog.json` schema in `srs/docs/schema/2.0/`. ADR-004 requires this test once a schema exists.
- Validation beyond field types (e.g., UUID format checking, semver catalogVersion) — deferred

---

## Phases

### Phase 1: Add Registry types to srs-core

**Goal:** `Registry` and `RegistryEntry` exist in `srs-core`, export cleanly from the crate root, and pass roundtrip + contract tests.

**Agent:** Core Model Worker

#### Tasks

- [ ] Create `crates/srs-core/src/extensions/registry.rs` with:
  - `RegistryEntry` struct, fields per spec:
    - `package_id: String` — UUID stored as String (pattern from `record.rs`)
    - `package_name: String`
    - `package_version: String`
    - `publisher: String`
    - `description: Option<String>` — `#[serde(skip_serializing_if = "Option::is_none")]`
    - `published_at: String` — ISO8601 stored as String
    - `homepage: Option<String>` — skip-if-none
    - `tags: Option<Vec<String>>` — skip-if-none
    - `field_count: u32` — min 0 enforced by u32
    - `type_count: u32`
    - `view_count: Option<u32>` — skip-if-none
    - `schema_count: Option<u32>` — skip-if-none
    - `protocol_count: Option<u32>` — skip-if-none
    - `relation_type_count: Option<u32>` — skip-if-none
    - `download_url: Option<String>` — skip-if-none
    - `checksum: Option<String>` — skip-if-none
  - `Registry` struct, fields per spec:
    - `schema_version: String`
    - `registry_id: String` — UUID stored as String
    - `registry_name: String`
    - `catalog_version: String` — semver stored as String
    - `updated_at: String` — ISO8601 stored as String
    - `homepage: Option<String>` — skip-if-none
    - `entries: Vec<RegistryEntry>`
  - Both structs: `#[derive(Debug, Clone, Serialize, Deserialize)]`, `#[serde(rename_all = "camelCase")]`. No `PartialEq` (large document-level structs omit it per `Blueprint`/`Protocol`/`Record` pattern). No `deny_unknown_fields` — external files may carry future fields.
- [ ] Update `crates/srs-core/src/extensions/mod.rs`:
  - Add `pub mod registry;` (no `pub use` re-export — matches `types/mod.rs` pattern; callers import as `srs_core::extensions::registry::{Registry, RegistryEntry}`)
- [ ] Update `crates/srs-core/src/lib.rs`:
  - Confirm `pub mod extensions;` is present (add if missing)
- [ ] Add unit tests in `registry.rs` (`#[cfg(test)]` block):
  - `registry_entry_roundtrips_json` — full entry with all fields, serialize → deserialize, check key fields
  - `registry_entry_omits_optional_fields` — minimal entry (required fields only), verify `description`, `homepage`, `tags`, `viewCount` etc. absent in JSON output
  - `registry_roundtrips_json` — full Registry with two entries
  - `registry_tolerates_unknown_fields` — deserialize JSON with an extra `"future_field": "x"`, confirm no error (forward compat for `Registry`)
  - `registry_entry_tolerates_unknown_fields` — parallel test for `RegistryEntry` (extra field in entry object), confirm no error (forward compat for entry rows)

#### Acceptance Criteria

- [ ] `Registry` and `RegistryEntry` importable as `srs_core::extensions::registry::{Registry, RegistryEntry}`
- [ ] All five unit tests exist and pass
- [ ] No other tests in `srs-core` regress

#### Testing

```bash
cargo test -p srs-core
```

Specific tests to write or verify:

- `registry_entry_roundtrips_json` — proves all required + optional fields serialize and deserialize correctly
- `registry_entry_omits_optional_fields` — proves `skip_serializing_if` works for all optional fields
- `registry_roundtrips_json` — proves the top-level container with entries works end-to-end
- `registry_tolerates_unknown_fields` — proves no `deny_unknown_fields` rejection

#### Milestone gate

1. All four named tests exist and pass.
2. No other tests in srs-core regress.
3. `Registry` and `RegistryEntry` are importable as `srs_core::extensions::{Registry, RegistryEntry}`.

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

4. Mark task checkboxes `[x]`.
5. Commit: `feat(srs-core): add Registry + RegistryEntry types for ext:registry (#243)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (no payload structs changed)
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] `Registry` and `RegistryEntry` importable from `srs_core::extensions`
- [ ] All four unit tests pass: roundtrip, optional-field omission, full-registry roundtrip, unknown-field tolerance

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- The `serde` and `serde_json` workspace dependencies are sufficient — no new crate dependencies required.
- `uuid` in `srs-core`'s `Cargo.toml` is used for generation elsewhere; UUIDs here are stored as `String` (matching the pattern in `record.rs`, `term.rs`, etc.).
- The `extensions` module exists (`src/extensions/mod.rs` is present, currently empty) — updating it is in scope. ADR-005 flagged it as "may be removed"; populating it here is a deliberate reversal of that direction, governed by ADR-028.
