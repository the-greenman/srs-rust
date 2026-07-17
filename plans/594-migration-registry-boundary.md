# Plan: Migration Registry Boundary — migrate_rfc014 and migrate packet

## Summary

Issue #461 created the `MIGRATIONS` static registry for in-repo post-load migrations and deferred registering `migrate_rfc014` and `migrate packet` to #594. This plan investigates whether those two operations can be refactored to accept `&dyn RepositoryStore` and, if not, documents the architectural boundary so future implementers don't attempt to register them again. After investigation, neither operation qualifies: `migrate_rfc014` is a pre-load string transformer (the store doesn't exist yet when it runs), and `migrate packet` is a read-only analysis/export tool with no idempotency semantics. The deliverable is boundary documentation and clarifying code comments — no entries are added to `MIGRATIONS`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-032](../docs/adr/032-migration-registry-fn-pointer-pattern.md) | Update Neutral section: `migrate_rfc014` and `migrate packet` are definitively NOT registry candidates (investigation #594) | accepted |

No new ADR is needed — this plan clarifies the scope of ADR-032's registry model, not a new architectural choice.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. No payload structs added or modified. Golden schemas stay as-is.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files. No action required.

---

## Scope

- Update `docs/adr/032-migration-registry-fn-pointer-pattern.md` Neutral section to replace the forward-looking "can be added" note with a definitive boundary statement explaining why each operation is not registry-eligible, citing the investigation in #594.
- Add a module-level doc comment to `crates/srs-repository/src/srsj_migration_service.rs` explaining that this module's operations are pre-load and intentionally outside the `MIGRATIONS` registry.
- Add a clarifying comment to `crates/srs-repository/src/analysis.rs` near `build_migration_packet` explaining that a `MigrationPacket` is a read-only analysis output, not a registry-eligible migration operation.
- Add a corresponding comment to `crates/srs-cli/src/commands/migrate.rs` noting the `migrate packet` command is analysis, not an upgrade migration.

**Out of scope:**

- Adding any entry to `MIGRATIONS` in `migration_registry_service.rs` (the investigation concludes neither qualifies).
- Refactoring `migrate_rfc014` to work post-load (doing so would break the SRSJ loading model and add noise to the registry — `status_fn` would always return `AlreadyApplied`).
- Renaming or restructuring the `srs migrate packet` CLI subcommand (that is a separate UX concern).

---

## Phases

### Phase 1: Boundary documentation

**Goal:** All three files carry accurate, precise comments and ADR-032 reflects the confirmed boundary; no functional code changes.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `docs/adr/032-migration-registry-fn-pointer-pattern.md`, replace the Neutral bullet that says "further migrations (`migrate_rfc014`, `migrate packet`) … can be added as registry entries in future plans" with a statement that: (a) the registry scope is limited to post-load stateful migrations accepting `&dyn RepositoryStore` with meaningful Needed/AlreadyApplied/NotApplicable semantics; (b) `migrate_rfc014` is pre-load (operates on raw `.srsj` strings before any store is constructed — status would always be `AlreadyApplied` once a store exists); (c) `migrate packet` is a read-only analysis export with no idempotency concept and is not a migration.
- [ ] In `crates/srs-repository/src/srsj_migration_service.rs`, add a module-level doc comment (above the first `use` statement) that states: this module provides pre-load bundle-format migrations operating on raw `.srsj` JSON strings; they run before a `RepositoryStore` is constructed and are not registered in the `MIGRATIONS` static (see `migration_registry_service.rs` and ADR-032).
- [ ] In `crates/srs-repository/src/analysis.rs`, add a doc comment on `build_migration_packet` (directly above its `pub fn` signature) that states: this function assembles a read-only analysis/handoff packet for external AI migration tooling; it does not modify the repository and has no idempotency status — it is not a candidate for the `MIGRATIONS` registry.
- [ ] In `crates/srs-cli/src/commands/migrate.rs`, add a single-line comment at the top of `cmd_migrate_packet` (or the module) clarifying that `migrate packet` is a read-only analysis command, not an upgrade migration, and does not correspond to a `MIGRATIONS` registry entry.

#### Acceptance Criteria

- [ ] `docs/adr/032-migration-registry-fn-pointer-pattern.md` no longer says `migrate_rfc014` and `migrate packet` "can be added as registry entries in future plans" — that phrase is replaced with the definitive boundary statement.
- [ ] ADR-032 names both the pre-load category and the analysis-tool category as explicit non-registry categories with rationale.
- [ ] `srsj_migration_service.rs` module-level doc comment references ADR-032 and explains the pre-load boundary.
- [ ] `analysis.rs` `build_migration_packet` doc comment explains the read-only/no-idempotency boundary.
- [ ] `migrate.rs` carries a clarifying comment on the `migrate packet` command.
- [ ] No changes to `MIGRATIONS` in `migration_registry_service.rs`.
- [ ] All existing tests continue to pass.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
cargo test --test payload_contracts
```

Specific tests to verify (no new tests needed — the change is documentation only, with no functional change):

- All existing `migration_registry_service.rs` tests must still pass unchanged.
- All existing `srsj_migration_service.rs` tests must still pass unchanged.
- `cargo test --test payload_contracts` must pass (payload.rs not touched).

#### Milestone gate

1. Verify all acceptance criteria above are checked.
2. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
cargo test --test payload_contracts
```

3. Mark completed task and acceptance-criteria checkboxes `[x]`.
4. Commit:

```bash
git commit -m "docs: document migration registry boundary — migrate_rfc014 and migrate packet are not registry candidates (#594)"
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] ADR-032 Neutral section contains the definitive boundary statement (no "can be added in future" for these two operations)
- [ ] `migrate_rfc014` pre-load boundary is documented in `srsj_migration_service.rs`
- [ ] `build_migration_packet` read-only boundary is documented in `analysis.rs`
- [ ] No entries added to `MIGRATIONS` in `migration_registry_service.rs`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `MIGRATIONS` registry (from #461) is stable on main and no concurrent changes are in flight.
- No new migration IDs need to be assigned for the two operations — the conclusion is they are not registry entries.
- Pre-existing tests for `srsj_migration_service.rs` and `migration_registry_service.rs` remain the correctness evidence; no new tests are needed for a documentation-only change.
