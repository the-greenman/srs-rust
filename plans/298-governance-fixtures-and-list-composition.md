# Plan: Regenerate governance fixtures + wire `srs-gov list` composition (#298)

> Sections **3c** and **4** of [`plans/srs-gov-tui-foundation.md`](srs-gov-tui-foundation.md). Built on
> branch `feat/srs-gov-tui-foundation`, which already carries the RFC-012 discovery foundation
> (`srs find`, `discovery_service`, `text_projection`, the `excludeLifecycleStates` axis — epic #212).

## Summary

RFC-012 Rev 7 added the `excludeLifecycleStates` discovery axis, and the canonical
`com.mudemocracy.governance@1.0.0` decision-log `DocumentView` was re-authored as a `type-query`
declaring `excludeLifecycleStates: [superseded, closed]` — the authored "default-hidden" set an
interactive list consumes. Two **derived** copies of that view still carry the old `container-subset`
source: the srs-gov runtime seed (`crates/srs-gov/assets/governance-seed.srsj`) and the spec gallery
example (`srs/docs/spec/examples/gallery-project-v2/...`). This plan (a) regenerates the seed
deterministically (ADR-017) so srs-gov sees the authored default-hidden states, (b) surfaces the
authored `excludeLifecycleStates` through the `container resolve-view` payload so clients never
re-derive list semantics (capability-layering), and (c) wires `srs-gov list` to compose
`resolve-view` (columns + ordered members) with `srs find` (lifecycle/tag/text query) — adding
`--search`, `--tag`, and `--all` (show-all) flags. The coordinated `srs`-repo fixture work (regenerate
seed, update gallery, re-render) lands on the existing srs branch `feat/rfc-012-exclude-lifecycle-states`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Core Model Worker | `agents.md#core-model-worker` (SectionSource already in place; no change expected) |
| Repository Service Worker | `agents.md#repository-service-worker` (resolve-view exclusion extraction) |
| CLI Worker | `agents.md#cli-worker` (payload field + schema regen) |
| srs-gov Worker | Lead (srs-gov is a thin client crate; not in the standard role table — treated as CLI-client work under Lead) |
| Verification | `agents.md#verification-agent` |
| Reviewers | `agents.md#architecture-reviewer`, `agents.md#plan-reviewer` |

See [agents.md](agents.md) for role definitions. **No new role required** — srs-gov is a thin client
crate driven by the Lead Integrator; all semantic work falls under the existing Repository/CLI roles.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-011](../docs/adr/011-cli-output-contract.md) | All CLI output is a named payload struct; `ContainerView` field addition regenerates the golden schema. | accepted |
| [ADR-017](../docs/adr/017-deterministic-srsj-serialization.md) | `.srsj` is a deterministic artifact — the seed is **regenerated**, never hand-edited. | accepted |
| [ADR-018](../docs/adr/018-container-view-column-source-precedence.md) | Governs which `DocumentSection` drives a resolved container view; this plan reuses that same section selection to source the authored exclusion list. | accepted |
| [ADR-019](../docs/adr/019-discovery-service.md) | `discovery_service::find` is the single discovery entry point; `srs-gov list` composes it, it does not re-implement filtering. | accepted |
| [ADR-020](../docs/adr/020-resolve-view-authored-list-defaults.md) | **NEW** — `container resolve-view` is the single surface that carries a container's authored list defaults (columns **and** default-hidden lifecycle states); clients consume them and never re-derive list semantics from `DocumentView` sources. | proposed |
| `docs/architecture/capability-layering.md` | Semantics live once in `srs-repository`; srs-gov adds presentation/forwarding only. | guide |

Design checkpoint resolved (user input, this session): surface `excludeLifecycleStates` in the
**core `ContainerView` payload** (not a client-side two-call `document-view get` composition).

---

## Contracts

### CLI output contract (ADR-011)

