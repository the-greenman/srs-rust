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
| Core Model Worker | Phase 0 (Display impl + skipped field) |
| Repository Service Worker | Phases 1–4 |
| CLI Worker | Phase 5 |
| Bindings Worker | Phase 6 |
| Verification Agent | After Phase 4, before Phase 7 |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-009](../docs/adr/009-package-boundary-model.md) | Import records stored per-boundary via `save_instance_json`; path construction lives in the service, governed by ADR-030 | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `list_package_imports` takes `ListPackageImportsFilter`; no logic in CLI handler; mapping via `From` impl | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | New `PackageImportsPayload` struct in `payload.rs`; golden schema regenerated | accepted |
| [ADR-024](../docs/adr/024-best-effort-rollback-multi-write-services.md) | Import-record writes are best-effort post-install; no rollback on partial failure | accepted |
| [ADR-028](../docs/adr/028-extension-catalog-types-in-srs-core.md) | `ImportRecord`, `ImportSummary`, `UpstreamPackage` in `srs-core::extensions::import_tracking` | accepted |
| [ADR-030](../docs/adr/030-import-record-storage-model.md) | Import records in `<boundary>/.srs-import/import-records.json`; reference copies in `<boundary>/.srs-import/refs/<kind>/<file>.json` | **accepted** |

---

## Contracts

### CLI output contract (ADR-011)

New command added: `srs package imports`

- Add `ImportRecordPayload`, `ImportSummaryPayload`, `PackageImportsPayload` to `crates/srs-cli/src/payload.rs`.
- Implement `From<ImportRecord> for ImportRecordPayload` and `From<ImportSummary> for ImportSummaryPayload` in `payload.rs`.
- Run `cargo run --bin generate-schemas` after adding the structs.
- Commit the generated `crates/srs-cli/schemas/payload/PackageImportsPayload.json`.
- `cargo test --test payload_contracts` must pass.

