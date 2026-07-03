# Plan: Typed manifest.container + containerIndex + identityInstanceId (#263)

## Summary

`manifest.container` and `containerIndex` live today only in the `Manifest.extra` bag, meaning every caller must re-parse them from raw `serde_json::Value`. `Container` also lacks the RFC-013 `identityInstanceId` pointer. This plan promotes all three to first-class typed fields: adds `identityInstanceId` to `Container` in `srs-core`, introduces a `ContainerIndexEntry` type, updates `Manifest` to carry `container: Option<Container>` and `container_index: Option<Vec<ContainerIndexEntry>>`, then migrates every caller off `manifest.extra.get("container*")` to use the typed accessors.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Core Model Worker | srs-core changes |
| Repository Service Worker | srs-repository changes + caller migration |
| Verification | — |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| ADR-001 (library-first) | `ContainerIndexEntry` goes in `srs-core`, not `srs-repository` or `srs-cli` | accepted |
| ADR-010 (service boundary) | No CLI logic change — this is a type promotion | accepted |
| ADR-011 (CLI output contract) | `Container` is embedded in some payload structs as `serde_json::Value`; no payload struct changes, so no schema regeneration | accepted |
| ADR-013 (WASM binding strategy) | `Container` is returned as `serde_json::Value` in the WASM surface; adding `identity_instance_id` (optional, skip_serializing_if) does not break WASM callers; no binding changes needed | accepted |

No new ADRs required — this plan implements a type promotion dictated by RFC-013 with no new architectural choices.

## Contracts

### CLI output contract (ADR-011)