**Existing command payload changed:** `ContainerView` (payload for `container resolve-view`) gains
`exclude_lifecycle_states: Vec<String>`. **Note (golden-schema no-op):** `ContainerViewPayload`
embeds `ContainerView` as opaque via `#[schemars(with = "serde_json::Value")]`, so
`cargo run --bin generate-schemas` produces **no diff** for `container-resolve-view.json` — the new
field is *not* reflected in the golden schema and `payload_contracts` will not assert it. Still run
`generate-schemas` (to prove no unintended diff) but expect zero staged changes; the field is verified
instead by (a) the new `srs-repository` unit tests and (b) a live-output check:
`srs container resolve-view <id> --format json --pretty | jq .payload.containerView.excludeLifecycleStates`.
`srs find` already exists (no new command). Verification: `cargo test --test payload_contracts` (must
still pass, unchanged).

### Entity schema sync (check-schema-sync.sh)

**No** entity JSON Schema under `srs/docs/schema/2.0/` is modified by this plan. The
`type-query`/`excludeLifecycleStates` schema already exists on the branch (RFC-012 Rev 7). The seed
and gallery are **instance/example data**, not schemas. No mirror sync needed for this plan's edits.

> **Pre-existing cross-repo note (not introduced here):** the branch already synced `discovery.json`'s
> `excludeLifecycleStates` axis into the srs-rust mirror (commit `1e0f0db`), which is **ahead of srs
> `master`** until srs PR `feb6fcf` merges. The srs-rust `schema-drift` CI job (which diffs the mirror
> against srs `master`) will therefore report drift on `discovery.json` until the coordinated srs spec
> PR merges — the documented mirror/spec merge-order tension (srs-rust `CLAUDE.md` → Schema Sync). This
> plan neither causes nor fixes it; it is surfaced in the PR body for the human coordinating the merge.

---

## Scope

- Add `exclude_lifecycle_states: Vec<String>` to `ContainerView`; populate it in
  `resolve_container_view` from the **same** governing `DocumentSection` that ADR-018 selects for
  columns (empty when that section is not a `type-query` or declares no exclusion).
- Regenerate `crates/srs-gov/assets/governance-seed.srsj` deterministically from the canonical
  package (via the srs `scripts/build-governance-seed.mjs` path), vendoring the result; document the
  reproducible regeneration recipe.
- Add `--search <text>`, `--tag <tag>` (repeatable), and `--all` flags to `srs-gov list`; compose
  `resolve-view` members ∩ `find` hits, passing the authored `excludeLifecycleStates` to `find`
  (dropped under `--all`), `--search → find --text`, `--tag → find --tag`.
- Self-contained `crates/srs-gov/tests/flow.rs` coverage for default-hidden, `--all`, `--search`,
  `--tag` (constructs its own multi-state repo via `repo-create` + `srs` writes — **not** gallery-dependent).
- Coordinated **srs-repo** change on `feat/rfc-012-exclude-lifecycle-states`: regenerate
  `empty-governance-document.srsj`, update the gallery `decision-log` view to `type-query`, add
  decisions in `draft`/`ratified`/`superseded`/`closed`, re-render the gallery.

**Out of scope (deferred / filed as follow-ups):**

- RFC #213 authored *searchable-fields* / *default-sort* view metadata — `--search` maps generically
  to `find --text` (content recall over all searchable fields via `project_text`); no per-view
  searchable metadata is needed for #298. (Deferred to the srs-gov-tui-foundation 3c remainder / RFC #213.)
- TUI foundation and interactions (Sections 5–6 of the parent plan).
- Removing srs-gov's `containerType`/title container-matching heuristic (parent plan Section 4
  "Refactor" — file as follow-up; #298 keeps the existing `resolve_container_id`).

---

## Phases

### Phase 1: Surface authored `excludeLifecycleStates` through `resolve-view`

**Goal:** `srs container resolve-view <id>` returns the authored default-hidden lifecycle states for
the container, sourced from the same section ADR-018 already selects for columns.

**Agent:** Repository Service Worker + CLI Worker (payload).

#### Tasks

- [ ] In `crates/srs-repository/src/container_view_service.rs`: add a sibling function
      `select_governing_section(dv: &DocumentView, container_id: &str) -> Option<&DocumentSection>`
      that applies the ADR-018 precedence (container-targeting section first, then first-by-`order`
      with a `render_view_id`) and returns the chosen `&DocumentSection`. Extend the "targets this
      container" check to also recognise a `SectionSource::TypeQuery` whose `container_ids` contains
      the container (the canonical decision-log section is now `type-query`, not `container-subset`).
      Add an inline comment: if both a `ContainerSubset` and a `TypeQuery` target the container, the
      lower-`order` one wins (deterministic tie-break; matches the canonical single-section case).
      Reimplement `select_column_view_id` as `select_governing_section(...).and_then(|s| s.render_view_id.clone())`
      so columns and the exclusion list derive from **one** selection (no parallel precedence logic).