`package import` command gains `--mode` flag (optional, defaults to `upstream-tracked`). Return
shape `PackageImportPayload` is unchanged — no schema regeneration for this command.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/` entity schemas. `bash scripts/check-schema-sync.sh` already
passes; this plan does not touch it.

---

## Scope

- Add `Display` for `DefinitionType`, `ImportMode`, `ConflictState` in `srs-core::extensions::import_tracking`.
- Add `skipped_definitions: Vec<String>` to `ImportSummary` in `srs-core` for non-fatal load-path skips.
- Promote `manifest.upstreamPackage` from raw `extra` to a typed `Option<UpstreamPackage>` field on `Manifest`; migrate in `load_manifest` for backward compat.
- Fix `install_package_bundle` provenance stamp: use the typed `UpstreamPackage` struct (serializes `"packageId"`, not `"id"`).
- Update `init_new_repository` to use the typed `manifest.upstream_package` field.
- On `package install`: create one `ImportRecord` per installed definition (for `DefinitionKind` variants that map to `DefinitionType`); store in `<boundary>/.srs-import/import-records.json`; store reference copies in `<boundary>/.srs-import/refs/<rel_path>`.
  - Mapping: `DefinitionKind::Field→DefinitionType::Field`, `Type→Type`, `View→View`, `Blueprint→Blueprint`, `Protocol→Protocol`, `RelationType→RelationType`. Skip DocumentView, Lifecycle, Vocabulary, Theme.
- On `package import --mode <mode>`: create `ImportRecord` per definition found in `PackageBoundary.field_paths / type_paths / blueprint_paths / protocol_paths` (Views and RelationTypes are not tracked in PackageBoundary — excluded); no reference copies.
- New `list_package_imports(store, ListPackageImportsFilter{}) -> ImportSummary`: aggregates all boundaries' import-records.json; runs live divergence detection for `upstream-tracked` records.
- Divergence: compare `current_json == reference_json` (serde_json Value equality); `Clean` if equal, `LocalAhead` if different. `update_available: None` (registry not available).
- New `PackageCommand::Imports {}` variant and `cmd_package_imports` handler (one service call via `From` impl).
- New `PackageImportsPayload` / `ImportSummaryPayload` / `ImportRecordPayload` payload structs with `From` impls.
- New WASM binding: `pub fn list_package_imports_json(&self) -> Result<String, JsValue>`.
- Cross-store (memory + file) roundtrip tests.
- Update `docs/dogfooding.md`; update test fixture if canonical spec is available.

**Out of scope:**

- `upstream-ahead` divergence state (requires `ext:registry`, deferred).
- `update_available: true` (requires registry version lookup).
- `package upgrade` / upgrade engine (Gate B of epic #234).
- Automatic divergence polling or watches.
- Views and RelationTypes in `package import` (not tracked in `PackageBoundary`).
- Changes to `srs/docs/schema/2.0/` entity schemas.

---

## Phases

### Phase 0: srs-core additions

**Goal:** `srs-core::extensions::import_tracking` exposes `Display` for the three enum types; `ImportSummary` has a `skipped_definitions` field for non-fatal skip messages.

**Agent:** Core Model Worker

#### Tasks

- [ ] In `crates/srs-core/src/extensions/import_tracking.rs`, add `Display` implementations for `DefinitionType`, `ImportMode`, `ConflictState` that return the same kebab-case string as their serde rename:
  ```rust
  impl std::fmt::Display for DefinitionType {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
          let s = match self {
              DefinitionType::Field => "field",
              DefinitionType::Type => "type",
              DefinitionType::View => "view",
              DefinitionType::Blueprint => "blueprint",
              DefinitionType::Protocol => "protocol",
              DefinitionType::RelationType => "relation-type",
          };
          write!(f, "{s}")
      }
  }
  // Same pattern for ImportMode and ConflictState.
  ```
- [ ] Add `skipped_definitions: Vec<String>` to `ImportSummary`:
  ```rust
  #[serde(skip_serializing_if = "Vec::is_empty", default)]
  pub skipped_definitions: Vec<String>,
  ```
  Update `Default`/construction sites, update the roundtrip test to cover the field.

#### Acceptance Criteria

- [ ] `DefinitionType::RelationType.to_string() == "relation-type"`.
- [ ] `ImportMode::UpstreamTracked.to_string() == "upstream-tracked"`.
- [ ] `ConflictState::LocalAhead.to_string() == "local-ahead"`.
- [ ] `ImportSummary` with no skipped definitions serializes without the `skippedDefinitions` key.
- [ ] Existing tests pass.

#### Testing

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

Specific tests to write or verify:

- `display_impls_match_serde_names` — tests all three enums' `Display` output against their serde string equivalents.

#### Milestone gate

1. All acceptance criteria met.
2. Run:
```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```
3. Commit:
```bash
git commit -m "feat(core): Display for DefinitionType/ImportMode/ConflictState; skipped_definitions on ImportSummary (#246)"
```

---

### Phase 1: Manifest typed field + naming fix

**Goal:** `Manifest.upstream_package` is a typed `Option<UpstreamPackage>` field; backward-compat migration lives in `load_manifest`; `install_package_bundle` and `init_new_repository` use the typed field; `"id"` key is corrected to `"packageId"`.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/manifest.rs`: add `use srs_core::extensions::import_tracking::UpstreamPackage;` and:
  ```rust
  #[serde(rename = "upstreamPackage", skip_serializing_if = "Option::is_none")]
  pub upstream_package: Option<UpstreamPackage>,
  ```
  Add to `Default::default()` as `None`. Add test `manifest_upstream_package_roundtrips`: verifies `upstream_package` serializes to `"upstreamPackage"` with `"packageId"` key and does NOT appear in `extra`.
- [ ] In `load_manifest` (same file), after deserializing, add a backward-compat migration: if `manifest.upstream_package.is_none()`, check `manifest.extra["upstreamPackage"]` (RFC-014 raw path). If found, attempt `serde_json::from_value::<UpstreamPackage>` — if successful, set `manifest.upstream_package`; if the raw JSON uses `"id"` instead of `"packageId"`, map it manually. Also check `manifest.extra["meta"]["upstreamPackage"]` (pre-RFC-014 path) as fallback. This migration applies to any repo loaded from disk, regardless of entry point.
- [ ] In `crates/srs-repository/src/repository_lifecycle.rs`: replace the raw `manifest.extra.get_mut("upstreamPackage")` / `meta.upstreamPackage` dual-path in `init_new_repository` with: load manifest (migration now runs automatically in `load_manifest`), set `manifest.upstream_package.as_mut().map(|up| up.installed_at = Utc::now().to_rfc3339())`, then `store.save_manifest`.
- [ ] In `crates/srs-repository/src/package_install_service.rs`, Phase 4 provenance stamp: replace raw JSON with typed `UpstreamPackage`:
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
  Update the test at line 825 from `["id"]` to `["packageId"]`.