No new/changed CLI commands. `Container` appears in some payloads as `serde_json::Value` (so schemars doesn't reach into `srs-core`). Adding a field to `Container` does not affect the JSON Schema golden files — no schema regeneration.

### Entity schema sync

No changes to `srs/docs/schema/2.0/`. The `identityInstanceId` field and `containerIndex` shape are already in the schemas. This plan brings the Rust types into alignment with those schemas.

---

## Scope

- Add `identity_instance_id: Option<String>` (serialised as `identityInstanceId`) to `Container` in `srs-core`; add `#[serde(default)]` to `title` so Container can deserialise from a manifest reference that omits title
- Add new `ContainerIndexEntry` struct in `srs-core` with fields: `container_id`, `title`, `path`, `container_type`, `tags` + extra bag
- Add `container: Option<Container>` and `container_index: Option<Vec<ContainerIndexEntry>>` to `Manifest` in `srs-repository`
- Update ALL ~35 Manifest struct literal constructions throughout `srs-repository` to add `container: None, container_index: None` (compiler errors will enumerate them)
- Migrate callers in `srs-repository` off `manifest.extra.get("container"/"containerIndex")`:
  - `repository_navigation_service.rs` → use `manifest.container`; update `nav_store()` test fixture to set `container: Some(Container {...})` directly on the struct
  - `store.rs` FileStore helpers → keep `Vec<(String,String,String)>` return type; replace deserialization body with typed mapping
  - `store.rs` MemoryStore methods → use `manifest.container_index`
  - `render_service.rs` → use `manifest.container_index`; preserve legacy `"type"` fallback via `entry.extra.get("type")`
- Round-trip tests in `manifest.rs`: memory → JSON → memory for manifests with `container` + `container_index`

**Out of scope:**
- Validation enforcement (manifest.container required) — that is #264
- Navigation service logic changes beyond swapping the raw-JSON read for the typed field
- json_store.rs: it manipulates `manifest_val["containerIndex"]` as raw JSON, independent of the Manifest struct, and continues to work correctly after this change

---

## Phases

### Phase 1: srs-core types

**Goal:** `Container` has `identityInstanceId`; `ContainerIndexEntry` type exists in `srs-core`.

**Agent:** Core Model Worker

#### Tasks

- [ ] Add `#[serde(default)]` to `title: String` in `Container` so it can be deserialized from a manifest reference that omits the title field
- [ ] Add `identity_instance_id: Option<String>` field with `#[serde(skip_serializing_if = "Option::is_none")]` to `Container` in `crates/srs-core/src/types/container.rs` (place before `root_instance_ids`)
- [ ] Add `ContainerIndexEntry` struct to `crates/srs-core/src/types/container.rs`:
  ```rust
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct ContainerIndexEntry {
      pub container_id: String,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub title: Option<String>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub path: Option<String>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub container_type: Option<String>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub tags: Option<Vec<String>>,
      #[serde(flatten)]
      pub extra: HashMap<String, serde_json::Value>,
  }
  ```
- [ ] Export `ContainerIndexEntry` from `crates/srs-core/src/types/mod.rs` (already exported via the `container` module; no mod.rs change needed unless a re-export is desired)
- [ ] Update `container_roundtrips_all_fields` test to include `identity_instance_id: Some("uuid-test".to_string())` and verify it round-trips
- [ ] Add `container_index_entry_roundtrips` test in `crates/srs-core/src/types/container.rs`:
  - Entry with all fields → JSON → entry (verify all fields survive)
  - Entry with only `container_id` → JSON → entry (verify optional fields are None)

#### Acceptance Criteria

- [ ] `Container` round-trip test includes `identity_instance_id: Some("uuid-test".to_string())` and passes
- [ ] `ContainerIndexEntry` has its own round-trip test
- [ ] `cargo test -p srs-core` passes

#### Testing

```bash
cargo test -p srs-core
```

Specific tests to write or verify:

- `container_roundtrips_all_fields` — update to include `identity_instance_id`
- `container_index_entry_roundtrips` — new test in container.rs

#### Milestone gate

1. All acceptance criteria above checked.
2. `cargo test -p srs-core` passes.
3. `cargo clippy -p srs-core -- -D warnings` passes.
4. Commit: `feat(srs-core): add identityInstanceId to Container + ContainerIndexEntry type (#263)`

---

### Phase 2: Manifest typed fields

**Goal:** `Manifest` carries `container` and `container_index` as typed fields; all ~35 struct literal construction sites compile; round-trip tests pass.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/manifest.rs`, add `use srs_core::types::container::{Container, ContainerIndexEntry};`
- [ ] Add to `Manifest` (before the `extra` field so flatten still catches unknown fields):
  ```rust
  #[serde(skip_serializing_if = "Option::is_none")]
  pub container: Option<Container>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub container_index: Option<Vec<ContainerIndexEntry>>,
  ```
- [ ] Run `cargo build -p srs-repository 2>&1 | grep "error\[" | grep -o "src/[^:]*" | sort -u` to enumerate all struct literal compilation errors
- [ ] Add `container: None, container_index: None` to every `Manifest { ... }` struct literal in `crates/srs-repository/src/`. Expected ~35 sites across: `store.rs` (4 sites), `writer.rs` (2 sites), `repository_navigation_service.rs` (1), `render_service.rs` (7), `record_store.rs` (4), `services.rs` (5), `view_service.rs` (2), `blueprint_brief_service.rs` (1), `type_schema_service.rs` (2), `container_view_service.rs` (1), `discovery_service.rs` (1), `blueprint_schema_service.rs` (1), `tree_service.rs` (1), `json_store.rs` (2)
- [ ] Add round-trip tests **in `crates/srs-repository/src/manifest.rs`**:
  - `manifest_with_container_roundtrips` — JSON with `container: {..., identityInstanceId: "uuid"}` → `Manifest` → JSON → parsed again; `container.identity_instance_id` is `Some("uuid")`
  - `manifest_with_container_index_roundtrips` — JSON with `containerIndex: [{containerId, path, title}]` → typed field round-trips

#### Acceptance Criteria

- [ ] A manifest JSON with `container` + `identityInstanceId` deserialises to `manifest.container.identity_instance_id == Some(...)`
- [ ] A manifest JSON with `containerIndex` deserialises to `manifest.container_index == Some([...])`
- [ ] Both fields serialise back to the same JSON keys (`"container"`, `"containerIndex"`)
- [ ] Fields absent in JSON → fields are `None` (no parse error)
- [ ] `cargo test -p srs-repository` passes

#### Testing

```bash
cargo test -p srs-repository
```

Specific tests to write or verify:

- `manifest_with_container_roundtrips` — new test in `manifest.rs`
- `manifest_with_container_index_roundtrips` — new test in `manifest.rs`
- `live_manifest_loads_and_has_correct_first_entry` — existing test; must still pass

#### Milestone gate

1. All acceptance criteria checked.
2. Tests named above exist and pass.
3. `cargo clippy -p srs-repository -- -D warnings` passes.
4. Commit: `feat(srs-repository): promote manifest.container + containerIndex to typed fields (#263)`

---

### Phase 3: Caller migration

**Goal:** No caller reads `manifest.extra.get("container")` or `manifest.extra.get("containerIndex")`; all tests pass.

**Agent:** Repository Service Worker

#### Tasks

**`repository_navigation_service.rs`**
- [ ] Remove the `ManifestContainerRef` local struct (lines 37-40)
- [ ] Replace `let Some(raw_container_ref) = manifest.extra.get("container").cloned()` with `let Some(root_container_manifest) = &manifest.container`
- [ ] Remove the `serde_json::from_value::<ManifestContainerRef>(raw_container_ref)` parse step and its error mapping
- [ ] Derive `container_id` from `root_container_manifest.container_id`
- [ ] Derive `identity_instance_id` from `root_container_manifest.identity_instance_id`
- [ ] Update `root_container_id` return value to `root_container_manifest.container_id.clone()`
- [ ] Update `nav_store()` test fixture: replace the `extra.insert("container", ...)` block with `manifest.container = Some(Container { container_id: "00000000-0000-4000-8000-00000000a000".to_string(), identity_instance_id: Some("00000000-0000-4000-8000-00000000a100".to_string()), title: String::new(), ... all_option_fields_none ... })` and `manifest.extra: HashMap::new()`; remove the stale local `extra` variable

**`store.rs` — FileStore helpers**
- [ ] `file_store_load_container_index`: keep return type `Vec<(String, String, String)>`; replace the `manifest.extra.get("containerIndex")` deserialization body with:
  ```rust
  Ok(manifest
      .container_index
      .unwrap_or_default()
      .into_iter()
      .filter_map(|e| e.path.map(|p| (e.container_id, e.title.unwrap_or_default(), p)))
      .collect())
  ```
- [ ] `file_store_upsert_container_index`: replace `manifest.extra.get("containerIndex")` / `manifest.extra.insert(...)` with:
  - Load `manifest.container_index.unwrap_or_default()` into `entries: Vec<ContainerIndexEntry>`
  - Retain all entries where `e.container_id != container_id`
  - Push `ContainerIndexEntry { container_id: container_id.to_string(), title: Some(title.to_string()), path: Some(path.to_string()), container_type: None, tags: None, extra: HashMap::new() }`
  - Set `manifest.container_index = Some(entries)` and save
- [ ] `file_store_remove_container_index`: replace `manifest.extra.get/insert` pattern with:
  - Load `manifest.container_index.unwrap_or_default()` into `entries: Vec<ContainerIndexEntry>`
  - Retain entries where `e.container_id != container_id`
  - Set `manifest.container_index = Some(entries)` and save

**`store.rs` — MemoryStore**
- [ ] `save_container`: replace `manifest.extra.get("containerIndex")` / `manifest.extra.insert(...)` with:
  - Load `manifest.container_index.get_or_insert(vec![])`
  - Retain entries where `e.container_id != id`
  - Push `ContainerIndexEntry { container_id: id.to_string(), title: Some(container.title.clone()), path: None, container_type: None, tags: None, extra: HashMap::new() }`
- [ ] `delete_container`: replace `manifest.extra.get/insert("containerIndex", ...)` with:
  - Load `manifest.container_index.get_or_insert(vec![])`
  - Retain entries where `e.container_id != container_id`
- [ ] `list_container_summaries`: replace `manifest.extra.get("containerIndex")` with `manifest.container_index.as_deref().unwrap_or(&[])` and map `.container_id` + `.title.unwrap_or_default()` fields

**`render_service.rs`**
- [ ] Replace `if let Some(container_index) = manifest.extra.get("containerIndex")` with direct use of `manifest.container_index.as_deref().unwrap_or(&[])`
- [ ] Replace `entry.get("containerId").and_then(|v| v.as_str())` with `entry.container_id.as_str()` (or `&*entry.container_id`)
- [ ] Replace `entry.get("title").and_then(|v| v.as_str())` with `entry.title.as_deref()`
- [ ] Replace the `containerType`/`"type"` fallback with: `entry.container_type.as_deref().or_else(|| entry.extra.get("type").and_then(|v| v.as_str()))` — preserves backward compatibility with older repositories that used `"type"` instead of `"containerType"`

#### Acceptance Criteria

- [ ] `grep -rn 'extra.*"container\|extra\.get.*"container' crates/srs-repository/src/` returns no hits (except comments)
- [ ] All existing `cargo test -p srs-repository` tests pass including navigation and render tests
- [ ] `cargo test` (full workspace) passes

#### Testing

```bash
cargo test -p srs-repository
cargo test
```

Specific tests to write or verify:

- All existing navigation service tests — must still pass
- All existing container CRUD tests — must still pass
- `list_container_summaries` behaviour — confirm returns correct (id, title) pairs after upsert/delete

#### Milestone gate

1. All acceptance criteria checked.
2. `cargo test` passes.
3. `cargo clippy -- -D warnings` passes.
4. Commit: `refactor(srs-repository): migrate callers from extra bag to typed manifest fields (#263)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `manifest_with_container_roundtrips` and `manifest_with_container_index_roundtrips` exist and pass
- [ ] `grep -rn 'extra.*"container' crates/srs-repository/src/` returns zero hits for the migrated callers

## Coordination Rules

- Core Model Worker writes only to `crates/srs-core/`.
- Repository Service Worker writes only to `crates/srs-repository/`.
- Milestone gates must pass before proceeding to next phase.
- No business logic added — this is a type promotion and caller migration.

## Assumptions

- The spec repo manifest (`srs/srs/manifest.json`) does not yet have a `container` field — that's fine; it will deserialise to `container: None`.
- json_store.rs's `upsert_container` / `delete_container` manipulate the raw JSON value for `"manifest.json"` directly (not through the Manifest struct). These continue to work correctly because the typed field promotion only affects Manifest struct serde, and the raw JSON is consistent with what `container_index` would hold. No change needed there.
