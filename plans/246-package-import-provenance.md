# Plan: Package Import Provenance, Divergence Detection, and Imports Listing

## Summary

Issue #246. `package_service::import_package_local` registers a boundary but records no
provenance. `install_package_bundle` stamps a minimal `upstreamPackage` raw-JSON blob (with the
wrong key `"id"` instead of `"packageId"`) and creates no per-definition `ImportRecord` entries.
This plan implements the full RFC-014 import-tracking machinery: typed `Manifest.upstream_package`,
per-definition `ImportRecord` creation during both `install` and `import`, divergence detection
(`clean` / `local-ahead`) via stored reference copies, a `package imports` CLI listing command, and
a WASM binding. Closes the `ext:import-tracking` coverage gap (dogfooding gap row) that landed with
core types in #245.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Core Model Worker | Phase 1 (Manifest typed field) |
| Repository Service Worker | Phases 1–4 |
| CLI Worker | Phase 5 |
| Bindings Worker | Phase 6 |
| Verification Agent | After Phase 4, before Phase 7 |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-009](../docs/adr/009-package-boundary-model.md) | Import records stored per-boundary via `save_instance_json`; path construction lives in the service | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `list_package_imports` is a service function; no logic in the CLI handler | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | New `PackageImportsPayload` struct in `payload.rs`; golden schema regenerated | accepted |
| [ADR-028](../docs/adr/028-extension-catalog-types-in-srs-core.md) | `ImportRecord`, `ImportSummary`, `UpstreamPackage` in `srs-core::extensions::import_tracking` | accepted |
| [ADR-030](../docs/adr/030-import-record-storage-model.md) | Import records in `<boundary>/.srs-import/import-records.json`; reference copies in `<boundary>/.srs-import/refs/<kind>/<file>.json` | proposed |

---

## Contracts

### CLI output contract (ADR-011)

New command added: `srs package imports`

- Add `PackageImportsPayload` to `crates/srs-cli/src/payload.rs` wrapping `ImportSummaryPayload`
  (mirrors `ImportSummary` fields; uses `schemars`-compatible types).
- Run `cargo run --bin generate-schemas` after adding the struct.
- Commit the generated `crates/srs-cli/schemas/payload/PackageImportsPayload.json`.
- `cargo test --test payload_contracts` must pass.

`package import` command gains `--mode` flag (optional, defaults to `upstream-tracked`). Return
shape `PackageImportPayload` is unchanged — no schema regeneration for this command.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/` entity schemas. `bash scripts/check-schema-sync.sh` already
passes; this plan does not touch it.

---

## Scope

- Promote `manifest.upstreamPackage` from raw `extra` to a typed `Option<UpstreamPackage>` field on `Manifest`.
- Fix `install_package_bundle` provenance stamp: use the typed `UpstreamPackage` struct (serializes `"packageId"`, not `"id"`).
- Update `init_new_repository` to use the typed `manifest.upstream_package` field.
- On `package install`: create one `ImportRecord` per installed definition; store in `<boundary>/.srs-import/import-records.json`; store reference copies in `<boundary>/.srs-import/refs/<kind>/<file>.json`.
- On `package import --mode <mode>`: create `ImportRecord` per definition in the boundary; store in `<boundary>/.srs-import/import-records.json`; no reference copies (no upstream to compare against for local imports).
- New service function `list_package_imports(store) -> Result<ImportSummary, RepositoryError>`: aggregates all boundaries' import-records.json; runs divergence detection for `upstream-tracked` records with a reference copy.
- Divergence detection: `clean` if current JSON == reference JSON; `local-ahead` if they differ; `update_available: None` (registry not available).
- New `PackageCommand::Imports {}` variant and `cmd_package_imports` handler.
- New `PackageImportsPayload` / `ImportSummaryPayload` / `ImportRecordPayload` payload structs.
- New WASM binding: `list_package_imports_json(repo_path: &str) -> Result<String, JsValue>`.
- Cross-store (memory → JSON → file) roundtrip tests.
- Update `docs/dogfooding.md` and `tests/fixtures/spec-repo/` extension record.

**Out of scope:**

- `upstream-ahead` divergence state (requires `ext:registry`, deferred).
- `update_available: true` (requires registry version lookup).
- `package upgrade` / upgrade engine (Gate B of epic #234).
- Automatic divergence polling or watches.
- WASM bindings for `package install` import-record creation (CLI only for now; WASM uses `list_package_imports_json`).
- Changes to `srs/docs/schema/2.0/` entity schemas.

---

## Phases

### Phase 1: Manifest typed field + naming fix

**Goal:** `Manifest.upstream_package` is a typed `Option<UpstreamPackage>` field; `install_package_bundle` and `init_new_repository` use it; old `"id"` key in boundary `package.json` is corrected to `"packageId"`.

**Agent:** Repository Service Worker (also touches `srs-repository::manifest`)

#### Tasks

- [ ] In `crates/srs-repository/src/manifest.rs`: add `use srs_core::extensions::import_tracking::UpstreamPackage;` and add field:
  ```rust
  #[serde(skip_serializing_if = "Option::is_none")]
  pub upstream_package: Option<UpstreamPackage>,
  ```
  with `#[serde(rename = "upstreamPackage")]`. Add it to `Default::default()` as `None`. Add a manifest roundtrip test: `manifest_upstream_package_roundtrips`.