#### Acceptance Criteria

- [ ] `Manifest` with `"upstreamPackage": {"packageId":"..."}` roundtrips (field in struct, not in `extra`).
- [ ] Old manifest with `"upstreamPackage": {"id":"..."}` migrates to typed field on `load_manifest` without data loss.
- [ ] `init_new_repository` stamps `installed_at` via typed field.
- [ ] `install_package_bundle` test shows `pkg_json["upstreamPackage"]["packageId"]`.
- [ ] No existing tests broken.

#### Testing

```bash
cargo test -p srs-repository manifest
cargo test -p srs-repository init_new_repository
cargo test -p srs-repository install_package
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `manifest_upstream_package_roundtrips` — typed field round-trips.
- `manifest_upstream_package_migrates_legacy_id_key` — JSON with `"id"` key is migrated to `upstream_package` with correct `package_id`.
- `memory_install_uses_default_boundary_and_counts` (existing, updated) — checks `"packageId"`.

#### Milestone gate

1. All acceptance criteria met.
2. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
3. Commit:
```bash
git commit -m "feat(repository): typed Manifest.upstream_package + fix packageId key (#246)"
```

---

### Phase 2: ImportRecord creation on `package install`

**Goal:** After `install_package_bundle` completes, `<boundary>/.srs-import/import-records.json` exists with one `ImportRecord` per installed definition, and reference copies exist at `<boundary>/.srs-import/refs/<kind>/<file>.json`.

**Agent:** Repository Service Worker

#### Path convention

`boundary_path` is the boundary's selector string, e.g. `"packages/gov"`. Calls to `store.save_instance_json` take repo-root-relative path strings:
- Import records: `"packages/gov/.srs-import/import-records.json"`
- Reference copy for `fields/title.json`: `"packages/gov/.srs-import/refs/fields/title.json"`

#### DefinitionKind → DefinitionType mapping

Only these kinds have a corresponding `DefinitionType`; all others are skipped:
- `DefinitionKind::Field` → `DefinitionType::Field`
- `DefinitionKind::Type` → `DefinitionType::Type`
- `DefinitionKind::View` → `DefinitionType::View`
- `DefinitionKind::Blueprint` → `DefinitionType::Blueprint`
- `DefinitionKind::Protocol` → `DefinitionType::Protocol`
- `DefinitionKind::RelationType` → `DefinitionType::RelationType`
- Skip: `DocumentView`, `Lifecycle`, `Vocabulary`, `Theme`

#### Tasks

- [ ] In `package_install_service.rs`, add Phase 5 after the provenance stamp:
  - Import: `use srs_core::extensions::import_tracking::{ImportMode, ImportRecord, ImportSummary, DefinitionType, ConflictState};`
  - For each entry in `bundle.definitions.iter().zip(&decisions)` where decision is `Decision::Install`:
    - Map `def.kind` to `Option<DefinitionType>` using the table above; skip if `None`.
    - Read `definition_id` from `def.value["id"].as_str().unwrap_or("").to_string()`.
    - Read `namespace` from `def.value["namespace"].as_str().unwrap_or("").to_string()`.
    - Read `name` from `def.value["name"].as_str().unwrap_or("").to_string()`.
    - Read `version` as `def.value["version"].as_u64().map(|v| v as u32).unwrap_or(0)`.
    - Build `ImportRecord { definition_id, definition_type, namespace, name, version, mode: ImportMode::UpstreamTracked, imported_at: installed_at.clone(), source_package_id: bundle.id.clone(), source_package_name: bundle.namespace.clone(), source_package_version: bundle.version.clone(), conflict_state: Some(ConflictState::Clean), ..all optional fields None }`.
    - Store reference copy: `store.save_instance_json(&format!("{boundary_path}/.srs-import/refs/{}", def.rel_path), &def.value)?`.
  - Group records into `ImportSummary` by `definition_type`.
  - Serialize summary: `store.save_instance_json(&format!("{boundary_path}/.srs-import/import-records.json"), &serde_json::to_value(&summary)?)`.
- [ ] **Error handling (ADR-024):** if `save_instance_json` for any import-record or reference-copy write fails, log nothing (services don't write to stderr per ADR-001), leave already-written files in place, and do NOT propagate the error — add the path to `summary.skipped_definitions` instead. The install result is returned successfully; `list_package_imports` will silently skip the boundary if import-records.json is missing.

#### Acceptance Criteria

- [ ] After install, `store.load_instance_json("packages/<name>/.srs-import/import-records.json")` succeeds and parses as `ImportSummary`.
- [ ] `ImportSummary.fields` has one entry per installed field with `mode = "upstream-tracked"`, `conflictState = "clean"`, `sourcePackageId = bundle.id`, `sourcePackageName = bundle.namespace`.
- [ ] Reference copy at `packages/<name>/.srs-import/refs/fields/alpha.json` matches the installed definition JSON.
- [ ] `DocumentView`/`Lifecycle`/`Vocabulary`/`Theme` definitions produce no ImportRecords.
- [ ] If an import-record write fails, the install still returns success (non-fatal).

#### Testing

```bash
cargo test -p srs-repository install_package
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `install_creates_import_records` — installs a bundle, parses import-records.json, checks count and mode.
- `install_creates_reference_copies` — checks `.srs-import/refs/fields/alpha.json` equals original value.
- `install_sets_source_package_name_to_namespace` — `source_package_name == bundle.namespace`.

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

