# Plan: Container Slice Export (srs-rust#631)

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

RFC-026 (`ext:slices`) defines a **container-membership closure** export that produces a valid `.srs` archive containing only the instances, relations, and definitions reachable from a specified container. The blocking RFC was accepted 2026-07-21 and the manifest.json schema already contains the `slice`/`Slice`/`SliceSpec`/`SliceExternalRef` definitions in both the spec repo and the srs-rust mirror. This plan implements the Rust service, CLI handler, and WASM binding for container-slice export. Record closure is explicitly deferred (per issue scope notes).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | claude (main loop) |
| Repository Worker | claude / subagent |
| CLI Worker | claude / subagent |
| Bindings Worker | claude / subagent |
| Architecture Reviewer | Architecture Reviewer (subagent, Stage 3 review) |
| Plan Reviewer | Plan Reviewer (subagent, Stage 3 review) |
| Verification | Verification Agent (subagent, Stage 7) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-033](../docs/adr/033-srs-archive-format.md) | `.srs` archive is a deterministic ZIP; determinism requirements apply to slice archives unchanged | accepted |
| [ADR-039](../docs/adr/039-srs-archive-pure-tree-zip.md) | Archive is a pure file-tree ZIP; slice implementation uses the same `tree_entries` enumeration as the basis, then filters | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `export_container_slice` is a service function in `srs-repository`; CLI handler is a thin wrapper | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | New `SliceExportPayload` struct in `payload.rs`; golden schema file regenerated | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM binding is a thin wrapper: deserialize input → one service call → serialize output | accepted |
| [ADR-036](../docs/adr/036-srs-is-default-working-format.md) | Slice output is a `.srs` archive (the default working format) | accepted |
| [ADR-038](../docs/adr/038-vfs-tree-primary-model.md) | Service uses `store.load_container()` (logical-id method, not path-based) | accepted |
| [ADR-043](../docs/adr/043-slice-export-is-dedicated-archive.md) | Container slices are dedicated `.srs` archives, not a snapshot-filter mode on `archive_pack` | **proposed** |

### ⚠️ Design Decision Pending: CLI Command Surface

**This decision is a new public API shape** (painful to reverse) and requires human input before the plan is finalised. Implementation proceeds only after this is resolved.

**Question:** What should the CLI command be?

**Option A (Recommended): `srs slice export --container <id> --output <path.srs>`**
- New top-level subcommand group `slice` with `export` variant.
- Creates a clear namespace for future `srs slice import` / `srs slice validate` etc.
- Follows the existing pattern: `srs archive pack`, `srs repo copy`, `srs record create`.
- Breaking change if renamed later; choose the name once.
- Tradeoff: adds one more top-level command to `--help` output.

**Option B: `srs archive pack --container <id> --output <path.srs>`**
- Extends the existing `ArchiveCommand::Pack` variant with an optional `--container` flag.
- Keeps "everything about producing archives" under `srs archive`.
- Tradeoff: semantically overloads `pack` (full-repo vs. filtered); the current `Pack` variant has `--output` positional arg — adding `--container` changes the flag schema; the two behaviours (full / filtered) are fundamentally different and would benefit from separate commands.

**Recommendation: Option A.** It cleanly separates slice export as a first-class operation, avoids overloading `archive pack`, and leaves room for future slice verbs. Record the decision in the Architecture Decisions table and in ADR-043.

---

## Contracts

### CLI output contract (ADR-011)

This plan adds two new CLI commands:

1. `srs slice export` → new `SliceExportPayload` struct:
   ```rust
   pub struct SliceExportPayload {
       pub output_path: String,
       pub file_size_bytes: u64,
       pub container_id: String,
       pub included_instance_count: usize,
       pub total_instance_count: usize,
       pub external_relation_ref_count: usize,
       pub slice_repository_id: String,   // new repositoryId assigned to the slice
   }
   ```

After adding `SliceExportPayload` to `crates/srs-cli/src/payload.rs`:
```bash
cargo run --bin generate-schemas
# commit updated crates/srs-cli/schemas/payload/slice-export.json
```

`cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No entity schema changes — `crates/srs-schema/schemas/2.0/manifest.json` already contains the `slice`/`Slice`/`SliceSpec`/`SliceExternalRef` definitions from RFC-026 (confirmed: 15 occurrences). No sync action required.

---

## Scope

- Container-membership closure: collect all instance IDs from `container.member_instance_ids` and `container.root_instance_ids`.
- Filter instances to closure members only.
- Filter relations: both endpoints in closure → include; exactly one endpoint in closure → `externalRelationRefs[]`; neither endpoint → omit entirely.
- Write manifest with `slice` block per RFC-026 schema: `origin.repositoryId`, `spec.type="container"`, `spec.id`, `exportedAt`, `externalRelationRefs[]`.
- Assign a new `repositoryId` UUID to the slice (required by RFC-026 R2 — the slice MUST NOT share the source's repositoryId).
- Add `ext:slices` to `declaredExtensions` in the slice manifest (RFC-026 R1).
- Slice archive must pass `srs repo validate`.
- CLI: `srs slice export --container <id> --output <path.srs>` (pending human confirmation of Option A).
- WASM binding: `export_container_slice(container_id: &str) -> Result<Vec<u8>, JsValue>`.
- Source documents: include all (sidecars + binary bytes) without filtering to closure instances. This is a simplification for the first cut — the RFC does not forbid having more source documents than strictly required.
- ADR-043 drafted and status flipped to `accepted` when this plan ships.

**Out of scope:**
- Record closure (traversal through relation edges from instances to other instances). Deferred — file issue.
- `PackageBoundary` SliceSpec type. Per RFC comments (2026-07-21), package export is RFC-003, not RFC-026. Deferred — `SliceSpec.type` is a closed enum `["container"]` per the schema.
- Partial source-document filtering (filtering source docs to only those referenced by closure instances). Deferred — file issue.
- Re-importability of a slice back into the source repository.
- `srs slice import` command.
- MCP tool for slice export. Deferred — file issue.
- Filtering definitions (fields, types, etc.) to only those used by closure instances. The full package is included. Deferred — file issue.

---

## Phases

### Phase 1: Core types in srs-core

**Goal:** The canonical `SliceSpec`, `SliceExternalRef`, and `Slice` types exist in `srs-core` with correct serde shapes matching the schema.

**Agent:** Repository Worker

#### Tasks

- [ ] Create `crates/srs-core/src/types/slice.rs` with:
  ```rust
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct SliceSpec {
      /// Always "container" per RFC-026 (closed enum in schema).
      #[serde(rename = "type")]
      pub slice_type: String,
      /// The containerId scoping this slice.
      pub id: String,
  }

  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct SliceExternalRef {
      pub relation_id: String,
      pub source_instance_id: String,
      pub target_instance_id: String,
      pub relation_type: String,
  }

  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct Slice {
      pub origin: SliceOrigin,
      pub spec: SliceSpec,
      pub exported_at: String,   // RFC 3339 date-time
      #[serde(default, skip_serializing_if = "Vec::is_empty")]
      pub external_relation_refs: Vec<SliceExternalRef>,
  }

  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct SliceOrigin {
      pub repository_id: String,
  }
  ```
- [ ] Register the module: add `pub mod slice;` to `crates/srs-core/src/types/mod.rs` (after the existing pub mod entries, in alphabetical position).
- [ ] Verify no `schemars` or file I/O is used in `slice.rs` (srs-core hard constraint).

#### Acceptance Criteria

- [ ] `cargo test -p srs-core` passes.
- [ ] `SliceSpec { slice_type: "container".into(), id: uuid }` round-trips through `serde_json::to_string` / `serde_json::from_str` with camelCase field names (`type`, `id`).
- [ ] `Slice` with non-empty `externalRelationRefs` serializes the array; `Slice` with empty `externalRelationRefs` omits the key.
- [ ] No `schemars`, no `std::fs`, no `std::io` imports in `slice.rs`.

#### Testing

```bash
cargo test -p srs-core
```

Specific tests to write in `slice.rs` (in a `#[cfg(test)]` mod):
- `slice_spec_round_trips` — verifies `SliceSpec` serde with `"type"` key
- `slice_external_ref_round_trips` — verifies camelCase field names
- `slice_empty_external_refs_omits_key` — `Slice` with `external_relation_refs: vec![]` has no `externalRelationRefs` key in serialized JSON
- `slice_origin_serializes_correctly` — `SliceOrigin` serializes to `{ "repositoryId": "..." }`

