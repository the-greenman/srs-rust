# Plan: Note guard for FixedInstances and RelationQuery arms of resolve_section_instances

## Summary

`resolve_section_instances` in `render_service.rs` has three arms: `ContainerSubset`, `FixedInstances`, and `RelationQuery`. The `ContainerSubset` arm was updated in #510 to handle Tier-0 notes by loading them via `get_instance_by_id`, which dispatches on tier and avoids the `missing field 'typeId'` crash. However, the `FixedInstances` and `RelationQuery` arms lack the required **note guard**: they call `get_instance_by_id` without first checking whether the resolved ID is a Tier-0 note, and they emit no diagnostic when a note appears where a typed record is expected. This plan adds the note guard (manifest-hoisted `entry.is_note()` check + diagnostic + skip) to both arms, and adds regression tests for each arm.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Repository Service Worker |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions. This plan implements the same guard pattern established by #510 for the `ContainerSubset` arm and applies it consistently to the remaining two arms.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All service logic in srs-repository; note guard belongs here not in CLI | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. The fix is internal to `render_service.rs`; no payload structs are added or modified. `cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

No schema files are modified. `bash scripts/check-schema-sync.sh` must still exit 0.

---

## Scope

- Extend the note guard to `SectionSource::FixedInstances` arm in `resolve_section_instances`
- Extend the note guard to `SectionSource::RelationQuery` arm in `resolve_section_instances`
- Add two regression tests (one per arm) in `render_service.rs`

**Out of scope:**

- Changing how `ContainerSubset` handles notes (that arm already works correctly)
- Changing `TypeQuery` arm (notes have tier != 2, so `list_records_by_type` already excludes them)
- Modifying the downstream markdown or JSON rendering paths

---

## Phases

### Phase 1: Apply note guard to FixedInstances and RelationQuery arms

**Goal:** Both arms skip Tier-0 notes with a diagnostic and do not push them into the records Vec.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `crates/srs-repository/src/render_service.rs`, locate `SectionSource::FixedInstances` arm inside `resolve_section_instances` (~line 1257)
- [x] Hoist `let manifest = store.load_manifest()?` before the `for id in instance_ids` loop
- [x] For each `id`, find the manifest entry. If `entry.is_note()`, push diagnostic `[FixedInstances] skipping Tier-0 note {id}; notes are not rendered in document-view sections` and `continue`; otherwise call `get_instance_by_id` as before
- [x] Locate `SectionSource::RelationQuery` arm (~line 1380). After the ID-collection loop, hoist `let manifest = store.load_manifest()?` before the record-loading loop
- [x] For each `id` in the RelationQuery record-loading loop, check manifest entry. If `entry.is_note()`, push diagnostic `[RelationQuery] skipping Tier-0 note {id}; notes are not rendered in document-view sections` and `continue`; otherwise call `get_instance_by_id` as before

#### Acceptance Criteria

- [x] A FixedInstances section referencing a Tier-0 note ID: render does not error, note is absent from output, diagnostic contains the note ID and the string "notes are not rendered in document-view sections"
- [x] A RelationQuery section whose relation resolves to a Tier-0 note ID: render does not error, note is absent from output, diagnostic contains the note ID and the string "notes are not rendered in document-view sections"
- [x] A FixedInstances section referencing only typed records: behaviour unchanged (no regression)
- [x] A RelationQuery section whose relations resolve only to typed records: behaviour unchanged (no regression)
- [x] ContainerSubset arm: no change in behaviour (notes still render via `render_note_at_level`)

#### Testing

```bash
cargo test -p srs-repository render_service
cargo clippy -p srs-repository -- -D warnings
```

Tests to write (in the `#[cfg(test)]` block of `render_service.rs`, near the existing `#509 / #510` block):

- `fixed_instances_arm_skips_tier0_note_with_diagnostic` — creates a MemoryStore with a Tier-0 note and a typed record, builds a DocumentView with a `FixedInstances` section listing both the note ID and the record ID, calls `render_document_view`, asserts: no error, rendered output includes the typed record's heading, rendered output does NOT include the note's title as a rendered heading, diagnostics contain the note ID
- `relation_query_arm_skips_tier0_note_with_diagnostic` — creates a MemoryStore with a Tier-0 note, a typed record, and a `refers-to` relation from a source record to both the note ID and the typed record ID, builds a DocumentView with a `RelationQuery` section, calls `render_document_view`, asserts: no error, rendered output includes the typed record's heading, diagnostics contain the note ID

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm `fixed_instances_arm_skips_tier0_note_with_diagnostic` and `relation_query_arm_skips_tier0_note_with_diagnostic` exist in the codebase and pass.
3. Run:

```bash
cargo test -p srs-repository render_service
cargo clippy -p srs-repository -- -D warnings
```

4. Mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit: `fix(render): add note guard to FixedInstances and RelationQuery arms (#527)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (`cargo test --test payload_contracts` passes)
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] `fixed_instances_arm_skips_tier0_note_with_diagnostic` test exists and passes
- [ ] `relation_query_arm_skips_tier0_note_with_diagnostic` test exists and passes
- [ ] Diagnostics for both arms match format `[{ArmName}] skipping Tier-0 note {id}; notes are not rendered in document-view sections`

## Coordination Rules

- Lead Integrator (Repository Service Worker) owns all writes to `crates/srs-repository/src/render_service.rs`.
- Verification Agent runs after the single phase completes.
- Do not proceed to commit until milestone gate passes.

## Assumptions

- `MemoryStore` is used as the test double (consistent with all surrounding tests)
- The `make_note_member_store` helper and `simple_field_and_type` helper in the test module are reused
- No new crate dependencies are required
- The `RelationQuery` test can reuse a simple "refers-to" relation type already available via the package fixture