**Goal:** `ImportPackageLocalInput` accepts `mode: ImportMode`; `import_package_local` creates `ImportRecord` entries for all definitions found in `PackageBoundary.field_paths`, `type_paths`, `blueprint_paths`, `protocol_paths`.

**Agent:** Repository Service Worker

#### Path convention

Same as Phase 2. After `store.save_package_boundary_metadata(&boundary)`, the boundary's selector is `Some(source_path.trim().to_string())`. Call `store.load_package_boundary(&boundary.selector)?` to retrieve `field_paths`, `type_paths`, etc.

#### Tasks

- [ ] Update `ImportPackageLocalInput`:
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
- [ ] After `store.save_package_boundary_metadata` and `register_package_boundary`, load the boundary: `let loaded = store.load_package_boundary(&boundary.selector)?;`. Build ImportRecords from:
  - `loaded.field_paths` → `DefinitionType::Field`
  - `loaded.type_paths` → `DefinitionType::Type`
  - `loaded.blueprint_paths` → `DefinitionType::Blueprint`
  - `loaded.protocol_paths` → `DefinitionType::Protocol`
  - For each path, load the JSON: `store.load_instance_json(&format!("{}/{path}", source_path.trim()))?`.
  - Read `definition_id` = `json["id"].as_str().unwrap_or("").to_string()`, etc. (use `as_u64().map(|v| v as u32).unwrap_or(0)` for version).
  - If `definition_id.is_empty()`, add `format!("skipped {path}: missing id")` to `skipped_definitions` and continue.
  - `mode = input.mode.clone()`, `imported_at = chrono::Utc::now().to_rfc3339()`.
  - `source_package_id = boundary.id.clone()`, `source_package_name = boundary.namespace.clone()`, `source_package_version = boundary.version.clone()`.
  - `conflict_state = None` (no reference copies for local imports).
- [ ] Serialize `ImportSummary` to `{source_path}/.srs-import/import-records.json` via `save_instance_json`. Non-fatal on failure (same ADR-024 strategy as Phase 2).
- [ ] In `crates/srs-cli/src/commands/mod.rs`, update `PackageCommand::Import`:
  ```rust
  Import {
      #[arg(long = "path")]
      path: String,
      /// Import mode: upstream-tracked (default), local-copy, or local-fork
      #[arg(long, default_value = "upstream-tracked")]
      mode: String,
  }
  ```
