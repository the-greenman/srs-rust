# Plan: Move storage-path constants from `paths.rs` to store boundary (#418)

> **Usage note:** This plan is written for agent execution. Tasks include explicit file paths,
> named functions, and checkable acceptance criteria.

## Summary

Issue #228 consolidated hardcoded path strings into `crates/srs-repository/src/paths.rs`, but
those constants still live in service-crate code and are imported directly by service modules.
CLAUDE.md §Storage Boundary Rules and ADR-008 both require that path strings (filesystem layout
knowledge) belong to the store adapter, not service logic. This plan introduces a `RecordTier`
enum in `store.rs`, adds a `record_tier_dir` default trait method that maps enum variants to
path strings, replaces all `paths::*` imports in service files with enum-based calls, and then
deletes `paths.rs`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Repository Service Worker |
| Repository Service Worker | Repository Service Worker |
| Verification | Verification Agent |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | "FileStore preserves the existing filesystem layout, but that layout is an adapter detail, not part of service contracts." This plan enforces that constraint by moving path strings out of service files. | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions must not contain raw path strings; the store boundary owns filesystem layout knowledge. | accepted |

No new ADRs needed — this plan implements an already-accepted constraint from ADR-008 and ADR-010.

### Design choice: `RecordTier` enum + default trait method

The path strings move into `RecordTier::dir()` — an associated method on a new `pub(crate)` enum
in `store.rs`. The `RepositoryStore` trait gets a default method `fn record_tier_dir(&self, tier:
RecordTier) -> &'static str { tier.dir() }`. Services call `store.record_tier_dir(RecordTier::Note)`
instead of `paths::NOTES_RECORD_DIR`. The path strings live once, in `store.rs` alongside the
store definitions. All three adapters (FileStore, MemoryStore, JsonStore) inherit the default;
a future SQL adapter can override. This satisfies the rule "must not appear in service logic"
without requiring all four adapter implementations to be updated separately.

The one accepted exception: `canonical_instance_path` in `repository_portability.rs` has a
structural catch-all `tier => format!("records/tier-{tier}/{filename}")` for snapshot instances
with UNKNOWN tier numbers. This is not a path constant — it is a defensive pattern for arbitrary
numeric tiers encountered during import. Known tiers (0, 1, 2) are replaced with `RecordTier` enum
calls. The catch-all raw string is retained with a comment.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. No new payload structs. No `generate-schemas` run
needed. `cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files changed. `bash scripts/check-schema-sync.sh` must still exit 0.

---

## Scope

- Add `RecordTier` enum with `Note`, `Tier1`, `Tier2`, `Extension` variants to `store.rs`
- Add `RecordTier::dir(self) -> &'static str` method containing all four path mappings
- Add `fn record_tier_dir(&self, tier: RecordTier) -> &'static str` default method to `RepositoryStore` trait
- Replace every `use crate::paths::*` import in service files with `use crate::store::RecordTier`
- Replace every `paths::*` usage in service files with `store.record_tier_dir(RecordTier::*)`
- Replace `paths::NOTES_RECORD_DIR`/`TIER1_RECORD_DIR`/`DEFAULT_RECORD_DIR` in `canonical_instance_path` with `RecordTier::*.dir()` (no store param needed)
- Delete `crates/srs-repository/src/paths.rs`
- Remove `pub(crate) mod paths;` from `crates/srs-repository/src/lib.rs`

**Out of scope:**

- Changing `RepositoryStore` method signatures (e.g. `list_instance_files`, `ensure_instance_dir` still take `&str` — services call them with `store.record_tier_dir(...)` as the argument)
- Adding new store methods beyond `record_tier_dir`
- Migrating json_store.rs's own path constants (e.g. for fields, types — those are already inside the store file)
- Any changes to `srs-bindings`, `srs-cli`, or `srs-core`

---

## Phases

### Phase 1: Add `RecordTier` enum and `record_tier_dir` default method

