# Plan: Migrate Tier-0 identity note to purpose record + repoint identityInstanceId

## Summary

RFC-018 I-81 (now landing as a validation warning) requires `manifest.container.identityInstanceId` to resolve to a `com.semanticops.core/purpose` Tier-2 Record. Existing repositories — including the `srs/srs` spec repo and the `srs-gov` seed — hold a Tier-0 Note as their identity, so they emit RFC-018 warnings until migrated. This plan implements:

1. A targeted `identityInstanceId` repoint path via `ContainerPatch.identity_instance_id`, removing the only-whole-embed-overwrite limitation in `manifest_service`.
2. A `migrate_identity` service function that promotes a Tier-0 Note (or non-purpose Tier-2 record) to a `com.semanticops.core/purpose` Record, repoints the manifest pointer, adds the record to container membership, and returns a structured result.
3. A `srs repo migrate-identity` CLI command backed by `RepoMigrateIdentityPayload`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| CLI Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Migration logic lives entirely in `srs-repository`; CLI handler is a thin wrapper | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | New `RepoMigrateIdentityPayload` struct in `payload.rs`; `generate-schemas` run after | accepted |
| [ADR-007](../docs/adr/007-file-index-io-ordering.md) | Record file written first; manifest index updated in the same pass as identity repoint (single write gate) | accepted |

