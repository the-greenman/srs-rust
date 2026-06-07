# Plan: RFC-006 Task 3 — Vocabulary Substrate Rust Implementation

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

Implements the remaining Rust components of RFC-006 (Vocabulary Substrate, Accepted Rev 8). The core types, service CRUD operations, CLI commands for vocabulary/lifecycle, and payload structs are already implemented (landed in srs-rust PR #70). This plan covers what was not yet built: validation invariants V2/V5/V7/V8/V9, the `srs term` subcommand, service unit tests and integration tests, V10 open→closed promotion pre-flight, and the `vocabulary promote` command. Spec records and schema changes are already merged (srs PR #19).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | `srs-repository` validation + service |
| CLI Worker | `srs-cli` term command + vocabulary promote |
| Verification | Verification Agent (after each phase) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs are needed — all choices are governed by existing ADRs.

| ADR | Decision | Status |
|---|---|---|
| [ADR-007](../docs/adr/007-file-index-io-ordering.md) | Write file before updating index; `promote_vocabulary` is a pure file read-modify-write (no index change) | accepted |
| [ADR-009](../docs/adr/009-package-boundary-model.md) | Services address vocabularies/lifecycles via `PackageSelector`; no raw file paths | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions take typed input structs; no business logic in CLI handlers | proposed (binding) |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | All CLI output via named payload structs in `payload.rs`; golden schema files committed | accepted |
| [ADR-012](../docs/adr/012-vocabulary-substrate.md) | Term/Vocabulary/Lifecycle are package-level definitions; TagDefinition write ops deprecated | accepted |

---

## Contracts

### CLI output contract (ADR-011)

This plan adds new commands and payload structs in `crates/srs-cli/src/payload.rs`.

**New payload structs:**

The canonical patterns in `payload.rs` for list and get commands with `srs-core` types:
- List payloads: `Vec<T>` field with `#[schemars(with = "Vec<serde_json::Value>")]` (matches `VocabularyListPayload`, `TagListPayload`)
- Get payloads: tagged enum with `#[schemars(with = "serde_json::Value")]` on the inner type field (matches `VocabularyGetPayload`, `LifecycleGetPayload`)

```rust
// term list — matches TagListPayload and VocabularyListPayload exactly
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TermListPayload {
    #[schemars(with = "Vec<serde_json::Value>")]
    pub terms: Vec<Term>,
}

// term get — matches the VocabularyGetPayload / LifecycleGetPayload pattern exactly
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "result")]
pub enum TermGetPayload {
    #[serde(rename = "found")]
    Found {
        #[schemars(with = "serde_json::Value")]
        term: Box<Term>,
    },
    #[serde(rename = "not_found")]
    NotFound { id: String },
}

// vocabulary promote — matches VocabularyCreatePayload pattern
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromoteVocabularyPayload {
    #[schemars(with = "serde_json::Value")]
    pub vocabulary: Vocabulary,
}
```

Note: `Term` and `Vocabulary` live in `srs-core` which has no `schemars` dep (ADR-011). The `#[schemars(with = ...)]` attributes bridge this. Follow the existing patterns exactly — do not use `Vec<serde_json::Value>` as the field type; use `Vec<Term>` / `Box<Term>` / `Vocabulary` as the Rust type with the schema override annotation.

After adding structs: run `cargo run --bin generate-schemas` and commit the new files in `crates/srs-cli/schemas/payload/`: `term-list.json`, `term-get.json`, `vocabulary-promote.json`.

Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No entity schemas under `srs/docs/schema/2.0/` change in this plan. Run `bash scripts/check-schema-sync.sh` for confirmation; expect exit 0.

---

## Scope

**In scope:**
- Validation invariants V2, V5, V7, V8, V9 in `crates/srs-repository/src/validation.rs`
- `srs term list` and `srs term get` commands in `crates/srs-cli/src/commands/term.rs`
- `TermListPayload` and `TermGetPayload` in `crates/srs-cli/src/payload.rs`
- `collect_tag_key_counts()` private helper in `vocabulary_service.rs` returning `HashMap<String, usize>` (shared by `derive_tag_set` and V10 pre-flight)
- `derive_tag_set(store, DeriveTagSetInput)` service function in `vocabulary_service.rs` (internal, consumed by `promote_vocabulary`)
- Unit tests for `vocabulary_service.rs` and `lifecycle_service.rs`
- Integration tests for `srs vocabulary list/get/create`, `srs vocabulary term-create`, `srs lifecycle list/get`, `srs term list/get`
- V10 open→closed promotion pre-flight in `promote_vocabulary(store, PromoteVocabularyInput)` service function
- `srs vocabulary promote <id>` CLI command in `crates/srs-cli/src/commands/vocabulary.rs`
- `PromoteVocabularyPayload` in `payload.rs`
- New `RepositoryError::VocabularyPromotionBlocked` variant in `crates/srs-repository/src/error.rs`

**Out of scope:**
- Removing `TagDefinition` struct or deprecated `tag_service` functions (ADR-012 defers final removal)
- Cross-vocabulary term relations (deferred in RFC-006)
- `srs term create/update/delete` CLI commands (terms are created via `vocabulary term-create`)
- `srs lifecycle create/update/delete` commands
- Making `derive_tag_set` a user-facing CLI command (`srs vocabulary derive-tag-set`) — V10 pre-flight is the immediate consumer; a future plan may expose it

---

## Phases

### Phase 1: Validation invariants V2, V5, V7, V8, V9

**Goal:** `srs repo validate` enforces RFC-006 invariants V2, V5, V7, V8, V9 and produces diagnostics for violations.

**Agent:** Repository Service Worker

#### Tasks

All changes in `crates/srs-repository/src/validation.rs`.

- [ ] Add a private function `validate_vocabulary_invariants(pkg: &Package, diagnostics: &mut Vec<ValidationDiagnostic>)` and call it from `validate_repository` after the package is loaded (after the existing relation validation block).

- [ ] **V2** — inside `validate_vocabulary_invariants`, for each `Field` in `pkg.fields` where `field.vocabulary_ref.is_some()`:
  - Match by UUID: check `pkg.vocabularies` contains a `Vocabulary` with `v.id == field.vocabulary_ref.as_deref().unwrap()`.
  - Do NOT attempt name-based resolution — consistent with `Package::resolve_vocabulary(id)` which matches by UUID only.
  - If no match: push `DiagnosticSeverity::Error` with message `format!("V2: field '{}' vocabularyRef '{}' does not resolve to an installed Vocabulary", field.name, ref_id)`.

- [ ] **V5** — inside `validate_vocabulary_invariants`, for each `Vocabulary` in `pkg.vocabularies`:
  - Collect the effective entry set via `vocab.effective_terms()` (non-retired terms).
  - Build a `HashSet<&str>` of all keys and aliases seen so far.
  - For each term in effective entry set: check `term.key`; if already in set, push Error `format!("V5: vocabulary '{}' has duplicate key '{}'", vocab.name, term.key)`. Then check each alias in `term.aliases`; if already in set, push Error. Insert `term.key` and all aliases into the set.
  - This catches: key-key, key-alias, alias-key, and alias-alias duplicates.

- [ ] **V7** — inside `validate_vocabulary_invariants`, for each `RecordType` in `pkg.record_types` (note: field is `pkg.record_types`, NOT `pkg.types`) where `rt.lifecycle_ref.is_some()`:
  - Match by UUID: check `pkg.lifecycles` contains a `Lifecycle` with `lc.id == rt.lifecycle_ref.as_deref().unwrap()`.
  - If no match: push `DiagnosticSeverity::Error` with message `format!("V7: type '{}' lifecycleRef '{}' does not resolve to an installed Lifecycle", rt.name, ref_id)`.
  - Track which types have unresolved `lifecycleRef` in a `HashSet<String>` keyed by type id — used in V8 to skip records whose type failed V7.

- [ ] **V8** — inside the existing tier-2 record validation branch of `validate_repository` (where `Record` is already deserialized from the loaded instance file), after the existing schema validation and before moving to the next instance:
  - If `record.lifecycle_state.is_none()`: skip V8.
  - Resolve the type via `package.resolve_type(&record.type_id, record.type_version)` (already available at this point in the loop).
  - If the type had an unresolved `lifecycleRef` (i.e., its type id is in the V7 failure set from `validate_vocabulary_invariants`): skip V8 (V7 already reported the broken ref).
  - Determine the effective lifecycle:
    - If `rt.lifecycle_ref.is_some()`: look up `pkg.resolve_lifecycle(lifecycle_ref)` (use `Package::resolve_lifecycle(id: &str) -> Option<&Lifecycle>`).
    - Else if `rt.lifecycle.is_some()`: use the inline lifecycle's states directly.
    - If neither: skip V8 (type has no lifecycle).
  - Check if `record.lifecycle_state.as_deref().unwrap()` is a valid state key in the effective lifecycle's `states`: `lc.states.iter().any(|s| s.key == state_value && !s.is_retired())`.
  - If not valid: push `DiagnosticSeverity::Error` with message `format!("V8: record '{}' lifecycleState '{}' is not a valid state key in the resolved lifecycle", instance_id, state_value)`.

- [ ] **V9** — inside `validate_vocabulary_invariants`, for each `Lifecycle` in `pkg.lifecycles`:
  - Count states with `is_initial == Some(true)`. If count != 1: push Error `format!("V9: lifecycle '{}' must have exactly one isInitial state (found {})", lc.name, count)`.
  - If count == 1: check `lc.initial_state == initial_state_key`. If mismatch: push Error `format!("V9: lifecycle '{}' initialState '{}' does not match isInitial state key '{}'", lc.name, lc.initial_state, initial_state_key)`.

- [ ] Add 15 unit tests in `validation.rs` (inside the existing `#[cfg(test)]` block):
  - `vocabulary_v2_missing_vocabulary_ref_produces_error` — field with `vocabularyRef: "nonexistent-uuid"`, expect Error containing "V2"
  - `vocabulary_v2_resolved_vocabulary_ref_no_error` — field with `vocabularyRef` matching installed vocab UUID
  - `vocabulary_v5_duplicate_key_produces_error` — vocabulary with two terms sharing same `key`
  - `vocabulary_v5_duplicate_alias_produces_error` — vocabulary where term2's alias matches term1's key
  - `vocabulary_v5_duplicate_alias_alias_produces_error` — vocabulary where term1 and term2 both have the same alias
  - `vocabulary_v5_retired_term_excluded_from_uniqueness` — retired term with same key as active term is not a conflict (no error)
  - `vocabulary_v7_missing_lifecycle_ref_produces_error` — type with `lifecycleRef: "nonexistent-uuid"`, expect Error containing "V7"
  - `vocabulary_v7_resolved_lifecycle_ref_no_error` — type with `lifecycleRef` matching installed lifecycle UUID
  - `lifecycle_v9_zero_initial_states_produces_error` — lifecycle with zero `isInitial: true` states
  - `lifecycle_v9_multiple_initial_states_produces_error` — lifecycle with two `isInitial: true` states
  - `lifecycle_v9_single_initial_state_no_error` — lifecycle with exactly one `isInitial: true` state and matching `initialState`
  - `lifecycle_v9_initial_state_key_mismatch_produces_error` — lifecycle with one `isInitial: true` state but `initialState` field pointing at a different key
  - `record_v8_invalid_lifecycle_state_produces_error` — record with `lifecycleState: "nonexistent"` and a type with an inline lifecycle
  - `record_v8_valid_lifecycle_state_no_error` — record with valid `lifecycleState` key
  - `record_v8_no_lifecycle_skips_check` — record with `lifecycleState` set but type has no lifecycle → no V8 error

  *(That is 15 tests — update acceptance criteria count accordingly.)*

#### Acceptance Criteria

- [ ] `cargo test -p srs-repository` — all 15 new tests pass, no existing tests regress
- [ ] `srs repo validate --repo ../srs/srs` reports 0 errors (the spec repo has no `lifecycleRef` / `vocabularyRef` fields; no false positives)
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
# Integration smoke check
cargo run --bin srs -- repo validate --repo ../srs/srs --pretty
```

#### Milestone gate

1. Verify all 15 tests exist and pass.
2. Run `srs repo validate --repo ../srs/srs` — confirm 0 errors.
3. Run `cargo clippy -p srs-repository -- -D warnings`.
4. Update plan checkboxes `[x]`.
5. Commit: `feat(validation): RFC-006 invariants V2/V5/V7/V8/V9 (#73)`

---

### Phase 2: `term` subcommand, service tests, and integration tests

**Goal:** `srs term list` and `srs term get` commands exist; vocabulary/lifecycle services have unit tests; all vocabulary/lifecycle/term CLI commands have integration tests.

**Agent:** CLI Worker + Repository Service Worker

#### Tasks

**Repository Service Worker — vocabulary_service.rs:**

- [ ] Add `#[cfg(test)] mod tests` block to `crates/srs-repository/src/vocabulary_service.rs`. In the test block, import `MemoryStore` via `use crate::store::MemoryStore;` (same crate, `#[cfg(test)]` items are visible to sibling `#[cfg(test)]` blocks). Tests:
  - `list_vocabularies_empty_when_no_package` — fresh `MemoryStore::default()` → `list_vocabularies` returns `Ok(vec![])`
  - `create_vocabulary_assigns_id_and_writes_file` — vocabulary with empty id gets a UUID assigned; `get_vocabulary_by_id` returns it after creation
  - `create_vocabulary_roundtrips_via_file_store` — create on `MemoryStore`, serialize the stored file to JSON, deserialize as `Vocabulary`, confirm `id`, `name`, `mode`, `terms` survive
  - `create_term_appends_to_vocabulary` — create vocabulary, call `create_term`, then `list_terms` → term appears
  - `get_vocabulary_by_id_finds_created` — create + get by id → id matches
  - `list_terms_returns_terms_across_vocabularies` — create two vocabularies each with one term → `list_terms` returns both terms

- [ ] Add `#[cfg(test)] mod tests` block to `crates/srs-repository/src/lifecycle_service.rs`:
  - `list_lifecycles_empty_when_no_package` — fresh `MemoryStore::default()` → returns `Ok(vec![])`
  - `get_lifecycle_by_id_returns_none_when_missing` — get unknown UUID → returns `Ok(None)`
  - `lifecycle_roundtrips_json` — construct `Lifecycle` with states + transitions, serialize to JSON, deserialize, confirm states and `initial_state` survive (pure serde roundtrip, no store needed)

**CLI Worker — term command:**

- [ ] Add to `crates/srs-cli/src/payload.rs` (after the existing `TagPayload` block, matching the `VocabularyListPayload` / `VocabularyGetPayload` pattern exactly — same derives, same serde attributes):
  ```rust
  // ── Term payloads (RFC-006) ────────────────────────────────────────────────
  /// Payload for `term list`.
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct TermListPayload {
      pub terms: Vec<Term>,
  }
  
  /// Payload for `term get`.
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase", tag = "result")]
  pub enum TermGetPayload {
      #[serde(rename = "found")]
      Found {
          #[schemars(with = "serde_json::Value")]
          term: Box<Term>,
      },
      #[serde(rename = "not_found")]
      NotFound { id: String },
  }
  ```
  Note: `TermListPayload.terms: Vec<Term>` follows the same pattern as `VocabularyListPayload.vocabularies: Vec<Vocabulary>` — `Term` implements `Serialize` but not `JsonSchema`; the golden schema file will show `terms` as an array of `{}` (same as vocabularies). This is acceptable per ADR-011.

- [ ] Create `crates/srs-cli/src/commands/term.rs`:
  ```rust
  use crate::commands::{with_store, CliContext, TermCommand};
  use crate::output;
  use crate::payload::{TermGetPayload, TermListPayload};
  use anyhow::Result;
  use srs_repository::vocabulary_service;
  
  pub fn dispatch(ctx: CliContext, cmd: TermCommand) -> Result<String> {
      match cmd {
          TermCommand::List => cmd_term_list(ctx),
          TermCommand::Get { id } => cmd_term_get(ctx, id),
      }
  }
  
  fn cmd_term_list(ctx: CliContext) -> Result<String> {
      let terms = with_store(&ctx, |store| Ok(vocabulary_service::list_terms(store)?))?;
      output::serialize("term list", TermListPayload { terms })
  }
  
  fn cmd_term_get(ctx: CliContext, id: String) -> Result<String> {
      match with_store(&ctx, |store| Ok(vocabulary_service::get_term_by_id(store, &id)?))? {
          Some(term) => output::serialize("term get", TermGetPayload::Found { term: Box::new(term) }),
          None => output::serialize("term get", TermGetPayload::NotFound { id }),
      }
  }
  ```
  Note: `list_terms` and `get_term_by_id` already exist in `vocabulary_service.rs` — do NOT re-implement them.

- [ ] Add to `crates/srs-cli/src/commands/mod.rs`:
  1. `pub mod term;` in the module list.
  2. Add `Term(TermCommand)` variant to the `Commands` enum (with doc comment "Term definition commands (RFC-006)").
  3. Add `TermCommand` enum — do NOT include a `json: bool` flag (term commands are new; JSON is the default):
     ```rust
     #[derive(Subcommand)]
     pub enum TermCommand {
         /// List all terms from all package vocabularies
         List,
         /// Get a term by id
         Get {
             /// Term UUID id
             id: String,
         },
     }
     ```
  4. Add `Commands::Term(term_cmd) => term::dispatch(ctx, term_cmd)` to the dispatch `match` block.

- [ ] Run `cargo run --bin generate-schemas` — confirm `schemas/payload/term-list.json` and `schemas/payload/term-get.json` are created. Stage and commit them as part of this phase's milestone commit.

**Integration tests (integration_tests.rs):**

All tests added to `crates/srs-cli/tests/integration_tests.rs`:

- [ ] `vocabulary_list_returns_ok_envelope` — run `srs vocabulary list` on empty repo; assert `result["ok"] == true` and `result["payload"]["vocabularies"].is_array()`.
- [ ] `vocabulary_create_writes_and_returns_vocabulary` — pipe `{"id":"","version":1,"namespace":"com.test","name":"my-vocab","mode":"open","terms":[],"createdAt":""}` to `srs vocabulary create`; assert `ok: true` and `payload.vocabulary.id` is non-empty UUID.
- [ ] `vocabulary_get_returns_created_vocabulary` — create vocabulary, run `srs vocabulary get <id>`; assert `payload["result"] == "found"` and `payload["vocabulary"]["id"]` matches.
- [ ] `vocabulary_get_not_found` — run `srs vocabulary get 00000000-0000-0000-0000-000000000000`; assert `ok: true` (command ran) and `payload["result"] == "not_found"`.
- [ ] `vocabulary_term_create_appends_term` — create vocabulary, run `srs vocabulary term-create --vocabulary-id <id>` with term JSON; then run `srs vocabulary get <id>`; assert `payload["result"] == "found"` and `payload["vocabulary"]["terms"]` has length 1 with the term's key present.
- [ ] `lifecycle_list_returns_ok_envelope` — run `srs lifecycle list` on empty repo; assert `ok: true` and `payload["lifecycles"].is_array()`.
- [ ] `lifecycle_get_not_found` — run `srs lifecycle get 00000000-0000-0000-0000-000000000000`; assert `ok: true` and `payload["result"] == "not_found"`.
- [ ] `term_list_returns_ok_envelope` — run `srs term list` on empty repo; assert `ok: true` and `payload["terms"].is_array()`.
- [ ] `term_list_returns_terms_from_vocabulary` — create vocabulary with one term via `vocabulary term-create`, run `srs term list`; assert `payload["terms"]` has length ≥ 1 and contains the term's key.
- [ ] `term_get_returns_term_by_id` — create term via `vocabulary term-create`, run `srs term get <term-id>`; assert `payload["result"] == "found"` and `payload["term"]["key"]` matches.

#### Acceptance Criteria

- [ ] `srs term list` and `srs term get` commands exist and return correct envelopes
- [ ] `cargo test --test payload_contracts` passes (term-list.json and term-get.json committed)
- [ ] All 10 integration tests pass
- [ ] All 9 service unit tests pass
- [ ] `cargo clippy -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository
cargo test --test integration_tests vocabulary_list
cargo test --test integration_tests vocabulary_create
cargo test --test integration_tests vocabulary_get
cargo test --test integration_tests vocabulary_term
cargo test --test integration_tests lifecycle_list
cargo test --test integration_tests lifecycle_get
cargo test --test integration_tests term_list
cargo test --test integration_tests term_get
cargo test --test payload_contracts
cargo clippy -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria.
2. Confirm `schemas/payload/term-list.json` and `term-get.json` exist and are staged.
3. Run `cargo test` and `cargo clippy -- -D warnings`.
4. Update plan checkboxes `[x]`.
5. Commit: `feat(term): term list/get commands and vocabulary/lifecycle service tests (#73)`

---

### Phase 3: V10 promotion pre-flight and `vocabulary promote` command

**Goal:** `srs vocabulary promote <id>` transitions a vocabulary from `mode: open` to `mode: closed`, with pre-flight validation that all in-use tag keys resolve in the closed vocabulary (V10). Note: V10 is a promotion-time gate; the existing C4 tag-enforcement check in `validate_repository` is an ongoing per-validate check. Both may fire for the same key — this is intentional.

**Agent:** Repository Service Worker + CLI Worker

#### Tasks

**Repository Service Worker — vocabulary_service.rs:**

- [ ] Add `RepositoryError::VocabularyPromotionBlocked { vocabulary_id: String, unresolvable_keys: Vec<String> }` to `crates/srs-repository/src/error.rs`. Update the `PartialEq` impl (which is a manual exhaustive match) to add a match arm for the new variant. Add `#[error("vocabulary promotion blocked: {vocabulary_id} has unresolvable keys: {unresolvable_keys:?}")]` attribute.

- [ ] Add a private helper in `vocabulary_service.rs`:
  ```rust
  /// Returns a map of tag-key → occurrence-count across all manifest index entries.
  /// Reads the manifest only (no per-instance file loads). Used by derive_tag_set and promote_vocabulary.
  fn collect_tag_key_counts(store: &dyn RepositoryStore) -> Result<HashMap<String, usize>, RepositoryError> {
      let manifest = store.load_manifest()?;
      let mut counts: HashMap<String, usize> = HashMap::new();
      for entry in &manifest.instance_index {
          if let Some(tags) = &entry.tags {
              for tag in tags {
                  *counts.entry(tag.clone()).or_insert(0) += 1;
              }
          }
      }
      Ok(counts)
  }
  ```
  Note: `store.load_manifest()` is a `RepositoryStore` trait method — calling it here is consistent with the existing pattern in `vocabulary_service::create_term`, which also calls `store.load_package_json()` directly. This is the established baseline for vocabulary service functions in this codebase.
  This helper is used by both `derive_tag_set` and `promote_vocabulary` below. Do not re-implement the scan inline in either function.

- [ ] Add input/result types in `vocabulary_service.rs`:
  ```rust
  pub struct DeriveTagSetInput { pub vocabulary_id: String }
  pub struct TagSetEntry { pub key: String, pub usage_count: usize, pub term: Option<Term> }
  pub struct DeriveTagSetResult { pub vocabulary: Vocabulary, pub entries: Vec<TagSetEntry> }
  pub struct PromoteVocabularyInput { pub vocabulary_id: String }
  pub struct PromoteVocabularyResult { pub vocabulary: Vocabulary }
  ```

- [ ] Add `derive_tag_set(store: &dyn RepositoryStore, input: DeriveTagSetInput) -> Result<DeriveTagSetResult, RepositoryError>`:
  - Load vocabulary by `input.vocabulary_id` (error if not found).
  - Call `collect_tag_key_counts(store)?` to get tag key → occurrence-count map.
  - For each (key, count) pair in the map: resolve via `vocab.resolve_term_by_key(key)` to get `term: Option<&Term>`. Build `TagSetEntry { key, usage_count: count, term: term.cloned() }`.
  - For each term in `vocab.effective_terms()` not already in the used-key map: add with `usage_count: 0`.
  - Return `DeriveTagSetResult { vocabulary, entries }`.

- [ ] Add `promote_vocabulary(store: &dyn RepositoryStore, input: PromoteVocabularyInput) -> Result<PromoteVocabularyResult, RepositoryError>`:
  - Load vocabulary by `input.vocabulary_id` (error if not found).
  - If `vocabulary.mode == VocabularyMode::Closed`: return `Err(RepositoryError::InvalidRepositoryInitialization { message: "vocabulary is already closed".to_string() })`.
  - **V10 pre-flight**: call `collect_tag_key_counts(store)?` to get all used keys.
  - For each used key: call `vocab.resolve_term_by_key(key)`.
    - If resolves to `None`: add to `unresolvable` list.
    - If resolves to a `retired` term: add to `unresolvable` list (retired entries cannot be read — hard error).
    - If resolves to a `deprecated` or `tombstone` term: note as warning (do not block; these terms resolve for reads but not writes).
  - If `!unresolvable.is_empty()`: return `Err(RepositoryError::VocabularyPromotionBlocked { vocabulary_id: input.vocabulary_id, unresolvable_keys: unresolvable })`.
  - If pre-flight passes: set `vocabulary.mode = VocabularyMode::Closed`.
  - **Read-modify-write the vocabulary file** (same pattern as `create_term`):
    - Find the vocabulary file path via `store.load_package_json()` → scan `vocabularies[]` array → load each and check `v["id"] == vocabulary_id`.
    - Call `store.save_vocabulary(&full_path, &vocabulary)`.
    - Do NOT call `add_definition_to_boundary` — the vocabulary is already in the package index; this is a pure file mutation with no index change (ADR-007).
  - Return `Ok(PromoteVocabularyResult { vocabulary })`.

- [ ] Add tests for `promote_vocabulary` in `vocabulary_service.rs` tests block:
  - `promote_vocabulary_succeeds_when_all_keys_resolve` — create vocabulary with a term, create a note using that tag key, call `promote_vocabulary`, assert mode is now Closed
  - `promote_vocabulary_fails_when_key_unresolvable` — create vocabulary with no terms, create note with a tag, call `promote_vocabulary`, expect `VocabularyPromotionBlocked`
  - `promote_vocabulary_fails_when_already_closed` — create closed vocabulary, call `promote_vocabulary`, expect `InvalidRepositoryInitialization`
  - `promote_vocabulary_retired_key_blocks_promotion` — create vocabulary with a retired term, create note using that key, call `promote_vocabulary`, expect `VocabularyPromotionBlocked`

**CLI Worker — vocabulary.rs:**

- [ ] Add `Promote { id: String }` variant to `VocabularyCommand` enum in `crates/srs-cli/src/commands/mod.rs`:
  ```rust
  /// Promote a vocabulary from open to closed (V10 pre-flight required)
  Promote {
      /// Vocabulary UUID id
      id: String,
  }
  ```

- [ ] Add `PromoteVocabularyPayload` to `crates/srs-cli/src/payload.rs`:
  ```rust
  /// Payload for `vocabulary promote`.
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct PromoteVocabularyPayload {
      #[schemars(with = "serde_json::Value")]
      pub vocabulary: Vocabulary,
  }
  ```

- [ ] Add handler in `crates/srs-cli/src/commands/vocabulary.rs`:
  ```rust
  fn cmd_vocabulary_promote(ctx: CliContext, id: String) -> Result<String> {
      let result = with_store(&ctx, |store| {
          Ok(vocabulary_service::promote_vocabulary(
              store,
              vocabulary_service::PromoteVocabularyInput { vocabulary_id: id.clone() },
          )?)
      })?;
      output::serialize("vocabulary promote", PromoteVocabularyPayload { vocabulary: result.vocabulary })
  }
  ```
  Wire `VocabularyCommand::Promote { id } => cmd_vocabulary_promote(ctx, id)` in the dispatch match.

- [ ] Run `cargo run --bin generate-schemas` — confirm `schemas/payload/vocabulary-promote.json` is created. Stage and commit it.

- [ ] Add integration tests in `integration_tests.rs`:
  - `vocabulary_promote_succeeds_when_all_in_use_keys_resolve` — create vocabulary with term "alpha", create note tagged "alpha", call `srs vocabulary promote <id>`; assert `ok: true` and `payload["vocabulary"]["mode"] == "closed"`.
  - `vocabulary_promote_fails_when_unresolvable_key_exists` — create vocabulary with no terms, create note tagged "orphan", call `srs vocabulary promote <id>`; assert `ok: false` and diagnostics contain "VocabularyPromotionBlocked" or "unresolvable".

#### Acceptance Criteria

- [ ] `srs vocabulary promote <id>` closes an open vocabulary when all in-use keys resolve
- [ ] `srs vocabulary promote <id>` returns an error envelope when any in-use key is unresolvable
- [ ] `cargo test --test payload_contracts` passes (vocabulary-promote.json committed)
- [ ] All 4 service tests and 2 integration tests pass
- [ ] `cargo test` — no failures
- [ ] `cargo clippy -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository promote_vocabulary
cargo test --test integration_tests vocabulary_promote
cargo test --test payload_contracts
cargo test
cargo clippy -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria.
2. Confirm `schemas/payload/vocabulary-promote.json` is staged.
3. Run `cargo test` and `cargo clippy -- -D warnings`.
4. Update plan checkboxes `[x]`.
5. Commit: `feat(vocabulary): V10 promote pre-flight and vocabulary promote command (#73)`

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schema changes; confirm no regressions)
- [ ] `cargo run --bin srs -- repo validate --repo ../srs/srs --pretty` exits 0 with 0 errors
- [ ] Integration tests exist for: `vocabulary list/get/create/term-create/promote`, `lifecycle list/get`, `term list/get`
- [ ] Validation rules V2, V5, V7, V8, V9 have unit tests that prove the error path
- [ ] V10 `promote_vocabulary` rejects unresolvable in-use keys
- [ ] Golden schema files committed: `term-list.json`, `term-get.json`, `vocabulary-promote.json`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- `pkg.fields` is a `Vec<Field>` on the `Package` struct (confirmed in `package.rs`).
- `pkg.record_types` is a `Vec<RecordType>` on `Package` (confirmed in `package.rs` — NOT `pkg.types`).
- `MemoryStore` is accessible via `use crate::store::MemoryStore;` inside `#[cfg(test)]` blocks in `vocabulary_service.rs` (same crate, `#[cfg(test)]` items are visible across module boundaries within the crate).
- `entry.tags` on manifest index entries carries the raw tag strings (confirmed by `tag_service::find_instances_using_tag`).
- `VocabularyMode::Closed` comparison works since `VocabularyMode` derives `PartialEq` (confirmed in `vocabulary.rs`).
- Issue number `#73` is the correct tracking reference for commit messages.
