# Plan: srs attachment add (store file + write .meta.json sidecar)

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

`srs attachment list` (#279) already exists. This plan adds `srs attachment add`, which copies a source file into the repository's `source-documents/` directory, writes a `.meta.json` sidecar with document metadata (documentId, contentPath, contentType, encoding, checksum), and records a `SourceDocumentIndexEntry` in `manifest.sourceDocumentIndex`. The command supports an optional `--subdir` target and is the entry-point for Gate A of epic #271.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | (self) |
| Repository Service Worker | (self) |
| CLI Worker | (self) |
| Verification | (self) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan implements ADR-007 (file-before-index ordering), ADR-010 (service boundary contract), and ADR-011 (CLI output contract). SHA-256 checksum computation is added as a `sha2 + hex` workspace dep (pure-Rust, wasm32-safe per ADR-013 constraint).

| ADR | Decision | Status |
|---|---|---|
| [ADR-007](../docs/adr/007-file-index-io-ordering.md) | Write content file first, sidecar file second, then update manifest index | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service logic in `srs-repository`, typed input/output structs, no business logic in CLI handler | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Named payload struct + golden schema | accepted |

---

## Contracts

### CLI output contract (ADR-011)

**New command added** — `srs attachment add` is a new command. One new named struct will be added to `crates/srs-cli/src/payload.rs`:

- `AttachmentAddPayload` — the `attachment add` envelope

After adding: `cargo run --bin generate-schemas` generates `crates/srs-cli/schemas/payload/attachment-add.json`. Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No — this plan does not modify `srs/docs/schema/2.0/` entity schemas.

---

## Scope

- `add_attachment(store, input)` service in `crates/srs-repository/src/attachment_service.rs`
- `AddAttachmentInput` and `AddAttachmentResult` structs alongside the service function
- `sha2 = "0.10"` + `hex = "0.4"` added to workspace `Cargo.toml` and `srs-repository/Cargo.toml`
- `AttachmentAddPayload` struct in `crates/srs-cli/src/payload.rs`
- `Add` variant in `AttachmentCommand` enum in `crates/srs-cli/src/commands/mod.rs`
- `cmd_attachment_add` handler in `crates/srs-cli/src/commands/attachment.rs`
- Golden schema `crates/srs-cli/schemas/payload/attachment-add.json`
- Unit tests in `attachment_service.rs` using `MemoryStore` + at least one `FileStore` roundtrip

**Out of scope:**
- `attachment remove` — future command
- WASM binding for `add_attachment` — deferred to a follow-up (Phase 4)
- Detecting duplicate files by content hash — deferred
- `--overwrite` / `--force` flag — initial version returns an error if the file already exists
- `srs attachment link` (`attaches` relation + sourceRefs) — issue #283

---

## Phases

### Phase 1: Dependencies + service

**Goal:** `add_attachment` compiles and all unit tests pass.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add to workspace `Cargo.toml` `[workspace.dependencies]`:
  ```toml
  sha2 = { version = "0.10", default-features = false }
  hex = "0.4"
  ```

- [ ] Add to `crates/srs-repository/Cargo.toml` `[dependencies]`:
  ```toml
  sha2 = { workspace = true }
  hex = { workspace = true }
  ```

- [ ] Add `use` imports in `attachment_service.rs`:
  ```rust
  use sha2::{Digest, Sha256};
  ```

- [ ] Add to `crates/srs-repository/src/attachment_service.rs` after the existing `list_attachments` code:

  ```rust
  // ── add_attachment ────────────────────────────────────────────────────────────

  /// Input for `add_attachment`. The CLI reads the source file from disk and passes
  /// its bytes here; the service never touches the local filesystem directly.
  pub struct AddAttachmentInput {
      /// Original filename (e.g. `"brief.pdf"`), used to derive `content_path` and `sidecar_path`.
      pub file_name: String,
      /// Raw bytes of the file to store.
      pub content: Vec<u8>,
      /// Optional subdirectory within `source-documents/` (e.g. `"phase-1"`).
      pub subdir: Option<String>,
      /// Optional human-readable title stored in the sidecar and manifest index.
      pub title: Option<String>,
      /// MIME type (e.g. `"application/pdf"`). Auto-detected from file extension if `None`.
      pub content_type: Option<String>,
  }

  pub struct AddAttachmentResult {
      /// Generated UUID for this document.
      pub document_id: String,
      /// Path of the content file relative to `source-documents/`.
      pub content_path: String,
      /// Path of the sidecar file relative to `source-documents/`.
      pub sidecar_path: String,
      /// Resolved `source-documents/` base dir (from manifest or default).
      pub source_documents_path: String,
      /// `sha256:<hex>` checksum of the content file.
      pub content_checksum: String,
      /// `sha256:<hex>` checksum of the sidecar JSON bytes.
      pub sidecar_checksum: String,
  }

  /// Infer a MIME type from a file extension; falls back to `"application/octet-stream"`.
  fn infer_content_type(file_name: &str) -> &'static str {
      match file_name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
          "pdf"  => "application/pdf",
          "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
          "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
          "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
          "txt"  => "text/plain",
          "md"   => "text/markdown",
          "html" | "htm" => "text/html",
          "png"  => "image/png",
          "jpg" | "jpeg" => "image/jpeg",
          "gif"  => "image/gif",
          "svg"  => "image/svg+xml",
          "zip"  => "application/zip",
          "json" => "application/json",
          "csv"  => "text/csv",
          _      => "application/octet-stream",
      }
  }

  fn sha256_hex(data: &[u8]) -> String {
      let hash = Sha256::digest(data);
      format!("sha256:{}", hex::encode(hash))
  }

  /// Store `input.content` under `source-documents/<[subdir/]file_name>`, write its
  /// `.meta.json` sidecar, and append a `SourceDocumentIndexEntry` to the manifest.
  ///
  /// Write order (ADR-007 file-before-index for create):
  ///   1. Write content file
  ///   2. Write sidecar file
  ///   3. Update and save manifest index
  ///
  /// Returns an error if the content file already exists.
  pub fn add_attachment(
      store: &dyn RepositoryStore,
      input: AddAttachmentInput,
  ) -> Result<AddAttachmentResult, RepositoryError> {
      // Validate the filename: must be non-empty, no path separators.
      let file_name = input.file_name.trim().to_string();
      if file_name.is_empty() {
          return Err(RepositoryError::InvalidInput {
              message: "file_name must not be empty".to_string(),
          });
      }
      if file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
          return Err(RepositoryError::InvalidInput {
              message: format!("file_name must not contain path separators: {file_name}"),
          });
      }

      // Validate subdir: if provided, must not be absolute and must not contain "..".
      if let Some(ref sub) = input.subdir {
          let sub = sub.trim();
          if sub.starts_with('/') || sub.starts_with('\\') || sub.contains("..") {
              return Err(RepositoryError::InvalidInput {
                  message: format!("subdir must be a relative path without '..': {sub}"),
              });
          }
      }

      let manifest = store.load_manifest()?;
      let src_docs_base = manifest
          .source_documents_path
          .as_deref()
          .unwrap_or("source-documents")
          .to_string();

      // Derive paths relative to src_docs_base.
      let rel_content_path = match &input.subdir {
          Some(sub) => {
              let sub = sub.trim().trim_matches('/');
              if sub.is_empty() {
                  file_name.clone()
              } else {
                  format!("{sub}/{file_name}")
              }
          }
          None => file_name.clone(),
      };
      let sidecar_name = {
          let stem = file_name
              .rsplit_once('.')
              .map(|(s, _)| s)
              .unwrap_or(&file_name);
          format!("{stem}.meta.json")
      };
      let rel_sidecar_path = match &input.subdir {
          Some(sub) => {
              let sub = sub.trim().trim_matches('/');
              if sub.is_empty() {
                  sidecar_name.clone()
              } else {
                  format!("{sub}/{sidecar_name}")
              }
          }
          None => sidecar_name.clone(),
      };

      // Full repo-relative paths.
      let full_content_path = format!("{src_docs_base}/{rel_content_path}");
      let full_sidecar_path = format!("{src_docs_base}/{rel_sidecar_path}");

      // Refuse to overwrite an existing content file.
      let existing = store.list_files_recursive(&src_docs_base);
      if existing.contains(&full_content_path) {
          return Err(RepositoryError::InvalidInput {
              message: format!("file already exists in repository: {full_content_path}"),
          });
      }

      // Compute content checksum before writing.
      let content_checksum = sha256_hex(&input.content);

      // Resolve content type.
      let content_type = input
          .content_type
          .as_deref()
          .map(|s| s.to_string())
          .unwrap_or_else(|| infer_content_type(&file_name).to_string());

      // Build sidecar JSON.
      let document_id = uuid::Uuid::new_v4().to_string();
      let sidecar_value = serde_json::json!({
          "documentId": document_id,
          "contentPath": rel_content_path,
          "contentType": content_type,
          "encoding": "binary",
          "checksum": content_checksum,
      });
      let sidecar_bytes = serde_json::to_vec_pretty(&sidecar_value).map_err(|e| {
          RepositoryError::InvalidInput {
              message: format!("failed to serialize sidecar: {e}"),
          }
      })?;
      let sidecar_checksum = sha256_hex(&sidecar_bytes);

      // ADR-007: write content file first, then sidecar, then update index.
      store.save_binary_file(&full_content_path, &input.content)?;
      store.save_text_file(
          &full_sidecar_path,
          &String::from_utf8(sidecar_bytes).map_err(|e| RepositoryError::InvalidInput {
              message: format!("sidecar is not valid UTF-8: {e}"),
          })?,
      )?;

      // Update manifest index.
      let mut manifest = store.load_manifest()?;
      let new_entry = SourceDocumentIndexEntry {
          document_id: document_id.clone(),
          sidecar_path: rel_sidecar_path.clone(),
          content_path: rel_content_path.clone(),
          title: input.title,
          sidecar_checksum: Some(sidecar_checksum.clone()),
          content_checksum: Some(content_checksum.clone()),
      };
      let mut index = manifest.source_document_index.unwrap_or_default();
      index.push(new_entry);
      manifest.source_document_index = Some(index);
      manifest.source_documents_path.get_or_insert_with(|| src_docs_base.clone());
      store.save_manifest(&manifest)?;

      Ok(AddAttachmentResult {
          document_id,
          content_path: rel_content_path,
          sidecar_path: rel_sidecar_path,
          source_documents_path: src_docs_base,
          content_checksum,
          sidecar_checksum,
      })
  }
  ```

- [ ] Write unit tests in `attachment_service.rs` (inside the existing `#[cfg(test)] mod tests`):
  - `add_attachment_happy_path` — stores content + sidecar, returns correct paths and checksums
  - `add_attachment_sets_manifest_index` — manifest index has the new entry after the call
  - `add_attachment_with_subdir` — stores under a subdirectory; paths are prefixed correctly
  - `add_attachment_duplicate_rejected` — second call with the same filename returns an error
  - `add_attachment_infers_content_type_pdf` — PDF extension → `application/pdf`
  - `add_attachment_explicit_content_type` — explicit `content_type` overrides extension inference
  - `add_attachment_filestore_roundtrip` — create FileStore, call add, then call list and confirm entry appears

#### Acceptance Criteria

- [ ] `cargo test -p srs-repository attachment_service` — all tests pass (at least 7 new tests)
- [ ] No business logic in CLI — service contains all file-writing and checksum logic
- [ ] File-before-index ordering holds (content written before manifest updated)

#### Testing

```bash
cargo test -p srs-repository attachment_service
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. All tests pass.
2. Clippy clean.
3. Commit.

---

### Phase 2: CLI + Payload + Schema

**Goal:** `srs attachment add <source-file> [--subdir <dir>] [--title <title>] [--repo <repo>]` emits a valid JSON envelope; `cargo test --test payload_contracts` is green.

**Agent:** CLI Worker

#### Tasks

- [ ] Add to `crates/srs-cli/src/payload.rs`:
  ```rust
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct AttachmentAddPayload {
      pub document_id: String,
      pub content_path: String,
      pub sidecar_path: String,
      pub source_documents_path: String,
      pub content_checksum: String,
      pub sidecar_checksum: String,
  }

  impl From<srs_repository::attachment_service::AddAttachmentResult> for AttachmentAddPayload {
      fn from(r: srs_repository::attachment_service::AddAttachmentResult) -> Self {
          Self {
              document_id: r.document_id,
              content_path: r.content_path,
              sidecar_path: r.sidecar_path,
              source_documents_path: r.source_documents_path,
              content_checksum: r.content_checksum,
              sidecar_checksum: r.sidecar_checksum,
          }
      }
  }
  ```

- [ ] Add `Add` variant to `AttachmentCommand` in `crates/srs-cli/src/commands/mod.rs`:
  ```rust
  #[derive(Subcommand)]
  pub enum AttachmentCommand {
      /// List source documents in the repository (walks subdirectories)
      List,
      /// Add a file as a source-document attachment
      Add {
          /// Path to the local source file to store
          source: std::path::PathBuf,
          /// Optional subdirectory within source-documents/ (e.g. "phase-1")
          #[arg(long)]
          subdir: Option<String>,
          /// Optional human-readable title for the attachment
          #[arg(long)]
          title: Option<String>,
          /// MIME type override (auto-detected from extension if omitted)
          #[arg(long = "content-type")]
          content_type: Option<String>,
      },
  }
  ```

- [ ] Add `cmd_attachment_add` handler in `crates/srs-cli/src/commands/attachment.rs`:
  ```rust
  use crate::payload::AttachmentAddPayload;
  use srs_repository::attachment_service::{self, AddAttachmentInput, ListAttachmentsFilter};
  // (update existing use line to add AddAttachmentInput)

  pub fn dispatch(ctx: CliContext, cmd: AttachmentCommand) -> Result<String> {
      match cmd {
          AttachmentCommand::List => cmd_attachment_list(ctx),
          AttachmentCommand::Add { source, subdir, title, content_type } =>
              cmd_attachment_add(ctx, source, subdir, title, content_type),
      }
  }

  fn cmd_attachment_add(
      ctx: CliContext,
      source: std::path::PathBuf,
      subdir: Option<String>,
      title: Option<String>,
      content_type: Option<String>,
  ) -> Result<String> {
      let file_name = source
          .file_name()
          .and_then(|n| n.to_str())
          .map(|s| s.to_string())
          .ok_or_else(|| anyhow::anyhow!("source path has no file name: {}", source.display()))?;
      let content = std::fs::read(&source)
          .with_context(|| format!("failed to read source file: {}", source.display()))?;
      let input = AddAttachmentInput { file_name, content, subdir, title, content_type };
      let result = with_store(&ctx, |store| Ok(attachment_service::add_attachment(store, input)?))?;
      output::serialize("attachment add", AttachmentAddPayload::from(result))
  }
  ```

- [ ] Run `cargo run --bin generate-schemas` → produces `crates/srs-cli/schemas/payload/attachment-add.json`

- [ ] Confirm `cargo test --test payload_contracts` is green

#### Acceptance Criteria

- [ ] `srs attachment add <file> --repo <path>` returns JSON with `ok: true`, `command: "attachment add"`, and `payload` with all six fields
- [ ] `cargo test --test payload_contracts` passes
- [ ] `crates/srs-cli/schemas/payload/attachment-add.json` exists and is committed

#### Testing

```bash
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
echo "hello attachment" > /tmp/test.pdf
cargo run --bin srs -- attachment add /tmp/test.pdf --repo /tmp/dogfood-test-repo --pretty
```

#### Milestone gate

1. All acceptance criteria checked.
2. Clippy clean.
3. `cargo run --bin generate-schemas` has been run and schema file is staged.
4. Commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `crates/srs-cli/schemas/payload/attachment-add.json` present and accurate
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (entity schemas untouched)
- [ ] `srs attachment add` is wired and dispatches correctly
- [ ] `srs attachment list` still works after the change (no regression)

## Coordination Rules

- Agents keep to their write scopes.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.

## Assumptions

- The RFC (srs#101) is accepted — issue #280 carries `ready` with no `requires-spec-rfc` block.
- `sha2 = "0.10"` with `default-features = false` is wasm32-safe (pure Rust, no OS RNG; consistent with ADR-013 constraint).
- `hex = "0.4"` is wasm32-safe (pure Rust).
- `store.save_binary_file` is available on both `FileStore` and `MemoryStore` (confirmed from `store.rs`).
- Refusing to overwrite existing files is the correct initial behavior; `--force` / `--overwrite` is deferred.
- The CLI reads the source file from disk before calling the service, which is acceptable (analogous to reading stdin) — the service itself never touches the local filesystem.
