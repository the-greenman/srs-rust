# Plan: 1.11 Tombstone — attaches resolves against the source-doc index; content-absent entries are valid (R2/R12)

## Summary

RFC-017 (srs#101, integrated 2026-07-17) introduced the `attaches` SourceRole for linking records to source documents. Two invariants were specified but not yet enforced in `validate_repository`: **R2** — an `attaches` sourceRef's `sourceId` MUST resolve to a `documentId` present in `sourceDocumentIndex`; **R12** — an index entry whose content file is absent (tombstone state) is valid and must not raise an error. This plan adds those two checks to `crates/srs-repository/src/validation.rs` and adds an archive tombstone pack→unpack roundtrip test that was missing from the existing tombstone coverage.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions. This plan implements existing invariants specified by RFC-017 under the following governing ADRs:

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Validation business logic lives in `srs-repository`, not `srs-cli` | accepted |
| [ADR-031](../docs/adr/031-source-doc-blob-portability.md) | Tombstone state (sidecar present, content file absent) is valid; `content_base64: None` in snapshots | accepted |
| [ADR-034](../docs/adr/034-source-refs-in-record-extra.md) | `sourceRefs` on `Record` is stored via `record.extra["sourceRefs"]`; accessed via raw JSON in validation | accepted |
| [ADR-039](../docs/adr/039-srs-archive-pure-tree-zip.md) | `.srs` archive is a deterministic ZIP of the exploded tree; tombstone pack→unpack test validates that absent content files are not included but index entries survive | accepted |

_No new ADRs are needed — this plan implements existing RFC-017 invariants (R2, R12) under the above accepted decisions._

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. The `validate` command's payload struct is unchanged — diagnostics are surfaced through the existing `RepositoryValidationReport` type, which already carries a `diagnostics: Vec<ValidationDiagnostic>` field. No payload struct changes; no schema regeneration needed.

Verification: `cargo test --test payload_contracts` must continue to pass.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are added or modified. No action required.

---

## Scope

- Add a pre-loop `attaches_doc_id_map: HashMap<String, bool>` (documentId → content_present) built from `manifest.source_document_index` and `manifest.source_documents_path` in `crates/srs-repository/src/validation.rs`.
- Inside the existing instance loop, after loading each instance JSON value, inspect `value["sourceRefs"]` for entries where `sourceType == "repository-document"` and `sourceRole == "attaches"`, and emit Error (R2) or Warning (R12) diagnostics.
- Add 7 named tests covering R2, R12, happy path, non-attaches roles, no-index, and the Record.extra path.
- Add one archive tombstone pack→unpack roundtrip test in `archive.rs`.

**Out of scope:**

- Validating `attaches` refs on `Relation` or `Revision` instances.
- Verifying stored checksums (`sidecarChecksum`, `contentChecksum`) against actual file bytes.
- MCP/bindings surface changes.

---

## Phases

### Phase 1: Add R2/R12 validation and tests

**Goal:** `validate_repository` enforces RFC-017 R2 and R12, backed by passing tests.

**Agent:** Repository Worker

#### Tasks

- [x] Build `attaches_doc_id_map` before the instance loop in `validation.rs`, using `store.file_byte_len()` for content presence.
- [x] Add R2/R12 sourceRef check inside the instance loop (after existing checks, before end of loop).
- [x] Write 7 named `test_attaches_*` tests using `MemoryStore`.
- [x] Add `test_archive_tombstone_roundtrip` in `archive.rs`.

#### Acceptance Criteria

- [x] `validate_repository` returns an `Error` diagnostic for any instance with an `attaches` sourceRef whose `sourceId` is not in `sourceDocumentIndex` (R2).
- [x] `validate_repository` returns a `Warning` diagnostic (not Error) for any instance with an `attaches` sourceRef whose `sourceId` is in the index but content file is absent (R12 tombstone).
- [x] `validate_repository` returns no diagnostic for `attaches` refs with a present, indexed document.
- [x] Non-`attaches` sourceRoles (e.g. `cites`, `quotes`) with unresolved IDs produce no diagnostic.
- [x] The check works for both Notes (typed `sourceRefs` field) and tier-2 Records (raw JSON `value["sourceRefs"]`).
- [x] All 7 named `test_attaches_*` tests exist and pass.
- [x] `test_archive_tombstone_roundtrip` exists and passes.
- [x] `cargo test -p srs-repository` passes with no failures.
- [x] `cargo clippy -p srs-repository -- -D warnings` passes.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:

- `test_attaches_r2_unresolved_source_id` — R2 Error for unresolved sourceId
- `test_attaches_r2_empty_index` — R2 Error when index is empty
- `test_attaches_r12_tombstone_warning` — R12 Warning for tombstone (index entry present, content file absent)
- `test_attaches_happy_path_no_diagnostic` — no diagnostic when document is indexed and content present
- `test_attaches_non_attaches_role_skipped` — non-attaches roles not checked
- `test_attaches_no_source_document_index` — no index field → R2 Error
- `test_record_attaches_r2_via_extra` — Record.extra["sourceRefs"] path produces R2 Error
- `test_archive_tombstone_roundtrip` — tombstone entry survives archive pack→unpack

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm every test listed passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update plan checkboxes `[x]`.
5. Commit: `feat(validation): enforce RFC-017 R2/R12 attaches source-ref checks (#385)`

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs were changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas were changed)
- [ ] All 7 `test_attaches_*` tests exist and pass
- [ ] `test_archive_tombstone_roundtrip` exists and passes
- [ ] `srs repo validate` on a repo with an unresolved `attaches` ref reports an Error diagnostic
- [ ] `srs repo validate` on a repo with a tombstone `attaches` ref reports a Warning (not Error)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs before final sign-off.

## Assumptions

- RFC-017 (srs#101) is fully integrated; no spec change is needed for this plan.
- `RepositoryStore` exposes `file_byte_len` (already present in store.rs L419) and `save_binary_file` for tests.
- `DiagnosticSeverity` has exactly two variants: `Error` and `Warning`.
- `MemoryStore` tracks binary files via `binary_data: RefCell<HashMap<String, Vec<u8>>>` (store.rs L2138), so `save_binary_file` / `load_binary_file` work in tests without disk I/O.
- The archive tombstone path already handles tombstone correctly per `archive.rs` — the test proves it rather than fixing it.
