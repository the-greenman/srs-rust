# Plan: RepositoryStore conformance/parity harness (ADR-041 G9)

## Summary

ADR-041 G9 directs that a shared `RepositoryStore` conformance/parity suite must be the
admission gate for any new backend. Currently, the store-matrix behavioral tests are scattered
inside `src/store.rs` (inline unit tests targeting MemoryStore only) — they prove individual
MemoryStore methods but do not exercise FileStore or JsonStore through the same assertions. This
plan creates a dedicated, parameterised integration test file (`tests/store_conformance.rs`) that
runs a shared suite of store-method behavioral assertions against all three current backends
(FileStore, JsonStore, MemoryStore) and establishes the file as the admission gate for any future
backend. Existing inline tests in `store.rs`, `field_json_parity_tests.rs`, and
`selector_parity_tests.rs` are not consolidated — they cover orthogonal concerns and remain in
place.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Worker | — |
| Verification Agent | — |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-041](../docs/adr/041-storage-backend-guardrails.md) | G9 — conformance/parity suite is the backend-admission gate; G6 — multi-write ops must use the batch seam | accepted |
| [ADR-042](../docs/adr/042-logical-id-instance-persistence.md) | Typed logical-id instance methods are the primary persistence surface | accepted |
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | Backend-neutral `RepositorySnapshot` portability engine; `copy_repository` is the cross-store round-trip vehicle | accepted |
| [ADR-007](../docs/adr/007-file-index-io-ordering.md) | Entity-before-index on create, index-before-entity on delete | accepted |
| [ADR-021](../docs/adr/021-jsonstore-batch-write-mode.md) | begin/commit/abort_batch batch write seam (ADR-041 G6) | accepted |

