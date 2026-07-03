# Plan: Enable manifest validation + require/validate root container (#264)

## Summary

`validate_repository()` validates manifest JSON schema, instances, relations, and package invariants, but does not check RFC-013 root container invariants. This plan adds four spec-defined invariant checks (I-79, I-80, I-81, I-82) to `validate_repository()` in `srs-repository`, completing the last open Gate A issue. No new CLI command, payload struct, or WASM binding is introduced — only the internals of the existing validation service change.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

_No new architectural decisions — this plan implements existing ADR-001 (library-first) and ADR-010 (service boundary contract) by adding spec-defined invariant checks to the existing `validate_repository` service function._

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | All business logic (including validation) lives in `srs-repository`, not `srs-cli` | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions own all validation; CLI calls one function | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | The existing WASM `validate` binding calls `validate_repository()` directly — new invariants propagate automatically with no binding changes needed | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new/changed commands — `validate_repository()` is an existing service called by `srs repo validate`. The `RepositoryValidationReport` struct is unchanged. No payload struct regeneration required.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files changed. No action required.

---

## Scope

- Add RFC-013 invariant checks I-79, I-80, I-81, I-82 to `validate_repository()` in `crates/srs-repository/src/validation.rs`
- Add unit tests for each new invariant (MemoryStore) within the same file
- Update `minimal_manifest()` test helper to include an identity note and update any tests affected by the stricter validation