**Goal:** `RecordTier` is defined in `store.rs` and usable by service files; `paths.rs` still
exists and is still used.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/store.rs`, above the `RepositoryStore` trait definition,
  add:
  ```rust
  /// Identifies the logical storage tier for instance records.
  /// Adapters map this to backend-specific paths or keys via `record_tier_dir`.
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub(crate) enum RecordTier {
      /// Notes (Tier 0): free-text sections — maps to `records/notes`
      Note,
      /// Typed records (Tier 1): named fields, no Type binding — maps to `records/tier-1`
      Tier1,
      /// Records (Tier 2): instantiated Type — maps to `records/tier-2`
      Tier2,
      /// Extension/package records — maps to `package/records`
      Extension,
  }

  impl RecordTier {
      /// Returns the relative directory path for this tier.
      ///
      /// This is the single source of truth for instance storage paths.
      /// All path strings must stay here, not in service modules.
      pub(crate) fn dir(self) -> &'static str {
          match self {
              RecordTier::Note => "records/notes",
              RecordTier::Tier1 => "records/tier-1",
              RecordTier::Tier2 => "records/tier-2",
              RecordTier::Extension => "package/records",
          }
      }
  }
  ```
- [ ] In the `RepositoryStore` trait in `store.rs`, add a default method after the instance
  section (near `list_instance_files`):
  ```rust
  /// Returns the relative directory for instance records of the given tier.
  ///
  /// Default implementation delegates to `RecordTier::dir()`. Override in adapters
  /// that use a different storage layout (e.g. SQL table names).
  fn record_tier_dir(&self, tier: RecordTier) -> &'static str {
      tier.dir()
  }
  ```
- [ ] Compile check only (no caller changes yet):
  ```bash
  cargo build -p srs-repository 2>&1 | head -20
  ```

#### Acceptance Criteria

- [ ] `RecordTier` enum is in `store.rs` with four variants and a `dir()` method
- [ ] `RepositoryStore` trait has a `record_tier_dir` default method
- [ ] `paths.rs` still exists (not yet deleted)
- [ ] `cargo build -p srs-repository` compiles cleanly

#### Testing

```bash
cargo build -p srs-repository
```

#### Milestone gate

1. Verify enum is defined with correct paths: `grep -A 20 "enum RecordTier" crates/srs-repository/src/store.rs`
2. Verify trait method is present: `grep -A 5 "fn record_tier_dir" crates/srs-repository/src/store.rs`
3. `cargo build -p srs-repository` exits 0
4. Commit:
```bash
git add crates/srs-repository/src/store.rs
git commit -m "feat(repository): add RecordTier enum and record_tier_dir default method (#418)"
```

---

### Phase 2: Replace all `paths::*` imports and usages in service files

**Goal:** Every service file that previously imported from `paths.rs` now uses
`store.record_tier_dir(RecordTier::*)` or `RecordTier::*.dir()` instead. `paths.rs` is
still present but has zero callers outside itself.

**Agent:** Repository Service Worker

#### Tasks

File-by-file changes (treat each as an independent sub-task):

**`crates/srs-repository/src/record_store.rs`** (uses `DEFAULT_RECORD_DIR` extensively)
- [ ] Remove `use crate::paths::DEFAULT_RECORD_DIR;`
- [ ] Add `use crate::store::RecordTier;`
- [ ] Replace every occurrence of `DEFAULT_RECORD_DIR` with `store.record_tier_dir(RecordTier::Tier2)`
  where `store: &dyn RepositoryStore` is in scope.
  Key sites (line numbers from HEAD):
  - `create_record` (line ~116): `create_record_at_dir(..., DEFAULT_RECORD_DIR)` → `create_record_at_dir(..., store.record_tier_dir(RecordTier::Tier2))`
  - `write_new_record` (line ~1408): takes `dir: &str` — callers pass `store.record_tier_dir(RecordTier::Tier2)`, the function signature stays unchanged
  - All other direct usages: search with `grep -n DEFAULT_RECORD_DIR crates/srs-repository/src/record_store.rs` and update each

**`crates/srs-repository/src/services.rs`** (uses `NOTES_RECORD_DIR`)
- [ ] Remove `use crate::paths::NOTES_RECORD_DIR;`
- [ ] Add `use crate::store::RecordTier;`
- [ ] Replace `NOTES_RECORD_DIR` with `store.record_tier_dir(RecordTier::Note)` at every usage in
  functions that have a `store: &dyn RepositoryStore` parameter
  Key sites (line numbers from HEAD):
  - `format!("{NOTES_RECORD_DIR}/{id8}.json")` → `format!("{}/{id8}.json", store.record_tier_dir(RecordTier::Note))`
  - `store.ensure_instance_dir(NOTES_RECORD_DIR)` → `store.ensure_instance_dir(store.record_tier_dir(RecordTier::Note))`
  - `std::path::PathBuf::from(NOTES_RECORD_DIR)` in error constructors → `std::path::PathBuf::from(store.record_tier_dir(RecordTier::Note))`

**`crates/srs-repository/src/extension_service.rs`** (uses `EXTENSION_RECORD_DIR`)
- [ ] Remove `use crate::paths::EXTENSION_RECORD_DIR;`
- [ ] Add `use crate::store::RecordTier;`
- [ ] Replace `EXTENSION_RECORD_DIR` with `store.record_tier_dir(RecordTier::Extension)` at all
  usage sites in functions that have `store: &dyn RepositoryStore`
  Key sites:
  - `create_record_at_dir(..., EXTENSION_RECORD_DIR)` → `create_record_at_dir(..., store.record_tier_dir(RecordTier::Extension))`
  - `store.list_instance_files(EXTENSION_RECORD_DIR)` → `store.list_instance_files(store.record_tier_dir(RecordTier::Extension))`
  - `format!("{EXTENSION_RECORD_DIR}/{id}.json")` → `format!("{}/{id}.json", store.record_tier_dir(RecordTier::Extension))`

**`crates/srs-repository/src/repository_lifecycle.rs`** (uses `DEFAULT_RECORD_DIR`)
- [ ] Remove `use crate::paths::DEFAULT_RECORD_DIR;`
- [ ] Add `use crate::store::RecordTier;`
- [ ] Replace `DEFAULT_RECORD_DIR` with `store.record_tier_dir(RecordTier::Tier2)`

**`crates/srs-repository/src/migrate_identity_service.rs`** (uses `paths::DEFAULT_RECORD_DIR`)
- [ ] Remove `use crate::paths;` (or replace with `use crate::store::RecordTier;`)
- [ ] Replace `paths::DEFAULT_RECORD_DIR` with `store.record_tier_dir(RecordTier::Tier2)` at all
  four call sites (lines ~103, ~182, ~306, ~609)

**`crates/srs-repository/src/analysis.rs`** (uses `NOTES_RECORD_DIR`)
- [ ] Remove `use crate::paths::NOTES_RECORD_DIR;`
- [ ] Add `use crate::store::RecordTier;`
- [ ] Replace `std::path::PathBuf::from(NOTES_RECORD_DIR)` with
  `std::path::PathBuf::from(store.record_tier_dir(RecordTier::Note))` in the error constructor
  at line ~226

**`crates/srs-repository/src/repository_portability.rs`** (uses three constants)
- [ ] Remove `use crate::paths::{DEFAULT_RECORD_DIR, NOTES_RECORD_DIR, TIER1_RECORD_DIR};`
- [ ] Add `use crate::store::RecordTier;`
- [ ] In `canonical_instance_path` (line ~763), replace known-tier constants using enum `dir()`:
  ```rust
  match instance.tier {
      0 => format!("{}/{filename}", RecordTier::Note.dir()),
      1 => format!("{}/{filename}", RecordTier::Tier1.dir()),
      2 => format!("{}/{filename}", RecordTier::Tier2.dir()),
      // Defensive catch-all for unknown tier numbers in snapshot data.
      // Cannot be expressed as a typed RecordTier variant.
      tier => format!("records/tier-{tier}/{filename}"),
  }
  ```
  Note: `canonical_instance_path` has no `store` parameter; using `RecordTier::*.dir()` directly
  avoids adding one. The catch-all raw string is retained as an accepted structural pattern for
  unknown tiers, not a named constant.

#### Acceptance Criteria

- [ ] `grep -rn "use crate::paths" crates/srs-repository/src/` returns empty
- [ ] `grep -rn "paths::" crates/srs-repository/src/ | grep -v "paths.rs"` returns empty
- [ ] `grep -rn "DEFAULT_RECORD_DIR\|NOTES_RECORD_DIR\|TIER1_RECORD_DIR\|EXTENSION_RECORD_DIR" crates/srs-repository/src/ | grep -v "paths.rs"` returns empty
- [ ] `cargo test -p srs-repository` passes
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests verifying behavior is unchanged:
- All existing `record_store` tests — prove create/list/delete record ops still write to `records/tier-2`
- All existing `services.rs` note tests — prove note ops still write to `records/notes`
- `extension_service` tests — prove extension ops still write to `package/records`
- `repository_portability` tests — prove `canonical_instance_path` maps tier numbers correctly

#### Milestone gate

1. Run all acceptance criteria checks above
2. `cargo test -p srs-repository` passes — record all pass/fail counts
3. Commit each file group in logical batches, e.g.:
```bash
git add crates/srs-repository/src/record_store.rs crates/srs-repository/src/repository_lifecycle.rs
git commit -m "refactor(repository): replace DEFAULT_RECORD_DIR with store.record_tier_dir (#418)"

