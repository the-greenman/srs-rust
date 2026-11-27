# Plan: Dedupe --into/--container conflict resolution in record.rs

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

`crates/srs-cli/src/commands/record.rs` has an identical ~12-line block that resolves a
conflict between the `--into` flag and the global `--container` flag, duplicated verbatim
in `cmd_record_fork` (pre-existing) and `cmd_record_duplicate` (added in commit 8b14309,
#1505). Extract the shared logic into one private helper function in the same file and
call it from both handlers. Pure DRY refactor: no behavior change, no public API change,
no payload/schema change. Issue: #1653.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | (this session) |
| CLI Worker | (this session) |
| Verification | (this session) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions. This plan is a pure refactor inside `srs-cli` command
handlers; it does not change any ADR-governed contract. ADR-010/ADR-011 (service function
contract, payload contract) are unaffected since no service signature or payload struct
changes.

**Helper location decision:** the helper is added as a private function in `record.rs`
itself, not in `commands/mod.rs`. Precedent check: `commands/mod.rs` hosts helpers used
*across* multiple command files (`with_store`, `parse_type_filter`, `resolve_repo`); a
grep of `crates/srs-cli/src/commands/*.rs` for the `--into`/`--container` conflict shape
shows it occurs only in these two `record.rs` handlers, so it stays file-local, matching
how other file-local logic in this codebase is kept in its own command file rather than
promoted to the shared module. This is a routine scoping call, not a long-term-consequence
decision, so it does not trigger the Stage 2 design-decision pause.

| ADR | Decision | Status |
|---|---|---|
| — | No new ADR — pure refactor, no new constraint | n/a |

---

## Contracts

### CLI output contract (ADR-011)

No new/changed commands, no payload struct changes. `RecordForkPayload` and
`RecordDuplicatePayload` are untouched. No action required.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are touched. No action required.

---

## Scope

- Add a private helper function in `crates/srs-cli/src/commands/record.rs`, e.g.:
  ```rust
  fn resolve_fork_container(
      cmd_name: &str,
      into: Option<String>,
      container_id: Option<String>,
  ) -> std::result::Result<Option<String>, RepositoryError>
  ```
  encapsulating the `(into, container_id)` match currently duplicated in both handlers,
  returning `Err(RepositoryError::InvalidInput { .. })` on conflict and `Ok(merged)`
  otherwise. `cmd_name` parametrizes the error-context string (`"record fork"` /
  `"record duplicate"`) if needed to keep existing output envelope naming exactly as-is;
  alternatively keep the `output::repo_err(...)` call at each call site and have the
  helper only return a `Result<Option<String>, RepositoryError>` — exact signature is an
  implementation detail, the acceptance criterion is byte-identical output.
- Call the helper from `cmd_record_fork` and `cmd_record_duplicate`, replacing the
  duplicated match blocks.
- No change to `cmd_record_fork`'s or `cmd_record_duplicate`'s external behavior, argument
  parsing, or output payloads.

**Out of scope:**
- Any change to `fork_service` or `duplicate_service` in `srs-repository`.
- Any change to CLI flags, help text, or payload shapes.
- Promoting the helper to `commands/mod.rs` (see Architecture Decisions).

---

## Phases

### Phase 1: Extract and wire the helper

**Goal:** `cmd_record_fork` and `cmd_record_duplicate` both call one shared private
helper for `--into`/`--container` conflict resolution; no behavior change.

**Agent:** CLI Worker

#### Tasks

- [x] Add the private helper function in `crates/srs-cli/src/commands/record.rs`.
- [x] Replace the duplicated block in `cmd_record_fork` with a call to the helper.
- [x] Replace the duplicated block in `cmd_record_duplicate` with a call to the helper.
- [x] Confirm error message text and `output::repo_err` call sites are byte-identical to
  current behavior for the conflict case.

#### Acceptance Criteria

- [x] `cmd_record_fork` and `cmd_record_duplicate` no longer contain duplicated
  conflict-resolution logic.
- [x] Existing tests for `record fork` and `record duplicate` (including any conflict-case
  tests) pass unchanged.
- [x] No new clippy warnings.

#### Testing

```bash
cargo test -p srs-cli
cargo test record_fork
cargo test record_duplicate
```

Specific tests to verify: any existing integration tests covering `record fork --into`
and `record duplicate --into` conflicting with `--container` (search
`crates/srs-cli/tests/` for `fork` and `duplicate`), run unchanged and passing.

#### Milestone gate

1. Verify acceptance criteria above.
2. Confirm existing conflict-case tests (if any) still pass; if none exist, note that
   explicitly — this plan does not add new tests since there is no behavior change to
   cover, but existing coverage must still pass.
3. Run:
   ```bash
   cargo test -p srs-cli
   cargo clippy -p srs-cli -- -D warnings
   ```
4. Mark checkboxes `[x]`.
5. Commit.

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] CLI output format unchanged (integration tests pass)
- [x] `cargo test --test payload_contracts` passes (no payload structs changed — this is
  a pass-through confirmation, not new work)
- [x] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [x] The duplicated 12-line block no longer appears twice in `record.rs`

## Coordination Rules

- Single-session implementation; no multi-agent write-scope conflicts expected given the
  size of this change.
- Verification Agent (or equivalent read-only pass) runs before final sign-off per Stage 7.

## Assumptions

- No existing test asserts on the literal duplicated code structure (only on CLI
  behavior/output), so refactoring the internals is safe.
- The helper's exact signature is an implementation detail left to the CLI Worker,
  constrained only by: private to `record.rs`, no behavior change, no new public API.
