# Plan: Add get_record_attachments service (fix R5 sourceRole nominal-string debt)

## Summary

`srs-gov get` determines which sourceRefs are attachment-type refs by filtering on the string literal `"attaches"` in the presentation layer, then cross-referencing the full `attachment list` payload client-side. Per capability-layering R5, semantic filtering belongs in a typed service function, not in a leaf client. This plan adds `get_record_attachments` to `srs-repository/attachment_service.rs`, exposes it via a new `srs record attachments <id>` CLI command and a WASM binding, and updates `srs-gov get` to call the service instead of doing client-side cross-referencing.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude Code |
| Repository Service Worker | Claude Code |
| CLI Worker | Claude Code |
| Bindings Worker | Claude Code |
| Verification Agent | Architecture Reviewer (Stage 7) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `get_record_attachments` uses typed input/output structs; no `serde_json::Value` parameters; validation in the service | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | New `RecordGetAttachmentsPayload` struct in `payload.rs`; golden schema generated and committed | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM binding is a thin `get_record_attachments` method on `SrsRepository`; calls same service as CLI | accepted |
| [ADR-034](../docs/adr/034-source-refs-in-record-extra.md) | Phase 1 accesses sourceRefs via `record.extra["sourceRefs"]` (not a typed field); deserialization failures surface as `RepositoryError::Serialize` following the same guard pattern as `resolve_document_view_attachments` | accepted |

No new ADRs are needed — this plan implements existing rules (ADR-010, ADR-011, ADR-013, ADR-034) and closes a known R5 violation.

---

## Contracts

### CLI output contract (ADR-011)

New command `srs record attachments <id>` is added. A new payload struct `RecordGetAttachmentsPayload` is defined in `crates/srs-cli/src/payload.rs` and `cargo run --bin generate-schemas` is run to produce `crates/srs-cli/schemas/payload/RecordGetAttachmentsPayload.json`. `cargo test --test payload_contracts` must pass after.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are added or modified. No schema sync required.

---

## Scope

- Add `GetRecordAttachmentsInput` and `GetRecordAttachmentsResult` structs in `crates/srs-repository/src/attachment_service.rs`
- Add `get_record_attachments(store, input) -> Result<Option<GetRecordAttachmentsResult>, RepositoryError>` service function
- Add `RecordGetAttachmentsPayload` struct in `crates/srs-cli/src/payload.rs`
- Add `RecordCommand::Attachments { id }` variant to the CLI enum in `crates/srs-cli/src/commands/mod.rs`
- Add `cmd_record_attachments` handler in `crates/srs-cli/src/commands/record.rs`
- Run `cargo run --bin generate-schemas` and commit the new `RecordGetAttachmentsPayload.json` golden file
- Update `srs-gov get` in `crates/srs-gov/src/main.rs` to call `run_srs(&["record", "attachments", id])` instead of the two-call client-side cross-referencing approach
- Add `get_record_attachments(&self, instance_id: &str) -> Result<JsValue, JsValue>` WASM binding in `crates/srs-bindings/src/lib.rs`
- MemoryStore unit tests for the service function
- FileStore roundtrip test for the service function
- WASM smoke test

**Out of scope:**

- `size_bytes` in the service result (file size not in SourceDocumentIndexEntry; requires separate filesystem walk — defer to a follow-up)
- Integrating attachment data into `srs record get` payload (different approach, deferred)
- Any changes to the SRS spec or JSON schema files

---

## Phases

### Phase 1: Service function

**Goal:** `get_record_attachments` exists in `srs-repository`, uses typed `SourceRole::Attaches` filtering, and all tests pass.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add `GetRecordAttachmentsInput { pub instance_id: String }` struct to `crates/srs-repository/src/attachment_service.rs`
- [ ] Add `GetRecordAttachmentsResult { pub source_documents_path: String, pub instance_id: String, pub attachments: Vec<ResolvedAttachment> }` struct to `crates/srs-repository/src/attachment_service.rs`. Add a doc comment: "Single-record analog to `RecordAttachments`; carries `source_documents_path` inline because the multi-record path (`ResolveDocumentViewAttachmentsResult`) surfaces it at the top level instead."
- [ ] Implement `pub fn get_record_attachments(store: &dyn RepositoryStore, input: GetRecordAttachmentsInput) -> Result<Option<GetRecordAttachmentsResult>, RepositoryError>` in `crates/srs-repository/src/attachment_service.rs`:
  - Load manifest for `source_documents_path` and `source_document_index`
  - Call `record_store::get_record_by_id(store, &input.instance_id)?` — return `Ok(None)` if absent
  - Parse `record.extra["sourceRefs"]` as `Vec<SourceReference>` (ADR-034: stored in extra map, not a typed field; map serde errors to `RepositoryError::Serialize`)
  - Filter by `r.source_role == Some(SourceRole::Attaches) && r.source_type == SourceType::RepositoryDocument` (typed enum, no string literals)
  - Cross-reference `r.source_id` against the source document index map
  - Return `Ok(Some(GetRecordAttachmentsResult { ... }))`
