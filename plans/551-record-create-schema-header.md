# Plan: Stamp $schema on record create

## Summary

`srs record create` writes Tier 2 record files without a `$schema` key. Every
file in the spec repo's `records/` tree carries `"$schema":
"https://srs.semanticops.com/schema/2.0/record.json"`, and `scripts/validate-all.mjs`
hard-fails any record that is missing the key. Notes already get the header via
`write_note` in `writer.rs`; records don't because `write_record` in
`record_store.rs` performs a bare `serde_json::to_value(record)` → save without
injecting the key. This plan patches `write_record` to mirror the established
Note behaviour and adds a test to lock in the regression.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | claude-sonnet |
| Repository Service Worker | claude-sonnet |
| Verification Agent | claude-haiku |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions. The write-time schema injection pattern is already
established by `write_note` / `write_note_stores_with_schema_header` in `writer.rs`.
This plan extends the same pattern to Tier 2 records.

| ADR | Decision | Status |
|---|---|---|
| [ADR-004](../docs/adr/004-schemas-embedded-at-compile-time.md) | Schema URLs come from `srs_schema` constants; no hard-coded strings in service logic | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No CLI command output shapes change. The `$schema` key is written to the record
file on disk, not to the `output::ok(...)` payload. No changes to `payload.rs`;
no schema regeneration needed.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are modified. No action required.

---

## Scope

- Patch `write_record` in `crates/srs-repository/src/record_store.rs` to inject
  `"$schema": RECORD_SCHEMA_ID` into the serialized JSON object before saving,
  using the structural injection pattern from `write_note` in `writer.rs`
  (the `if let Value::Object(ref mut obj)` block — not the hard-coded literal).
- Fix the existing ADR-004 violation in `write_note` (`writer.rs`) where the
  `note.json` URL is hard-coded; replace with `srs_schema::NOTE_SCHEMA_ID`.
- Use the `srs_schema::RECORD_SCHEMA_ID` constant (already `"https://srs.semanticops.com/schema/2.0/record.json"`).
- Add a test `write_record_includes_schema_header` in `record_store.rs` that
  verifies the written JSON contains the correct `$schema` value.

**Out of scope:**

- `--path`/`--dir` option for placing records in semantic directories (separate
  enhancement; mentioned in issue but not the primary bug).
- TypedRecord (Tier 1) — no `create_typed_record` service exists yet; Tier 1
  records are written only through migration/import paths, not `srs record create`.
- Note creation — already correct via `write_note`.

---

## Phases

### Phase 1: Patch write_record and add test

**Goal:** `write_record` injects `$schema` into every Tier 2 record file it writes,
and a regression test confirms this.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add `use srs_schema::{NOTE_SCHEMA_ID, RECORD_SCHEMA_ID};` to the imports in
  `crates/srs-repository/src/record_store.rs` (neither constant is currently imported there).
- [ ] In `write_record` (in `record_store.rs`), after `serde_json::to_value(record)`, inject
  `$schema` using the structural pattern from `write_note` (`writer.rs` lines 60–67):
  ```rust
  if let serde_json::Value::Object(ref mut obj) = value {
      obj.insert("$schema".to_string(), serde_json::Value::String(RECORD_SCHEMA_ID.to_string()));
  }
  ```
  Use `RECORD_SCHEMA_ID` from `srs_schema` — do NOT copy the hard-coded string from
  `write_note` (that string is an existing ADR-004 violation being fixed in the next task).
- [ ] Fix the ADR-004 violation in `write_note` (`crates/srs-repository/src/writer.rs`,
  lines 60–67): replace the hard-coded literal
  `"https://srs.semanticops.com/schema/2.0/note.json".to_string()` with
  `NOTE_SCHEMA_ID.to_string()`. Add the required import if not present.
- [ ] Add test `write_record_includes_schema_header` in the `#[cfg(test)]` block
  of `record_store.rs`. It must:
  - Construct a minimal `Record` inline (adapt the struct literal from
    `srs-core/src/types/record.rs` `minimal_record()` — that function is private to its
    crate and cannot be imported).
  - Call `write_record` against a `MemoryStore`.
  - Load the stored JSON via `store.load_instance_json(...)`.
  - Assert `val["$schema"] == "https://srs.semanticops.com/schema/2.0/record.json"`.
- [ ] Verify existing test `record_extra_fields_survive_roundtrip` in
  `srs-core/src/types/record.rs` still passes (it sets `$schema` via `extra`; the
  patch uses unconditional insert, which is safe — `write_note` already does this
  and no test relies on the key being absent from written files).

#### Acceptance Criteria

- [ ] `cargo test -p srs-repository` passes with zero failures, including
  `write_record_includes_schema_header`.
- [ ] `cargo test -p srs-core` passes (no regression in `record_extra_fields_survive_roundtrip`).
- [ ] Manual spot-check: `srs record create` in a scratch repo writes a file that
  contains the `$schema` key.

#### Testing

```bash
cargo test -p srs-repository -- write_record
cargo test -p srs-core -- record_extra_fields
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:

- `write_record_includes_schema_header` — proves write path stamps the schema URL.
- `record_extra_fields_survive_roundtrip` — proves no double-write / parse regression.

#### Milestone gate

1. All acceptance criteria above checked.
2. Both named tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Mark checkboxes `[x]` in this plan.
5. Commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged — `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] `write_record_includes_schema_header` test exists and passes
- [ ] A record file created by `srs record create` on the branch contains `$schema`

## Coordination Rules

- Single-crate change; no cross-crate coordination needed.
- Lead Integrator reviews the diff before the PR.
- Verification Agent runs `cargo test` and clippy after Phase 1.

## Assumptions

- `RECORD_SCHEMA_ID` from `srs_schema` is already in scope via the existing
  `use srs_schema::...` import block in `record_store.rs`; if not, add it.
- No consumers depend on the absence of `$schema` in the written file.
