# Plan: Schema-driven record display label (`identityFieldId`)

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

`record_label::record_display_label()` currently resolves a Record's display label with a hardcoded English name ladder (`title` > `name` > `label`), falling back to `type_name`. Every SRS repository whose primary field isn't named one of those three shows meaningless type-name labels everywhere: `list_records`, discovery, tree/navigation, container views, and search-text projection. RFC-020 (srs#144, accepted, merging as srs#148) adds `Type.identityFieldId` to the canonical schema — a cascading, inheritable, overridable pointer to the field that holds a record's identity/display text. This plan makes `record_display_label()` and its six call sites schema-driven against that new property, adds the corresponding `Package::effective_identity_field_id()` resolution and Rule [N+33] validation, and surfaces the identity field as an explicit marker on `resolve_container_view`'s `ColumnSpec` (ADR-023).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Core Model Worker | — |
| Repository Service Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-018](../docs/adr/018-container-view-column-source-precedence.md) | Column-source precedence for container-view projections (which section/View resolves columns) — unaffected by this plan; `is_identity_column` is a marker on already-resolved columns, not a resolution-order change | accepted |
| [ADR-023](../docs/adr/023-columnspec-identity-column-marker.md) | `ColumnSpec.isIdentityColumn: bool` marks the column matching the Type's effective `identityFieldId`, scoped to containers whose View is anchored to exactly one Type via `root_type_refs` (revised during plan review — the original draft assumed a Type-resolution mechanism that doesn't exist; see ADR's "Correction" note); never reorders columns (preserves RFC-015 view-owned order) | proposed (this plan) |

RFC-015 (`ext:views-l2`, spec repo) established that View-authored column/field order is presentational and must not be silently overridden by another ordering signal. ADR-023 is scoped specifically to respect that: the new flag carries no ordering information.

---

## Contracts

### CLI output contract (ADR-011)

- **Existing command payload changed, but opaque:** `container resolve-view`'s `ContainerViewPayload.container_view` (in `crates/srs-cli/src/payload.rs:520-524`) embeds `container_view_service::ContainerView` via `#[schemars(with = "serde_json::Value")]` — the opaque pattern. Adding `ColumnSpec.is_identity_column` changes the JSON shape returned by this command but **does not** require a golden-schema regeneration, per the established rule for opaque-embedded types (`semanticops/CLAUDE.md`, `srs-rust/CLAUDE.md` Payload Contract section). Verify `cargo test --test payload_contracts` still passes unmodified (Phase 4) — it should, since the golden schema for this field is `{"type": "object"}` / value-opaque already.
- No new CLI command is added by this plan.

### Entity schema sync (check-schema-sync.sh)