- [ ] Add MemoryStore test: `get_record_attachments_returns_none_for_missing_record` — call with a non-existent ID and assert `Ok(None)`
- [ ] Add MemoryStore test: `get_record_attachments_empty_when_no_source_refs` — record with no sourceRefs returns `Ok(Some(...))` with empty attachments
- [ ] Add MemoryStore test: `get_record_attachments_filters_by_attaches_role` — record with two sourceRefs (one `attaches`, one `evidence`) returns only the `attaches` ref
- [ ] Add MemoryStore test: `get_record_attachments_resolves_indexed_document` — record with `sourceRole: attaches` cross-references with source document index and returns `content_path`, `title`, etc.
- [ ] Add FileStore roundtrip test: `get_record_attachments_filestore_roundtrip` — create a temp repo with a record and linked attachment on disk; assert all fields are populated

#### Acceptance Criteria

- [ ] `get_record_attachments` uses `SourceRole::Attaches` (typed enum) — no `"attaches"` string literal in the new code
- [ ] Returns `Ok(None)` for a record ID not in the manifest index
- [ ] Returns `Ok(Some(...))` with empty `attachments` when the record has no qualifying sourceRefs
- [ ] Filters correctly: only `sourceRole: attaches` + `sourceType: repository-document` refs included
- [ ] Cross-references with source document index for title/content_path/checksums
- [ ] All named tests above pass
- [ ] `cargo test -p srs-repository` passes
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository attachment_service
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `get_record_attachments_returns_none_for_missing_record` — proves NotFound-style behavior
- `get_record_attachments_empty_when_no_source_refs` — proves empty result, not error
- `get_record_attachments_filters_by_attaches_role` — proves typed filtering (not string match)
- `get_record_attachments_resolves_indexed_document` — proves cross-reference against source doc index
- `get_record_attachments_filestore_roundtrip` — proves real filesystem works end-to-end

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm every test listed exists and passes: `cargo test -p srs-repository get_record_attachments`.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update plan checkboxes `[x]`.
5. Commit: `feat(srs-repository): add get_record_attachments service (fix R5 debt) (#618)`

---

### Phase 2: CLI command + payload

**Goal:** `srs record attachments <id>` is a working CLI command with a golden schema file.

**Agent:** CLI Worker

#### Tasks

- [ ] Add `RecordGetAttachmentsPayload` to `crates/srs-cli/src/payload.rs`:
  ```rust
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct RecordGetAttachmentsPayload {
      pub instance_id: String,
      pub source_documents_path: String,
      pub attachments: Vec<ResolvedAttachmentPayload>,
  }

  impl From<GetRecordAttachmentsResult> for RecordGetAttachmentsPayload {
      fn from(r: GetRecordAttachmentsResult) -> Self {
          Self {
              instance_id: r.instance_id,
              source_documents_path: r.source_documents_path,
              attachments: r.attachments.into_iter().map(ResolvedAttachmentPayload::from).collect(),
          }
      }
  }
  ```
  (`#[serde(rename_all = "camelCase")]` means `instance_id` serializes as `instanceId`, `source_documents_path` as `sourceDocumentsPath` in the JSON output.)
