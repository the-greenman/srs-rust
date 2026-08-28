# Plan: Cross-store test coverage for lifecycle retrofit and I-66 container traversal

> Save this file to `plans/885-cross-store-coverage-retrofit-i66.md` before assigning agents.

## Summary

Two service features merged this week — `transition_record_lifecycle`'s retrofit-entry branch (#881, closing #880) and `container_service`'s I-66 condition-3 transitive `contains` traversal (#876) — are tested only against `MemoryStore`, with no `FileStore` coverage. CLAUDE.md's Storage Boundary Rules require "at least one cross-store roundtrip test" for new service features. This plan adds the missing coverage, no behavior change.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Repository Worker | Claude (this session) |
| Verification | Claude (this session) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan implements the existing Storage Boundary Rule (CLAUDE.md) and follows the established cross-store roundtrip pattern already in `record_store.rs` (`rfc022_fulfillment_roundtrip_stores`, via `crate::repository_portability::copy_repository`).

| ADR | Decision | Status |
|---|---|---|
| n/a | Follows CLAUDE.md Storage Boundary Rules ("New service features need at least one cross-store roundtrip test") | accepted (pre-existing) |

---

## Contracts

### CLI output contract (ADR-011)

No new/changed commands. No action required.

### Entity schema sync (check-schema-sync.sh)

No entity schemas touched. No action required.

---

## Scope

- Add a `FileStore`-backed roundtrip test for `transition_record_lifecycle`'s retrofit-entry branch in `crates/srs-repository/src/record_store.rs`, covering the same repro as `retrofit_880_direct_entry_to_reachable_non_initial_state_ok`: a record created before its Type carried a `lifecycleRef` (no prior `lifecycleState`), entering a state reachable-but-not-initial from the Lifecycle's `initialState`, verified after a memory→file `copy_repository` roundtrip.
- Add a `FileStore`-backed roundtrip test for the I-66 condition-3 transitive `contains` traversal in `crates/srs-repository/src/container_service.rs`, covering the same scenario as `member_ids_includes_transitive_contains_from_roots` (a `contains`-only member reachable purely by traversal from a root, absent from both `rootInstanceIds` and `memberInstanceIds`), verified after a memory→file `copy_repository` roundtrip.
- Both new tests use the existing `crate::repository_portability::copy_repository(&store, &file_store)` helper already used by `rfc022_fulfillment_roundtrip_stores` — no new test infrastructure.

**Out of scope:**
- Any behavior change to `transition_record_lifecycle` or `member_ids`/`list_members`.
- CLI-level (`srs record transition`, `srs find --container`) integration tests — the `FileStore` roundtrip test satisfies the Storage Boundary Rule at the service layer, matching the precedent set by `rfc022_fulfillment_roundtrip_stores`.
- Any other test-parity gaps outside these two features.

---

## Phases

### Phase 1: Add cross-store coverage

**Goal:** Both service features have a passing `FileStore`-backed roundtrip test alongside their existing `MemoryStore` coverage.

**Agent:** Repository Worker

#### Tasks

- [ ] In `crates/srs-repository/src/record_store.rs`, add `retrofit_880_direct_entry_to_reachable_non_initial_state_roundtrips_via_filestore`: build the same relational-state fixture as `retrofit_880_direct_entry_to_reachable_non_initial_state_ok` on a `MemoryStore`, copy it to a `FileStore` via `crate::repository_portability::copy_repository`, then call `transition_record_lifecycle` against the `FileStore` and assert the same success as the memory-store test.
- [ ] In `crates/srs-repository/src/container_service.rs`, add `member_ids_includes_transitive_contains_from_roots_roundtrips_via_filestore`: build the same fixture as `member_ids_includes_transitive_contains_from_roots` on a `MemoryStore`, copy it to a `FileStore` via `crate::repository_portability::copy_repository`, then call `member_ids`/`list_members` against the `FileStore` and assert the same transitive-membership result.

#### Acceptance Criteria

- [ ] Both new tests exist and pass.
- [ ] No existing test's behavior or assertions changed.
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes.

#### Testing

```bash
cargo test -p srs-repository retrofit_880_direct_entry_to_reachable_non_initial_state_roundtrips_via_filestore
cargo test -p srs-repository member_ids_includes_transitive_contains_from_roots_roundtrips_via_filestore
cargo test -p srs-repository
```

Specific tests to write:

- `retrofit_880_direct_entry_to_reachable_non_initial_state_roundtrips_via_filestore` — proves the retrofit-entry lifecycle transition works identically when the record is persisted to and loaded from disk, not just in memory.
- `member_ids_includes_transitive_contains_from_roots_roundtrips_via_filestore` — proves the I-66 condition-3 transitive `contains` traversal works identically against a `FileStore`.

#### Milestone gate

1. Verify both acceptance-criteria checkboxes above.
2. Confirm both named tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Mark task/acceptance checkboxes `[x]` in this file.
5. Commit: `git commit`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass) — no CLI surface touched
- [ ] `cargo test --test payload_contracts` passes — no payload structs changed, N/A
- [ ] `bash scripts/check-schema-sync.sh` exits 0 — no entity schemas changed, N/A
- [ ] Both new `FileStore`-backed tests exist, are named per the plan, and pass
- [ ] No behavior change to `transition_record_lifecycle`, `member_ids`, or `list_members`

## Coordination Rules

- Single-session, single-worker plan — no cross-agent handoff needed.
- Milestone gate must pass before committing.

## Assumptions

- The existing `crate::repository_portability::copy_repository` helper correctly round-trips a `MemoryStore`'s full relational/container state into a `FileStore` (already proven by `rfc022_fulfillment_roundtrip_stores`) — no new portability code needed.
- No CLI-level integration test is required to satisfy CLAUDE.md's cross-store rule; the existing precedent (`rfc022_fulfillment_roundtrip_stores`) satisfies it at the service-test layer.
