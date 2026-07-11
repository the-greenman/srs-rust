# Plan: Fix repository_navigation silently dropping sections when root is also a member

## Summary

`section_containers_by_root()` in `crates/srs-repository/src/repository_navigation_service.rs`
contains an erroneous guard: it excludes a sub-container from the root→container map whenever the
sub-container's own root record appears in that container's `memberInstanceIds`. Because
"root record is also a member" is the natural shape for real governance repositories (the record
that identifies a section is typically also listed as a member of its own container), the guard
silently drops every sub-container in those repos — producing a nav payload where every section
has `sectionContainerId: null` and zero diagnostics. This blocks RFC-013 migration for any
existing governance repo using this shape (srs-web issue closed; confirmed against
`gallery.srsj`). The fix is to drop the guard; the test suite is extended to cover the case.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (main) |
| Repository Service Worker | Claude (main) |
| Verification | Claude (main) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions. This plan fixes a logic error in an existing service function.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Business logic lives in `srs-repository`; fix is entirely inside the service module | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM consumer (`srs-bindings`) calls the same service; the fix propagates automatically; WASM build verified in Final Acceptance | accepted |

_No new ADRs are needed — the fix removes an incorrect predicate; it introduces no new API shape, cross-crate dependency, or architectural constraint._

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. The `repository_navigation` payload struct is unchanged — the
bug caused missing `sectionContainerId` values (null instead of a string); the fix populates
them correctly. The field has always existed in the payload. No schema regeneration needed.

### Entity schema sync (check-schema-sync.sh)

No entity schema files are changed.

---

## Scope

- Remove the `(!root_is_member).then(...)` guard in `section_containers_by_root()` so that every
  sub-container root is unconditionally mapped to its container.
- Add a unit test (`repository_navigation_root_is_member_of_its_own_sub_container`) that
  exercises the failure mode: sub-containers whose `root_instance_ids[0]` also appears in their
  own `member_instance_ids` must resolve `section_container_id` correctly with zero diagnostics.
- Existing tests must all continue to pass.

**Out of scope:**

- Any diagnostic message when a root-is-member shape is detected (it is valid, not an error).
- Any change to the CLI handler or payload structs.
- srs-web changes (unblocked by this fix; tracked separately).

---

## Phases

### Phase 1: Fix `section_containers_by_root` and add regression test

**Goal:** The guard is removed, a new test covers the "root is also a member" shape, and all
existing tests pass.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/repository_navigation_service.rs`, remove the `root_is_member`
  predicate from `section_containers_by_root()`. The `filter_map` closure over `roots` should
  return `Some((root_id, container_id.clone()))` unconditionally (drop the `let root_is_member`
  binding and the `(!root_is_member).then(...)` call).

- [ ] In the same file's `#[cfg(test)]` block, add a test
  `repository_navigation_root_is_member_of_its_own_sub_container` that:
  - Follows the same store-setup pattern as `repository_navigation_returns_identity_and_precedes_ordered_sections`
    (use the existing `record()`, `add_record()`, `empty_package()`, `add_precedes()` helpers).
  - Creates a MemoryStore with:
    - Root container `00000000-0000-4000-8000-00000000a000` with
      `member_instance_ids: Some(vec!["...a100...", "...a200...", "...a300..."])` and
      `root_instance_ids: Some(vec!["...a100..."])`.
    - Sub-container `00000000-0000-4000-8000-00000000b000` with
      `root_instance_ids: Some(vec!["...a200..."])` and
      `member_instance_ids: Some(vec!["...a200..."])` (root record is its only member — exercises the bug).
    - Sub-container `00000000-0000-4000-8000-00000000c000` with
      `root_instance_ids: Some(vec!["...a300..."])` and
      `member_instance_ids: Some(vec!["...a300..."])` (same shape).
    - A `precedes` relation `a200 → a300` via `add_precedes()` to guarantee deterministic section ordering.
    - All three records (`a100`, `a200`, `a300`) registered in the instance index via `add_record()`.
  - Calls `repository_navigation(&store)`.
  - Asserts `sections.len() == 2`.
  - Asserts `sections[0].section_container_id.as_deref() == Some("00000000-0000-4000-8000-00000000b000")`.
  - Asserts `sections[1].section_container_id.as_deref() == Some("00000000-0000-4000-8000-00000000c000")`.
  - Asserts `nav.diagnostics.is_empty()`.

#### Acceptance Criteria

- [ ] `section_containers_by_root()` no longer contains the `root_is_member` predicate.
- [ ] New test `repository_navigation_root_is_member_of_its_own_sub_container` exists and passes.
- [ ] `repository_navigation_returns_identity_and_precedes_ordered_sections` still passes.
- [ ] All other existing tests in this module still pass.
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes.

#### Testing

```bash
cargo test -p srs-repository repository_navigation
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to verify:

- `repository_navigation_root_is_member_of_its_own_sub_container` — proves the bug is fixed
- `repository_navigation_returns_identity_and_precedes_ordered_sections` — regression guard
- `repository_navigation_missing_manifest_container_returns_empty_with_diagnostic` — regression
- `navigation_tier0_note_identity_returns_diagnostic` — regression
- `navigation_tier0_note_identity_no_title_falls_back_to_id` — regression
- `navigation_tier0_identity_and_missing_member_accumulates_both_diagnostics` — regression

#### Milestone gate

1. All acceptance criteria checked above.
2. All six named tests exist and pass.
3. Run:

```bash
cargo test -p srs-repository repository_navigation
cargo clippy -p srs-repository -- -D warnings
```

4. Mark checkboxes `[x]`.
5. Commit:

```bash
git commit -m "fix(nav): section_containers_by_root always maps root→container (#460)"
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (payload contract unaffected — no payload struct modified)
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no schema files changed)
- [ ] New test `repository_navigation_root_is_member_of_its_own_sub_container` passes
- [ ] `cargo build --target wasm32-unknown-unknown -p srs-bindings` succeeds (ADR-013; `srs-bindings` calls `repository_navigation` directly — WASM consumer of this fix)

## Coordination Rules

- Single-agent implementation (small bug fix).
- Lead Integrator owns the fix; no concurrent writers.
- Commit at the Phase 1 milestone gate before proceeding to Stage 6.

## Assumptions

- The `root_is_member` guard was never intentional — the issue body describes it as a bug and the
  only test covering `section_containers_by_root` uses `member_instance_ids: None`, leaving the
  guard untested.
- "Root is also a member" is a valid, common repo shape; no diagnostic is needed.
- No `srs-web` changes are in scope for this PR (the web fix is already tracked).
