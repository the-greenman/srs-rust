# Plan: Move storage-path constants from `paths.rs` to store boundary (#418)

> **Usage note:** This plan is written for agent execution. Tasks include explicit file paths,
> named functions, and checkable acceptance criteria. Line numbers are approximate — use the
> grep commands in each task to find the exact location before editing.

## Summary

Issue #228 consolidated hardcoded path strings into `crates/srs-repository/src/paths.rs`, but
those constants still live in service-crate code and are imported directly by service modules.
CLAUDE.md §Storage Boundary Rules and ADR-008 both require that path strings (filesystem layout
knowledge) belong to the store adapter, not service logic. This plan introduces a `RecordTier`
enum in `store.rs`, adds a `record_tier_dir` **required** trait method (no default body) that
adapters implement explicitly, replaces all `paths::*` imports in service files with
`store.record_tier_dir(RecordTier::*)` calls, and then deletes `paths.rs`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Repository Service Worker |
| Repository Service Worker | Repository Service Worker |
| Verification | Verification Agent |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | "FileStore preserves the existing filesystem layout, but that layout is an adapter detail, not part of service contracts." This plan enforces that constraint by moving path strings out of service files and into adapters. | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions must not contain raw path strings; the store boundary owns filesystem layout knowledge. | accepted |

No new ADRs needed — this plan implements an already-accepted constraint from ADR-008 and ADR-010.

### Design choice: `RecordTier` enum + required trait method

The path strings move into `RecordTier::dir()` — a **private** associated method on a new
`pub(crate)` enum in `store.rs`. The `RepositoryStore` trait gets a **required** (no default body)
method `fn record_tier_dir(&self, tier: RecordTier) -> &'static str`. Each adapter must implement
it explicitly:

- `FileStore` and `MemoryStore` (both in `store.rs`): `fn record_tier_dir(&self, tier: RecordTier) -> &'static str { tier.dir() }`
- `JsonStore` (in `json_store.rs`): own `match` block with the same four strings (cannot call the private `dir()`)
- `BrokenManifestStore` (in `relation_service.rs`): `unreachable!("record_tier_dir not expected in BrokenManifestStore tests")`

Making `record_tier_dir` **required** (not a default method) ensures adapters explicitly
declare their layout — this is the ADR-008 constraint: layout is adapter-owned, not in the
contract itself. Making `dir()` **private** (no visibility qualifier) prevents service code from
bypassing store dispatch and calling `RecordTier::*.dir()` directly.

Services call `store.record_tier_dir(RecordTier::Note)` instead of `paths::NOTES_RECORD_DIR`.
The path strings live once, in `store.rs` alongside the store definitions (via `dir()`), and are
reachable only through adapter dispatch.

The one accepted exception: `canonical_instance_path` in `repository_portability.rs` has a
structural catch-all `tier => format!("records/tier-{tier}/{filename}")` for snapshot instances
with UNKNOWN tier numbers. This is not a path constant — it is a defensive pattern for arbitrary
numeric tiers encountered during import. Known tiers (0, 1, 2) use `store.record_tier_dir(RecordTier::*)`.
The catch-all raw string is retained intentionally with a comment.

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
- Add `RecordTier::dir(self) -> &'static str` as a **private** method (no `pub`/`pub(crate)`) — callable only within `store.rs`
- Add `fn record_tier_dir(&self, tier: RecordTier) -> &'static str` as a **required** method (no default body) to `RepositoryStore` trait
- Implement `record_tier_dir` explicitly in all four adapters: FileStore, MemoryStore, JsonStore, BrokenManifestStore
- Add pinning test `record_tier_dir_values` in `store.rs` verifying all four path strings
- Replace every `use crate::paths::*` import in service files with `use crate::store::RecordTier`
- Replace every `paths::*` usage in service files with `store.record_tier_dir(RecordTier::*)`
- Update `canonical_instance_path` signature to take `store: &dyn RepositoryStore` parameter; use `store.record_tier_dir(RecordTier::*)` for known tiers; update both call sites
- Update doc comments and test asserts referencing path constant values (e.g. `"records/tier-2"`)
- Delete `crates/srs-repository/src/paths.rs`
- Remove `pub(crate) mod paths;` from `crates/srs-repository/src/lib.rs`

**Out of scope:**

- Changing `RepositoryStore` method signatures (e.g. `list_instance_files`, `ensure_instance_dir` still take `&str` — services call them with `store.record_tier_dir(...)` as the argument)
- Adding new store methods beyond `record_tier_dir`
- Migrating `json_store.rs`'s own path constants (e.g. for fields, types — those are already inside the store file)
- Any changes to `srs-bindings`, `srs-cli`, or `srs-core`

---

## Phases

### Phase 1: Add `RecordTier` enum and `record_tier_dir` required method

