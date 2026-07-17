# Plan: srs-gov dogfood — show a decision with its linked attachments (#285)

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

`srs-gov get decision_log <id>` currently shows a decision record's field values but does not show the source documents materially attached to it via `srs attachment link`. This plan enhances `cmd_get` in `crates/srs-gov/src/main.rs` to display the linked attachments section below the field detail — title, content path, document ID, and on-disk file size — surfacing all the material a facilitator needs to see in one glance. This is the Gate B ★ demo deliverable: a facilitator can see "this decision, these source materials" at a glance. No new service function or payload struct is needed; the change is entirely in the `srs-gov` presentation layer, composing two existing capabilities (`srs record get` + `srs attachment list`).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Lead Integrator |
| srs-gov Display Worker | Lead Integrator (single-crate change) |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions. This plan is a single-crate, single-phase display enhancement; no separate worker assignment is needed.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-034](../docs/adr/034-source-refs-in-record-extra.md) | `sourceRefs` on Record is in `record.extra["sourceRefs"]`; access via `record["sourceRefs"]` in the JSON payload | accepted |
| [capability-layering](../docs/architecture/capability-layering.md) | Cross-referencing `record.sourceRefs` with `attachment list` metadata is presentation logic, not shared semantics; it lives in `srs-gov` (thin governance client), not in `srs-repository`. | accepted |
| R5 nominal-string filter (capability-layering R5) | `build_linked_attachments` filters `sourceRole == "attaches"` as a string literal. This is semantic determination (which source refs are attachment-type?) keyed on a nominal string — R5 debt. The correct fix is a typed `get_record_attachments` service in `srs-repository`. Deferred as a follow-up issue; this plan ships the interim shortcut with explicit acknowledgement. | known-debt |
| Client-side file size (`std::fs::metadata`) | `build_linked_attachments` computes file size by joining `repo + sourceDocumentsPath + path` and calling `std::fs::metadata`. The correct fix is to add `sizeBytes: Option<u64>` to `AttachmentEntry` in `payload.rs`, computed by the `FileStore` side of `attachment_service`. Deferred as a follow-up issue; JSON-store repos already degrade gracefully (no path → no size). | known-debt |

No new ADRs are needed. This plan is governed by ADR-034 (storage location of sourceRefs on Record) and the capability-layering rule (format-specific rendering stays in clients). A future `get_record_attachments` service in `srs-repository` (if multiple clients need it) is deferred — see *Out of scope*.

---

## Contracts

### CLI output contract (ADR-011)

No new/changed commands in `srs-cli`. `srs-gov` does not emit JSON payloads through `srs-cli/src/payload.rs`; it renders human-readable text directly. No payload structs change. `cargo test --test payload_contracts` must pass unchanged.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files under `srs/docs/schema/2.0/`. `bash scripts/check-schema-sync.sh` must exit 0 unchanged.

---

## Scope

- Modify `cmd_get` in `crates/srs-gov/src/main.rs` to extract `sourceRefs` from the fetched record, filter for `sourceRole == "attaches"`, cross-reference with `srs attachment list`, and pass the matched entries to a new `render::linked_attachments` function.
- Add `render::linked_attachments(attachments: &[LinkedAttachment])` to `crates/srs-gov/src/render.rs` — renders a "Linked Attachments" section showing title (or doc ID fallback), content path, document ID (short), and on-disk file size.
- Add `LinkedAttachment` struct in `crates/srs-gov/src/render.rs` (plain data carrier for the render function; no serde needed).
- Degrade gracefully when no attachments are linked: silently omit the section.
- Degrade gracefully when a linked document ID is not found in the attachment list: show the document ID without metadata.
- Degrade gracefully on JSON-store repos (no disk files): show metadata from the index entry only, omit size.
- Add render unit tests for `linked_attachments`.

**Out of scope:**

- A `get_record_attachments` service in `srs-repository` (a future enhancement if other clients need this data; deferred as a follow-up issue).
- A WASM binding for record attachment queries (deferred to the same future follow-up).
- Size-warning *policy* (non-blocking diagnostics based on configurable limits) — deferred to #284, which is blocked on RFC srs#101.
- Adding `sizeBytes: Option<u64>` to `AttachmentEntry` in `payload.rs` (the `FileStore` side should compute the size and include it in the list payload; deferred follow-up issue — until then, `srs-gov` reads size client-side via `std::fs::metadata`, acknowledged in the Architecture Decisions table).
- A typed service function for `sourceRole`-based filtering of `sourceRefs` (R5 debt; deferred follow-up issue — see Architecture Decisions table for acknowledgement).
- The `--json` flag case for `srs-gov get` with attachments: the raw `srs record get` JSON already includes `sourceRefs`; no change needed.
- Any change to `srs-cli`, `srs-repository`, or `srs-core`.

