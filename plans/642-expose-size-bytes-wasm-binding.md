# Plan: Expose sizeBytes via srs-bindings WASM binding for attachment list (#642)

## Summary

`AttachmentEntry.size_bytes` was added to the `srs-repository` service layer and is already
serialised to `sizeBytes` in the JSON output of the `list_attachments` WASM binding (via `to_js`).
The gap is test coverage: no integration test in `crates/srs-bindings/tests/list_attachments.rs`
exercises the path where binary content is added via `add_attachment`, verifying that `size_bytes`
is populated by the service and that the serialised output contains the camelCase `sizeBytes` key
(exactly what WASM consumers receive). This plan adds that smoke test and files a follow-up issue
in `srs-web` to remove a stale comment in `srs-client.ts`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Bindings Worker |
| Bindings Worker | Adds test to `crates/srs-bindings/tests/list_attachments.rs` |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan implements ADR-013 (WASM binding strategy) and ADR-010
(service boundary contract). The `size_bytes` field already flows from the `srs-repository` service
through `to_js` serialisation in the existing binding. This plan adds test coverage only; no
structural change is required.

| ADR | Decision | Status |
|---|---|---|
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM bindings are thin wrappers; `to_js` handles camelCase serialisation via serde attributes | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions return typed structs; no business logic in binding layer | accepted |
| [ADR-011](../docs/adr/011-cli-payload-contract.md) | No payload struct changes; `cargo test --test payload_contracts` must remain clean | accepted |
| [ADR-031](../docs/adr/031-source-doc-blob-portability.md) | `JsonStore::save_binary_file` (Amendment #291) writes to an in-memory `binary_files` map; `file_byte_len` reads from it via `load_binary_file`. Test uses `srsj_empty()` to keep `data`/`binary_files` disjoint (ADR-031 invariant enforced by `list_files_recursive` chaining the two maps without dedup). | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No CLI commands are added or modified. `AttachmentEntry.size_bytes` is not surfaced via a CLI
payload struct. No `payload.rs` changes. No schema regeneration required.

Verification: `cargo test --test payload_contracts` must continue to pass unchanged.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are added or modified. `bash scripts/check-schema-sync.sh` must exit 0 with no changes.

---

## Scope

- Add `binding_list_attachments_size_bytes_from_binary_content` test to
  `crates/srs-bindings/tests/list_attachments.rs`. The test must:
  1. Load a `JsonStore` from the existing `srsj_empty()` fixture (empty store keeps `data` and
     `binary_files` maps disjoint per ADR-031 — never call `save_binary_file` on a path already
     registered in `data`, which would cause `list_files_recursive` to return duplicates).
  2. Call `add_attachment` to register a file with known byte content.
  3. Call `list_attachments` and assert `entry.size_bytes == Some(n)`.
  4. Assert that `serde_json::to_value(&result)` produces JSON with `entries[0]["sizeBytes"] == n`
     (camelCase, proving `to_js` will carry the field to WASM consumers).
- File a follow-up issue in `srs-web` to update the stale comment in `src/lib/srs-client.ts`
  (`AttachmentEntry.sizeBytes` JSDoc currently says "Absent until srs-rust#645 lands"; the correct
  issue is #642, and it will have landed once this PR merges).

**Out of scope:**

- Displaying `sizeBytes` in `AttachmentsPanel.svelte` (deferred; `srs-web` UI work).
- Adding `size_bytes` to any CLI payload struct (separate decision if needed).
- Adding `size_bytes` to `ResolvedAttachment` / `get_record_attachments` (different code path).
- Any change to the service layer — it already populates `size_bytes` correctly.

---

## Phases

### Phase 1: Add size_bytes smoke test to the bindings integration test suite

**Goal:** `crates/srs-bindings/tests/list_attachments.rs` contains a test proving that binary
content added via `add_attachment` is reflected as `sizeBytes` in the serialised JSON output.

**Agent:** Bindings Worker

#### Tasks

- [x] In `crates/srs-bindings/tests/list_attachments.rs`, append the following test. The file
  already imports `srs_repository::attachment_service::{list_attachments, ListAttachmentsFilter}`
  and `srs_repository::JsonStore`. Import `add_attachment` and `AddAttachmentInput` inside the
  test body so the module-level import list stays minimal.

  ```rust
  #[test]
  fn binding_list_attachments_size_bytes_from_binary_content() {
      use srs_repository::attachment_service::{add_attachment, AddAttachmentInput};

      // Start from an empty store so data and binary_files maps stay disjoint (ADR-031).
      // Calling save_binary_file on a path already in data would violate the disjointness
      // invariant and cause list_files_recursive to return the path twice.
      let store = JsonStore::from_srsj(&srsj_empty()).expect("load store");
      add_attachment(
          &store,
          AddAttachmentInput {
              file_name: "brief.pdf".to_string(),
              content: b"PDF bytes".to_vec(),
              subdir: None,
              title: Some("Board Brief".to_string()),
              content_type: None,
          },
      )
      .expect("add_attachment must succeed");

      let result =
          list_attachments(&store, ListAttachmentsFilter::default()).expect("list_attachments ok");
      assert_eq!(result.entries.len(), 1, "one content file");
      let entry = &result.entries[0];
      assert_eq!(
          entry.size_bytes,
          Some(9),
          "size_bytes must reflect binary content length (b\"PDF bytes\" = 9)"
      );
      // Verify camelCase JSON key — this is what WASM consumers receive via to_js
      let json = serde_json::to_value(&result).expect("ListAttachmentsResult must serialise");
      assert_eq!(
          json["entries"][0]["sizeBytes"].as_u64(),
          Some(9),
          "sizeBytes must appear in JSON when present"
      );
  }
  ```

  The constant `9` is the byte length of `b"PDF bytes"`.

- [x] File a follow-up issue in `srs-web` titled "Remove stale sizeBytes JSDoc comment in
  srs-client.ts (srs-rust#642 landed)". Body: the `AttachmentEntry.sizeBytes` JSDoc in
  `src/lib/srs-client.ts` says "Absent until srs-rust#645 lands" — correct issue is #642, and
  it will be live once this PR merges. Remove or rewrite the comment. Record the filed issue
  number in a comment on srs-rust#642.

#### Acceptance Criteria

- [x] `binding_list_attachments_size_bytes_from_binary_content` test exists in
  `crates/srs-bindings/tests/list_attachments.rs` and passes.
- [x] All three pre-existing tests in that file continue to pass unchanged.
- [x] `entry.size_bytes == Some(9)` assertion confirms the service populates the field.
- [x] `json["entries"][0]["sizeBytes"].as_u64() == Some(9)` confirms camelCase serialisation.
- [x] No unused-import warnings (`cargo clippy -p srs-bindings -- -D warnings` clean).

#### Testing

```bash
cargo test -p srs-bindings --test list_attachments
cargo clippy -p srs-bindings -- -D warnings
```

Specific tests to verify:

- `binding_list_attachments_size_bytes_from_binary_content` — proves binary add → `size_bytes` → `sizeBytes` JSON round-trip
- `binding_list_attachments_empty_repo` — must still pass (no regression)
- `binding_list_attachments_with_indexed_entry` — must still pass (no regression)
- `binding_list_attachments_unindexed_file_appears_path_only` — must still pass (no regression)

#### Milestone gate

1. All acceptance criteria above are checked.
2. All four tests listed above pass.
3. Run:

```bash
cargo test -p srs-bindings --test list_attachments
cargo clippy -p srs-bindings -- -D warnings
```

4. Update this plan: mark task checkboxes `[x]`.
5. Commit:

```bash
git commit -m "test(srs-bindings): add size_bytes smoke test for list_attachments binding (#642)"
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged — `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no schema changes)
- [ ] `binding_list_attachments_size_bytes_from_binary_content` test exists and passes
- [ ] All pre-existing `list_attachments` binding tests pass unchanged

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and
  pass, update the plan checkboxes, then commit. Do not proceed to the next phase without
  completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `JsonStore::from_srsj` produces a store where `data` and `binary_files` are disjoint (the
  `.srsj` format never carries binary content per RFC-017). `add_attachment` writes only to
  `binary_files`, keeping the disjointness invariant.
- `srsj_empty()` (from the existing test helpers in `list_attachments.rs`) is a valid minimal
  `.srsj` fixture with an empty `instanceIndex` and no data entries.
- `AddAttachmentInput` and `add_attachment` are pub-accessible from `srs_repository::attachment_service`.