- [ ] Add `RecordCommand::Attachments { id: String }` variant to the `RecordCommand` enum in `crates/srs-cli/src/commands/mod.rs` below the `AllowedTransitions` variant, with the clap doc comment `/// List attachments linked to a record` on the line immediately above the variant (following the same doc-comment pattern as existing variants)
- [ ] Add `RecordCommand::Attachments { id } => cmd_record_attachments(ctx, id)` dispatch arm in `crates/srs-cli/src/commands/record.rs`
- [ ] Add handler `fn cmd_record_attachments(ctx: CliContext, id: String) -> Result<String>` in `crates/srs-cli/src/commands/record.rs`. `output::not_found` does not exist — the established pattern (record.rs lines 139–142, 296–299) is `Ok(output::err(...))`:
  ```rust
  fn cmd_record_attachments(ctx: CliContext, id: String) -> Result<String> {
      match with_store(&ctx, |store| {
          Ok(attachment_service::get_record_attachments(
              store,
              attachment_service::GetRecordAttachmentsInput { instance_id: id.clone() },
          )?)
      })? {
          Some(result) => output::serialize("record attachments", RecordGetAttachmentsPayload::from(result)),
          None => Ok(output::err(
              "record attachments",
              vec![format!("Record '{id}' not found")],
          )),
      }
  }
  ```
- [ ] Add import for `attachment_service` and `RecordGetAttachmentsPayload` in `crates/srs-cli/src/commands/record.rs`
- [ ] Run `cargo run --bin generate-schemas` and commit the new `crates/srs-cli/schemas/payload/RecordGetAttachmentsPayload.json`

#### Acceptance Criteria

