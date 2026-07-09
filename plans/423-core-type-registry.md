# Plan: Core-Type Registry — Implicit Core Package Merge (#423)

## Summary

Every SRS repository must resolve `com.semanticops.core/*` types (specifically
`purpose`, added by RFC-018) with zero per-repo config. This plan implements
**Mechanism A**: embed the canonical core-bundle artifact in `srs-repository`
and implicitly merge its fields and types into every `load_package()` result,
in both `FileStore` and `MemoryStore`. After this change,
`package.resolve_type_by_name("com.semanticops.core", "purpose")` succeeds on
every repository, and `srs type list` surfaces core types automatically. A
repo that explicitly declares a `com.semanticops.core/*` field or type fails
with a loud conflict error.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification Agent | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-004](../docs/adr/004-schemas-embedded-at-compile-time.md) | Core bundle embedded at compile time via `include_str!`, not fetched at runtime | accepted |
| [ADR-009](../docs/adr/009-package-boundary-model.md) | Core package is NOT a packageRef boundary — it merges transparently without any manifest entry | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Merge logic in `srs-repository`, not in `srs-cli` | accepted |
| [ADR-025 (new)](../docs/adr/025-implicit-core-package-merge.md) | `com.semanticops.core` is implicitly available; repos declaring their own core types fail loudly | proposed |

_New ADR-025 drafted in Phase 1 — records the implicit-merge invariant so it
is not revisited._

---

## Contracts

### CLI output contract (ADR-011)

`repo map` output gains a new `corePackage` field (in `RepoMap` → serialised
into the `repoMap` key of `RepoMapPayload`). `RepoMapPayload` uses
`#[schemars(with = "serde_json::Value")]` so the JSON Schema golden file is
**unchanged** — no `cargo run --bin generate-schemas` needed. The golden test
passes as-is.