---

## Phases

### Phase 1: Add `linked_attachments` render function

**Goal:** A `render::linked_attachments` function exists in `crates/srs-gov/src/render.rs` with passing unit tests.

**Agent:** Lead Integrator

#### Tasks

- [ ] Add `LinkedAttachment` struct to `crates/srs-gov/src/render.rs`:
  ```rust
  pub struct LinkedAttachment {
      pub document_id: String,
      pub title: Option<String>,
      pub content_path: Option<String>,
      /// Size in bytes, `None` when unavailable (JSON-store repos or file not found).
      pub size_bytes: Option<u64>,
  }
  ```

- [ ] Add `pub fn linked_attachments(attachments: &[LinkedAttachment])` to `render.rs`:
  - If `attachments` is empty, return immediately without printing anything.
  - Print a `section("Linked Attachments")` header.
  - Print a column header: `PATH / DOCUMENT ID  TITLE  SIZE`
  - For each attachment, print one row:
    - `path`: `content_path` if available, else `"(no path)"`
    - `title`: `title` if available, else `"—"`
    - `size`: formatted as `"N B"`, `"N KB"`, or `"N MB"` (pick the largest unit where N ≥ 1); `"—"` if `size_bytes` is `None`
    - `doc_id`: `short_id(&attachment.document_id)` shown after path
  - Close with a blank line.

- [ ] Add unit tests in `render.rs` `#[cfg(test)] mod tests`:
  - `linked_attachments_empty_silent` — call `linked_attachments(&[])` and confirm it does not panic (output goes to stdout, visible in test output only; no assertion on text content needed for this case).
  - `linked_attachments_renders_row` — build one `LinkedAttachment` with all fields present, call `linked_attachments`, confirm it does not panic. (srs-gov render functions write to stdout; unit tests verify they do not panic and that the helper `fmt_size` returns correct strings.)
  - `fmt_size_thresholds` — unit test the private `fmt_size(n: u64) -> String` helper directly: `0 → "0 B"`, `1023 → "1023 B"`, `1024 → "1 KB"`, `1048576 → "1 MB"`.

- [ ] Add private `fn fmt_size(bytes: u64) -> String` to `render.rs`:
  ```
  if bytes >= 1_048_576 { format!("{} MB", bytes / 1_048_576) }
  else if bytes >= 1024   { format!("{} KB", bytes / 1024) }
  else                    { format!("{} B", bytes) }
  ```

#### Acceptance Criteria

- [ ] `LinkedAttachment` struct is in `render.rs`
- [ ] `linked_attachments(&[])` does not print anything (no `section` header for empty list)
- [ ] `linked_attachments` with entries prints `THIN` section separator, column header, one row per attachment
- [ ] `fmt_size(0)` == `"0 B"`, `fmt_size(1023)` == `"1023 B"`, `fmt_size(1024)` == `"1 KB"`, `fmt_size(1048576)` == `"1 MB"`
- [ ] Tests `linked_attachments_empty_silent`, `linked_attachments_renders_row`, `fmt_size_thresholds` pass

#### Testing

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests to write or verify:
- `linked_attachments_empty_silent` — empty slice causes no output
- `linked_attachments_renders_row` — single attachment does not panic
- `fmt_size_thresholds` — four size boundary assertions

#### Milestone gate

1. All acceptance criteria above are met.
2. `cargo test -p srs-gov` passes.
3. `cargo clippy -p srs-gov -- -D warnings` passes.
4. Update plan: mark completed task checkboxes `[x]`.
5. Commit:
```bash
git add crates/srs-gov/src/render.rs
git commit -m "feat(srs-gov): add linked_attachments render function (#285)"
```

---

### Phase 2: Wire `cmd_get` to show linked attachments

**Goal:** `srs-gov get decision_log <id>` shows the "Linked Attachments" section when the record has source refs with `sourceRole: "attaches"`, and the section is absent when no attachments are linked.

**Agent:** Lead Integrator

