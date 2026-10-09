# Plan: type_schema `title` comes only from displayLabel (#1382)

> **Usage note:** This plan is for agents to review and run. It uses unambiguous tasks, explicit
> file paths, named functions and checkable acceptance criteria.

**Answers:** SP-05 (via srs-web#547, parent story semanticops.com#22). **Issue:** the-greenman/srs-rust#1382.
**Branch:** `feat/1382-type-schema-title` (worktree `~/dev/.wt/srs-rust/1382-type-schema-title`, off `origin/master` a34dcc8b).
**Classification:** Mode: clear · Door 2 (a change in behaviour of a machine-facing projection that other repos consume; the payload struct does not change) · owner-merge.

---

## Decisions for the owner

> **Owner rulings (2026-10-09), taken one at a time:** (a) **A1**: omit `title` when no `displayLabel` is authored. (b) **B1**: amend ADR-026 with the drafted amendment. (c) **C1**: version note in the PR body and the ADR amendment only. (d) At landing, file the srs-vscode cleanup issue under semanticops.com#22, and comment on srs-web#547 naming the build that carries the fix.


Phase 1 cannot start until the owner rules on (a) and (b). (c) and (d) have defaults that the plan
applies unless the owner overrides them.

### (a) What a property carries when its FieldAssignment has no `displayLabel`

Today `field_to_property` (`crates/srs-repository/src/type_schema_service.rs:147-161`) sets
`title = displayLabel ?? Field.description`. The description is also written to `x-srs-description` (line 168).

| Option | What it does | Trade-offs | Consumers affected |
|---|---|---|---|
| **A1. Omit `title`** (recommended) | `title` is present only when a non-empty `displayLabel` is authored. `x-srs-description` is unchanged. | **For:** smallest diff, about 10 lines deleted. It matches the spec's own JSON Schema emitter, RFC-035 `srs/scripts/lib/schema-emitter.mjs:516` (`if (a.displayLabel) frag.title = a.displayLabel;`), and this repo's validation projection `srs-projection/src/json_schema.rs:180-183` (title only from `displayLabel`). Under it, "`title` present" means "a label was authored", the same never-inferred rule as ADR-044. It adds no new key and no new rule. **Against:** every client must have its own fallback for a missing title. All three known clients already have one. | srs-web `fieldLabel` (already `title \|\| humanise(name)`): improves with no code change. srs-vscode `looksLikeShortLabel` heuristic: still works, becomes dead weight. srs-gov TUI `detail_rows` (`title` else `name`): shows the raw field name in place of the description sentence. MCP agents: the description is still in `x-srs-description`. |
| A2. Core-resolved label key `x-srs-label` = `displayLabel ?? Field.name`, with `title` = `displayLabel` only | Every property carries a label the core has resolved, following the core render rule (`render_service.rs:3391`, spec "Step 3 — Labels: use displayLabel; fall back to Field.name"). | **For:** one label rule for every client (layer test, ADR-048 rule 2). **Against:** it creates a second key for the goal `title` already serves (rule 3, one way per goal). It is still the raw `Field.name`, so srs-web would keep humanising it, and the fallback lives in two places anyway. It is a new vendor key that the ecosystem must keep. | The same as A1, plus every client must switch to the new key to benefit. |
| A3. `title` = `displayLabel ?? Field.name` (raw), or a humanised name in the core | `title` is always present. | **For:** clients never miss a title. **Against:** a client cannot tell an authored label from a fallback. That is the same category of error as today, only milder. Humanising in the core is a new presentation rule with no spec backing (the spec humanises only relation keys, srs-spec §Humanization). That would make it a spec change (ADR-048 rule 1). srs-web's `humanise(name)` would never run again, so its labels get worse (`ratified_at` in place of `Ratified at`). | srs-web labels get worse unless it adds its own heuristic. srs-vscode is unchanged. |

**Recommendation: A1.** It deletes the overloading in place of re-encoding it, it matches both the
spec emitter and our own validation projection, and no client needs a change to stay correct.

### (b) What happens to ADR-026

ADR-026 states the fallback as context ("falling back to the field's `description` when no label is
set") and admits that it "entangles `description` with `title`". Its decision is the two vendor keys, and that decision still stands.

| Option | Trade-offs |
|---|---|
| **B1. Amend ADR-026** (recommended) | **For:** the decision (the `x-srs-*` help keys) is unchanged; only the description of the `title` slot is corrected. There is an in-repo precedent: the ADR-048 amendment section. History stays in one file. **Against:** the original Context paragraph stays in place and needs the amendment beside it. |
| B2. New ADR-055 "type-schema `title` is the authored label only", superseding ADR-026's title clause | **For:** a standalone record that is easy to cite from clients. **Against:** an ADR that only partly supersedes another is confusing (ADR-026's main decision is still accepted), and it adds a file. |

**Recommendation: B1.** Draft text, to be appended to `docs/adr/026-type-schema-field-help-keys.md`:

```markdown
## Amendment (2026-10-09, #1382) — `title` is the authored label only

The Context above records that `title` fell back to the field's `description` when no
`displayLabel` was set. That fallback is withdrawn. A property's `title` is emitted **only** from a
non-empty `FieldAssignment.displayLabel`. With no label authored, `title` is absent. The field's
`description` lives only in `x-srs-description`, as the Decision above already provides.

**Why.** The fallback made a description indistinguishable from a label. Every schema-driven form
showed a sentence where a label belongs (srs-web#547). The only client-side defence was a
length-and-word-count heuristic (srs-vscode `looksLikeShortLabel`), which also dropped legitimately
long authored labels. A present `title` now means "a label was authored", the never-inferred rule of
ADR-044. A client with no `title` renders its own presentation of `Field.name` (the property key), as
the spec's label rule does ("use `displayLabel`; fall back to `Field.name`").

**Consistency.** This matches the spec's JSON Schema emitter (RFC-035, `scripts/lib/schema-emitter.mjs`:
`if (a.displayLabel) frag.title = a.displayLabel`) and this repo's validation projection
(`srs-projection::json_schema`, `title` from `displayLabel` only). The editor-facing and validation
projections now agree on `title`.

**Contract.** `TypeSchemaPayload.schema` is opaque (ADR-011), so no payload golden changes. This is
a behaviour change inside the projection: consumers that used `title` as a description lose it and
must read `x-srs-description`. No first-party consumer does (checked 2026-10-09: srs-web, srs-vscode,
srs-gov, muDemocracy.org).
```

The Neutral bullet "Group-level `title`/`description` … are unchanged" stays as it is. Composite
sub-fields resolve through the same `field_to_property`, so the amendment covers them without
further wording.

### (c) Coordinated changes and the version note

- **Corpora:** none. This is a read projection. No stored data changes, there is no `dataModelRevision` bump, and no migration.
- **Payload golden:** `crates/srs-cli/schemas/payload/type-schema.json` declares `schema` as an opaque
  value (`payload.rs:1192-1197`, `#[schemars(with = "serde_json::Value")]`). `cargo run --bin generate-schemas`
  must produce **no diff**, and the plan checks that.
- **Fixtures / committed snapshots:** none hold type-schema output (no non-`.rs` file outside `docs/` and `plans/` contains `x-srs-order`).
  `crates/srs-mcp/tests/resources.rs:538` compares against a live `type_schema()` call, so it follows automatically.
- **Is it breaking?** For the JSON contract it is not: `title` was always optional, and every known reader treats it as optional.
  As behaviour, a reader that relied on `title` == description changes, and none is known.

| Option | Trade-offs |
|---|---|
| **C1. Version note in the PR body and the ADR-026 amendment only** (recommended) | No CHANGELOG file exists in this repo. Release notes come from PR titles. The ADR amendment is the durable record. |
| C2. Add a schema-level marker (for example `x-srs-projection-version`) so clients can branch | YAGNI. It adds a key that no client needs, because every client already handles a missing `title`. |

**Recommendation: C1.** The PR body names the behaviour change in one line: "`type schema`: `title`
is now present only when `displayLabel` is authored; the description stays in `x-srs-description`."

### (d) Follow-up issues in the sibling clients

| Client | Needed? | Recommendation |
|---|---|---|
| srs-web | **No code change.** `src/lib/labels.ts:44` `fieldLabel = title \|\| humanise(name)` and `src/lib/editor/blueprint-fields.ts:63,95` already do the right thing. srs-web picks up the change when its WASM pin moves (`scripts/ensure-bindings.mjs:29`, currently `v0.1.0-build.502`). | No new issue. Add a comment on srs-web#547 naming the srs-rust build that carries the fix, so the pin bump on #547 closes the symptom. |
| srs-vscode | **Cleanup, not a fix.** `src/cli/typeFields.ts:60-69` `looksLikeShortLabel` exists only because of this fallback. It also drops authored labels longer than 6 words or 50 characters. Its test `test/suite/typeFields.test.ts:21` encodes the old behaviour. | **File one issue** (srs-vscode, parented to semanticops.com#22): delete the heuristic, use `title ?? name`, and replace the test with "no title → field name". Note in the same issue that `typeFields.ts` reads `node.description` (string aiGuidance) as the help text, where it should read `x-srs-description` (ADR-026). The issue is filed when this PR lands (ADR-048 rule 4). |
| muDemocracy.org | No consumer of the type-schema `title` (grep 2026-10-09). | None. |
| srs spec | None. `srs-usage.md:1815` does not describe `title`. | None. |

---

## Spec gate (Stage 1.5)

**Verdict: the spec does not govern this. It is an implementation choice recorded by ADR-026.** The editor-facing
`type schema` projection is not specified by any SRS spec text (srs-rust#770 keeps it separate from the
spec-backed validation projection). The spec's own JSON Schema emitter (RFC-035, `schema-emitter.mjs:516`)
already sets `title` from `displayLabel` only. The spec's rendering label rules fall back to `Field.name`,
never to the description (srs-spec "Step 3 — Labels"; composite field rows `displayLabel -> Field.name -> fieldId`).
So the fix moves the implementation *toward* the spec, and no RFC is needed.

## Summary

`type schema` (CLI), WASM `type_schema` and the MCP `type_schema` tool / `srs://…/type/{id}` resource all
call `type_schema_service::type_schema`. When a FieldAssignment has no `displayLabel`, that service sets
the property's `title` to the field's `description`. As a result, every schema-driven form shows a sentence as a
label, and no client can tell a label from a description without a heuristic. This plan removes the
fallback at the one shared function (`field_to_property`), which fixes all three adapters and every
composite sub-field at once. It also amends ADR-026 to match.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | /ship session |
| Repository Worker | /ship session (one function plus its tests) |
| CLI Worker | /ship session (integration test only, no handler or payload change) |
| Verification | /ship session |

The change is one function in one crate, so there are no parallel workers.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-026](../docs/adr/026-type-schema-field-help-keys.md) | Amended: `title` comes only from `displayLabel`. The description lives only in `x-srs-description`. | accepted → amended (decision (b)) |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | `TypeSchemaPayload.schema` is opaque, so the golden does not change | accepted (governs) |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) / [ADR-037](../docs/adr/037-mcp-adapter-surface.md) | Bindings and MCP are one-call adapters, so the fix lands once in the service | accepted (governs) |
| [ADR-044](../docs/adr/044-navigation-identity-optional-never-inferred.md) | Never infer a value for an optional slot. Absent means not authored | accepted (principle cited) |
| [ADR-018](../docs/adr/018-container-view-column-source-precedence.md) | Precedent: a column label is `display_label` else `name`, never the description | accepted (precedent) |
| [ADR-048](../docs/adr/048-implementation-decision-rules.md) | Rules 1–4 below | accepted |

