# Plan: srs-gov --json coverage for the 5 editing verbs

> Issue: srs-rust#741

## Summary

`crates/srs-gov/tests/flow.rs` has no tests exercising `--json` output for any of the five
editing verbs landed in the #378 cluster: `create`, `transition`, `relate`, `relations`, and
`unrelate`. Commit `9658dd4` fixed a real regression in `cmd_relations --json` (incoming
relations were dropped) but added no regression test to guard the fix. This plan adds
`--json` mode assertions for all five verbs so a future refactor cannot reintroduce the same
class of bug silently.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Lead |
| Gov Test Worker | Lead (write scope: `crates/srs-gov/tests/flow.rs` only) |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs needed. This plan adds tests only; it does not change production code, payloads,
CLI contracts, or data model. Existing ADR-010/ADR-011 are unaffected (no handler or payload
changes).

| ADR | Decision | Status |
|---|---|---|
| — | No new architectural decisions — pure test addition | — |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed commands. No payload structs changed. `cargo test --test payload_contracts`
must continue to pass.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files changed. Schema sync check must pass.

---

## Scope

- Add 5 new test functions to `crates/srs-gov/tests/flow.rs`, one per editing verb.
- Add a private `gov_json` helper to reduce boilerplate.
- Assert the minimum shape needed to guard the regression: for `relations`, verify both
  outgoing **and** incoming relations appear in the JSON output.

**Out of scope:**
- Changes to any production source file.
- Testing `--json` for read-only verbs (top-level, `list`, `get`) — already covered.
- Exhaustive field validation of every payload field.

---

## Phases

### Phase 1: Add `--json` test coverage for all five editing verbs

**Goal:** `cargo test -p srs-gov` passes with five new tests, all green, and the
`cmd_relations --json` regression is explicitly guarded.

**Agent:** Gov Test Worker

#### Tasks

- [ ] Add a `gov_json` private helper to `tests/flow.rs` that runs `srs-gov --json <args>`
  and returns a parsed `serde_json::Value`.
- [ ] Add `create_json_returns_record_instanceid` — call
  `["--json", "create", "decision_log", "decision", "--title", "T", "--statement", "S"]`,
  parse JSON, assert `envelope["ok"] == true` and `envelope["payload"]["record"]["instanceId"]`
  is a non-empty string.
- [ ] Add `transition_json_returns_updated_lifecycle_state` — transition a draft decision to
  `proposed` with `["--json", "transition", <id>, "--to", "proposed"]`, parse JSON, assert
  `ok: true` and `envelope["payload"]["record"]["lifecycleState"] == "proposed"`.
- [ ] Add `relate_json_returns_relation_id` — create a `supersedes` relation with
  `["--json", "relate", <a_id>, "--type", "supersedes", "--target", <b_id>]`, parse JSON,
  assert `ok: true` and `envelope["payload"]["relation"]["relationId"]` is a non-empty string.
- [ ] Add `unrelate_json_returns_ok` — create a relation via `srs relation create` (using
  `srs_json`), then delete it with `["--json", "unrelate", <relation_id>]`, assert `ok: true`.
- [ ] Add `relations_json_includes_both_directions` — create an A→B `supersedes` relation,
  call `["--json", "relations", <b_id>]` (B has an **incoming** relation from A), parse the
  raw `{"relations": [...]}` JSON (not an `ok` envelope), assert the array contains at least
  one entry with `relationType == "supersedes"`. This directly guards the `9658dd4` regression:
  incoming relations must appear even when the caller queries from the target's perspective.

#### Acceptance Criteria

- [ ] Five new `#[test]` functions present and named exactly as listed above.
- [ ] `cargo test -p srs-gov` passes with zero failures.
- [ ] `cargo clippy -p srs-gov -- -D warnings` passes.
- [ ] `relations_json_includes_both_directions` specifically exercises the incoming-relation path
  (queries from B's perspective after A→B relation is created).

#### Testing

```bash
cargo build -p srs-gov
cargo test -p srs-gov json
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests:
- `create_json_returns_record_instanceid` — guards `create --json` envelope shape
- `transition_json_returns_updated_lifecycle_state` — guards `transition --json` envelope shape
- `relate_json_returns_relation_id` — guards `relate --json` envelope shape
- `unrelate_json_returns_ok` — guards `unrelate --json` envelope shape
- `relations_json_includes_both_directions` — guards the `9658dd4` regression
  (incoming relations must not be dropped in `--json` mode)

#### Milestone gate

1. Verify all five acceptance criteria above are met.
2. Confirm each test exists in `crates/srs-gov/tests/flow.rs` and passes.
3. Run:

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

4. Update plan checkboxes to `[x]`.
5. Commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (no handler edits; integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] All five new tests exist and are green
- [ ] `relations_json_includes_both_directions` exercises the incoming-relation path

## Coordination Rules

- Write scope is `crates/srs-gov/tests/flow.rs` only.
- No production source changes.
- Lead Integrator commits at the Phase 1 milestone gate.

## Assumptions

- `setup_repo` produces exactly one draft, one ratified, one superseded, and one closed
  decision — the existing tests rely on this; new tests may rely on it too.
- The `srs-gov --json` flag is a global flag placed before the subcommand name.
- `run_srs_with_stdin` with `print_raw=true` prints the full srs envelope (including `ok`,
  `command`, `payload`) verbatim; `cmd_relations --json` prints a custom `{"relations": [...]}`
  object that is NOT wrapped in an `ok` envelope.
