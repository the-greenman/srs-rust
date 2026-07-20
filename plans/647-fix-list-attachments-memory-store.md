# Plan: fix list_attachments returning empty entries after add_attachment on MemoryStore and JsonStore (#647)

## Summary

`list_attachments` returns `{ entries: [] }` immediately after `add_attachment` on in-memory-backed repos. Root cause: both `MemoryStore::list_files_recursive` and `JsonStore::list_files_recursive` scan only their text/JSON data maps but not their binary file maps. `add_attachment` writes content bytes via `save_binary_file` into the binary map; the sidecar `.meta.json` lands in the text map and is then excluded as a sidecar — leaving zero entries. `MemoryStore` is the canonical test double; `JsonStore` is used by the WASM bindings (srs-web). Both must be fixed together.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Repository Service Worker |
| Repository Service Worker | Repository Service Worker |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this is a bug in both MemoryStore and JsonStore adapters introduced by the ADR-031 amendment that added `binary_data`/`binary_files` fields without updating `list_files_recursive` in either store.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Business logic lives in `srs-repository`; MemoryStore is the canonical test double | accepted |
| [ADR-031](../docs/adr/031-source-doc-blob-portability.md) | Binary attachment blobs stored separately from text data in both MemoryStore and JsonStore | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No CLI output shapes change — this is a pure store / service bug fix.

### Entity schema sync (check-schema-sync.sh)

No entity schema changes.

---

## Scope

- Fix `MemoryStore::list_files_recursive` in `crates/srs-repository/src/store.rs` (~line 3014) to union `self.data` and `self.binary_data` key sets.
- Fix `JsonStore::list_files_recursive` in `crates/srs-repository/src/json_store.rs` (~line 1565) to union `state.data` and `state.binary_files` key sets (including the `relative_dir.is_empty()` branch).
- Add regression test `add_then_list_attachments_memory_store` in `crates/srs-repository/src/attachment_service.rs`.
- Add regression test `add_then_list_attachments_json_store` in `crates/srs-repository/src/attachment_service.rs`.
- Update stale comments in `crates/srs-repository/src/attachment_service.rs` at lines ~78–79 and ~744–745.

**Out of scope:**
- WASM binding layer changes (the bug is in `JsonStore`; fixing it propagates to WASM automatically)
- FileStore (not affected — it walks the real filesystem)
- CLI / payload / schema changes

---

## Phases

### Phase 1: Fix both stores and add regression tests

**Goal:** `list_files_recursive` on both `MemoryStore` and `JsonStore` unions text and binary paths; `add_attachment` followed by `list_attachments` returns the added entry on both stores.

**Agent:** Repository Service Worker

#### Tasks

- [x] **Fix MemoryStore** — in `crates/srs-repository/src/store.rs`, replace `MemoryStore::list_files_recursive` (~line 3014) with a `chain()` union of `self.data` and `self.binary_data` keys.

- [x] **Fix JsonStore** — in `crates/srs-repository/src/json_store.rs`, replace `JsonStore::list_files_recursive` (~line 1565) with a `chain()` union of `state.data` and `state.binary_files` keys (both branches: empty dir and prefix filter).

- [x] **Update stale comment in `attachment_service.rs`** at lines ~78–79 to reflect that MemoryStore returns `Some(n)` for binary files after the fix.

- [x] **Update stale comment in `attachment_service.rs`** at lines ~744–745 to reflect that `touch` is for the text-data path; `add_attachment` covers the binary-data path.

- [x] **Add MemoryStore regression test** `add_then_list_attachments_memory_store` — calls `add_attachment`, then `list_attachments`, asserts path/document_id/title/content_checksum/size_bytes all present.

- [x] **Add JsonStore regression test** `add_then_list_attachments_json_store` — same flow on a scaffolded JsonStore, proving the WASM path works.

#### Acceptance Criteria

- [ ] `add_then_list_attachments_memory_store` exists and passes
- [ ] `add_then_list_attachments_json_store` exists and passes
- [ ] All existing `list_attachments_*` and `add_attachment_*` tests still pass
- [ ] Updated stale comments at `attachment_service.rs` lines ~78–79 and ~744–745
- [ ] `cargo test -p srs-repository` passes with no failures
- [ ] `cargo clippy -p srs-repository -- -D warnings` is clean

#### Testing

```bash
cargo test -p srs-repository add_then_list_attachments_memory_store
cargo test -p srs-repository add_then_list_attachments_json_store
cargo test -p srs-repository attachment_service
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. Both regression tests pass.
2. All existing attachment service tests pass.
3. Clippy clean.
4. Update plan checkboxes `[x]`.
5. Commit: `fix(store): list_files_recursive unions binary paths on MemoryStore and JsonStore (#647)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (payload structs not changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (entity schemas not changed)
- [ ] `add_then_list_attachments_memory_store` passes
- [ ] `add_then_list_attachments_json_store` passes
- [ ] `add_attachment_filestore_roundtrip` (existing test) continues to pass

## Coordination Rules

- Single-phase fix confined to `crates/srs-repository/src/store.rs`, `crates/srs-repository/src/json_store.rs`, and comments + tests in `crates/srs-repository/src/attachment_service.rs`.

## Assumptions

- `self.data`/`state.data` and `self.binary_data`/`state.binary_files` are disjoint in normal usage. The `chain()` approach makes this assumption explicit (duplicates appear twice, fail assertions) rather than silently hiding them with a `HashSet`.
- FileStore walks the real filesystem and is unaffected.
- WASM bindings call attachment_service functions through `JsonStore`; fixing `JsonStore::list_files_recursive` propagates to WASM without any binding-layer changes.
