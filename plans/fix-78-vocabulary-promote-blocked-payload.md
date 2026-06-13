# Plan: Fix vocabulary promote blocked-error payload (#78)

## Summary

`srs vocabulary promote` currently discards the list of unresolvable tag keys when the V10 pre-flight
check blocks promotion. Callers receive only a count in a diagnostic string, not the actionable key
names. The service already returns a structured `VocabularyPromotionBlocked` error with a sorted
`Vec<String>` of unresolvable keys; this plan threads that data through the CLI output layer so that
callers receive a structured `ok: false` payload with `vocabularyId` and `unresolvableKeys`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| CLI Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Every CLI command output is a named struct in `payload.rs`; no `json!()` literals in handlers | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | CLI handlers: arg parsing + one service call + output; no business logic | accepted |

No new ADRs required. ADR-011 already covers the `err_payload` gap — we are adding `output::err_with_payload` as an implementation detail of the existing contract, not changing the contract.

---

## Contracts

### CLI output contract (ADR-011)

This plan adds a new payload struct `PromoteVocabularyBlockedPayload` in `crates/srs-cli/src/payload.rs` and produces a new schema file `crates/srs-cli/schemas/payload/vocabulary-promote-blocked.json`.

The existing `vocabulary-promote.json` (the success payload) is unchanged.

Steps:
1. Add `PromoteVocabularyBlockedPayload` struct to `payload.rs`.
2. Run `cargo run --bin generate-schemas`.
3. Commit the new `schemas/payload/vocabulary-promote-blocked.json`.
4. Verify `cargo test --test payload_contracts` passes.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/` — not applicable.

---

## Scope

- Add `PromoteVocabularyBlockedPayload { vocabulary_id: String, unresolvable_keys: Vec<String> }` to `crates/srs-cli/src/payload.rs`.
- Add `output::err_with_payload<T: Serialize>(command: &str, diagnostics: Vec<String>, payload: T) -> String` to `crates/srs-cli/src/output.rs`.
- Update `cmd_vocabulary_promote` in `crates/srs-cli/src/commands/vocabulary.rs` to catch `RepositoryError::VocabularyPromotionBlocked` and call `output::err_with_payload` instead of bubbling to the generic string handler.
- Run `cargo run --bin generate-schemas` and commit `schemas/payload/vocabulary-promote-blocked.json`.
- Add integration test `test_vocabulary_promote_blocked_payload` in `crates/srs-cli/tests/` that asserts `ok == false` and `payload.unresolvableKeys` contains the expected key names.

**Out of scope:**
- Exposing `derive-tag-set` as a user-facing subcommand (deferred; tracked in a separate issue filed at Stage 3).
- Changing output format for any other blocked/error variants.
- Modifying `srs-repository` — the service layer is already correct.

---

## Phases

### Phase 1: Output plumbing — `err_with_payload` and new struct

**Goal:** `output.rs` has a typed error-with-payload function and `payload.rs` has `PromoteVocabularyBlockedPayload`; golden schemas are regenerated and the contract test passes.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-cli/src/output.rs`, add immediately after `output::serialize`:
  ```rust
  /// Emit an `ok: false` envelope with a typed payload (for structured error responses).
  /// Note: like `output::err`, this hardcodes `Json, false` — `--pretty` and `--format`
  /// are not honoured on error paths. This is a known limitation consistent with the
  /// existing `output::err` behaviour; tracked for a future format-aware output refactor.
  pub fn err_with_payload<T: serde::Serialize>(command: &str, diagnostics: Vec<String>, payload: T) -> anyhow::Result<String> {
      let value = serde_json::to_value(payload)
          .map_err(|e| anyhow::anyhow!("Failed to serialize error payload for '{}': {}", command, e))?;
      let dto = OutputDTO {
          ok: false,
          command: command.to_string(),
          version: VERSION.to_string(),
          payload: Some(value),
          diagnostics: Some(diagnostics),
      };
      Ok(dto.render(crate::commands::OutputFormat::Json, false))
  }
  ```