- [ ] Extract `exclude_lifecycle_states: Vec<String>` from the governing section's `SectionSource`
      (only `SectionSource::TypeQuery { exclude_lifecycle_states, .. }`; `Vec::new()` otherwise).
- [ ] Add `pub exclude_lifecycle_states: Vec<String>` to the `ContainerView` struct (after `columns`),
      and populate it in `resolve_container_view`.
- [ ] Run `cargo run --bin generate-schemas` and confirm **no** golden-schema diff (see Contracts note).
- [ ] Draft `docs/adr/020-resolve-view-authored-list-defaults.md` (status `proposed`) using
      `ADR-TEMPLATE.md`; add `Extended by: ADR-020` to ADR-018's header and a cross-reference note to
      ADR-019 §6 (extraction of authored defaults is delegated to the resolve-view service, not clients).

#### Acceptance Criteria

- [ ] resolve-view payload includes `excludeLifecycleStates` (camelCase JSON), `["superseded","closed"]`
      for the decision-log container, `[]` for a container-subset/articles container.
- [ ] Column resolution behaviour is unchanged (same `render_view_id` chosen as before).
- [ ] `cargo test --test payload_contracts` passes with the regenerated golden schema.

#### Testing

```bash
cargo test -p srs-repository container_view
cargo test --test payload_contracts
```

Specific tests (in `crates/srs-repository/src/container_view_service.rs` `#[cfg(test)]`):
- `resolve_view_surfaces_type_query_exclude_lifecycle_states` — a DocumentView whose governing
  section is a `type-query` with `excludeLifecycleStates` surfaces them on `ContainerView`.
- `resolve_view_exclude_lifecycle_states_empty_for_container_subset` — container-subset section ⇒ `[]`.
- `resolve_view_columns_unchanged_after_exclude_states_addition` — guards the acceptance criterion
  that `document_view_id`/`columns` output is identical before/after (no column regression).
- `resolve_view_roundtrip_type_query_exclude_states` — **cross-store** (memory → file) roundtrip with a
  `TypeQuery { container_ids, exclude_lifecycle_states }` fixture confirms the populated field survives
  (satisfies CLAUDE.md storage-boundary cross-store rule for the path that actually populates it).

#### Milestone gate

`cargo test -p srs-repository`, `cargo test --test payload_contracts`,
`cargo clippy -p srs-repository -p srs -- -D warnings`; mark checkboxes; commit `(#298)`.

---

### Phase 2: Regenerate the deterministic governance seed asset (3c)

**Goal:** `crates/srs-gov/assets/governance-seed.srsj` carries the canonical `type-query` +
`excludeLifecycleStates` decision-log view, produced by the reproducible export path, not hand-edited.

**Agent:** Lead Integrator (drives the srs regeneration script + vendors the artifact).

#### Tasks

- [ ] **(srs repo, `feat/rfc-012-exclude-lifecycle-states`)** Build `srs` from this worktree and run
      `SRS_BIN=<worktree>/target/debug/srs node scripts/build-governance-seed.mjs` to regenerate
      `packages/com.mudemocracy.governance/1.0.0/seed/empty-governance-document.srsj` from the
      now-`type-query` canonical package. Confirm `--check` reports byte-stability.
- [ ] Vendor the regenerated seed into `crates/srs-gov/assets/governance-seed.srsj` (byte-copy).
- [ ] Confirm the asset's `package/document-views/decision-log-b5c8d124.json` is now `type-query` with
      `excludeLifecycleStates: ["superseded","closed"]` (no remaining `container-subset` for that view).