- [ ] In `crates/srs-repository/src/repository_lifecycle.rs`: replace the raw `manifest.extra.get_mut("upstreamPackage")` / `meta.upstreamPackage` dual-path logic in `init_new_repository` with a typed read/write using `manifest.upstream_package`. Keep a backward-compat fallback: if `manifest.upstream_package` is `None`, check `manifest.extra["upstreamPackage"]` (the RFC-014 raw pre-migration path) then `manifest.extra["meta"]["upstreamPackage"]` (pre-RFC-014). On reading either fallback location, populate `manifest.upstream_package` and write through `save_manifest`.
- [ ] In `crates/srs-repository/src/package_install_service.rs` Phase 4 provenance stamp: replace the raw `serde_json::json!({...})` stamp with a typed `UpstreamPackage` struct:
  ```rust
  use srs_core::extensions::import_tracking::UpstreamPackage;
  let upstream = UpstreamPackage {
      package_id: bundle.id.clone(),
      namespace: bundle.namespace.clone(),
      name: bundle.name.clone(),
      version: bundle.version.clone(),
      installed_at: installed_at.clone(),
  };
  boundary_pkg_json["upstreamPackage"] = serde_json::to_value(&upstream)?;
  ```
  Update the test at line 825 from `pkg_json["upstreamPackage"]["id"]` to `pkg_json["upstreamPackage"]["packageId"]`.

#### Acceptance Criteria

- [ ] `Manifest` with `"upstreamPackage": {"packageId":"..."}` in JSON roundtrips correctly (field in struct, not in `extra`).
- [ ] `init_new_repository` stamps `installed_at` via typed field for both RFC-014 and pre-RFC-014 seeds.
- [ ] `install_package_bundle` test shows `pkg_json["upstreamPackage"]["packageId"]` (not `"id"`).
- [ ] No existing tests broken.

#### Testing

```bash
cargo test -p srs-repository manifest
cargo test -p srs-repository init_new_repository
cargo test -p srs-repository install_package
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `manifest_upstream_package_roundtrips` — verifies `upstream_package` serializes to `"upstreamPackage"` with `"packageId"` key and round-trips; `"upstreamPackage"` does not appear in `extra`.
- `memory_install_uses_default_boundary_and_counts` (existing, updated) — now checks `"packageId"` key.

#### Milestone gate

1. All acceptance criteria above met.
2. All listed tests pass.
3. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
4. Mark task checkboxes `[x]` and commit:
```bash
git commit -m "feat(repository): typed Manifest.upstream_package + fix packageId key (#246)"
```

---

### Phase 2: ImportRecord creation on `package install`

**Goal:** After `install_package_bundle` completes, one `ImportRecord` exists per installed definition stored in `<boundary>/.srs-import/import-records.json`, and a reference copy of each definition JSON is stored in `<boundary>/.srs-import/refs/<kind>/<file>.json`.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/package_install_service.rs`, after Phase 4 provenance stamp, add Phase 5:
  - Collect `ImportRecord` per installed definition from `decisions`/`bundle.definitions`.
  - `DefinitionType` mapping from `DefinitionKind`: `Field→Field`, `Type→Type`, `RelationType→RelationType`, `View→View`, `Blueprint→Blueprint`, `Protocol→Protocol`.
  - `imported_at`: reuse the `installed_at` timestamp.
  - `source_package_id`: `bundle.id.clone()`, `source_package_name`: `bundle.namespace` + "/" + `bundle.name` (format `"com.foo/bar"`) or just `bundle.name` — use `bundle.namespace` as `source_package_name` field? Actually: `source_package_id = bundle.id`, `source_package_name = bundle.name`, `source_package_version = bundle.version`.
  - `mode`: `ImportMode::UpstreamTracked`.
  - `conflict_state`: `Some(ConflictState::Clean)` — clean immediately after install.
  - Optional fields all `None` at install time.
  - Group records into an `ImportSummary` (generated_at = installed_at) and serialize to `{boundary_path}/.srs-import/import-records.json` via `store.save_instance_json`.