ADR-048 check: (1) **Spec-first:** RFC-035 emitter rule `title ← displayLabel`, and the spec label rule falls
back to `Field.name`. (2) **Layer:** the core service, once. The adapters are untouched, and the client fallback
(`humanise(name)`) is presentation. (3) **One way per goal:** removes a second meaning from `title`, and adds
no key. (4) **Parity/mirror:** no entity-schema mirror is involved and no payload golden changes. The srs-web pin bump goes
through srs-web#547, and the srs-vscode heuristic cleanup gets a follow-up issue (decision (d)).

---

## Contracts

### CLI output contract (ADR-011)

No payload struct changes. `TypeSchemaPayload.schema` is `#[schemars(with = "serde_json::Value")]`, so
`cargo run --bin generate-schemas` must leave `crates/srs-cli/schemas/payload/` with no diff. This is a
behaviour change inside the opaque value and is recorded in the ADR-026 amendment and the PR body (decision (c)).

### Entity schema sync (check-schema-sync.sh)

No change to `srs/docs/schema/2.0/`. No action.

---

## Scope

- `field_to_property` in `crates/srs-repository/src/type_schema_service.rs`: `title` only from a non-empty `assignment.display_label`.
- Update the service unit test, plus one CLI integration assertion and one bindings regression assertion.
- Append the ADR-026 amendment.
- Add a dogfood step to `docs/dogfooding.md`.

