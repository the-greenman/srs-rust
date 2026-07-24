# Plan: srs-bindings list_attachments integration test

## Summary

`attachment_service::list_attachments` has a WASM binding at `SrsRepository::list_attachments`
(lib.rs:1112) and a unit-level serialization test at `list_attachments_result_serialises`
(lib.rs:2185). The only remaining gap from issue #611 is that `crates/srs-bindings/tests/`
lacks a dedicated integration test file, while every other attachment-adjacent surface has one
(`get_record_attachments.rs`, `resolve_view_attachments.rs`, `attachment_bytes.rs`). This plan
adds `tests/list_attachments.rs`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | claude |
| Bindings Worker | claude |
| Verification | claude |

## Architecture Decisions

No new architectural decisions — this plan implements ADR-013 (WASM binding strategy). The
binding itself already exists; this plan adds the missing integration-test coverage.

| ADR | Decision | Status |
|---|---|---|
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM bindings are thin service wrappers; bindings crate owns coverage of the binding surface | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new/changed commands — no action required.

### Entity schema sync (check-schema-sync.sh)

No schema changes — no action required.

---

## Scope

- Add `crates/srs-bindings/tests/list_attachments.rs` with integration tests covering:
  - Empty repo → returns empty `entries`, default `source_documents_path`
  - Repo with an index entry (srsj fixture) → entry appears with `documentId`, `title`, etc.
  - File listed but not in index → entry appears with only `path` populated

**Out of scope:**

- Any changes to the binding implementation (already exists and matches ADR-013 pattern)
- `ListAttachmentsFilter` extension (empty struct; deferred by design)
- FileStore-based test for on-disk attachment listing (already covered extensively in
  `srs-repository/src/attachment_service.rs` tests)

---

## Phases

### Phase 1: Integration tests

**Goal:** `crates/srs-bindings/tests/list_attachments.rs` exists, all tests pass.

**Agent:** Bindings Worker

**Note on test scope:** `SrsRepository::list_attachments` returns `Result<JsValue, JsValue>` which
is not meaningful on a native `cargo test` target. Following the pattern established by
`tests/attachment_bytes.rs` (which documents this caveat explicitly), the integration tests
import and call `attachment_service::list_attachments` directly. The wasm32 build gate confirms
the binding wrapper compiles and links correctly. This approach is DRY with the existing
`add_then_list_attachments_json_store` test in `srs-repository/src/attachment_service.rs`
(which exercises the same service via `add_attachment`); the difference is that these tests
exercise the static srsj-deserialization path through `manifest.sourceDocumentIndex`, which
the srs-repository tests don't cover.

#### Tasks

- [x] Write `crates/srs-bindings/tests/list_attachments.rs` with three tests:
  1. `binding_list_attachments_empty_repo` — srsj with no `source-documents/` keys in the
     `data` map → `entries` is empty, `source_documents_path` is `"source-documents"`
  2. `binding_list_attachments_with_indexed_entry` — srsj fixture with a
     `"source-documents/brief.pdf"` key in `data` AND a matching `sourceDocumentIndex`
     entry (with `documentId`, `title`, checksums) → entry appears with all metadata fields
     populated; `.meta.json` sidecar is filtered out
  3. `binding_list_attachments_unindexed_file_appears_path_only` — srsj with a
     `"source-documents/orphan.pdf"` key in `data` but NO matching `sourceDocumentIndex`
     entry → entry appears with only `path` populated and no `documentId`, `title`, or checksums

#### Acceptance Criteria

- [x] All three tests pass under `cargo test -p srs-bindings`
- [x] No changes to `src/lib.rs` or any other file
- [x] `cargo clippy -p srs-bindings -- -D warnings` passes

#### Milestone gate result

All three acceptance criteria confirmed. `cargo test -p srs-bindings` passes; `cargo clippy -p srs-bindings -- -D warnings` passes.

#### Testing

```bash
cargo test -p srs-bindings list_attachments
cargo clippy -p srs-bindings -- -D warnings
```

Specific tests:
- `binding_list_attachments_empty_repo` — proves the service returns empty results for a fresh repo
- `binding_list_attachments_with_index_entry` — proves indexed metadata is surfaced correctly
- `binding_list_attachments_unindexed_file` — proves unindexed files appear path-only

#### Milestone gate

1. All three acceptance criteria checked above.
2. `cargo test -p srs-bindings` passes with no failures.
3. `cargo clippy -p srs-bindings -- -D warnings` passes.
4. Commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [x] `cargo test --test payload_contracts` passes (no payload changes)
- [x] `bash scripts/check-schema-sync.sh` exits 0 (no schema changes)
- [x] All three new integration tests exist and pass

## Coordination Rules

Single agent; no concurrency needed.

## Assumptions

- `list_attachments` service scans `store.list_files_recursive(src_docs_base)` — for JsonStore
  this returns no files (no virtual FS), so the "empty repo" and "with index entry" tests both
  use JsonStore and the srsj fixture approach (matching the pattern in `resolve_view_attachments.rs`
  and `get_record_attachments.rs`). The `unindexed_file` test requires `add_attachment` via
  FileStore to actually write a file; alternatively it can be tested via a MemoryStore path
  registration — choose whichever is simpler given the existing fixture patterns.
