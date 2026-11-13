# Plan: MemoryStore cross-store coverage for package_move_service::move_definition

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

`package_move_service::move_definition` (srs-rust#1639, merged via PR #1641) and its test file
`crates/srs-repository/tests/package_move.rs` only construct a `FileStore`. `CLAUDE.md`'s
Storage Boundary Rules require `MemoryStore` coverage for every service feature, since
`MemoryStore` is the canonical test double and `FileStore`-only tests exercise the adapter, not
the service. `MemoryStore` already implements every `RepositoryStore` method `move_definition`
calls, so this closes a coverage gap with a test-only change — no production code in
`package_move_service.rs` changes. Found during the 2026-11-13 scheduled architecture-review
routine. Closes #1654.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | (this session) |
| Repository Worker | (this session) |
| Verification | (this session) |

See [agents.md](agents.md) for role definitions. This is a single-session, single-file,
test-only change; one session plays all roles directly rather than spawning separate workers.

## Architecture Decisions

No new architectural decisions — this plan implements the existing Storage Boundary Rules in
`srs-rust/CLAUDE.md` ("MemoryStore is the canonical test double...", "New service features need
at least one cross-store roundtrip test") and follows the established cross-store test idiom
already present in this crate (`crates/srs-repository/tests/delete_membership_cascade.rs`,
`crates/srs-repository/tests/store_conformance.rs`): a generic `fn run_..._suite(store: &dyn
RepositoryStore)` helper exercised once per store via one `#[test]` per store. No new ADR is
needed: this is Door 1 (executes the standing storage-boundary rule), test-only, no public API
or payload-contract change.

| ADR | Decision | Status |
|---|---|---|
| — | No new ADR — implements existing CLAUDE.md Storage Boundary Rules + established test idiom | n/a |

---

## Contracts

### CLI output contract (ADR-011)

No new/changed commands, no payload struct changes. No action required.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` touched. No action required.

---

## Scope

- Refactor `crates/srs-repository/tests/package_move.rs` to extract the test bodies that will
  gain cross-store coverage into generic `fn run_..._suite(store: &dyn RepositoryStore)` helpers,
  following the idiom in `delete_membership_cascade.rs`.
- Add a `MemoryStore` factory analogous to the existing `fn repo() -> (TempDir, FileStore)`, built
  the same way `store_conformance.rs`/`delete_membership_cascade.rs` build `MemoryStore::empty()`
  and then populate it via the same `create_package`/`create_field_normalized`/
  `create_type_normalized`/`create_relation_type_normalized` calls already used for `FileStore`
  (all of those take `&dyn RepositoryStore`, so the same populate logic works unchanged).
- Add `#[test]` functions so the following cases run against **both** `FileStore` and
  `MemoryStore`:
  1. the core lineage-move-together case (moving every version of a definition together) —
     mirrors `moves_every_version_of_a_type_together`.
  2. the dry-run case (writes nothing) — mirrors `a_dry_run_writes_nothing`. The existing
     `FileStore` version uses a filesystem `snapshot()` helper that is disk-specific; the
     `MemoryStore` arm instead asserts no observable change via the store's own read surface
     (e.g. the boundary lists before/after are equal, and `moved_definition.len()` still reports
     what *would* move) rather than trying to snapshot memory byte-for-byte.
  3. at least two refusal-code cases: `definition-not-found` and `move-same-boundary` — mirrors
     `refuses_an_unknown_id_an_unknown_target_and_the_same_boundary` (the `package-not-found` arm
     in that same test can be included too since it comes for free in the same suite fn).
- Keep the existing `FileStore`-only tests that only make sense on disk (file-rename collision,
  partial-move resume via direct `store.save_instance_json`/`add_definition_to_boundary` pokes,
  installed-package refusals using `install_package_bundle_bytes`) exactly as they are — the plan
  does not require duplicating every existing test, only the three categories named above (the
  issue's "at minimum").

**Out of scope:**
- No changes to `package_move_service.rs` production code. If implementation is found during
  Phase 1 to need non-trivial rework to support `MemoryStore`, stop and report it as a blocker
  rather than making the change.
- No new MemoryStore coverage for the file-rename-collision, partial-move-resume, or
  installed-package-refusal tests — those remain FileStore-only for now (not required by the
  issue; deferring them avoids scope creep in a small mechanical fix).
- No change to the `.srsj` tree-session store arm; the issue and CLAUDE.md rule name `MemoryStore`
  specifically as the canonical double.

---

## Phases

### Phase 1: Add MemoryStore factory and refactor shared cases into generic suite fns

**Goal:** `crates/srs-repository/tests/package_move.rs` has a `MemoryStore` factory and the three
named cases run against both stores via shared `fn run_..._suite(store: &dyn RepositoryStore)`
helpers.

**Agent:** Repository Worker

#### Tasks

- [x] Add `fn memory_repo() -> MemoryStore` (or populate an existing `MemoryStore::empty()`
      inline) that runs the same `create_package`/`create_field_normalized`/
      `create_type_normalized`/`create_relation_type_normalized` sequence as `fn repo()`, against
      `MemoryStore::empty()` instead of `FileStore::new(...)` + `initialize_repository`.
- [x] Extract `moves_every_version_of_a_type_together`'s body into
      `fn run_moves_every_version_of_a_type_together(store: &dyn RepositoryStore)`, taking the
      store by trait object so it works for either backend; keep `list()`/`code()` helpers
      store-generic (already take `&FileStore` today — widen to `&dyn RepositoryStore` since both
      only call `load_instance_json`, which is on the trait).
- [x] Extract `refuses_an_unknown_id_an_unknown_target_and_the_same_boundary`'s body into
      `fn run_refuses_an_unknown_id_an_unknown_target_and_the_same_boundary(store: &dyn
      RepositoryStore)`.
- [x] Write a `MemoryStore`-appropriate dry-run suite fn,
      `fn run_a_dry_run_writes_nothing(store: &dyn RepositoryStore)`, asserting via
      `list(store, "package", "types")` / `list(store, SITE, "types")` before and after (equal)
      plus `out.moved.len() == 2` and `out.dry_run == true`, instead of the FileStore-only
      `snapshot()` byte comparison. Keep the existing FileStore `a_dry_run_writes_nothing` test
      (with its stronger byte-for-byte `snapshot()` assertion) as-is; add the new generic fn as an
      additional assertion path, not a replacement.
- [x] Add `#[test] fn memory_store_moves_every_version_of_a_type_together()`,
      `#[test] fn memory_store_refuses_an_unknown_id_an_unknown_target_and_the_same_boundary()`,
      `#[test] fn memory_store_a_dry_run_writes_nothing()` calling the new suite fns against
      `memory_repo()`.
- [x] Add `#[test] fn file_store_moves_every_version_of_a_type_together()` and
      `#[test] fn file_store_refuses_an_unknown_id_an_unknown_target_and_the_same_boundary()`
      calling the extracted suite fns against the existing `repo()` FileStore factory, replacing
      the two original single-backend tests (`moves_every_version_of_a_type_together`,
      `refuses_an_unknown_id_an_unknown_target_and_the_same_boundary`) so there is one definition
      of each case, run twice. Leave `a_dry_run_writes_nothing` (FileStore, byte-snapshot version)
      untouched and add the new memory-only dry-run test alongside it rather than merging the two
      dry-run assertions into one shared fn (the snapshot mechanism is disk-only).
- [x] Update the module doc comment at the top of `package_move.rs` to note the cross-store
      coverage, mirroring the note in `delete_membership_cascade.rs`'s header comment.

#### Acceptance Criteria

- [x] `cargo test -p srs-repository --test package_move` passes with the new `memory_store_*`
      tests present and green.
- [x] The two refactored cases (lineage-move-together, unknown-id/target/same-boundary refusals)
      each have exactly one shared suite fn, invoked from one `file_store_*` test and one
      `memory_store_*` test — no duplicated assertion logic between the two backends.
- [x] No change to any file under `crates/srs-repository/src/` (test-only diff).
- [x] Every pre-existing test in `package_move.rs` not named above (file-rename-collision,
      partial-move-resume, installed-package refusals, moves-a-relation-type,
      moves-back-into-primary, moves-a-type-that-records-use, package-names-primary-boundary)
      still exists and passes unchanged.

#### Testing

```bash
cargo test -p srs-repository --test package_move
cargo clippy -p srs-repository --all-targets -- -D warnings
```

Specific tests to write or verify:

- `memory_store_moves_every_version_of_a_type_together` — proves `move_definition` moves every
  version of a lineage together against the in-memory backend.
- `memory_store_a_dry_run_writes_nothing` — proves a dry run against `MemoryStore` reports the
  move but leaves the boundary lists unchanged.
- `memory_store_refuses_an_unknown_id_an_unknown_target_and_the_same_boundary` — proves the
  `definition-not-found`, `package-not-found` and `move-same-boundary` refusal codes are
  store-independent.
- `file_store_moves_every_version_of_a_type_together`,
  `file_store_refuses_an_unknown_id_an_unknown_target_and_the_same_boundary` — same assertions,
  FileStore backend, proving the extraction changed no FileStore behavior.

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository --all-targets -- -D warnings
```

4. Update this plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit, referencing the issue: `git commit` with a message ending `(#1654)`.

This is the only phase in this plan — the change is small enough that one phase covers it
end-to-end.

---

## Final Acceptance

- [x] `cargo test` passes with no failures (scoped: `cargo test -p srs-repository --test
      package_move` — 15 passed, 0 failed; see Stage 6 milestone-gate note below on scoping vs.
      a full `--workspace` run)
- [x] `cargo clippy -- -D warnings` passes (scoped: `cargo clippy -p srs-repository --all-targets
      -- -D warnings` — clean, exit 0)
- [x] CLI output format unchanged (no `srs-cli` files touched)
- [x] `cargo test --test payload_contracts` — not applicable, no payload structs changed; skip
- [x] `bash scripts/check-schema-sync.sh` — not applicable, no entity schemas changed; skip
- [x] `crates/srs-repository/tests/package_move.rs` exercises `MemoryStore` for lineage-move-
      together, dry-run, and refusal-code cases, matching the crate's established cross-store
      idiom
- [x] No production-code change in `package_move_service.rs` (diff vs `origin/master` touches
      only `crates/srs-repository/tests/package_move.rs` and this plan file)

## Coordination Rules

- Single-session change; no multi-agent write-scope conflicts expected.
- Do not revert or alter any pre-existing `FileStore`-only test not named in Scope.
- At the end of Phase 1: verify acceptance criteria, confirm tests exist and pass, update
  checkboxes, commit.

## Assumptions

- `MemoryStore::empty()` is the right factory (per `store_conformance.rs`'s own comment: "already
  carries a valid manifest/package in memory; `create_repository` is not required"), not
  `MemoryStore::uninitialized()` + `initialize_repository` (the latter is used in
  `store_conformance.rs` only where the test deliberately checks `initialize_repository`'s own
  behavior, which this plan does not).
- `create_package`/`create_field_normalized`/`create_type_normalized`/
  `create_relation_type_normalized` all already take `&dyn RepositoryStore` (confirmed: see
  `crates/srs-repository/src/package_service.rs` signatures), so the existing `repo()` body's
  calls port to a `MemoryStore` argument with no change beyond the store construction itself.
- `move_definition<S: RepositoryStore + ?Sized>` is already generic over the store (confirmed in
  `crates/srs-repository/src/package_move_service.rs`), so no signature change is needed to call
  it with either backend.