**Out of scope:**

- Any new label key (the A2/A3 options), unless the owner picks them under decision (a).
- `FieldAssignmentOverride.display_label` handling in `Package::effective_fields`, which is unchanged and only consumed here.
- MCP tool description text (`DESC_TYPE_SCHEMA` does not mention `title`).
- Client code in srs-web and srs-vscode (decision (d): a comment and a follow-up issue only).
- The string-aiGuidance → JSON Schema `description` mapping (lines 191-195), which is unchanged.

---

## Phases

### Phase 1: Remove the description fallback from `title`

**Goal:** a property with no authored `displayLabel` has no `title`, and `x-srs-description` still holds the description.

**Agent:** Repository Worker

**Write scope:** `crates/srs-repository/src/type_schema_service.rs` only.

#### Tasks

- [ ] In `field_to_property` (`type_schema_service.rs:147-161`), replace the `title` block with:
      `if let Some(label) = assignment.display_label.as_deref().filter(|s| !s.is_empty()) { prop.insert("title".into(), json!(label)); }`
      and change the comment to `// title: the authored displayLabel only — never the description (ADR-026 amendment, #1382).`
- [ ] Update the comment at lines 163-166 so it no longer implies that `title` can carry help text.
- [ ] Rename the test `type_schema_title_prefers_display_label` (line 1011) to `type_schema_title_only_from_display_label`, and change its assertions:
      `a.title == "Custom Label"`; `b.get("title").is_none()`; `b["x-srs-description"] == "b description"`.
