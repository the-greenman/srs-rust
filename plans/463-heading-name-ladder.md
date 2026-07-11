# Plan: Add "heading" to record_display_label name ladder (#463)

## Summary

`record_display_label` in `srs-repository` recognizes only `title`, `name`, and `label` as fallback field names when a Type has no `identityFieldId` set. The muDemocracy guide package uses a field named `heading` as the human-visible section title, so every `section.*` record falls through to the `type_name` fallback and renders as e.g. `section.text` regardless of what the user entered. Adding `"heading"` to the priority list (`["title", "heading", "name", "label"]`) is the minimal, self-contained fix: it covers this package without any package-side change, generalizes gracefully to any other package that uses `heading` as a title field, and does not affect the RFC-020 identity-field path (which continues to take precedence over the whole name ladder).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification Agent | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs required. The name-ladder heuristic is an internal implementation detail of `srs-repository`; adding a word to it does not establish a new boundary, reject a design alternative worth documenting, or change an accepted ADR.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Fix stays entirely in `srs-repository`; no CLI or binding changes | accepted |
| [ADR-023](../docs/adr/023-columnspec-identity-column-marker.md) | RFC-020 identity field path (already present) continues to take priority over name ladder; this fix only extends the fallback heuristic below it | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No CLI command output shapes change. `record_display_label` is an internal helper; its result surfaces through existing payload fields (`displayLabel` on list items) whose schema is already `#[schemars(with = "serde_json::Value")]` — no golden-schema regeneration needed.

### Entity schema sync (check-schema-sync.sh)

No entity schema files change.

---

## Scope

- Add `"heading"` to the name-ladder priority list in `record_display_label` at `crates/srs-repository/src/record_label.rs` ~line 78 (position: after `"title"`, before `"name"`).
- Add unit test `display_label_finds_heading_field` — record with string-valued field named `heading` and no `identityFieldId` entry returns the heading value.
- Add unit test `display_label_title_takes_priority_over_heading` — record with both `title` and `heading` fields returns the title value.

**Out of scope:**
- Other title-synonyms (`subject`, `caption`) — deferred until a concrete package need arises; no data that any in-use package requires them.
- Updating the muDemocracy package to set `identityFieldId` — that is the correct long-term fix and is tracked separately; this PR provides immediate relief.
- Any change to the RFC-020 identity-field path (which already takes priority over the whole name ladder and is unaffected).

---

## Phases

### Phase 1: Extend name ladder + unit tests

**Goal:** `record_display_label` returns the value of a string-valued `heading` field when no `identityFieldId` is set and no `title` field is present; all existing tests continue to pass.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `crates/srs-repository/src/record_label.rs` ~line 78, change:
  ```rust
  for priority in &["title", "name", "label"] {
  ```
  to:
  ```rust
  for priority in &["title", "heading", "name", "label"] {
  ```
- [x] Update doc comment on `record_display_label` to reflect `"heading"` in the priority ladder description.
- [x] Add test `display_label_finds_heading_field` — record with string-valued field named `heading` (no identity index entry) returns heading value.
- [x] Add test `display_label_title_takes_priority_over_heading` — record with both `title` and `heading` string-valued fields returns title value.

#### Acceptance Criteria

- [x] A record whose only string-valued field is named `heading` (and whose Type has no `identityFieldId` entry) returns that field's value from `record_display_label`.
- [x] A record with both `title` and `heading` string-valued fields returns the `title` value.
- [x] A record with an `identityFieldId`-mapped field still returns that field's value (existing test `display_label_identity_field_wins_over_title_field` passes unchanged).
- [x] All existing `record_label` unit tests pass.
- [x] `cargo test -p srs-repository` passes.
- [x] `cargo clippy -p srs-repository -- -D warnings` passes.

#### Testing

```bash
cargo test -p srs-repository record_label
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `display_label_finds_heading_field` — proves `heading` is recognized as a string-valued title synonym.
- `display_label_title_takes_priority_over_heading` — proves `title` still wins over `heading`.

#### Milestone gate

1. All acceptance criteria above are met — each checkbox marked `[x]`.
2. Both new tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Commit with `fix: add "heading" to record_display_label name ladder (#463)`.

---

## Final Acceptance

- [x] `cargo test` passes with no failures (subsumes `cargo test -p srs-repository`)
- [x] `cargo clippy -- -D warnings` passes
- [x] CLI output format unchanged (integration tests pass)
- [x] `cargo test --test payload_contracts` passes (no payload structs changed)
- [x] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [x] A record with a string-valued `heading`-named field and no `identityFieldId` returns the heading value from `record_display_label`.
- [x] PR body contains `Closes #463`

## Coordination Rules

- All changes confined to `crates/srs-repository/src/record_label.rs`.
- No changes to CLI, bindings, or entity schemas.

## Assumptions

- The muDemocracy package's `section` Type has no `identityFieldId` set; that is why the current identity-field path does not help and the name ladder is reached.
- Priority order `["title", "heading", "name", "label"]` is correct: `heading` is more semantically specific than `name` but less canonical than `title`.