git add crates/srs-repository/src/services.rs crates/srs-repository/src/analysis.rs
git commit -m "refactor(repository): replace NOTES_RECORD_DIR with store.record_tier_dir (#418)"

git add crates/srs-repository/src/extension_service.rs
git commit -m "refactor(repository): replace EXTENSION_RECORD_DIR with store.record_tier_dir (#418)"

git add crates/srs-repository/src/migrate_identity_service.rs crates/srs-repository/src/repository_portability.rs
git commit -m "refactor(repository): replace remaining paths:: usages with RecordTier (#418)"
```

---

### Phase 3: Delete `paths.rs` and clean up

**Goal:** `paths.rs` no longer exists; `lib.rs` has no `mod paths` declaration; the codebase
compiles and all tests pass.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Verify no remaining callers: `grep -rn "paths::\|mod paths\|use crate::paths" crates/srs-repository/src/` must return empty
- [ ] Delete `crates/srs-repository/src/paths.rs`
- [ ] In `crates/srs-repository/src/lib.rs`, remove the line `pub(crate) mod paths;`
- [ ] `cargo build -p srs-repository` — confirm no "unresolved module" errors
- [ ] `cargo test -p srs-repository` — full suite passes

#### Acceptance Criteria

- [ ] `crates/srs-repository/src/paths.rs` does not exist
- [ ] `crates/srs-repository/src/lib.rs` has no `mod paths` line
- [ ] `grep -rn "paths" crates/srs-repository/src/` returns no service-logic hits (only the `record_tier_dir` comment if any)
- [ ] `cargo test -p srs-repository` passes
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. Confirm file is deleted and lib.rs is updated
2. All tests pass
3. Commit:
```bash
git add -u crates/srs-repository/src/
git commit -m "refactor(repository): delete paths.rs — path constants now in RecordTier::dir (#418)"
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures (full workspace)
- [ ] `cargo clippy -- -D warnings` passes (full workspace)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `crates/srs-repository/src/paths.rs` does not exist
- [ ] `grep -rn "use crate::paths" crates/srs-repository/src/` returns empty
- [ ] `grep -rn "paths::" crates/srs-repository/src/ | grep -v "pub(crate) mod paths"` returns empty
- [ ] All pre-existing integration tests in `cargo test -p srs-cli --test integration_tests` pass

## Coordination Rules

- Keep to `crates/srs-repository/` write scope only — no changes to `srs-cli`, `srs-bindings`, `srs-core`.
- Do not change any public (non-`pub(crate)`) API surface.
- Do not change `RepositoryStore` method signatures (e.g. `list_instance_files`, `ensure_instance_dir` remain `&str`-based).
- Run the milestone gate before moving to the next phase.

## Assumptions

- `paths.rs` constants are only imported by `srs-repository` internal modules (confirmed by initial grep).
- `canonical_instance_path` catch-all for unknown tiers is an accepted structural pattern, not a named constant violation.
- `core_package.rs` (added in #423/PR#467) does not reference `paths.rs` — confirm with `grep "paths" crates/srs-repository/src/core_package.rs` before Phase 3.
- `BrokenManifestStore` in `relation_service.rs` does not need updating because `record_tier_dir` has a default implementation.