**Out of scope:**
- Changes to any other file (CLI, bindings, payload, schema)
- Adding a write service for `manifest.container` (tracked separately as #318)
- Updating the live srs spec repo's manifest.json (pre-RFC-013 repo; its `container` is already present in the vendored spec-repo fixture used by tests)

---

## Phases

### Phase 1: RFC-013 root container validation

**Goal:** `validate_repository()` enforces I-79, I-80, I-81, and I-82 and all tests pass.

**Agent:** Repository Service Worker

#### Context: Root container structure

The `manifest.container` embed (the `Container` struct typed in #263/#312) typically contains only `containerId` and `identityInstanceId`. The full container data (`memberInstanceIds`, `rootInstanceIds`) lives in the separate container file `containers/{id}.json`, which is found via `containerIndex`.

`store.load_container(container_id)` resolves the path from `containerIndex` entries. If the root container is not in `containerIndex` (older repos, vendored spec-repo fixture), it returns `ContainerNotFound` — handle gracefully.

#### Invariants to implement

| ID | Severity | Check |
|---|---|---|
| I-79 | ERROR | `manifest.container` is present (also enforced by JSON schema) |
| I-79/I-80 | ERROR | Root container file validates via `validate_container_invariants` (member/root IDs in instanceIndex) |
| I-81 | ERROR | `identityInstanceId` (when present in embed) is in `rootInstanceIds` or `memberInstanceIds` of the full container |
| I-82 | WARNING | Each non-identity member of the root container should be the root of some container in `containerIndex` (suppressed if `containerIndex` absent or empty) |

#### Tasks

- [ ] In `crates/srs-repository/src/validation.rs`, after line 88 (`let manifest = store.load_manifest()?;`), insert the RFC-013 block described below.
- [ ] Add `use crate::container_service;` or use full path `crate::container_service::validate_container_invariants` (prefer full path to keep the import list minimal).
- [ ] Write unit tests for each invariant in the `#[cfg(test)]` block in the same file (use `MemoryStore`).
- [ ] Verify `live_srs_repo_validates_cleanly` still passes (vendored fixture has `container` with no members — all four invariants trivially pass).

#### Implementation

Insert after `let manifest = store.load_manifest()?;` (currently line 88), before `// --- Validate each instanceIndex entry ---`:

```rust
// --- RFC-013 root container invariants (I-79, I-80, I-81, I-82) ---
// I-79 is also enforced structurally by the manifest JSON schema above.
match manifest.container.as_ref() {
    None => {
        diagnostics.push(ValidationDiagnostic {
            severity: DiagnosticSeverity::Error,
            relative_path: "manifest.json".to_string(),
            schema_id: None,
            message:
                "RFC-013 I-79: manifest.container is absent; every SRS repository must declare a root container"
                    .to_string(),
        });
    }
    Some(root) => {
        // Try to load the full container from the store.
        // Repos where the root container is not in containerIndex (e.g. pre-RFC-013 fixtures)
        // return ContainerNotFound — skip file-based checks in that case.
        // NOTE: `store.load_container` is called here; `validate_container_invariants` also calls
        // it internally, resulting in two loads. This is acceptable (stores cache cheaply) and
        // the outer load is needed to branch on ContainerNotFound vs. other errors. A comment in
        // the code should acknowledge the redundant load.
        let full_container_opt: Option<srs_core::types::container::Container> =
            match store.load_container(&root.container_id) {
                Ok(c) => {
                    // I-79 / I-80: validate structural invariants (Inv 20-21) via existing helper.
                    // NOTE: `validate_container_invariants` internally calls `store.load_manifest()`
                    // a second time to get the instanceIndex. This is a known double-load inherited
                    // from the helper's design (it takes only store + container_id). No fix needed here.
                    match crate::container_service::validate_container_invariants(
                        store,
                        &root.container_id,
                    ) {
                        Ok(report) => {
                            for err in report.errors {
                                diagnostics.push(ValidationDiagnostic {
                                    severity: DiagnosticSeverity::Error,
                                    relative_path: "manifest.json".to_string(),
                                    schema_id: None,
                                    message: format!(
                                        "RFC-013 I-79/I-80: root container '{}': {}",
                                        root.container_id, err
                                    ),
                                });
                            }
                        }
                        Err(e) => {
                            diagnostics.push(ValidationDiagnostic {
                                severity: DiagnosticSeverity::Error,
                                relative_path: "manifest.json".to_string(),
                                schema_id: None,
                                message: format!(
                                    "RFC-013 I-79: failed to validate root container '{}': {}",
                                    root.container_id, e
                                ),
                            });
                        }
                    }
                    Some(c)
                }
                Err(RepositoryError::ContainerNotFound { .. }) => {
                    // Root container not in containerIndex — graceful degradation.
                    // I-81/I-82 checks are SKIPPED: without the full container file we cannot
                    // determine memberInstanceIds/rootInstanceIds, so we cannot validate them.
                    // This is intentionally lenient for pre-RFC-013 repos and minimally-initialised
                    // stores (e.g. vendored spec-repo fixture). I-79 schema check above already
                    // enforces the container field is present.
                    None
                }
                Err(e) => {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        relative_path: "manifest.json".to_string(),
                        schema_id: None,
                        message: format!(
                            "RFC-013 I-79: could not load root container '{}': {}",
                            root.container_id, e
                        ),
                    });
                    None
                }
            };

        // I-81 and I-82 only run when the full container file was successfully loaded.
        // If the container wasn't found (ContainerNotFound) both checks are silently skipped.
        if let Some(ref full_container) = full_container_opt {
            // I-81: identityInstanceId (from the manifest embed) MUST be in rootInstanceIds
            //       or memberInstanceIds of the full container.
            if let Some(ref identity_id) = root.identity_instance_id {
                let in_roots = full_container
                    .root_instance_ids
                    .as_ref()
                    .is_some_and(|ids| ids.contains(identity_id));
                let in_members = full_container
                    .member_instance_ids
                    .as_ref()
                    .is_some_and(|ids| ids.contains(identity_id));
                if !in_roots && !in_members {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        relative_path: "manifest.json".to_string(),
                        schema_id: None,
                        message: format!(
                            "RFC-013 I-81: identityInstanceId '{}' is not in rootInstanceIds or memberInstanceIds of the root container",
                            identity_id
                        ),
                    });
                }
            }

            // I-82: each non-identity member of the root container SHOULD be the root of
            //       some container listed in containerIndex (Warning; suppressed when containerIndex
            //       is absent or empty).
            if let Some(ref ci) = manifest.container_index {
                if !ci.is_empty() {
                    if let Some(ref members) = full_container.member_instance_ids {
                        // Build the set of instance IDs that serve as roots of containerIndex containers.
                        let mut section_container_roots: HashSet<String> = HashSet::new();
                        for entry in ci {
                            if let Ok(c) = store.load_container(&entry.container_id) {
                                if let Some(ref roots) = c.root_instance_ids {
                                    section_container_roots.extend(roots.iter().cloned());
                                }
                            }
                        }
                        let identity_id = root.identity_instance_id.as_deref().unwrap_or("");
                        for member_id in members {
                            if member_id.as_str() == identity_id {
                                continue;
                            }
                            if !section_container_roots.contains(member_id.as_str()) {
                                diagnostics.push(ValidationDiagnostic {
                                    severity: DiagnosticSeverity::Warning,
                                    relative_path: "manifest.json".to_string(),
                                    schema_id: None,
                                    message: format!(
                                        "RFC-013 I-82: root container member '{}' is not the root of any container in containerIndex",
                                        member_id
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}
```

#### Unit tests to add (MemoryStore)

All tests use `MemoryStore` and are placed in the existing `#[cfg(test)]` block.

**Test helper:** `fn make_store_with_root_container(root_embed: Container, full_container: Container, instance_ids: Vec<&str>) -> MemoryStore`
- Creates a MemoryStore with:
  - `manifest.container = Some(root_embed)` (the embed, typically `containerId + identityInstanceId` only)
  - Calls `store.save_container(&full_container)` to persist the container file (so `load_container(full_container.container_id)` succeeds)
  - Adds an instanceIndex entry for each id in `instance_ids`
- When testing ContainerNotFound paths, do NOT call `store.save_container`, so the root container has no file.

**Note on `validate_container_invariants` re-loads:** The existing `validate_container_invariants(store, id)` loads the container from the store and then calls `store.load_manifest()` internally to get the instanceIndex. This results in the manifest being loaded twice (once at line 88, once inside the helper). This is acceptable — both stores (MemoryStore, FileStore) cache or re-read cheaply, and no circular dependency exists. No action needed; documented here for clarity.

**I-82 suppression rule:** A non-identity member in the root container suppresses the I-82 warning if it appears in `rootInstanceIds` of ANY container in containerIndex (regardless of whether it's also in that container's `memberInstanceIds`). The warning means "this member has no section container anchored on it". If the member IS a root of some section container, it has an anchor → no warning. This is the intended behavior and must be tested explicitly.

Tests:

1. `rfc013_i79_missing_container_emits_error` — MemoryStore with manifest where `container: None` → `is_ok()` is false, error message contains "I-79".

2. `rfc013_i80_member_not_in_instance_index_emits_error` — root container with `memberInstanceIds: ["ghost"]`, but "ghost" not in instanceIndex → error contains "I-79/I-80" or similar membership check message.

3. `rfc013_i81_identity_not_in_members_emits_error` — embed has `identityInstanceId: Some("a")`, full container file has `memberInstanceIds: ["b"]` (not "a") → error contains "I-81".

4. `rfc013_i81_identity_in_members_passes` — embed has `identityInstanceId: Some("a")`, full container has `memberInstanceIds: ["a", "b"]`, "a" and "b" in instanceIndex → `is_ok()` is true.

5. `rfc013_i82_section_root_without_container_emits_warning` — root container members `["identity-id", "section-id"]`, embed `identityInstanceId: Some("identity-id")`, containerIndex has one section container whose `rootInstanceIds` does NOT include "section-id" → warning contains "I-82", `is_ok()` still true (warning ≠ error).

6. `rfc013_i82_section_root_with_container_no_warning` — same as 5 but the section container's `rootInstanceIds` DOES include "section-id" → no I-82 warning emitted.

7. `rfc013_i82_suppressed_without_container_index` — root container has members but containerIndex is None → no I-82 warning.

8. `rfc013_i82_suppressed_with_empty_container_index` — containerIndex is `Some(vec![])` (empty) → no I-82 warning (same suppression path as absent containerIndex).

9. `rfc013_i82_no_identity_all_members_checked` — embed has `identityInstanceId: None`, root container members `["section-a", "section-b"]`, containerIndex non-empty but neither member is a root of any container → 2 I-82 warnings emitted (all members are non-identity).

10. `rfc013_root_container_not_in_store_is_graceful` — manifest.container present (containerId = "X") but no container file saved and "X" not in containerIndex → no I-79 error (ContainerNotFound is handled gracefully; I-81/I-82 are skipped). Confirm `identityInstanceId` present in embed does NOT trigger I-81 in this path.

11. `rfc013_cross_store_roundtrip` — cross-store test required by CLAUDE.md Storage Boundary Rules: seed a `MemoryStore` with a valid root container (members in instanceIndex, identityInstanceId in members), serialize to `.srsj` via `crate::repository_portability::export_srsj`, then load and validate via `crate::store::JsonStore`. Confirm `is_ok()` and no RFC-013 diagnostics. This ensures the validate path behaves consistently across store implementations.

#### Acceptance Criteria

- [ ] `validate_repository` emits ERROR for absent `manifest.container` (I-79)
- [ ] `validate_repository` emits ERROR when root container's member/root IDs are not in instanceIndex (I-80 via `validate_container_invariants`)
- [ ] `validate_repository` emits ERROR when `identityInstanceId` is not in root container's members/roots (I-81)
- [ ] `validate_repository` emits WARNING for non-identity members without a matching section container (I-82), suppressed when containerIndex absent/empty
- [ ] All 11 new unit tests pass (10 MemoryStore + 1 cross-store roundtrip)
- [ ] `live_srs_repo_validates_cleanly` still passes (vendored fixture has container with no members — all four invariants trivially pass)
- [ ] `cargo test -p srs-repository` passes with 0 failures
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `rfc013_i79_missing_container_emits_error` — I-79 error fires for absent container
- `rfc013_i80_member_not_in_instance_index_emits_error` — I-80 member resolution error
- `rfc013_i81_identity_not_in_members_emits_error` — I-81 identity pointer error
- `rfc013_i81_identity_in_members_passes` — valid identity pointer does not fire
- `rfc013_i82_section_root_without_container_emits_warning` — I-82 warning fires
- `rfc013_i82_section_root_with_container_no_warning` — I-82 suppressed when member is a root
- `rfc013_i82_suppressed_without_container_index` — I-82 suppressed when containerIndex absent
- `rfc013_i82_suppressed_with_empty_container_index` — I-82 suppressed when containerIndex empty
- `rfc013_i82_no_identity_all_members_checked` — all members checked when identityInstanceId is None
- `rfc013_root_container_not_in_store_is_graceful` — ContainerNotFound handled gracefully; I-81/I-82 skipped
- `rfc013_cross_store_roundtrip` — MemoryStore seed → srsj → JsonStore validate passes

#### Milestone gate

1. Verify all acceptance criteria above are checked.
2. Confirm every test listed exists and passes.
3. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
4. Update the plan checkboxes.
5. Commit: `feat(srs-repository): enforce RFC-013 root container invariants I-79/I-80/I-81/I-82 (#264)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `live_srs_repo_validates_cleanly` passes
- [ ] All 7 RFC-013 unit tests pass
- [ ] All 10 RFC-013 unit tests pass
- [ ] Dogfooding: `srs-gov repo-create` + `srs repo validate` on a fresh repo reports 0 errors

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- **Write scope:** `crates/srs-repository/src/validation.rs` only.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of the phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed without completing the milestone gate.

## Assumptions

- The vendored spec-repo fixture at `tests/fixtures/spec-repo/manifest.json` has `manifest.container` present (it does: `containerId: 4172fada-bc38-5479-ac18-4be3194a68ca`), so `live_srs_repo_validates_cleanly` is unaffected.
- The root container in the vendored fixture has no `memberInstanceIds` or `rootInstanceIds`, so I-80, I-81, I-82 trivially pass for it.
- `srs-gov repo-create` (Gate A, issue #265) already writes a `manifest.container` embed with `containerId` and `identityInstanceId`, and stores the container file in `containers/`, so the srs-gov dogfood scenario validates cleanly.
- No change to `validate_container_invariants` signature — it's used as-is.