#### Tasks

- [ ] In `crates/srs-gov/src/main.rs`, modify `cmd_get` after `record_detail(id, schema_props, &field_values);`:

  ```rust
  // Show linked attachments (sourceRefs with sourceRole == "attaches")
  let linked = resolve_linked_attachments(record, repo);
  if !linked.is_empty() {
      render::linked_attachments(&linked);
  }
  ```

- [ ] Add `fn resolve_linked_attachments(record: &serde_json::Value, repo: &str) -> Vec<render::LinkedAttachment>` and `fn build_linked_attachments(record: &serde_json::Value, attach_payload: &serde_json::Value, repo: &str) -> Vec<render::LinkedAttachment>` to `main.rs`:

  ```rust
  /// Thin acquisition wrapper: calls run_srs to fetch the attachment list, then delegates
  /// all cross-referencing to `build_linked_attachments`.
  fn resolve_linked_attachments(record: &serde_json::Value, repo: &str) -> Vec<render::LinkedAttachment> {
      // 1. Extract sourceRefs with sourceRole == "attaches" (ADR-034: in record.extra["sourceRefs"])
      let empty = vec![];
      let source_refs = record["sourceRefs"].as_array().unwrap_or(&empty);
      let has_attaches = source_refs
          .iter()
          .any(|r| r["sourceRole"].as_str() == Some("attaches"));
      if !has_attaches {
          return vec![];
      }

      // 2. Fetch attachment list from the store (best-effort: degrade to doc IDs on error)
      let attach_payload = match run_srs(&["attachment", "list"], repo, false, false) {
          Ok(p) => p,
          Err(_) => {
              // Degrade: return stub entries with doc IDs only
              return source_refs
                  .iter()
                  .filter(|r| r["sourceRole"].as_str() == Some("attaches"))
                  .filter_map(|r| r["sourceId"].as_str())
                  .map(|id| render::LinkedAttachment {
                      document_id: id.to_string(),
                      title: None,
                      content_path: None,
                      size_bytes: None,
                  })
                  .collect();
          }
      };

      // 3. Delegate cross-referencing to the pure function
      build_linked_attachments(record, &attach_payload, repo)
  }

  /// Pure function: cross-references sourceRefs against a pre-fetched attachment list payload.
  /// Accepts pre-fetched JSON so the logic is fully testable without spawning subprocesses.
  fn build_linked_attachments(
      record: &serde_json::Value,
      attach_payload: &serde_json::Value,
      repo: &str,
  ) -> Vec<render::LinkedAttachment> {
      let empty = vec![];
      let source_refs = record["sourceRefs"].as_array().unwrap_or(&empty);
      let attached_doc_ids: Vec<&str> = source_refs
          .iter()
          .filter(|r| r["sourceRole"].as_str() == Some("attaches"))
          .filter_map(|r| r["sourceId"].as_str())
          .collect();

      let base_dir = attach_payload["sourceDocumentsPath"]
          .as_str()
          .unwrap_or("source-documents");
      let empty_entries = vec![];
      let entries = attach_payload["entries"].as_array().unwrap_or(&empty_entries);

      // Cross-reference and build result (R5 known-debt: filtering by "attaches" string)
      attached_doc_ids.iter().map(|&doc_id| {
          // AttachmentEntry has #[serde(rename_all = "camelCase")] → "documentId" in JSON
          let entry = entries.iter().find(|e| e["documentId"].as_str() == Some(doc_id));
          let content_path = entry.and_then(|e| e["path"].as_str()).map(String::from);
          let title = entry.and_then(|e| e["title"].as_str()).map(String::from);
          // Compute on-disk size (best-effort; None on JSON-store repos or missing file)
          // Known-debt: should come from sizeBytes in AttachmentEntry payload instead
          let size_bytes = content_path.as_deref().and_then(|rel_path| {
              let full = std::path::Path::new(repo).join(base_dir).join(rel_path);
              std::fs::metadata(full).ok().map(|m| m.len())
          });
          render::LinkedAttachment {
              document_id: doc_id.to_string(),
              title,
              content_path,
              size_bytes,
          }
      }).collect()
  }
  ```

  **Note on the `"path"` key**: `attachment list` returns entries where `path` is the path relative to `source-documents/`. `AttachmentEntry.path: String` is confirmed in `crates/srs-cli/src/payload.rs` line 2048. The on-disk path is `<repo>/<base_dir>/<path>`.