#### Milestone gate

1. All acceptance criteria checked.
2. All four tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-core
   cargo clippy -p srs-core -- -D warnings
   ```
4. Mark task checkboxes `[x]`. Commit:
   ```bash
   git add crates/srs-core/
   git commit -m "feat(srs-core): add SliceSpec, Slice, SliceExternalRef types (RFC-026) (#631)"
   ```

---

### Phase 2: Slice export service in srs-repository

**Goal:** `srs_repository::slice_service::export_container_slice` exists, correctly filters instances and relations, writes a valid deterministic `.srs` ZIP, and has a passing test suite.

**Agent:** Repository Worker

#### Tasks

- [ ] Create `crates/srs-repository/src/slice_service.rs`:
  ```rust
  use crate::archive::tree_entries;
  use crate::error::RepositoryError;
  use crate::store::RepositoryStore;
  use srs_core::types::relation::Relation;
  use srs_core::types::slice::{Slice, SliceExternalRef, SliceOrigin, SliceSpec};
  use std::collections::{BTreeMap, HashSet};
  use std::io::{Read, Seek, Write};
  use zip::write::SimpleFileOptions;
  use uuid::Uuid;

  pub struct ContainerSliceInput {
      pub container_id: String,
  }

  pub struct ContainerSliceResult {
      pub included_instance_count: usize,
      pub total_instance_count: usize,
      pub external_relation_ref_count: usize,
      pub slice_repository_id: String,
  }

  pub fn export_container_slice(
      source: &dyn RepositoryStore,
      input: ContainerSliceInput,
      writer: impl Write + Seek,
  ) -> Result<ContainerSliceResult, RepositoryError>
  ```

- [ ] Implement the container-closure algorithm inside `export_container_slice`:

  1. **Load the container** via `source.load_container(&input.container_id)?` (logical-id method, per ADR-042). Return `RepositoryError::ContainerNotFound { container_id: input.container_id.clone() }` if absent.
  2. **Compute closure** (type `HashSet<String>`):
     ```
     closure = container.member_instance_ids.unwrap_or_default()
             ∪ container.root_instance_ids.unwrap_or_default()
     ```
  3. **Get the full tree** via `tree_entries(source)?` → `BTreeMap<String, Vec<u8>>`.
  4. **Parse `manifest.json`** from the tree (not a second I/O call — use the bytes already in the tree). Use `serde_json::from_slice` on `tree["manifest.json"]`.
  5. **Filter `instanceIndex`**: keep only entries where `instance_index_entry.instance_id` is in the closure (note: the `path` field is the storage path; filter by `instance_id` which is in the JSON key `"instanceId"` — check the InstanceIndexEntry struct field name).
  6. **Identify instance files to keep**: build a `HashSet<String>` of paths for kept instanceIndex entries.
  7. **Parse relations**: load `relations/relations-collection.json` or `relations/relations.json` bytes from the tree. If absent, relations are empty. Deserialize as `{ "relations": Vec<Relation> }`.
  8. **Split relations**:
     - Both `source_instance_id` AND `target_instance_id` in closure → `included_relations`
     - Exactly one in closure → `external_refs` (build `SliceExternalRef`)
     - Neither in closure → drop
  9. **Build the slice manifest value** (`serde_json::Value`):
     - Start from the parsed manifest value.
     - Replace `repositoryId` with `Uuid::new_v4().to_string()` (store as `slice_repository_id` to return).
     - Update `instanceIndex` to only the filtered entries.
     - Update `containerIndex` to just the one entry for the sliced container (find by `container_id` in source's `container_index`; if the container is embedded in `manifest.container` only, build a minimal `ContainerIndexEntry`).
     - Set `container` to the sliced container object (from `source.load_container`).
     - Set or update `declaredExtensions` to include `"ext:slices"`.
     - Set `slice` key to the serialized `Slice { origin: SliceOrigin { repository_id: source_repo_id }, spec: SliceSpec { slice_type: "container", id: container_id }, exported_at: now_rfc3339(), external_relation_refs: external_refs }`.
     - Remove any `upstreamPackage` (the slice is not a downstream package).
  10. **Serialize updated manifest** to JSON bytes with `srs_core::ser::to_vec_deterministic` (the ADR-017 serializer) or `serde_json::to_vec` if deterministic is not needed at the service level (the determinism comes from lexicographic ZIP entry ordering, not manifest field ordering). Use `serde_json::to_vec` for now; the ZIP order provides the determinism guarantee.
  11. **Build filtered tree** (`BTreeMap<String, Vec<u8>>`):
      - Start with all entries from the full tree.
      - Remove instance files NOT in `paths_to_keep`.
      - Replace `"manifest.json"` with the new manifest bytes.
      - Replace the relations file with the serialized included-only relations (same filename as in source).
      - Keep all package files (definitions are not filtered).
      - Keep all `source-documents/` files (not filtered in first cut).
      - Keep all container files ONLY for the sliced container (remove container files not belonging to the sliced container, using containerIndex paths to identify).
  12. **Write deterministic ZIP** from the filtered BTreeMap (same as `archive_pack`): sorted iteration (BTreeMap), zeroed timestamps, Deflated compression.
  13. Return `ContainerSliceResult`.

- [ ] Helper `now_rfc3339() -> String`: use `chrono` crate if available, or check if there's already a time helper in the codebase (`grep -r "chrono\|rfc3339" crates/`). If chrono is not already a dependency, use a fallback that formats a static string from environment/build timestamp (or just `std::time::SystemTime::now()`). Check `Cargo.toml` first.

- [ ] Re-export from `crates/srs-repository/src/lib.rs`:
  ```rust
  pub mod slice_service;
  pub use slice_service::{export_container_slice, ContainerSliceInput, ContainerSliceResult};
  ```

- [ ] Check what `InstanceIndexEntry` looks like (the `instance_id` field name might be `id` in the JSON). Read `crates/srs-repository/src/index.rs` to confirm. The `path` field in `InstanceIndexEntry` is adapter-private (CLAUDE.md) but the archiver legitimately needs it to find the file bytes.

- [ ] Verify `uuid` is already a dependency in `srs-repository/Cargo.toml`. If not, add `uuid = { version = "1", features = ["v4"] }`. (Check: `grep uuid crates/srs-repository/Cargo.toml`.)

#### Acceptance Criteria

- [ ] `export_container_slice` returns `RepositoryError::ContainerNotFound` when the container_id does not exist.
- [ ] The produced archive is a valid ZIP containing `manifest.json`.
- [ ] The slice's `manifest.json` contains `"slice": { "origin": { "repositoryId": "<source-id>" }, "spec": { "type": "container", "id": "<container-id>" }, "exportedAt": "...", ... }`.
- [ ] The slice's `manifest.json` `repositoryId` differs from the source repository's `repositoryId`.
- [ ] The slice's `instanceIndex` contains only instances whose IDs are in the container's `member_instance_ids` ∪ `root_instance_ids`.
- [ ] Relations where both endpoints are in closure are in the slice's relations file.
- [ ] Relations where exactly one endpoint is in closure appear in `slice.externalRelationRefs` and NOT in the relations file.
- [ ] Relations where neither endpoint is in closure do not appear anywhere in the slice.
- [ ] `declaredExtensions` in the slice manifest includes `"ext:slices"`.
- [ ] ZIP entries are lexicographically sorted (BTreeMap iteration guarantees this).
- [ ] `cargo test -p srs-repository` passes.

#### Testing

```bash
cargo test -p srs-repository
```

Specific tests to write (in `crates/srs-repository/tests/slice_service_tests.rs` or inline in `slice_service.rs`):
- `export_container_slice_unknown_container_returns_error` — call with non-existent container_id, expect `ContainerNotFound`.
- `export_container_slice_produces_valid_zip` — create a MemoryStore with 3 instances and a container containing 2 of them; call `export_container_slice`; verify the result is a valid ZIP via `zip::ZipArchive::new`.
- `export_container_slice_filters_instances` — as above, verify the slice's `instanceIndex` has exactly the 2 members, not the third.
- `export_container_slice_splits_relations` — create relations: both-in-closure, one-in-one-out, both-out; verify the slice's relations and `externalRelationRefs` are correct.
- `export_container_slice_manifest_has_slice_block` — verify `manifest.slice.spec.type == "container"`, `spec.id == container_id`, `origin.repositoryId == source_repository_id`.
- `export_container_slice_new_repository_id` — verify slice `repositoryId` != source `repositoryId`.
- `export_container_slice_declares_ext_slices` — verify `declaredExtensions` contains `"ext:slices"`.

#### Milestone gate

1. All acceptance criteria checked.
2. All 7 tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Mark task checkboxes `[x]`. Commit:
   ```bash
   git add crates/srs-repository/
   git commit -m "feat(srs-repository): export_container_slice service — RFC-026 container closure (#631)"
   ```

---

### Phase 3: CLI command (srs-cli)

**Goal:** `srs slice export --container <id> --output <path.srs>` is a working CLI command that delegates to `export_container_slice` and outputs a `SliceExportPayload`.

**Agent:** CLI Worker

**Prerequisite:** Phase 2 milestone gate passed; human confirmed Option A for CLI surface (or override).

#### Tasks

- [ ] Add `SliceExportPayload` to `crates/srs-cli/src/payload.rs` (near the `ArchivePackPayload` entries):
  ```rust
  /// Payload for `srs slice export`.
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct SliceExportPayload {
      pub output_path: String,
      pub file_size_bytes: u64,
      pub container_id: String,
      pub included_instance_count: usize,
      pub total_instance_count: usize,
      pub external_relation_ref_count: usize,
      pub slice_repository_id: String,
  }
  ```

- [ ] Run `cargo run --bin generate-schemas` to create `crates/srs-cli/schemas/payload/slice-export.json`.

- [ ] Add `SliceCommand` enum to `crates/srs-cli/src/commands/mod.rs`:
  ```rust
  #[derive(Subcommand)]
  pub enum SliceCommand {
      /// Export a container's closure as a self-contained .srs archive (RFC-026).
      Export {
          /// Container ID (UUID) to export.
          #[arg(long)]
          container: String,
          /// Output file path for the .srs archive.
          output: PathBuf,
      },
  }
  ```

- [ ] Add `Slice(SliceCommand)` variant to the `Commands` enum in `mod.rs` (keep `ArchiveCommand` unchanged):
  ```rust
  /// Slice export commands (container closure, RFC-026 ext:slices)
  #[command(name = "slice")]
  Slice(SliceCommand),
  ```

- [ ] Create `crates/srs-cli/src/commands/slice.rs`:
  ```rust
  use crate::commands::{with_store, CliContext, SliceCommand};
  use crate::output;
  use crate::payload::SliceExportPayload;
  use anyhow::Result;
  use std::path::PathBuf;

  pub fn dispatch(ctx: CliContext, cmd: SliceCommand) -> Result<String> {
      match cmd {
          SliceCommand::Export { container, output } => cmd_slice_export(ctx, container, output),
      }
  }

  fn cmd_slice_export(ctx: CliContext, container_id: String, output: PathBuf) -> Result<String> {
      let mut file = std::fs::File::create(&output)
          .map_err(|e| anyhow::anyhow!("cannot create output file {:?}: {}", output, e))?;
      let result = with_store(&ctx, |store| {
          srs_repository::export_container_slice(
              store,
              srs_repository::ContainerSliceInput { container_id: container_id.clone() },
              &mut file,
          )
          .map_err(anyhow::Error::from)
      })?;
      let file_size_bytes = std::fs::metadata(&output)
          .map_err(|e| anyhow::anyhow!("cannot stat output file {:?}: {}", output, e))?
          .len();
      output::serialize(
          "slice export",
          SliceExportPayload {
              output_path: output.to_string_lossy().into_owned(),
              file_size_bytes,
              container_id,
              included_instance_count: result.included_instance_count,
              total_instance_count: result.total_instance_count,
              external_relation_ref_count: result.external_relation_ref_count,
              slice_repository_id: result.slice_repository_id,
          },
      )
  }
  ```

- [ ] Register `slice.rs` in `crates/srs-cli/src/commands/mod.rs`: add `pub mod slice;` and wire `Commands::Slice(cmd) => slice::dispatch(ctx, cmd)` in the dispatch function.

- [ ] Verify `crates/srs-cli/src/commands/mod.rs` imports `SliceCommand` from the right module.

#### Acceptance Criteria

- [ ] `srs slice export --container <uuid> <output.srs>` executes without error against a repository that has a container.
- [ ] The output file is a valid ZIP.
- [ ] The CLI JSON output matches `SliceExportPayload` shape (check `srs slice export ... | jq .payload`).
- [ ] `cargo test --test payload_contracts` passes (golden schema file committed).
- [ ] `cargo build` passes; no new clippy warnings.

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

Specific tests:
- `payload_contracts` golden file test (automatic via `cargo test --test payload_contracts`).

#### Milestone gate

1. All acceptance criteria checked.
2. `cargo test --test payload_contracts` passes.
3. Run:
   ```bash
   cargo clippy -p srs-cli -- -D warnings
   cargo build --bin srs
   ```
4. Mark task checkboxes `[x]`. Commit:
   ```bash
   git add crates/srs-cli/ schemas/
   git commit -m "feat(srs-cli): srs slice export command — RFC-026 container closure (#631)"
   ```

---

### Phase 4: WASM binding (srs-bindings)

**Goal:** WASM callers can call `export_container_slice` and receive `Vec<u8>` (the `.srs` archive bytes).

**Agent:** Bindings Worker

#### Tasks

- [ ] Add a new method to `SrsRepository` in `crates/srs-bindings/src/lib.rs`:
  ```rust
  pub fn export_container_slice(&self, container_id: String) -> Result<Vec<u8>, JsValue> {
      let mut buf = std::io::Cursor::new(Vec::new());
      srs_repository::export_container_slice(
          self.store(),
          srs_repository::ContainerSliceInput { container_id },
          &mut buf,
      )
      .map_err(|e| JsValue::from_str(&e.to_string()))?;
      Ok(buf.into_inner())
  }
  ```
  The return type `Vec<u8>` is exposed to JS as a `Uint8Array` by wasm-bindgen.

- [ ] Ensure the method is annotated `#[wasm_bindgen]` if the file uses that pattern, or add `#[wasm_bindgen]` to the `SrsRepository` impl block if not already present for this method.

