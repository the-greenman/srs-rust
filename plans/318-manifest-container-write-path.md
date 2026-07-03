# Plan: manifest.container write path (srs-repository service + CLI command)

> Issue #318 — srs-gov cmd_repo_create raw-JSON-patches `manifest.container`
> because no service or CLI command exists to write it.

## Summary

`srs-gov repo-create` currently writes `manifest.container` by deserializing the `.srsj` file to a `serde_json::Value`, patching `["manifest"]["container"]` directly, and writing the file back. This violates the capability-layering rule (ADR-001, ADR-010): semantics belong in `srs-repository`, not in a leaf client. The fix is a three-layer change:
1. Add `set_manifest_root_container` service in `srs-repository::manifest_service`.
2. Expose it via `srs repo set-root-container` CLI command with a typed payload (ADR-011).
3. Replace the raw-patch block in `srs-gov::cmd_repo_create` with a call to the new command.

No spec change is needed — `manifest.container` (RFC-013) already exists; this is purely an implementation write-path.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | `agents.md#repository-service-worker` |
| CLI Worker | `agents.md#cli-worker` |
| Verification Agent | `agents.md#verification-agent` |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Service logic in `srs-repository`, not in leaf clients | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service takes typed input struct, returns typed result struct | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | CLI output is a named struct in `payload.rs`; golden schema committed | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | Every write service gets a WASM binding; deferred here — no web consumer yet | deferred |
| [ADR-015](../docs/adr/015-wasm-write-surface.md) | Write surface in WASM mirrors CLI typed structs; deferred here — no web consumer yet | deferred |
| [ADR-017](../docs/adr/017-deterministic-serialization.md) | Manifest byte layout changes must be explicitly noted; `title: ""` will appear in serialized output (see Phase 1 AC) | accepted |

No new ADRs required — this plan is a clean application of ADR-001/010/011.

Decision: **Service in `manifest_service.rs`** (not `repository_lifecycle.rs`). Rationale: writing `manifest.container` is a manifest-level field mutation, following the same pattern as `add_package_ref`/`add_declared_extension` in `manifest_service.rs`. It is not a creation/initialization operation.

Decision: **CLI flags not stdin JSON**. `srs repo set-root-container --container-id <id> --identity-instance-id <id>` — both arguments are flat UUIDs; flags are simpler and match the `container members add` pattern.

---

## Contracts

### CLI output contract (ADR-011)

New CLI command `srs repo set-root-container` — adds `RepoSetRootContainerPayload { container_id: String, identity_instance_id: String }` to `crates/srs-cli/src/payload.rs`. Run `cargo run --bin generate-schemas` and commit `schemas/payload/RepoSetRootContainerPayload.json`.

### Entity schema sync

No changes to entity schemas under `srs/docs/schema/2.0/`. No sync needed.

---

## Scope

- `crates/srs-repository/src/error.rs` — add `InvalidInput { message: String }` variant to `RepositoryError`
- `crates/srs-repository/src/manifest_service.rs` — new service function + input/result types + MemoryStore test
- `crates/srs-cli/src/payload.rs` — new `RepoSetRootContainerPayload`
- `crates/srs-cli/src/commands/mod.rs` — new `SetRootContainer` variant on `RepoCommand`
- `crates/srs-cli/src/commands/repo.rs` — new handler + dispatch arm
- `crates/srs-cli/schemas/payload/RepoSetRootContainerPayload.json` — generated
- `crates/srs-cli/tests/repo_set_root_container.rs` — integration test
- `crates/srs-gov/src/main.rs` — remove raw JSON patch; add `srs_set_manifest_root_container` helper; update stale comment

**Out of scope:**
- WASM binding for this service (deferred; no web consumer yet — ADR-013/015)
- Container existence validation in the service (the manifest embed is a pointer; existence is checked by navigation service at read time, same as today)
- Migrating existing repos that have the embed without a `title` field (the navigation service works whether or not `title` is present)

---

## Phases

### Phase 1: Service function in `srs-repository`

**Goal:** `set_manifest_root_container` exists in `manifest_service.rs`, takes a typed input, loads and mutates the manifest via the store, and returns a typed result; MemoryStore test passes.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/error.rs`, add to `RepositoryError` enum:
  ```rust
  InvalidInput { message: String },
  ```
  and a matching arm in the hand-written `Display` / `PartialEq` impls.

- [ ] In `crates/srs-repository/src/manifest_service.rs`, add:

  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct SetManifestRootContainerInput {
      pub container_id: String,
      pub identity_instance_id: String,
  }

  #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct SetManifestRootContainerResult {
      pub container_id: String,
      pub identity_instance_id: String,
  }

  pub fn set_manifest_root_container(
      store: &dyn RepositoryStore,
      input: SetManifestRootContainerInput,
  ) -> Result<SetManifestRootContainerResult, RepositoryError> {
      // validate
      // load manifest
      // set manifest.container (Container with container_id, identity_instance_id)
      // save manifest
      // return result
  }
  ```

