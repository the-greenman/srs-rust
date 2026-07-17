# Plan: srs attachment list (walks subdirs) + payload struct + golden schema

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

The `srs attachment` subcommand group does not yet exist. This plan adds `srs attachment list`, which discovers source documents in `source-documents/` (or the path given by `manifest.sourceDocumentsPath`) by walking the directory recursively, then annotates each content file with metadata from `manifest.sourceDocumentIndex` where available. The feature closes a gap in the attachment pipeline (epic #271): agents and UIs need a machine-readable listing of what attachments are present in a repo.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | (self) |
| Repository Service Worker | (self) |
| CLI Worker | (self) |
| Verification | (self) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan implements ADR-010 (service boundary contract) and ADR-011 (CLI output contract). No new ADR needed.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service logic in `srs-repository`, handler in `srs-cli` | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Named payload struct + golden schema | accepted |

---

## Contracts

### CLI output contract (ADR-011)

**New command added** — `srs attachment list` is a new command. Two new named structs will be added to `crates/srs-cli/src/payload.rs`:

- `AttachmentEntry` — one file found in the source-documents directory
- `AttachmentListPayload` — the `attachment list` envelope

After adding the structs: `cargo run --bin generate-schemas` generates `crates/srs-cli/schemas/payload/attachment-list.json`. Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No — this plan does not modify `srs/docs/schema/2.0/` entity schemas.

---

## Scope

- `attachment_service::list_attachments(store)` in `crates/srs-repository/src/attachment_service.rs`
- `AttachmentEntry` + `AttachmentListPayload` structs in `crates/srs-cli/src/payload.rs`
- `crates/srs-cli/src/commands/attachment.rs` handler for `attachment list`
- Wire `Attachment(AttachmentCommand)` into `Commands` enum and `dispatch()` in `crates/srs-cli/src/commands/mod.rs`
- Expose `attachment_service` from `crates/srs-repository/src/lib.rs`
- Golden schema `crates/srs-cli/schemas/payload/attachment-list.json`
- Unit tests in `attachment_service.rs` using `MemoryStore`

**Out of scope:**
- `attachment get`, `attachment add`, `attachment remove` — future commands
- WASM binding for `list_attachments` — deferred to a follow-up
- `--filter` / `--unindexed` flags — deferred to a follow-up

---

## Phases

### Phase 1: Service

**Goal:** `attachment_service::list_attachments` exists, compiles, and passes unit tests.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Create `crates/srs-repository/src/attachment_service.rs` with:
  - `pub struct AttachmentEntry` (fields: `path: String`, `document_id: Option<String>`, `title: Option<String>`, `content_checksum: Option<String>`, `sidecar_checksum: Option<String>`)
  - `pub struct ListAttachmentsResult` (fields: `source_documents_path: String`, `entries: Vec<AttachmentEntry>`)
  - `pub fn list_attachments(store: &dyn RepositoryStore) -> Result<ListAttachmentsResult, RepositoryError>`

- [ ] Service logic in `list_attachments`:
  1. Call `store.load_manifest()` to get manifest
  2. Derive `src_docs_base = manifest.source_documents_path.as_deref().unwrap_or("source-documents")`
  3. Build `index_map: HashMap<String, &SourceDocumentIndexEntry>` keyed on `entry.content_path` (path relative to `src_docs_base`)
  4. Call `store.list_files_recursive(src_docs_base)` → paths prefixed with `src_docs_base/`
  5. Strip the `src_docs_base/` prefix from each path to get the relative path within the directory
  6. Exclude `.meta.json` sidecar files from the listing
  7. For each remaining path, look up `index_map.get(&relative_path)` and populate `AttachmentEntry`
  8. Sort entries by `path` and return `ListAttachmentsResult`

- [ ] Add `pub mod attachment_service;` to `crates/srs-repository/src/lib.rs`

- [ ] Write unit tests in `attachment_service.rs` using `crate::store::memory::MemoryStore`:
  - `list_attachments_empty_store` — no `source-documents/` directory → empty entries, default path
  - `list_attachments_indexed_file` — manifest index entry + matching file → entry has metadata
  - `list_attachments_unindexed_file` — file present but not in manifest index → entry has no metadata
  - `list_attachments_walks_subdirs` — file at `source-documents/subdir/doc.pdf` → appears with `path = "subdir/doc.pdf"`
  - `list_attachments_excludes_sidecars` — `.meta.json` file present → not included in entries
  - `list_attachments_custom_path` — `manifest.source_documents_path = "attachments"` → uses that path

#### Acceptance Criteria

- [ ] `cargo test -p srs-repository attachment_service` — all 6 unit tests pass
- [ ] No file I/O in `srs-core`; all business logic in `srs-repository`
- [ ] Service does not call `list_files_recursive` on a hard-coded `"source-documents"` string — it reads `manifest.source_documents_path` first

#### Testing

```bash
cargo test -p srs-repository attachment_service
cargo clippy -p srs-repository -- -D warnings
```

Specific tests: `list_attachments_empty_store`, `list_attachments_indexed_file`, `list_attachments_unindexed_file`, `list_attachments_walks_subdirs`, `list_attachments_excludes_sidecars`, `list_attachments_custom_path`

#### Milestone gate

1. All 6 tests pass.
2. Clippy clean.
3. Commit.

---

### Phase 2: CLI + Payload + Schema

**Goal:** `srs attachment list --repo <path>` emits a valid JSON envelope; `cargo test --test payload_contracts` is green.

**Agent:** CLI Worker

#### Tasks

- [ ] Add to `crates/srs-cli/src/payload.rs`:
  ```rust
  // ── Attachment payloads ───────────────────────────────────────────────────────

  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct AttachmentEntry {
      pub path: String,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub document_id: Option<String>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub title: Option<String>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub content_checksum: Option<String>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub sidecar_checksum: Option<String>,
  }

  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct AttachmentListPayload {
      pub source_documents_path: String,
      pub entries: Vec<AttachmentEntry>,
  }
  ```

- [ ] Create `crates/srs-cli/src/commands/attachment.rs`:
  - `pub fn dispatch(ctx: CliContext, cmd: AttachmentCommand) -> Result<String>`
  - `fn cmd_attachment_list(ctx: CliContext) -> Result<String>` — calls `attachment_service::list_attachments`, maps to `AttachmentListPayload`, returns `output::serialize("attachment list", payload)`

- [ ] Add `pub mod attachment;` to `crates/srs-cli/src/commands/mod.rs`

- [ ] Add to `Commands` enum in `mod.rs`:
  ```rust
  /// Source document attachment commands
  #[command(subcommand)]
  Attachment(AttachmentCommand),
  ```

- [ ] Add `AttachmentCommand` enum in `mod.rs`:
  ```rust
  #[derive(Subcommand)]
  pub enum AttachmentCommand {
      /// List source documents in the repository (walks subdirectories)
      List,
  }
  ```

- [ ] Add to `dispatch()` in `mod.rs`:
  ```rust
  Commands::Attachment(cmd) => attachment::dispatch(ctx, cmd),
  ```

- [ ] Implement conversion from service `AttachmentEntry` to payload `AttachmentEntry`:
  - The service result struct (`srs_repository::attachment_service::AttachmentEntry`) and the payload struct in `srs-cli` must remain separate per ADR-010 (crate boundary).
  - Add `impl From<srs_repository::attachment_service::AttachmentEntry> for AttachmentEntry` in `crates/srs-cli/src/payload.rs`, mapping all five fields.
  - In the handler, use `.map(AttachmentEntry::from).collect()` to convert the service vec to the payload vec.

- [ ] Run `cargo run --bin generate-schemas` → produces `crates/srs-cli/schemas/payload/attachment-list.json`

- [ ] Confirm `cargo test --test payload_contracts` is green

#### Acceptance Criteria

- [ ] `srs attachment list --repo <path>` returns JSON with `ok: true`, `command: "attachment list"`, and `payload` with `sourceDocumentsPath` and `entries`
- [ ] `cargo test --test payload_contracts` passes
- [ ] `crates/srs-cli/schemas/payload/attachment-list.json` exists and is committed

#### Testing

```bash
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
cargo run --bin srs -- attachment list --repo ../srs/srs --pretty
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
- [ ] `crates/srs-cli/schemas/payload/attachment-list.json` present and schema is accurate
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (entity schemas untouched)
- [ ] `srs attachment list` command is wired and dispatches correctly

## Coordination Rules

- Agents keep to their write scopes.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.

## Assumptions

- `list_files_recursive(dir)` returns paths prefixed with `dir/` (relative to repo root), confirmed by reading `collect_paths_recursive` in `store.rs`.
- MemoryStore's `list_files_recursive` also returns keys prefixed with the passed directory, confirmed by its implementation.
- Sidecar files (`.meta.json`) are excluded from the listing; they are metadata artifacts surfaced via index fields, not content attachments.
- The payload `AttachmentEntry` struct is a separate, schema-owning struct from the service `AttachmentEntry` struct, per crate-boundary rules.
