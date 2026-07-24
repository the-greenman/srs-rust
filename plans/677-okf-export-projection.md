# Plan: OKF export projection — render a container as a Google Open Knowledge Format bundle

> Issue: srs-rust#677
> Alignment register: item 2 — NOW, weight 80 (R4 × F5 × C4)

## Summary

Add a new CLI command `srs render okf-bundle --container <id> --output <dir>` that exports an SRS container as a Google Open Knowledge Format (OKF v0.1) knowledge bundle — one markdown file per record with YAML frontmatter (`type` from the record's Type name) plus an `index.md` generated from the container's navigation (precedes chain). The export is a thin projection: no OKF concepts leak into core semantics, and the feature is cheap to track or abandon if OKF churns. Export only; import is deferred.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Worker | — |
| CLI Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service function takes typed input struct, returns typed result struct; no JSON literals | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | CLI handler is arg-parse → one service call → `output::serialize`; payload struct in `payload.rs` | accepted |
| Alignment register item 2 | OKF export is an adapter-layer projection, no new semantics; thin and disposable | cited |
| No new ADR | All decisions follow from ADR-010, ADR-011, and the capability-layering guide. The OKF renderer is a self-contained function (not a DocumentView) to avoid requiring package configuration — this is intentional and consistent with "thin projection." A `--view` option could be added later to use a DocumentView for richer prose. | — |

---

## Contracts

### CLI output contract (ADR-011)

New command: `srs render okf-bundle`

Add to `crates/srs-cli/src/payload.rs`:

```rust
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OkfBundlePayload {
    pub file_count: usize,
    pub output_dir: String,
    pub diagnostics: Vec<String>,
}
```

Run `cargo run --bin generate-schemas` after adding the struct. Commit the new `schemas/payload/okf-bundle.json`.

Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No entity schemas modified. No action required.

---

## Scope

- New service function `export_okf_bundle` in `crates/srs-repository/src/okf_export_service.rs`
- New CLI command `srs render okf-bundle --container <id> --output <dir>`
- New payload struct `OkfBundlePayload` in `crates/srs-cli/src/payload.rs`
- Regenerated `crates/srs-cli/schemas/payload/okf-bundle.json`
- Service uses precedes-chain ordering and the existing `record_label` module for headings
- YAML frontmatter contains `type` (record's `type_name`) and `srs_id` (the `instance_id` for traceability)

**Out of scope:**

- OKF import (bundle → Tier-0/Tier-1 records) — deferred until format shows 2+ quarters of life (file a follow-up issue)
- WASM binding — deferred (add after CLI is proven; file a follow-up issue under the same parent story)
- Using a DocumentView for prose rendering — the OKF renderer is self-contained; a `--view` option is a follow-up
- OKF content-blob attachments — OKF v0.1 does not define attachment semantics; deferred
- Support for containers-of-containers (nested OKF structure) — deferred

---

## Phases

### Phase 1: OKF export service in srs-repository

**Goal:** `export_okf_bundle` service is implemented and all unit tests pass.

**Agent:** Repository Worker

#### Tasks

- [ ] Create `crates/srs-repository/src/okf_export_service.rs`:
  - `pub struct OkfExportInput { pub container_id: String }`
  - `pub struct OkfEntry { pub path: String, pub content: String }`
  - `pub struct OkfBundle { pub entries: Vec<OkfEntry>, pub diagnostics: Vec<String> }`
  - `pub fn export_okf_bundle(store: &dyn RepositoryStore, input: OkfExportInput) -> Result<OkfBundle, RepositoryError>`
- [ ] In `export_okf_bundle`:
  1. Load the container via `container_service::get_container(&input.container_id)`; return `RepositoryError::NotFound` if missing.
  2. Get the container title from `container.title` (or `container.name.as_deref().unwrap_or("index")` as fallback).
  3. Get member IDs via `container_service::list_container_members(store, &input.container_id)`.
  4. Load all member instances via `record_store::get_instance_by_id` for each member ID; accumulate diagnostics for any member that fails to load (treat as `InstanceWrapper`) and continue.
  5. Build label indexes via `record_label::build_label_indexes(store)`.
  6. Separate instances into records (Tier 2) and notes/other (Tier 0/1); load all relations via `relation_service::list_relations(store, None)`.
  7. Sort Tier-2 records by `relation_graph::sort_by_precedes_chain`; sort Tier-0 instances the same way.
  8. Produce a merged ordered list: maintain original interleaving by matching against the original `member_ids` order for instances that don't appear in precedes relations.
  9. For each instance in order, call `render_okf_entry(instance, &field_name_index, &identity_field_index)` → `OkfEntry { path, content }`.
  10. Generate `index.md` via `render_okf_index(&container_title, &entries)`.
  11. Return `OkfBundle { entries: all_entries, diagnostics }` — entries list has `index.md` first, then per-record files.
- [ ] Implement `fn render_okf_entry(instance: &InstanceWrapper, fni: &FieldNameIndex, ifi: &IdentityFieldIndex) -> OkfEntry`:
  - Compute `display_label`: use `record_label::record_display_label` for Tier-2 records; fall back to `instance_id` prefix for Tier-0/1.
  - Compute `type_label`: `instance.as_record().map(|r| r.type_name.clone()).unwrap_or_else(|| "note".to_string())`.
  - Compute `filename`: `slugify(&display_label) + "-" + &instance_id[..8.min(instance_id.len())] + ".md"`. Slugify: lowercase, replace non-alphanumeric with `-`, collapse consecutive `-`, trim trailing `-`.
  - Render YAML frontmatter: `---\ntype: {type_label}\nsrs_id: {instance_id}\n---\n`.
  - Render heading: `# {display_label}\n\n`.
  - For Tier-2 records: render field values as `**{field_name}**: {value}\n\n` for each `FieldValue` with a non-null value; look up field name from `field_name_index`; fall back to `field_id` if the field name is not in the index.
  - For Tier-0 notes: render the note's `text` field as-is (or empty).
  - Return `OkfEntry { path: filename, content: full_markdown }`.
- [ ] Implement `fn render_okf_index(title: &str, entries: &[OkfEntry]) -> OkfEntry`:
  - Returns `OkfEntry { path: "index.md".to_string(), content: ... }`.
  - Content: `# {title}\n\n` followed by `- [{display_label}]({path})\n` for each non-index entry. Extract display_label from the `# ` heading line of the entry's content.
- [ ] Add `pub mod okf_export_service;` to `crates/srs-repository/src/lib.rs`.
- [ ] Add `pub use okf_export_service::{export_okf_bundle, OkfBundle, OkfEntry, OkfExportInput};` to the appropriate pub re-exports in `lib.rs`.

#### Acceptance Criteria

- [ ] `export_okf_bundle` on a 3-record container (with a precedes chain A→B→C) returns entries sorted as A, B, C plus `index.md`.
- [ ] `index.md` content starts with `# ` and lists all record files in order with markdown links.
- [ ] Each record entry has YAML frontmatter starting with `---`, containing `type:` and `srs_id:` fields.
- [ ] Missing container returns `RepositoryError::NotFound`.
- [ ] Members that fail to load appear in `diagnostics` and are skipped.
- [ ] Works on `MemoryStore` (no `FileStore`-only I/O paths).

#### Testing

```bash
cargo test -p srs-repository okf
cargo clippy -p srs-repository -- -D warnings
```

Tests to write in `crates/srs-repository/src/okf_export_service.rs` `#[cfg(test)]` block:

- `test_okf_export_three_records_precedes_order` — builds `MemoryStore` with 3 Tier-2 records connected by `precedes` relations A→B→C; asserts order in returned entries and that `index.md` lists them in order A, B, C.
- `test_okf_export_frontmatter` — asserts a record entry starts with `---\ntype:` and contains `srs_id:`.
- `test_okf_export_index_md_first` — asserts `entries[0].path == "index.md"`.
- `test_okf_export_missing_container` — asserts `NotFound` error.
- `test_okf_export_slugify_filename` — asserts a record with label "My Decision!" produces filename `my-decision-<id8>.md`.

#### Milestone gate

1. All acceptance criteria above met.
2. All five named tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository okf
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Mark task checkboxes `[x]`, commit: `feat(srs-repository): OKF export service (#677)`.

---

### Phase 2: CLI command and payload

**Goal:** `srs render okf-bundle --container <id> --output <dir>` is wired up and the schema golden file is committed.

**Agent:** CLI Worker

#### Tasks

- [ ] Add `OkfBundlePayload` to `crates/srs-cli/src/payload.rs`:
  ```rust
  #[derive(Debug, Serialize, Deserialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct OkfBundlePayload {
      pub file_count: usize,
      pub output_dir: String,
      pub diagnostics: Vec<String>,
  }
  ```
- [ ] Add `OkfBundle` variant to the `RenderCommand` enum in `crates/srs-cli/src/commands/mod.rs`:
  ```rust
  /// Export a container as a Google OKF knowledge bundle (folder of markdown files)
  OkfBundle {
      /// Container UUID
      #[arg(long)]
      container: String,
      /// Output directory path (will be created if it does not exist)
      #[arg(long)]
      output: PathBuf,
  },
  ```
- [ ] Add handler `cmd_render_okf_bundle` in `crates/srs-cli/src/commands/render.rs`:
  ```rust
  fn cmd_render_okf_bundle(ctx: CliContext, container_id: String, output_dir: PathBuf) -> Result<String> {
      match with_store(&ctx, |store| {
          Ok(srs_repository::okf_export_service::export_okf_bundle(
              store,
              srs_repository::okf_export_service::OkfExportInput { container_id: container_id.clone() },
          )?)
      }) {
          Ok(bundle) => {
              std::fs::create_dir_all(&output_dir).map_err(|e| anyhow::anyhow!("cannot create output dir {:?}: {}", output_dir, e))?;
              for entry in &bundle.entries {
                  let dest = output_dir.join(&entry.path);
                  std::fs::write(&dest, entry.content.as_bytes()).map_err(|e| anyhow::anyhow!("failed to write {:?}: {}", dest, e))?;
              }
              output::serialize("render okf-bundle", OkfBundlePayload {
                  file_count: bundle.entries.len(),
                  output_dir: output_dir.to_string_lossy().into_owned(),
                  diagnostics: bundle.diagnostics,
              })
          }
          Err(e) => Ok(output::err("render okf-bundle", vec![e.to_string()])),
      }
  }
  ```
- [ ] Wire `RenderCommand::OkfBundle { container, output }` in the `dispatch` function in `render.rs`.
- [ ] Run `cargo run --bin generate-schemas` and commit the new `crates/srs-cli/schemas/payload/okf-bundle.json`.

#### Acceptance Criteria

- [ ] `cargo run --bin srs -- render okf-bundle --help` shows the command description and `--container` / `--output` flags.
- [ ] `cargo test --test payload_contracts` passes.
- [ ] The handler is ≤ 15 lines of non-trivial code (file I/O in the CLI layer is intentional glue — the directory write loop is acceptable here per the `cmd_render_export_bundle` precedent with `std::fs::File::create`).

#### Testing

```bash
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

Tests to write:
- `test_render_okf_bundle_payload_schema` — covered by `cargo test --test payload_contracts`.

#### Milestone gate

1. All acceptance criteria above met.
2. Run:
   ```bash
   cargo test --test payload_contracts
   cargo test -p srs-cli
   cargo clippy -- -D warnings
   ```
3. Mark task checkboxes `[x]`, commit: `feat(srs-cli): render okf-bundle command (#677)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `srs render okf-bundle --help` displays expected description
- [ ] A dogfood run on the `srs/srs` spec repository produces a valid OKF bundle (all entries have YAML frontmatter, `index.md` exists)
- [ ] All deferred items filed as follow-up issues

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Repository Worker must not touch `srs-cli` payload files.
- CLI Worker must not add business logic to the handler.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- The OKF v0.1 format requires at minimum `type:` in YAML frontmatter; `srs_id:` is a SRS-specific extension that aids round-tripping and is not forbidden by OKF.
- `srs_id:` in frontmatter is a safe extension — OKF is a "folder convention" format without strict schema enforcement.
- For members that are Notes (Tier 0/1), the `type` frontmatter value is `"note"`.
- The container title is taken from `container.title`; if empty, falls back to `container.name.unwrap_or("index")`.
- File naming collisions (two records with the same display label) are resolved by the instance_id suffix.
- No ZIP output: OKF is defined as a folder structure; the CLI writes files directly to disk.