- [ ] In `crates/srs-cli/src/payload.rs`, add immediately after the closing brace of `PromoteVocabularyPayload` (before the `// ── Field payloads` section comment):
  ```rust
  /// Error payload for `vocabulary promote` when pre-flight blocks promotion.
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct PromoteVocabularyBlockedPayload {
      pub vocabulary_id: String,
      pub unresolvable_keys: Vec<String>,
  }
  ```
- [ ] Run `cargo run --bin generate-schemas` from `srs-rust/` to produce `crates/srs-cli/schemas/payload/vocabulary-promote-blocked.json`.
- [ ] In `crates/srs-cli/tests/payload_contracts.rs`, add:
  ```rust
  #[test]
  fn vocabulary_promote_blocked() {
      check::<PromoteVocabularyBlockedPayload>("vocabulary-promote-blocked");
  }
  ```
  Also add `PromoteVocabularyBlockedPayload` to the use statement at the top of that file.

#### Acceptance Criteria

- [ ] `output::err_with_payload` compiles and is pub in `output.rs`.
- [ ] `PromoteVocabularyBlockedPayload` derives `Serialize` and `JsonSchema` and uses `camelCase`.
- [ ] `crates/srs-cli/schemas/payload/vocabulary-promote-blocked.json` exists and contains `vocabularyId` and `unresolvableKeys` in the JSON Schema.
- [ ] `cargo test --test payload_contracts -- vocabulary_promote_blocked` passes.
- [ ] `cargo test --test payload_contracts` passes (all entries).

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

Specific tests to write or verify:
- `payload_contracts` golden-file test — proves schema file matches struct.

#### Milestone gate

1. Check all acceptance criteria above.
2. Confirm `payload_contracts` test exists and passes.
3. Run:
   ```bash
   cargo test -p srs-cli
   cargo clippy -p srs-cli -- -D warnings
   ```
4. Mark task checkboxes `[x]`.
5. Commit: `feat(cli): add PromoteVocabularyBlockedPayload and err_with_payload (#78)`.

---

### Phase 2: Wire the handler

