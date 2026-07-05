# Plan: Graduate note → record operation (#270)

## Summary

Tier-0 notes accumulate semantic content as understanding deepens. Once a note is fully
formalised, it should be promotable to a typed Tier-2 Record in one step rather than
three manual ones (create typed record + repoint identity pointer + manually stamp
`graduated_at`). This plan implements the `graduate` service, CLI command, and WASM
binding. The service creates the typed Record via `create_record_in_context`, stamps
`graduated_at` on the Note, and returns both. The `graduated_at` field already exists
in the `Note` core type and `note.json` schema; no spec change is required. Relation
creation is explicitly out of scope for this plan (see Architecture Decisions).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Repository Service Worker | `agents.md#repository-service-worker` |
| CLI Worker | `agents.md#cli-worker` |
| Bindings Worker | `agents.md#bindings-worker` |
| Verification | `agents.md#verification-agent` |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `graduate_note` is a single service: typed input → typed output, all orchestration inside. | accepted (governs) |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | New `NoteGraduatePayload` struct added to `payload.rs`; schema golden file generated. | accepted (governs) |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | `graduate_note` WASM binding calls the same service, no logic duplicated. | accepted (governs) |
| [capability-layering](../docs/architecture/capability-layering.md) | All note+record orchestration lives in `srs-repository`; CLI and WASM are adapters only. | active guidance (governs) |

**ADR check (every ADR in `docs/adr/`):** ADRs 001–021 reviewed. No new architectural
decision is required. The `graduate_note` operation is a multi-step service operation
under the established pattern (ADR-010): load note → create record → update note →
return typed result. No new crate boundary, no new extension model, no new payload
convention. The `graduated_at` timestamp is written using the same `chrono::Utc::now()
.to_rfc3339()` pattern already used throughout `srs-repository`.

**Decision — no `derived-from` Relation in V1.** The `note.json` schema notes that
"authoritative record of successors is in derived-from Relations from the successor
Records." Creating such a relation requires the repository to have a registered
`derived-from` RelationTypeDefinition (E1 validation). Most repos don't; creating the
relation in `graduate_note` would fail silently or error for every repo that hasn't
explicitly registered that RTD. The `graduated_at` timestamp is itself a sufficient
and queryable formalisation signal — it is the `Note` field the spec defines for this
purpose. Relation creation is therefore left as a future follow-up when a standardised
`derived-from` RTD exists in the governance scaffold.

---

## Contracts

### CLI output contract (ADR-011)

**New command added** — `srs note graduate`. A new payload struct `NoteGraduatePayload`
is added to `crates/srs-cli/src/payload.rs`. After adding it, run
`cargo run --bin generate-schemas` and commit the new
`crates/srs-cli/schemas/payload/note-graduate.json`.

### Entity schema sync (check-schema-sync.sh)

**No** — this plan adds no new JSON Schema files under `srs/docs/schema/2.0/`. The
`graduated_at` field already exists in `note.json`. No schema sync needed.

---

## Scope

- `GraduateNoteInput` and `GraduateNoteResult` structs and `graduate_note` service
  function in `crates/srs-repository/src/services.rs`.
- `NoteCommand::Graduate` variant in `crates/srs-cli/src/commands/mod.rs`.
- `cmd_note_graduate` handler in `crates/srs-cli/src/commands/note.rs`.
- `NoteGraduatePayload` struct in `crates/srs-cli/src/payload.rs` + generated golden schema.
- `SrsRepository::graduate_note` WASM binding in `crates/srs-bindings/src/lib.rs`.
- Cross-store roundtrip test in `srs-repository`.

**Out of scope:**

- Creating a `derived-from` Relation from the new Record to the Note (future work when
  a standard `derived-from` RTD exists in the governance scaffold).
- Auto-mapping note section content into record field values (section names ≠ field IDs;
  user supplies field values via stdin, as with `record create`).
- Repointing references from the Note ID to the Record ID in other records or relations.
- Preventing re-graduation (if a note already has `graduated_at`, the service updates it
  and creates another successor Record — idempotent creation of additional successors).

---

## Phases

### Phase 1: `srs-repository` service + input/result types

**Goal:** `graduate_note(store, GraduateNoteInput) -> Result<GraduateNoteResult>` is
implemented, tested with MemoryStore and FileStore.

