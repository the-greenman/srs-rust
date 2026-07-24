# Plan: OKF export projection — render a container as a Google Open Knowledge Format bundle

> Issue: srs-rust#677
> Alignment register: item 2 — NOW, weight 80 (R4 × F5 × C4)

## Summary

Add a new CLI command `srs render okf-bundle --container <id> --output <dir>` that exports an SRS container as a Google Open Knowledge Format (OKF v0.1) knowledge bundle — one markdown file per record with YAML frontmatter (`type` from the record's Type name) plus an `index.md` generated from the container's navigation (precedes chain).

The service (`srs-repository`) returns typed structured data per entry; the CLI layer renders entries to YAML frontmatter + markdown and writes files to disk. This separation keeps format-specific rendering in the client per ADR-010. Export only; import is deferred.

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
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service returns typed structured `OkfEntry` data; CLI layer renders markdown and writes files — format-specific rendering must not live in the service (render-vs-project boundary) | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | CLI handler is arg-parse → one service call → write helper → `output::serialize`; payload struct in `payload.rs` | accepted |
| [ADR-038](../docs/adr/038-vfs-seam.md) | All repository storage I/O routes through the Vfs seam; no `std::fs` in service code | accepted |
| [ADR-041](../docs/adr/041-instance-persistence.md) | Service functions access instances via typed logical-id methods only; `InstanceIndexEntry.path` is adapter-private | accepted |
| [ADR-042](../docs/adr/042-typed-logical-id-methods.md) | `LoadedInstance` and `get_instance_by_id` are the canonical types for mixed-tier instance access | accepted |
| Alignment register item 2 | OKF export is an adapter-layer projection, no new semantics; thin and disposable | cited |
| No new ADR | All decisions follow from ADR-010, ADR-011, ADR-038, ADR-041, ADR-042, and the capability-layering guide. The OKF renderer is a thin service that produces structured data; markdown rendering is a client concern. | — |

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
- New structs `OkfExportInput`, `OkfEntry`, `OkfBundle` in the same file
- New CLI command `srs render okf-bundle --container <id> --output <dir>`
- New helper `write_okf_bundle_to_dir` in `crates/srs-cli/src/commands/render.rs` (renders markdown and writes files)
- New payload struct `OkfBundlePayload` in `crates/srs-cli/src/payload.rs`
- Regenerated `crates/srs-cli/schemas/payload/okf-bundle.json`
- Service uses precedes-chain ordering and the existing `record_label` module for headings
- Service returns structured `OkfEntry` data; CLI renders YAML frontmatter + markdown
- YAML frontmatter contains `type:` (record's `type_name`) and `srs_id:` (`instance_id` for traceability)

**Out of scope:**

- OKF import (bundle → Tier-0/Tier-1 records) — deferred until format shows 2+ quarters of life; a follow-up issue must be filed before this plan is merged
- WASM binding — deferred; a concrete follow-up issue must be filed and linked under the parent story before this plan is merged
- Using a DocumentView for prose rendering — self-contained renderer is intentional; a `--view` option is a follow-up
- OKF content-blob attachments — OKF v0.1 does not define attachment semantics; deferred
- Support for containers-of-containers (nested OKF structure) — deferred

---

## Phases

### Phase 1: OKF export service in srs-repository

**Goal:** `export_okf_bundle` service is implemented, returns typed structured `OkfEntry` data (not rendered markdown), and all unit tests pass.

**Agent:** Repository Worker

#### Tasks

- [ ] Create `crates/srs-repository/src/okf_export_service.rs` with these types:

  ```rust
  pub struct OkfExportInput {
      pub container_id: String,
  }

  pub struct OkfEntry {
      pub path: String,           // slugified filename, e.g. "my-decision-a1b2c3d4.md"
      pub display_label: String,  // resolved display label for this entry
      pub instance_id: String,    // for `srs_id:` frontmatter field
      pub type_label: String,     // OKF `type:` frontmatter value
      /// Non-null field values for Tier-2 records, as (field_name_or_id, value) pairs
      pub field_pairs: Vec<(String, serde_json::Value)>,
      /// Body text for Tier-0 notes; None for Tier-2 records
      pub note_text: Option<String>,
  }

  pub struct OkfBundle {
      pub container_title: String,  // for the index.md heading
      pub entries: Vec<OkfEntry>,
      pub diagnostics: Vec<String>,
  }
  ```

- [ ] Implement `pub fn export_okf_bundle(store: &dyn RepositoryStore, input: OkfExportInput) -> Result<OkfBundle, RepositoryError>`:

  1. Load container: `container_service::get_container(store, &input.container_id)?`. If the container is missing, return `Err(RepositoryError::ContainerNotFound { container_id: input.container_id.clone() })`.
  2. Compute container title: `if !container.title.is_empty() { container.title.clone() } else if let Some(n) = &container.name { n.clone() } else { "index".to_string() }`.
  3. Get member IDs: `container_service::list_container_members(store, &input.container_id)?`.
  4. Load instances: for each `member_id` in member_ids, call `record_store::get_instance_by_id(store, &member_id)` and match:
     - `Ok(Some(instance))` → push `instance` to `instances` vec
     - `Ok(None)` → push `format!("member {} not found in instance index", member_id)` to `diagnostics`; skip
     - `Err(e)` → push `format!("member {} failed to load: {}", member_id, e)` to `diagnostics`; skip
  5. Build label indexes: `record_label::build_label_indexes(store)?` → `(field_name_index, identity_field_index)`.
  6. Load relations: `relation_service::load_relations(store)?` → `Vec<Relation>`.
  7. Sort instances by precedes chain: `relation_graph::sort_by_precedes_chain(instances, &relations)`.
  8. For each sorted instance, call `okf_entry_from_instance(&instance, &field_name_index, &identity_field_index)` → push `OkfEntry` to `entries`.
  9. Return `Ok(OkfBundle { container_title, entries, diagnostics })`.

- [ ] Implement `fn okf_entry_from_instance(instance: &LoadedInstance, fni: &FieldNameIndex, ifi: &IdentityFieldIndex) -> OkfEntry`:

  - `(display_label, type_label, instance_id, field_pairs, note_text)` by matching `instance`:
    - `LoadedInstance::Record(r)`:
      - `display_label = record_label::record_display_label(r, ifi, fni)`
      - `type_label = r.type_name.clone()`
      - `instance_id = r.instance_id.clone()`
      - `field_pairs`: collect `r.field_values.iter().filter_map(|fv| { if fv.value.is_null() { return None; } let name = fni.get(&fv.field_id).cloned().unwrap_or_else(|| fv.field_id.clone()); Some((name, fv.value.clone())) })` → `Vec<(String, serde_json::Value)>`
      - `note_text = None`
    - `LoadedInstance::Note(n)`:
      - `display_label`: `n.title.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| s[..40.min(s.len())].to_string()).unwrap_or_else(|| n.instance_id[..8.min(n.instance_id.len())].to_string())`
      - `type_label = "note".to_string()`
      - `instance_id = n.instance_id.clone()`
      - `field_pairs = vec![]`
      - `note_text = Some(n.sections.iter().map(|s| s.content.as_str()).collect::<Vec<_>>().join("\n\n"))` — concatenated section contents
  - Compute `path`:
    - `let slug = writer::slugify_instance_name(&display_label);`
    - `let id_suffix = &instance_id[..8.min(instance_id.len())];`
    - `if slug.is_empty() { format!("{}.md", id_suffix) } else { format!("{}-{}.md", slug, id_suffix) }`
  - Return `OkfEntry { path, display_label, instance_id, type_label, field_pairs, note_text }`.

- [ ] Add `pub mod okf_export_service;` to `crates/srs-repository/src/lib.rs`.
- [ ] Add `pub use okf_export_service::{export_okf_bundle, OkfBundle, OkfEntry, OkfExportInput};` to the pub re-exports in `lib.rs`.

#### Acceptance Criteria

- [ ] `export_okf_bundle` on a 3-record container with a precedes chain A→B→C returns `entries` with `display_label` in order A, B, C.
- [ ] Each `OkfEntry` for a Tier-2 record has `type_label == record.type_name`.
- [ ] Missing container returns `Err(RepositoryError::ContainerNotFound { .. })`.
- [ ] A member in the index but with no matching instance file accumulates a diagnostic and is skipped (entry count is reduced).
- [ ] `OkfEntry.field_pairs` contains resolved field names (not raw field IDs) for a record with a known field.
- [ ] Works on `MemoryStore` (no `FileStore`-only I/O paths).

#### Testing

```bash
cargo test -p srs-repository okf
cargo clippy -p srs-repository -- -D warnings
```

Tests to write in `crates/srs-repository/src/okf_export_service.rs` `#[cfg(test)]` block:

- `test_okf_export_three_records_precedes_order` — builds `MemoryStore` with 3 Tier-2 records connected by `precedes` relations A→B→C; asserts `entries[0].display_label == "A"`, `[1] == "B"`, `[2] == "C"`.
- `test_okf_entry_type_label` — asserts a Tier-2 record entry has `type_label == record.type_name`.
- `test_okf_export_missing_container` — asserts `Err(RepositoryError::ContainerNotFound { .. })`.
- `test_okf_export_member_not_found_diagnostic` — member ID in index but no matching instance → diagnostic accumulated, entry skipped, `entries.len()` is reduced.
- `test_okf_export_slugify_filename` — record with display label `"My Decision!"` produces `entry.path` matching `my-decision-<id8>.md`.
- `test_okf_export_roundtrip_stores` — same 3-record container built on `MemoryStore`, serialized to a temp `FileStore`, re-exported: both stores return equal `entries` (same paths, labels, type_labels in same order).

#### Milestone gate

1. All six acceptance criteria above met.
2. All six named tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository okf
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Mark task checkboxes `[x]`, commit: `feat(srs-repository): OKF export service (#677)`.

---

### Phase 2: CLI command and payload

**Goal:** `srs render okf-bundle --container <id> --output <dir>` is wired up; the schema golden file is committed; markdown rendering and file writes live in a helper in `commands/render.rs`, not in the handler.

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

- [ ] Add helper function in `crates/srs-cli/src/commands/render.rs` — `fn write_okf_bundle_to_dir(bundle: &srs_repository::OkfBundle, dir: &Path) -> anyhow::Result<usize>`:

  - `std::fs::create_dir_all(dir)?`
  - For each `entry` in `bundle.entries`:
    - Render markdown: `format!("---\ntype: {}\nsrs_id: {}\n---\n\n# {}\n\n{}", entry.type_label, entry.instance_id, entry.display_label, body)` where:
      - `body` = if `entry.note_text.is_some()`: `entry.note_text.as_deref().unwrap_or("")` else: `entry.field_pairs.iter().map(|(k, v)| format!("**{}**: {}\n\n", k, v)).collect::<String>()`
    - `std::fs::write(dir.join(&entry.path), rendered.as_bytes())?`
  - Render `index.md`:
    - `let index_content = format!("# {}\n\n", bundle.container_title) + &bundle.entries.iter().map(|e| format!("- [{}]({})\n", e.display_label, e.path)).collect::<String>()`
    - `std::fs::write(dir.join("index.md"), index_content.as_bytes())?`
  - Return `Ok(bundle.entries.len() + 1)` (entries + index.md)

- [ ] Add handler `fn cmd_render_okf_bundle(ctx: CliContext, container_id: String, output_dir: PathBuf) -> Result<String>` in `commands/render.rs`:

  ```rust
  fn cmd_render_okf_bundle(ctx: CliContext, container_id: String, output_dir: PathBuf) -> Result<String> {
      let bundle = with_store(&ctx, |store| {
          Ok(srs_repository::export_okf_bundle(
              store,
              srs_repository::OkfExportInput { container_id },
          )?)
      })?;
      let file_count = write_okf_bundle_to_dir(&bundle, &output_dir)?;
      Ok(output::serialize("render okf-bundle", OkfBundlePayload {
          file_count,
          output_dir: output_dir.to_string_lossy().into_owned(),
          diagnostics: bundle.diagnostics,
      })?)
  }
  ```

  The handler is ≤ 10 lines with no business logic — arg mapping + one service call + one write helper + output::serialize.

- [ ] Wire `RenderCommand::OkfBundle { container, output }` in the `dispatch` function in `render.rs`.
- [ ] Run `cargo run --bin generate-schemas` and commit the new `crates/srs-cli/schemas/payload/okf-bundle.json`.

#### Acceptance Criteria

- [ ] `cargo run --bin srs -- render okf-bundle --help` shows the command description and `--container` / `--output` flags.
- [ ] `cargo test --test payload_contracts` passes.
- [ ] The handler (`cmd_render_okf_bundle`) is ≤ 10 lines and contains no business logic — only arg mapping, one service call, one helper call, `output::serialize`.
- [ ] The file-write loop and markdown rendering live in `write_okf_bundle_to_dir`, not in the handler.

#### Testing

```bash
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

Tests to write:
- `test_render_okf_bundle_payload_schema` — covered by `cargo test --test payload_contracts`.
- `test_cmd_render_okf_bundle_writes_files` — integration test (in `tests/` or `#[cfg(test)]` in `render.rs`) that calls `cmd_render_okf_bundle` with a temp directory, verifies `index.md` and at least one entry file are written, and asserts each entry file starts with `---\ntype:` and `index.md` starts with `# `.

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

### Phase 3: File deferred follow-up issues

**Goal:** Both deferred items (OKF import and WASM binding) have GitHub issues filed and linked under their parent story before this plan is merged.

**Agent:** Lead Integrator

#### Tasks

- [ ] File follow-up issue: "OKF import: ingest a knowledge bundle into SRS records" — label `enhancement`, body explains deferred scope and why (no OKF spec stability, no known consumers yet). Then link to parent story:
  ```bash
  node /tmp/gh-project.mjs link srs-rust#677 srs-rust#<new-import-issue>
  ```
- [ ] File follow-up issue: "WASM binding for export_okf_bundle" — label `enhancement`, body explains CLI-first rationale and what the future plan needs. Then link to parent story:
  ```bash
  node /tmp/gh-project.mjs link srs-rust#677 srs-rust#<new-wasm-issue>
  ```
- [ ] Record both issue numbers in the Final Acceptance checklist below.

#### Acceptance Criteria

- [ ] OKF import follow-up issue filed with clear explanation of deferred scope and linked under parent story
- [ ] WASM binding follow-up issue filed with clear explanation of deferred scope and linked under parent story
- [ ] Both issue numbers recorded in Final Acceptance (replacing `#___` placeholders)

#### Milestone gate

All three acceptance criteria met. Add issue numbers to Final Acceptance before proceeding.

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `srs render okf-bundle --help` displays expected description
- [ ] A dogfood run on the `srs/srs` spec repository produces a valid OKF bundle: all entries have YAML frontmatter, `index.md` exists with a link list
- [ ] Follow-up issue for OKF import is filed and linked (issue #___)
- [ ] Follow-up issue for WASM binding is filed and linked (issue #___)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Repository Worker must not touch `srs-cli` payload files or `commands/` files.
- CLI Worker must not add business logic to the handler; markdown rendering lives in `write_okf_bundle_to_dir`, not in the handler.
- Verification Agent runs after each major phase and before final sign-off.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- The OKF v0.1 format requires at minimum `type:` in YAML frontmatter; `srs_id:` is a SRS-specific extension that aids round-tripping and is not forbidden by OKF.
- `srs_id:` in frontmatter is a safe extension — OKF is a "folder convention" format without strict schema enforcement.
- For members that are Notes (Tier 0/1), the `type` frontmatter value is `"note"`.
- The container title is taken from `container.title`; if empty, falls back to `container.name` if set; else `"index"`.
- File naming collisions (two records with the same display label) are resolved by the `instance_id` suffix in the filename.
- No ZIP output: OKF is defined as a folder structure; the CLI writes files directly to disk.
- `writer::slugify_instance_name` (pub(crate) in srs-repository) is the canonical slug function — no new slugify implementation.
- `relation_service::load_relations` (pub(crate) in srs-repository) is the correct call for loading all relations within the service.