- [ ] In `cmd_package_import`, parse `mode` string → `ImportMode`:
  ```rust
  let mode = match mode.as_str() {
      "upstream-tracked" => ImportMode::UpstreamTracked,
      "local-copy" => ImportMode::LocalCopy,
      "local-fork" => ImportMode::LocalFork,
      other => return Err(anyhow::anyhow!("invalid --mode: {other}")),
  };
  ```
  Pass `mode` in `ImportPackageLocalInput`. Add `use srs_core::extensions::import_tracking::ImportMode;`.

#### Acceptance Criteria

- [ ] `package import --path packages/gov` creates `packages/gov/.srs-import/import-records.json`.
- [ ] `package import --path packages/gov --mode local-fork` creates records with `mode = "local-fork"`.
- [ ] Invalid `--mode` returns a clear error string.
- [ ] No `.srs-import/refs/` created for `package import`.
- [ ] Definitions with missing `id` are skipped (listed in `skippedDefinitions`), not fatal.
- [ ] `PackageImportPayload` shape unchanged.

#### Testing

```bash
cargo test -p srs-repository import_package
cargo test -p srs-cli package_import
cargo clippy -- -D warnings
```

Specific tests to write or verify:

- `import_package_local_creates_import_records` — creates records with correct mode.
- `import_package_local_respects_mode` — `local-fork` stored correctly.

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

**Goal:** `list_package_imports(store, ListPackageImportsFilter{})` aggregates all boundaries' import records and runs live divergence detection.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add to `package_service.rs`:
  ```rust
  #[derive(Debug, Clone, Default)]
  pub struct ListPackageImportsFilter {}

  pub fn list_package_imports(
      store: &dyn RepositoryStore,
      _filter: ListPackageImportsFilter,
  ) -> Result<ImportSummary, RepositoryError> { ... }
  ```
- [ ] Walk `store.list_package_boundaries()?`. For each boundary, derive `boundary_path` from `boundary.selector`:
  - `None` → use `store.primary_boundary_path()` or equivalent (check existing service code for how primary boundary path is resolved).
  - `Some(p)` → use `p`.
- [ ] Try `store.load_instance_json(&format!("{boundary_path}/.srs-import/import-records.json"))`. If error (file missing) → skip this boundary (add nothing to results).
- [ ] Deserialize as `ImportSummary`. Collect all records from all fields, types, views, blueprints, protocols, relation_types into a flat list for divergence processing.
- [ ] For each `ImportRecord` with `mode == ImportMode::UpstreamTracked`:
  - Find the definition path: iterate `boundary.field_paths / type_paths / blueprint_paths / protocol_paths` to find the entry whose JSON `id` matches `record.definition_id`. Use `store.load_instance_json` to read, compare `["id"]`. If not found → skip divergence for this record (keep stored `conflict_state`, add path to `skipped_definitions`).
  - Load reference: `store.load_instance_json(&format!("{boundary_path}/.srs-import/refs/{def_path}"))`. If missing → skip divergence.
  - Compare: `if current == reference` → `record.conflict_state = Some(ConflictState::Clean)` else `Some(ConflictState::LocalAhead)`.
- [ ] Rebuild `ImportSummary` from the updated records, with `generated_at = chrono::Utc::now().to_rfc3339()`.

#### Acceptance Criteria

- [ ] No boundaries → empty `ImportSummary`.
- [ ] After install, all records show `conflictState = "clean"`.
- [ ] After modifying an installed field file via `save_instance_json`, that field's record shows `conflictState = "local-ahead"`.
- [ ] Boundaries without `.srs-import/import-records.json` are silently skipped.
- [ ] After deleting an installed definition file, the record is still present with its `conflict_state` unchanged (divergence skipped, not fatal).

#### Testing