`srs type list` automatically includes core types via the merged package — no
payload struct change.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/` entity schemas. No drift check needed.

---

## Scope

- `crates/srs-repository/assets/core-bundle.srsj` — embedded core artifact
- `crates/srs-repository/src/core_package.rs` — new module; lazy-parsed package
- `crates/srs-repository/src/store.rs` — merge core types in FileStore + MemoryStore
- `crates/srs-repository/src/analysis.rs` — `CorePackageSummary` field in `RepoMap`
- `crates/srs-repository/src/core_purpose.rs` — update comment; add constant-validation test
- `docs/adr/025-implicit-core-package-merge.md` — new ADR

**Out of scope:**
- Replacing `core_purpose` hardcoded constants in callers — deferred to #434 (WASM binding plan)
- WASM binding for `migrate_identity` — #434
- Adding the core package to `srs repo copy` snapshot export — separate follow-up
- `MemoryStore` tests that construct stores with explicit packages are updated where they
  directly assert `fields.is_empty()`, but no wholesale test rewrites

---

## Phases

### Phase 1: Core Package Module

**Goal:** `core_package::core_package()` returns a parsed `EmbeddedCorePackage`
containing 2 fields and 1 type from the embedded bundle; all unit tests pass.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Copy `/home/user/srs/packages/com.semanticops.core/1.0.0/core-bundle.srsj`
      to `crates/srs-repository/assets/core-bundle.srsj` (create `assets/` dir).
- [ ] Create `crates/srs-repository/src/core_package.rs`:
  - `const CORE_BUNDLE_JSON: &str = include_str!("../assets/core-bundle.srsj");`
  - `struct CoreBundle { package_id, package_name, package_version, fields: Vec<Field>, types: Vec<RecordType> }` with `#[serde(rename_all = "camelCase")]` and `#[serde(rename = "types")]` on `record_types`.
  - `pub struct EmbeddedCorePackage { pub package_id, pub package_name, pub package_version, pub fields: Vec<Field>, pub record_types: Vec<RecordType> }`
  - `static CORE_PACKAGE: OnceLock<EmbeddedCorePackage>`
  - `pub fn core_package() -> &'static EmbeddedCorePackage` — panics if bundle is malformed (compile-time inclusion guarantees it won't be).
- [ ] Add `pub(crate) mod core_package;` to `crates/srs-repository/src/lib.rs`.
- [ ] Draft `docs/adr/025-implicit-core-package-merge.md` using `ADR-TEMPLATE.md`.

#### Acceptance Criteria

- [ ] `core_package()` returns a struct with `fields.len() == 2` and `record_types.len() == 1`
- [ ] The type's namespace is `"com.semanticops.core"` and name is `"purpose"`
- [ ] The two fields are `"statement"` and `"title"` (in `com.semanticops.core`)
- [ ] `core_package()` is idempotent (calling twice returns same pointer address)

#### Testing

```bash
cargo test -p srs-repository core_package
```

Tests to write inside `core_package.rs`:
- `core_package_parses_successfully` — calls `core_package()`, checks len
- `core_package_has_expected_purpose_type` — `record_types[0].name == "purpose"`
- `core_package_has_expected_fields` — fields are statement + title
- `core_package_idempotent` — two calls return same pointer

#### Milestone gate

1. All tests above pass.
2. `cargo clippy -p srs-repository -- -D warnings` clean.
3. Commit: `feat(core-package): embed core-bundle and expose core_package() (#423)`.

---

### Phase 2: Store Integration

**Goal:** `FileStore::load_package` and `MemoryStore::load_package` both
include core fields/types. A repo that declares its own `com.semanticops.core`
field/type fails with `PackageRefConflict`.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add `merge_core_into_package` free function in `store.rs`:
  ```rust
  fn merge_core_into_package(
      fields: &mut Vec<Field>,
      record_types: &mut Vec<RecordType>,
  ) -> Result<(), RepositoryError>
  ```
  - Iterates `core_package().fields`; for each, if same `id` already in `fields`
    → `PackageRefConflict { path: "<core-package>", kind: "field", id, first_path: PathBuf::from("<repo>"), second_path: PathBuf::from("<core-package>") }`.
    Otherwise push.
  - Same pattern for `record_types` (keyed on `id` + `version`).
- [ ] Call `merge_core_into_package(&mut fields, &mut record_types)?;` at the
  **end** of `FileStore::load_package` — after all sub-packages are folded in,
  before assembling the `Package` return value.
- [ ] Call the same merge at the **end** of `MemoryStore::load_package` — after
  protocol augmentation, before returning `pkg`.
  _MemoryStore::load_package works on a cloned `Vec` from `self.package.borrow().clone()`,
  so mutate the clone's `fields` and `record_types` fields._
- [ ] Update the test `file_store_load_package_returns_package` in
  store.rs (search by name): remove `assert!(package.fields.is_empty())` — core fields are now
  always present. Replace with:
  ```rust
  assert!(package.fields.iter().any(|f| f.namespace == "com.semanticops.core"));
  ```
- [ ] Add new tests:
  - `load_package_includes_core_types` — FileStore on a minimal repo; verify
    `resolve_type_by_name("com.semanticops.core", "purpose")` is `Some`.
  - `load_package_memory_store_includes_core_types` — MemoryStore; same assert.
  - `load_package_repo_declaring_core_type_conflicts` — FileStore with a package
    dir that contains a field with id `"3b000001-0000-4000-a000-000000000001"`;
    `load_package()` returns `Err(PackageRefConflict { kind: "field", ... })`.

#### Acceptance Criteria

- [ ] `srs type list` on any repo shows `com.semanticops.core/purpose`
- [ ] `resolve_type_by_name("com.semanticops.core", "purpose")` returns `Some` on both FileStore and MemoryStore
- [ ] A repo with a conflicting core field/type returns `PackageRefConflict` error
- [ ] `cargo test -p srs-repository` passes (including existing tests)

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests (in `store.rs` `#[cfg(test)]` block):
- `load_package_includes_core_types`
- `load_package_memory_store_includes_core_types`
- `load_package_repo_declaring_core_type_conflicts`

#### Milestone gate

1. All tests pass.
2. Clippy clean.
3. Commit with message: `feat(core-package): merge core types into load_package (#423)`.

---

### Phase 3: Repo Map Surfacing + Drift Check

**Goal:** `srs repo map` output includes a `corePackage` section; an
integration test detects bundle drift from the canonical srs repo.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add to `analysis.rs`:
  ```rust
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct CorePackageSummary {
      pub id: String,
      pub name: String,
      pub version: String,
      /// Qualified names in "namespace/name" format, e.g. "com.semanticops.core/purpose".
      pub types: Vec<String>,
      /// Qualified names in "namespace/name" format, e.g. "com.semanticops.core/statement".
      pub fields: Vec<String>,
  }
  ```
- [ ] Add `pub core_package: CorePackageSummary` field to `RepoMap`.
- [ ] Populate in `build_repo_map_from_manifest`:
  ```rust
  let cp = crate::core_package::core_package();
  let core_package = CorePackageSummary {
      id: cp.package_id.clone(),
      name: cp.package_name.clone(),
      version: cp.package_version.clone(),
      types: cp.record_types.iter()
          .map(|rt| format!("{}/{}", rt.namespace, rt.name))
          .collect(),
      fields: cp.fields.iter()
          .map(|f| format!("{}/{}", f.namespace, f.name))
          .collect(),
  };
  ```
  Add `core_package` to the `RepoMap { ... }` constructor.
- [ ] Add drift-check integration test in
  `crates/srs-repository/tests/core_bundle_drift.rs`:
  ```rust
  #[test]
  fn core_bundle_matches_canonical_srs_repo() {
      let canonical = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
          .join("../../../srs/packages/com.semanticops.core/1.0.0/core-bundle.srsj");
      if !canonical.exists() {
          // srs/ repo not present in this checkout — skip gracefully.
          // In CI, the srs repo is present and this will catch drift.
          return;
      }
      let canonical_content = std::fs::read_to_string(&canonical).unwrap();
      let embedded = include_str!("../assets/core-bundle.srsj");
      assert_eq!(
          embedded.trim(), canonical_content.trim(),
          "Embedded core-bundle.srsj has drifted from the canonical srs repo. \
           Copy packages/com.semanticops.core/1.0.0/core-bundle.srsj to \
           crates/srs-repository/assets/core-bundle.srsj to refresh."
      );
  }
  ```
- [ ] Update constant-validation comment in `core_purpose.rs`: replace the
  `// Replace with core_package::resolve_type once #423 lands` comment with a
  note that #423 has landed and these constants will be removed in #434.
- [ ] Add a test in `core_purpose.rs` that calls `core_package()` and asserts the
  constants match the embedded values.

#### Acceptance Criteria

- [ ] `srs repo map` JSON output contains a `corePackage` key with correct id/name/version/types/fields
- [ ] Drift test passes (or skips gracefully when `srs/` repo absent)
- [ ] Constant-validation test passes

#### Testing

```bash
cargo test -p srs-repository
cargo test -p srs-repository --test core_bundle_drift  # skip if srs/ absent
cargo clippy -- -D warnings
```

Tests:
- `core_bundle_matches_canonical_srs_repo` (integration test, skips when srs/ absent)
- `core_purpose_constants_match_embedded_core_package` (in core_purpose.rs)

#### Milestone gate

1. All tests pass.
2. Clippy clean.
3. Commit with message: `feat(core-package): surface in repo map + drift check (#423)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload schema changes)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schema changes)
- [ ] `srs type list` on a fresh repo shows `com.semanticops.core/purpose`
- [ ] `srs repo map` JSON contains `corePackage` field
- [ ] A repo declaring a `com.semanticops.core/*` type returns a loud conflict error

## Coordination Rules

- Repository Service Worker does all implementation; Lead Integrator reviews.
- Workers keep to write scopes listed above.
- At each phase milestone: verify criteria, update checkboxes, commit.

## Assumptions

- The `core-bundle.srsj` at `srs/packages/com.semanticops.core/1.0.0/core-bundle.srsj`
  is the canonical source of truth for the embedded asset.
- `Field` and `RecordType` with `#[serde(rename_all = "camelCase")]` can
  deserialize directly from the bundle's inline JSON format (verified by
  inspecting both serde shapes).
- `MemoryStore::load_package` returns a clone from `self.package.borrow()`;
  merging into the clone after all protocol augmentation is correct.
- No WASM surface changes in this plan — that is #434.