**Agent:** Repository Service Worker (`agents.md#repository-service-worker`)

#### Tasks

- [ ] In `crates/srs-repository/src/services.rs`, add the following public types and function
  immediately after the `UpdateNoteResult` struct and `update_note_validated` function
  (line ~297):

  ```rust
  /// Input for `graduate_note`.
  pub struct GraduateNoteInput {
      /// The instance ID of the Tier-0 Note to graduate.
      pub note_id: String,
      /// Type reference in "namespace/name" format (passed to `create_record_in_context`).
      pub type_ref: String,
      /// Optional version pin for the type. None → latest.
      pub type_version: Option<u32>,
      /// Field values, group values, and tags for the new Record.
      pub record_input: CreateRecordInput,
      /// If Some, the new Record is added to this container after creation.
      pub container_id: Option<String>,
  }

  /// Result of `graduate_note`.
  pub struct GraduateNoteResult {
      /// The Note with `graduated_at` stamped (ISO-8601 UTC timestamp).
      pub note: Note,
      /// The newly created typed Record.
      pub record: Record,
  }
  ```

- [ ] Implement `pub fn graduate_note(store: &dyn RepositoryStore, input: GraduateNoteInput) -> Result<GraduateNoteResult, RepositoryError>` in `services.rs`:

  ```rust
  pub fn graduate_note(
      store: &dyn RepositoryStore,
      input: GraduateNoteInput,
  ) -> Result<GraduateNoteResult, RepositoryError> {
      // Step 1: load and validate the note exists and is actually a Note (tier 0)
      let mut note = match get_note_by_id(store, &input.note_id)? {
          GetNoteResult::Found(n) => *n,
          GetNoteResult::NotFound => return Err(RepositoryError::NoteNotFound {
              path: std::path::PathBuf::from("records/notes"),
              id: input.note_id.clone(),
          }),
          GetNoteResult::NotANote { tier } => return Err(RepositoryError::InvalidRepositoryInitialization {
              message: format!(
                  "Instance '{}' is not a Note (tier {}); cannot graduate",
                  input.note_id, tier
              ),
          }),
      };

      // Step 2: create the typed Record
      let create_result = create_record_in_context(
          store,
          &input.type_ref,
          input.type_version,
          input.record_input,
          input.container_id,
          None,
      )?;

      // Step 3: stamp graduated_at and write the note back
      note.graduated_at = Some(chrono::Utc::now().to_rfc3339());
      let update_result = update_note(store, note)?;

      Ok(GraduateNoteResult {
          note: update_result.note,
          record: create_result.record,
      })
  }
  ```

  Add these imports at the top of `services.rs` (they are NOT currently present):
  ```rust
  use chrono::Utc;
  use crate::record_store::{create_record_in_context, CreateRecordInput};
  ```
  `chrono` is already in `srs-repository/Cargo.toml`; no `Cargo.toml` change needed.

- [ ] Add `GraduateNoteResult` (and `GraduateNoteInput` if needed at the call site) to
  the `pub use` exports in `crates/srs-repository/src/lib.rs` or export directly.

#### Acceptance Criteria

- [ ] Calling `graduate_note` with a valid note ID and type creates a new Record and
  returns it paired with the updated Note that has `graduated_at` set.
- [ ] Calling `graduate_note` with a non-existent note ID returns
  `RepositoryError::NoteNotFound`.
- [ ] Calling `graduate_note` with an ID belonging to a non-Note (tier != 0) returns an
  appropriate `RepositoryError`.
- [ ] `graduated_at` on the returned Note is a valid ISO-8601 UTC timestamp string.
- [ ] Cross-store roundtrip test passes (MemoryStore create → FileStore verify, or
  FileStore write → reload from disk).

#### Testing

```bash
cargo test -p srs-repository graduate_note
cargo test -p srs-repository
```

Specific tests to write in `crates/srs-repository/src/services.rs` `#[cfg(test)]`,
using `MemoryStore` (the canonical test double per CLAUDE.md Storage Boundary Rules):

- `graduate_note_creates_record_and_stamps_graduated_at` — set up a repo with a note
  and a type; call `graduate_note`; assert `result.note.graduated_at.is_some()` and
  `result.record.instance_id` is non-empty.