**Goal:** `cmd_vocabulary_promote` catches `RepositoryError::VocabularyPromotionBlocked` and returns a structured `ok: false` payload with `vocabularyId` and `unresolvableKeys`.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-cli/src/commands/vocabulary.rs`:
  - Add `use srs_repository::error::RepositoryError;` import (if not already present — check existing imports at top of file).
  - Add `PromoteVocabularyBlockedPayload` to the existing `use crate::payload::{...}` import.
  - Replace the current `cmd_vocabulary_promote` body using the established `downcast_ref` pattern (consistent with `protocol.rs` and other handlers that catch specific `RepositoryError` variants):
    ```rust
    fn cmd_vocabulary_promote(ctx: CliContext, id: String) -> Result<String> {
        match with_store(&ctx, |store| {
            Ok(vocabulary_service::promote_vocabulary(
                store,
                vocabulary_service::PromoteVocabularyInput { vocabulary_id: id.clone() },
            )?)
        }) {
            Ok(r) => output::serialize(
                "vocabulary promote",
                PromoteVocabularyPayload { vocabulary: r.vocabulary },
            ),
            Err(e) => {
                if let Some(RepositoryError::VocabularyPromotionBlocked {
                    vocabulary_id,
                    unresolvable_keys,
                }) = e.downcast_ref::<RepositoryError>()
                {
                    return output::err_with_payload(
                        "vocabulary promote",
                        vec![format!(
                            "vocabulary '{}' promotion blocked: {} in-use key(s) have no active term in the vocabulary",
                            vocabulary_id,
                            unresolvable_keys.len()
                        )],
                        PromoteVocabularyBlockedPayload {
                            vocabulary_id: vocabulary_id.clone(),
                            unresolvable_keys: unresolvable_keys.clone(),
                        },
                    );
                }
                Err(e)
            }
        }
    }
    ```
  Note: `downcast_ref` gives shared references, so `.clone()` is required on `vocabulary_id` and `unresolvable_keys`.

#### Acceptance Criteria

- [ ] On a blocked promote, the CLI returns `"ok": false` with a `payload` containing `vocabularyId` (camelCase) and `unresolvableKeys` (camelCase array of strings).
- [ ] On a blocked promote, `diagnostics` still contains the human-readable message with count (backward-compat diagnostic string preserved).
- [ ] On a successful promote, output is unchanged from before this fix.
- [ ] The unhandled `?` propagation to the generic error handler is gone for `VocabularyPromotionBlocked`.

#### Testing

```bash
cargo test -p srs-cli
cargo test -p srs-repository
cargo clippy -p srs-cli -- -D warnings
```

Specific tests to write or verify:
- `test_vocabulary_promote_blocked_payload` (new, Phase 3) — black-box integration test.

#### Milestone gate

1. Check all acceptance criteria above.
2. Run:
   ```bash
   cargo test -p srs-cli
   cargo clippy -p srs-cli -- -D warnings
   ```
3. Mark task checkboxes `[x]`.
4. Commit: `feat(cli): wire vocabulary promote blocked error to structured payload (#78)`.

---

### Phase 3: Integration test

**Goal:** A test exercises the blocked promote path end-to-end and asserts the structured payload.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-cli/tests/integration_tests.rs`, add `test_vocabulary_promote_blocked_payload` in the existing RFC-006 vocabulary/term/lifecycle section (search for the comment `// ── RFC-006 vocabulary / term / lifecycle integration tests`):
  - Follow the existing subprocess pattern used by all other vocabulary tests in this file (e.g. `run_srs_any_status_in_dir`, `run_srs_stdin_any_status_in_dir`).
  - Setup: use an existing test fixture or `tempdir` + `srs repo init`; create a vocabulary via stdin with `vocabulary create`; create a term for key `"alpha"` with status `active`; do NOT create a term for key `"beta"` (so `"beta"` is unresolvable).
  - Create a record in the repo tagged with `"alpha"` and `"beta"`.
  - Run `srs vocabulary promote <id>` against that repo; capture the output with a non-zero-OK status.
  - Parse the output as JSON.
  - Assert: `result["ok"] == false`.
  - Assert: `result["payload"]["unresolvableKeys"]` is a JSON array containing `"beta"`.
  - Assert: `result["payload"]["vocabularyId"]` matches the vocabulary id used.
  - Assert: `result["diagnostics"][0]` contains `"promotion blocked"`.
  - Assert successful promote still works: after adding an active term for `"beta"`, re-run promote and assert `result["ok"] == true`.

#### Acceptance Criteria

- [ ] `test_vocabulary_promote_blocked_payload` compiles and passes.
- [ ] Test asserts `ok == false`, `payload.unresolvableKeys == ["missing-key"]`, `payload.vocabularyId` matches.
- [ ] No existing vocabulary tests regressed.

#### Testing

```bash
cargo test -p srs-cli -- test_vocabulary_promote_blocked_payload
cargo test -p srs-cli
```

Specific tests to write or verify:
- `test_vocabulary_promote_blocked_payload` — proves the full blocked path end-to-end via subprocess invocation (consistent with existing vocabulary integration tests).

#### Milestone gate

1. Check all acceptance criteria above.
2. Run full test suite:
   ```bash
   cargo test
   cargo clippy -- -D warnings
   ```
3. Mark task checkboxes `[x]`.
4. Commit: `test(cli): add integration test for vocabulary promote blocked payload (#78)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `schemas/payload/vocabulary-promote-blocked.json` is committed
- [ ] Blocked promote returns `ok: false` with `payload.unresolvableKeys` array
- [ ] Successful promote output is unchanged from before

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `downcast_ref::<RepositoryError>()` on the `anyhow::Error` returned by `with_store` correctly identifies `VocabularyPromotionBlocked` at runtime. This is the established pattern used in `protocol.rs` and other handlers. The `?` operator in the closure converts `RepositoryError` into `anyhow::Error` via `impl From<RepositoryError> for anyhow::Error`, preserving the concrete type for `downcast_ref`.
- The `derive-tag-set` subcommand already computes what callers need; this plan does not expose it — a follow-up issue will track that.
- No callers currently depend on `payload` being absent from error envelopes for `vocabulary promote` (it was always `null`/absent before this change).