_No new ADRs required — this plan implements ADR-041 G9 without establishing new architectural constraints._

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. No payload structs touched. No schema regeneration required.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/`. No schema sync required.

---

## Scope

**In scope:**
- New integration test file `crates/srs-repository/tests/store_conformance.rs`
- `run_conformance_suite(store: &dyn RepositoryStore)` helper covering 4 behavioral areas
- Three admission-gate tests: one each for FileStore, JsonStore, MemoryStore
- Cross-store portability tests (memory→file, memory→json, full chain memory→json→file→memory)
- FailPoint tests (MemoryStore-only, separate functions — not inside `run_conformance_suite`)
- `ARCHITECTURE.md` (`/home/user/srs-rust/ARCHITECTURE.md`) update: reference `tests/store_conformance.rs`

**Out of scope:**
- Migrating existing inline `store.rs` tests to call the harness (they remain)
- Adding a FailPoint mechanism to FileStore or JsonStore
- Any new RepositoryStore implementations (SQLite spike is a separate filed follow-up)

---

## Phases

### Phase 1: Create the conformance harness

**Goal:** `crates/srs-repository/tests/store_conformance.rs` exists with a `run_conformance_suite` helper and all three store admission-gate tests pass.

**Agent:** Lead Integrator / Repository Worker

**Write scope:** `crates/srs-repository/tests/store_conformance.rs`, `ARCHITECTURE.md`

#### Tasks

- [x] Create `crates/srs-repository/tests/store_conformance.rs`
- [x] Implement helper constructors:
  - `fn init_file_store() -> (FileStore, TempDir)` — create `TempDir`, call `FileStore::new(tmp.path())`, then `srs_repository::repository_lifecycle::create_repository(&store, &InitializeRepositoryInput { repository: RepositoryMetadata { repository_id: "conf-repo".to_string(), namespace: "com.test.conf".to_string(), srs_version: "2.0-draft".to_string(), title: None, description: None }, primary_package: PrimaryPackageMetadata { id: "conf-pkg".to_string(), namespace: "com.test.conf".to_string(), name: "primary".to_string(), version: "1.0.0".to_string() }, package_refs: vec![], intent: None })`, return `(store, tmp)`
  - `fn init_json_store() -> (JsonStore, TempDir)` — create `TempDir`, call `JsonStore::create(tmp.path().join("repo.srsj"))`, then `create_repository` with same input as above, return `(store, tmp)`
  - `fn init_memory_store() -> MemoryStore` — call `MemoryStore::empty()` (does not require `create_repository` as it constructs a valid manifest/package in memory)
- [x] Implement `run_conformance_suite(store: &dyn RepositoryStore)` covering the following via raw `RepositoryStore` trait methods — NOT through service-layer functions (the suite proves backend behavioral contracts, not service logic):
  - **Manifest round-trip:** `save_manifest(manifest)` → `load_manifest()` returns a manifest with identical `instance_index`, `namespace` fields
  - **Container CRUD (typed logical-id — the gold standard per ADR-041):** using `store.save_container(&container)` / `store.load_container(id)` / `store.list_container_summaries()` / `store.delete_container(id)`:
    - Save a container with a known id → `load_container(id)` returns it with matching `title`
    - `list_container_summaries()` returns the saved container's id and title
    - `delete_container(id)` → `load_container(id)` returns `ContainerNotFound`-variant error
    - Two containers with different ids coexist (list returns both)
  - **Instance persistence (ADR-042 typed methods):** using `store.save_record` / `store.load_record_by_id` / `store.save_note` / `store.load_note_by_id` / `store.delete_instance` / `store.find_instance` / `store.list_instances`:
    - `save_record(&rec)` → `load_record_by_id(id)` round-trip preserves `instance_id`, `type_name`, `tags`
    - `save_note(&note)` → `load_note_by_id(id)` round-trip preserves `instance_id`, `title`
    - `find_instance(note_id)` returns `Some(InstanceRef)` with `tier == 0` for a saved note
    - `delete_instance(id)` → `find_instance(id)` returns `None`, `load_record_by_id(id)` returns an `InstanceNotFound`-variant error
    - `find_instance("nonexistent")` returns `None` (not an error)
    - `list_instances(&InstanceQuery { tier: Some(2), tag: None })` returns refs for saved records (tier 2), not notes (tier 0)
    - Existing-id update: second `save_record` on same id updates the content (`load_record_by_id` returns the new `type_name`) and the index entry (`find_instance` returns updated tags), path in index unchanged
  - **Batch write mode (ADR-021):** using `store.begin_batch()` / `store.commit_batch()` / `store.abort_batch()`:
    - `begin_batch()` + `save_note(&note)` + `commit_batch()` → `load_note_by_id(id)` succeeds (data persisted)
    - `begin_batch()` + `save_note(&note_b)` + `abort_batch()` → `find_instance(note_b_id)` returns `None` (aborted writes must not be visible)
- [x] Implement three admission-gate tests:
  - `fn file_store_passes_conformance()` — calls `init_file_store()`, passes store ref to `run_conformance_suite`
  - `fn json_store_passes_conformance()` — calls `init_json_store()`, passes store ref to `run_conformance_suite`
  - `fn memory_store_passes_conformance()` — calls `init_memory_store()`, passes store ref to `run_conformance_suite`
- [x] Implement cross-store portability tests (each using `srs_repository::repository_portability::copy_repository`):
  - `fn copy_repository_memory_to_file_preserves_instances()` — populate MemoryStore with record + note via `save_record`/`save_note`, `copy_repository(&mem, &file)`, verify `file.load_record_by_id(id)` and `file.load_note_by_id(id)` succeed with matching ids and titles
  - `fn copy_repository_memory_to_json_preserves_instances()` — same but target is JsonStore
  - `fn copy_repository_full_chain_memory_json_file_memory()` — memory → json (`copy_repository`) → file (`copy_repository`) → fresh memory (`copy_repository`); verify final memory store returns correct instance count via `list_instances` and matching content via `load_record_by_id`
- [x] Implement FailPoint tests (MemoryStore-only; use `srs_repository::store::memory::FailPoint`; these are separate top-level test functions, NOT inside `run_conformance_suite`):
  - `fn adr007_index_ordering_instance_save()` — arm `FailPoint::SaveInstanceIndex` before `save_record` → verify `save_record` returns `Io` error; `find_instance` returns `None` (no dangling index entry)
  - `fn adr007_index_ordering_container_save()` — arm `FailPoint::SaveContainerIndex` before `save_container` → verify `save_container` returns `Io` error; `list_container_summaries` returns empty (no dangling index entry)

#### Acceptance Criteria

- [x] All three admission-gate tests pass: `file_store_passes_conformance`, `json_store_passes_conformance`, `memory_store_passes_conformance`
- [x] All cross-store portability tests pass
- [x] Both FailPoint tests pass
- [x] `cargo clippy -p srs-repository -- -D warnings` clean

#### Testing

```bash
cargo test -p srs-repository --test store_conformance
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write and verify:
- `file_store_passes_conformance` — FileStore satisfies the full suite
- `json_store_passes_conformance` — JsonStore satisfies the full suite
- `memory_store_passes_conformance` — MemoryStore satisfies the full suite
- `copy_repository_memory_to_file_preserves_instances`
- `copy_repository_memory_to_json_preserves_instances`
- `copy_repository_full_chain_memory_json_file_memory`
- `adr007_index_ordering_instance_save`
- `adr007_index_ordering_container_save`