- `graduate_note_not_found_returns_error` — call with a non-existent ID; assert
  `Err(RepositoryError::NoteNotFound { .. })`.
- `graduate_note_cross_store_roundtrip` — MemoryStore: create note + graduate; serialise
  to JSON; FileStore: load from JSON; assert `note.graduated_at` survives the roundtrip
  (matches the `MemoryStore` → `srsj` → reload pattern in existing tests).

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Mark checkboxes `[x]`, commit:
`feat(repository): graduate_note service — creates Record, stamps graduated_at (#270)`

---

### Phase 2: CLI command + payload

**Goal:** `srs note graduate <id> --type <namespace/name>` is available, reads
`CreateRecordInput` from stdin, and emits `NoteGraduatePayload`.

**Agent:** CLI Worker (`agents.md#cli-worker`)

#### Tasks

- [ ] In `crates/srs-cli/src/commands/mod.rs`, add `Graduate` to `NoteCommand` after
  the `Foundations` variant:

  ```rust
  /// Graduate a note to a typed record (reads CreateRecordInput JSON from stdin)
  Graduate {
      /// Note instance ID to graduate
      id: String,
      /// Target type in namespace/name format (e.g. com.example/article)
      #[arg(long = "type", visible_alias = "type-ref")]
      type_ref: String,
      /// Optional type version override (defaults to latest)
      #[arg(long)]
      type_version: Option<u32>,
  },
  ```

  (The container ID comes from the global `ctx.container_id` flag, not a local flag —
  consistent with `record create`.)

- [ ] In `crates/srs-cli/src/payload.rs`, add `NoteGraduatePayload`:

  ```rust
  #[derive(Debug, Serialize, Deserialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct NoteGraduatePayload {
      pub note: Note,
      #[schemars(with = "serde_json::Value")]
      pub record: Record,
  }
  ```

- [ ] In `crates/srs-cli/src/commands/note.rs`:
  - Add `Graduate { id, type_ref, type_version }` to the `dispatch` match arm:
    `NoteCommand::Graduate { id, type_ref, type_version } => cmd_note_graduate(ctx, id, type_ref, type_version),`
  - Add to imports: `use srs_repository::services::{graduate_note, GraduateNoteInput, GraduateNoteResult}` (verify exact path).
  - Add `NoteGraduatePayload` to the `payload` imports.
  - Implement `cmd_note_graduate`:

    ```rust
    fn cmd_note_graduate(
        ctx: CliContext,
        id: String,
        type_ref: String,
        type_version: Option<u32>,
    ) -> Result<String> {
        let mut stdin = String::new();
        io::stdin().read_to_string(&mut stdin)?;
        let record_input: CreateRecordInput = serde_json::from_str(&stdin)
            .map_err(|e| anyhow::anyhow!("Failed to parse record input JSON: {}", e))?;
        let container_id = ctx.container_id.clone();
        match with_store(&ctx, |store| {
            Ok(graduate_note(store, GraduateNoteInput {
                note_id: id.clone(),
                type_ref: type_ref.clone(),
                type_version,
                record_input,
                container_id,
            })?)
        }) {
            Ok(GraduateNoteResult { note, record }) => {
                output::serialize("note graduate", NoteGraduatePayload { note, record })
            }
            Err(e) => Ok(output::err("note graduate", vec![e.to_string()])),
        }
    }
    ```

- [ ] Run `cargo run --bin generate-schemas` from `srs-rust/` and commit the new file
  `crates/srs-cli/schemas/payload/note-graduate.json`.

#### Acceptance Criteria

- [ ] `srs note graduate <id> --type namespace/name` (with valid `CreateRecordInput` JSON
  from stdin) prints a `NoteGraduatePayload` with `ok: true` and both `note`
  (with `graduatedAt` set) and `record` (with the new instance ID).
- [ ] Handler is ≤ ~15 lines: parse stdin → one service call → `output::serialize`.
- [ ] `cargo test --test payload_contracts` passes (golden schema committed).

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
```

Specific tests (in `crates/srs-cli/src/commands/note.rs` `#[cfg(test)]`, following
the existing handler test style):

- `cmd_note_graduate_returns_note_and_record` — set up a MemoryStore-backed `CliContext`
  with a note and a type; pipe valid `CreateRecordInput` JSON; assert the output payload
  has `ok: true` and `data.note.graduatedAt` is present.
