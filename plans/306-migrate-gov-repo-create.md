# Plan: Migrate srs-gov repo-create to call init_new_repository service (#306)

## Summary

`create_governance_repository` in `srs-repository` currently duplicates the manifest-stamping
logic that `init_new_repository` (added in #258) already provides: it manually inserts
`repositoryId`, `namespace`, `title`, and stamps `installedAt` on the top-level
`upstreamPackage`. Governance seeds are always RFC-014-migrated before being loaded into a store,
so `upstreamPackage` is at the top level — but `init_new_repository` only knows about the
pre-RFC-014 `meta.upstreamPackage` location. This plan extends `init_new_repository` to handle
both locations, then replaces the duplicated code in `create_governance_repository` with a single
call to the service.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | primary session agent |
| Repository Service Worker | primary session agent |
| Verification | primary session agent (milestone gates) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs required. Existing ADRs govern every decision:

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All business logic in `srs-repository`; `create_governance_repository` remains one service call | accepted |
| [ADR-017](../docs/adr/017-governance-seed.md) | Governance seeds are always RFC-014-migrated before store creation; `init_new_repository` must handle the resulting top-level `upstreamPackage` | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No CLI commands added or changed. `srs-gov repo-create` output is unchanged.
Verification: `cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

No schema files change. `bash scripts/check-schema-sync.sh` requires no action.

---

## Scope

**In scope:**
- `crates/srs-repository/src/repository_lifecycle.rs` — extend `init_new_repository` to stamp
  `installedAt` at the RFC-014 top-level `upstreamPackage` (tried first) or `meta.upstreamPackage`
  (fallback). Add one new test for the RFC-014 path.
- `crates/srs-repository/src/governance_scaffold_service.rs` — replace the manual manifest-patching
  block in `create_governance_repository` with a call to `init_new_repository`. Remove now-unused
  `write_manifest` and `Utc` imports; add `init_new_repository` / `InitNewRepositoryInput` imports.
  Add a test verifying `installedAt` is set after `create_governance_repository`.

**Out of scope:**
- `crates/srs-gov/src/main.rs` — no changes needed; `cmd_repo_create` already calls
  `create_governance_repository` via a single service call.
- CLI command surface, WASM bindings, payload structs.

---

## Phases

### Phase 1: Extend `init_new_repository` to handle RFC-014 stores

**Goal:** `init_new_repository` stamps `installedAt` on whichever location carries `upstreamPackage`
(top-level RFC-014 or `meta.upstreamPackage` pre-RFC-014), and errors if neither is present.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/repository_lifecycle.rs`, replace the `meta.upstreamPackage`
      lookup block (lines currently after `store.save_manifest` but before returning) with logic
      that first tries top-level `upstreamPackage`, then falls back to `meta.upstreamPackage`:

  ```rust
  // Stamp installedAt at whichever location carries upstreamPackage.
  // RFC-014-migrated stores have it at top level; pre-RFC-014 seeds have it under meta.
  if let Some(up) = manifest.extra.get_mut("upstreamPackage").and_then(|v| v.as_object_mut()) {
      up.insert("installedAt".to_string(), Value::String(Utc::now().to_rfc3339()));
  } else {
      let meta_val = manifest.extra.get_mut("meta").ok_or_else(|| {
          RepositoryError::InvalidRepositoryInitialization {
              message: "upstreamPackage is absent — store must be a seed with upstream provenance"
                  .to_string(),
          }
      })?;
      let upstream = meta_val
          .get_mut("upstreamPackage")
          .and_then(|v| v.as_object_mut())
          .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
              message: "meta.upstreamPackage is absent or not an object".to_string(),
          })?;
      upstream.insert("installedAt".to_string(), Value::String(Utc::now().to_rfc3339()));
  }
  ```

- [ ] Add test `init_new_repository_handles_rfc014_top_level_upstream_package` to the existing
      `#[cfg(test)]` block in `repository_lifecycle.rs`. The test builds a `MemoryStore` with
      top-level `upstreamPackage` (RFC-014 format, no `meta.upstreamPackage`), calls
      `init_new_repository`, and asserts:
      - result `repository_id` and `namespace` match input
      - `manifest.extra["upstreamPackage"]["installedAt"]` is a non-empty ISO-8601 string
      - other `upstreamPackage` fields (`packageId`, `namespace`) are preserved

#### Acceptance Criteria

- [ ] All existing `init_new_repository_*` tests still pass (pre-RFC-014 path unchanged)
- [ ] New test `init_new_repository_handles_rfc014_top_level_upstream_package` passes
- [ ] Error path: store with neither location still returns `InvalidRepositoryInitialization`

#### Testing

```bash
cargo test -p srs-repository init_new_repository
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. All tests pass.
2. Clippy clean.
3. Mark checkboxes `[x]`.
4. Commit: `fix(repository): extend init_new_repository to handle RFC-014 upstreamPackage (#306)`

---

### Phase 2: Delegate manifest-stamping in `create_governance_repository`

**Goal:** `create_governance_repository` calls `init_new_repository` for identity and
`installedAt` stamping, removing the duplicate manual manifest-patching code.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/governance_scaffold_service.rs`:
  - Add import: `use crate::repository_lifecycle::{init_new_repository, InitNewRepositoryInput};`
  - Remove imports no longer needed: `use crate::writer::write_manifest;` and `use chrono::Utc;`
  - In `create_governance_repository`, replace the manual manifest block:
    - Remove: `let mut manifest = store.load_manifest()?;` and all `manifest.extra.insert(...)` calls
    - Remove: `write_manifest(store, &manifest)?;`
    - Remove: `let repository_id = input.repository_id.unwrap_or_else(...);`
    - Add:
      ```rust
      let init_result = init_new_repository(
          store,
          InitNewRepositoryInput {
              repository_id: input.repository_id,
              namespace: namespace.clone(),
              title: input.title.clone(),
              description: None,
          },
      )?;
      ```
    - Update the result construction to use `init_result.repository_id` instead of `repository_id`.

- [ ] Add test `create_governance_repository_sets_installed_at` to the `#[cfg(test)]` block in
      `governance_scaffold_service.rs`. Uses `load_seed_store()` (already defined there), calls
      `create_governance_repository`, then re-exports the store as JSON and asserts
      `manifest["upstreamPackage"]["installedAt"]` is a non-empty string.

#### Acceptance Criteria

- [ ] All existing `governance_scaffold_service` tests still pass
- [ ] New test `create_governance_repository_sets_installed_at` passes
- [ ] `create_governance_repository` no longer contains any `manifest.extra.insert(...)` calls
- [ ] `create_governance_repository` no longer imports or calls `write_manifest` or `chrono::Utc`

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. All tests pass.
2. Clippy clean.
3. Mark checkboxes `[x]`.
4. Commit: `refactor(governance-scaffold): delegate manifest stamping to init_new_repository (#306)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `srs-gov repo-create --output /tmp/test.srsj --title "Test Org"` produces a valid .srsj with
      `installedAt` set in `upstreamPackage`

## Coordination Rules

- Agents keep to their write scopes.
- At the end of each phase: verify all acceptance criteria, confirm tests pass, update plan checkboxes, then commit.
- Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- The governance seed asset (`crates/srs-gov/assets/governance-seed.srsj`) is always in pre-RFC-014
  format; `cmd_repo_create` applies `migrate_rfc014` before loading into a store. So after store
  creation, `upstreamPackage` is always at the top level.
- `MemoryStore::empty()` provides a default package (`id: "test-pkg"`, `version: "1.0.0"`) which
  satisfies the `store.load_package()` call in `init_new_repository`.
