# Plan: Extract PackageBoundary::from_pkg_json constructor

## Summary

Blueprint/protocol path extraction from a raw `serde_json::Value` is duplicated identically across three places in `srs-repository`: `file_store_boundary_from_json` in `store.rs`, `json_store_boundary_from_json` in `json_store.rs`, and an inline struct literal in `import_package_local` in `package_service.rs`. The service occurrence in `package_service.rs` additionally crosses the storage-adapter boundary that ADR-009 and ADR-010 prohibit — a service function must not reach into raw JSON fields that only the storage adapter should know about. This plan consolidates all three into a single `PackageBoundary::from_pkg_json` associated function on the type in `package_types.rs`, updates all three call sites, and removes the now-redundant private functions.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions. This plan implements the storage adapter boundary rule from ADR-009 and the service function contract from ADR-010: service code (`package_service.rs`) must not duplicate storage-adapter logic for parsing raw JSON.

| ADR | Decision | Status |
|---|---|---|
| [ADR-009](../docs/adr/009-package-boundary-model.md) | Storage adapters own the mapping from raw JSON to `PackageBoundary`; services address packages through `PackageSelector` and boundary methods | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions take typed structs, not raw JSON values; no duplication across service and adapter layers | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. No `payload.rs` changes. No schema regeneration required.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files are added or modified. `bash scripts/check-schema-sync.sh` is expected to exit 0 with no changes.

---

## Scope

- Add `PackageBoundary::from_pkg_json(pkg_json: &serde_json::Value, selector: PackageSelector) -> PackageBoundary` to `crates/srs-repository/src/package_types.rs`.
- Replace `file_store_boundary_from_json` in `crates/srs-repository/src/store.rs` with a call to `PackageBoundary::from_pkg_json`; delete the private function.
- Replace `json_store_boundary_from_json` in `crates/srs-repository/src/json_store.rs` with a call to `PackageBoundary::from_pkg_json`; delete the private function.
- Replace the inline `PackageBoundary { ... }` struct literal in `import_package_local` in `crates/srs-repository/src/package_service.rs` with a call to `PackageBoundary::from_pkg_json`.

**Out of scope:**

- Changing the `PackageBoundary` struct fields.
- Changing the `RepositoryStore` trait or any store method signatures.
- Any changes outside `crates/srs-repository/`.
- The empty-boundary construction in `create_package` (`package_service.rs:926`) — that correctly initialises an empty boundary for a new package and does not use `from_pkg_json`.

---

## Phases

### Phase 1: Add constructor and update call sites

**Goal:** All three duplicated `PackageBoundary` constructions from JSON are replaced by `PackageBoundary::from_pkg_json`; the two private functions are deleted; all tests pass.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/package_types.rs`: add `impl PackageBoundary` block with `pub fn from_pkg_json(pkg_json: &serde_json::Value, selector: PackageSelector) -> PackageBoundary`. The body is the shared extraction logic currently duplicated across the three sites: read `id`, `namespace`, `name`, `version` as strings (default `""`), and read `fields`, `types`, `blueprints`, `protocols` as `Vec<String>` (default empty vec).
- [ ] In `crates/srs-repository/src/store.rs`: replace calls to `file_store_boundary_from_json(pkg_json, selector)` with `PackageBoundary::from_pkg_json(pkg_json, selector)` (two call sites: lines ~1588 and ~1624). Delete the `file_store_boundary_from_json` private function (lines ~1817–1864).
- [ ] In `crates/srs-repository/src/json_store.rs`: replace calls to `json_store_boundary_from_json(pkg_json, selector)` with `PackageBoundary::from_pkg_json(pkg_json, selector)` (two call sites: lines ~1509 and ~1550). Delete the `json_store_boundary_from_json` private function (lines ~164–211).
- [ ] In `crates/srs-repository/src/package_service.rs` (`import_package_local`, lines ~1007–1044): replace the inline `PackageBoundary { ... }` struct literal with `PackageBoundary::from_pkg_json(&pkg_json, Some(source_path.clone()))`. Remove the four now-redundant local variables `id`, `namespace`, `name`, `version` (lines ~987–990) and instead use `boundary.id`, `boundary.namespace`, `boundary.name` after construction. Update the duplicate-id check to use `boundary.id`, the error variant to clone `boundary.id.clone()`, and the `ImportPackageLocalResult` return to use `boundary.id.clone()`, `boundary.namespace.clone()`, `boundary.name.clone()`. Keep the `id.is_empty()` validation — just check `boundary.id.is_empty()` instead.
- [ ] Add a unit test in `package_types.rs` — `from_pkg_json_extracts_all_fields` — that constructs a minimal `serde_json::json!({...})` value and asserts all fields parse correctly.
- [ ] Add a unit test — `from_pkg_json_missing_arrays_default_to_empty` — that passes a JSON object with no `fields`/`types`/`blueprints`/`protocols` keys and asserts the resulting vecs are empty (not a panic).

#### Acceptance Criteria

- [ ] `PackageBoundary::from_pkg_json` exists in `package_types.rs` and is `pub`.
- [ ] Neither `file_store_boundary_from_json` nor `json_store_boundary_from_json` exists anywhere in the codebase (`rg "boundary_from_json"` returns no matches).
- [ ] `import_package_local` in `package_service.rs` contains no inline `PackageBoundary { ... }` literal that reads JSON fields directly (no `pkg_json["fields"]` references in that function).
- [ ] `cargo test -p srs-repository` passes with no failures, including cross-store roundtrip tests that exercise `list_package_boundaries`.
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `from_pkg_json_extracts_all_fields` — proves the constructor reads all fields correctly.
- `from_pkg_json_missing_arrays_default_to_empty` — proves no panic when arrays are absent.
- Existing tests: `file_store_list_package_boundaries`, `json_store_list_package_boundaries`, `import_package_local_*` — must continue to pass (no behavior change, only structural).

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm both new tests exist and pass.
3. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update plan checkboxes `[x]`.
5. Commit: `refactor: extract PackageBoundary::from_pkg_json (#356)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged — `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no schema changes)
- [ ] `rg "boundary_from_json"` returns no matches in `src/` (both private fns deleted)
- [ ] `rg 'pkg_json\["fields"\]' crates/srs-repository/src/package_service.rs` returns no matches (inline duplication removed)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming.
- Verification Agent runs after phase completion and before final sign-off.

## Assumptions

- `serde_json` is already a dependency of `srs-repository` (confirmed in `Cargo.toml`). No `Cargo.toml` changes needed.
- The `from_pkg_json` constructor is infallible (matches existing behaviour: all three current implementations return a `PackageBoundary`, not a `Result`).
- The empty-boundary construction in `create_package` (line ~926) is intentionally different and must not be changed — it correctly creates an empty boundary for a new package, not one parsed from existing JSON.