#### Milestone gate

1. All acceptance criteria checked.
2. All 8 listed tests exist and pass.
3. Run:
```bash
cargo test -p srs-repository --test store_conformance
cargo clippy -p srs-repository -- -D warnings
```
4. Update plan checkboxes `[x]`.
5. Commit.

---

### Phase 2: Documentation

**Goal:** `ARCHITECTURE.md` names `tests/store_conformance.rs` as the admission gate, matching ADR-041 G9.

**Agent:** Lead Integrator

**Write scope:** `/home/user/srs-rust/ARCHITECTURE.md`

#### Tasks

- [x] Update `Store Matrix Testing` section of `/home/user/srs-rust/ARCHITECTURE.md` to add: "The admission gate for any new backend is `crates/srs-repository/tests/store_conformance.rs` — every new `impl RepositoryStore` must pass all tests in that file before its PR is mergeable."

#### Acceptance Criteria

- [x] `ARCHITECTURE.md` `Store Matrix Testing` paragraph names `crates/srs-repository/tests/store_conformance.rs` as the admission gate

#### Milestone gate

1. Check acceptance criterion.
2. Commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (no payload structs changed)
- [ ] `cargo test -p srs-repository --test payload_contracts` passes (no payload change)
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] `cargo test -p srs-repository --test store_conformance` passes with ≥ 8 tests

## Coordination Rules

- Lead Integrator owns final naming and test structure.
- Repository Worker write scope: `crates/srs-repository/tests/store_conformance.rs`, `/home/user/srs-rust/ARCHITECTURE.md`.
- Do NOT touch `src/store.rs` inline tests — they remain as-is.
- **At the end of each phase:** verify acceptance criteria, confirm tests pass, update checkboxes, commit.

## Assumptions

- `srs_repository::store::memory::MemoryStore` and `srs_repository::store::memory::FailPoint` are accessible from integration tests via `pub mod memory` in `store.rs`.
- `srs_repository::FileStore`, `srs_repository::JsonStore`, `srs_repository::RepositoryStore` are accessible at the crate root.
- `srs_repository::repository_portability::copy_repository` is accessible via the `pub mod repository_portability` in `lib.rs` (not a crate-root re-export).
- `srs_repository::repository_lifecycle::create_repository`, `InitializeRepositoryInput`, `RepositoryMetadata`, `PrimaryPackageMetadata` are accessible via `pub mod repository_lifecycle` in `lib.rs`.
- `srs_repository::container_service::{get_container, create_container, delete_container, list_containers}` are accessible via `pub mod container_service` in `lib.rs` (but the harness uses raw store trait methods, not these service functions).
- `srs_repository::index::{InstanceQuery, InstanceRef}` are accessible via `pub mod index` in `lib.rs`.
- `MemoryStore::empty()` produces a store with a valid default manifest and empty package, suitable for conformance testing without calling `create_repository`.
- `abort_batch()` on `FileStore` and `MemoryStore` is a no-op (default trait impl); only `JsonStore` has a meaningful rollback. The abort test asserts `find_instance` returns `None` for all three stores, which holds because: for JsonStore the write is rolled back; for FileStore and MemoryStore the batch is a no-op so the write was committed on `save_note`, but the test correctly only passes `abort_batch` — need to verify per-store. CORRECTION: For stores where abort is a no-op, the test MUST use JsonStore only. The abort assertion is therefore only included in `json_store_passes_conformance` (not `run_conformance_suite`), or the batch tests are split: a commit sub-test in `run_conformance_suite` (runs all three) and an abort sub-test only in `json_store_passes_conformance` or a standalone `json_store_batch_abort_rolls_back` test. Plan decision: add `fn json_store_batch_abort_rolls_back()` as a 9th standalone test; `run_conformance_suite` only asserts `commit_batch` persists.