```bash
cargo test -p srs-repository list_package_imports
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `list_package_imports_empty` — no boundaries → empty summary.
- `list_package_imports_clean_after_install` — install bundle, list → all clean.
- `list_package_imports_local_ahead_after_edit` — install, modify field JSON, list → local-ahead.
- `list_package_imports_skips_boundary_without_records` — boundary with no import-records.json skipped.
- `list_package_imports_keeps_record_after_definition_deleted` — definition file removed, record still present.

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

**Goal:** `srs package imports` returns a valid `PackageImportsPayload` JSON envelope; `From` impls convert service types to payload; golden schema committed.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-cli/src/payload.rs`, add three structs (use `String` for enum fields to avoid schemars issues with imported enums):
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
      #[serde(skip_serializing_if = "Vec::is_empty", default)]
      pub skipped_definitions: Vec<String>,
  }

  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct PackageImportsPayload {
      pub generated_at: String,
      pub fields: Vec<ImportRecordPayload>,
      pub types: Vec<ImportRecordPayload>,
      pub views: Vec<ImportRecordPayload>,
      pub blueprints: Vec<ImportRecordPayload>,
      pub protocols: Vec<ImportRecordPayload>,
      pub relation_types: Vec<ImportRecordPayload>,
      #[serde(skip_serializing_if = "Vec::is_empty", default)]
      pub skipped_definitions: Vec<String>,
  }
  ```
  Note: `PackageImportsPayload` is flat (same fields as `ImportSummaryPayload`; no nesting wrapper).

- [ ] Add `From` impls:
  ```rust
  impl From<ImportRecord> for ImportRecordPayload {
      fn from(r: ImportRecord) -> Self {
          ImportRecordPayload {
              definition_id: r.definition_id,
              definition_type: r.definition_type.to_string(),
              namespace: r.namespace,
              name: r.name,
              version: r.version,
              mode: r.mode.to_string(),
              imported_at: r.imported_at,
              source_package_id: r.source_package_id,
              source_package_name: r.source_package_name,
              source_package_version: r.source_package_version,
              latest_known_upstream_version: r.latest_known_upstream_version,
              update_available: r.update_available,
              update_checked_at: r.update_checked_at,
              conflict_state: r.conflict_state.map(|s| s.to_string()),
              conflict_detected_at: r.conflict_detected_at,
              local_version: r.local_version,
              local_edited_at: r.local_edited_at,
          }
      }
  }

  impl From<ImportSummary> for PackageImportsPayload {
      fn from(s: ImportSummary) -> Self {
          PackageImportsPayload {
              generated_at: s.generated_at,
              fields: s.fields.into_iter().map(ImportRecordPayload::from).collect(),
              types: s.types.into_iter().map(ImportRecordPayload::from).collect(),
              views: s.views.into_iter().map(ImportRecordPayload::from).collect(),
              blueprints: s.blueprints.into_iter().map(ImportRecordPayload::from).collect(),
              protocols: s.protocols.into_iter().map(ImportRecordPayload::from).collect(),
              relation_types: s.relation_types.into_iter().map(ImportRecordPayload::from).collect(),
              skipped_definitions: s.skipped_definitions,
          }
      }
  }
  ```
  Note: `ImportSummaryPayload` is only used internally if needed; `PackageImportsPayload` is the CLI output type.

- [ ] In `commands/mod.rs`, add to `PackageCommand`:
  ```rust
  /// List all import records across all package boundaries
  Imports,
  ```

- [ ] In `commands/package.rs`:
  - Add `PackageCommand::Imports => cmd_package_imports(ctx)` to `dispatch`.
  - Add `use crate::payload::{..., PackageImportsPayload, ImportRecordPayload};`.
  - Add `use srs_repository::package_service::{list_package_imports, ListPackageImportsFilter};`.
  - Implement:
    ```rust
    fn cmd_package_imports(ctx: CliContext) -> Result<String> {
        let summary = with_store(&ctx, |store| {
            Ok(list_package_imports(store, ListPackageImportsFilter {})?)
        })?;
        output::serialize("package imports", PackageImportsPayload::from(summary))
    }
    ```

- [ ] Run `cargo run --bin generate-schemas` and commit generated files.

#### Acceptance Criteria

- [ ] `srs package imports --repo /tmp/test` returns JSON with `"ok": true` and flat `fields`, `types`, etc. fields at the top level of `payload`.
- [ ] `cargo test --test payload_contracts` passes.
- [ ] `crates/srs-cli/schemas/payload/PackageImportsPayload.json` exists.

#### Testing

```bash
cargo build --bin srs
cargo test --test payload_contracts
cargo clippy -- -D warnings
```

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

**Goal:** `list_package_imports_json(&self)` WASM binding returns serialized `ImportSummary` JSON.

**Agent:** Bindings Worker

#### Tasks

- [ ] In `crates/srs-bindings/src/lib.rs`, add to the `SrsRepository` impl block:
  ```rust
  /// List all import records across all package boundaries.
  /// Returns the `ImportSummary` as a JSON string.
  pub fn list_package_imports_json(&self) -> Result<String, JsValue> {
      let summary = package_service::list_package_imports(
          &self.store,
          package_service::ListPackageImportsFilter {},
      )
      .map_err(js_err)?;
      serde_json::to_string(&summary).map_err(|e| js_err(e.into()))
  }
  ```
- [ ] Expand the `use srs_repository::package_service::` import to include `list_package_imports, ListPackageImportsFilter`.
- [ ] Add a smoke test (in `#[cfg(test)]` block or `tests/bindings_smoke.rs`):
  ```rust
  fn list_package_imports_returns_parseable_json() {
      // Install a package into MemoryStore; call service directly;
      // verify JSON parses with "generatedAt" and "fields" keys.
  }
  ```

