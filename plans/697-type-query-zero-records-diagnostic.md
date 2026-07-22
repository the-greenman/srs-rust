# Plan: type-query zero-records warning diagnostic (#697)

## Summary

When a `DocumentView` section with a `TypeQuery` source resolves to 0 records, the render
service currently returns silently (`ok:true`, empty rendered output, no diagnostics). A
misaimed `semanticObjectType` (e.g. pointing at `com.semanticops.srs/meta.section` instead
of `com.semanticops.spec/section`) is indistinguishable from a genuinely empty repository.
This plan adds a `warning` diagnostic whenever a TypeQuery resolves to 0 matches, emitted
in `resolve_section_instances` so both the markdown and JSON-projection render paths receive
it, regardless of `emptyBehavior`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Repository Worker |
| Repository Worker | — |
| Verification | Verification Agent |

## Architecture Decisions

No new architectural decisions — this change applies the existing diagnostic pattern already
used by `list_members_degraded` (#509) and the malformed-namespace TypeQuery check. All
diagnostics live in the `RenderResult.diagnostics` vector; consumers decide how to surface
them (per ADR-010/ADR-011).

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Business logic (diagnostic emission) stays in `srs-repository`, not `srs-cli` | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | No payload struct changes — `diagnostics: Vec<String>` already present in `RenderDocumentViewOutput` | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed payload structs. The `diagnostics` field in `RenderDocumentViewOutput`
already carries advisory messages; this plan adds entries to it under new conditions.
`cargo test --test payload_contracts` requires no changes.

### Entity schema sync

No entity schemas added or modified.

---

## Scope

- Emit a `[section:{id}] type-query '{sot}' matched 0 records` warning diagnostic from
  `resolve_section_instances` when a TypeQuery arm returns an empty result set, covering
  all post-filter empty outcomes (wrong type name, container scoping that removes all
  records, lifecycle filtering that removes all records).
- Add unit tests covering: (a) markdown render path with zero-match TypeQuery, (b) JSON
  projection path with zero-match TypeQuery, (c) `emptyBehavior: hide` still emits the
  diagnostic (the output block is hidden, not the warning).

**Out of scope:**

- Zero-record warnings for `ContainerSubset` or `RelationQuery` sources (separate concern,
  filed as follow-up if warranted after dogfooding).
- `repo validate` integration (render diagnostics already appear in CLI output; adding them
  to a separate validate pass is a separate enhancement).

---

## Phases

### Phase 1: Emit diagnostic in TypeQuery arm

**Goal:** `resolve_section_instances` pushes a warning diagnostic whenever a TypeQuery
resolves to 0 records, and tests verify this for both render paths.

**Agent:** Repository Worker

#### Tasks

- [x] In `crates/srs-repository/src/render_service.rs`, function `resolve_section_instances`,
  TypeQuery arm: after all filtering, before the final `Ok(records…)`, check if `records`
  is empty and push a diagnostic:
  ```
  [section:{section_id}] type-query '{semantic_object_type}' matched 0 records
  ```
- [x] Add test `type_query_zero_records_emits_diagnostic` (markdown path): construct a
  `MemoryStore` with a `DocumentView` containing a TypeQuery section pointing at a type
  with no instances; assert `result.diagnostics` contains the expected message and
  `result.rendered` does not contain the section title (hide behaviour preserved).
- [x] Add test `type_query_zero_records_emits_diagnostic_json` (JSON projection path):
  same setup, `format: Some("json")`; assert the diagnostic is present and
  `result.projection.sections[0].records` is empty.
- [x] Add test `type_query_zero_records_empty_behavior_hide_still_warns`: set
  `empty_behavior: Some(EmptyBehavior::Hide)` explicitly; assert diagnostic still present.

#### Acceptance Criteria

- [ ] `result.diagnostics` contains a string matching `[section:…] type-query '…' matched 0 records` whenever a TypeQuery arm returns empty, regardless of `empty_behavior`.
- [ ] Existing render behaviour unchanged: the rendered output (or projection) for a zero-match TypeQuery section remains empty/hidden.
- [ ] No regression in any existing `render_type_query_*` or `dangling_container_*` tests.

#### Testing

```bash
cargo test -p srs-repository type_query_zero_records
cargo test -p srs-repository render_type_query
cargo test -p srs-repository dangling
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `type_query_zero_records_emits_diagnostic` — markdown path warns, section still empty
- `type_query_zero_records_emits_diagnostic_json` — JSON projection path warns
- `type_query_zero_records_empty_behavior_hide_still_warns` — emptyBehavior:hide doesn't suppress the diagnostic

#### Milestone gate

1. All acceptance criteria checked above.
2. All new tests listed exist and pass.
3. `cargo test -p srs-repository && cargo clippy -p srs-repository -- -D warnings`
4. Mark task checkboxes `[x]`, commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `result.diagnostics` contains `[section:…] type-query '…' matched 0 records` for zero-match TypeQuery sections
- [ ] `emptyBehavior: hide` suppresses the rendered block but not the diagnostic

## Coordination Rules

- Lead Integrator owns all writes to `render_service.rs`.
- Verification Agent runs `cargo test -p srs-repository` and confirms no regressions.

## Assumptions

- The existing `diagnostics: Vec<String>` contract is sufficient; no structured severity level is needed at this stage.
- "0 records after all filtering" is always worth diagnosing — there is no scenario where a TypeQuery legitimately returning 0 records should be silent (the alternative is logging at verbose level, which the service layer does not have).
