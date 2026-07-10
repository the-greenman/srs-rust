# Plan: Fix shadow-containerIndex migration importing pathless entries (#490)

## Summary

The open-time migration in `json_store.rs::from_srsj` copies pre-#466 shadow
`data["manifest.json"].containerIndex` entries into the typed `manifest.container_index`.
Old entries carry only `{containerId, title}` (no `path`), and `ContainerIndexEntry.path`
is `Option<String>` in Rust, so deserialization silently succeeds with `path: None`.
When the manifest is later serialized and validated against the JSON schema — which
requires `path` — validation fails with `[/containerIndex/N] "path" is a required property`.
This turns any old `.srsj` that was valid before bindings build ~127 into an invalid one
on first open. The fix is to derive the path during migration (`containers/<id>.json` when
that key exists in `data`) and skip entries for which no path can be derived.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

No new agent roles required.

## Architecture Decisions

No new architectural decisions. This plan implements an open-time migration fix inside
`srs-repository::json_store`, which is the correct layer per ADR-010 (all business logic
in `srs-repository`, not in CLI). ADR-011 is unaffected (no CLI payload changes).

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Migration logic stays in `srs-repository` / `json_store.rs`, not CLI | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed commands. No action required; golden schemas stay as-is.

### Entity schema sync (check-schema-sync.sh)

No schema files under `srs/docs/schema/2.0/` are modified. No action required.

---

## Scope

- Fix the open-time migration in `crates/srs-repository/src/json_store.rs::from_srsj`
  (lines ~244–258) to derive `path` for pathless shadow `containerIndex` entries using
  the pre-#466 convention `containers/<containerId>.json`, and skip entries with no
  matching data key.
- Add two regression tests directly in the `#[cfg(test)]` block of `json_store.rs`:
  1. Pathless entry with a matching data key → entry migrated with derived path.
  2. Pathless entry with no matching data key → entry skipped (not imported).

**Out of scope:**

- Changes to `ContainerIndexEntry` in `srs-core` (the `Option<String>` for `path` is correct for representing already-valid entries that have the path).
- Fixing `srs-web`'s `sample.srsj` fixture (tracked independently).
- Emitting structured diagnostic output for skipped entries (a comment in code is sufficient; no consumer can currently observe per-entry migration diagnostics from `from_srsj`).

---

## Phases

### Phase 1: Fix migration + regression tests

**Goal:** `from_srsj` never imports a pathless `ContainerIndexEntry`; all related tests pass.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/json_store.rs` at the open-time migration block
  (lines 244–259, inside the `if manifest.container_index.is_none()` block), change
  the `filter_map` closure to:
  1. Deserialize the entry to `ContainerIndexEntry` (existing step).
  2. If `entry.path.is_some()`, pass the entry through unchanged.
  3. If `entry.path.is_none()`, derive `let derived = format!("containers/{}.json", entry.container_id)`.
     The field is `ContainerIndexEntry.path: Option<String>` defined in
     `crates/srs-core/src/types/container.rs` line 46.
  4. If `envelope.data.contains_key(&derived)`, set `entry.path = Some(derived)`.
  5. Otherwise return `None` (skip; inline comment: entries that can't produce a valid
     path are dropped rather than imported with path: None, which would fail schema
     validation on the next load).
- [ ] Add test `from_srsj_shadow_migration_derives_path_for_pathless_entry` in the
  `#[cfg(test)]` block of `json_store.rs` — `.srsj` with a shadow entry lacking `path`
  but whose `containers/<id>.json` key exists in `data`; asserts the migrated index entry
  has `path = Some("containers/<id>.json")`.
- [ ] Add test `from_srsj_shadow_migration_skips_entry_with_no_matching_data_key` — same
  setup but the data key does not exist in `envelope.data`; asserts the entry is absent
  from `container_index` (i.e., `container_index` is `None` or the list does not contain
  an entry with that `containerId`:
  `assert!(!idx.iter().any(|e| e.container_id == "<id>"))`).
  Example fixture: `"manifest.json": { "containerIndex": [{"containerId": "aaa...", "title": "T"}] }`
  with NO matching `"containers/aaa....json"` key in `data`.
- [ ] Verify the existing test `json_store_legacy_shadow_delete_does_not_resurrect_container`
  still passes (its fixture has the data key present).

#### Acceptance Criteria

- [ ] A `.srsj` containing a pre-#466 shadow `containerIndex` entry with `{containerId, title}`
  (no `path`) and the matching `containers/<id>.json` data key loads without error and
  has `container_index` populated with a derived path.
- [ ] A `.srsj` with a shadow entry and no matching data key loads without error and the
  entry is absent from `container_index` (the list does not contain an entry with that `containerId`).
- [ ] Schema validation of the migrated manifest succeeds (path is now present, so the
  JSON schema `required: [containerId, path]` constraint is met).
- [ ] The existing migration test `json_store_legacy_shadow_delete_does_not_resurrect_container` still passes (2 new + 1 existing = 3 migration-related tests total).

#### Testing

```bash
cargo test -p srs-repository from_srsj_shadow_migration
cargo test -p srs-repository json_store_legacy_shadow
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `from_srsj_shadow_migration_derives_path_for_pathless_entry` — proves pathless entries get a valid path when data key is present
- `from_srsj_shadow_migration_skips_entry_with_no_matching_data_key` — proves entries with no data key are dropped
- `json_store_legacy_shadow_delete_does_not_resurrect_container` — existing regression guard still passes

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all three named tests exist and pass.
3. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update plan checkboxes `[x]`.
5. Commit: `fix(json_store): derive path for pathless shadow containerIndex entries on migration (#490)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (`cargo test --test payload_contracts` passes)
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] Migrating a pre-#466 `.srsj` with pathless shadow entries no longer fails schema validation

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.

## Assumptions

- The pre-#466 JsonStore path convention for containers is `containers/<containerId>.json`
  (confirmed by existing test fixtures and `save_container` at json_store.rs:1422–1425).
- Entries without any matching data key are rare/impossible in practice but must be
  handled gracefully rather than causing a panic or a schema validation error.
