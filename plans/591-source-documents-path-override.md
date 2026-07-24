# Plan: Support sourceDocumentsPath manifest override for non-standard layouts

## Summary

SRS repositories with non-standard source-document directory layouts currently cannot be served by `list_source_documents` because the store's `list_source_document_sidecar_paths()` default hardcodes `"source-documents"`. The `manifest.json` already parses a `sourceDocumentsPath` field (`Manifest.source_documents_path: Option<String>`) but the store default ignores it. This plan makes the store default honour that field, falling back to `"source-documents"` when absent — matching the pattern already used in `archive.rs`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | Path resolution stays in the store layer; service/CLI code must not construct or interpret filesystem paths | accepted |
| [ADR-031](../docs/adr/031-source-document-blob-portability.md) | Amendment §2026-07-17 promoted `source_documents_path` to a typed `Manifest` field; `archive.rs` pattern `manifest.source_documents_path.as_deref().unwrap_or("source-documents")` is the established precedent this plan follows | accepted |

No new ADRs needed — this plan implements ADR-008 and the typed-field read from ADR-031.

**Known limitation (ADR-031 §Consequences, not fixed here):** When a repo with `"sourceDocumentsPath": "attachments"` is copied via `copy_repository` and all source-document sidecars are tombstones, `source_documents_path` is not written to the target manifest. After this fix, `list_source_document_sidecar_paths()` would fall back to `"source-documents"` — wrong for that repo. This edge case is tracked under srs-rust#604 and is out of scope for this plan.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. `list_source_documents` is a service function exposed via `srs source-document list`. The service return type (`Vec<SourceDocumentMeta>`) is unchanged. No payload struct changes. No schema regeneration needed.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are added or modified. No action required.

---

## Scope

- Change the default `list_source_document_sidecar_paths()` method in `RepositoryStore` (`crates/srs-repository/src/store.rs`) to read `source_documents_path` from the manifest, falling back to `"source-documents"`.
- Add a test fixture repo (or in-memory setup) that sets a non-default path and verifies that `list_source_documents` finds sidecars there.
- Add a fixture repo for FileStore testing with a non-default path in `manifest.json`.

**Out of scope:**

- Changes to `ListSourceDocumentsFilter` (no new filter fields)
- Changes to `srs-cli`, `srs-bindings`, or `srs-mcp`
- Changes to `SourceDocumentMeta` type or serde shape
- Any new public service function signatures

---

## Phases

### Phase 1: Store default reads manifest path

**Goal:** `list_source_document_sidecar_paths()` honours `manifest.json`'s `sourceDocumentsPath` field, falling back to `"source-documents"` if absent, and all tests pass.

**Agent:** Repository Worker

#### Tasks

- [x] In `crates/srs-repository/src/store.rs`, update the default `list_source_document_sidecar_paths()` method to:
  ```rust
  fn list_source_document_sidecar_paths(&self) -> Vec<String> {
      let base = self
          .load_manifest()
          .ok()
          .and_then(|m| m.source_documents_path)
          .unwrap_or_else(|| "source-documents".to_string());
      self.list_files_recursive(&base)
          .into_iter()
          .filter(|p| p.ends_with(".meta.json"))
          .collect()
  }
  ```
- [x] In `crates/srs-repository/src/source_document_service.rs`, add a test `memory_store_custom_source_documents_path`:
  - Create `MemoryStore::empty()`
  - Save manifest with `source_documents_path: Some("attachments".to_string())`
  - Save a valid sidecar at `"attachments/doc.md.meta.json"`
  - Assert `list_source_documents()` returns 1 entry with the correct `document_id`
  - Also assert nothing is returned from `"source-documents/"` (the default path) when none exist there
- [x] In `crates/srs-repository/src/source_document_service.rs`, add a test `memory_store_missing_manifest_falls_back_to_default`:
  - Create `MemoryStore::empty()` with no manifest (or empty manifest)
  - Save a sidecar at `"source-documents/doc.md.meta.json"`
  - Assert `list_source_documents()` returns 1 entry (fallback works)
- [x] Create `tests/fixtures/custom-source-path-repo/` fixture with:
  - `manifest.json` containing `"sourceDocumentsPath": "attachments"`
  - `attachments/doc.md.meta.json` with a valid `SourceDocumentMeta` sidecar
- [x] In `crates/srs-repository/src/source_document_service.rs`, add a FileStore test `file_store_custom_source_documents_path` that loads the fixture repo and asserts `list_source_documents()` returns the sidecar from `attachments/`.

#### Acceptance Criteria

- [x] `list_source_documents` on a repo with `"sourceDocumentsPath": "attachments"` in `manifest.json` returns sidecars from `attachments/`, not `source-documents/`
- [x] `list_source_documents` on a repo with no `sourceDocumentsPath` in `manifest.json` still returns sidecars from `source-documents/` (fallback)
- [x] Existing spec-repo FileStore test still passes with 4 sidecars
- [x] All existing MemoryStore tests still pass
- [x] `cargo test -p srs-repository` passes
- [x] `cargo clippy -p srs-repository -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `memory_store_custom_source_documents_path` — proves the manifest override is honoured in-memory
- `memory_store_missing_manifest_falls_back_to_default` — proves the fallback still works
- `file_store_custom_source_documents_path` — proves FileStore reads the override from a real fixture repo
- `file_store_list_source_documents_spec_repo` (existing) — proves no regression on the standard path

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit:

```bash
git commit
```

Do not start the next phase until the milestone gate passes and the plan is updated.

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed — this should be a no-op)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed — this should be a no-op)
- [ ] `list_source_documents` honours `sourceDocumentsPath` from `manifest.json`
- [ ] Fallback to `"source-documents"` when field is absent

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `MemoryStore::load_manifest()` returns `Ok(self.manifest.borrow().clone())` — the in-memory manifest is mutable and can be set in tests via `save_manifest()` or direct construction.
- `MemoryStore::empty()` creates an empty manifest where `source_documents_path` is `None`.
- The `Manifest` struct's `source_documents_path: Option<String>` field is already correctly parsed from `"sourceDocumentsPath"` in JSON (confirmed by existing tests in `manifest.rs`).
- The `archive.rs` pattern `manifest.source_documents_path.as_deref().unwrap_or("source-documents")` is the established precedent for this fallback.
