# Plan: Lenient parsing mode for source_document_service (#592)

## Summary

`source_document_service::list_source_documents` currently returns `Err` on the first malformed
`.meta.json` sidecar, blocking all attachment listings when a single corrupt file exists. This plan
adds an opt-in lenient mode: when `ListSourceDocumentsFilter.lenient = true` the service skips
malformed sidecars, collects parse errors in a side-channel, and returns the valid entries. The
default (`lenient: false`) preserves existing strict behaviour. This unlocks resilient attachment
listing for consumers that prefer partial results over a hard failure.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude Code (orchestrator) |
| Repository Worker | Claude Code (implementer) |

See [agents.md](agents.md) for role definitions.

_Verification is handled by the Phase 1 milestone gate (`cargo test` + `clippy`) — no separate Verification agent needed for this single-file, single-phase plan._

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Filter struct extended (not a new overloaded function); result struct is the contract for all consumers | accepted |

New return type `ListSourceDocumentsResult` is introduced alongside `ListSourceDocumentsFilter`.
No new ADR required — this directly implements the ADR-010 filter-struct + typed-result pattern.

_Design rationale recorded here (no separate ADR because no new architectural constraint is
established):_

- **`lenient: bool` in filter** — preferred over `ParseErrorPolicy` enum because only two states
  exist; an enum adds ceremony with no practical gain at this scope.
- **New result struct** — `ListSourceDocumentsResult { entries, errors }` returned from both modes.
  Strict mode returns `Err` on first parse failure (unchanged semantics); if it returns `Ok` the
  `errors` vec is always empty. Lenient mode never returns `Err` for parse failures — errors
  accumulate in `errors`.
- **`SourceDocumentParseError { path: PathBuf, message: String }`** — `path` is the repo-relative
  sidecar path as `PathBuf` (matching `RepositoryError::SourceDocumentMetaLoad.path`); `message`
  is the `serde_json::Error` display string. `serde_json::Error` is not `Clone`, so we store the
  formatted string rather than box the error (sufficient for diagnostics).
- **No changes to `attachment_service::list_source_documents`** — that function has a different
  return type (`Vec<SourceDocumentEntry>`) and is a parallel implementation. Applying lenient mode
  there is explicitly deferred (see Out of scope).

No spec change required — this is a purely internal Rust service enhancement with no SRS data
model impact.

---

## Contracts

### CLI output contract (ADR-011)

**No new/changed commands.** `source_document_service::list_source_documents` is not yet exposed
via a CLI command. No payload changes, no schema regeneration needed.
Verification: `cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

**No.** No JSON Schema files under `srs/docs/schema/2.0/` are changed.

---

## Scope

- Add `SourceDocumentParseError { path: PathBuf, message: String }` to `source_document_service` (`PathBuf` matches the convention in `RepositoryError::SourceDocumentMetaLoad`).
- Add `ListSourceDocumentsResult { entries: Vec<SourceDocumentMeta>, errors: Vec<SourceDocumentParseError> }` to `source_document_service`.
- Add `lenient: bool` field to `ListSourceDocumentsFilter` (default `false`).
- Update `list_source_documents` signature: `-> Result<ListSourceDocumentsResult, RepositoryError>`.
- Update existing tests to use `result.entries` / `result.errors` instead of the old `Vec<SourceDocumentMeta>` return.
- Add new tests: lenient mode skips malformed sidecar and returns it in `errors`; strict mode still returns `Err`.

**Out of scope:**

- Applying lenient mode to `attachment_service::list_source_documents` (different function, deferred as a follow-up).
- Exposing lenient mode via a CLI command or WASM binding (no CLI consumer exists yet; binding is tracked by #620).
- Merging the two `list_source_documents` implementations — tracked as #760 (linked under epic #271).

---

## Phases

### Phase 1: Extend service with lenient mode

**Goal:** `source_document_service::list_source_documents` accepts a `lenient` flag, returns
`ListSourceDocumentsResult`, and all existing tests pass with the new types.

**Agent:** Repository Worker

#### Tasks

- [ ] In `crates/srs-repository/src/source_document_service.rs`:
  - [ ] Add `pub struct SourceDocumentParseError { pub path: PathBuf, pub message: String }` (derive `Debug`, `Clone`). Use `PathBuf` to match `RepositoryError::SourceDocumentMetaLoad.path`.
  - [ ] Add `pub struct ListSourceDocumentsResult { pub entries: Vec<SourceDocumentMeta>, pub errors: Vec<SourceDocumentParseError> }` (derive `Debug`).
  - [ ] Add `pub lenient: bool` field to `ListSourceDocumentsFilter`; implement `Default` explicitly with `lenient: false` (or `#[derive(Default)]` if that gives `false`).
  - [ ] Update `list_source_documents` return type to `Result<ListSourceDocumentsResult, RepositoryError>`.
  - [ ] In the function body: when `filter.lenient` is `false`, propagate parse errors as `Err` (existing behaviour, now returns `Ok(ListSourceDocumentsResult { entries, errors: vec![] })` on success). When `filter.lenient` is `true`, catch `RepositoryError::SourceDocumentMetaLoad` per entry, push `SourceDocumentParseError { path, message }` into `errors`, and continue.