- [ ] Validate both fields are non-empty; return `RepositoryError::InvalidInput { message: ... }` with a clear message if either is empty.
- [ ] Build the manifest embed as:
  ```rust
  srs_core::types::container::Container {
      container_id: input.container_id.clone(),
      title: String::new(),
      identity_instance_id: Some(input.identity_instance_id.clone()),
      namespace: None,
      name: None,
      description: None,
      container_type: None,
      root_instance_ids: None,
      member_instance_ids: None,
      tags: None,
      created_at: None,
      updated_at: None,
      meta: None,
      extra: std::collections::HashMap::new(),
  }
  ```
- [ ] Import types directly in `repo.rs` via `use srs_repository::manifest_service::{SetManifestRootContainerInput, set_manifest_root_container}` — no re-export needed; follow existing import pattern in other CLI handlers.

#### Acceptance Criteria

- [ ] `set_manifest_root_container` with valid IDs writes `manifest.container` (load back and verify `container_id` + `identity_instance_id` match)
- [ ] `set_manifest_root_container` with empty `container_id` returns `Err(InvalidInput { ... })`
- [ ] `set_manifest_root_container` with empty `identity_instance_id` returns `Err(InvalidInput { ... })`
- [ ] No path strings (`manifest.json`, `containers/`) appear in service code — all I/O through store
- [ ] Serialized manifest contains `"title": ""` in the container embed (ADR-017: explicit format note — harmless, navigation service ignores it)

#### Testing

```bash
cargo test -p srs-repository set_manifest_root_container
```

Specific tests (all in `manifest_service.rs` `#[cfg(test)]` using `MemoryStore`):
- `set_manifest_root_container_writes_and_reads_back` — write, reload manifest, assert fields match
- `set_manifest_root_container_empty_container_id_returns_error`
- `set_manifest_root_container_empty_identity_id_returns_error`

#### Milestone gate

1. All three tests pass.
2. `cargo test -p srs-repository` passes.
3. `cargo clippy -p srs-repository -- -D warnings` passes.
4. Mark task checkboxes `[x]`, commit: `feat(srs-repository): add set_manifest_root_container service (#318)`.

---

### Phase 2: CLI command `srs repo set-root-container`

**Goal:** `srs repo set-root-container --container-id X --identity-instance-id Y` executes and returns a JSON envelope with `RepoSetRootContainerPayload`.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-cli/src/payload.rs`, add:
  ```rust
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct RepoSetRootContainerPayload {
      pub container_id: String,
      pub identity_instance_id: String,
  }
  ```

- [ ] In `crates/srs-cli/src/commands/mod.rs`, add to `RepoCommand` enum:
  ```rust
  /// Set the manifest root container embed (containerId + identityInstanceId).
  /// Used by repo-create tooling — writes manifest.container so the navigation
  /// service can find the repository's structural root.
  #[command(name = "set-root-container")]
  SetRootContainer {
      #[arg(long = "container-id")]
      container_id: String,
      #[arg(long = "identity-instance-id")]
      identity_instance_id: String,
  },
  ```

- [ ] In `crates/srs-cli/src/commands/repo.rs`:
  - Add dispatch arm in `dispatch()`: `RepoCommand::SetRootContainer { container_id, identity_instance_id } => cmd_repo_set_root_container(ctx, container_id, identity_instance_id)`
  - Add handler (use `?` propagation — canonical pattern, no `match` + `output::err`):
    ```rust
    fn cmd_repo_set_root_container(
        ctx: CliContext,
        container_id: String,
        identity_instance_id: String,
    ) -> Result<String> {
        let input = SetManifestRootContainerInput { container_id, identity_instance_id };
        let result = with_store(&ctx, |store| Ok(set_manifest_root_container(store, input)?))?;
        output::serialize(
            "repo set-root-container",
            RepoSetRootContainerPayload {
                container_id: result.container_id,
                identity_instance_id: result.identity_instance_id,
            },
        )
    }
    ```
  - Add imports: `use srs_repository::manifest_service::{SetManifestRootContainerInput, set_manifest_root_container}` and `use crate::payload::RepoSetRootContainerPayload`.

- [ ] Run `cargo run --bin generate-schemas` — commit the generated `crates/srs-cli/schemas/payload/RepoSetRootContainerPayload.json`.

- [ ] Add integration test at `crates/srs-cli/tests/repo_set_root_container.rs`:
  ```rust
  // test: set_root_container_writes_manifest
  // Create a temp FileStore repo, call set_manifest_root_container via store,
  // reload manifest, assert container_id and identity_instance_id match.
  ```

#### Acceptance Criteria

- [ ] `srs repo set-root-container --container-id A --identity-instance-id B` on a FileStore repo emits `{"ok": true, "command": "repo set-root-container", "payload": {"containerId": "A", "identityInstanceId": "B"}}`
- [ ] Reloading the manifest after the command shows `container.containerId == "A"` and `container.identityInstanceId == "B"`
- [ ] `cargo test --test payload_contracts` passes (golden schema committed)
- [ ] `cargo test --test repo_set_root_container` passes

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo test --test repo_set_root_container
```