- [ ] Confirm `std::io::Cursor` is available (no new dependency needed).

#### Acceptance Criteria

- [ ] `cargo build -p srs-bindings --target wasm32-unknown-unknown` (or the workspace WASM check) passes.
- [ ] `cargo test -p srs-bindings` passes.

#### Testing

```bash
cargo build -p srs-bindings
cargo test -p srs-bindings
```

#### Milestone gate

1. All acceptance criteria checked.
2. Run:
   ```bash
   cargo clippy -p srs-bindings -- -D warnings
   ```
3. Mark task checkboxes `[x]`. Commit:
   ```bash
   git add crates/srs-bindings/
   git commit -m "feat(srs-bindings): export_container_slice WASM binding (#631)"
   ```

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (schema unchanged; this is a verification that nothing was accidentally modified)
- [ ] `srs slice export --container <id> --output /tmp/test-slice.srs --repo <dogfood-repo>` executes successfully
- [ ] `srs repo validate --repo <unpacked-slice-dir>` reports 0 errors on the unpacked slice
- [ ] The slice archive's `manifest.json` contains a valid `slice` block per RFC-026 schema
- [ ] All 7 service-layer tests pass
- [ ] ADR-043 status flipped to `accepted`

---

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after Phase 2 and before final sign-off (Stage 7).

## Assumptions

- `uuid` crate with `v4` feature is available or can be added to `srs-repository/Cargo.toml`.
- `chrono` or equivalent is available for RFC 3339 timestamp generation; if not, use `std::time::SystemTime` with a manual formatter.
- Container membership is flat (no recursive container nesting through relation edges) — RFC-026 container closure is only `member_instance_ids ∪ root_instance_ids` of the direct container, not transitive.
- Source documents are included without filtering to closure instances (first-cut simplification; a follow-up issue will add proper filtering).
- The `InstanceIndexEntry.instance_id` field name in the JSON is `"instanceId"` — verify against `index.rs` during Phase 2.
- The container file paths in `containerIndex` can be identified by the `ContainerIndexEntry.path` field (may be `None` for embedded-only containers); containers with `path: None` are embedded in `manifest.container` and will be handled by setting `manifest.container` directly.
