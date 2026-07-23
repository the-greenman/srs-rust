# Plan: srs-gov Editing Verbs — Real Create, Transition, Relate

> **Issue:** the-greenman/srs-rust#378

## Summary

`srs-gov create` is permanently dry-run, printing the underlying `srs` command rather than executing it. `srs-gov` has no `transition` verb to advance a decision's lifecycle state (ADR-022: governance status IS lifecycle state), and no `relate`/`relations`/`unrelate` verbs for managing `supersedes`/`depends-on` relations between decisions. This forces operators to fall back to raw `srs` invocations for every write action, defeating the purpose of the governance CLI layer. This plan adds `--dry-run` to `create` (preserving the old behaviour as an opt-in), implements `transition <id> --to <state>`, and adds `relate`, `unrelate`, and `relations` verbs. All new verbs follow the established `srs-gov` subprocess pattern: compose `srs` subcommands via `run_srs`/`run_srs_with_stdin` and render friendly output. No new `srs-cli` payload structs or service changes are needed.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Lead |
| CLI Worker | Lead (write scope: `crates/srs-gov/src/**`, `crates/srs-gov/tests/**`) |
| Verification | Verification Agent (read-only, post-implementation) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs are required — this plan implements existing decisions.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `srs-gov` calls `srs` as a subprocess — one service call per concern, no business logic in the CLI compositor | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | `srs-gov` has no structured payload contract of its own; it renders human-readable text. No `payload.rs` changes needed. | accepted |
| [ADR-022](../docs/adr/022-governance-status-is-lifecycle-state.md) | Governance status IS lifecycle state. `transition` must call `srs record transition`, never a raw field write. | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No changes to `crates/srs-cli/src/payload.rs`. `srs-gov` does not have a JSON envelope payload of its own — it passes `--format json` to `srs` subprocesses and renders human-readable text from their payloads. The `--json` flag on `srs-gov` commands forwards the raw `srs` envelope to stdout unchanged (existing behaviour; new verbs follow the same pattern).

`cargo test --test payload_contracts` is not affected.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/` or its mirrors. `check-schema-sync.sh` is not affected.

---

## Scope

- **`srs-gov create <key> <child>`**: add `--dry-run` flag; when absent, execute the record creation for real and print a confirmation.
- **`srs-gov transition <id> --to <state>`**: advance a governance record's lifecycle state via `srs record transition`.
- **`srs-gov relations <id>`**: list outgoing and incoming relations for a governance record via `srs relation list`.
- **`srs-gov relate <id> --type <relation_type> --target <target_id>`**: create a relation (type must be a canonical relation type; governance-layer restricts to `supersedes` and `depends-on`).
- **`srs-gov unrelate <relation_id>`**: delete a relation by its UUID.

**Out of scope:**
- Interactive allowed-transitions picker (the `--to` flag is mandatory; a future `allowed-transitions` sub-command could add interactivity).
- RFC-022 fulfillment paths for `supersedes` (the `requiresRelation` constraint on the `superseded` state requires providing a successor instance ID; this is an advanced flow for which operators can use raw `srs record transition` with `byTransition`+`fulfillment` JSON). Deferred.
- `relate` with arbitrary relation types beyond `supersedes`/`depends-on` — governance layer enforces these two. Other types remain accessible via `srs relation create`.
- Bindings (`srs-bindings`) or MCP surface — those surfaces already have relation and lifecycle coverage via existing service calls.

---

## Phases

### Phase 1: Expose `run_srs_with_stdin` in `srs.rs`

**Goal:** `main.rs` handlers can call `srs` with JSON piped to stdin, enabling write operations.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-gov/src/srs.rs`, add a public wrapper:
  ```rust
  pub fn run_srs_with_stdin(
      args: &[&str],
      repo: &str,
      stdin_json: &str,
      explain: bool,
      print_raw: bool,
  ) -> Result<Value> {
      run_srs_impl(args, repo, Some(stdin_json), explain, print_raw)
  }
  ```
  `run_srs_impl` already accepts `stdin_json: Option<&str>` — this is a one-line wrapper.

#### Acceptance Criteria

- [ ] `run_srs_with_stdin` compiles and is visible from `main.rs`.
- [ ] Existing `run_srs` function is unchanged.
- [ ] `cargo build -p srs-gov` passes.