**Goal:** `RecordTier` is defined in `store.rs` with a private `dir()` method; `record_tier_dir`
is a required trait method implemented explicitly in all four adapters; a pinning test confirms
the path strings. `paths.rs` still exists and is still used.

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
      fn dir(self) -> &'static str {
          match self {
              RecordTier::Note      => "records/notes",
              RecordTier::Tier1     => "records/tier-1",
              RecordTier::Tier2     => "records/tier-2",
              RecordTier::Extension => "package/records",
          }
      }
  }
  ```
  Note: `dir()` has no visibility qualifier — private to `store.rs` only. This prevents service
  code from bypassing adapter dispatch.

- [ ] In the `RepositoryStore` trait in `store.rs`, add a **required** method (no default body)
  near `list_instance_files`:
  ```rust
  /// Returns the relative directory for instance records of the given tier.
  ///
  /// Required — each adapter declares its own layout explicitly.
  fn record_tier_dir(&self, tier: RecordTier) -> &'static str;
  ```

- [ ] In `FileStore`'s `impl RepositoryStore for FileStore` in `store.rs`, add:
  ```rust
  fn record_tier_dir(&self, tier: RecordTier) -> &'static str {
      tier.dir()
  }
  ```

- [ ] In `MemoryStore`'s `impl RepositoryStore for MemoryStore` in `store.rs`, add:
  ```rust
  fn record_tier_dir(&self, tier: RecordTier) -> &'static str {
      tier.dir()
  }
  ```

- [ ] In `crates/srs-repository/src/json_store.rs`, in `impl RepositoryStore for JsonStore`,
  add:
  ```rust
  fn record_tier_dir(&self, tier: RecordTier) -> &'static str {
      match tier {
          RecordTier::Note      => "records/notes",
          RecordTier::Tier1     => "records/tier-1",
          RecordTier::Tier2     => "records/tier-2",
          RecordTier::Extension => "package/records",
      }
  }
  ```
  (JsonStore cannot call the private `dir()` from `store.rs`, so it has its own match block.)
  Add `use crate::store::RecordTier;` to the imports if not already present.

- [ ] In `crates/srs-repository/src/relation_service.rs`, locate `BrokenManifestStore` and
  its `impl RepositoryStore for BrokenManifestStore`. Add:
  ```rust
  fn record_tier_dir(&self, _tier: RecordTier) -> &'static str {
      unreachable!("record_tier_dir not expected in BrokenManifestStore tests")
  }
  ```
  Add `use crate::store::RecordTier;` to the imports in that file if not already present.

- [ ] In `store.rs`, add a pinning test in the `#[cfg(test)]` section:
  ```rust
  #[test]
  fn record_tier_dir_values() {
      let store = MemoryStore::empty();
      assert_eq!(store.record_tier_dir(RecordTier::Note),      "records/notes");
      assert_eq!(store.record_tier_dir(RecordTier::Tier1),     "records/tier-1");
      assert_eq!(store.record_tier_dir(RecordTier::Tier2),     "records/tier-2");
      assert_eq!(store.record_tier_dir(RecordTier::Extension), "package/records");
  }
  ```

- [ ] Compile check only (no caller changes yet):
  ```bash
  cargo build -p srs-repository 2>&1 | head -20
  ```

#### Acceptance Criteria

- [ ] `RecordTier` enum is in `store.rs` with four variants and a private `dir()` method
- [ ] `RepositoryStore` trait has `record_tier_dir` as a required method (no default body)
- [ ] All four adapters implement `record_tier_dir` explicitly
- [ ] `record_tier_dir_values` pinning test is in `store.rs`
- [ ] `paths.rs` still exists (not yet deleted)
- [ ] `cargo build -p srs-repository` compiles cleanly

#### Testing

```bash
cargo test -p srs-repository record_tier_dir_values
cargo build -p srs-repository
```

Expected: `test record_tier_dir_values ... ok`

#### Milestone gate

1. Verify enum is defined: `grep -A 20 "enum RecordTier" crates/srs-repository/src/store.rs`
2. Verify `dir()` has no pub prefix: `grep "fn dir" crates/srs-repository/src/store.rs` — must NOT show `pub`
3. Verify trait method has no body: `grep -A 3 "fn record_tier_dir" crates/srs-repository/src/store.rs`
4. Verify four adapter impls: `grep -c "fn record_tier_dir" crates/srs-repository/src/store.rs` (expect 3: trait + FileStore + MemoryStore), `grep -c "fn record_tier_dir" crates/srs-repository/src/json_store.rs` (expect 1), `grep -c "fn record_tier_dir" crates/srs-repository/src/relation_service.rs` (expect 1)
5. `cargo test -p srs-repository record_tier_dir_values` passes
6. `cargo build -p srs-repository` exits 0
7. Commit:
```bash
git add crates/srs-repository/src/store.rs crates/srs-repository/src/json_store.rs crates/srs-repository/src/relation_service.rs
git commit -m "feat(repository): add RecordTier enum and record_tier_dir required method (#418)"
```