- **Yes** — `srs/docs/schema/2.0/type.json` gains `identityFieldId` (RFC-020, accepted, merging as srs#148; branch `rfc/020-type-level-identity-field` at worktree `/home/greenman/dev/semanticops/.worktrees/rfc-020-type-level-identity-field` has the CI-green content now).
- Since srs#148 has not yet merged to `srs` `master`, sync from the RFC worktree explicitly rather than the default `../srs` (which is still on `master` and lacks the field):
  ```bash
  scripts/sync-schemas-from-spec.sh /home/greenman/dev/semanticops/.worktrees/rfc-020-type-level-identity-field
  ```
- This satisfies `srs-rust/CLAUDE.md`'s documented multi-repo merge order ("the srs-rust... schema mirror PRs must be merged before the corresponding srs spec PR") — this repo's mirror PR is expected to merge before or alongside srs#148, not after.
- `srs-vscode`'s mirror is **out of scope** for this plan (separate repo, syncs independently per its own pipeline) — file a best-effort tracking issue in Stage 9 of `/ship` if not already covered.
- Verify `bash scripts/check-schema-sync.sh` — it checks both `srs-rust` and `srs-vscode`; the `srs-vscode` half is expected to still report drift until that repo's own sync happens (not a blocker for this plan; note it explicitly in the milestone gate rather than silently ignoring the residual drift).
- **Declined finding (plan review):** the Architecture Reviewer flagged that `srs-rust/CLAUDE.md`'s Schema Sync section says "the srs-rust *and* srs-vscode schema mirror PRs must be merged before the corresponding srs spec PR," and suggested this plan should also sync `srs-vscode`. Declined: the `/ship` pipeline's own Stage 6 instructions for the `srs`-repo-side RFC work are explicit that a session working in one repo must not reach into sibling repos' working trees ("Do not reach into `../srs-vscode` or any other sibling working tree... each mirror repo syncs itself from the `srs` release artifact through its own pipeline"). That instruction is more specific to *this session's conduct* than CLAUDE.md's general merge-order statement, which describes the eventual, cross-session merge order rather than mandating same-session, same-PR coordination. This plan's existing "file a tracking issue" step is the correct scope boundary; `srs-vscode`'s own maintainers/pipeline own that repo's sync timing.

---

## Scope

- `crates/srs-core/src/types/record_type.rs` — add `identity_field_id: Option<String>` to `RecordType`.
- `crates/srs-schema/schemas/2.0/type.json` (+ `SHA256SUMS`) — synced mirror of RFC-020's `identityFieldId` addition.
- `crates/srs-repository/src/package.rs` — new `Package::effective_identity_field_id(&self, record_type: &RecordType) -> Result<Option<String>, RepositoryError>`, cascading the ancestor chain per RFC-020 Rule [N+34].
- `crates/srs-repository/src/validation.rs` — new Rule [N+33] check: every Type's effective `identityFieldId` (own or inherited) must reference a `fieldId` in that Type's effective field set.
- `crates/srs-repository/src/record_label.rs` — `record_display_label()` gains an identity-field lookup as the new first-priority source, ahead of the name ladder, per RFC-020 Rule [N+36].
- All six call sites of `record_display_label()`: `record_store.rs`, `tree_service.rs`, `discovery_service.rs`, `repository_navigation_service.rs`, `container_view_service.rs`, `text_projection.rs`.
- `crates/srs-repository/src/container_view_service.rs` — `ColumnSpec.is_identity_column: bool` (ADR-023).

**Out of scope:**

- Tier 1 (TypedRecord) label resolution — RFC-020 explicitly scopes `identityFieldId` to Tier 2 Records only (Rule [N+35]); Tier 1 keeps the existing name-ladder-only fallback, unchanged.
- The Default Rendering Baseline / Heading Hierarchy `titleFieldId` fallback (RFC-020 Rule [N+37]) — that's `render_service.rs` / document-view rendering, a separate capability from record-label resolution; not touched by this plan. File as a follow-up issue in Stage 3.4 of `/ship` if not already covered.
- `srs-vscode` UI changes to render `isIdentityColumn` — backend contract only in this plan.
- `srs-bindings` (WASM) — no code changes expected; it calls the same `srs-repository` services this plan modifies, so the new behavior is inherited automatically. Verification Agent confirms no `srs-bindings` code duplicates `record_display_label` logic.
- Resolving `identityFieldId` for records loaded via `MemoryStore` fixtures used only in tests other than this plan's own — out of scope to retrofit unrelated test fixtures beyond what's needed to keep them compiling (struct-literal field additions only).
- Marking `is_identity_column` for heterogeneous (multi-Type) containers — ADR-023 scopes this to the unambiguous single-Type case (`root_type_refs` with exactly one entry); every column is `false` otherwise. A follow-up issue may be filed if per-member identity marking for heterogeneous containers is wanted later.

---

## Phases

### Phase 1: Schema mirror sync

**Goal:** `crates/srs-schema/schemas/2.0/type.json` matches RFC-020's accepted `identityFieldId` addition. This runs first because Phase 2 adds a schema-contract round-trip test that validates against the synced `type.json` — running schema sync first avoids a spurious test failure against the pre-sync schema.

**Agent:** Lead Integrator

#### Tasks

- [x] Run from `srs-rust/`:
  ```bash
  scripts/sync-schemas-from-spec.sh /home/greenman/dev/semanticops/.worktrees/rfc-020-type-level-identity-field
  ```
- [x] Confirm `crates/srs-schema/schemas/2.0/type.json` now has an `identityFieldId` property matching the description authored in the RFC (format `uuid`, references Rules [N+32]–[N+34]).
- [x] Confirm `SHA256SUMS` was regenerated by the script (never hand-edit it).
- [x] Run `bash scripts/check-schema-sync.sh` and confirm the `srs-rust` half reports in-sync. The `srs-vscode` half is expected to still report drift (out of scope, see Contracts) — do not attempt to fix it from this worktree.

#### Acceptance Criteria

- [x] `crates/srs-schema/schemas/2.0/type.json` contains `identityFieldId`.
- [x] `crates/srs-schema/schemas/2.0/SHA256SUMS` regenerated via the script (not hand-edited).
- [x] `check-schema-sync.sh`'s `srs-rust` check passes.

#### Testing

```bash
bash scripts/check-schema-sync.sh
```

#### Milestone gate

1. Verify acceptance criteria.
2. Commit: `git commit -m "chore(schema): sync type.json for identityFieldId (RFC-020, #376)"`.

---

### Phase 2: Core model — `RecordType.identity_field_id`

**Goal:** `RecordType` carries the new field; the crate compiles and all existing tests pass with the field defaulted to `None` everywhere it isn't explicitly set.

**Agent:** Core Model Worker

#### Tasks

- [x] In `crates/srs-core/src/types/record_type.rs`, add to the `RecordType` struct (after `field_assignment_overrides`, matching its exact pattern):
  ```rust
  #[serde(skip_serializing_if = "Option::is_none")]
  pub identity_field_id: Option<String>,
  ```
- [x] Update every `RecordType { ... }` struct literal in this file's test module to include `identity_field_id: None` (or a test-specific `Some(...)` where a test explicitly needs one). Lines ~125, 201, 236, 363, 397, 421, 471, 555 are example locations as of plan-writing time — treat them as a starting point, not an exhaustive list; they may have shifted.
- [x] Update the `RecordType` struct literals in `crates/srs-repository/src/package.rs`'s test helpers `make_type` (~line 1275-1293) and `make_child_type` (~line 1301-1319), and the standalone literal at `package.rs:1469-1500`.
- [x] Grep the full workspace for any other `RecordType { ... }` struct literal this task missed: `rg -n "RecordType\s*\{" crates/ --type rust`. This is the authoritative completeness check — fix every compile error `cargo build` surfaces until the workspace builds clean.
- [x] Add a third schema-contract round-trip test to `record_type.rs`, alongside the existing `minimal_record_type_passes_schema_contract` and `type_with_inheritance_passes_schema` (ADR-004's "serialize Rust values and validate them through `srs-schema`" pattern): a `RecordType` with `identity_field_id: Some(<valid uuid>)`, serialized and validated against `type.json` (already synced in Phase 1, which runs before this phase). This catches a Phase 1/Phase 2 naming or ordering mismatch (e.g. `identity_field_id` vs `identityFieldId` casing) that unit tests alone wouldn't.

#### Acceptance Criteria

- [x] `cargo build -p srs-core` succeeds.
- [x] `cargo build -p srs-repository` succeeds (no missing-field errors from `RecordType` literals).
- [x] `cargo test -p srs-core` passes with no new failures.

#### Testing

```bash
cargo test -p srs-core
cargo build --workspace
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm every test listed in Testing exists and passes.
3. `cargo test -p srs-core && cargo clippy -p srs-core -- -D warnings`
4. Mark checkboxes `[x]`, commit: `git commit -m "feat(srs-core): add RecordType.identity_field_id (#376)"`.

---

### Phase 3: Repository service — identity resolution and all consumers

**Goal:** Every label-producing service resolves a Record's display label via the Type's effective `identityFieldId` first, falling back to the existing name ladder, then `type_name`. `resolve_container_view` marks the identity column. Rule [N+33] validation catches a dangling `identityFieldId`.

**Agent:** Repository Service Worker

#### Tasks

**3a — `Package::effective_identity_field_id`** (`crates/srs-repository/src/package.rs`, near `effective_fields` ~line 180):

- [x] **Extract a shared ancestor-chain helper first.** `effective_fields` (lines 197-226) currently walks the inheritance chain (cycle detection via `visited: HashSet<String>`, `resolve_type` lookups, `TypeInheritanceCycle`/`TypeNotFound` errors) inline. Extract this into a private helper, e.g.:
  ```rust
  fn ancestor_chain(&self, record_type: &RecordType) -> Result<Vec<&RecordType>, crate::error::RepositoryError> {
  ```
  returning the chain from `record_type` up through its ancestors (or an empty/single-element result if it doesn't extend anything), with the exact same cycle-detection and error behavior `effective_fields` has today. Refactor `effective_fields` to use this helper instead of its inline walk — this must not change `effective_fields`'s existing behavior or any of its existing tests. Do not implement two independent copies of chain-walking; both `effective_fields` and the new `effective_identity_field_id` below must call this one helper.
- [x] Add:
  ```rust
  pub fn effective_identity_field_id(
      &self,
      record_type: &RecordType,
  ) -> Result<Option<String>, crate::error::RepositoryError> {
  ```
  Implementation: call `self.ancestor_chain(record_type)?`, then walk the returned chain from `record_type` itself upward through ancestors, returning the **first** `identity_field_id` found that is `Some` (Rule [N+34] — a Type's own value wins over any ancestor's; if `record_type` itself has none, the nearest ancestor's applies, transitively). Return `Ok(None)` if no Type in the chain declares one.
- [x] Add unit tests in `package.rs`'s test module: (a) a Type with its own `identityFieldId` returns it directly without walking the chain; (b) a derived Type with no own `identityFieldId` inherits the base Type's; (c) a three-level chain (grandchild → child → base) where only the base declares `identityFieldId` — grandchild resolves it transitively; (d) a derived Type overriding with its own `identityFieldId` (pointing at a field the derived Type itself adds) wins over the base's; (e) no Type in the chain declares one → `Ok(None)`; (f) a cycle returns `TypeInheritanceCycle`, matching `effective_fields`'s existing cycle test pattern.

**3b — Rule [N+33] validation** (`crates/srs-repository/src/validation.rs`, near the V7 lifecycle-ref check ~line 864-879):

- [x] Add a check, in the same style as the lifecycle-ref-resolution loop, iterating `pkg.record_types()`: for each `RecordType`, call `pkg.effective_identity_field_id(rt)`. **This call returns a `Result` — `match` it, do not propagate with `?`.** On `Err` (e.g. an unrelated inheritance cycle elsewhere in the chain), push a `ValidationDiagnostic` describing the resolution failure and continue to the next Type — do not abort the validation pass for the whole repository. This mirrors the existing `validation.rs:334-361` pattern for `effective_fields` errors (`plans/111-validate-accumulate-diagnostics.md`'s "accumulate, don't fail fast" principle), not a `?`-propagating pattern. On `Ok(Some(field_id))`, confirm `field_id` is present in `pkg.effective_fields(rt)?`'s resulting `Vec<FieldAssignment>` (by `field_id`) — if not present, push a `ValidationDiagnostic` (matching the existing diagnostic shape used by the V7/Inv-43 checks in this file — same `severity`/`relative_path`/`message` field pattern) with a message like `"Type {namespace}/{name}@{version}: identityFieldId {field_id} is not in the effective field set"`. On `Ok(None)`, no diagnostic.
- [x] This check runs independent of whether any Tier-2 Record of that Type exists (matching Inv-43's shape, not Inv-41's lazily-triggered shape — see the investigation note: Inv-41 only fires when a Record is validated; this is a pure Type-level invariant and should always run).
- [x] Add a test fixture repository (or extend an existing `validation.rs` test) with a Type whose `identityFieldId` does not resolve to any field in its effective set, and assert the diagnostic is produced. Add a second test confirming a *valid* `identityFieldId` (present in the effective set) produces no diagnostic. Add a third test confirming an `effective_identity_field_id` `Err` (e.g. a cycle) on one Type produces a diagnostic but does not prevent diagnostics from being collected for other, unrelated Types in the same repository.

**3c — `record_label.rs` rewrite:**

- [x] Add a new index builder alongside `build_field_name_index`. **`effective_identity_field_id` errors (e.g. an inheritance cycle on some unrelated Type in the package) MUST NOT propagate out of this function** — this index is consumed via `?` by all six call sites (task 3d), so a naive `?` here would hard-fail basic record listing/display across the entire read surface whenever *any* Type in the package has a broken chain, even for records of unrelated, perfectly valid Types. Skip (don't insert an entry for) any Type whose resolution errors — that Type's records simply fall through to the name-ladder heuristic, exactly as if it had no `identityFieldId` at all. Rule [N+33] validation (task 3b) is the correct, non-silent place a broken chain gets surfaced to the user; this index-builder's job is graceful degradation, not error reporting.
  ```rust
  pub(crate) fn build_identity_field_index(
      store: &dyn RepositoryStore,
  ) -> Result<HashMap<(String, u32), String>, RepositoryError> {
      let package = store.load_package()?;
      let mut index = HashMap::new();
      for rt in package.record_types() {
          if let Ok(Some(field_id)) = package.effective_identity_field_id(rt) {
              index.insert((rt.id.clone(), rt.version), field_id);
          }
      }
      Ok(index)
  }
  ```
  (Only Types with a resolved effective `identityFieldId` get an entry — absence, for any reason including an error, means "fall through to the name ladder.")
- [x] Add a unit test: a package containing one Type with a valid `identityFieldId` and a second, unrelated Type with a broken/cyclic `identityFieldId` chain → `build_identity_field_index` returns `Ok` with an entry for the valid Type and no entry (not an error) for the broken one.
- [x] Change `record_display_label`'s signature to:
  ```rust
  pub(crate) fn record_display_label(
      record: &Record,
      identity_field_index: &HashMap<(String, u32), String>,
      field_name_index: &HashMap<String, String>,
  ) -> String {
      if let Some(field_id) = identity_field_index.get(&(record.type_id.clone(), record.type_version)) {
          if let Some(fv) = record.field_values.iter().find(|fv| &fv.field_id == field_id) {
              if let Some(s) = fv.value.as_str() {
                  if !s.is_empty() {
                      return s.to_string();
                  }
              }
          }
      }
      for priority in &["title", "name", "label"] {
          for fv in &record.field_values {
              if field_name_index.get(&fv.field_id).map(|n| n.as_str()) == Some(priority) {
                  if let Some(s) = fv.value.as_str() {
                      return s.to_string();
                  }
              }
          }
      }
      record.type_name.clone()
  }
  ```
  (An identity field present but empty/non-string falls through to the name ladder, not straight to `type_name` — matches the existing name-ladder's own empty-string handling via `fv.value.as_str()`.)
- [x] Update this function's doc comment to describe the new three-tier priority: effective `identityFieldId` → name ladder (`title`/`name`/`label`) → `type_name`, citing RFC-020 Rule [N+36].
- [x] Add unit tests in `record_label.rs`: (a) a record whose Type has an `identityFieldId` set and the corresponding field has a value → returns that value, even when a `title`/`name`/`label` field is *also* present with a different value (identity wins); (b) a record whose Type has no `identityFieldId` → falls through to today's name-ladder behavior unchanged (regression test — existing behavior must not break); (c) identity field present in the index but the field's value is empty string → falls through to the name ladder.

**Ordering note: do 3e before 3d.** Task 3d's discovery/text-projection call-site updates depend on the `FieldTextIndex` accessor task 3e adds. Implement in the order 3a → 3b → 3c → **3e → 3d** → 3f.

**3e — `text_projection.rs`: extend `FieldTextIndex`:**

- [x] Locate `FieldTextIndex`'s constructor (the function that builds it — likely in `text_projection.rs` itself or wherever it's first constructed per-batch, e.g. alongside `discovery_service.rs`'s call site). Add a field `identity_field_ids: HashMap<(String, u32), String>`, populated the same way `build_identity_field_index` populates its map (reuse that function directly if `FieldTextIndex`'s constructor has `store`/`Package` access at that point — do not duplicate the resolution logic; call `record_label::build_identity_field_index(store)` and store the result).
- [x] Add `pub(crate) fn identity_field_ids(&self) -> &HashMap<(String, u32), String>` alongside the existing `pub(crate) fn names(&self) -> &HashMap<String, String>`.
- [x] Update `project_text`'s call to `record_display_label` to pass `index.identity_field_ids()` as the new middle argument.

**3d — Update all six call sites** to build and thread `identity_field_index` alongside the existing `field_name_index`, mirroring exactly how each site already threads `field_name_index` today (same build-once-per-batch pattern, same parameter-passing shape):

- [x] `crates/srs-repository/src/record_store.rs:608-626` (`list_record_summaries`) — add `let identity_field_index = record_label::build_identity_field_index(store)?;` next to the existing `field_name_index` build; pass both into `record_display_label`.
- [x] `crates/srs-repository/src/tree_service.rs` (`build_tree` ~line 63-64 builds; `build_node` ~line 141-165 threads and uses) — same pattern; `build_node`'s parameter list gains `identity_field_index: &HashMap<(String,u32), String>`.
- [x] `crates/srs-repository/src/repository_navigation_service.rs` (~line 70 builds; `node_for_record`/`display_label` ~line 127-134 thread and use) — same pattern.
- [x] `crates/srs-repository/src/container_view_service.rs` (~line 122-123 builds `field_name_index`; `resolve_member` ~line 218-222 threads and uses) — same pattern; `resolve_member`'s parameter list gains the new index.
- [x] `crates/srs-repository/src/discovery_service.rs:182` — this site currently uses `field_text_index.names()` (from `text_projection::FieldTextIndex`), not `record_label`'s own builder. Use the `identity_field_ids()` accessor added in 3e (e.g. `field_text_index.identity_field_ids()`), matching the existing `field_text_index.names()` call shape.
- [x] `crates/srs-repository/src/text_projection.rs:113` (`project_text`) — already updated as part of 3e's last sub-task; confirm it's done.

**3f — `ColumnSpec.is_identity_column`** (`crates/srs-repository/src/container_view_service.rs`, ADR-023 — read the ADR before implementing this task; it was corrected during plan review and its current text is authoritative over any earlier summary):

- [x] Add to `ColumnSpec` (~line 34-46):
  ```rust
  /// True when this column's fieldId is the resolved View's Type's effective
  /// `identityFieldId` (RFC-020), and that Type was unambiguously resolvable
  /// (see ADR-023). Never affects column order.
  pub is_identity_column: bool,
  ```
- [x] **Reuse the `identity_field_index` already built for this call, do not re-resolve.** `resolve_container_view` already builds `identity_field_index: HashMap<(String,u32), String>` once (task 3d's `container_view_service.rs` bullet, alongside the existing `field_name_index` build at ~line 122-123) and threads it to `resolve_member`. Thread that same index into `resolve_columns` too — add an `identity_field_index: &HashMap<(String, u32), String>` parameter, mirroring exactly how `field_name_index` is already a parameter of `resolve_columns` today. This avoids both a second `Package` load *and* a second, independent `effective_identity_field_id` resolution (the earlier draft of this task called `package.resolve_type`/`package.effective_identity_field_id` directly inside `resolve_columns`, duplicating work `build_identity_field_index` already did once for the whole container-view resolution — don't do that).
- [x] In `resolve_columns` (~line 258-312), which already receives `dv: &DocumentView`: check `dv.root_type_refs`. If it is `Some(refs)` with `refs.len() == 1`, look up `identity_field_index.get(&(refs[0].type_id.clone(), refs[0].type_version))`. If that returns `Some(field_id)`, set `is_identity_column: fv.field_id == *field_id` on each constructed `ColumnSpec`; otherwise (any of: `root_type_refs` absent/empty/multi-entry, or no entry in `identity_field_index` for that Type — which covers "Type declares no `identityFieldId`," "Type not found," and "resolution errored," since `build_identity_field_index` (task 3c) already collapses all of those to "no entry") set `is_identity_column: false` on every column. No diagnostic is pushed for any of these cases — they're all normal, expected outcomes of a lookup miss, not new failures introduced by this task (a genuine resolution error was already surfaced once, non-silently, by Rule [N+33] validation at index-build time, not re-surfaced here).
- [x] Add tests in `container_view_service.rs`'s test module: (a) single-Type container (`root_type_refs` with exactly one entry) whose Type declares `identityFieldId` pointing at a field that IS one of the resolved View's columns → that column's `is_identity_column` is `true`, all others `false`; (b) same but the Type declares no `identityFieldId` → all columns `false` (regression, unchanged from today); (c) `root_type_refs` with zero or more than one entry → all columns `false`; (d) `input.view_id` supplied explicitly (the branch that skips `document_views_for_container` entirely, per `container_view_service.rs:126-135`) → confirm this still resolves `is_identity_column` correctly via `dv.root_type_refs` on the explicitly-referenced `DocumentView` (not via the skipped root-record-type lookup) — this is the case the original plan draft missed entirely.

#### Acceptance Criteria

- [x] `Package::effective_identity_field_id` resolves own → inherited (transitively) → `None`, matching RFC-020 Rule [N+34]; all 6 unit tests in 3a pass.
- [x] Rule [N+33] validation fires on a dangling `identityFieldId` and stays silent on a valid one; an `Err` from `effective_identity_field_id` on one Type does not abort diagnostics collection for other Types (accumulate, don't fail fast); all three `validation.rs` tests pass.
- [x] `build_identity_field_index` never propagates an `effective_identity_field_id` error — a broken chain on one Type is skipped (no index entry), not a hard failure of the whole index build; the new 3c index-builder test passes.
- [x] `record_display_label` prefers the effective identity field, falls through to the name ladder on absence or empty value, and preserves 100% of existing name-ladder-only behavior when no `identityFieldId` is set (regression test 3c-b passes unmodified in assertions).
- [x] All six call sites compile and pass their existing test suites with no behavior change for repositories that declare no `identityFieldId` (this is the critical backward-compatibility bar — every existing test in `record_store.rs`, `tree_service.rs`, `discovery_service.rs`, `repository_navigation_service.rs`, `container_view_service.rs`, `text_projection.rs` must still pass unmodified).
- [x] `ColumnSpec.is_identity_column` is `true` exactly for the column matching the effective `identityFieldId` when the container's View is anchored to exactly one Type via `root_type_refs`; `false` for every column when that resolution is ambiguous (zero or multiple `root_type_refs` entries) or the index has no entry for that Type; column **order** unchanged from before this plan in every case (ADR-023, as corrected during plan review).
- [x] `resolve_columns` looks up `is_identity_column` from the shared `identity_field_index` (built once in `resolve_container_view`) rather than independently calling `Package::resolve_type`/`Package::effective_identity_field_id` — no duplicated resolution work, no second `Package` load.

#### Testing

```bash
cargo test -p srs-repository
cargo test -p srs-repository record_label
cargo test -p srs-repository package::tests
cargo test -p srs-repository validation
cargo test -p srs-repository container_view_service
```

Specific tests to write or verify (see task lists above for exact cases):
- `package.rs`: `effective_identity_field_id_*` (own, single-level inherit, transitive inherit, override, none, cycle)
- `validation.rs`: identity-field-id validation, dangling and valid cases
- `record_label.rs`: identity-field priority, name-ladder regression, empty-value fallthrough
- `container_view_service.rs`: `is_identity_column` true/false cases

#### Milestone gate

1. Verify all acceptance criteria above — check each checkbox.
2. Confirm every listed test exists and passes.
3. `cargo test -p srs-repository && cargo clippy -p srs-repository -- -D warnings`
4. Mark checkboxes `[x]`, commit: `git commit -m "feat(srs-repository): schema-driven record_display_label via identityFieldId (#376)"`.

---

### Phase 4: Verification

**Goal:** The whole workspace is green; no crate-boundary or duplication regressions; the schema/payload contracts hold.

**Agent:** Verification Agent

#### Tasks

- [x] Run the full workspace test suite and lint.
- [x] Confirm `srs-bindings` has no duplicated `record_display_label`/identity-resolution logic — it must call the same `srs-repository` services this plan modified, inheriting the new behavior for free.
- [x] Confirm `cargo test --test payload_contracts` passes with **no schema diff** (per the Contracts section — `ContainerView` is opaque in the payload, so this is expected to be a no-op regeneration).
- [x] Confirm `bash scripts/check-schema-sync.sh`'s `srs-rust` half passes (the `srs-vscode` half is expected to still show drift — note this explicitly, don't let it read as "passed" if it silently isn't checked).
- [x] Produce a crate-boundary audit: confirm no business logic leaked into `srs-cli` handlers, no file I/O introduced into `srs-core`, and the `effective_identity_field_id`/Rule [N+33] logic lives only in `srs-repository`.
- [x] Produce a duplicated-logic report: confirm the six call sites all delegate to the same `record_label`/`text_projection` primitives rather than reimplementing identity-field lookup inline.

#### Acceptance Criteria

- [x] `cargo test` passes workspace-wide, zero failures.
- [x] `cargo clippy -- -D warnings` passes workspace-wide.
- [x] `cargo test --test payload_contracts` passes, no schema file changes generated.
- [x] `check-schema-sync.sh` — `srs-rust` clean; `srs-vscode` drift explicitly noted as out of scope, not silently passed over.
- [x] Crate-boundary audit and duplication report both come back clean (or file follow-up issues for anything found, per `/ship` Stage 3.4 conventions).

#### Testing

```bash
cargo test
cargo clippy -- -D warnings
cargo test --test payload_contracts
bash scripts/check-schema-sync.sh
```

#### Milestone gate

1. Verify all acceptance criteria.
2. Report findings (crate-boundary audit, duplication report, test transcript summary) back to the Lead Integrator / issue thread.
3. No commit required from this phase unless a fix is needed — if so, hand back to Repository Service Worker.

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] CLI output format unchanged for repositories without `identityFieldId` (integration tests pass)
- [x] `cargo test --test payload_contracts` passes (no schema regeneration needed — `ContainerView` is opaque)
- [x] `bash scripts/check-schema-sync.sh` — `srs-rust` half exits 0 (srs-vscode drift noted, out of scope)
- [x] `Package::effective_identity_field_id` correctly cascades per RFC-020 Rule [N+34] (own → inherited transitively → none)
- [x] `record_display_label` priority order matches RFC-020 Rule [N+36] exactly: identity field → name ladder → type_name
- [x] `ColumnSpec.is_identity_column` never alters column order (ADR-023)
- [x] Rule [N+33] validation fires on a dangling `identityFieldId`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- RFC-020 (srs#144 / srs#148) content is stable at the worktree `/home/greenman/dev/semanticops/.worktrees/rfc-020-type-level-identity-field` for the duration of this plan's Phase 1 schema sync. If srs#148 merges to `srs` `master` before Phase 1 runs, sync from `../srs` (default) instead — the content is identical either way.
- No other in-flight srs-rust branch is concurrently modifying `record_label.rs`, `package.rs`'s `effective_fields`, or `container_view_service.rs`'s `ColumnSpec` in a conflicting way. Rebase and re-run the milestone gates if the sync in Stage 6 of `/ship` surfaces conflicts.
- The dogfooding scenario for this issue (Stage 7.6 of `/ship`) will use a fixture repository whose primary field is not named `title`/`name`/`label` — mirroring the RFC's own muDemocracy `heading` field repro — to prove the fix end-to-end.