- [ ] Also store reference copies: for each `Decision::Install` entry, call `store.save_instance_json(&format!("{boundary_path}/.srs-import/refs/{}", def.rel_path), &def.value)`.
- [ ] For skipped (`SkipIdentical`) and conflict definitions, do NOT create ImportRecords (only track what was actually installed).

#### Acceptance Criteria

- [ ] After `install_package_bundle`, `store.load_instance_json("packages/<name>/.srs-import/import-records.json")` succeeds and parses as `ImportSummary`.
- [ ] `ImportSummary.fields` has one entry per installed field, with `mode = "upstream-tracked"`, `conflictState = "clean"`.
- [ ] Reference copy exists at `packages/<name>/.srs-import/refs/fields/alpha.json` for each installed field.
- [ ] `ImportRecord.sourcePackageId` matches `bundle.id`.

#### Testing

```bash
cargo test -p srs-repository install_package
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `install_creates_import_records` — installs a bundle, checks `import-records.json` exists, parses, has correct count and mode.
- `install_creates_reference_copies` — checks `.srs-import/refs/fields/alpha.json` matches the installed value.

#### Milestone gate

1. All acceptance criteria met.
2. Run:
```bash
cargo test -p srs-repository install_package
cargo clippy -p srs-repository -- -D warnings
```
3. Commit:
```bash
git commit -m "feat(repository): create ImportRecords + ref copies on package install (#246)"
```

---

### Phase 3: `package import --mode` with ImportRecord creation

**Goal:** `ImportPackageLocalInput` accepts `mode: ImportMode`; `import_package_local` creates `ImportRecord` entries for all definitions found in the boundary directory.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `package_service.rs`, update `ImportPackageLocalInput`:
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize, Default)]
  #[serde(rename_all = "camelCase")]
  pub struct ImportPackageLocalInput {
      pub source_path: String,
      #[serde(default = "default_import_mode")]
      pub mode: ImportMode,
  }
  fn default_import_mode() -> ImportMode { ImportMode::UpstreamTracked }
  ```
- [ ] After `store.save_package_boundary_metadata`, load the boundary to get its definition lists. Use `store.load_package_boundary(&boundary.selector)?` to get field_paths, type_paths, etc.
- [ ] For each definition path in the boundary, create an `ImportRecord` with:
  - `definition_id`: load the JSON file and read `["id"]` as string (or use a sentinel if absent).
  - `definition_type`: derive from path prefix (`fields/` → Field, etc.).
  - `namespace`, `name`, `version`: from the JSON file (read `["namespace"]`, `["name"]`, `["version"]`).
  - `mode`: `input.mode.clone()`.
  - `imported_at`: `chrono::Utc::now().to_rfc3339()`.
  - `source_package_id`: `boundary.id.clone()`, `source_package_name`: `boundary.name.clone()`, `source_package_version`: `boundary.version.clone()`.
  - `conflict_state`: `None` (no reference copies for local imports).