---

### Phase 2: Replace all `paths::*` imports and usages in service files

**Goal:** Every service file that previously imported from `paths.rs` now uses
`store.record_tier_dir(RecordTier::*)` instead. `canonical_instance_path` takes a `store`
parameter and uses `store.record_tier_dir()` for known tiers. `paths.rs` is still present but
has zero callers outside itself.

**Agent:** Repository Service Worker

#### Tasks

File-by-file changes. For each file, use grep to find exact line numbers before editing.

**`crates/srs-repository/src/record_store.rs`** (uses `DEFAULT_RECORD_DIR`)
- [ ] Find all usages: `grep -n "DEFAULT_RECORD_DIR\|use crate::paths" crates/srs-repository/src/record_store.rs`
- [ ] Remove `use crate::paths::DEFAULT_RECORD_DIR;`
- [ ] Add `use crate::store::RecordTier;`
- [ ] Replace every occurrence of `DEFAULT_RECORD_DIR` with `store.record_tier_dir(RecordTier::Tier2)`
  where `store: &dyn RepositoryStore` is in scope.
  Key sites:
  - `create_record` function: `create_record_at_dir(..., DEFAULT_RECORD_DIR)` → `create_record_at_dir(..., store.record_tier_dir(RecordTier::Tier2))`
  - `write_new_record` function takes `dir: &str` — callers pass `store.record_tier_dir(RecordTier::Tier2)`, the function signature stays unchanged
  - Doc comments referencing `DEFAULT_RECORD_DIR` or `"records/tier-2"` — update to use the enum description
  - Test asserts like `assert!(entry.path().starts_with(DEFAULT_RECORD_DIR), "expected path under {DEFAULT_RECORD_DIR}...")` → replace constant with literal `"records/tier-2"` (test helpers don't have a store)

**`crates/srs-repository/src/services.rs`** (uses `NOTES_RECORD_DIR`)
- [ ] Find all usages: `grep -n "NOTES_RECORD_DIR\|use crate::paths" crates/srs-repository/src/services.rs`
- [ ] Remove `use crate::paths::NOTES_RECORD_DIR;`
- [ ] Add `use crate::store::RecordTier;`
- [ ] Replace `NOTES_RECORD_DIR` with `store.record_tier_dir(RecordTier::Note)` at every usage in
  functions that have a `store: &dyn RepositoryStore` parameter:
  - `format!("{NOTES_RECORD_DIR}/{id8}.json")` → `format!("{}/{id8}.json", store.record_tier_dir(RecordTier::Note))`
  - `store.ensure_instance_dir(NOTES_RECORD_DIR)` → `store.ensure_instance_dir(store.record_tier_dir(RecordTier::Note))`
  - `std::path::PathBuf::from(NOTES_RECORD_DIR)` in error constructors → `std::path::PathBuf::from(store.record_tier_dir(RecordTier::Note))`

**`crates/srs-repository/src/extension_service.rs`** (uses `EXTENSION_RECORD_DIR`)
- [ ] Find all usages: `grep -n "EXTENSION_RECORD_DIR\|use crate::paths" crates/srs-repository/src/extension_service.rs`
- [ ] Remove `use crate::paths::EXTENSION_RECORD_DIR;`
- [ ] Add `use crate::store::RecordTier;`
- [ ] Replace `EXTENSION_RECORD_DIR` with `store.record_tier_dir(RecordTier::Extension)` at all
  usage sites:
  - `create_record_at_dir(..., EXTENSION_RECORD_DIR)` → `create_record_at_dir(..., store.record_tier_dir(RecordTier::Extension))`
  - `store.list_instance_files(EXTENSION_RECORD_DIR)` → `store.list_instance_files(store.record_tier_dir(RecordTier::Extension))`
  - `format!("{EXTENSION_RECORD_DIR}/{id}.json")` → `format!("{}/{id}.json", store.record_tier_dir(RecordTier::Extension))`

**`crates/srs-repository/src/repository_lifecycle.rs`** (uses `DEFAULT_RECORD_DIR`)
- [ ] Find all usages: `grep -n "DEFAULT_RECORD_DIR\|use crate::paths" crates/srs-repository/src/repository_lifecycle.rs`
- [ ] Remove `use crate::paths::DEFAULT_RECORD_DIR;`
- [ ] Add `use crate::store::RecordTier;`
- [ ] Replace `DEFAULT_RECORD_DIR` with `store.record_tier_dir(RecordTier::Tier2)`

**`crates/srs-repository/src/migrate_identity_service.rs`** (uses `paths::DEFAULT_RECORD_DIR`)
- [ ] Find all usages: `grep -n "paths::\|use crate::paths" crates/srs-repository/src/migrate_identity_service.rs`
- [ ] Remove `use crate::paths;` (or the specific `use crate::paths::DEFAULT_RECORD_DIR;`)
- [ ] Add `use crate::store::RecordTier;`
- [ ] Replace `paths::DEFAULT_RECORD_DIR` with `store.record_tier_dir(RecordTier::Tier2)` at all
  four call sites

**`crates/srs-repository/src/analysis.rs`** (uses `NOTES_RECORD_DIR`)
- [ ] Find all usages: `grep -n "NOTES_RECORD_DIR\|use crate::paths" crates/srs-repository/src/analysis.rs`
- [ ] Remove `use crate::paths::NOTES_RECORD_DIR;`
- [ ] Add `use crate::store::RecordTier;`
- [ ] Replace `std::path::PathBuf::from(NOTES_RECORD_DIR)` with
  `std::path::PathBuf::from(store.record_tier_dir(RecordTier::Note))` in the error constructor

**`crates/srs-repository/src/repository_portability.rs`** (uses three constants; `canonical_instance_path` needs store param)
- [ ] Find all usages: `grep -n "DEFAULT_RECORD_DIR\|NOTES_RECORD_DIR\|TIER1_RECORD_DIR\|use crate::paths" crates/srs-repository/src/repository_portability.rs`
- [ ] Remove `use crate::paths::{DEFAULT_RECORD_DIR, NOTES_RECORD_DIR, TIER1_RECORD_DIR};`
- [ ] Add `use crate::store::{RecordTier, RepositoryStore};` (if `RepositoryStore` not already imported)
- [ ] Update `canonical_instance_path` signature and body:
  ```rust
  pub(crate) fn canonical_instance_path(instance: &SnapshotInstance, store: &dyn RepositoryStore) -> String {
      // ... existing logic for filename ...
      match instance.tier {
          0 => format!("{}/{filename}", store.record_tier_dir(RecordTier::Note)),
          1 => format!("{}/{filename}", store.record_tier_dir(RecordTier::Tier1)),
          2 => format!("{}/{filename}", store.record_tier_dir(RecordTier::Tier2)),
          // Defensive catch-all for unknown tier numbers in snapshot data.
          // Cannot be expressed as a typed RecordTier variant.
          tier => format!("records/tier-{tier}/{filename}"),
      }
  }
  ```
- [ ] Find both call sites of `canonical_instance_path` with:
  `grep -n "canonical_instance_path" crates/srs-repository/src/repository_portability.rs`
  Expected sites: approximately line ~280 (using `target` store) and line ~687 (using `store` param).
  Update each call to pass the appropriate store reference as the second argument.

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

- [ ] Verify no remaining callers across all service files AND `core_package.rs` (added in #423):
  ```bash
  grep -rn "paths::\|mod paths\|use crate::paths" crates/srs-repository/src/
  grep -n "paths" crates/srs-repository/src/core_package.rs
  ```
  Both must return empty (or only hits inside `paths.rs` itself).
- [ ] Delete `crates/srs-repository/src/paths.rs`
- [ ] In `crates/srs-repository/src/lib.rs`, remove the line `pub(crate) mod paths;`
  Verify: `grep "mod paths" crates/srs-repository/src/lib.rs` must return empty
- [ ] `cargo build -p srs-repository` — confirm no "unresolved module" errors
- [ ] `cargo test -p srs-repository` — full suite passes

#### Acceptance Criteria

- [ ] `crates/srs-repository/src/paths.rs` does not exist
- [ ] `grep "mod paths" crates/srs-repository/src/lib.rs` returns empty
- [ ] `grep -rn "paths" crates/srs-repository/src/` returns no service-logic hits (only `record_tier_dir` doc comment references if any)
- [ ] `cargo test -p srs-repository` passes
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. Confirm file is deleted: `ls crates/srs-repository/src/paths.rs 2>&1` — must say "No such file"
2. Confirm `lib.rs` updated: `grep "mod paths" crates/srs-repository/src/lib.rs` — must return empty
3. All tests pass
4. Commit:
```bash
git add -u crates/srs-repository/src/
git commit -m "refactor(repository): delete paths.rs — path constants now in RecordTier (#418)"
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
- `core_package.rs` (added in #423/PR#467) does not reference `paths.rs` — confirmed with `grep "paths" crates/srs-repository/src/core_package.rs` before Phase 3 deletion.
- `BrokenManifestStore` in `relation_service.rs` gets an `unreachable!()` implementation since none of its tests exercise the record tier path.