- [ ] Add the test `type_schema_empty_display_label_emits_no_title`: an assignment with `display_label = Some("")` → no `title` key, and `x-srs-description` is present.

#### Acceptance Criteria

- [ ] A field with a description and no `displayLabel` has no `title` key and has `x-srs-description` == its description.
- [ ] A field with a `displayLabel` keeps `title` == the label.
- [ ] An empty `displayLabel` emits no `title`.

#### Testing

```bash
cargo test -p srs-repository type_schema
```

- `type_schema_title_only_from_display_label`: the label wins, and with no label there is no title and the description stays in `x-srs-description`.
- `type_schema_empty_display_label_emits_no_title`: an empty label counts as absent.

#### Milestone gate

1. Check every acceptance criterion above.
2. Run `cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`, each judged by its exit code.
3. Mark the checkboxes and commit (signed, plain `git commit`).

### Phase 2: Prove it at the adapters (CLI + WASM)

**Goal:** the issue's acceptance holds end to end through `srs type schema`, and the WASM-path test guards that labelled fields are unchanged.

**Agent:** CLI Worker + Bindings Worker (tests only)

**Write scope:** `crates/srs-cli/tests/integration_tests.rs`, `crates/srs-bindings/tests/type_schema.rs`.

#### Tasks

- [ ] In `type_schema_emits_draft07_for_record_field_values` (`integration_tests.rs:6858`), remove `"displayLabel": "Status"` from the status assignment (the one at line ~6921). Make sure the status field definition carries a non-empty `description` (add `"description": "Current status of the decision"` if it is missing). Then add:
      `assert!(schema["properties"]["status"].get("title").is_none(), "no displayLabel → no title (#1382)");`
      and `assert_eq!(schema["properties"]["status"]["x-srs-description"], "<that description>");`.
      Keep the existing `title.title == "Title"` assertion.