- [ ] Document the regeneration recipe: a comment immediately above `const GOVERNANCE_SEED` in
      `crates/srs-gov/src/main.rs` giving the exact reproduction command
      (`SRS_BIN=<target>/debug/srs node ../srs/scripts/build-governance-seed.mjs` then byte-copy into
      `assets/governance-seed.srsj`), and a one-line cross-reference in `crates/srs-gov/README.md` if
      present (else in the dogfooding scenario added in Stage 7.6).

#### Acceptance Criteria additions

- [ ] The reproduction command appears verbatim in a `crates/srs-gov/src/main.rs` comment so a future
      maintainer can regenerate the asset without reading this plan.

#### Acceptance Criteria

- [ ] `srs-gov repo-create` from the regenerated seed validates (`srs repo validate` ⇒ 0 errors).
- [ ] A repo stamped from the seed exposes the decision-log container's `excludeLifecycleStates`
      through `container resolve-view`.
- [ ] `repo_create_produces_valid_srsj` (existing test) still passes.

#### Testing

```bash
cargo build --bin srs --bin srs-gov
cargo test -p srs-gov repo_create
```

Specific test:
- `seed_decision_log_view_is_type_query_with_excludes` — parse the embedded seed, assert the
  decision-log DocumentView section source is `type-query` and lists the two excluded states.

#### Milestone gate

`cargo test -p srs-gov`, `cargo clippy -p srs-gov -- -D warnings`; mark checkboxes; commit `(#298)`.

---

### Phase 3: Wire `srs-gov list` composition (Section 4)

**Goal:** `srs-gov <key> list` composes `resolve-view` + `find`: hides authored default-hidden states
by default, `--all` shows them, `--search`/`--tag` narrow the set; `--json`/`--explain` still work.

**Agent:** Lead Integrator (srs-gov client).

#### Tasks

- [ ] Add to the `List` subcommand in `crates/srs-gov/src/main.rs`: `--search <text>` (`Option<String>`),
      `--tag <tag>` (`Vec<String>`, repeatable, `ArgAction::Append`), `--all` (`bool`).
- [ ] In `cmd_list`: after `container resolve-view`, read `containerView.excludeLifecycleStates`.
      Build a `srs find` call scoped with global `--container <container-id>`, passing
      `--exclude-lifecycle-state <s>` for each authored state **unless `--all`**, `--text <search>`
      when `--search` is given (generic forwarding — `find` runs `text_projection` over all searchable
      record text incl. title + `decision_statement`; srs-gov adds **no** search semantics), and
      `--tag <t>` per `--tag`. Intersect resolve-view members with the `find` hit `instanceId` set;
      render the surviving members in resolve-view order with the resolve-view columns. When no runtime
      filters and no exclusions apply, output is unchanged.
- [ ] `--explain` prints both underlying commands (`container resolve-view` and the composed `find`)
      as runnable shell snippets. `--json` prints the **complete `srs find` envelope** (`{ok, command,
      payload, diagnostics}`, unintersected) — document this in the flag help text.
- [ ] Keep `resolve_container_id` as-is (heuristic removal is out of scope / follow-up).

#### Acceptance Criteria

- [ ] Default `srs-gov decision_log list` omits `superseded` and `closed` decisions.
- [ ] `--all` includes them.
- [ ] `--search <term>` narrows to records matching title or `decision_statement` (content recall);
      `--tag <t>` narrows to tagged records; combined filters AND together.
- [ ] `--json` and `--explain` remain non-interactive and useful.

#### Testing

```bash
cargo test -p srs-gov
```

Specific **self-contained** integration tests added to the existing `crates/srs-gov/tests/flow.rs`
(each spawns `srs-gov repo-create` into a temp `.srsj`, then drives `srs record create` to add
decisions in `draft`/`ratified`/`superseded`/`closed` via `SRS_BIN`; **no** gallery dependency — these
must pass in CI where srs is checked out at `master`):
- `list_hides_superseded_and_closed_by_default`
- `list_all_flag_shows_hidden_states`
- `list_search_narrows_by_content` (includes a non-title `decision_statement` match)
- `list_tag_narrows_by_tag`

The existing gallery-based read-only tests in `flow.rs` stay unchanged and continue to pass against the
`master` gallery (a `container-subset` view ⇒ `excludeLifecycleStates = []` ⇒ no filtering applied).

#### Milestone gate

