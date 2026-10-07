# Plan: Fix `add_attachment` sidecar naming to `<file>.meta.json`

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents.

## Summary

`add_attachment` (`crates/srs-repository/src/attachment_service.rs:288-294`) derives
the sidecar file name from the content file's **stem** (`rsplit_once('.')` dropping
everything after the last `.`), producing `<stem>.meta.json` — e.g. `brief.pdf` →
`brief.meta.json`. The spec's `source-document-meta.json` schema's own description
states the convention directly: "The sidecar file has the same base name as the
source file with .meta.json appended" (`docs/schema/2.0/source-document-meta.json`
— correcting an earlier mis-citation of RFC-017 [R10], which governs sidecar
co-location, not the filename-derivation scheme). Every other sidecar-producing/
consuming path in this codebase (`source_document_service.rs`,
`catalog.rs`, `validation.rs`, the spec corpus fixture
`source-documents/ai-sessions/chatgpt-origin.md.meta.json`) use the **full file name**
form, `<file>.meta.json` — e.g. `chatgpt-origin.md.meta.json`. The stem form also
collides: `brief.md` and `brief.txt` both resolve to `brief.meta.json`, so adding the
second attachment either silently overwrites the first sidecar or corrupts its
identity. Fix `add_attachment` to emit the full-filename form, matching every other
sidecar writer/reader in the tree. Part of the-greenman/muDemocracy.org#299.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | self (single-phase fix) |
| Verification | self |

No worker fan-out needed — single function, single call site, one crate.

## Architecture Decisions

No new architectural decisions — this corrects `add_attachment` to conform to the
sidecar-naming convention RFC-017 [R10] already establishes and every other
sidecar-producing/consuming path in this codebase (`source_document_service.rs`,
`catalog.rs`) already follows. Sidecar *discovery* (`resolve_source_documents` /
`classify_source_document_candidate`) never parses the sidecar filename to derive the
content path — it reads `contentPath` out of the sidecar's JSON body, gated only on
the `.meta.json` suffix — so this is a pure bug fix with no migration surface: no
on-disk repository's existing sidecars (right-named or wrong-named) need rewriting,
and no `dataModelRevision` bump applies.

## Contracts

### CLI output contract (ADR-011)

No command output shape changes. `AddAttachmentResult.sidecar_path` is still a
`String`; only the value it holds changes (now matches the full filename + suffix
instead of the stem + suffix). No payload struct change, no schema regen needed.

### Entity schema sync (check-schema-sync.sh)

No schema files touched. N/A.

---

## Scope

- Fix `sidecar_name` derivation in `add_attachment` (`attachment_service.rs`) to use
  the full `file_name` rather than its stem.
- Add a regression test proving two same-stem, different-extension attachments
  (`brief.md`, `brief.txt`) now get distinct sidecar files and both succeed.
- Audit existing tests in `attachment_service.rs` that assert stem-form sidecar paths
  (e.g. `"brief.meta.json"` for content `"brief.pdf"` or `"brief.md"`) and update their
  expected values to the full-filename form where the fix changes the actual output.

**Out of scope:**

- Any migration of existing on-disk `<stem>.meta.json` sidecars — not needed; see
  Architecture Decisions above.
- Changes to sidecar discovery/classification logic (`resolve_source_documents`,
  `catalog.rs::classify_source_document_candidate`) — already naming-agnostic.
- `srs-rust#1291`, `#1208`, `#1173` (unrelated catalog/packaging bugs, already
  `needs-input`-gated) — out of scope for this fix.

---

## Phases

### Phase 1: Fix sidecar naming + regression test

**Goal:** `add_attachment("brief.md", ...)` and `add_attachment("brief.txt", ...)` in
the same directory both succeed and produce distinct sidecars
(`brief.md.meta.json`, `brief.txt.meta.json`).

**Agent:** self

#### Tasks

- [x] In `attachment_service.rs::add_attachment`, change the `sidecar_name` block
  (lines ~288-294) to use `file_name` directly instead of its stem:
  `format!("{file_name}.meta.json")`.
- [x] Update the doc comment above `add_attachment` (line ~236-238) if it references
  the old naming shape. (No update needed — already generic, per plan review.)
- [x] Add a new test (near the existing `add_attachment` tests, e.g. after the test
  covering `"brief.pdf"` → `"brief.meta.json"`) proving the collision is fixed:
  create `brief.md` then `brief.txt` in the same (or no) subdir and assert both
  `add_attachment` calls succeed with `sidecar_path` values `"brief.md.meta.json"`
  and `"brief.txt.meta.json"` respectively, and that both sidecar files exist with
  distinct content.
- [x] Update every existing assertion in `attachment_service.rs` (and any other test
  file under `crates/srs-repository` / `crates/srs-bindings`) that currently expects
  a stem-form sidecar path produced *by `add_attachment` itself* (not fixture data
  pre-seeded with a filename already in full-filename form) to the new full-filename
  form.

#### Acceptance Criteria

- [x] `add_attachment` for `brief.pdf` now returns `sidecar_path: "brief.pdf.meta.json"`.
- [x] `add_attachment("brief.md", ...)` followed by `add_attachment("brief.txt", ...)`
  in the same subdir both succeed (no spurious "file already exists" collision) and
  produce two distinct sidecar files.
- [x] No existing test regresses silently — any assertion on `add_attachment`'s
  returned/written sidecar path reflects the new naming.

#### Testing

```bash
cargo test -p srs-repository attachment_service
cargo test -p srs-bindings
```

Specific tests to write or verify:

- `add_attachment_distinct_extensions_same_stem_no_collision` (new) — proves the fix.
- All pre-existing `attachment_service.rs` tests asserting a sidecar path produced by
  `add_attachment` — updated expectations, still passing.

#### Milestone gate

1. Verify acceptance criteria above. Done.
2. Confirm the new test exists and passes, and no other test was left asserting the
   stale stem-form path. Done — 45/45 `srs-repository` attachment tests pass, 30/30
   `srs-bindings` tests pass, `cargo test --workspace` green (with `SRS_SPEC_DIR`
   pointed at a fresh `origin/master` clone — the local sibling trap, not this fix).
3. `cargo test -p srs-repository && cargo test -p srs-bindings && cargo clippy -- -D warnings` — all green.
4. Mark checkboxes `[x]`, commit. Done.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass) — no payload shape change
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] New collision regression test passes
- [ ] No remaining test asserts the old stem-form sidecar name for output produced by
  `add_attachment`

## Coordination Rules

- Single-agent fix; no cross-worker coordination needed.

## Assumptions

- No on-disk repository in this codebase's test fixtures or corpora currently has an
  `add_attachment`-created stem-form sidecar that this fix would orphan — confirmed by
  grep: every other sidecar-producing path already uses full-filename form, and
  sidecar discovery never parses the filename to recover `contentPath`.