- [ ] Verify that `cmd_get` `--json` flag path returns before `resolve_linked_attachments` is called (the `if json { return Ok(()); }` guard is already in place after the `run_srs` calls).

#### Acceptance Criteria

- [ ] `srs-gov get decision_log <id>` with no linked attachments: output unchanged (no "Linked Attachments" section printed)
- [ ] `srs-gov get decision_log <id>` with one linked attachment: "Linked Attachments" section appears after field detail, with title, path, short doc ID, and size
- [ ] `srs-gov get decision_log <id>` with `--json`: outputs raw `srs record get` JSON (unchanged; `resolve_linked_attachments` not called in JSON path)
- [ ] If `srs attachment list` fails (e.g. empty new repo): section shows doc IDs only, no crash

#### Testing

```bash
cargo build -p srs-gov
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests to write or verify:
- `resolve_linked_attachments_empty_refs` — record with no sourceRefs returns empty Vec (calls `resolve_linked_attachments`, returns early before `run_srs`)
- `resolve_linked_attachments_no_attaches_role` — record with sourceRefs but none with `sourceRole: "attaches"` returns empty Vec (same early-exit path)
- `build_linked_attachments_matches_entry` — record with one attaches ref, payload with matching entry: result contains one `LinkedAttachment` with correct title and path

Add them in a `#[cfg(test)] mod tests` block in `main.rs`:

```rust
#[test]
fn resolve_linked_attachments_empty_refs() {
    let record = serde_json::json!({});
    let result = resolve_linked_attachments(&record, ".");
    assert!(result.is_empty());
}

#[test]
fn resolve_linked_attachments_no_attaches_role() {
    let record = serde_json::json!({
        "sourceRefs": [
            { "sourceType": "repository-document", "sourceId": "doc-1", "sourceRole": "evidence" }
        ]
    });
    let result = resolve_linked_attachments(&record, ".");
    assert!(result.is_empty());
}

#[test]
fn build_linked_attachments_matches_entry() {
    let record = serde_json::json!({
        "sourceRefs": [
            { "sourceType": "repository-document", "sourceId": "doc-abc123", "sourceRole": "attaches" }
        ]
    });
    let attach_payload = serde_json::json!({
        "sourceDocumentsPath": "source-documents",
        "entries": [
            { "documentId": "doc-abc123", "path": "report.pdf", "title": "Q3 Report" }
        ]
    });
    let result = build_linked_attachments(&record, &attach_payload, ".");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].document_id, "doc-abc123");
    assert_eq!(result[0].title.as_deref(), Some("Q3 Report"));
    assert_eq!(result[0].content_path.as_deref(), Some("report.pdf"));
}
```

#### Milestone gate

1. All acceptance criteria above are met.
2. `cargo test -p srs-gov` passes.
3. `cargo build -p srs-gov` succeeds.
4. `cargo clippy -p srs-gov -- -D warnings` passes.
5. Update plan: mark completed task checkboxes `[x]`.
6. Commit:
```bash
git add crates/srs-gov/src/main.rs
git commit -m "feat(srs-gov): show linked attachments in get command (#285)"
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures (full workspace)
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs were changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas were changed)
- [ ] `srs-gov get decision_log <id>` on a record with one linked attachment shows "Linked Attachments" section
- [ ] `srs-gov get decision_log <id>` on a record with no attachments shows no "Linked Attachments" section
- [ ] `srs-gov get decision_log <id> --json` output is unchanged

## Coordination Rules

- Single-agent plan: Lead Integrator implements and reviews.
- No changes to `srs-core`, `srs-repository`, or `srs-cli`.
- Verification Agent runs `cargo test` + `cargo clippy` + `cargo test --test payload_contracts` + `bash scripts/check-schema-sync.sh` after Phase 2.

## Assumptions

- `srs attachment list` payload field name for path is `"path"` (relative to `sourceDocumentsPath`), confirmed in `crates/srs-cli/src/payload.rs` `AttachmentEntry`.
- `srs record get` returns `sourceRefs` in `record_payload["record"]["sourceRefs"]` (ADR-034: `Record.extra` flattens into the JSON object).
- `srs-gov` operates on directory-format repos for the dogfood scenario; JSON-store repos degrade gracefully (no file sizes).
- Size-warning thresholds (per attachment policy) are deferred to #284 (blocked on RFC srs#101).