- [ ] Serialize to `{source_path}/.srs-import/import-records.json` via `store.save_instance_json`.
- [ ] In `crates/srs-cli/src/commands/mod.rs`, update `PackageCommand::Import`:
  ```rust
  Import {
      #[arg(long = "path")]
      path: String,
      #[arg(long, default_value = "upstream-tracked")]
      mode: String,  // parsed to ImportMode in handler
  }
  ```
- [ ] In `crates/srs-cli/src/commands/package.rs`, update `cmd_package_import` to parse `mode` string → `ImportMode` (return error on invalid value) and pass to `ImportPackageLocalInput`.

#### Acceptance Criteria

- [ ] `package import --path packages/gov` creates `packages/gov/.srs-import/import-records.json`.
- [ ] `package import --path packages/gov --mode local-fork` creates records with `mode = "local-fork"`.
- [ ] Invalid `--mode` value returns a clear error.
- [ ] No reference copies are created for `package import` (no `.srs-import/refs/` directory).
- [ ] `PackageImportPayload` shape is unchanged; no new golden schema needed.

#### Testing

```bash
cargo test -p srs-repository import_package
cargo test -p srs-cli package_import
cargo clippy -- -D warnings
```

Specific tests to write or verify:

- `import_package_local_creates_import_records` — imports a boundary, checks `import-records.json`.
- `import_package_local_respects_mode` — verifies mode is stored correctly.

#### Milestone gate

1. All acceptance criteria met.
2. Run:
```bash
cargo test -p srs-repository
cargo test -p srs-cli
cargo clippy -- -D warnings
```
3. Commit:
```bash
git commit -m "feat(repository,cli): package import --mode + ImportRecord creation (#246)"
```

---

### Phase 4: `list_package_imports` service + divergence detection

**Goal:** `list_package_imports(store) -> Result<ImportSummary, RepositoryError>` aggregates all boundaries' import records and runs divergence detection for upstream-tracked definitions.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add to `crates/srs-repository/src/package_service.rs`:
  ```rust
  pub fn list_package_imports(
      store: &dyn RepositoryStore,
  ) -> Result<ImportSummary, RepositoryError> { ... }
  ```
