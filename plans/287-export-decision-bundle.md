# Plan: Export Decision Bundle — srs-rust#287

## Summary

RFC-017 (srs#101, merged 2026-07-17) defines the `.srs` archive and attachment model. Gate A (archive pack/unpack, #276) and Gate B (attach + explore, #283–#285) are complete. Gate C is the export surface: given a decision record's instance ID and a document view ID, produce a deterministic ZIP bundle containing the rendered document (md/html/json) plus all source-document attachment files linked to the records in that view. This is the CLI-first dogfood surface for `srs-gov export-decision`; `srs-web` parity is deferred to Gate D.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| CLI Worker | — |
| Verification Agent | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `export_decision_bundle` is a single service call with typed input/output; no business logic in CLI handler | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | `ExportDecisionBundlePayload` added to `payload.rs`; no `json!()` in handler | accepted |
| [ADR-033](../docs/adr/033-srs-archive-format.md) | Bundle uses deterministic ZIP (sorted entries, zeroed timestamps, Deflated) — same conventions as `.srs` but a different top-level layout | accepted |
| [ADR-034](../docs/adr/034-source-refs-in-record-extra.md) | Attachment resolution via `sourceRefs` in `record.extra` — `resolve_document_view_attachments` already handles this | accepted |
| ADR-035 (new) | Bundle format: flat export ZIP (document file + attachments/) rather than valid `.srs` subset. Decision pending human input — see Stage 2 design pause. | proposed |

---

## Contracts

### CLI output contract (ADR-011)

New `srs export decision-bundle` command adds one payload struct:

```rust
// crates/srs-cli/src/payload.rs
pub struct ExportDecisionBundlePayload {
    pub output_path: String,
    pub rendered_format: String,          // "md", "html", or "json"
    pub attachment_count: usize,
    pub bundle_entry_count: usize,
}
```

After adding this struct, run `cargo run --bin generate-schemas` and commit the new `schemas/payload/export_decision_bundle.json`.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files under `srs/docs/schema/2.0/`. No action required.

---

## Scope

- New `export_decision_bundle` service function in `crates/srs-repository/src/attachment_service.rs`
- New `ExportDecisionBundleInput` and `ExportDecisionBundleResult` types alongside it
- New top-level `Export` subcommand in `srs-cli` with `decision-bundle` variant
- New `commands/export.rs` handler file
- New `ExportDecisionBundlePayload` in `payload.rs` + regenerated golden schema
- Tests: happy path (MemoryStore roundtrip), determinism, no-attachments case, unknown-instance error

**Out of scope:**

- WASM binding for `export_decision_bundle` — follow-up issue (Gate D, #290 range)
- Web UI surface (`srs-web`) — deferred until Gate C passes (Gate D, #291 range)
- VS Code extension — follow-up
- Streaming/chunked export for large bundles
- Bundle determinism golden test (issue #288 — separate issue)
- `srs-gov dogfood: export-decision` TUI (issue #289 — separate issue)

---

## Phases

### Phase 1: Service layer — `export_decision_bundle`

**Goal:** A working, tested service function in `srs-repository` that accepts a typed input, renders the document view, resolves attachments, and writes a deterministic ZIP bundle to any `Write + Seek` implementor.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add `ExportDecisionBundleInput` struct to `crates/srs-repository/src/attachment_service.rs`:
  ```rust
  #[derive(Debug, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct ExportDecisionBundleInput {
      pub instance_id: String,
      pub view_id: String,
      #[serde(default)]
      pub format: Option<String>,       // "md" (default), "html", "json"
      #[serde(default)]
      pub theme_variant: Option<String>,
  }
  ```

- [ ] Add `ExportDecisionBundleResult` struct to `attachment_service.rs`:
  ```rust
  #[derive(Debug)]
  pub struct ExportDecisionBundleResult {
      pub rendered_format: String,      // actual format used
      pub attachment_count: usize,
      pub bundle_entry_count: usize,
  }
  ```

- [ ] Implement `export_decision_bundle` in `attachment_service.rs`:
  ```rust
  pub fn export_decision_bundle(
      store: &dyn RepositoryStore,
      input: ExportDecisionBundleInput,
      writer: impl std::io::Write + std::io::Seek,
  ) -> Result<ExportDecisionBundleResult, RepositoryError>
  ```

  Algorithm:
  1. Determine format: `input.format.as_deref().unwrap_or("md")`.
  2. Call `render_service::render_document_view(RenderDocumentViewOptions { store, view_id: &input.view_id, format: Some(&format), theme_variant: input.theme_variant.as_deref(), container_id: None, instance_id_filter: Some(&input.instance_id) })`. Map `RepositoryError` if the view_id or instance_id is not found.
  3. Collect `instance_ids` from `render_result.projection` (if present):
     ```rust
     let instance_ids: Vec<String> = render_result.projection.as_ref()
         .map(|p| p.sections.iter().flat_map(|s| s.records.iter().map(|r| r.instance_id.clone())).collect())
         .unwrap_or_default();
     ```
     If no projection (non-json format renders may omit it), fall back to `vec![input.instance_id.clone()]`.
  4. Call `resolve_document_view_attachments(store, ResolveDocumentViewAttachmentsInput { instance_ids })`.
  5. Determine document filename: `"document.md"` / `"document.html"` / `"document.json"` based on format.
  6. Collect `src_docs_base` from the resolve result.
  7. Build entries `Vec<(String, Vec<u8>)>`:
     - `(doc_filename, rendered_bytes)` — rendered text as UTF-8 bytes
     - For each `RecordAttachments` → each `ResolvedAttachment` with a `content_path`:
       - Load binary: `store.load_binary_file(&format!("{}/{}", src_docs_base, content_path))`
       - Push `(format!("attachments/{}", content_path), bytes)` — preserve subdir structure
  8. Deduplicate entries by ZIP path (same document may be linked from multiple records).
  9. Sort entries lexicographically (ADR-033 determinism rule).
  10. Write deterministic ZIP:
      ```rust
      let options = SimpleFileOptions::default()
          .compression_method(zip::CompressionMethod::Deflated)
          .last_modified_time(zip::DateTime::default());
      ```
  11. Return `ExportDecisionBundleResult { rendered_format: format, attachment_count, bundle_entry_count }`.

  Error cases:
  - `view_id` not found → `RepositoryError::NotFound { kind: "document-view", id: view_id }`
  - `instance_id` not found → no records in projection; still produce the bundle with rendered text only (do not error — the view may render a preamble even for unknown instances)
  - `content_path` file missing in store → skip the attachment and add a diagnostic (do not error the whole export)

- [ ] Write tests in `attachment_service.rs` `#[cfg(test)]` block:
  - `test_export_decision_bundle_happy_path` — set up MemoryStore with a document view, a linked attachment, call the service, open the ZIP, assert `document.md` present with rendered content, assert `attachments/brief.pdf` present with correct bytes
  - `test_export_decision_bundle_no_attachments` — no source refs on the record; bundle contains only `document.md`
  - `test_export_decision_bundle_determinism` — call twice, assert byte-identical ZIP
  - `test_export_decision_bundle_deduplicates_shared_attachment` — two records link to the same document; bundle has `attachments/brief.pdf` once

#### Acceptance Criteria

- [ ] `export_decision_bundle` compiles and is `pub` in `srs-repository`
- [ ] All four tests pass
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes
- [ ] The service calls `render_document_view` and `resolve_document_view_attachments` — no duplicated logic

#### Testing

```bash
cargo test -p srs-repository attachment_service
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `test_export_decision_bundle_happy_path` — full roundtrip with real content
- `test_export_decision_bundle_no_attachments` — bundle has 1 entry (document only)
- `test_export_decision_bundle_determinism` — byte-identical on two calls
- `test_export_decision_bundle_deduplicates_shared_attachment` — attachment appears once

#### Milestone gate

1. Verify all acceptance criteria above.
2. Confirm all four tests exist and pass.
3. Run lint and tests:

```bash
cargo test -p srs-repository attachment_service
cargo clippy -p srs-repository -- -D warnings
```

4. Mark completed checkboxes `[x]`.
5. Commit: `feat(repository): export_decision_bundle service (#287)`.

---

### Phase 2: CLI handler — `srs export decision-bundle`

**Goal:** A new top-level `srs export` subcommand with a `decision-bundle` variant that writes the bundle to a caller-specified output path and prints a JSON envelope with metadata.

**Agent:** CLI Worker

#### Tasks

- [ ] Add `ExportCommand` enum and `Export` variant to `SrsCommand` in `crates/srs-cli/src/commands/mod.rs`:
  ```rust
  #[derive(Subcommand)]
  pub enum ExportCommand {
      /// Export a decision and its linked attachments as a ZIP bundle
      DecisionBundle {
          /// Instance ID of the decision record to export
          instance_id: String,
          /// Document view ID to render (provides document structure and attachment resolution)
          view_id: String,
          /// Render format: md (default), html, json
          #[arg(long, default_value = "md")]
          format: String,
          /// Optional theme variant override
          #[arg(long)]
          theme_variant: Option<String>,
          /// Output path for the ZIP bundle (required)
          #[arg(long, short = 'o')]
          output: std::path::PathBuf,
      },
  }
  ```
  And in `SrsCommand`:
  ```rust
  /// Export commands: produce portable bundles from repository content
  #[command(subcommand)]
  Export(ExportCommand),
  ```

- [ ] Create `crates/srs-cli/src/commands/export.rs` with handler:
  ```rust
  use crate::commands::{with_store, CliContext, ExportCommand};
  use crate::output;
  use crate::payload::ExportDecisionBundlePayload;
  use anyhow::Result;
  use srs_repository::attachment_service::{self, ExportDecisionBundleInput};

  pub fn dispatch(ctx: CliContext, cmd: ExportCommand) -> Result<String> {
      match cmd {
          ExportCommand::DecisionBundle { instance_id, view_id, format, theme_variant, output } => {
              cmd_export_decision_bundle(ctx, instance_id, view_id, format, theme_variant, output)
          }
      }
  }

  fn cmd_export_decision_bundle(
      ctx: CliContext,
      instance_id: String,
      view_id: String,
      format: String,
      theme_variant: Option<String>,
      output_path: std::path::PathBuf,
  ) -> Result<String> {
      let mut file = std::fs::File::create(&output_path)
          .map_err(|e| anyhow::anyhow!("failed to create output file {:?}: {}", output_path, e))?;
      let input = ExportDecisionBundleInput { instance_id, view_id, format, theme_variant: theme_variant.clone() };
      let result = with_store(&ctx, |store| {
          Ok(attachment_service::export_decision_bundle(store, input, &mut file)?)
      })?;
      output::serialize(
          "export decision-bundle",
          ExportDecisionBundlePayload {
              output_path: output_path.display().to_string(),
              rendered_format: result.rendered_format,
              attachment_count: result.attachment_count,
              bundle_entry_count: result.bundle_entry_count,
          },
      )
  }
  ```

- [ ] Add `Export` dispatch arm to `main.rs` (or the top-level dispatch match):
  ```rust
  SrsCommand::Export(cmd) => commands::export::dispatch(ctx, cmd),
  ```

- [ ] Add `pub mod export;` to `crates/srs-cli/src/commands/mod.rs`.

- [ ] Add `ExportDecisionBundlePayload` struct to `crates/srs-cli/src/payload.rs`:
  ```rust
  // ── Export payloads ──────────────────────────────────────────────────────────
  #[derive(Debug, Serialize, Deserialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct ExportDecisionBundlePayload {
      pub output_path: String,
      pub rendered_format: String,
      pub attachment_count: usize,
      pub bundle_entry_count: usize,
  }
  ```

- [ ] Run `cargo run --bin generate-schemas` and stage the new `crates/srs-cli/schemas/payload/export_decision_bundle.json`.

- [ ] Verify `cargo test --test payload_contracts` passes.

#### Acceptance Criteria

- [ ] `srs export decision-bundle --instance <uuid> --view-id <vid> --output /tmp/out.zip` writes a ZIP to disk
- [ ] JSON envelope contains `outputPath`, `renderedFormat`, `attachmentCount`, `bundleEntryCount`
- [ ] Handler is ≤ 15 lines of logic (ADR-010)
- [ ] `cargo clippy -p srs-cli -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes

#### Testing

```bash
cargo build --bin srs
cargo clippy -p srs-cli -- -D warnings
cargo test --test payload_contracts
```

#### Milestone gate

1. Verify all acceptance criteria.
2. Run:
```bash
cargo build --bin srs
cargo clippy -p srs-cli -- -D warnings
cargo test --test payload_contracts
```
3. Mark completed checkboxes `[x]`.
4. Commit: `feat(cli): srs export decision-bundle command (#287)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schema changes in this plan)
- [ ] `srs export decision-bundle --instance <uuid> --view <vid> --output /tmp/bundle.zip` produces a ZIP
- [ ] ZIP contains `document.md` and (if attachments present) `attachments/<content_path>`
- [ ] Two consecutive calls produce byte-identical ZIP (determinism)
- [ ] Handler for `export decision-bundle` is ≤ 15 lines

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit.

## Assumptions

- `render_service::render_document_view` with `instance_id_filter` correctly filters to the single decision record.
- `resolve_document_view_attachments` correctly returns attachments for all records in the projection.
- The `RepositoryStore::load_binary_file` method is available and returns `Vec<u8>` (confirmed in `archive.rs`).
- MemoryStore supports `load_binary_file` for tests.
- The `zip` crate dependency is already in the workspace (added in plan #273).
- The service tests can construct a minimal MemoryStore with a fake DocumentView definition — if the MemoryStore cannot load package definitions needed by `render_document_view`, the happy-path test may need to use a fixture FileStore instead.