#### Testing

```bash
cargo build -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

No new unit tests at this phase — the function is a thin wrapper tested indirectly by Phase 2–4.

#### Milestone gate

1. Verify acceptance criteria above.
2. Run lint and build.

```bash
cargo build -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

3. Commit: `feat(gov): expose run_srs_with_stdin for write operations (#378)`.

---

### Phase 2: `create` real write

**Goal:** `srs-gov create <key> <child>` executes the record write by default; `--dry-run` preserves the old behaviour.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-gov/src/main.rs`, in `Commands::Create`, add field `dry_run: bool` with `#[arg(long)]`.
- [ ] In `crates/srs-gov/src/main.rs`, in `run()`, pass `dry_run` to `cmd_create`.
- [ ] In `crates/srs-gov/src/main.rs`, update `cmd_create` signature to accept `dry_run: bool`.
- [ ] When `dry_run = true`: existing behaviour (print the heredoc, return `Ok(())`).
- [ ] `--explain` mode (when `cli.explain = true`): print the underlying `srs record create` command with its stdin JSON as a comment block (same format as the existing dry-run heredoc), without executing. `--explain` takes priority over the default write path.
- [ ] When `dry_run = false` and `explain = false` (default): call `run_srs_with_stdin` to execute the write:
  ```rust
  let payload = run_srs_with_stdin(
      &["record", "create", "--type", type_ref, "--container", &container_id],
      repo,
      &input_json,
      explain,
      json,
  )?;
  if json || explain { return Ok(()); }
  let instance_id = payload["record"]["instanceId"].as_str().unwrap_or("(unknown)");
  render::record_created(instance_id, child, &container_id);
  ```
- [ ] Add `render::record_created(instance_id: &str, child: &str, container_id: &str)` in `crates/srs-gov/src/render.rs`:
  ```rust
  pub fn record_created(instance_id: &str, child: &str, container_id: &str) {
      header(&format!("Created  {}", short_id(instance_id)));
      println!();
      println!("  Type:       {child}");
      println!("  Container:  {container_id}");
      println!("  ID:         {instance_id}");
      println!();
      println!("  Run: srs-gov get {container_id} {instance_id}  to view the record");
      println!();
  }
  ```
- [ ] Update existing tests that exercise `create` without `--dry-run` to add `--dry-run`:
  - `create_decision_dry_run_emits_correct_command` → add `"--dry-run"` arg
  - `create_decision_dry_run_does_not_mutate` → add `"--dry-run"` arg
  - `create_decision_dry_run_escapes_quoted_values` → add `"--dry-run"` arg
- [ ] Add test `create_decision_writes_record` in `crates/srs-gov/tests/flow.rs`:
  - Use `setup_repo("create-write")` to create a fresh governance repo.
  - Run `srs-gov create decision_log decision --title "Test Write Decision" --statement "this proves the write"` (`decision_log` is a hardcoded key defined by the governance package; no navigation call needed).
  - Assert exit 0 and stdout contains "Created" and the new instance ID (check for a UUID-shaped string or the `short_id` prefix format used elsewhere in render.rs).
  - Verify `srs record list --type com.mudemocracy.governance/decision` shows the new record (record count increases by 1 vs baseline).
  - Run `srs repo validate` and assert 0 errors.

#### Acceptance Criteria

- [ ] `srs-gov create decision_log decision --title "Test" --statement "s"` persists a record to the repository, verified by `srs record list --type com.mudemocracy.governance/decision` showing an increased count.
- [ ] `srs-gov create decision_log decision --dry-run` still prints the command without writing.
- [ ] `srs-gov create ... --json` pipes the raw `srs record create` envelope to stdout.
- [ ] `srs-gov create ... --explain` prints the underlying `srs record create` command + stdin JSON without executing (no record created).
- [ ] `srs repo validate` returns 0 errors after a real write.
- [ ] All three existing dry-run tests still pass (with `--dry-run` added).
- [ ] `create_decision_writes_record` passes.

#### Testing

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests:
- `create_decision_dry_run_emits_correct_command` — unchanged semantics, `--dry-run` flag added
- `create_decision_dry_run_does_not_mutate` — unchanged semantics, `--dry-run` flag added
- `create_decision_dry_run_escapes_quoted_values` — unchanged semantics, `--dry-run` flag added
- `create_decision_writes_record` — new: proves the default path writes a real record