**Intentional bypass — core type UUIDs hardcoded (#423 not yet landed):**  
`com.semanticops.core/purpose` is not yet in the installed package because the core-type registry (#423) is not merged. The migration service uses hardcoded type/field UUID constants (same pattern as #424). These are bounded to `migrate_identity_service.rs` via named constants with a comment referencing #423/#135. When #423 lands, the constants will be replaced with the registry call. No new ADR is created here — the governing ADR will be filed by #423 (as noted in the #424 plan).

**`update_container` + manifest sync for `identity_instance_id`:**  
When `ContainerPatch.identity_instance_id` is set, `update_container` additionally checks whether the container being patched is the manifest's root container (by comparing `manifest.container.container_id`). If yes, it updates `manifest.container.identity_instance_id` in the same request. This collapses the old two-step (update container file + overwrite entire `manifest.container` via `set_manifest_root_container`) into a single targeted patch. This is consistent with ADR-010: a service owns the complete invariant-preserving operation.

---

## Contracts

### CLI output contract (ADR-011)

New command: `srs repo migrate-identity` → `RepoMigrateIdentityPayload` added to `payload.rs`.

After adding the struct: `cargo run --bin generate-schemas` → commit `schemas/payload/repo-migrate-identity.json`.

Existing command `container update` gains a new optional input field (`identityInstanceId`) via `ContainerPatch`. Since `ContainerPatch` is a deserialized input struct (not a payload output struct), no schema regeneration is needed for the patch change.

Verification: `cargo test --test payload_contracts` must pass after payload struct addition.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files under `srs/docs/schema/2.0/`. No action required.

---

## Scope

- Add `identity_instance_id: Option<String>` to `ContainerPatch` in `container_service.rs`
- Handle `identity_instance_id` in `update_container`: patch container file + sync `manifest.container` if this is the root container
- New `crates/srs-repository/src/migrate_identity_service.rs` with `migrate_identity` function
- `MigrateIdentityResult` typed output struct (serializable)
- New `RepoMigrateIdentityPayload` in `payload.rs` + generated schema
- `RepoCommand::MigrateIdentity {}` in `commands/mod.rs`
- Handler `cmd_repo_migrate_identity` in `commands/repo.rs`
- Unit tests (MemoryStore) + cross-store roundtrip test
- `migrate_identity_service` exported from crate root

**Out of scope:**

- `root_instance_ids`/`member_instance_ids` in `ContainerPatch` — deferred follow-up (tracked as a new issue after plan review)
- Core-type registry (#423) — migration uses hardcoded constants, bounded bypass
- WASM binding for `migrate_identity` — deferred until #423 lands (otherwise bindings would also need the bypass)
- Automatic migration on `repo validate` — this is a deliberate CLI-only operation per RFC-018 R8
- Migrating the `srs/srs` spec repo and `srs-gov` seed — done in Stage 7.6 (dogfooding), not in code

---

## Phases

### Phase 1: ContainerPatch.identity_instance_id + manifest sync

**Goal:** `update_container` can repoint `identityInstanceId` on the container file and, when the container is the root container, sync `manifest.container.identity_instance_id` in the same request.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/container_service.rs`, add to `ContainerPatch`:
  ```rust
  pub identity_instance_id: Option<String>,
  ```
- [ ] In `update_container` (after existing field patches, before schema validation), handle the new field:
  ```rust
  if let Some(ref v) = patch.identity_instance_id {
      container.identity_instance_id = Some(v.clone());
  }
  ```
  — apply before the `validate_container` call so the new value is validated.
- [ ] After `store.save_container(&container)`, if `patch.identity_instance_id` is Some AND the container is the root container, sync to manifest:
  ```rust
  // Sync identityInstanceId to manifest.container when patching the root container.
  if let Some(ref new_identity_id) = patch.identity_instance_id {
      let mut manifest = store.load_manifest()?;
      let is_root = manifest.container.as_ref()
          .map(|mc| mc.container_id.as_str() == container_id)
          .unwrap_or(false);
      if is_root {
          if let Some(ref mut mc) = manifest.container {
              mc.identity_instance_id = Some(new_identity_id.clone());
          }
          write_manifest(store, &manifest)?;
      }
  }
  ```
  Import `use crate::writer::write_manifest;` at the top of `container_service.rs`.
- [ ] Return the updated container (no change to return type).

#### Acceptance Criteria

- [ ] `ContainerPatch` has `identity_instance_id: Option<String>` field
- [ ] Patching `identity_instance_id` on a non-root container: container file updated, manifest unchanged
- [ ] Patching `identity_instance_id` on the root container: both container file and `manifest.container.identity_instance_id` updated
- [ ] All existing `update_container` tests still pass (no regression)
- [ ] New unit tests pass (see Testing)

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Tests to write in `container_service.rs` `#[cfg(test)]` block:

- `patch_identity_instance_id_on_root_container_syncs_manifest` — creates a root container in a MemoryStore, sets `manifest.container.container_id` to match, calls `update_container` with `identity_instance_id: Some("new-id")`, asserts the returned container has `identity_instance_id == Some("new-id")` AND `manifest.container.identity_instance_id == Some("new-id")`
- `patch_identity_instance_id_on_non_root_container_does_not_touch_manifest` — same setup but uses a different container than the root; asserts manifest.container.identity_instance_id is unchanged

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
git commit -m "feat(container): patch identityInstanceId in update_container + manifest sync (#426)"
```

---

### Phase 2: Migration service

**Goal:** `migrate_identity` service function takes a store, promotes the current Tier-0 identity note to a `com.semanticops.core/purpose` Record, repoints the manifest pointer, adds the record to root container membership, and returns `MigrateIdentityResult`.

**Agent:** Repository Service Worker

#### Core type constants (hardcoded bypass, bounded to this module)

Define at the top of `migrate_identity_service.rs`:

```rust
// Hardcoded pending core-type registry (#423) and canonical spec authoring (#135).
// These values must match what #423 registers. Once that issue lands, replace with
// a registry lookup: core_package::resolve_type("com.semanticops.core", "purpose").
const CORE_PURPOSE_TYPE_ID: &str = "3c000001-0000-4000-a000-000000000001";
const CORE_PURPOSE_TYPE_VERSION: u32 = 1;
const CORE_PURPOSE_TYPE_NAMESPACE: &str = "com.semanticops.core";
const CORE_PURPOSE_TYPE_NAME: &str = "purpose";
// Field UUIDs: canonical values TBD by #423/#135.
const CORE_STATEMENT_FIELD_ID: &str = "fc000001-0000-4000-a000-000000000001";
const CORE_TITLE_FIELD_ID: &str = "fc000002-0000-4000-a000-000000000002";
```

#### Tasks

- [ ] Create `crates/srs-repository/src/migrate_identity_service.rs`.

- [ ] Define `MigrateIdentityResult`:
  ```rust
  #[derive(Debug, Clone, Serialize)]
  #[serde(rename_all = "camelCase")]
  pub struct MigrateIdentityResult {
      pub old_identity_id: String,
      pub old_identity_tier: u8,
      pub new_identity_id: String,
      pub statement: String,
      pub title: Option<String>,
  }
  ```

- [ ] Define private helper `extract_identity_text(store, entry: &InstanceIndexEntry) -> Result<(String, Option<String>), RepositoryError>`:
  - `tier == 0` (Note): load with `loader::load_note(store, entry.path())`. `statement` = all `sections[].content` joined by `"\n"`, trimmed. `title` = `note.title`. If `statement` is empty after trim, fallback to `note.title.clone().unwrap_or_default()`. If still empty, error with `"identity note has no extractable statement"`.
  - `tier == 2` (non-purpose Tier-2): load with `store.load_instance_json(entry.path())`. Try fields `body`, `title`, `name`, `statement` in that order; take the first non-empty string value. `title` = `raw.get("title").and_then(|v| v.as_str()).map(|s| s.to_string())`. Trim statement; if empty, use `entry.instance_id()` as fallback.
  - Other tiers: `Err(RepositoryError::InvalidInput { message: format!("unsupported identity tier {}: only Tier-0 and Tier-2 identities can be migrated", entry.tier()) })`.

- [ ] Implement `pub fn migrate_identity(store: &dyn RepositoryStore) -> Result<MigrateIdentityResult, RepositoryError>`:

  ```
  1. let mut manifest = store.load_manifest()?;
  2. let mc = manifest.container.as_ref().ok_or(InvalidInput "manifest.container is not set")?.clone();
  3. let old_id = mc.identity_instance_id.clone().ok_or(InvalidInput "manifest.container.identityInstanceId is not set")?;
  4. let root_container_id = mc.container_id.clone();
  5. Find identity_entry: manifest.instance_index.iter().find(|e| e.instance_id() == old_id).cloned().ok_or(InvalidInput "identity instance not found in instanceIndex")?
  6. If tier==2: load raw JSON, check typeNamespace/typeName. If already purpose: return Err(InvalidInput "already a com.semanticops.core/purpose record; no migration needed").
  7. let old_tier = entry.tier();
  8. let (statement, title) = extract_identity_text(store, &entry)?;
  9. Mint new_id = writer::new_instance_id().
  10. Build field_values: always include CORE_STATEMENT_FIELD_ID with statement.clone(); if title.is_some() include CORE_TITLE_FIELD_ID.
  11. Build Record { instance_id: new_id, type_id: CORE_PURPOSE_TYPE_ID, type_version: 1, type_namespace: CORE_PURPOSE_TYPE_NAMESPACE, type_name: CORE_PURPOSE_TYPE_NAME, field_values, group_values: None, lifecycle_state: None, tags: None, created_at: Some(now), updated_at: Some(now), extra: HashMap::new() }.
  12. let dir = paths::DEFAULT_RECORD_DIR ("records/tier-2").
  13. store.ensure_instance_dir(dir)?.
  14. let relative_path = format!("{dir}/purpose-{}.json", &new_id[..8]).
  15. Serialize record to JSON value, insert "$schema" key.
  16. store.save_instance_json(&relative_path, &record_json)?.
  17. Upsert InstanceIndexEntry { instance_id: new_id.clone(), tier: 2, path: relative_path, title: None, tags: None } into manifest.instance_index.
  18. Update manifest.container.identity_instance_id = Some(new_id.clone()) (on the same `manifest` already loaded).
  19. writer::write_manifest(store, &manifest)?.
  20. container_service::add_member(store, &root_container_id, &new_id)?.
  21. Return MigrateIdentityResult { old_identity_id: old_id, old_identity_tier: old_tier, new_identity_id: new_id, statement, title }.
  ```

  Steps 17+18+19 are one manifest write. Step 16 (record file) and step 20 (container file) are separate writes. This is consistent with ADR-007's file-then-index ordering.

- [ ] Export from crate: in `crates/srs-repository/src/lib.rs`, add `pub mod migrate_identity_service;`.

#### Acceptance Criteria

- [ ] Migration on a Tier-0 Note identity:
  - New purpose record exists at `records/tier-2/purpose-{id8}.json`
  - Record has `typeNamespace: "com.semanticops.core"`, `typeName: "purpose"`
  - Record has `fieldValues` containing CORE_STATEMENT_FIELD_ID with note body as value
  - Record appears in `manifest.instance_index` with `tier: 2`
  - `manifest.container.identity_instance_id` equals the new record's instanceId
  - New record is a member of root container's `memberInstanceIds`
- [ ] Already-migrated error: calling again on a repo where identity is already a purpose record returns `Err(InvalidInput { "already a com.semanticops.core/purpose record" })`
- [ ] Missing `manifest.container`: returns `Err(InvalidInput "manifest.container is not set")`
- [ ] Missing `identityInstanceId`: returns `Err(InvalidInput "...identityInstanceId is not set")`
- [ ] 7+ unit tests pass (MemoryStore)
- [ ] 1 cross-store roundtrip test passes

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Tests to write in `migrate_identity_service.rs` `#[cfg(test)]` block:

- `migrate_creates_purpose_record` — MemoryStore with a Tier-0 Note as identity; after `migrate_identity`, verify record file exists at correct path and has correct type fields
- `migrate_sets_statement_from_note_sections` — note with two sections; verify statement is joined content
- `migrate_uses_title_from_note` — verify result.title == note.title
- `migrate_updates_manifest_identity_pointer` — verify `manifest.container.identity_instance_id` after migration
- `migrate_adds_record_to_container_members` — verify new record id in container's `memberInstanceIds`
- `migrate_index_entry_has_tier_2` — verify InstanceIndexEntry for new record has tier == 2
- `migrate_errors_if_already_purpose` — calling migrate twice produces InvalidInput error
- `migrate_errors_if_no_identity_pointer` — `identityInstanceId` not set → InvalidInput
- `cross_store_roundtrip` — migrate in MemoryStore, export to .srsj, import to second MemoryStore, verify identity_instance_id matches new record

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
git commit -m "feat(repository): migrate_identity service — Tier-0 note → purpose record (#426)"
```

---

### Phase 3: CLI command + payload

**Goal:** `srs repo migrate-identity` is callable, produces a well-typed payload, and the golden schema file is committed.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-cli/src/payload.rs`, add:
  ```rust
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct RepoMigrateIdentityPayload {
      pub old_identity_id: String,
      pub old_identity_tier: u8,
      pub new_identity_id: String,
      pub statement: String,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub title: Option<String>,
  }
  ```

- [ ] In `crates/srs-cli/src/commands/mod.rs`, add to `RepoCommand`:
  ```rust
  /// Graduate the repository's Tier-0 identity note to a com.semanticops.core/purpose Record
  /// and repoint manifest.container.identityInstanceId.
  #[command(name = "migrate-identity")]
  MigrateIdentity,
  ```

- [ ] In `crates/srs-cli/src/commands/repo.rs`:
  - Add to the `dispatch` match: `RepoCommand::MigrateIdentity => cmd_repo_migrate_identity(ctx),`
  - Add handler:
    ```rust
    fn cmd_repo_migrate_identity(ctx: CliContext) -> Result<String> {
        let result = with_store(&ctx, |store| {
            Ok(srs_repository::migrate_identity_service::migrate_identity(store)?)
        })?;
        output::serialize(
            "repo migrate-identity",
            RepoMigrateIdentityPayload {
                old_identity_id: result.old_identity_id,
                old_identity_tier: result.old_identity_tier,
                new_identity_id: result.new_identity_id,
                statement: result.statement,
                title: result.title,
            },
        )
    }
    ```
  - Add import: `use crate::payload::RepoMigrateIdentityPayload;` (or add to existing import block)
  - Check: `with_store` must be imported (should already be via `use crate::commands::{with_store, ...}`)

- [ ] Run `cargo run --bin generate-schemas` and commit `schemas/payload/repo-migrate-identity.json`.

- [ ] Verify `cargo test --test payload_contracts` passes.

#### Acceptance Criteria

- [ ] `srs repo migrate-identity --repo <path>` produces a JSON envelope with `payload.newIdentityId` and `payload.statement`
- [ ] Handler is ≤15 lines (ADR-010)
- [ ] `schemas/payload/repo-migrate-identity.json` exists and is committed
- [ ] `cargo test --test payload_contracts` passes

#### Testing

```bash
cargo test --test payload_contracts
cargo build --bin srs
cargo clippy -- -D warnings
```

No new test file needed — payload_contracts integration test covers schema correctness.

#### Milestone gate

```bash
cargo build --bin srs
cargo clippy -- -D warnings
cargo test --test payload_contracts
git commit -m "feat(cli): srs repo migrate-identity command + RepoMigrateIdentityPayload (#426)"
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schema changes in this plan)
- [ ] `srs repo migrate-identity --repo <path>` on a legacy repo produces a valid output and `srs repo validate --repo <path>` reports 0 RFC-018 warnings for the identity
- [ ] `migrate_identity` returns a meaningful error when called on an already-migrated repo (not a panic)
- [ ] Cross-store roundtrip test passes

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `#423` (core-type registry) is not yet landed; hardcoded UUID constants are used throughout, bounded to `migrate_identity_service.rs`.
- `#424` (repo create scaffolds purpose record) is not yet landed; the migration service is independent and compatible with it.
- The migration preserves the old identity record (does NOT delete or modify it) — RFC-018 R8 says "MAY be retained".
- The `statement` field uses the full verbatim content of all Note sections joined by newline (RFC-018 R8: "without truncation or summarization").
- `container_service::add_member` is accessible from `migrate_identity_service.rs` — both are in `srs-repository` crate; `add_member` is `pub`.