`cargo test -p srs-gov`, `cargo clippy -p srs-gov -- -D warnings`; mark checkboxes; commit `(#298)`.

---

### Phase 4: Coordinated srs-repo gallery fixtures + re-render (srs branch)

**Goal:** The spec gallery example matches the canonical `type-query` view and exercises hide/show-all
with mixed lifecycle states; rendered output regenerated.

**Agent:** Lead Integrator, acting in the **srs repo** on branch `feat/rfc-012-exclude-lifecycle-states`.
This phase runs as a **separate srs PR**, coordinated with (but **not blocking**) the srs-rust PR —
srs-rust CI is self-contained (Phase 3 tests build their own fixtures; the gallery read-only tests run
against srs `master`). Execute it in the same session against the local `../srs` checkout; it does not
gate the srs-rust terminal state.

#### Tasks

- [ ] Update `srs/docs/spec/examples/gallery-project-v2/package/document-views/decision-log-b5c8d124.json`
      to the `type-query` source with `excludeLifecycleStates: ["superseded","closed"]`, matching the
      canonical package copy (preserve `renderViewId`, `titleFieldId`, `rootTypeRefs`).
- [ ] Add decisions in `draft`, `superseded`, and `closed` states to the gallery decisions (alongside
      the existing `ratified` ones); register them in the decisions container membership + manifest
      `instanceIndex`; add `supersedes` relation for the superseded one if natural.
- [ ] `srs repo validate --repo docs/spec/examples/gallery-project-v2` ⇒ 0 errors.
- [ ] `node scripts/render-spec.mjs`; commit regenerated rendered output if it changes.

#### Acceptance Criteria

- [ ] Gallery decision-log view is `type-query` and validates.
- [ ] Gallery contains ≥1 decision in each of `draft`/`ratified`/`superseded`/`closed`.
- [ ] `node scripts/validate-all.mjs` and `srs repo validate` pass.

#### Testing

```bash
# in srs/
node scripts/validate-all.mjs
srs repo validate --repo docs/spec/examples/gallery-project-v2 --pretty
node scripts/render-spec.mjs
```

> This phase produces a **separate srs PR** on `feat/rfc-012-exclude-lifecycle-states`. The srs-rust PR
> does not depend on it for CI (Phase 3 tests are self-contained), but the two are linked for review.

#### Milestone gate

srs validations pass; commit on the srs branch.

---

## Final Acceptance

- [ ] `cargo test` passes (full srs-rust workspace).
- [ ] `cargo clippy -- -D warnings` and `cargo fmt --check` pass (CI `lint` job mirror).
- [ ] `cargo test --test payload_contracts` passes (unchanged — ContainerView field is golden-schema-opaque).
- [ ] `bash scripts/check-schema-sync.sh` — no entity schema changed by this plan (pre-existing
      `discovery.json` mirror/spec ordering noted in Contracts; surfaced in PR, not a #298 regression).
- [ ] `srs-gov decision_log list` hides `superseded`/`closed` by default and `--all` reveals them,
      verified end-to-end on a stamped seed repo (dogfooding).
- [ ] Regenerated seed asset validates and is reproducible from `build-governance-seed.mjs`.
- [ ] srs gallery updated + re-rendered on the srs branch (separate PR).

## Coordination Rules

- All srs-rust work on `feat/srs-gov-tui-foundation`; all srs work on `feat/rfc-012-exclude-lifecycle-states`.
- srs-gov stays a thin client: no lifecycle/filter semantics in srs-gov beyond forwarding authored
  values to `find` (capability-layering; ADR-019). Picking the governing section and the exclusion
  list happens in `srs-repository`.
- Phase 3 tests must not depend on the spec gallery (CI checks out srs `master`, which lags the
  gallery change) — construct fixtures in-test.

## Assumptions

- The branch's `srs find` / `discovery_service` / `excludeLifecycleStates` foundation is correct and
  already tested (epic #212); this plan consumes it.
- `srs repo copy` export determinism (ADR-017) and `build-governance-seed.mjs` remain the seed recipe.
- `resolve-view` already auto-selects the decision-log DocumentView for the decision_log container
  (verified: `document_views_for_container` matches on `rootTypeRefs`).