#### Acceptance Criteria

- [ ] `list_package_imports_json` compiles cleanly.
- [ ] Smoke test: result parses as JSON with `generatedAt` and `fields` keys.

#### Testing

```bash
cargo build -p srs-bindings
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

#### Milestone gate

1. All acceptance criteria met.
2. Commit:
```bash
git commit -m "feat(bindings): list_package_imports_json WASM binding (#246)"
```

---

### Phase 7: Cross-store tests + fixture update

**Goal:** End-to-end divergence detection verified with FileStore; fixture updated if canonical spec is present.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add to integration tests (or `package_service.rs` tests): a test using `FileStore` that installs a bundle, lists imports (all clean), modifies a definition file on disk, lists again (one local-ahead).
- [ ] Update `tests/fixtures/spec-repo/records/extensions/ext-import-tracking.json`: if `../srs/srs/records/extensions/ext-import-tracking.json` exists (relative to workspace root), copy it verbatim to `tests/fixtures/spec-repo/records/extensions/ext-import-tracking.json`. If the canonical file is absent, skip this step and note it in the commit message. Acceptance: if updated, `srs repo validate --repo tests/fixtures/spec-repo` exits 0 (if `srs` binary is available).

#### Acceptance Criteria

- [ ] FileStore roundtrip: install → modify definition file → detect divergence → `local-ahead`.
- [ ] MemoryStore: install → list → all clean (no reference file I/O in memory store, so divergence is skipped for MemoryStore definitions that have no real file path — this is expected and acceptable).
- [ ] Fixture updated (or confirmed absent with note).

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
git commit -m "test: cross-store import tracking roundtrip tests; update fixture (#246)"
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] `srs package imports` returns valid flat JSON payload
- [ ] `srs package import --mode local-fork --path <path>` works
- [ ] After `package install`, `.srs-import/import-records.json` exists under the boundary
- [ ] After editing an installed definition, `srs package imports` reports `conflictState: "local-ahead"`
- [ ] `PackageImportsPayload.json` golden schema committed
- [ ] ADR-030 status is `accepted`

## Coordination Rules

- Agents keep to their write scopes.
- Repository Service Worker does not touch `payload.rs` or CLI command handlers.
- CLI Worker does not implement service logic — one `list_package_imports` call per handler.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests pass, update plan checkboxes, then commit.

## Assumptions

- `chrono::Utc::now().to_rfc3339()` is available in `srs-repository` (already used in existing code).
- `MemoryStore` does not need real file paths to store instance JSON; for MemoryStore-based tests, divergence detection silently skips records whose reference copies don't exist (because MemoryStore has no real filesystem paths). This is acceptable since divergence detection is a FileStore concern.
- The `srs-bindings` crate compiles for the host target in tests (WASM target build is CI-only).
- Views and RelationTypes are not tracked in `PackageBoundary`; they are excluded from Phase 3 ImportRecord creation.
- For `package import`, definition IDs are read from the JSON files in the boundary; if missing or malformed, the definition is skipped with an entry in `skipped_definitions` (non-fatal).