- `cmd_note_graduate_unknown_note_returns_error` — unknown note ID; assert `ok: false`.

#### Milestone gate

```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
cargo test --test payload_contracts
```

Mark checkboxes `[x]`, commit:
`feat(cli): note graduate command — promote note to typed record (#270)`

---

### Phase 3: WASM binding

**Goal:** `SrsRepository.graduate_note(note_id, type_ref, input_json)` is available in
the WASM surface and calls the same service.

**Agent:** Bindings Worker (`agents.md#bindings-worker`)

#### Tasks

- [ ] In `crates/srs-bindings/src/lib.rs`, import `services::{graduate_note, GraduateNoteInput}`
  (confirm exact path from `crates/srs-repository/src/lib.rs` pub-use exports).
- [ ] Add to `SrsRepository` impl:

  ```rust
  /// Graduate a Note to a typed Record. `input_json` is a `CreateRecordInput` JSON
  /// object (`fieldValues`, `groupValues?`, `tags?`). Returns a `NoteGraduateResult`
  /// object: `{ note, record }` where `note` has `graduatedAt` stamped.
  #[wasm_bindgen]
  pub fn graduate_note(
      &self,
      note_id: &str,
      type_ref: &str,
      type_version: Option<u32>,
      input_json: &str,
  ) -> Result<JsValue, JsValue> {
      let record_input: CreateRecordInput =
          serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
      let result = graduate_note_service(&self.store, GraduateNoteInput {
          note_id: note_id.to_string(),
          type_ref: type_ref.to_string(),
          type_version,
          record_input,
          container_id: None,
      })
      .map_err(js_err)?;
      to_js(&result)
  }
  ```

  Note: `graduate_note` the service function and `graduate_note` the method clash in
  name. Use a local alias: `use srs_repository::services::graduate_note as
  graduate_note_service;` or call it via the full path.

- [ ] Ensure `GraduateNoteResult` derives `Serialize` so `to_js` can serialise it.

#### Acceptance Criteria

- [ ] Binding compiles for the workspace target (WASM target or native test target).
- [ ] A smoke test asserts the binding output contains `note.graduatedAt` and `record`.
- [ ] No business logic in the binding — it's one `from_str` + one service call + `to_js`.

#### Testing

```bash
cargo test -p srs-bindings
```

Specific tests (in `crates/srs-bindings/src/lib.rs` `#[cfg(test)]`, following the
existing smoke-test style in that file):

- `graduate_note_binding_smoke` — set up a repo with a note and a type; call
  `graduate_note_service` directly (not via the `#[wasm_bindgen]` method, which
  requires a JS runtime); assert the result fields are present. Mirror the pattern of
  existing service-call tests in `srs-bindings`.

#### Milestone gate

```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

Mark checkboxes `[x]`, commit:
`feat(bindings): graduate_note WASM binding (#270)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (golden schema committed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed — no-op)
- [ ] `srs note graduate <id> --type ns/name` produces a typed Record and stamps
  `graduated_at` on the Note (issue acceptance criterion)
- [ ] Cross-store roundtrip test passes in Phase 1

## Coordination Rules

- Phase order is strict: service first (defines types used by CLI and binding), then CLI, then WASM binding.
- Write scopes: Repository → `srs-repository`, CLI → `srs-cli`, Bindings → `srs-bindings`.
- Lead Integrator owns final naming: `graduate_note`, `GraduateNoteInput`, `GraduateNoteResult`, `NoteGraduatePayload`.

## Assumptions

- `chrono` is already in `srs-repository/Cargo.toml` but NOT yet imported in `services.rs`; the implementation adds `use chrono::Utc;` to the imports.
- `update_note(store, note) -> Result<UpdateNoteResult>` is `pub fn` in `services.rs` and callable by `graduate_note` in the same file.
- `create_record_in_context` is accessible from `services.rs` via `record_store::create_record_in_context` or equivalent import (confirm at implementation time).
- `GraduateNoteResult` will need `#[derive(Serialize)]` so `to_js` can serialise it; `Record` and `Note` already implement `Serialize`.
- `CreateRecordInput` from `record_store.rs` is already re-exported from `srs-repository/src/lib.rs` or accessible as `crate::record_store::CreateRecordInput` within `srs-bindings`.
