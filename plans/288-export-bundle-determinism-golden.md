# Plan: Export-Bundle Determinism + Golden Test — srs-rust#288

## Summary

Gate C of the attachment epic (srs-rust#271) requires a deterministic export surface: `srs export
decision-bundle` produces a ZIP containing a rendered document view and its linked attachment files.
Issue #287 deferred the bundle format decision to this issue. This plan (a) settles that
architectural decision (ADR-035), (b) implements the `export_decision_bundle` service in
`srs-repository` and the CLI handler, and (c) adds a golden-fixture test that pins the byte output
to catch future format regressions — the same pattern used for `archive_pack` in #277.

This plan delivers the full Gate C export surface because #287 advanced only to a plan file
(commit `00733a1`); no implementation was written. All export functionality is in scope here so
that the Gate C milestone can be reviewed in one PR.

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
| [ADR-011](../docs/adr/011-cli-output-contract.md) | `ExportDecisionBundlePayload` added to `payload.rs`; `cargo run --bin generate-schemas` after change | accepted |
| [ADR-033](../docs/adr/033-srs-archive-format.md) | Determinism conventions (sorted entries, zeroed timestamps, Deflated) reused for the export bundle ZIP regardless of which format is chosen | accepted |
| [ADR-034](../docs/adr/034-source-refs-in-record-extra.md) | Attachment resolution uses `resolve_document_view_attachments` (sourceRefs-only, per RFC-017) | accepted |
| ADR-035 (new) | **Bundle format: flat export ZIP vs valid `.srs` archive subset** — see Design Decision Pause below | proposed |

---

## ⚠ Design Decision Pause (Stage 2)

Before the plan can be finalised, the bundle format must be chosen. This is a **long-term
architectural choice**: the format is the public wire contract that `srs-gov`, `srs-web`, and
external tools will consume. Reversing it later is a breaking change to `srs export
decision-bundle` output and to `docs/dogfooding.md` scenarios.

### Option A — Flat export ZIP

**Layout:**
```
document.md          ← rendered document view (md / html / json based on --format flag)
attachments/
  brief.pdf          ← source-document content files, preserving subdir structure
  reports/2026/x.pdf
```

**Pros:**
- Immediately human-readable with any ZIP/file tool — recipient does not need SRS to open it.
- Self-describing for the stated use case: share a decision with stakeholders.
- Simpler implementation: collect entries, sort, write ZIP with ADR-033 conventions.
- Smaller archive: no SRS structural overhead (manifest, package snapshot, relations).

**Cons:**
- Not re-importable as an SRS repository.
- Requires a separate ZIP construction path (though using the same determinism conventions as ADR-033 — sorted, zero timestamps, Deflated).
- The golden test must encode the flat layout.

### Option B — Valid `.srs` archive subset

**Layout:**
```
manifest.json        ← SRS manifest (single record in instanceIndex)
package/
  package.json
  package.snapshot.json
records/
  decisions/abc12345.json   ← the target decision record
relations/
  relations-collection.json ← only relations involving this record
source-documents/
  brief.pdf
  brief.meta.json
```

**Pros:**
- Re-importable into any SRS repository via `archive_unpack`.
- Reuses `archive_pack` almost directly (filtered snapshot with one record + its source docs).
- Single format for all SRS interchange: archive = export.
- Determinism already proven (ADR-033 + existing golden test).

**Cons:**
- Recipient needs to know the `.srs` format to use the structured data (not human-readable without SRS).
- The "rendered document view" is absent from the archive — the archive carries raw record JSON, not the rendered markdown/HTML. The rendered view must be fetched separately or added as an extra entry (which diverges from the archive format).
- Filtering a snapshot to a single record + relations is new complexity in the pack path.
- The conceptual mismatch: an export-to-share operation produces a backup archive format.

### Recommendation

**Option A (flat export ZIP)** is recommended. The stated purpose of Gate C is:
> "srs-gov export-decision → ZIP bundling the **rendered document view** + attachment files"

The rendered document is the primary artifact. Option B would carry the raw record JSON alongside source docs but no rendered view — which is the wrong output for the use case. A separate service call to render would add complexity. Option A delivers exactly what the gate criterion describes.

---

## Contracts

### CLI output contract (ADR-011)

New `srs export decision-bundle` command. New payload struct in `crates/srs-cli/src/payload.rs`:

```rust
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExportDecisionBundlePayload {
    pub output_path: String,
    pub rendered_format: String,   // "md", "html", or "json"
    pub attachment_count: usize,
    pub bundle_entry_count: usize,
}
```

After adding this struct: `cargo run --bin generate-schemas` → commit new
`crates/srs-cli/schemas/payload/export-decision-bundle.json`.

Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files under `srs/docs/schema/2.0/`. No action required.

---

## Scope

- `export_decision_bundle` service function in `crates/srs-repository/src/attachment_service.rs`
  - Typed input `ExportDecisionBundleInput` and result `ExportDecisionBundleResult`
  - Calls `render_document_view` → `resolve_document_view_attachments` → writes deterministic ZIP
- `srs export decision-bundle` CLI command in `crates/srs-cli/src/commands/export.rs`
  - New `ExportCommand` enum; dispatch in `commands/mod.rs` and `main.rs`
- `ExportDecisionBundlePayload` in `payload.rs` + regenerated golden schema
- Golden fixture test in `crates/srs-repository/tests/export_bundle_golden.rs`
  - Canonical store fixture (fixed UUIDs, pinned timestamps) → byte-stable golden `.zip`
  - Fixture stored at `crates/srs-repository/tests/fixtures/golden-export-bundle.zip`
  - Regeneration: `REGENERATE_GOLDEN=1 cargo test -p srs-repository -- test_export_bundle_golden_fixture`
- ADR-035 in `docs/adr/035-export-bundle-format.md`

**Out of scope:**

- WASM binding for `export_decision_bundle` — deferred to Gate D (#290 range)
- Web UI surface (`srs-web`) — deferred until Gate D
- VS Code extension — deferred
- Streaming / chunked export for large bundles
- `srs-gov` dogfood TUI command (`export-decision`) — separate issue #289
- `srs archive pack/unpack` CLI handlers (covered by a separate future issue)

---

## Phases

### Phase 1: Service layer — `export_decision_bundle`

**Goal:** A working, tested `export_decision_bundle` function in `srs-repository/src/attachment_service.rs` that renders a document view, resolves attachments, and writes a deterministic flat ZIP bundle.

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
      pub format: Option<String>,        // "md" (default), "html", "json"
      #[serde(default)]
      pub theme_variant: Option<String>,
  }
  ```

- [ ] Add `ExportDecisionBundleResult` struct to `attachment_service.rs`:
  ```rust
  #[derive(Debug)]
  pub struct ExportDecisionBundleResult {
      pub rendered_format: String,
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
  1. Determine format string: `let format = input.format.as_deref().unwrap_or("md").to_string();`
  2. Call `render_service::render_document_view(RenderDocumentViewOptions { store, view_id: &input.view_id, format: Some(&format), theme_variant: input.theme_variant.as_deref(), container_id: None, instance_id_filter: Some(&input.instance_id) })`. Propagate `RepositoryError` (view_id not found → `DocumentViewNotFound`).
  3. Collect instance IDs from the projection for attachment resolution:
     ```rust
     let instance_ids: Vec<String> = render_result.projection
         .as_ref()
         .map(|p| p.sections.iter()
             .flat_map(|s| s.records.iter().map(|r| r.instance_id.clone()))
             .collect())
         .unwrap_or_else(|| vec![input.instance_id.clone()]);
     ```
  4. Call `resolve_document_view_attachments(store, ResolveDocumentViewAttachmentsInput { instance_ids })`.
  5. Determine document filename: `"document.md"` / `"document.html"` / `"document.json"`.
  6. Collect `src_docs_base` from `resolve_result.source_documents_path`.
  7. Build entries `Vec<(String, Vec<u8>)>`:
     - `(doc_filename, render_result.rendered.into_bytes())` — the rendered text.
     - For each `RecordAttachments` → each `ResolvedAttachment` with `content_path = Some(cp)`:
       - `let store_path = format!("{}/{}", src_docs_base, cp);`
       - Match on `store.load_binary_file(&store_path)`:
         - `Ok(bytes)` → push `(format!("attachments/{}", cp), bytes)`.
         - `Err(_)` → skip (missing content is non-fatal; attachment metadata may index a file not yet on disk).
  8. Deduplicate entries by ZIP path: collect into a `BTreeMap<String, Vec<u8>>` (deduplication + implicit lexicographic sort in one step).
  9. Convert `BTreeMap` back to `Vec<(String, Vec<u8>)>` — already sorted.
  10. Write deterministic ZIP (per ADR-033 conventions):
      ```rust
      let options = SimpleFileOptions::default()
          .compression_method(zip::CompressionMethod::Deflated)
          .last_modified_time(zip::DateTime::default());
      let mut zip = zip::ZipWriter::new(writer);
      for (path, bytes) in &entries {
          zip.start_file(path, options)?;
          zip.write_all(bytes)?;
      }
      let _ = zip.finish()?;
      ```
  11. Compute counts: `attachment_count = entries.len() - 1` (all entries minus the document); `bundle_entry_count = entries.len()`.
  12. Return `ExportDecisionBundleResult { rendered_format: format, attachment_count, bundle_entry_count }`.

  Error mapping: wrap `zip::result::ZipError` as `RepositoryError::InvalidArchive { message }`.

- [ ] Add the following tests in `attachment_service.rs` `#[cfg(test)]` block. Use `MemoryStore` unless noted:

  - **`test_export_decision_bundle_happy_path`** — Set up a MemoryStore with a document view definition, one Tier-2 record with a `sourceRef` (role: attaches, type: repository-document, source_id: "test-doc-001"), and a source document indexed under "test-doc-001" at content_path "brief.pdf". Save binary content `b"PDF bytes"` at `source-documents/brief.pdf`. Call `export_decision_bundle`. Open the resulting ZIP (in-memory via `Cursor`). Assert `document.md` is present with the rendered text (non-empty). Assert `attachments/brief.pdf` is present with `b"PDF bytes"`.
  - **`test_export_decision_bundle_no_attachments`** — Same setup but no sourceRefs on the record. Assert ZIP has exactly 1 entry (`document.md`). `attachment_count == 0`, `bundle_entry_count == 1`.
  - **`test_export_decision_bundle_determinism`** — Call `export_decision_bundle` twice on the same store. Assert the resulting byte slices are identical.
  - **`test_export_decision_bundle_deduplicates_shared_attachment`** — Two records in the view share the same source document ("shared.pdf"). Assert `attachments/shared.pdf` appears exactly once in the ZIP.
  - **`test_export_decision_bundle_missing_content_skipped`** — Attachment is indexed but its binary file is absent from the store. Assert the call succeeds (no error). Assert ZIP has `document.md` but no `attachments/` entry for the missing file.

  Note: `RenderDocumentViewOptions::instance_id_filter` renders only the target record. Tests that need a real `render_document_view` call must supply a minimal package with a `DocumentView` definition. If MemoryStore cannot load packages needed by the render path, use a fixture-based FileStore. Check `render_service.rs` tests for the minimal package fixture pattern.

#### Acceptance Criteria

- [ ] `export_decision_bundle` compiles and is `pub` in `srs-repository`
- [ ] All five tests pass
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes
- [ ] Service calls `render_document_view` and `resolve_document_view_attachments` — no duplicated logic
- [ ] ZIP entries are sorted lexicographically and all timestamps are `zip::DateTime::default()`

#### Testing

```bash
cargo test -p srs-repository -- attachment_service
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `test_export_decision_bundle_happy_path` — roundtrip with real content
- `test_export_decision_bundle_no_attachments` — 1-entry bundle
- `test_export_decision_bundle_determinism` — byte-identical on two calls
- `test_export_decision_bundle_deduplicates_shared_attachment` — dedup
- `test_export_decision_bundle_missing_content_skipped` — graceful skip

#### Milestone gate

1. All five acceptance criteria checked.
2. Run:
```bash
cargo test -p srs-repository -- attachment_service
cargo clippy -p srs-repository -- -D warnings
```
3. Mark completed checkboxes `[x]`.
4. Commit: `feat(repository): export_decision_bundle service (#288)`.

---

### Phase 2: CLI handler — `srs export decision-bundle`

**Goal:** New `srs export decision-bundle` subcommand writes a bundle ZIP to a caller-specified output path and prints a JSON envelope with metadata.

**Agent:** CLI Worker

#### Tasks

- [ ] Add `ExportCommand` enum to `crates/srs-cli/src/commands/mod.rs`:
  ```rust
  #[derive(Subcommand)]
  pub enum ExportCommand {
      /// Export a decision and its linked attachments as a flat ZIP bundle
      DecisionBundle {
          /// Instance ID of the decision record
          instance_id: String,
          /// Document view ID to render
          view_id: String,
          /// Render format: md (default), html, json
          #[arg(long, default_value = "md")]
          format: String,
          /// Optional theme variant override
          #[arg(long)]
          theme_variant: Option<String>,
          /// Output path for the ZIP bundle
          #[arg(long, short = 'o')]
          output: std::path::PathBuf,
      },
  }
  ```
  Add `Export(ExportCommand)` variant to `SrsCommand` with docstring "Export commands: produce portable bundles from repository content".

- [ ] Create `crates/srs-cli/src/commands/export.rs`:
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
          .map_err(|e| anyhow::anyhow!("cannot create {}: {}", output_path.display(), e))?;
      let input = ExportDecisionBundleInput { instance_id, view_id,
          format: Some(format), theme_variant };
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

- [ ] Add `pub mod export;` to `crates/srs-cli/src/commands/mod.rs`.

- [ ] Wire `SrsCommand::Export(cmd)` → `commands::export::dispatch(ctx, cmd)` in `crates/srs-cli/src/main.rs`.

- [ ] Add `ExportDecisionBundlePayload` to `crates/srs-cli/src/payload.rs` under `// ── Export payloads ──`:
  ```rust
  #[derive(Debug, Serialize, Deserialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct ExportDecisionBundlePayload {
      pub output_path: String,
      pub rendered_format: String,
      pub attachment_count: usize,
      pub bundle_entry_count: usize,
  }
  ```

- [ ] Run `cargo run --bin generate-schemas` and stage the new
  `crates/srs-cli/schemas/payload/export-decision-bundle.json`.

- [ ] Verify `cargo test --test payload_contracts` passes.

#### Acceptance Criteria

- [ ] `srs export decision-bundle --instance-id <uuid> --view-id <vid> --output /tmp/bundle.zip` writes a ZIP to disk
- [ ] JSON envelope contains `outputPath`, `renderedFormat`, `attachmentCount`, `bundleEntryCount`
- [ ] Handler body ≤ 15 lines of logic (ADR-010)
- [ ] `cargo clippy -p srs-cli -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes

#### Testing

```bash
cargo build --bin srs
cargo clippy -p srs-cli -- -D warnings
cargo test --test payload_contracts
```

#### Milestone gate

1. All acceptance criteria checked.
2. Run:
```bash
cargo build --bin srs
cargo clippy -p srs-cli -- -D warnings
cargo test --test payload_contracts
```
3. Mark checkboxes `[x]`.
4. Commit: `feat(cli): srs export decision-bundle command (#288)`.

---

### Phase 3: Golden fixture test

**Goal:** A byte-stable golden test pins the export bundle format, catching unintended regressions in the same way `tests/archive_golden.rs` guards `archive_pack`.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Create `crates/srs-repository/tests/export_bundle_golden.rs`:

  ```rust
  //! Golden-fixture test for export_decision_bundle determinism.
  //!
  //! The golden file at tests/fixtures/golden-export-bundle.zip is the expected
  //! byte-for-byte output of export_decision_bundle on the canonical_export_store() defined here.
  //! Regenerate with:
  //!   REGENERATE_GOLDEN=1 cargo test -p srs-repository -- test_export_bundle_golden_fixture

  use srs_repository::attachment_service::{export_decision_bundle, ExportDecisionBundleInput};
  use srs_repository::{FileStore, RepositoryStore};
  // ... (imports for InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata)
  use std::io::Cursor;
  use std::path::Path;
  use tempfile::tempdir;
  ```

  The canonical store setup must:
  - Use `FileStore` with a `tempdir`
  - Use fixed, stable values for `repository_id`, `namespace`, `srs_version`, `title`, package `id/name/version`
  - Pin `createdAt` in the manifest extra to `"2026-01-01T00:00:00Z"` (same pattern as `archive_golden.rs`)
  - Add exactly one Tier-2 record with a fixed `instance_id` (e.g. `"golden-decision-00000000000000000000000000000001"`) under `records/decisions/golden-dec.json`
  - Write the record with `sourceRefs` pointing to `"golden-doc-0001"` (role: attaches)
  - Add a source-document index entry for `"golden-doc-0001"` with `content_path: "brief.pdf"` and `sidecar_path: "brief.meta.json"`, title `"Golden Brief"`, and fixed checksum values
  - Save a fixed binary content at `source-documents/brief.pdf`: `b"golden-pdf-content-fixture\n"`
  - Save the sidecar at `source-documents/brief.meta.json`: fixed JSON string
  - Add a package with a `DocumentView` definition that has a `view_id: "golden-view-001"` and a `ContainerSubset` section referencing the decisions type

  Note: because constructing a full package with a DocumentView is complex (requires the package JSON structure that `render_service` consumes), an alternative approach for the golden store is:
  - Write the record with explicit `sections` so `render_document_view` returns a non-empty rendered string from the default view
  - OR use the same `view_id` already present in the test fixture repos in `tests/fixtures/`

  Simpler fallback: use a MemoryStore and construct a raw rendered output directly — but the golden test should exercise the full service path. Check how `render_service` tests set up MemoryStore with packages.

  If the package setup is too complex for a byte-stable fixture in the golden test, use this alternative structure: call `export_decision_bundle` with a view_id that is known to return a minimal but non-empty rendered output. The golden test verifies byte stability, not semantic content.

- [ ] Add `test_export_bundle_golden_fixture` that:
  1. Calls the canonical store setup.
  2. Calls `export_decision_bundle` to a `Cursor<Vec<u8>>`.
  3. If `REGENERATE_GOLDEN=1`, writes to `tests/fixtures/golden-export-bundle.zip` and returns.
  4. Otherwise, reads the fixture and asserts `actual == expected`.

- [ ] Add `test_export_bundle_golden_roundtrip` that:
  1. Reads `tests/fixtures/golden-export-bundle.zip`.
  2. Opens the ZIP.
  3. Asserts `document.md` is present and non-empty.
  4. Asserts `attachments/brief.pdf` is present with content `b"golden-pdf-content-fixture\n"`.

- [ ] Generate the initial golden fixture:
  ```bash
  REGENERATE_GOLDEN=1 cargo test -p srs-repository -- test_export_bundle_golden_fixture
  ```
  Commit `tests/fixtures/golden-export-bundle.zip`.

- [ ] Add `"tests/fixtures/golden-export-bundle.zip"` to `.gitattributes` as a binary file
  (same entry pattern as `golden-archive.srs` if already present).

#### Acceptance Criteria

- [ ] `test_export_bundle_golden_fixture` passes (actual == golden on second run)
- [ ] `test_export_bundle_golden_roundtrip` passes
- [ ] The `.zip` file is committed to `tests/fixtures/`
- [ ] `cargo test -p srs-repository` passes with no failures

#### Testing

```bash
REGENERATE_GOLDEN=1 cargo test -p srs-repository -- test_export_bundle_golden_fixture
cargo test -p srs-repository -- export_bundle
```

Specific tests:
- `test_export_bundle_golden_fixture` — byte-stable output
- `test_export_bundle_golden_roundtrip` — ZIP structure correct

#### Milestone gate

1. All acceptance criteria checked.
2. Run:
```bash
cargo test -p srs-repository -- export_bundle
cargo clippy -p srs-repository -- -D warnings
```
3. Mark checkboxes `[x]`.
4. Commit: `test(repository): golden fixture for export_decision_bundle (#288)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schema changes)
- [ ] `srs export decision-bundle --instance-id <uuid> --view-id <vid> --output /tmp/bundle.zip` produces a flat ZIP
- [ ] ZIP contains `document.md` and `attachments/<content_path>` entries
- [ ] Two consecutive calls produce byte-identical output (determinism)
- [ ] `test_export_bundle_golden_fixture` passes
- [ ] Handler for `export decision-bundle` is ≤ 15 lines (ADR-010)
- [ ] ADR-035 committed to `docs/adr/035-export-bundle-format.md` with status: accepted

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit.

## Assumptions

- `render_service::render_document_view` with `instance_id_filter` is available on the master branch (it is — merged in #286).
- `resolve_document_view_attachments` is available on master (it is — merged in #286).
- `store.load_binary_file` is available on `RepositoryStore` (confirmed in `archive.rs`).
- MemoryStore supports `load_binary_file` for tests (confirmed in `store.rs` tests).
- `zip` workspace dependency is already present (added in #273).
- The golden fixture render path may require a minimal package definition with a `DocumentView`. If MemoryStore cannot load that structure without a real package JSON, the golden test will use a FileStore pointing at a temp repo with the package definition written directly as JSON.
- The bundle format decision (ADR-035) is settled before Phase 1 begins — this plan defaults to Option A (flat export ZIP) per the Stage 2 design pause; the implementor should not proceed until the user confirms.