- [ ] In `crates/srs-bindings/tests/type_schema.rs::type_schema_resolves_latest_version`, add one regression assertion on the gallery decision type: the property for field `73cd845a-…` (displayLabel "Decision Question") has `title == "Decision Question"`. Resolve its key from the schema by `x-srs-order == 2` or by the field name in the gallery, whichever the test can do without a new helper. This proves the labelled path on a real corpus through the same call the WASM `type_schema` binding makes (the binding is a one-call adapter, ADR-013, so the service tests are the WASM proof for the unlabelled case).

#### Acceptance Criteria

- [ ] `srs type schema <typeId>` returns no `title` for an unlabelled field and keeps `x-srs-description`.
- [ ] Labelled fields keep their `title` on the gallery corpus.

#### Testing

```bash
cargo test -p srs-cli --test integration_tests type_schema_emits_draft07_for_record_field_values
cargo test -p srs-bindings --test type_schema
cargo run --bin generate-schemas && git diff --exit-code crates/srs-cli/schemas/payload/
cargo test -p srs-cli --test payload_contracts
```

#### Milestone gate

Same as Phase 1 (`cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`), plus the golden diff check above exits 0. Mark the checkboxes and commit.

### Phase 3: Record the decision and dogfood

**Goal:** ADR-026 reflects the new rule, and a live CLI run confirms it.

**Agent:** Lead Integrator

**Write scope:** `docs/adr/026-type-schema-field-help-keys.md`, `docs/dogfooding.md`, this plan.

#### Tasks

- [ ] Append the amendment text from decision (b) to ADR-026 (or write ADR-055 if the owner picks B2).
- [ ] In `docs/dogfooding.md`, in the type-schema scenario (step 7 / the bullets near line 94), add: "A field assigned **without** `displayLabel` carries **no** `title` in its schema property; its `description` is only in `x-srs-description` (#1382)."
- [ ] Dogfood: in a scratch repo, run `field create` (with a description) → `type create` (no displayLabel) → `srs type schema <typeId> --pretty`. Confirm there is no `title` and `x-srs-description` is present. Record "Verified 2026-10-…" under the scenario.

#### Acceptance Criteria

- [ ] ADR-026 carries the amendment, and its Context no longer reads as current behaviour without that amendment.
- [ ] The dogfood run is recorded.

#### Milestone gate

Run the full workspace gates again. Mark the checkboxes and commit.

---

## Final Acceptance

- [ ] `cargo build --workspace` exits 0
- [ ] `cargo test --workspace` exits 0 (zero failures)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo test --test payload_contracts` passes, and `generate-schemas` leaves no diff
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] For a field with a description and no `displayLabel`, `srs type schema` and the service behind WASM `type_schema` return no `title`, and `x-srs-description` holds the description
- [ ] ADR-026 amended (or ADR-055 written), per the owner's ruling
- [ ] At landing: the srs-vscode follow-up issue is filed and parented to semanticops.com#22, and a comment on srs-web#547 names the build

## Coordination Rules

- Agents keep to their write scopes unless the Lead Integrator expands them.
- Agents must not revert edits made by others.
- Workers return the changed file paths and a short summary of the behaviour change when done.
- At the end of each phase: run the milestone gate, update the checkboxes, commit, and only then move on.
- Push the branch only. Review the diff before opening the PR. The PR carries `gate:owner-merge` (Door 2).

## Assumptions

- No first-party consumer reads `title` as a description. Inventory, checked 2026-10-09:
  - srs-rust:
    - `type_schema_service.rs:147-161`: the producer.
    - `:1011-1039`: the unit test.
    - `srs-cli/tests/integration_tests.rs:6950` (`title.title == "Title"`, labelled, unaffected).
    - `srs-gov/src/tui_data.rs:258` (`title` else `name`).
    - `srs-mcp/tests/resources.rs:538-546` (compares against a live call).
    - `blueprint_schema_service.rs:100` (passes `type_schema` output through).
  - srs-web: `src/lib/labels.ts:44`, `src/lib/editor/blueprint-fields.ts:63,95`, `src/lib/srs-client.ts` `SchemaProperty.title?`.
  - srs-vscode: `src/cli/typeFields.ts:60-72`, `test/suite/typeFields.test.ts:15-34`.
  - muDemocracy.org: none.
- `FieldAssignment.display_label` already reflects any `FieldAssignmentOverride.display_label` by the time `effective_fields` returns. This plan does not change that.