Specific tests:
- `set_root_container_writes_manifest` in `crates/srs-cli/tests/repo_set_root_container.rs` — create FileStore repo, call service, reload manifest, assert container fields set

#### Milestone gate

1. `cargo build --bin srs` succeeds.
2. `cargo test --test repo_set_root_container` passes.
3. `cargo test --test payload_contracts` passes.
4. `cargo clippy -p srs-cli -- -D warnings` passes.
5. Mark checkboxes, commit: `feat(srs-cli): add repo set-root-container command (#318)`.

---

### Phase 3: srs-gov migration

**Goal:** `srs-gov::cmd_repo_create` no longer contains raw `repo_json["manifest"]["container"] = ...` JSON mutation; it calls the new `srs repo set-root-container` CLI instead.

**Agent:** Repository Service Worker (srs-gov crate)

#### Tasks

- [ ] In `crates/srs-gov/src/main.rs`, add helper after `srs_roots_add`:
  ```rust
  fn srs_set_manifest_root_container(
      repo: &str,
      container_id: &str,
      identity_instance_id: &str,
  ) -> Result<()> {
      srs::run_srs_write(
          &[
              "repo", "set-root-container",
              "--container-id", container_id,
              "--identity-instance-id", identity_instance_id,
          ],
          repo,
          "",
      )?;
      Ok(())
  }
  ```

- [ ] Replace the raw JSON patch block in `cmd_repo_create` (lines ~739-748 of `main.rs`):
  ```rust
  // DELETE: re-read file, patch repo_json["manifest"]["container"], write back
  let mut repo_json: serde_json::Value = ...
  repo_json["manifest"]["container"] = serde_json::json!({...});
  std::fs::write(out_path, ...)?;
  ```
  WITH:
  ```rust
  srs_set_manifest_root_container(output, &root_container_id, &intent_id)?;
  ```
  (Note: `output` is the repo path string in `cmd_repo_create`; verify the correct parameter name at the call site.)

- [ ] Remove the stale comment referencing `srs-rust#263` — the issue is now resolved.

- [ ] Remove any now-unused imports (e.g. `serde_json::json!` if only used for that block; `serde_json::from_str`/`std::fs::write` if only used for that block — check for other uses first).

#### Acceptance Criteria

- [ ] `cmd_repo_create` contains no `repo_json["manifest"]["container"] = ...` assignment.
- [ ] `srs-gov repo-create` still creates a repo where `srs repo navigation` succeeds (reads `manifest.container` correctly).
- [ ] `srs repo validate --repo <created-repo>` exits with 0 diagnostics.
- [ ] Existing test `repo_create_navigation_works` passes without modification.

#### Testing

```bash
cargo test -p srs-gov
```

Specific tests:
- `repo_create_navigation_works` — already exists; must pass without modification.

#### Milestone gate

1. `cargo test -p srs-gov` passes (including `repo_create_navigation_works`).
2. `cargo clippy -p srs-gov -- -D warnings` passes.
3. Mark checkboxes, commit: `feat(srs-gov): remove raw manifest.container JSON patch, use CLI (#318)`.

---

## Final Acceptance

- [ ] `cargo test` (workspace) passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `cargo test --test repo_set_root_container` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schema changes expected)
- [ ] `srs-gov::cmd_repo_create` contains no `repo_json["manifest"]["container"] = ...` raw JSON mutation
- [ ] `srs repo set-root-container --container-id X --identity-instance-id Y` emits a valid JSON envelope
- [ ] `srs repo navigation` works on a repo created by the updated `srs-gov repo-create`
- [ ] `srs repo validate` reports 0 diagnostics on such a repo

## Coordination Rules

- Repository Service Worker writes to `crates/srs-repository/` and `crates/srs-gov/` only.
- CLI Worker writes to `crates/srs-cli/` only.
- Lead Integrator reviews naming consistency (`set_manifest_root_container` vs `set_repository_root_container`) before Phase 2 starts; name chosen: `set_manifest_root_container` (consistent with manifest-level helpers in `manifest_service.rs`).
- Commit at the end of each phase, not per-task.

## Assumptions

- `srs-gov` calls `srs` as a subprocess (via `srs::run_srs_write`); this is already established for all other write operations (`srs_roots_add`, `srs_containers_create`, etc.).
- The `out_path` variable in `cmd_repo_create` refers to the `.srsj` seed path, while `output` (or equivalent) is the repo path passed to `run_srs_write`. Verify the correct repo-path variable name before Phase 3.
- No WASM binding is needed for this service in this plan (no web consumer exists — ADR-013/015 deferred).
- The manifest embed format change (`title: ""` will appear in serialized output) is acceptable — it's a default value, not present in the current raw-JSON write, but harmless because navigation ignores the embed title (ADR-017).
