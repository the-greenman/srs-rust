# Plan: Wrap migrate-packet two-service-call handler to satisfy ADR-010

## Summary

`cmd_migrate_packet` in `crates/srs-cli/src/commands/migrate.rs` makes two service calls inside a single `with_store` closure — `load_analysis_profile` followed by `build_migration_packet` — violating ADR-010's one-service-call-per-handler rule. The fix is a thin wrapper function `build_migration_packet_for_profile` in `srs-repository` that absorbs the two-step orchestration, allowing the handler to make a single service call. No CLI output, payload struct, or entity schema changes are required.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Repository Service Worker | Claude (this session) |
| CLI Worker | Claude (this session) |
| Verification | Architecture Reviewer + Verification Agent (Stage 7) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan implements ADR-010 (service boundary contract). The wrapper function follows the established orchestration pattern: multi-step operations belong in the service, not the handler.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Multi-step service orchestration belongs in `srs-repository`; CLI handler calls one function | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Pre-existing violation: handler serialises `MigrationPacket` via `serde_json::to_value` rather than a payload struct in `payload.rs`. This plan does not change that. Deferred — tracked in #601 | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed commands. The `migrate packet` command output shape is unchanged — `build_migration_packet_for_profile` returns the same `MigrationPacket` struct. No action required; golden schemas stay as-is. `cargo test --test payload_contracts` must continue to pass.

### Entity schema sync (check-schema-sync.sh)

No entity schema files are added or modified. No action required.

---

## Scope

- Add `pub fn build_migration_packet_for_profile(store: &dyn RepositoryStore, profile_id: &str) -> Result<MigrationPacket, RepositoryError>` to `crates/srs-repository/src/analysis.rs`.
- Update `cmd_migrate_packet` in `crates/srs-cli/src/commands/migrate.rs` to call only `build_migration_packet_for_profile`, removing the direct calls to `load_analysis_profile` and `build_migration_packet` from the handler.
- Add at least one unit test for `build_migration_packet_for_profile` in `srs-repository` exercising the happy path.

**Out of scope:**
- Changing the public signatures of `build_migration_packet` or `load_analysis_profile` (they remain available for direct callers).
- Any WASM binding changes.
- Any changes to the `--foundation` flag validation logic.
- Any other handlers or ADR-010 violations beyond this one handler.

---

## Phases

### Phase 1: Add wrapper service function and update handler

**Goal:** `cmd_migrate_packet` makes exactly one service call and `cargo test -p srs-repository` plus `cargo clippy -- -D warnings` pass.

**Agent:** Repository Service Worker + CLI Worker (single session, sequential tasks)

#### Tasks

- [x] In `crates/srs-repository/src/analysis.rs`, add the following public function after `load_analysis_profile` (around line 476):
  ```rust
  pub fn build_migration_packet_for_profile(
      store: &dyn RepositoryStore,
      profile_id: &str,
  ) -> Result<MigrationPacket, RepositoryError> {
      let profile = load_analysis_profile(store, profile_id)?;
      build_migration_packet(store, &profile.profile_id, &profile.include_tags)
  }
  ```
- [x] In `crates/srs-cli/src/commands/migrate.rs`, update the import line to include `build_migration_packet_for_profile` and remove `build_migration_packet` and `load_analysis_profile` from the import:
  ```rust
  use srs_repository::analysis::build_migration_packet_for_profile;
  ```
- [x] Update `cmd_migrate_packet` in `crates/srs-cli/src/commands/migrate.rs` to:
  ```rust
  fn cmd_migrate_packet(ctx: CliContext, foundation: bool) -> Result<String> {
      if !foundation {
          return Err(anyhow!(
              "migrate packet currently requires the --foundation profile"
          ));
      }
      let packet = with_store(&ctx, |store| {
          Ok(build_migration_packet_for_profile(store, "foundation")?)
      })?;
      Ok(output::ok("migrate packet", serde_json::to_value(packet)?))
  }
  ```
- [x] Add a unit test in `crates/srs-repository/src/analysis.rs` (in the existing `#[cfg(test)]` block) that exercises `build_migration_packet_for_profile` against a `MemoryStore` with a profile fixture. Test name: `test_build_migration_packet_for_profile_foundation`.

#### Acceptance Criteria

- [x] `cmd_migrate_packet` body contains exactly one service call inside `with_store`.
- [x] `load_analysis_profile` and `build_migration_packet` are no longer imported or called from `crates/srs-cli/src/commands/migrate.rs`.
- [x] `build_migration_packet_for_profile` is exported from `crates/srs-repository/src/analysis.rs`.
- [x] `test_build_migration_packet_for_profile_foundation` exists and passes.
- [x] `cargo test -p srs-repository` passes with no failures.
- [x] `cargo test -p srs` passes with no failures.
- [x] `cargo clippy -- -D warnings` passes with no warnings.

#### Testing

```bash
cargo test -p srs-repository
cargo test -p srs-cli
cargo clippy -- -D warnings
cargo test --test payload_contracts
```

Specific tests:
- `test_build_migration_packet_for_profile_foundation` — proves the wrapper loads the profile and calls `build_migration_packet` end-to-end via `MemoryStore`.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm `test_build_migration_packet_for_profile_foundation` exists and passes.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo test -p srs-cli
   cargo clippy -- -D warnings
   cargo test --test payload_contracts
   ```
4. Mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit: `refactor(analysis): wrap migrate-packet to single service call (ADR-010) (#598)`.

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] `cargo test --test payload_contracts` passes
- [x] `cmd_migrate_packet` contains exactly one service call inside `with_store`
- [x] `build_migration_packet_for_profile` is exported from `crates/srs-repository/src/analysis.rs`
- [x] `test_build_migration_packet_for_profile_foundation` passes

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- No new payload structs. No schema regeneration needed.

## Assumptions

- The existing test at `analysis.rs:872` calls `build_migration_packet` directly with `signal_tags` — it does NOT use `load_analysis_profile` and does NOT seed a `.srs/profiles/` file in `fixture_store()`. The new test must seed the profile via `store.save_text_file(".srs/profiles/foundation.json", ...)` before calling `build_migration_packet_for_profile`.
- `MemoryStore::save_text_file` (at `store.rs:3012–3020`) stores arbitrary text by relative path key; `MemoryStore::load_text_file` retrieves by the same key. Both are available without any setup.
- The wrapper `build_migration_packet_for_profile` takes a bare `&str` profile_id, consistent with `load_analysis_profile`'s established module precedent. A typed input struct is the ADR-010 ideal but is not enforced here to match the existing module pattern. A future ADR-010 cleanup may add the struct.
- The new wrapper does not need a WASM binding: this is a refactor (consolidating existing orchestration), not a new capability. `build_migration_packet` was already absent from `srs-bindings` before this change. The capability-layering checklist does not apply to internal orchestration refactors.