#### Milestone gate

1. All acceptance criteria checked.
2. All named tests pass.

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

3. Commit: `feat(gov): add --dry-run to create; default now writes real record (#378)`.

---

### Phase 3: `transition` verb

**Goal:** `srs-gov transition <id> --to <state>` advances a governance record's lifecycle state.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-gov/src/main.rs`, add to `Commands` enum:
  ```rust
  /// Transition a governance record's lifecycle state (ext:lifecycle)
  #[command(name = "transition")]
  Transition {
      /// Instance ID (or unique prefix) of the record to transition
      id: String,
      /// Target lifecycle state (e.g. "proposed", "ratified", "superseded", "closed", "abandoned")
      #[arg(long)]
      to: String,
  },
  ```
- [ ] In `crates/srs-gov/src/main.rs`, dispatch in `run()`: `Some(Commands::Transition { id, to }) => cmd_transition(&id, &to, &cli.repo, cli.explain, cli.json)`.
- [ ] In `crates/srs-gov/src/main.rs`, implement `cmd_transition(id, to, repo, explain, json)`:
  - Build `stdin_json = serde_json::json!({"to": to}).to_string()`.
  - `explain` mode: print `srs record allowed-transitions --id <id>` (explain=true) then `srs record transition --id <id>` (explain=true) with the stdin JSON comment.
  - Normal mode: call `run_srs_with_stdin(&["record", "transition", "--id", id], repo, &stdin_json, false, json)`.
  - If `json`: return `Ok(())` (envelope already printed).
  - Extract `payload["record"]["lifecycleState"].as_str()` and call `render::transition_applied(id, to_state)`.
- [ ] Add `render::transition_applied(id: &str, state: &str)` in `crates/srs-gov/src/render.rs`:
  ```rust
  pub fn transition_applied(id: &str, state: &str) {
      header(&format!("Transitioned  {}", short_id(id)));
      println!();
      println!("  New state:  {state}");
      println!();
      println!("  Run: srs record get --id {id}  to see the full record");
      println!();
  }
  ```
- [ ] Add test `transition_decision_succeeds`:
  - Use `setup_repo("transition-ok")`.
  - Create a decision (starts in `draft`). Get its instanceId.
  - Run `srs-gov transition <id> --to proposed`.
  - Assert exit 0.
  - Verify via `srs record get --id <id>` that `lifecycleState == "proposed"`.
- [ ] Add test `transition_invalid_state_fails`:
  - Use `setup_repo("transition-bad")`.
  - Create a decision (draft).
  - Run `srs-gov transition <id> --to nonexistent_state`.
  - Assert exit code != 0.

#### Acceptance Criteria

- [ ] `srs-gov transition <id> --to proposed` advances a draft decision to `proposed`.
- [ ] `srs-gov transition <id> --to nonexistent_state` exits non-zero with an error message.
- [ ] `srs-gov transition ... --json` outputs the raw `srs record transition` envelope.
- [ ] `srs-gov transition ... --explain` prints the underlying `srs` commands without executing.
- [ ] `srs repo validate` returns 0 errors after a transition.

#### Testing

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests:
- `transition_decision_succeeds` — happy path: draft → proposed
- `transition_invalid_state_fails` — negative: invalid target state rejected

#### Milestone gate

1. All acceptance criteria checked.
2. All named tests pass.

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

3. Commit: `feat(gov): add transition verb (#378)`.

---

### Phase 4: `relate`, `unrelate`, `relations` verbs

