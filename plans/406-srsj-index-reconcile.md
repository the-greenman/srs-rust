# Plan: Reconcile `.srsj` instanceIndex tags from record files on load

## Summary

When `JsonStore::from_srsj` loads a `.srsj` bundle it deserializes the manifest verbatim, trusting
the `instanceIndex` entries even when their `tags` field is absent or stale. Because
`list_records_filtered` and `discovery_service` use the index `tags` field to apply tag filters
without ever loading the record file, a tagless index entry silently drops the record from tag
discovery entirely. The fix reconciles each index entry against the bundled record file at load
time: if an entry is missing `tags` (i.e. `tags` is `None`) but the corresponding `data` value
carries a `tags` array, the array is copied into the index entry before the store is returned.

This is a pure correctness fix inside `srs-repository`; it changes no public API, no CLI command,
and no payload struct.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Repository Service Worker | Claude (this session) |
| Verification | Claude (this session) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs required. The fix is entirely within `srs-repository::json_store` and implements
correct load-time invariant maintenance — consistent with how the spec defines the index as a
derived, denormalised cache of record metadata.

| ADR | Decision | Status |
|---|---|---|
| ADR-010 | Service logic lives in `srs-repository`, not `srs-cli` | accepted — no change needed |
| ADR-011 | No new payload structs; no schema regeneration needed | accepted — no change needed |
| ADR-017 | Deterministic `.srsj` serialisation — reconciliation does not affect `to_srsj_string` | accepted — no change needed |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. This is an internal load-path fix. Golden schemas
unchanged. No `cargo run --bin generate-schemas` required.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files changed. No sync required.

---

## Scope

- Modify `JsonStore::from_srsj` in `crates/srs-repository/src/json_store.rs` to reconcile
  `instanceIndex` `tags` fields against the bundled `data` record files.
- Add a unit test inside the same file that verifies tag-filtered lookup works when the manifest
  index has `tags: null` but the bundled record has `tags: ["foo"]`.

**Out of scope:**
- Reconciling other index fields (e.g. `title`) — future work, not part of this fix.
- Changing `list_records_filtered` or `discovery_service` to fall back to the record file.
- Any CLI surface change, payload struct change, or new public API.

---

## Phases

### Phase 1: Reconcile index tags in `from_srsj`

**Goal:** `JsonStore::from_srsj` returns a store where every `instanceIndex` entry's `tags` field
matches what is in the bundled record file.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `json_store.rs`, in `from_srsj` (line ~200), after building the `manifest` and before
  constructing the `JsonStoreState`, walk `manifest.instance_index` mutably. For each entry where
  `entry.tags.is_none()`, look up `envelope.data.get(entry.path())` and, if present, attempt to
  extract the `tags` field from the record JSON as `Option<Vec<String>>`. If `tags` is found and
  non-null, assign it to `entry.tags`.

  The extraction helper should be a small private function:
  ```rust
  fn extract_tags_from_record(record_json: &serde_json::Value) -> Option<Vec<String>> {
      record_json
          .get("tags")
          .and_then(|v| serde_json::from_value(v.clone()).ok())
  }
  ```

- [ ] Add a unit test `from_srsj_reconciles_tags_from_record_file` inside the existing
  `#[cfg(test)]` module in `json_store.rs`:
  - Build an inline `.srsj` JSON string where:
    - `manifest.instanceIndex` has one tier-2 entry with `tags: null` (absent).
    - `data` has the corresponding record file with `"tags": ["foo"]`.
  - Load via `JsonStore::from_srsj`.
  - Call `record_store::list_records_filtered` with `RecordListFilter { tag: Some("foo".to_string()), .. }`.
  - Assert the record is returned (len == 1).

  A second test `from_srsj_reconcile_skips_entries_without_data` verifies that an index entry
  with no corresponding `data` key and no `tags` is left as `None` (not panicked, not errored).

#### Acceptance Criteria

- [ ] A `.srsj` bundle where `instanceIndex[*].tags` is absent but the record file has `tags: ["foo"]`
  produces a store where `list_records_filtered(tag: "foo")` returns the record.
- [ ] A `.srsj` bundle where `instanceIndex[*].tags` is already present is not modified.
- [ ] A `.srsj` bundle where a record file is absent from `data` (malformed bundle) does not panic.
- [ ] `cargo test -p srs-repository` passes with zero failures.
- [ ] `cargo clippy -p srs-repository -- -D warnings` passes.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `from_srsj_reconciles_tags_from_record_file` — proves missing index tags are filled from record
  data, making tag filter work.
- `from_srsj_reconcile_skips_entries_without_data` — proves the reconciliation is safe when a
  record file is not present in `data`.

#### Milestone gate

1. All acceptance criteria above checked.
2. Both new tests exist and pass.
3. `cargo test -p srs-repository && cargo clippy -p srs-repository -- -D warnings` both exit 0.
4. Plan checkboxes updated.
5. Commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs were changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas were changed)
- [ ] `from_srsj_reconciles_tags_from_record_file` test exists and passes
- [ ] `from_srsj_reconcile_skips_entries_without_data` test exists and passes

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.

## Assumptions

- The `data` map key for a record file is exactly the string in the index entry's `path` field.
- `tags` in a record file JSON is always `Option<Vec<String>>` serialized as a JSON array or absent.
- Records that genuinely have no tags have no `tags` key in their JSON (not `"tags": null`), so
  `tags: None` in the index is the correct sentinel for "not yet reconciled" as well as "truly empty".
  To avoid incorrectly treating a legitimately-empty-tags record as unreconciled, the reconciliation
  only fills `entry.tags` when the record JSON actually has a non-null, non-empty `tags` value.