- [ ] Walk `store.list_package_boundaries()?`. For each boundary:
  - Try `store.load_instance_json(&format!("{boundary_path}/.srs-import/import-records.json"))`.
  - If missing (boundary predates #246): skip (no records).
  - Deserialize as `ImportSummary` and collect all records (fields, types, etc.).
- [ ] For each collected `ImportRecord` with `mode == ImportMode::UpstreamTracked`:
  - Find the definition's file path: iterate the boundary's `field_paths`, `type_paths`, etc. to match by `definition_id`. If found, use that path. If not found, skip divergence (definition may have been removed).
  - Load current definition: `store.load_instance_json(&format!("{boundary_path}/{def_path}"))`.
  - Load reference copy: `store.load_instance_json(&format!("{boundary_path}/.srs-import/refs/{def_path}"))`. If missing: skip (reference not available — no divergence determination).
  - Compare with `current_json == reference_json` (serde_json Value equality). If equal: `ConflictState::Clean`. If not: `ConflictState::LocalAhead`.
  - Update the `ImportRecord.conflict_state` field.
- [ ] Merge all records into one `ImportSummary` with `generated_at = chrono::Utc::now().to_rfc3339()`.
- [ ] Helper struct `ListPackageImportsFilter {}` (empty for now, for ADR-010 filter-struct convention).

#### Acceptance Criteria

- [ ] `list_package_imports` with no boundaries returns an empty `ImportSummary`.
- [ ] After install, `list_package_imports` returns records with `conflictState = "clean"`.
- [ ] After modifying an installed field JSON, `list_package_imports` returns `conflictState = "local-ahead"` for that definition.
- [ ] Boundaries without `.srs-import/import-records.json` are silently skipped (backward compat).

#### Testing

```bash
cargo test -p srs-repository list_package_imports
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `list_package_imports_empty` — no boundaries → empty summary.
- `list_package_imports_clean_after_install` — install bundle, list → all clean.
- `list_package_imports_local_ahead_after_edit` — install bundle, modify one field JSON via `save_instance_json`, list → that field's record is `local-ahead`.
- `list_package_imports_skips_boundary_without_records` — boundary with no import-records.json is skipped.

#### Milestone gate

1. All acceptance criteria met.
2. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
3. Commit:
```bash
git commit -m "feat(repository): list_package_imports + divergence detection (#246)"
```

---

### Phase 5: CLI command + payload structs

**Goal:** `srs package imports` returns a valid `PackageImportsPayload` JSON envelope; golden schema committed.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-cli/src/payload.rs`, add:
  ```rust
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct ImportRecordPayload {
      pub definition_id: String,
      pub definition_type: String,
      pub namespace: String,
      pub name: String,
      pub version: u32,
      pub mode: String,
      pub imported_at: String,
      pub source_package_id: String,
      pub source_package_name: String,
      pub source_package_version: String,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub latest_known_upstream_version: Option<u32>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub update_available: Option<bool>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub update_checked_at: Option<String>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub conflict_state: Option<String>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub conflict_detected_at: Option<String>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub local_version: Option<u32>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub local_edited_at: Option<String>,
  }

  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct ImportSummaryPayload {
      pub generated_at: String,
      pub fields: Vec<ImportRecordPayload>,
      pub types: Vec<ImportRecordPayload>,
      pub views: Vec<ImportRecordPayload>,
      pub blueprints: Vec<ImportRecordPayload>,
      pub protocols: Vec<ImportRecordPayload>,
      pub relation_types: Vec<ImportRecordPayload>,
  }

  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct PackageImportsPayload {
      pub summary: ImportSummaryPayload,
  }
  ```
  Use flat `String` for `definition_type`, `mode`, `conflict_state` to avoid `schemars` issues with imported enums.

- [ ] In `crates/srs-cli/src/commands/mod.rs`, add to `PackageCommand`:
  ```rust
  /// List all import records across all package boundaries
  Imports,
  ```

- [ ] In `crates/srs-cli/src/commands/package.rs`:
  - Add `PackageCommand::Imports => cmd_package_imports(ctx)` to `dispatch`.
  - Add `use crate::payload::{..., PackageImportsPayload, ImportSummaryPayload, ImportRecordPayload};`.
  - Implement:
    ```rust
    fn cmd_package_imports(ctx: CliContext) -> Result<String> {
        let summary = with_store(&ctx, |store| Ok(list_package_imports(store)?))?;
        // map ImportSummary -> PackageImportsPayload
        output::serialize("package imports", PackageImportsPayload { summary: ... })
    }
    ```
  - Mapping function from `ImportRecord` → `ImportRecordPayload` (inline or as a `From` impl):
    - `definition_type`: `serde_json::to_string(&r.definition_type)?.trim_matches('"').to_string()`
    - `mode`: same pattern
    - `conflict_state`: `r.conflict_state.as_ref().map(|s| serde_json::to_string(s).unwrap().trim_matches('"').to_string())`

- [ ] Add `use srs_repository::package_service::list_package_imports;` import.

- [ ] Run `cargo run --bin generate-schemas` and commit generated files.

#### Acceptance Criteria

- [ ] `srs package imports --repo /tmp/test` returns valid JSON with `"ok": true` and `"payload"` containing `"summary"`.
- [ ] `cargo test --test payload_contracts` passes.
- [ ] `crates/srs-cli/schemas/payload/PackageImportsPayload.json` exists and is committed.

#### Testing

```bash
cargo build --bin srs
cargo run --bin srs -- package imports --repo /tmp/test-repo 2>&1
cargo test --test payload_contracts
cargo clippy -- -D warnings
```

Specific tests to write or verify:

- `payload_contracts` (existing golden test) — must pass after `generate-schemas` run.

#### Milestone gate

1. All acceptance criteria met.
2. Run:
```bash
cargo test --test payload_contracts
cargo clippy -- -D warnings
```
3. Commit:
```bash
git commit -m "feat(cli): package imports command + PackageImportsPayload (#246)"
```

---

### Phase 6: WASM binding

**Goal:** `list_package_imports_json(repo_path)` WASM binding returns serialized `ImportSummary` JSON.

**Agent:** Bindings Worker

#### Tasks

- [ ] In `crates/srs-bindings/src/lib.rs`, add:
  ```rust
  /// List all import records across all package boundaries in the repository.
  /// Returns the `ImportSummary` as a JSON string.
  pub fn list_package_imports_json(&self) -> Result<String, JsValue> {
      let summary = package_service::list_package_imports(&self.store)
          .map_err(js_err)?;
      serde_json::to_string(&summary).map_err(|e| js_err(e.into()))
  }
  ```
- [ ] Add `use srs_repository::package_service::list_package_imports;` or expand the existing `package_service::` import.
- [ ] Add a smoke test (no WASM target needed — use existing bindings test pattern if available):
  ```rust
  // tests/bindings_smoke.rs or inline in lib.rs under #[cfg(test)]
  fn list_package_imports_returns_parseable_json() {
      // Create a MemoryStore, install a package, call list_package_imports_json
  }
  ```

#### Acceptance Criteria

- [ ] `list_package_imports_json` compiles cleanly in the bindings crate.
- [ ] Smoke test: after installing a package, `list_package_imports_json` returns valid JSON that deserializes as `ImportSummary`.

#### Testing

```bash
cargo build -p srs-bindings
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

Specific tests to write or verify:

- `list_package_imports_returns_parseable_json` — parseable JSON with `generatedAt` and `fields` keys.

#### Milestone gate

1. All acceptance criteria met.
2. Run:
```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```
3. Commit:
```bash
git commit -m "feat(bindings): list_package_imports_json WASM binding (#246)"
```

---

### Phase 7: Cross-store roundtrip tests + cleanup

**Goal:** End-to-end tests verify the full flow works with both MemoryStore and FileStore; divergence detection confirmed; test fixture updated.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Write `tests/integration_import_tracking.rs` (or add to existing integration test file) with at least one test that: creates a repo with `FileStore`, installs a package, lists imports (all clean), modifies a definition file, lists imports again (one local-ahead).
- [ ] Update `tests/fixtures/spec-repo/records/extensions/ext-import-tracking.json` to match the canonical spec: add the "Repository-Level Provenance (RFC-014)" section (copy from `../srs/srs/records/extensions/ext-import-tracking.json` if available, or write the missing section based on the canonical content).
- [ ] Confirm `srs repo validate --repo tests/fixtures/spec-repo` still passes (if srs CLI available) or verify the JSON is well-formed.

#### Acceptance Criteria

- [ ] FileStore roundtrip: install → modify → detect divergence.
- [ ] MemoryStore: install → list → all clean (no divergence detection without actual files — records are still created).
- [ ] `tests/fixtures/spec-repo/records/extensions/ext-import-tracking.json` updated.

#### Testing

```bash
cargo test import_tracking
cargo test -p srs-repository
cargo clippy -- -D warnings
```

#### Milestone gate

1. All acceptance criteria met.
2. Run:
```bash
cargo test
cargo clippy -- -D warnings
```
3. Commit:
```bash
git commit -m "test: cross-store import tracking roundtrip tests (#246)"
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no schema changes)
- [ ] `srs package imports` returns valid JSON on a test repository
- [ ] `srs package import --mode local-fork --path <path>` works
- [ ] After `package install`, import records exist at `<boundary>/.srs-import/import-records.json`
- [ ] After editing an installed definition, `srs package imports` reports `conflictState: "local-ahead"` for it
- [ ] `PackageImportsPayload.json` golden schema committed
- [ ] ADR-030 committed at `docs/adr/030-import-record-storage-model.md`

## Coordination Rules

- Agents keep to their write scopes.
- Repository Service Worker does not touch `payload.rs` or CLI command handlers.
- CLI Worker does not implement service logic — one `list_package_imports` call per handler.
- Lead Integrator resolves any API naming disagreements between phases.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests pass, update plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- `chrono::Utc::now().to_rfc3339()` is available in `srs-repository` (already used in existing code).
- `MemoryStore` does not need real file paths to store instance JSON — it uses the key string as the address.
- The `srs-bindings` crate compiles for the host target in tests (WASM target build is CI-only).
- The spec-repo test fixture can be updated in this PR; it is vendored data, not generated.
- For `package import`, definition IDs are read from the JSON files in the boundary; if a definition file is malformed or missing an `id` field, that definition is skipped with a warning (non-fatal).