**Goal:** `srs-gov relate/unrelate/relations` manage `supersedes`/`depends-on` relations between governance records.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-gov/src/main.rs`, add to `Commands` enum:
  ```rust
  /// List outgoing and incoming relations for a governance record
  #[command(name = "relations")]
  Relations {
      /// Instance ID (or unique prefix) of the record
      id: String,
  },
  /// Create a relation between two governance records
  #[command(name = "relate")]
  Relate {
      /// Source instance ID (or unique prefix)
      id: String,
      /// Relation type: "supersedes" or "depends-on"
      #[arg(long = "type")]
      relation_type: String,
      /// Target instance ID (or unique prefix)
      #[arg(long)]
      target: String,
  },
  /// Delete a relation by its UUID
  #[command(name = "unrelate")]
  Unrelate {
      /// Relation UUID to delete
      relation_id: String,
  },
  ```
- [ ] In `crates/srs-gov/src/main.rs`, dispatch all three in `run()`.
- [ ] In `crates/srs-gov/src/main.rs`, implement `cmd_relations(id, repo, explain, json)`:
  - If `explain`: call `run_srs(&["relation", "list", "--source", id], repo, true, false)?` and `run_srs(&["relation", "list", "--target", id], repo, true, false)?`, return.
  - Otherwise: call `srs relation list --source <id>` and `srs relation list --target <id>` (two calls), merge the `relations` arrays (deduplicating by `relationId`), call `render::relations_list(id, &merged)`.
  - `json` mode: pipe the raw envelope from the `srs relation list --source <id>` call only (outgoing relations). The `--json` flag help text must document this limitation: `"Output raw JSON (outgoing relations only; for full graph use srs relation list --source/--target directly)"`.

- [ ] Add `render::relations_list(id: &str, relations: &[serde_json::Value])` in `crates/srs-gov/src/render.rs`:
  ```rust
  pub fn relations_list(id: &str, relations: &[serde_json::Value]) {
      header(&format!("Relations  —  {}", short_id(id)));
      println!();
      if relations.is_empty() {
          println!("  (no relations)");
          println!();
          return;
      }
      println!("  {:<10}  {:<20}  {:<16}  {:<16}  ID", "DIRECTION", "TYPE", "SOURCE", "TARGET");
      println!("  {}", "─".repeat(80));
      for r in relations {
          let source = r["sourceInstanceId"].as_str().unwrap_or("");
          let target = r["targetInstanceId"].as_str().unwrap_or("");
          let rtype = r["relationType"].as_str().unwrap_or("");
          let rid = r["relationId"].as_str().unwrap_or("");
          let direction = if source.starts_with(id) || source == id { "outgoing" } else { "incoming" };
          println!(
              "  {:<10}  {:<20}  {:<16}  {:<16}  {}",
              direction, rtype, short_id(source), short_id(target), short_id(rid)
          );
      }
      println!();
      println!("  Run: srs-gov unrelate <ID>  to remove a relation");
      println!();
  }
  ```
- [ ] In `crates/srs-gov/src/main.rs`, implement `cmd_relate(id, relation_type, target, repo, explain, json)`:
  - Validate `relation_type` is `supersedes` or `depends-on`; error otherwise with `anyhow::bail!`.
  - **If `explain = true`**: early-return IMMEDIATELY before any `run_srs` calls, printing placeholder-based command previews:
    ```
    # Would run:
    srs record get --id <id> --repo <repo>
    srs record get --id <target> --repo <repo>
    srs relation create --repo <repo>
    # stdin: {"relationType": "<relation_type>", "sourceInstanceId": "<source-id>", "targetInstanceId": "<target-id>"}
    ```
    Then `return Ok(())`.
  - Resolve the full instance ID for `id` and `target` via `run_srs(&["record", "get", id], ...)` → `payload["record"]["instanceId"]` (needed because the user may pass a prefix; `srs relation create` requires full UUIDs). This resolution step only runs when `explain = false`.
  - Build `stdin_json = json!({"relationType": relation_type, "sourceInstanceId": full_source, "targetInstanceId": full_target}).to_string()`.
  - Normal mode: call `run_srs_with_stdin(&["relation", "create"], repo, &stdin_json, false, json)`.
  - Extract `payload["relation"]["relationId"]` and call `render::relation_created(...)`.
- [ ] Add `render::relation_created(relation_id: &str, relation_type: &str, source_id: &str, target_id: &str)` in `crates/srs-gov/src/render.rs`:
  ```rust
  pub fn relation_created(relation_id: &str, relation_type: &str, source_id: &str, target_id: &str) {
      header(&format!("Relation created  {}", short_id(relation_id)));
      println!();
      println!("  Type:    {relation_type}");
      println!("  Source:  {source_id}");
      println!("  Target:  {target_id}");
      println!();
      println!("  Run: srs-gov unrelate {relation_id}  to remove");
      println!();
  }
  ```
- [ ] In `crates/srs-gov/src/main.rs`, implement `cmd_unrelate(relation_id, repo, explain, json)`:
  - Call `run_srs(&["relation", "delete", &relation_id], repo, explain, json)`.
  - Extract `payload["relationId"]` and call `render::relation_deleted(relation_id)`.
- [ ] Add `render::relation_deleted(relation_id: &str)` in `crates/srs-gov/src/render.rs`:
  ```rust
  pub fn relation_deleted(relation_id: &str) {
      header(&format!("Relation deleted  {}", short_id(relation_id)));
      println!();
      println!("  ID:  {relation_id}");
      println!();
  }
  ```
- [ ] Add test `relate_and_unrelate`:
  - `setup_repo("relate-test")`, create two decisions (decision A and B).
  - Run `srs-gov relate <A_id> --type depends-on --target <B_id>`.
  - Assert exit 0, output contains "depends-on" or "Created" or "relation".
  - Run `srs-gov relations <A_id>` and assert output contains "depends-on" and short ID of B.
  - Parse the relation ID from the `srs relation list --source <A_id>` output.
  - Run `srs-gov unrelate <relation_id>`.
  - Assert exit 0.
  - Run `srs-gov relations <A_id>` and assert output does NOT contain "depends-on".
  - Run `srs repo validate` and assert 0 errors.
- [ ] Add test `relate_invalid_type_rejected`:
  - `setup_repo("relate-bad-type")`, create a decision.
  - Run `srs-gov relate <id> --type unknown_type --target <id>`.
  - Assert exit code != 0.

#### Acceptance Criteria

- [ ] `srs-gov relate <A> --type depends-on --target <B>` creates a `depends-on` relation from A to B.
- [ ] `srs-gov relate <A> --type supersedes --target <B>` creates a `supersedes` relation.
- [ ] `srs-gov relate <A> --type unknown_type --target <B>` exits non-zero.
- [ ] `srs-gov relations <id>` shows both outgoing and incoming relations in normal mode.
- [ ] `srs-gov relations <id> --json` outputs the raw `srs relation list --source <id>` envelope (outgoing only; documented limitation in flag help text).
- [ ] `srs-gov unrelate <relation_id>` deletes the relation.
- [ ] `srs repo validate` returns 0 errors after relate/unrelate.
- [ ] `--json` and `--explain` flags work on all three verbs.

#### Testing

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests:
- `relate_and_unrelate` — happy path round-trip
- `relate_invalid_type_rejected` — governance type restriction

#### Milestone gate

1. All acceptance criteria checked.
2. All named tests pass.

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

3. Commit: `feat(gov): add relate, unrelate, relations verbs (#378)`.

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (all existing `srs-gov` integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (payload structs not changed — sanity check only)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (schemas not changed — sanity check only)
- [ ] `srs-gov create <key> <child>` writes a real record (no `--dry-run`)
- [ ] `srs-gov create <key> <child> --dry-run` still prints the command without writing
- [ ] `srs-gov transition <id> --to <state>` changes the record's `lifecycleState`
- [ ] `srs-gov relate <id> --type supersedes --target <target>` creates a relation
- [ ] `srs-gov relations <id>` lists outgoing and incoming relations
- [ ] `srs-gov unrelate <relation_id>` removes the relation
- [ ] `srs repo validate` returns 0 errors after all write operations

## Coordination Rules

- Only the CLI Worker writes to `crates/srs-gov/src/**` and `crates/srs-gov/tests/**`.
- No changes to `crates/srs-cli/`, `crates/srs-repository/`, or `crates/srs-bindings/`.
- Milestone gate must pass before the next phase starts.
- Lead Integrator owns final API naming (subcommand names, flag names).

## Assumptions

- The `srs` binary is available on PATH or via `SRS_BIN` when `srs-gov` runs (established requirement).
- The governance package's `supersedes` and `depends-on` relation types are already defined in `com.mudemocracy.governance` (they are — the package ships with them).
- RFC-022 fulfillment (providing a `fulfillment.existingInstanceId` when transitioning to `superseded`) is out of scope; the `transition --to superseded` command will fail with the Rust validation error until the user provides the correct input via raw `srs record transition`. Deferred to a follow-up.
