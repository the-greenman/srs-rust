# Plan: srs-gov export-decision → shareable bundle (srs-rust#289)

## Summary

Gate C of the attachments epic requires `srs-gov export-decision <id>` to produce a shareable
ZIP bundle: the decision's rendered document view (markdown) plus its linked attachment files.
All prerequisites are in place — `archive_pack`/`archive_unpack` (#276 closed), `resolve_document_view_attachments` (#286 merged) — but no export-bundle service or `srs-gov export-decision` command exists yet.
This plan delivers the service, a `srs render export-bundle` CLI command, and the srs-gov command, completing Gate C.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| CLI Worker | — |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `export_record_bundle` service lives in `srs-repository`, not in `srs-gov` or `srs-cli` | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | New `ExportBundlePayload` struct in `payload.rs`; generate-schemas run after | accepted |
| [ADR-033](../docs/adr/033-srs-archive-format.md) | Export bundle is a FLAT ZIP (doc + files), explicitly distinct from the `.srs` archive | accepted |
| [ADR-035](../docs/adr/035-flat-export-bundle-format.md) | Gate C bundle = flat ZIP: `decision.md` + `attachments/<filename>`. Not a re-importable `.srs` subset. | proposed |

No new ADRs for crate boundaries — this follows the existing `archive.rs` precedent (writer parameter pattern, `srs-repository` owns the logic).

---

## Contracts

### CLI output contract (ADR-011)

New command: `srs render export-bundle --view <view-id> --instance <id> --output <path>`

New payload struct in `crates/srs-cli/src/payload.rs`:

```rust
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExportBundlePayload {
    pub rendered_filename: String,
    pub attachment_count: usize,
    pub output_path: String,
    pub diagnostics: Vec<String>,
}
```

Run `cargo run --bin generate-schemas` after adding this struct and verify
`crates/srs-cli/schemas/payload/render-export-bundle.json` is created.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files under `srs/docs/schema/2.0/`. No action required.

---

## Scope

- Add `export_record_bundle(store, input, writer)` service to `crates/srs-repository/src/export_service.rs`.
- Add `ExportBundle` variant to `RenderCommand` enum in `crates/srs-cli/src/commands/mod.rs`.
- Add `cmd_render_export_bundle` handler in `crates/srs-cli/src/commands/render.rs`.
- Add `ExportBundlePayload` to `crates/srs-cli/src/payload.rs`; regenerate schemas.
- Add `Commands::ExportDecision` to `crates/srs-gov/src/main.rs` with `cmd_export_decision`.
- Write tests: `export_record_bundle` roundtrip (MemoryStore), empty-attachments case.
- Update `docs/dogfooding.md` with Gate C scenario (S-new: export a decision as a shareable bundle).

**Out of scope:**
- WASM binding for export-bundle (Phase 4 / #290–#292).
- `.srs` re-importable subset format (explicitly not Gate C; flat ZIP only).
- Streaming / chunked export for large repos.
- Sending the bundle email / sharing directly (presentation layer concern outside SRS).
- Re-importing the flat bundle into SRS (not round-trip in the SRS sense; "round-trip" in the epic means the ZIP unpacks correctly and the files are intact).
- Closing issues #287 / #288 (this PR closes only #289; the unmerged work from those issues is absorbed here with attribution).

---

## Phases

### Phase 1: `export_record_bundle` service in srs-repository

**Goal:** A tested, pure service function that renders a decision and its attachments into a flat ZIP, callable with a MemoryStore.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Create `crates/srs-repository/src/export_service.rs` with:
  - `pub struct ExportBundleInput { pub instance_id: String, pub view_id: String, pub format: Option<String> }`
  - `pub struct ExportBundleMetadata { pub rendered_filename: String, pub attachment_count: usize, pub diagnostics: Vec<String> }`
  - `pub fn export_record_bundle(store: &dyn RepositoryStore, input: ExportBundleInput, writer: impl Write + Seek) -> Result<ExportBundleMetadata, RepositoryError>`
- [ ] In `export_record_bundle` (required `use` statements at top of file: `use std::io::{Write, Seek}; use zip::{ZipWriter, CompressionMethod}; use zip::write::SimpleFileOptions;`):
  1. Call `render_document_view(RenderDocumentViewOptions { store, view_id: &input.view_id, format: input.format.as_deref(), theme_variant: None, container_id: None, instance_id_filter: Some(&input.instance_id) })` → returns `RenderResult { rendered: String, diagnostics: Vec<String>, ... }`. Capture `render_result.rendered` as the document bytes and `render_result.diagnostics` as the diagnostics to return.
  2. Call `resolve_document_view_attachments(store, ResolveDocumentViewAttachmentsInput { instance_ids: vec![input.instance_id.clone()] })` → returns `ResolveDocumentViewAttachmentsResult { source_documents_path, records }`. Note: this result has NO diagnostics field.
  3. Build `Vec<(String, Vec<u8>)>` (path, bytes) pairs:
     - `("decision.md".to_string(), render_result.rendered.into_bytes())`
     - Flatten via `attach_result.records.iter().flat_map(|r| r.attachments.iter())` to get `&ResolvedAttachment` entries. For each `a` where `a.content_path.is_some()`:
       - Compute `basename` = last path component of `a.content_path.unwrap()`.
       - Compute entry key = `format!("attachments/{}", basename)`.
       - **Collision check:** if `entries` already contains an entry with that key, use the full relative path instead: `format!("attachments/{}", a.content_path.as_ref().unwrap())`. This avoids silent overwrites when two attachments share a filename in different subdirectories.
       - Load bytes: `store.load_binary_file(&format!("{}/{}", attach_result.source_documents_path, a.content_path.as_ref().unwrap()))`.
       - Push `(entry_key, bytes)`.
  4. Sort the `(path, bytes)` pairs lexicographically by path (alphabetically `attachments/*` < `decision.md`).
  5. Create `let mut zip = ZipWriter::new(writer)`. Write each sorted entry with `CompressionMethod::Deflated` + `SimpleFileOptions::default().last_modified_time(zip::DateTime::default())`. After all entries, call `zip.finish().map_err(|e| RepositoryError::InvalidExportBundle { message: format!("failed to finalize ZIP: {}", e) })?;`
  6. Return `ExportBundleMetadata { rendered_filename: "decision.md".to_string(), attachment_count, diagnostics: render_result.diagnostics }` (diagnostics from the render step, not from attachments).
- [ ] Add `RepositoryError::InvalidExportBundle { message: String }` variant to `crates/srs-repository/src/error.rs` (follows `InvalidArchive` naming convention; for read/write failures during bundling).
- [ ] Re-export `export_service` from `crates/srs-repository/src/lib.rs`:
  ```rust
  pub mod export_service;
  pub use export_service::{ExportBundleInput, ExportBundleMetadata, export_record_bundle};
  ```

#### Acceptance Criteria

- [ ] `export_record_bundle` compiles against MemoryStore.
- [ ] `test_export_bundle_no_attachments`: renders a doc + empty attachment list → ZIP contains only `decision.md`.
- [ ] `test_export_bundle_with_attachments`: sets up a MemoryStore with a source-document binary, calls the service, verifies ZIP has `decision.md` + `attachments/<name>` with correct bytes.
- [ ] `RepositoryError::InvalidExportBundle` variant exists in `error.rs`.
- [ ] `test_export_bundle_cross_store_roundtrip`: packs via MemoryStore to a `tempfile::NamedTempFile`, re-opens with a zip reader, asserts `decision.md` is present and attachment bytes are byte-equal (required by CLAUDE.md Storage Boundary Rules).

#### Testing

```bash
cargo test -p srs-repository export_bundle
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write in `crates/srs-repository/src/export_service.rs` (inline `#[cfg(test)]`):

- `test_export_bundle_no_attachments` — MemoryStore with a type+view+record (no sourceRefs) → ZIP has exactly `decision.md`, no `attachments/` entries.
- `test_export_bundle_with_attachments` — MemoryStore with a record that has `sourceRefs: [{sourceRole:"attaches", sourceType:"repository-document", sourceId:"doc-abc"}]` + a matching source-document index entry + binary content → ZIP has `decision.md` + `attachments/<filename>` with byte-equal content.
- `test_export_bundle_cross_store_roundtrip` — packs via MemoryStore to a `tempfile::NamedTempFile`, opens the temp file with `zip::ZipArchive`, asserts `decision.md` is present by name, and asserts attachment file bytes are identical to the source bytes stored in the MemoryStore. Mirrors `test_archive_cross_store_roundtrip` in `archive.rs`.

#### Milestone gate

1. All acceptance criteria met.
2. Both named tests exist and pass: `cargo test -p srs-repository export_bundle`.
3. Clippy: `cargo clippy -p srs-repository -- -D warnings` (0 warnings).
4. Update plan checkboxes `[x]`.
5. Commit: `feat(repository): add export_record_bundle service (#289)`.

---

### Phase 2: CLI command `srs render export-bundle`

**Goal:** `srs render export-bundle --view <view-id> --instance <id> --output <path>` writes a flat ZIP to `--output` and prints the `ExportBundlePayload` JSON envelope.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-cli/src/commands/mod.rs`, add variant to `RenderCommand`:
  ```rust
  /// Export a decision as a shareable flat ZIP bundle (rendered doc + attachments)
  #[command(name = "export-bundle")]
  ExportBundle {
      /// DocumentView UUID to render the record against
      #[arg(long = "view")]
      view: String,
      /// Instance UUID of the record to export
      #[arg(long)]
      instance: String,
      /// Output path for the .zip bundle file
      #[arg(long)]
      output: PathBuf,
  },
  ```
- [ ] In `crates/srs-cli/src/commands/render.rs`, add to `dispatch` match and implement:
  ```rust
  RenderCommand::ExportBundle { view, instance, output } =>
      cmd_render_export_bundle(ctx, view, instance, output),
  ```
  Handler body (use plain `File`, not `BufWriter` — `ZipWriter` calls `seek()` frequently and `BufWriter::seek` flushes the buffer before delegating, negating the performance benefit; this matches the `archive_pack` caller pattern in `archive.rs`):
  ```rust
  fn cmd_render_export_bundle(ctx: CliContext, view_id: String, instance_id: String, output: PathBuf) -> Result<String> {
      use srs_repository::export_service::{export_record_bundle, ExportBundleInput};
      let mut file = std::fs::File::create(&output)
          .map_err(|e| anyhow::anyhow!("cannot create output file {:?}: {}", output, e))?;
      let meta = with_store(&ctx, |store| {
          Ok(export_record_bundle(store, ExportBundleInput {
              instance_id: instance_id.clone(),
              view_id: view_id.clone(),
              format: None,
          }, &mut file)?)
      })?;
      output::serialize("render export-bundle", ExportBundlePayload {
          rendered_filename: meta.rendered_filename,
          attachment_count: meta.attachment_count,
          output_path: output.to_string_lossy().into_owned(),
          diagnostics: meta.diagnostics,
      })
  }
  ```
- [ ] Add `ExportBundlePayload` struct to `crates/srs-cli/src/payload.rs` (payload structs are output-only — `Deserialize` is NOT derived, consistent with every other struct in `payload.rs`):
  ```rust
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct ExportBundlePayload {
      pub rendered_filename: String,
      pub attachment_count: usize,
      pub output_path: String,
      pub diagnostics: Vec<String>,
  }
  ```
- [ ] Run `cargo run --bin generate-schemas` and commit the new golden schema file `crates/srs-cli/schemas/payload/render-export-bundle.json`.

#### Acceptance Criteria

- [ ] `srs render export-bundle --view <id> --instance <id> --output /tmp/out.zip` creates a ZIP.
- [ ] JSON envelope key is `"render export-bundle"` and payload matches `ExportBundlePayload` shape.
- [ ] `cargo test --test payload_contracts` passes after schema regeneration.

#### Testing

```bash
cargo build --bin srs
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

Specific tests: no new integration test needed for the CLI handler (the service is tested in Phase 1; the CLI handler is thin I/O glue). The `payload_contracts` golden test validates the schema.

#### Milestone gate

1. All acceptance criteria met.
2. `cargo test --test payload_contracts` passes.
3. Clippy: `cargo clippy -p srs-cli -- -D warnings` (0 warnings).
4. Update plan checkboxes.
5. Commit: `feat(cli): add srs render export-bundle command (#289)` (includes generated schemas).

---

### Phase 3: `srs-gov export-decision <id>` command

**Goal:** `srs-gov export-decision <id>` discovers the decision-deliberation document view, calls `srs render export-bundle`, and shows a friendly success message.

**Agent:** CLI Worker (srs-gov)

#### Tasks

- [ ] Add to `Commands` enum in `crates/srs-gov/src/main.rs`:
  ```rust
  /// Export a decision as a shareable bundle (rendered doc + attachments)
  #[command(name = "export-decision")]
  ExportDecision {
      /// Instance ID (or unique prefix) of the decision to export
      id: String,
      /// Output path for the .zip bundle (default: ./<id-prefix>.zip)
      #[arg(long)]
      output: Option<String>,
  },
  ```
- [ ] Add `Some(Commands::ExportDecision { id, output }) => cmd_export_decision(&id, output.as_deref(), &cli.repo, cli.explain, cli.json),` to the `run()` match.
- [ ] Implement `cmd_export_decision` (explain mode pre-stages all three commands so the user sees all three underlying `srs` calls, not just the first):
  ```rust
  fn cmd_export_decision(id: &str, output: Option<&str>, repo: &str, explain: bool, json: bool) -> Result<()> {
      // In explain mode, print all three underlying srs commands and return.
      // Do NOT early-return after only the first run_srs call — all three must be shown.
      if explain {
          run_srs(&["record", "get", id], repo, true, false)?;
          run_srs(
              &["document-view", "list", "--namespace", "governance", "--name", "decision-deliberation"],
              repo, true, false,
          )?;
          let out_path = output.unwrap_or("<id-prefix>.zip");
          run_srs(
              &["render", "export-bundle", "--view", "<view-id>", "--instance", "<instance-id>", "--output", out_path],
              repo, true, false,
          )?;
          return Ok(());
      }

      // 1. Resolve the instance ID (may be a prefix): call `srs record get <id>`
      let record_payload = run_srs(&["record", "get", id], repo, false, json)?;
      if json { return Ok(()); }
      let instance_id = record_payload["record"]["instanceId"]
          .as_str()
          .ok_or_else(|| anyhow::anyhow!("record not found: {id}"))?
          .to_string();

      // 2. Discover the decision-deliberation document view by namespace+name
      let view_payload = run_srs(
          &["document-view", "list", "--namespace", "governance", "--name", "decision-deliberation"],
          repo, false, false,
      )?;
      let view_id = view_payload["documentViews"]
          .as_array()
          .and_then(|a| a.first())
          .and_then(|v| v["id"].as_str())
          .ok_or_else(|| anyhow::anyhow!(
              "decision-deliberation document view not found in repo {repo}. \
               Is the governance package installed?"
          ))?
          .to_string();

      // 3. Determine output path
      let out_path = output
          .map(|s| s.to_string())
          .unwrap_or_else(|| format!("{}.zip", &instance_id[..8.min(instance_id.len())]));

      // 4. Call srs render export-bundle
      let bundle_payload = run_srs(
          &["render", "export-bundle", "--view", &view_id, "--instance", &instance_id, "--output", &out_path],
          repo, false, false,
      )?;

      // 5. Render friendly output
      let rendered_filename = bundle_payload["renderedFilename"].as_str().unwrap_or("decision.md");
      let attachment_count = bundle_payload["attachmentCount"].as_u64().unwrap_or(0);
      render::export_bundle_created(&out_path, rendered_filename, attachment_count as usize);
      Ok(())
  }
  ```
- [ ] Add `render::export_bundle_created` function to `crates/srs-gov/src/render.rs`:
  ```rust
  pub fn export_bundle_created(output_path: &str, rendered_filename: &str, attachment_count: usize) {
      println!();
      println!("  Bundle created: {output_path}");
      println!("  Contents:");
      println!("    {rendered_filename}");
      if attachment_count > 0 {
          println!("    attachments/  ({attachment_count} file{})", if attachment_count == 1 { "" } else { "s" });
      }
      println!();
  }
  ```

#### Acceptance Criteria

- [ ] `srs-gov export-decision <id>` produces a `.zip` file in the current directory (default output path).
- [ ] `srs-gov export-decision <id> --output /tmp/bundle.zip` produces the ZIP at the specified path.
- [ ] Friendly output lists the bundle path and attachment count.
- [ ] `srs-gov export-decision <nonexistent-id>` fails with a clear error message.
- [ ] `srs-gov export-decision <id> --explain` prints the underlying `srs` commands without writing files.

#### Testing

```bash
cargo build --bin srs-gov
cargo test -p srs-gov
```

Specific tests: no new srs-gov unit test needed (the service and CLI layer are tested; srs-gov is thin orchestration). Manual dogfood scenario in Stage 7.6 proves correctness end-to-end.

#### Milestone gate

1. All acceptance criteria met.
2. `cargo test -p srs-gov` passes.
3. Clippy: `cargo clippy -p srs-gov -- -D warnings` (0 warnings).
4. Update plan checkboxes.
5. Commit: `feat(srs-gov): add export-decision command (#289)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (new ExportBundlePayload schema added)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `srs render export-bundle` command exists and writes a valid ZIP
- [ ] `srs-gov export-decision <id>` command exists and produces a bundle
- [ ] Gate C demo: a decision with attachments exports a ZIP containing `decision.md` + `attachments/<file>`
- [ ] ADR-035 status promoted from `proposed` to `accepted` in `docs/adr/035-flat-export-bundle-format.md` (matching ADR-033 governance precedent)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `zip::SimpleFileOptions` and `zip::DateTime::default()` are available (same dependency as `archive.rs`).
- `store.load_binary_file(path)` works on MemoryStore when binary content has been saved via `store.save_binary_file(path, bytes)`.
- The `document-view list --namespace <ns> --name <name>` flags exist on the CLI (confirmed in `crates/srs-cli/src/commands/document_view.rs`).
- The governance package seed defines a `governance/decision-deliberation` document view (confirmed: id `5a3ce87e-8340-4d91-a140-ab56b57f704f`).
- `render::export_bundle_created` follows the existing pattern in `crates/srs-gov/src/render.rs`.
