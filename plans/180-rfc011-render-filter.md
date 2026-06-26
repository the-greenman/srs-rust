# Plan: RFC-011 Render Filter — Lifecycle-state exclusion and repo-wide type query (srs-rust#180)

> **Usage note:** This plan implements RFC-011 (formerly RFC-M, srs#41, Accepted 2026-06-25) in the Rust workspace. The spec defines the required behaviour; this plan details the Rust changes without inventing any new architectural decisions.

## Summary

RFC-011 extends `SectionSource.type-query` with three optional fields: `lifecycleStates` (inclusive multi-value filter), `excludeLifecycleStates` (exclusion filter applied after inclusion), and `containerScope` (`"explicit"` / `"repository"` / `"subtree"`). Currently `render_service.rs` ignores `lifecycle_state` entirely (`lifecycle_state: _`) and has no notion of repository-wide scope. This plan implements all three RFC-011 additions in `srs-core` (model) and `srs-repository` (service), updates the `document-view.json` JSON Schema in the spec repo, and adds tests covering the acceptance criteria from issue #180.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | claude (this pipeline) |
| Core Model Worker | Phase 1 |
| Repository Service Worker | Phase 2 |
| Verification | Phase 3 (tests + final acceptance) |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All filtering logic lives in `srs-repository` service; CLI handler passes flags through unchanged | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | No payload struct changes — `render document-view` output shape unchanged | accepted |
| [ADR-004](../docs/adr/004-schemas-embedded-at-compile-time.md) | `document-view.json` mirror updated via `sync-schemas-from-spec.sh` after canonical schema change | accepted |

No new ADR: `SectionSource` is extended with fields mandated by the accepted RFC-011 spec. No architectural decision is introduced — the API shape, enum names, and conformance rules are all specified by RFC-011.

**Subtree scope (v1):** `containerScope: "subtree"` is specified by RFC-011 but requires container-hierarchy traversal that depends on the `contains`-reachable containers pattern. For v1, `"subtree"` is implemented by traversing `contains` Relations from root instances in each `containerIds[]` entry to find member instances across containers. When `containerIds[]` is absent with `"subtree"`, emit a diagnostic and return an empty result (per [N+27]: "when the context container cannot be determined, treat as explicit with empty containerIds"). Deferred full RFC-N integration is filed as a follow-up issue.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. `render document-view` accepts the same flags and returns the same payload struct. The only change is behavioural: sections with lifecycle filters now correctly exclude or include records. Golden schemas are unchanged; `payload_contracts` test passes without regeneration.

### Entity schema sync (check-schema-sync.sh)

**Yes.** `srs/docs/schema/2.0/document-view.json` must add three optional fields to the `type-query` SectionSource variant. After updating the canonical schema, `scripts/sync-schemas-from-spec.sh` syncs the mirror and regenerates `SHA256SUMS`. `bash scripts/check-schema-sync.sh` must exit 0 before closing.

---

## Scope

- Update `srs/docs/schema/2.0/document-view.json` to add `lifecycleStates`, `excludeLifecycleStates`, `containerScope` to the `type-query` SectionSource.
- Add `ContainerScope` enum and three new optional fields to `SectionSource::TypeQuery` in `srs-core`.
- Implement lifecycle filtering (back-compat `lifecycle_state`, `lifecycleStates`, `excludeLifecycleStates`) in `render_service::resolve_section_instances`.
- Implement `containerScope: "repository"` (skip container filtering; return all records of type).
- Implement `containerScope: "subtree"` (v1: contains-relation traversal from root instances in listed containers to find additional member containers; emit diagnostic if containerIds absent and subtree requested).
- Tests: MemoryStore-based unit tests for lifecycle filtering and scope control; cross-store roundtrip.
- Schema sync and SHA256SUMS regeneration.

**Out of scope:**
- Full RFC-N container-hierarchy for subtree (deferred — see follow-up issue).
- CLI new flags (no new flags needed; the fields come from the DocumentView JSON, not from CLI args).
- Any srs-web or srs-bindings changes (no new service signatures exposed by this plan).

---

## Phases

### Phase 1: Schema + srs-core model (`SectionSource::TypeQuery`)

**Goal:** `SectionSource::TypeQuery` carries three new optional fields, the JSON schema is updated and synced, and all existing tests still pass.

**Agent:** Core Model Worker

#### Tasks

- [ ] Update `srs/docs/schema/2.0/document-view.json` — add `lifecycleStates` (`array of string`), `excludeLifecycleStates` (`array of string`), `containerScope` (enum `"explicit" | "repository" | "subtree"`) to the `type-query` SectionSource object.
- [ ] Run `cd srs-rust && scripts/sync-schemas-from-spec.sh` to sync the mirror and regenerate `SHA256SUMS`.
- [ ] Verify `bash scripts/check-schema-sync.sh` exits 0.
- [ ] Add `ContainerScope` enum to `crates/srs-core/src/types/view.rs` (derives `Debug, Clone, PartialEq, Serialize, Deserialize`; `#[serde(rename_all = "lowercase")]`; variants `Explicit`, `Repository`, `Subtree`).
- [ ] Add three fields to `SectionSource::TypeQuery`:
  - `#[serde(skip_serializing_if = "Option::is_none")] lifecycle_states: Option<Vec<String>>`  (serde `lifecycleStates`)
  - `#[serde(skip_serializing_if = "Option::is_none")] exclude_lifecycle_states: Option<Vec<String>>` (serde `excludeLifecycleStates`)
  - `#[serde(skip_serializing_if = "Option::is_none")] container_scope: Option<ContainerScope>` (serde `containerScope`)
- [ ] Update every `SectionSource::TypeQuery { .. }` struct literal in tests and fixtures to include the three new fields (set to `None` unless a test specifically exercises them). Use `rg "TypeQuery {" crates/` to find all sites.
- [ ] Extend the `section_source_type_query_deserialises` test to assert all three new fields round-trip correctly (absent → `None`, present → populated value).

#### Acceptance Criteria

- [ ] `SectionSource::TypeQuery` carries `lifecycle_states`, `exclude_lifecycle_states`, `container_scope`; all omitted when `None`.
- [ ] `ContainerScope` serialises as lowercase strings (`"explicit"`, `"repository"`, `"subtree"`).
- [ ] `bash scripts/check-schema-sync.sh` exits 0.
- [ ] `cargo test -p srs-core` green.

#### Testing

```bash
cd srs-rust
bash scripts/check-schema-sync.sh
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

- `section_source_type_query_deserialises` — extended to assert new fields round-trip.
- `section_source_type_query_new_fields_absent_round_trip` — TypeQuery with no new fields serialises without the new keys; deserialises back to `None`.

#### Milestone gate

1. All acceptance criteria checked.
2. `cargo test -p srs-core` green, clippy clean.
3. `bash scripts/check-schema-sync.sh` exits 0.
4. Update plan checkboxes, commit: `feat(core): add RFC-011 lifecycle filter fields to SectionSource::TypeQuery (#180)`.

---

### Phase 2: Repository service — `resolve_section_instances` implementation

**Goal:** `render document-view` correctly applies lifecycle filters and container scope; the stub `lifecycle_state: _` is removed.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/render_service.rs`, update `resolve_section_instances` `TypeQuery` arm to destructure all five fields: `semantic_object_type`, `container_ids`, `lifecycle_state`, `lifecycle_states`, `exclude_lifecycle_states`, `container_scope`.
- [ ] Implement **container scoping**:
  - `None` or `Some(ContainerScope::Explicit)`: use `container_ids` as today (existing logic).
  - `Some(ContainerScope::Repository)`: skip container filtering entirely; return all records of the type after lifecycle filters.
  - `Some(ContainerScope::Subtree)`: traverse `contains` Relations from each root instance in the listed containers. Collect all reachable container member sets via BFS on the `contains` relation graph. If `container_ids` is absent/empty, emit a diagnostic and return empty (per [N+27]).
- [ ] Implement **lifecycle filtering** (applied after scoping):
  1. Back-compat: if `lifecycle_state` is `Some(s)` and `lifecycle_states` is `None`, treat as `lifecycle_states = Some(vec![s])`.
  2. `lifecycleStates` inclusion: if non-empty, retain only records whose `lifecycle_state` matches any value (OR semantics). Records with `lifecycle_state = None` are excluded.
  3. `excludeLifecycleStates` exclusion: if non-empty, remove records whose `lifecycle_state` matches any value. Records with `lifecycle_state = None` are NOT removed by this step.
  4. Both present: apply inclusion first, then exclusion.
- [ ] Ensure the diagnostic message for subtree-without-containerIds is pushed to the `diagnostics` vec.
- [ ] Helper: `fn contains_reachable_members(relations: &[Relation], initial_ids: &HashSet<String>) -> HashSet<String>` — BFS/DFS over `contains` edges collecting reachable instance IDs from any initial set.

#### Acceptance Criteria

- [ ] A TypeQuery with `excludeLifecycleStates: ["superseded", "abandoned"]` omits records in those states from Markdown, HTML, and text renders.
- [ ] A TypeQuery with `lifecycleStates: ["active"]` includes only active records; records with no `lifecycleState` are excluded.
- [ ] A TypeQuery with `containerScope: "repository"` returns all decisions regardless of container.
- [ ] The back-compat `lifecycle_state: "active"` filter works identically to `lifecycleStates: ["active"]`.
- [ ] `containerScope: "subtree"` with no `containerIds` emits a diagnostic and returns empty.
- [ ] `cargo test -p srs-repository` green.

#### Testing

```bash
cd srs-rust
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Tests (all using `MemoryStore`):

- `render_type_query_exclude_lifecycle_states` — two records, one `superseded`; section with `excludeLifecycleStates: ["superseded"]` returns only the non-superseded record.
- `render_type_query_lifecycle_states_inclusive` — three records with states `draft`, `active`, `superseded`; `lifecycleStates: ["active"]` returns only the active record.
- `render_type_query_no_lifecycle_state_not_excluded` — a record with no `lifecycleState` is NOT excluded by `excludeLifecycleStates`.
- `render_type_query_no_lifecycle_state_excluded_by_include` — a record with no `lifecycleState` IS excluded when `lifecycleStates` is non-empty.
- `render_type_query_repository_scope` — two containers each with one record; `containerScope: "repository"` returns both records.
- `render_type_query_backcompat_lifecycle_state` — `lifecycle_state: "active"` (back-compat singular) filters correctly.
- `render_rfc011_cross_store_roundtrip` — MemoryStore → JSON → FileStore: same TypeQuery with lifecycle filter produces same result set from both stores.

#### Milestone gate

1. All acceptance criteria checked.
2. All seven tests listed pass.
3. `cargo test -p srs-repository` green, clippy clean.
4. Update plan checkboxes, commit: `feat(repository): implement RFC-011 lifecycle filter and repo-scope in render_service (#180)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload struct changes)
- [ ] `bash scripts/check-schema-sync.sh` exits 0
- [ ] `srs repo validate --repo ../srs/srs` reports 0 errors (no false positives from RFC-011 conformance rules)
- [ ] A DocumentView section with `excludeLifecycleStates: ["superseded"]` correctly omits superseded records in a test render
- [ ] A TypeQuery with `containerScope: "repository"` returns records from all containers

## Coordination Rules

Standard (see template). Phases are sequential.

## Assumptions

- `lifecycle_state` on `Record` uses the string key form (not the Lifecycle Term `id`), consistent with existing `Record.lifecycle_state: Option<String>`.
- `contains` Relations use `relationType: "contains"` as the string match key.
- The `srs/` repo local checkout at `../srs` (relative to `srs-rust/`) is available and up to date.
