# Dogfooding the SRS CLI

This guide defines how we exercise the `srs` CLI against real SRS repositories — not to prove a command returns `ok: true`, but to prove the system **does the semantic thing it is meant to do**. SRS is a semantic record system: every command should advance a *meaningful intention*, and every scenario here is built around one.

It is the reference for **Stage 11 of `/ship`** (see `.claude/commands/ship.md`). Each time a feature adds or changes a CLI surface, the relevant scenario is run *and* this guide is updated so the scenarios keep pace with the spec and CLI.

## How to use this guide

1. Build the CLI from the merged state: `cargo build --bin srs`.
2. Find the scenario(s) that cover the surface you changed (use the [Coverage matrix](#coverage-matrix)).
3. Run the scenario's steps end-to-end against a real repository. Run the happy path **and** the named negative case.
4. Confirm each "Done when" signal. A scenario is not satisfied because commands exited `0` — exit code means *the command ran*, not *the data is correct*. Check `payload` and run `srs repo validate` (diagnostics live in the payload, not the exit code).
5. If your change touched a surface no scenario covers, **extend a scenario or add a new one** before you finish (see [Maintaining this guide](#maintaining-this-guide)).

Throughout, follow `srs/srs-usage.md`: CLI-first, discovery before writing, validate after every write batch. Never hand-edit JSON to make a scenario pass — if the CLI can't express a step, that gap *is the finding*.

## Principles these scenarios encode

- **Intention first.** A scenario names what a person is trying to accomplish, then shows the SRS capabilities that serve it. If a scenario reads like a command list, it has lost the point.
- **Semantic maturity is a ladder, not a gate.** Capture can start as free text (Tier 0) and grow into a typed, validated Record (Tier 2). The system must support every rung and the moves between them.
- **Records are the source of truth; renders are projections.** Document output is derived. Changing a record (or a relation) must change what renders.
- **Relations are claims, not ownership.** Asserting a relation never mutates lifecycle state on either endpoint.
- **Immutability by supersession.** A settled record is not silently overwritten; a successor is created and linked.

## Reference repositories

These existing repos anchor the scenarios — use them as the representative target when a scenario calls for pre-existing structure, or read them to see the shape a scenario builds toward.

| Repo | Path | What it demonstrates |
|---|---|---|
| Spec-as-repo | `../srs/srs` | The SRS spec authored as an SRS repository: sections/subsections, `precedes` ordering, document-view rendering. Always valid. |
| Gallery example | `../srs/docs/spec/examples/gallery-project`, `…/gallery-project-v2` | A general project repo (notes → typed records → records, relations, federation, containers) and the governance v2 layout (containers, document-views). |
| Governance profile | `../srs/docs/spec/profiles/governance-profile.md` | The semantic vocabulary for decisions, exercises, articles, roles, ratifications, and the deliberation protocols. |
| muDemocracy guide repo | `../../muDemocracy.org/muSrs` | Governance profile in live use: guide containers, decision/exercise records, document views. |

Paths are relative to `srs-rust/`. For a fresh throwaway repo use `srs repo create --repo /tmp/dogfood-<slug> --namespace com.example.dogfood`.

---

## Scenarios

Each scenario uses a fixed template so the set stays comparable and updatable:

- **Intention** — what the user wants, in their words.
- **Capabilities exercised** — the SRS concepts the scenario proves.
- **CLI surface** — the commands/flags/stdin shapes it drives (this is what the coverage matrix indexes).
- **Steps** — the happy path.
- **Negative case** — at least one wrong-input path that must produce a correct error envelope or diagnostic.
- **Done when** — the semantic signals that prove it worked.

### S1 — Capture before structure (the tier ladder)

**Intention.** *"I have a rough idea right now. I want to record it before I lose it, and turn it into something structured once I understand it better."*

**Capabilities exercised.** Tier 0 Note → Tier 2 Record promotion; `derived-from` linking the structured record back to its origin so the raw thinking is preserved, not discarded.

**CLI surface.** `note create`, `note get`, `note list`, `record create`, `relation create`, `repo validate`.

**Steps.**
1. Orient: `srs repo map --repo <repo> --pretty`.
2. Capture raw thinking as a Note (Tier 0): `srs note create` with a free-text `sections[]` body.
3. Later, create a Tier 2 Record that structures the same idea against a real type (see S2 if the type doesn't exist yet).
4. Assert `derived-from` from the Record to the Note so the lineage survives.
5. `srs repo validate --repo <repo> --pretty`.

**Negative case.** Create a record referencing a `typeId` that isn't in the package — confirm `ok: false` with a diagnostic, and that no ghost file is left in `instanceIndex`.

**Done when.** Both instances appear in `record list` / `note list`; a `derived-from` relation connects them in `relation list`; validate returns zero diagnostics. The Note is *not* deleted when the Record is created — promotion preserves the origin.

### S2 — Define a reusable shape (Fields + Type composition)

**Intention.** *"This kind of record keeps recurring. I want a named, versioned shape so every instance carries the same meaning."*

**Capabilities exercised.** Field as the atomic semantic unit with immutable semantics; Type as a composition of FieldAssignments; `displayLabel` is rendering-only and never changes meaning; `type schema` as the machine contract for a record's `fieldValues`; `record validate` as a no-write preflight that runs the same checks `record create`/`update` run before persist.

**CLI surface.** `field create`, `field list`, `field get`, `type create`, `type get`, `type schema`, `record validate`, `record create`, `repo validate`.

**Steps.**
1. Discover existing fields/types first — do not invent UUIDs: `srs field list`, `srs type list`.
2. Create any missing Fields (each self-contained: `namespace`, `name`, `version`, `valueType`, optional `aiGuidance`).
3. Compose a Type from those fields via FieldAssignments (`fieldId`, `order`, `required`, optional `displayLabel`).
4. Resolve the type's field IDs with `srs type get --id <typeId>` — `fieldId` is authoritative, never the filename or `name`.
5. **Preflight a record input without writing it:** pipe `{ "typeId", "typeVersion", "fieldValues" }` to `srs record validate`. A clean input returns `payload.ok: true`; a missing required field or an unknown/extra `fieldId` returns `ok: false` with the problem in `diagnostics`. Confirm `srs record list` count is unchanged — nothing was persisted. This is the editor-preflight primitive: validate the whole document, then write only if all sections pass.
6. Create the valid Record against the type (`record create`).
7. Emit the contract: `srs type schema <typeId>` and confirm it matches the fields.

**Negative case.** Send a `record validate` input that omits a **required** field, or carries a `fieldId` not assigned to the type — confirm each surfaces as a diagnostic with `ok: false`, and that `record list` count stays flat (no write). Confirm a `displayLabel` override does not change which field is resolved. *(Note: `validate` mirrors the write path exactly — it does **not** check enum `allowedValues` or `valueType` conformance, because the model's record validation does not validate those today; do not expect a value outside `select` options to be rejected here.)*

**Done when.** `type get` resolves every `fieldId` in the package; `record validate` passes a clean input and flags missing-required / unknown-field inputs as diagnostics **without persisting anything**; the valid record then creates clean; `type schema` reflects required/optional and value types correctly.

### S3 — Assert meaning between records (Relations)

**Intention.** *"These records are related: this one replaces that one; this one was derived from that one; this one depends on that one. I want those claims to be first-class and queryable."*

**Capabilities exercised.** Relations as first-class typed edges held outside the records; the canonical relation vocabulary (`contains`, `depends-on`, `supersedes`, `refines`, `derived-from`, `evidences`, `precedes`); the invariant that **asserting a relation does not change lifecycle state**; `record successor` as the supported supersession move.

**CLI surface.** `relation create`, `relation list`, `relation get`, `relation delete`, `record successor`.

**Steps.**
1. `srs relation list --repo <repo> --pretty` to see existing edges before adding.
2. Assert a point-to-point relation (`from`/`to`) such as `depends-on` between two records.
3. Create a supersession the supported way: `srs record successor --id <old>` (relation flag `supersedes` or `refines`), then confirm the new record and the `supersedes` edge both exist.
4. Confirm the old record's lifecycle state is unchanged by the relation itself.
5. Delete a relation and confirm it disappears from `relation list` without touching either endpoint.

**Negative case.** Create a relation whose `sourceInstanceId` or `targetInstanceId` is not in the `instanceIndex` — confirm it is rejected. Confirm a Container's `containerId` cannot be used as a relation endpoint.

**Done when.** Relations appear/disappear in `relation list`; `record successor` produces both a successor record and the supersession edge; neither endpoint's lifecycle state changed as a side effect of any relation operation.

### S4 — Deliberate, ratify, and supersede a decision (governance lifecycle)

**Intention.** *"Our group needs to decide something. I want to preserve the unresolved thinking, record the decision with its reasoning and alternatives, ratify it, and — when it later changes — replace it without erasing the original."*

This is the governance-profile workflow (`governance-profile.md` §6.3–6.4, §8.4) as used in `muSrs`.

**Capabilities exercised.** Governance `exercise` and `decision` types; lifecycle `draft → proposed → ratified → closed → superseded`; `derived-from` linking a Decision to the Exercise it came from; Containers as the durable home for decisions (the meeting is context, not owner); immutability after ratification enforced by creating a successor rather than editing in place; document-view rendering of a decision log.

**CLI surface.** `record create`, `record transition`, `record successor`, `relation create`, `container create`, `container members`, `document-view get`, `render document-view`.

**Steps.**
1. Create (or target) a governance Container that owns durable records: `srs container create`.
2. Capture the live thinking as an `exercise` Record (`thinking_reached`, `unresolved_questions`).
3. Start a `decision` Record in `draft`; fill deliberation fields as understanding advances (`decision_question`, `alternatives_considered`, `key_requirements`, `decision_statement`, `rationale`, `revisit_when`).
4. Link the Decision to the Exercise with `derived-from`.
5. Move the Decision through lifecycle: `record transition` `draft → proposed → ratified`, recording `ratification_note`.
6. Add the durable records to the Container's membership; confirm the (session-scoped) exercise is *not* owned by the meeting.
7. When the decision later changes, `record successor` it (`supersedes`) — do not edit the ratified record.
8. Render the decision log: `srs render document-view --view <decision-log-view>`.

**Negative case.** Attempt a lifecycle transition that the lifecycle definition does not allow (e.g. `draft → ratified` skipping `proposed`, if disallowed), or attempt to edit a `closed`/ratified record's semantic fields — confirm the operation is rejected or flagged.

**Done when.** The Decision visibly progresses through its states; `derived-from` ties it to the Exercise; the ratified record is superseded (not mutated) on change; the decision-log view renders the ratified decision with its reasoning. The Exercise remains part of the record after a Decision is derived from it.

### S5 — Assemble and render a document (records as source of truth)

**Intention.** *"I have a set of records that together form a document. I want an ordered, human-readable rendering — and I want the rendering to follow the records, not a hand-maintained copy."*

This is the spec-as-repo pattern (`../srs/srs`): sections are records, order is a relation, the markdown is a projection.

**Capabilities exercised.** Ordering relations (`precedes`, or `members[]` sequence relations like `section-sequence`); document views (`ext:views-l2`); `render` as a pure projection of records + relations; `tree` as the hierarchy view.

**CLI surface.** `document-view create`, `document-view list`, `document-view get`, `render document-view`, `relation create`, `tree`.

**Steps.**
1. Inspect the spec repo to see the target shape: `srs document-view list --repo ../srs/srs`, `srs render document-view --repo ../srs/srs --view <view>`.
2. In a working repo, define (or reuse) a document view that selects records by type and renders them.
3. Establish order with `precedes` (or a `members[]` sequence relation).
4. `srs render document-view --view <view>` and read the output.
5. Reorder the records (change the `precedes`/`members` relation) and re-render — confirm the output order changed.
6. `srs tree --repo <repo>` to see the derived hierarchy.

**Negative case.** Render a view that references a type with no instances, or a view ID that doesn't exist — confirm an empty-but-valid render or a correct error envelope (not a crash).

**Done when.** The render reflects record content and the ordering relation; **changing the relation changes the render** (proving the markdown is derived); `tree` shows the expected hierarchy.

### S6 — Govern the tag space and record states (vocabulary + lifecycle, RFC-006)

**Intention.** *"I want tags to mean something — a controlled vocabulary, not a free-for-all — and I want record state changes to follow a defined lifecycle."*

**Capabilities exercised.** Vocabulary `open` vs `closed` mode; Terms; the V10 promotion pre-flight (closing a vocabulary must not orphan in-use keys); lifecycle states and declared transitions; tagging records against a vocabulary.

**CLI surface.** `vocabulary create`, `vocabulary get`, `vocabulary list`, `vocabulary term-create`, `vocabulary derive-tag-set`, `vocabulary promote`, `term list`, `term get`, `lifecycle list`, `lifecycle get`, `record tag`, `record transition`.

**Steps.**
1. Discover what exists: `srs vocabulary list`, `srs lifecycle list`.
2. Create an `open` vocabulary; tag a record with an arbitrary key — confirm `open` accepts it.
3. Add Terms for the keys you intend to keep: `srs vocabulary term-create`.
4. **Preview the consequences of closing without writing anything:** `srs vocabulary derive-tag-set <vocab>` (positional id). Read `payload.entries` — each in-use tag key is classified `used-and-active`, `read-only-after-close`, or `will-be-invalid`. The `will-be-invalid` keys are exactly what `promote` will block on. This is the read-only V10 oracle: run it before promoting so there are no surprises.
5. Run promotion: `srs vocabulary promote <vocab>` (positional id). If an in-use key has no active term, confirm `ok: false` with `payload.unresolvableKeys` listing exactly the keys `derive-tag-set` flagged `will-be-invalid` (V10).
6. Add the missing term (or accept the consequence). Re-run `derive-tag-set` to confirm the key is now `used-and-active`, then promote successfully; confirm a now-`closed` vocabulary rejects an unknown key.
7. Inspect a lifecycle (`lifecycle get`) and drive a record through an allowed transition.

**Negative case.** (a) Promote with an unresolvable in-use key and confirm the structured block payload lists the same keys `derive-tag-set` classified `will-be-invalid`. (b) `derive-tag-set` on an unknown vocabulary id → `ok: false` with a diagnostic (no panic). (c) Attempt a `record transition` not present in the lifecycle's `transitions` and confirm rejection.

**Done when.** `open` accepts arbitrary keys; `closed` rejects unknown keys; **`derive-tag-set`'s `will-be-invalid` set equals `promote`'s `unresolvableKeys`** — the read-only pre-flight predicts the write outcome exactly; `promote` blocks with `unresolvableKeys` exactly when an in-use key lacks an active term (and succeeds within a grace `promotionWindow` if one is set); lifecycle transitions honour the declared state machine.

### S7 — Verify a document type is correctly composed (Blueprint schema)

**Intention.** *"I've declared a guide document type — a root record plus an ordered set of section types. Before building an editor or a render pipeline on top of it, I want to verify the composition is correct and machine-readable: all section types are reachable, each type's fields are discoverable, and composite groups (like data tables) surface with enough metadata for a generic authoring tool."*

**Capabilities exercised.** Blueprint as a composition validator; `blueprint schema` as the machine contract for a multi-record document; the field-group (`x-srs-composite-renderer`) hint for composite sections; how an authoring tool or agent discovers the correct form shape without type-specific code.

**CLI surface.** `blueprint list`, `blueprint get`, `blueprint validate`, `blueprint structure`, `blueprint schema`.

**Steps.**
1. Discover the repo's blueprints: `srs blueprint list --repo ../../muDemocracy.org/muSrs --pretty`. Identify the guide blueprint ID.
2. Inspect its declaration: `srs blueprint get --repo ../../muDemocracy.org/muSrs --blueprint 7bfa600b-f7b2-4a0e-82d4-34c02d9d6770 --pretty`. Note `rootTypes[]` and `structure[]`.
3. Validate the blueprint itself: `srs blueprint validate --blueprint 7bfa600b-f7b2-4a0e-82d4-34c02d9d6770 --repo ../../muDemocracy.org/muSrs --pretty`. Should return zero `payload.diagnostics`.
4. Project the schema: `srs blueprint schema 7bfa600b-f7b2-4a0e-82d4-34c02d9d6770 --repo ../../muDemocracy.org/muSrs --pretty`.
5. Confirm the schema shape:
   - `payload.schema.properties.root.$ref` resolves to the guide type definition in `definitions`.
   - `payload.schema.properties.contains.items.oneOf` lists exactly 4 `$ref` entries — one per section type declared in the blueprint.
   - Each `definitions[<section-type-id>]` has a `properties` map with `x-srs-field-id` and `x-srs-order` annotations on every flat field.
6. For the table section type (`d8d09d3b-8253-4d8d-b187-42f35c8446a7`), confirm its definition includes a `tables` array property carrying `x-srs-group-id`, `x-srs-repeatable: true`, and `x-srs-composite-renderer: "table"`, with sub-fields (`columns`, `rows`) inside `items.properties`. This proves a generic editor can discover the table widget from schema alone — no type-specific code needed.

**Negative case.** `srs blueprint schema <nonexistent-uuid> --repo ../../muDemocracy.org/muSrs --pretty` → `ok: false` with a diagnostic naming the unknown blueprint ID.

**Done when.** `payload.schema.properties.contains.items.oneOf` has exactly the section types declared in the blueprint; the table section type's definition includes the `x-srs-composite-renderer: "table"` group property; removing a type from the blueprint's `structure[]` and re-projecting drops it from `items.oneOf` — the schema is derived, not cached; `blueprint validate` shows zero diagnostics.

---

## Coverage matrix

Maps each CLI command group to the scenario(s) that exercise it. A command group with **no scenario** is a dogfooding gap — adding or changing such a surface in a PR means extending a scenario or adding one (see below).

| Command group | Exercised by |
|---|---|
| `repo` (map, validate, init) | S1–S6 (orientation + validation in every scenario) |
| `note` (create/get/list/update/delete) | S1 |
| `field` (create/list/get/update/delete) | S2 |
| `type` (create/get/list/schema/update/delete) | S2 |
| `record` (create/get/list/update/delete) | S1, S2, S4 |
| `record validate` (no-write preflight) | S2 |
| `record transition` | S4, S6 |
| `record successor` | S3, S4 |
| `record tag` | S6 |
| `relation` (create/list/get/delete) | S1, S3, S5 |
| `relation-type` | _gap — no scenario yet_ |
| `container` (create/members/roots/validate/…) | S4 |
| `document-view` (create/get/list/…) | S4, S5 |
| `render document-view` | S4, S5 |
| `view` (L1) | _gap — no scenario yet_ |
| `tree` | S5 |
| `vocabulary` (create/get/list/term-create/derive-tag-set/promote) | S6 |
| `term` (list/get) | S6 |
| `lifecycle` (list/get) | S4, S6 |
| `blueprint` (list/get/validate/structure/schema) | S7 |
| `protocol` | _gap — no scenario yet (governance protocols described in S4 prose)_ |
| `theme` | _gap — no scenario yet_ |
| `extension` | _gap — no scenario yet_ |
| `migrate` | _gap — no scenario yet_ |
| `tag` (definition) | _gap — being deprecated; see open issues_ |
| `package` | _covered implicitly by field/type creation in S2_ |

Gaps are intentional and visible: they are the backlog of surfaces that need a meaningful scenario. Do not delete a gap row — fill it when a feature gives the surface a real workflow to demonstrate.

## Maintaining this guide

`/ship` Stage 11 keeps this guide current. When a PR adds or changes a CLI command, flag, stdin shape, or observable behaviour:

1. **If an existing scenario already covers that surface**, run it against the change and, if the change alters the workflow, update the scenario's steps / done-when so they reflect reality.
2. **If the surface is a `gap` row (or entirely new)**, decide whether it belongs in an existing scenario (extend it) or needs a new one. A new scenario must lead with a *meaningful intention* — if you can't state the intention, the feature may not yet be ready to dogfood, and that itself is worth noting on the issue.
3. **Update the coverage matrix** in the same PR so it never drifts from the scenarios.
4. Keep scenarios runnable: every command block must work against a real repo. A scenario step that no longer runs is a regression in this guide.

Scenarios should stay few and meaningful. Prefer deepening an existing scenario over proliferating shallow ones.