- [ ] `srs record attachments <valid-id>` outputs a JSON envelope with `ok: true` and payload fields `instanceId`, `sourceDocumentsPath`, `attachments`
- [ ] `srs record attachments <unknown-id>` outputs a JSON envelope with `ok: false` / not-found error
- [ ] `RecordGetAttachmentsPayload.json` exists in `crates/srs-cli/schemas/payload/`
- [ ] `cargo test --test payload_contracts` passes
- [ ] `cargo clippy -p srs-cli -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

Specific tests to write or verify:

- `cargo test --test payload_contracts` — golden schema validation (run after `generate-schemas`)

#### Milestone gate

1. Verify all acceptance criteria.
2. Run:

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

3. Update plan checkboxes `[x]`.
4. Commit: `feat(srs-cli): add record attachments command (#618)`

---

### Phase 3: srs-gov update

**Goal:** `srs-gov get` no longer does client-side cross-referencing of sourceRefs against the attachment list; it calls `srs record attachments <id>` instead.

**Agent:** Lead Integrator

#### Tasks

- [ ] In `crates/srs-gov/src/main.rs`, change `resolve_linked_attachments` signature from `(record: &serde_json::Value, repo: &str)` to `(instance_id: &str, repo: &str)` — the instance ID is what the service call needs, not the full record JSON.
- [ ] Update the call site in `cmd_get` (currently line 450: `resolve_linked_attachments(record, repo)`) to instead extract `instance_id` from `record["instanceId"].as_str().unwrap_or_default()` and call `resolve_linked_attachments(instance_id, repo)`.
- [ ] Replace `resolve_linked_attachments` body with a single `run_srs` call:
  ```rust
  fn resolve_linked_attachments(instance_id: &str, repo: &str) -> Vec<render::LinkedAttachment> {
      let payload = match run_srs(&["record", "attachments", instance_id], repo, false, false) {
          Ok(p) => p,
          Err(err) => {
              // Graceful degradation: degrade to empty list on subprocess failure (same policy as before).
              eprintln!("warn: could not fetch record attachments: {err}");
              return vec![];
          }
      };
      let empty = vec![];
      let entries = payload["attachments"].as_array().unwrap_or(&empty);
      entries.iter().map(|e| render::LinkedAttachment {
          // RecordGetAttachmentsPayload.attachments[] uses camelCase field names in JSON
          document_id: e["documentId"].as_str().unwrap_or_default().to_string(),
          title: e["title"].as_str().map(String::from),
          content_path: e["contentPath"].as_str().map(String::from),
          size_bytes: None,  // not provided by the service; renders as "—"
      }).collect()
  }
  ```
- [ ] Remove `build_linked_attachments` (fully replaced by the service call) — delete the function and remove or update any tests that exercise it directly.

Note on error handling: maintain **graceful degradation** (return `vec![]` on subprocess failure). This matches the prior policy where attachment resolution failure degrades rather than fails the entire `get` command. The new policy drops the "fallback to doc IDs" behavior since the service itself returns a proper empty list for records with no attachments; the only failure case is a subprocess error.

Note on `size_bytes`: `size_bytes` is `None` after this change (the service doesn't include filesystem size). `render::linked_attachments` already handles `None` gracefully with `"—"`. This is a minor output change (deferred enhancement, not a regression).

#### Acceptance Criteria

- [ ] `srs-gov get <key> <id>` displays linked attachments using the new service path
- [ ] No `"attaches"` string literal remains in `srs-gov/src/main.rs` for filtering sourceRefs
- [ ] `build_linked_attachments` function is removed
- [ ] `resolve_linked_attachments` body no longer calls `srs attachment list` and no longer does manual `sourceRole == "attaches"` string comparison
- [ ] `cargo test -p srs-gov` passes
- [ ] `cargo clippy -p srs-gov -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests to write or verify:

- Update the `resolve_linked_attachments_empty_refs` and `resolve_linked_attachments_no_attaches_role` tests in `srs-gov/src/main.rs` to match the new call pattern (they may need adjustment or removal since the function signature changes)
- Verify tests for `build_linked_attachments` are removed

#### Milestone gate

1. Verify all acceptance criteria.
2. Run:

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

3. Update plan checkboxes `[x]`.
4. Commit: `fix(srs-gov): use record attachments service, drop client-side R5 filtering (#618)`

---

### Phase 4: WASM binding

**Goal:** `SrsRepository.get_record_attachments(instanceId)` is available in `srs-bindings`.

**Agent:** Bindings Worker

#### Tasks

- [ ] Add `use srs_repository::attachment_service::{get_record_attachments, GetRecordAttachmentsInput};` import in `crates/srs-bindings/src/lib.rs`
- [ ] Add method to `SrsRepository`:
  ```rust
  /// Return attachments linked to a single record (`sourceRole: attaches`).
  ///
  /// Returns `{instanceId, sourceDocumentsPath, attachments: [{documentId, contentPath,
  /// sidecarPath, title, contentChecksum, sidecarChecksum}]}`, or a JS error when the
  /// record is not found or the store cannot be read.
  #[wasm_bindgen]
  pub fn get_record_attachments(&self, instance_id: &str) -> Result<JsValue, JsValue> {
      let result = get_record_attachments(
          &self.store,
          GetRecordAttachmentsInput { instance_id: instance_id.to_string() },
      ).map_err(js_err)?;
      match result {
          Some(r) => to_js(&r),
          None => Err(js_err(format!("record '{}' not found", instance_id))),
      }
  }
  ```
- [ ] Add smoke test `get_record_attachments_smoke` in the `#[cfg(test)]` block of `crates/srs-bindings/src/lib.rs` that:
  - Creates a MemoryStore via `store_with_manifest`
  - Links an attachment to a record using `link_attachment`
  - Calls `get_record_attachments` with the record ID
  - Parses the JSON result and asserts `attachments.len() == 1`

#### Acceptance Criteria

- [ ] `SrsRepository::get_record_attachments` is `pub #[wasm_bindgen]` decorated
- [ ] Returns a `JsValue` containing `{instanceId, sourceDocumentsPath, attachments: [...]}`
- [ ] Returns a JS error string when record not found
- [ ] `cargo test -p srs-bindings` passes
- [ ] `cargo build --target wasm32-unknown-unknown -p srs-bindings` passes

#### Testing

```bash
cargo test -p srs-bindings
cargo build --target wasm32-unknown-unknown -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

Specific tests to write or verify:

- `get_record_attachments_smoke` — basic end-to-end through the binding surface

#### Milestone gate

1. Verify all acceptance criteria.
2. Run:

```bash
cargo test -p srs-bindings
cargo build --target wasm32-unknown-unknown -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

3. Update plan checkboxes `[x]`.
4. Commit: `feat(srs-bindings): add get_record_attachments WASM binding (#618)`

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (new payload struct added)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] No `"attaches"` string literal used for semantic filtering in `srs-gov/src/main.rs`
- [ ] `get_record_attachments` service uses typed `SourceRole::Attaches` enum throughout
- [ ] `srs record attachments <id>` returns the correct payload shape
- [ ] WASM target builds: `cargo build --target wasm32-unknown-unknown -p srs-bindings`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `SourceRole::Attaches` and `SourceType::RepositoryDocument` are already defined in `srs-core` (confirmed: `crates/srs-core/src/types/source_reference.rs`)
- `ResolvedAttachment`, `RecordAttachments`, `ResolvedAttachmentPayload` types already exist and are reused (confirmed: `attachment_service.rs` lines 400–427, `payload.rs` lines 2132–2178)
- `record_store::get_record_by_id` works for all record tiers (confirmed: searches all `instance_index` entries)
- `size_bytes` absence after the `srs-gov get` update is acceptable (renders as `—` in the UI)
