# Plan: Fix clippy lints blocking pre-commit hook (#553)

## Summary

The pre-commit hook runs `cargo clippy --workspace --all-targets -- -D warnings`. A newer clippy
now enforces `clippy::op_ref` and `clippy::io_other_error` on code paths that only compile under
`--all-targets` (i.e. test harness targets). Seven errors block all commits:
- 4× `clippy::op_ref` in `crates/srs-repository/src/record_store.rs` (test-only code)
- 3× `clippy::io_other_error` in `crates/srs-repository/src/store.rs` (FailStore test double)

All fixes are mechanical. No behaviour change, no public API change.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — (record_store.rs + store.rs) |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan applies mechanical lint suggestions with no design
choices. ADR-010 and ADR-011 are unaffected (no service API or payload change).

---

## Contracts

### CLI output contract (ADR-011)

No new or changed commands. No action required; golden schemas stay as-is.

### Entity schema sync (check-schema-sync.sh)

No schema files added or modified. No action required.

---

## Scope

- Fix `clippy::op_ref` at `record_store.rs` lines 6310, 6333, 6361, 6384 by removing the `&` from
  the right-hand operand in comparisons: `e.instance_id() == &instance_id` → `e.instance_id() == instance_id`
  (and `!= &instance_id` → `!= instance_id` at line 6384).
- Fix `clippy::io_other_error` at `store.rs` lines 2400, 2743, 2843 by replacing
  `std::io::Error::new(std::io::ErrorKind::Other, msg)` → `std::io::Error::other(msg)`.

**Out of scope:**
- Any refactor of `find_gallery_fixture` in `tests/export_srsj.rs` (no lint fires there).
- Any feature work in `record_store.rs` or `store.rs`.

---

## Phases

### Phase 1: Apply lint fixes

**Goal:** `cargo clippy --workspace --all-targets -- -D warnings` exits 0.

**Agent:** Repository Service Worker

#### Tasks

- [x] `record_store.rs:6310` — change `== &instance_id` to `== instance_id`
- [x] `record_store.rs:6333` — change `== &instance_id` to `== instance_id`
- [x] `record_store.rs:6361` — change `== &instance_id` to `== instance_id`
- [x] `record_store.rs:6384` — change `!= &instance_id` to `!= instance_id`
- [x] `store.rs:2400` — replace `std::io::Error::new(std::io::ErrorKind::Other, ...)` with `std::io::Error::other(...)`
- [x] `store.rs:2743` — same replacement
- [x] `store.rs:2843` — same replacement

#### Acceptance Criteria

- [x] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [x] `cargo test -p srs-repository` passes
- [x] Pre-commit hook passes without `--no-verify`

#### Testing

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p srs-repository
```

Specific tests to verify:

- `delete_record_index_first_manifest_fail_leaves_record_intact` — contains op_ref sites at lines 6310, 6333; run with `cargo test -p srs-repository delete_record_index_first_manifest_fail`
- `delete_record_index_first_file_fail_leaves_orphaned_file_safe` — contains op_ref sites at lines 6361, 6384; run with `cargo test -p srs-repository delete_record_index_first_file_fail`
- All FailPoint / fault-injection tests — confirm no behaviour change from `io::Error::other()` substitution

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run:

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p srs-repository
```

3. Commit with message `fix(clippy): resolve op_ref and io_other_error lints (#553)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] CLI output format unchanged (`cargo test --test payload_contracts` passes)
- [ ] `bash scripts/check-schema-sync.sh` exits 0

## Coordination Rules

- Agents keep to their write scopes.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm tests pass, update
  checkboxes, then commit.

## Assumptions

- The `io::Error::other(msg)` API is available (Rust 1.82+, confirmed by toolchain 1.94).
- Removing `&` from comparison right-hand operands is safe because `instance_id()` returns a type
  that implements `PartialEq<InstanceId>` by value.