- [ ] Update all existing tests in the module to access `.entries` instead of the bare `Vec`, and assert `result.errors.is_empty()` in strict-mode success tests.
- [ ] Create fixture directory `tests/fixtures/malformed-sidecar-repo/` with:
  - `manifest.json` (minimal valid manifest, no `sourceDocumentsPath` override)
  - `source-documents/valid.md.meta.json` (valid sidecar: documentId, contentPath, contentType, createdAt)
  - `source-documents/corrupt.md.meta.json` (content: `not-valid-json`)
- [ ] Add new tests:
  - `lenient_mode_skips_malformed_returns_err_in_side_channel` — MemoryStore with one valid + one malformed sidecar; `lenient: true` returns `Ok` with 1 entry and 1 error; error `.path` is non-empty and `.message` is non-empty.
  - `lenient_mode_all_malformed_returns_empty_entries` — MemoryStore with two malformed sidecars; `lenient: true` returns `Ok` with 0 entries and 2 errors.
  - `strict_mode_still_returns_err_on_malformed` — `lenient: false` (default) returns `Err` when a sidecar is malformed (confirms existing contract).
  - `file_store_lenient_mode_skips_malformed_sidecar` — FileStore against `tests/fixtures/malformed-sidecar-repo/`; `lenient: true` returns `Ok` with 1 valid entry and 1 error. (Cross-store roundtrip test per CLAUDE.md storage boundary rules.)

#### Acceptance Criteria

- [ ] `list_source_documents` returns `Result<ListSourceDocumentsResult, RepositoryError>`.
- [ ] `ListSourceDocumentsFilter::default()` has `lenient: false`.
- [ ] Strict mode (lenient: false): first parse error → `Err`; on success `errors` is empty.
- [ ] Lenient mode (lenient: true): parse errors go into `errors`, never `Err`; valid entries returned.
- [ ] All pre-existing tests pass (updated for new return type).
- [ ] Four new tests pass (see Tasks above), including at least one FileStore cross-store test.
- [ ] `cargo clippy -p srs-repository -- -D warnings` clean.

#### Testing

```bash
cargo test -p srs-repository source_document_service
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `memory_store_list_source_documents_empty` — updated to use `result.entries`
- `memory_store_list_source_documents_single` — updated to use `result.entries`
- `memory_store_list_source_documents_subdirectory` — updated to use `result.entries`
- `memory_store_malformed_sidecar_returns_err` — verifies strict-mode `Err` still holds
- `memory_store_valid_json_missing_required_field_returns_err` — verifies strict-mode `Err`
- `file_store_list_source_documents_spec_repo` — updated to use `result.entries`
- `lenient_mode_skips_malformed_returns_err_in_side_channel` — new (MemoryStore)
- `lenient_mode_all_malformed_returns_empty_entries` — new (MemoryStore)
- `strict_mode_still_returns_err_on_malformed` — new (confirms strict contract; MemoryStore)
- `file_store_lenient_mode_skips_malformed_sidecar` — new (FileStore cross-store test; CLAUDE.md requirement)

#### Milestone gate

1. All acceptance criteria above checked.
2. All named tests exist and pass.
3. Run lint and tests:

```bash
cargo test -p srs-repository source_document_service
cargo clippy -p srs-repository -- -D warnings
```

4. Update plan checkboxes.
5. Commit:

```bash
git commit -m "feat(source-document-service): add lenient parse mode (#592)"
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `list_source_documents` lenient mode skips malformed sidecars and returns errors in side-channel
- [ ] Strict mode (default) returns `Err` on first malformed sidecar (unchanged behaviour)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Repository Worker write scope: `crates/srs-repository/src/source_document_service.rs` only.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **Verification (milestone gate):** run `cargo test -p srs-repository source_document_service` and `cargo clippy -p srs-repository -- -D warnings`. All named tests must exist and pass; clippy must be clean. Lead Integrator performs this check before committing.
- **At the end of Phase 1:** verify all acceptance criteria, confirm planned tests exist and pass, update plan checkboxes, then commit. Do not proceed to Final Acceptance without completing the milestone gate.

## Assumptions

- `serde_json::Error` display string is sufficient for `SourceDocumentParseError.message`; callers wanting structured error data should use the strict mode instead.
- No CLI or WASM consumer needs to be updated — `source_document_service::list_source_documents` has no call sites outside its own test module.
- The `attachment_service::list_source_documents` (separate function, same name) is out of scope.
