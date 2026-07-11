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
| Gallery example | `../srs/docs/spec/examples/gallery-project-v2` | The LiMoMa governance repo: notes → typed records → records, relations, containers, document-views, and a shared `Lifecycle` bound via `Type.lifecycleRef` (records carry `lifecycleState`). |
| Governance profile | `../srs/docs/spec/profiles/governance-profile.md` | The semantic vocabulary for decisions, exercises, articles, roles, ratifications, and the deliberation protocols. |
| muDemocracy guide repo | `../../muDemocracy.org/muSrs` | Governance profile in live use: guide containers, decision/exercise records, document views. |
| RFC-008 container-subset fixture | `crates/srs-cli/tests/fixtures/rfc008-container-subset` | A heterogeneous container (two `section.text` + two `section.table` records in a `table-1 → text-1 → table-2 → text-2` precedes chain) with three document views demonstrating `typeFilter`, `typeDispatch`, and cross-type precedes ordering. Anchors S11. |
| Blueprint Brief fixture | `crates/srs-cli/tests/fixtures/blueprint-brief` | Self-contained blueprint + protocol with three stages: clean (s1), valid typeId (s2), ghost typeId (s3). Anchors the S7 negative case for `contributesTo.typeId` resolution in `blueprint brief`. |

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

**Capabilities exercised.** Tier 0 Note → Tier 2 Record promotion; `derived-from` linking the structured record back to its origin so the raw thinking is preserved, not discarded; RFC-020 schema-driven `displayLabel` resolution via `Type.identityFieldId` (#376).

**CLI surface.** `note create`, `note get`, `note list`, `field create`, `type create` (`identityFieldId`), `record create`, `record list` (per-record `displayLabel`), `tree`, `find`, `repo navigation`, `relation create`, `repo validate`.

**Steps.**
1. Orient: `srs repo map --repo <repo> --pretty`.
2. Capture raw thinking as a Note (Tier 0): `srs note create` with a free-text `sections[]` body.
3. Later, create a Tier 2 Record that structures the same idea against a real type (see S2 if the type doesn't exist yet).
4. Assert `derived-from` from the Record to the Note so the lineage survives.
5. List the records: `srs record list --repo <repo> --pretty`. Each item is `{ instanceId, displayLabel, record }` — confirm the new record's `displayLabel` is the core-resolved human label, **not** a raw UUID. Priority (RFC-020, #376): the record's Type's effective `identityFieldId` (own or inherited via `ext:type-inheritance`) when set on a field with a non-empty value; else a field named `title` → `heading` → `name` → `label`; else the type name. Cross-check against `srs tree --repo <repo>`, `srs find --repo <repo>`, and `srs repo navigation --repo <repo>`: the record's `displayLabel` must equal its label on every surface (one core resolution, many clients — none re-derives a title).
6. `srs repo validate --repo <repo> --pretty`.
7. **identityFieldId end-to-end (#376):** create a Field named something *other than* `title`/`heading`/`name`/`label` (e.g. `summary` — use a name the heuristic fallback does not cover), a Type whose `fields[]` includes it and whose `identityFieldId` names that field's UUID (`type create` accepts `identityFieldId` directly in the stdin JSON — no bespoke CLI work needed, the generic Type read/write path already carries it), and a record with a value on that field. Confirm `record list`'s `displayLabel` is the field's *value*, not the type name — even though no `title`/`heading`/`name`/`label` field exists on the type at all.

**Negative case.** Create a record referencing a `typeId` that isn't in the package — confirm `ok: false` with a diagnostic, and that no ghost file is left in `instanceIndex`. **Label fallback:** a record of a type with no `identityFieldId` and no `title`/`heading`/`name`/`label` field still lists with a non-empty `displayLabel` equal to its `typeName` (the resolver falls back through every tier, never returns an empty label or a bare UUID). **Dangling `identityFieldId` (RFC-020 Rule [N+33], #376):** author a Type whose `identityFieldId` does not resolve to any `fieldId` in its effective field set (e.g. a random UUID) — confirm `srs repo validate` returns `ok: false` with a diagnostic containing `"RFC-020 (Rule [N+33])"` and naming both the type and the dangling field id. Records of *other*, valid types keep resolving correctly — one Type's broken `identityFieldId` does not suppress diagnostics or labels for the rest of the repository. **Manifest schema validation:** tamper `manifest.json` to remove a required field (e.g. `title`) and re-run `srs repo validate` — confirm the output is `ok: false` with a diagnostic containing `"manifest.json" … "title" is a required property`. Similarly, adding an undeclared field (e.g. `"name": "foo"`) produces `ok: false` with "Additional properties are not allowed". A freshly created repo with an untampered manifest returns `ok: true` with no manifest diagnostics. **RFC-013 I-79:** removing `manifest.container` entirely produces `ok: false` with *two* diagnostics — a schema-level one (`"container" is a required property`) and an invariant-level one (`RFC-013 I-79: manifest.container is absent`). A freshly created repo always has `manifest.container` set and validates with 0 RFC-013 errors.

**Done when.** Both instances appear in `record list` / `note list`; a `derived-from` relation connects them in `relation list`; validate returns zero diagnostics. The Note is *not* deleted when the Record is created — promotion preserves the origin. Every `record list` item carries a `displayLabel` equal to that record's label on every other label-producing surface (`tree`, `find`, `repo navigation`, `container resolve-view`): identity-field-bearing records show that field's value even with no `title`/`heading`/`name`/`label` field present; title-bearing records (no `identityFieldId`) show their title; heading-bearing records (no `identityFieldId`, no `title`) show their heading; field-less types fall back to `typeName`. Manifest tampering cases produce `ok: false` with a diagnostic naming `manifest.json` and the violated rule. A dangling `identityFieldId` produces an `ok: false` Rule [N+33] diagnostic naming the offending type, without blocking validation of other types.

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
   - Flat fields carry `x-srs-field-id` and a 1-based `x-srs-order` reflecting their merged position.
   - If the type declares `fieldGroups`, each group appears as an array property with `x-srs-group-id`, `x-srs-repeatable`, and an `x-srs-order` drawn from the **same** positional sequence as the flat fields — not from the raw `group.order` integer. No two entries (fields or groups) share the same `x-srs-order` value. This is the invariant fixed in #148.
   - A field created with both `description` and `instructions` carries `x-srs-description` and `x-srs-instructions` in its schema property; a field created without `instructions` omits `x-srs-instructions` entirely (not `null`) — confirmed live end-to-end (`field create` → `type create` → `type schema`, each a fresh process against the on-disk repo) in #415.

**Negative case.** Send a `record validate` input that omits a **required** field *and* carries a `fieldId` not assigned to the type — confirm **both** problems come back in `diagnostics` from the single call (`validate` reports every violation at once, not just the first), with `ok: false` and `record list` count flat (no write). Confirm a `displayLabel` override does not change which field is resolved. *(Note: `validate` mirrors the write path exactly — it does **not** check enum `allowedValues` or `valueType` conformance, because the model's record validation does not validate those today; do not expect a value outside `select` options to be rejected here.)*

Also confirm `srs type schema <nonexistent-uuid>` → `ok: false` with a diagnostic naming the unknown type.

**Done when.** `type get` resolves every `fieldId` in the package; `record validate` passes a clean input and, for an input with multiple problems, returns **all** of them as diagnostics in one pass **without persisting anything**; the valid record then creates clean; `type schema` reflects required/optional and value types correctly; for types with `fieldGroups`, every `x-srs-order` value in the schema is unique across both fields and groups — no positional collisions.

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

**Done when.** The Decision visibly progresses through its states; `derived-from` ties it to the Exercise; the ratified record is superseded (not mutated) on change; the decision-log view renders the ratified decision with its reasoning. The Exercise remains part of the record after a Decision is derived from it. When transitioning to a final state (`closed` or `superseded`), `payload.warnings` contains a `LIFECYCLE_FINAL_STATE` entry and envelope `diagnostics` is absent — the warning is informational, not an error.

### S5 — Assemble and render a document (records as source of truth)

**Intention.** *"I have a set of records that together form a document. I want an ordered, human-readable rendering — and I want the rendering to follow the records, not a hand-maintained copy."*

This is the spec-as-repo pattern (`../srs/srs`): sections are records, order is a relation, the markdown is a projection.

**Capabilities exercised.** Ordering relations (`precedes`, or `members[]` sequence relations like `section-sequence`); document views (`ext:views-l2`); the RFC-009 typed anchor (`DocumentView.rootTypeRefs` — version-exact `ExactTypeRef`, the validated successor to the free-string `containerType` join); `render` as a pure projection of records + relations; `tree` as the hierarchy view; **RFC-020 Rule [N+37]** identity-field fallback for per-record headings (#453).

**CLI surface.** `document-view create`, `document-view list` (incl. `--root-type <typeId>`), `document-view get`, `render document-view`, `relation create`, `tree`.

**Steps.**
1. Inspect the spec repo to see the target shape: `srs document-view list --repo ../srs/srs`, `srs render document-view --repo ../srs/srs --view <view>`.
2. In a working repo, define (or reuse) a document view that selects records by type and renders them. Anchor it to its root type with `rootTypeRefs: [{ "typeId": <type-uuid>, "typeVersion": <n> }]` (keep `containerType` as a human-readable hint if you like).
3. Establish order with `precedes` (or a `members[]` sequence relation).
4. `srs render document-view --view <view>` and read the output.
5. Reorder the records (change the `precedes`/`members` relation) and re-render — confirm the output order changed.
6. `srs tree --repo <repo>` to see the derived hierarchy.
7. Find views by anchor: `srs document-view list --root-type <type-uuid>` returns only views whose `rootTypeRefs` include that Type id; each summary carries `rootTypeRefs`.
8. **RFC-020 Rule [N+37] — identity field fallback heading (#453):** use a Type whose `identityFieldId` is set to a field named something *other than* `title`/`name`/`label` (e.g. `headline`). Create a `document-view` section with **no `titleFieldId`** and render it — confirm each record produces an `### <headline-value>` H3 heading drawn from the identity field. Confirm the identity field *still appears in the body* (structured mode is NOT activated by the fallback — the field is rendered twice, once as heading and once in the field table). Then create a second view with **`titleFieldId` explicitly set** to the same field and render it — confirm the heading is still present but the field is now *absent from the body* (structured mode is active when `titleFieldId` is set, skipping the title field from the body). This proves `titleFieldId` takes precedence over `identityFieldId` and that the two paths have different body-skip behaviour.

**Negative case.** Render a view that references a type with no instances, or a view ID that doesn't exist — confirm an empty-but-valid render or a correct error envelope (not a crash). `srs document-view list --root-type <unknown-uuid>` returns an empty list with `ok: true` (not an error). RFC-009 validation diagnostics are **advisory `Warning`s** that never change `errors`/exit code: declare a `rootTypeRefs` entry that does not resolve to a package Type and confirm `repo validate` emits **I-63** (the entry is ignored for matching); give a rooted Container a `containerType` that differs from its root Record's resolved Type `name` and confirm **I-64** (the hint is stale; the container stays valid). **Rule [N+37] negative:** render a section with no `titleFieldId` whose Type has no `identityFieldId` — confirm no `### ` heading is emitted (the fallback chain terminates without a heading, which is the correct baseline behaviour).

**Done when.** The render reflects record content and the ordering relation; **changing the relation changes the render** (proving the markdown is derived); `tree` shows the expected hierarchy; `document-view list --root-type` narrows to the anchored views, and a stale `containerType` surfaces as an advisory I-64 warning with `repo validate` still reporting `0 errors`. **Rule [N+37]:** a section with no `titleFieldId` and a Type with `identityFieldId` emits an `### <identity-value>` heading per record, with the identity field also present in the body (structured mode off); a section with `titleFieldId` set emits the heading and skips that field from the body (structured mode on); a section with no `titleFieldId` and no `identityFieldId` emits no per-record heading.

### S6 — Govern the tag space and record states (vocabulary + lifecycle, RFC-006)

**Intention.** *"I want tags to mean something — a controlled vocabulary, not a free-for-all — and I want record state changes to follow a defined lifecycle."*

**Capabilities exercised.** Vocabulary `open` vs `closed` mode; Terms; the V10 promotion pre-flight (closing a vocabulary must not orphan in-use keys); lifecycle states and declared transitions (both inline `lifecycle` and referenceable `lifecycleRef` forms); tagging records against a vocabulary; `$schema` editor hints are silently absorbed by Lifecycle and Vocabulary loaders (#117).

**CLI surface.** `vocabulary create`, `vocabulary get`, `vocabulary list`, `vocabulary term-create`, `vocabulary derive-tag-set`, `vocabulary promote`, `term list`, `term get`, `lifecycle list`, `lifecycle get`, `record tag`, `record transition`.

**Steps.**
1. Discover what exists: `srs vocabulary list`, `srs lifecycle list`.
2. Create an `open` vocabulary; tag a record with an arbitrary key — confirm `open` accepts it.
3. Add Terms for the keys you intend to keep: `srs vocabulary term-create`.
4. **Preview the consequences of closing without writing anything:** `srs vocabulary derive-tag-set <vocab>` (positional id). Read `payload.entries` — each in-use tag key is classified `used-and-active`, `read-only-after-close`, or `will-be-invalid`. The `will-be-invalid` keys are exactly what `promote` will block on. This is the read-only V10 oracle: run it before promoting so there are no surprises.
5. Run promotion: `srs vocabulary promote <vocab>` (positional id). If an in-use key has no active term, confirm `ok: false` with `payload.unresolvableKeys` listing exactly the keys `derive-tag-set` flagged `will-be-invalid` (V10).
6. Add the missing term (or accept the consequence). Re-run `derive-tag-set` to confirm the key is now `used-and-active`, then promote successfully; confirm a now-`closed` vocabulary rejects an unknown key.
7. Inspect a lifecycle (`lifecycle get`) and drive a record through an allowed transition.
   - If the type uses an inline `lifecycle`, the steps above work as described.
   - To exercise the referenceable form: use the gallery-project-v2 or any repo with a standalone `Lifecycle` referenced via `lifecycleRef`. Confirm `record create` sets `lifecycleState` to the lifecycle's `initialState` (e.g. `"draft"`). Then pipe `{"byTransition": "<name>"}` or `{"to": "<state>"}` to `record transition` and confirm the state advances. (This path was broken before #114 — records were created without an initial state and transitions were rejected.)
   - When advancing to a final state (`isFinal: true`), confirm `payload.warnings` contains a `LIFECYCLE_FINAL_STATE` message and that envelope `diagnostics` is absent (the warning is non-fatal and lives in the typed payload per ADR-011, not the error envelope).
8. **`$schema` loader tolerance (#117):** If your editor adds a top-level `"$schema"` key to lifecycle or vocabulary JSON files (the standard JSON Schema association hint), confirm `lifecycle list` and `vocabulary list` still succeed. Before #117, the Lifecycle loader rejected `$schema` with "unknown field". Note: adding `$schema` via CLI is not yet supported — you will encounter this in practice when an editor or schema-aware tool writes the file. The gap (no `lifecycle create` CLI command) is tracked in issue #116.

**Negative case.** (a) Promote with an unresolvable in-use key and confirm the structured block payload lists the same keys `derive-tag-set` classified `will-be-invalid`. (b) `derive-tag-set` on an unknown vocabulary id → `ok: false` with a diagnostic (no panic). (c) Attempt a `record transition` not present in the lifecycle's `transitions` and confirm rejection — this applies to both inline and `lifecycleRef`-bound Types. (d) Confirm `lifecycle list` succeeds even when a lifecycle file carries a `"$schema"` key (the old rejection error no longer occurs). (e) **`repo validate` lifecycle invariants (#239):** Write a type file directly into the package with an invalid inline lifecycle (e.g. no `isInitial: true` state, or `initialState` pointing to the wrong state key), then run `srs repo validate --repo <repo> --pretty` — confirm a `"V9"` error appears in `payload.diagnostics`. Write a type with a `lifecycleRef` UUID that does not match any installed lifecycle, run `repo validate` — confirm a `"V8"` error naming the dangling UUID appears. Note: both V7 (mutual exclusion) and V9 (structural) violations can only be introduced via direct file editing because the CLI enforces these rules at write time — `repo validate` is the net that catches repos created or modified by other tools.

**Done when.** `open` accepts arbitrary keys; `closed` rejects unknown keys; **`derive-tag-set`'s `will-be-invalid` set equals `promote`'s `unresolvableKeys`** — the read-only pre-flight predicts the write outcome exactly; `promote` blocks with `unresolvableKeys` exactly when an in-use key lacks an active term (and succeeds within a grace `promotionWindow` if one is set); lifecycle transitions honour the declared state machine for both inline and `lifecycleRef`-bound Types; lifecycle and vocabulary files with a `$schema` key load without error; `repo validate` catches V8 (dangling lifecycleRef UUID) with `ok: false` and a top-level `diagnostics` string naming the missing UUID, and V9 (invalid inline lifecycle) with the structural error in the same envelope format — a valid inline lifecycle returns `ok: true` with no errors.

### S7 — Verify a document type is correctly composed (Blueprint schema + brief)

**Intention.** *"I've declared a guide document type — a root record plus an ordered set of section types. Before building an editor, an extraction pipeline, or an AI prompt on top of it, I want to verify the composition is correct and machine-readable: all section types are reachable, each type's fields are discoverable, and composite groups (like data tables) surface with enough metadata for a generic authoring tool. I also want the layered AI guidance context — field semantics, extraction hints, and any targeting protocol — composed into a single brief I can hand directly to an agent."*

**Capabilities exercised.** Blueprint as a composition validator; `blueprint schema` as the machine contract for a multi-record document; the field-group (`x-srs-composite-renderer`) hint for composite sections; `blueprint brief` as the layered guidance context for AI extraction pipelines (blueprint `aiGuidance`, each root type's `aiGuidance` + fields in `order`, structure RelationSpecs, and any targeting Protocol); non-fatal diagnostics when a root type is unresolvable, no protocol is found, or a protocol stage `contributesTo` references a `fieldId` or `typeId` that doesn't exist in the package.

**CLI surface.** `blueprint list`, `blueprint get`, `blueprint validate`, `blueprint structure`, `blueprint schema`, `blueprint brief`.

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
7. Compose the AI guidance brief:
   ```
   srs blueprint brief 7bfa600b-f7b2-4a0e-82d4-34c02d9d6770 \
     --repo ../../muDemocracy.org/muSrs --pretty
   ```
   Confirm:
   - `ok: true`, `payload.diagnostics` is empty.
   - `payload.types` contains the root type (`com.mudemocracy/guide`) with its fields listed in `order`.
   - Every field has a `fieldId`, `name`, `valueType`, `required` flag, and an `aiGuidance` object (or `null` if none declared).
   - `payload.structure` lists the 4 `contains` RelationSpecs with `cardinality` and `required` (all `false` / `0..*` for the guide blueprint).
   - `payload.protocol` is `null` (the guide blueprint has no targeting extraction protocol yet).
   - `payload.rendered` is a non-empty markdown string beginning with `# Blueprint:` that an agent can consume directly.

**Negative case.**
- `srs blueprint schema <nonexistent-uuid> --repo ../../muDemocracy.org/muSrs --pretty` → `ok: false` with a diagnostic naming the unknown blueprint ID.
- `srs blueprint brief 00000000-0000-0000-0000-000000000000 --repo ../../muDemocracy.org/muSrs --pretty` → `ok: false`, `diagnostics[0]` names the unknown blueprint ID; no crash or empty envelope.
- Protocol stage with a bad `contributesTo.fieldId`: `blueprint brief` returns `ok: true`; `payload.diagnostics` contains `"contributesTo field <id> not found in package"`; the stage is still present in `payload.protocol.stages` with both valid and invalid field refs intact. Confirms non-fatal: a typo in one field ref does not suppress the rest of the brief.
- Protocol stage with a bad `contributesTo.typeId`: `blueprint brief` returns `ok: true`; `payload.diagnostics` contains `"contributesTo type <id> not found in package"`; the stage is still present. Mirrors the fieldId case — a ghost typeId is non-fatal. Run against the self-contained fixture to verify:
  ```
  srs repo validate --repo crates/srs-cli/tests/fixtures/blueprint-brief --pretty
  # → ok: true, summary.errors: 0

  srs blueprint brief 00000000-0000-4000-8000-000000004621 \
    --repo crates/srs-cli/tests/fixtures/blueprint-brief --pretty
  # → ok: true
  # payload.diagnostics: ["contributesTo type 00000000-0000-0000-0000-000000000000 not found in package"]
  # payload.protocol.stages has 3 entries (s1 Gather, s2 Extract, s3 Classify all present)
  # payload.protocol.stages[2].name == "Classify" — ghost-typeId stage is present and named correctly
  # payload.diagnostics total count == 1 (stage s2's valid typeId produces no diagnostic)
  ```

**Done when.** `payload.schema.properties.contains.items.oneOf` has exactly the section types declared in the blueprint; the table section type's definition includes the `x-srs-composite-renderer: "table"` group property; removing a type from the blueprint's `structure[]` and re-projecting drops it from `items.oneOf` — the schema is derived, not cached; `blueprint validate` shows zero diagnostics. `blueprint brief` returns a non-empty `rendered` string and structured `types[]` with field-level `aiGuidance`; missing-blueprint input yields a correct `ok: false` envelope. A protocol stage whose `contributesTo` carries an unresolvable `fieldId` or `typeId` yields `ok: true` with a diagnostic — the rest of the brief is unaffected. For the typeId case, the fixture command above is the verification path: `repo validate` → 0 errors; `blueprint brief` → `ok: true`, exactly 1 diagnostic for the ghost typeId, and all 3 stages present in `payload.protocol.stages`. **Blueprint schema validation (RFC-020, #355):** adding an undeclared field to a blueprint file (e.g. `"unknownBadField": "x"`) and running `srs repo validate` produces `ok: false` with a top-level `diagnostics` string containing `"Additional properties are not allowed"` and the blueprint schema URL `https://srs.semanticops.com/schema/2.0/blueprint.json`. The fixture repo itself (`crates/srs-cli/tests/fixtures/blueprint-brief`) has no extra fields and must continue to report `ok: true` with 0 blueprint JSON Schema errors.

### S8 — Render a document view in multiple formats with per-format themes

**Intention.** *"My document view renders cleanly in my editor's markdown preview, but I also need it to render as valid HTML for a web preview — without maintaining two separate document views or changing how I call the render command."*

This is the muSrs guide pattern: `guide-body-view` has a default `themeRef` targeting `markdown` and a `themeVariant` named `html` targeting `html`. The render command auto-selects the correct theme based on `--view-format`, with no caller change required.

**Capabilities exercised.** `theme list` to discover available themes and their format targets; `theme get` to inspect element templates; `themeVariants[]` on a document view as named format alternates; format-driven auto-selection in `resolve_active_theme` (`[T-2]` diagnostic when no theme targets the requested format; `[T-3]` when multiple variants match).

**CLI surface.** `theme list`, `theme get`, `document-view get`, `render document-view --view-format`.

**Steps.**
1. Discover available themes: `srs theme list --repo ../../muDemocracy.org/muSrs --pretty`. Note `guide-prose` (targets `markdown`) and `guide-prose-html` (targets `html`).
2. Inspect the HTML theme's templates: `srs theme get <guide-prose-html-id> --repo ../../muDemocracy.org/muSrs --pretty`. Confirm `fieldRow`, `groupFieldRowTemplates` for `item-term` and `item-body`.
3. Inspect the document view: `srs document-view get --view 2aba4d85-317b-44e1-a600-d38a743b4cb4 --repo ../../muDemocracy.org/muSrs --pretty`. Confirm `themeRef` → `guide-prose` and `themeVariants[0]` → `guide-prose-html` with `name: "html"`.
4. Render as HTML:
   ```
   srs render document-view --repo ../../muDemocracy.org/muSrs \
     --view 2aba4d85-317b-44e1-a600-d38a743b4cb4 \
     --container 1c843817-c0f9-4ba6-b65f-c6d23af161a7 \
     --view-format html --pretty
   ```
   Confirm `payload.diagnostics` has no `[T-2]`, output contains `<p>` tags, no `field-label` spans.
5. Render as markdown (same command without `--view-format`, or `--view-format markdown`). Confirm `**` bold markers present, no `[T-2]` or `[T-3]` diagnostics.
6. Confirm both renders produce non-empty `payload.rendered` with clean prose — no raw field-name labels (`item-term`, `item-body`).

**Negative case.** Render with `--view-format text` (no theme targets `text`) — confirm `payload.diagnostics` contains a `[T-2]` entry naming the view and theme IDs, and `payload.rendered` is non-empty (render proceeds without theme).

**Done when.** HTML render has no `[T-2]`, uses `<p>` and `<strong>` tags, no field-label spans; markdown render is unchanged; `text` format triggers `[T-2]` cleanly; both renders reflect actual record content (prose, not plumbing labels). The two format renders differ only in markup — not in what records or sections they include.

---

### S9 — Migrate a working repository to a new location (repo copy)

**Intention.** *"I want to move my notes repository from one place to another — maybe from a local path to a shared drive, or from a `.srsj` bundle back to a file store — and I want to see my familiar filenames, not raw UUIDs, when I open the target directory."*

**Capabilities exercised.** `srs repo copy` (file → file and `.srsj` bundle → file); the `{slug}-{id8}.json` filename convention; copy rejection on a non-empty target; `repo validate` confirming structural integrity after copy.

**CLI surface.** `repo create`, `note create`, `repo copy`, `repo validate`.

**Steps.**

1. Create a fresh source repo:
   ```
   srs repo create --repo /tmp/s9-src --namespace com.example.s9
   ```
2. Add a titled note and an untitled note:
   ```
   echo '{"title":"Deployment Checklist","sections":[{"name":"body","content":"Steps to verify before any release."}]}' \
     | srs note create --repo /tmp/s9-src
   echo '{"sections":[{"name":"body","content":"Quick scratch thought."}]}' \
     | srs note create --repo /tmp/s9-src
   ```
3. Confirm source filenames follow the convention:
   ```
   ls /tmp/s9-src/records/notes/
   # deployment-checklist-<id8>.json
   # <id8>.json  (untitled falls back to id-only)
   ```
4. Copy to a new file store:
   ```
   srs repo copy --from /tmp/s9-src --to /tmp/s9-dst
   ```
5. Confirm destination filenames match source exactly:
   ```
   ls /tmp/s9-dst/records/notes/
   # same two files as step 3
   ```
6. Validate the destination:
   ```
   srs repo validate --repo /tmp/s9-dst
   ```

**Negative case.** Run `srs repo copy` a second time targeting the same non-empty `/tmp/s9-dst` — confirm `ok: false` and a diagnostic naming "target is not empty".

**Done when.**
- The titled note file in the destination is named `deployment-checklist-<id8>.json` (slug from title, 8-char UUID prefix) — not a bare UUID.
- The untitled note file is named `<id8>.json` (id-only fallback).
- Filenames in source and destination are identical.
- `srs repo validate` on the destination returns `ok: true` with 0 errors and `summary.checked` equal to the instance count.
- The non-empty-target copy returns `ok: false` with a clear diagnostic.

---

### S9b — Normalise instance file paths in-place (repo upgrade)

**Intention.** *"My repository was created before the `{slug}-{id8}.json` convention was enforced. I want to normalise all file paths in-place without creating a full copy, so that the filenames are human-readable."*

**Capabilities exercised.** `srs repo upgrade` (in-place path normalisation); idempotency guarantee; FileStore-only restriction; `repo validate` confirming structural integrity after upgrade.

**CLI surface.** `repo create`, `repo upgrade`, `repo validate`.

1. Create a fresh repo: `srs repo create --repo /tmp/s9b --namespace com.example.s9b --package-name primary --package-version 1.0.0 --srs-version "2.0-draft"`.
2. Create a note: `srs note create --repo /tmp/s9b` (stdin: `{"title": "Deployment Checklist"}`).
3. Run upgrade on a repo that already has canonical paths:
   ```bash
   srs repo upgrade --repo /tmp/s9b --pretty
   ```
4. Run upgrade a second time to verify idempotency:
   ```bash
   srs repo upgrade --repo /tmp/s9b --pretty
   ```
5. Inject a non-canonical file manually to simulate a pre-ADR-008 repo (edit `manifest.json` to add an entry at an arbitrary path; write the file). Run upgrade:
   ```bash
   srs repo upgrade --repo /tmp/s9b --pretty
   ```
6. Validate the repo after upgrade:
   ```bash
   srs repo validate --repo /tmp/s9b --pretty
   ```
7. **Negative case.** Run upgrade with `--store json` — confirm `ok: false` with a clear message that upgrade requires a file-backed repository.

**Done when:**
- Step 3 returns `renames: []` and `alreadyCanonicalCount` equals the number of instances.
- Step 4 (idempotency) also returns `renames: []` — the second run is a no-op.
- Step 5 returns a rename entry with `fromPath` being the injected path and `toPath` being the canonical `{slug}-{id8}.json` form; the old file is absent on disk and the new file is present.
- Step 6 returns `ok: true` with 0 errors.
- Step 7 returns `ok: false` with `"repo upgrade only supports file-backed repositories"`.

---

### S10 — Edit a `.srsj` bundle and get a reviewable diff

**Intention.** *"I keep my repository as a single `.srsj` bundle in git. When I change one record through the CLI, I want the commit to show just that change — so I can review it and trust it — not a whole-file reshuffle."*

**Capabilities exercised.** Deterministic `.srsj` serialisation (entries written in sorted key order); idempotent writes (a no-op write reproduces the file byte-for-byte); minimal-diff single-record edits; in-place CLI mutation of a `.srsj` via `--repo <bundle>.srsj`. This is the behaviour ADR-017 guarantees.

**CLI surface.** `repo copy` (file → `.srsj`), `note create`/`note get`/`note update` operating on a `.srsj` repo, `repo validate`.

**Steps.**

1. Create a source file repo and add several notes (so the bundle holds multiple `data` entries):
   ```
   srs repo create --repo /tmp/s10-src --namespace com.example.s10
   for n in alpha bravo charlie delta echo; do
     echo "{\"title\":\"Note $n\",\"sections\":[{\"name\":\"body\",\"content\":\"content for $n\"}]}" \
       | srs note create --repo /tmp/s10-src
   done
   ```
2. Bundle it and confirm the `data` keys are in sorted order:
   ```
   srs repo copy --from /tmp/s10-src --to /tmp/s10.srsj
   jq -r '.data | keys[]' /tmp/s10.srsj   # package/package.json first, then notes A→Z
   ```
3. Snapshot for diffing: `cp /tmp/s10.srsj /tmp/s10-before.srsj`.
4. **Idempotent no-op:** round-trip one note through the CLI with no semantic change (the full payload, including `instanceId`, must be passed back):
   ```
   ALPHA=$(jq -r '.data["records/notes/note-alpha-"*".json"].instanceId' /tmp/s10.srsj)  # or read it from `note list`
   srs note get $ALPHA --repo /tmp/s10.srsj | jq -c '.payload.note' \
     | srs note update $ALPHA --repo /tmp/s10.srsj            # ok: true — a real write
   diff /tmp/s10-before.srsj /tmp/s10.srsj                    # ZERO lines
   ```
5. **Single edit:** change one note's title and confirm the diff is confined to that record:
   ```
   cp /tmp/s10.srsj /tmp/s10-before2.srsj
   echo "{\"instanceId\":\"$CHARLIE\",\"title\":\"Note charlie EDITED\",\"sections\":[{\"name\":\"body\",\"content\":\"content for charlie\"}]}" \
     | srs note update $CHARLIE --repo /tmp/s10.srsj
   diff -U1 /tmp/s10-before2.srsj /tmp/s10.srsj               # only charlie's title (+ its manifest index hint)
   ```
6. Validate the mutated bundle: `srs repo validate --repo /tmp/s10.srsj`.

**Negative case.** `echo '{"instanceId":"00000000-0000-0000-0000-000000000000","title":"ghost","sections":[]}' | srs note update 00000000-0000-0000-0000-000000000000 --repo /tmp/s10.srsj` — confirm `ok: false` with a "note not found" diagnostic, and that `/tmp/s10.srsj` is byte-for-byte unchanged afterwards.

**Done when.**
- `jq -r '.data | keys[]'` lists the bundle's entries in sorted order.
- The no-op write returns `ok: true` yet `diff` reports **zero** changed lines — a real serialisation that reproduces the file byte-for-byte. Repeating it stays stable.
- The single-title edit produces a diff limited to that one record's entry (plus its denormalised `instanceIndex` title hint in the manifest) — no other entry moves or reorders.
- `repo validate` on the mutated bundle returns `ok: true` with 0 errors.
- The unknown-id update returns `ok: false` and leaves the bundle unchanged.

---

### S11 — Render a heterogeneous container in authored order (RFC-008 typeFilter + typeDispatch)

**Intention.** *"My container holds mixed record types — prose sections and data tables — that together form one ordered document. I want to render them in a single section in their authored (precedes) order, choose a different layout per type, and sometimes show only one kind — without splitting them into separate type-grouped sections that lose the interleaved order."*

This is the RFC-008 capability: a `container-subset` document-view section that (a) restricts to chosen types via `typeFilter` and (b) routes each type to its own L1 view via `typeDispatch`, while preserving the container's full `precedes` order. The anchor repo is the `rfc008-container-subset` fixture — two `section.text` and two `section.table` records in a `table-1 → text-1 → table-2 → text-2` precedes chain, with views `type-filter-view` (`…3507`), `type-dispatch-view` (`…3508`), and `cross-type-order-view` (`…3509`).

**Capabilities exercised.** `container-subset` section source; `typeFilter` (version-independent `namespace/name` keys) applied as a **filter-then-project step *after* the precedes sort**, so cross-type edges still order the survivors; `typeDispatch` selecting a per-type L1 view (consulted before `renderViewId`, falling back to `renderViewId` then the record's own type); records-as-source-of-truth (changing the `precedes` relation changes the render); both fields use the package-resolved type identity, never the record's denormalised `typeNamespace`/`typeName` hints.

**CLI surface.** `document-view create`, `document-view get`, `render document-view`, `relation create` / `relation list`, `repo validate`.

**Steps.**
1. Orient on the anchor repo and confirm it is valid: `srs repo validate --repo crates/srs-cli/tests/fixtures/rfc008-container-subset --pretty` → `ok: true`, `summary.checked: 4`, 0 errors.
2. **typeFilter** — render `…3507`: `srs render document-view --repo <fixture> --view 00000000-0000-4000-8000-000000003507`. Confirm only `Text-One` and `Text-Two` appear (both `Table-*` records dropped) **and** the two survivors keep their relative order — `Text-One` before `Text-Two`. That ordering only holds because the filter runs *after* the chain sort over the full container; filtering first would strip the `table-*` links and collapse to `createdAt` order.
3. **typeDispatch** — render `…3508`: confirm all four records appear in full precedes order (`Table-One → Text-One → Table-Two → Text-Two`) and each carries its per-type marker (`TABLE-VIEW:` / `TEXT-VIEW:` preamble), proving each type resolved to its own L1 view.
4. **Cross-type order** — render `…3509` (no filter, no dispatch): all four in the same precedes order, each rendered by its own type.
5. **Prove records are the source of truth:** copy the fixture to a scratch dir, then reorder the chain — e.g. reverse the head edge so `text-1 → table-1` (delete `table-1 → text-1`, add `text-1 → table-1`) makes `Text-One` the new head — and re-render `…3509`. Confirm the rendered order changes to match the new relation (keep the edits a valid DAG; a cycle just falls back to `createdAt`).
6. **Authoring round-trip:** re-create a `typeFilter`/`typeDispatch` view from scratch via `document-view create` (stdin must include `createdAt`); read the persisted file under `package/document-views/` and confirm `source.typeFilter` and `section.typeDispatch` survived — these fields are CLI-authorable, not fixture-only.

**Negative case.** Author a `container-subset` section whose `typeFilter` matches **no** container member (e.g. `["fixture.rfc008/section.nonexistent"]`) and render it — confirm the section is **empty-but-valid**: `ok: true`, `payload.rendered` present, no record titles, and an empty `payload.diagnostics` (not an error or crash). Separately, confirm that records with **no** `typeDispatch` entry and **no** `renderViewId` emit **no** `[view-dispatch]` diagnostic — an absent dispatch is a silent fall-through to the record's own type, not a warning.

**Done when.** The `typeFilter` render contains exactly the in-filter types and preserves their cross-type precedes order; the `typeDispatch` render shows every record under its per-type view marker in full chain order; changing the `precedes` relation changes the rendered order (the markdown is derived, not stored); a no-match `typeFilter` yields an empty-but-valid section; `typeFilter`/`typeDispatch` survive a `document-view create` round-trip to disk.

### S12 — Filter a type-query section by lifecycle state (RFC-011 lifecycleStates + excludeLifecycleStates + containerScope)

**Intention.** *"My document view should only show active decisions — not drafts, not superseded ones — and it should pull from the whole repository, not just one container I have to name upfront."*

This is the RFC-011 capability: `type-query` SectionSource extended with `lifecycleStates` (inclusive OR filter), `excludeLifecycleStates` (exclusion after inclusion), and `containerScope` (`"repository"` / `"explicit"` / `"subtree"`).

**Capabilities exercised.** `type-query` section source with `excludeLifecycleStates`; `lifecycleStates` inclusive filter; `containerScope: "repository"` ignoring container membership; `emptyBehavior: "hide"` for sections with no surviving records; no regression in existing `container-subset` or `fixed-instances` sections.

**CLI surface.** `document-view create`, `render document-view`, `repo validate`.

**Anchor repo.** `srs/docs/spec/examples/gallery-project-v2` — 7 `governance/decision` records (all `ratified`), 1 `governance/decision_log` record (`draft`).

**Steps.**
1. Validate the anchor repo: `srs repo validate --repo srs/docs/spec/examples/gallery-project-v2` → `ok: true`, `summary.errors: 0`.
2. **Exclude filter** — create a DocumentView with:
   ```json
   {
     "source": {
       "type": "type-query",
       "semanticObjectType": "governance/decision",
       "containerScope": "repository",
       "excludeLifecycleStates": ["draft"]
     }
   }
   ```
   Place in `package/document-views/`, add to `package.json "documentViews"`, then render. Confirm all 7 ratified decisions appear, diagnostics is `[]`.
3. **Inclusive filter** — create a second view with `"lifecycleStates": ["draft"]`. Render — confirm 0 decisions appear and the section is hidden (`emptyBehavior: "hide"` default).
4. **Exclusion of all** — create a view with `"excludeLifecycleStates": ["ratified"]`. Render — confirm 0 decisions appear (all excluded).
5. `srs repo validate --repo <copied-repo>` after adding views — must still report `ok: true`, `summary.errors: 0`.

**Negative case.** A `type-query` with `lifecycleStates: ["active"]` applied to the gallery (no decisions have state `active`) returns an empty section with `ok: true`, no error. A record without `lifecycleState` is **not** excluded by `excludeLifecycleStates` but **is** excluded when `lifecycleStates` is non-empty.

**Done when.** The exclude-filter view renders exactly the non-excluded records; the include-filter view renders only records matching the listed states; a non-matching inclusive filter yields an empty-but-valid render; `repo validate` still reports 0 errors after adding RFC-011 views; `diagnostics` is empty for `containerScope: "repository"` (no noise).

---

### S13 — Exercise protocol read-side after create: list, get, stages

**Intention.** *"I've declared an extraction protocol that tells AI agents how to pull structured decisions from governance discussions — stage by stage, field by field. Before I wire it to a blueprint brief, I want to confirm the protocol is machine-readable: the stage list comes back in the right order, the full protocol definition is retrievable by ID, and missing IDs return a clean error envelope."*

**Capabilities exercised.** `protocol create` (write path), `protocol list` (compiled-model read), `protocol get` (compiled-model read by ID), `protocol stages` (stage projection from compiled model), `protocol find-by-target-type` (lookup by target typeId), `blueprint brief` (verifies `BriefStageResult.output_type` is typed as `TypeRef`). This scenario specifically verifies that the refactored read-side service functions source data from the compiled `Package.protocols` (populated at load time) rather than re-reading package files on every call, and that `outputType` serializes as a `TypeRef` object when set and is absent (not `null`) when unset.

**CLI surface.** `protocol create`, `protocol list`, `protocol get`, `protocol stages`, `protocol find-by-target-type`, `blueprint brief`, `repo validate`.

**Anchor repo.** None — build from scratch with `srs repo create`.

**Steps.**
1. `srs repo create --repo /tmp/dogfood-protocols --namespace com.example.dogfood` → `ok: true`.
2. Create the target type:
   ```json
   {"id":"com.example.dogfood/decision","namespace":"com.example.dogfood","name":"Decision","version":1,"description":"A governance decision record","createdAt":"2026-06-26T00:00:00Z","fields":[],"allowedRelationTypes":[]}
   ```
   piped to `srs type create --repo /tmp/dogfood-protocols` → `ok: true`.
3. Create the protocol:
   ```json
   {
     "protocolId": "com.example.dogfood/extraction-protocol",
     "protocolNamespace": "com.example.dogfood",
     "protocolName": "Decision Extraction Protocol",
     "protocolVersion": 1,
     "protocolTargetType": "com.example.dogfood/decision",
     "protocolDescription": "A protocol for extracting structured decisions from governance discussions",
     "protocolCreatedAt": "2026-06-26T00:00:00Z",
     "protocolStages": [
       {"stageId": "com.example.dogfood/extraction-protocol/identify", "name": "Identify", "description": "Identify the decision being made", "order": 1, "dependsOn": []}
     ]
   }
   ```
   piped to `srs protocol create --repo /tmp/dogfood-protocols` → `ok: true`, `payload.protocol.protocolId` = `"com.example.dogfood/extraction-protocol"`.
4. `srs protocol list --repo /tmp/dogfood-protocols --pretty` → `payload.protocols` has 1 entry with `protocolId`, `name`, `namespace`, `version`, `stageCount: 1`.
5. `srs protocol get --repo /tmp/dogfood-protocols com.example.dogfood/extraction-protocol --pretty` → `ok: true`, `payload.protocol.protocolStages` has the `identify` stage with `order: 1`.
6. `srs protocol stages --repo /tmp/dogfood-protocols com.example.dogfood/extraction-protocol --pretty` → `payload.stages` has 1 entry with `stageId` and `name`.
6a. **`outputType` typed check (srs-rust#204):** add a second stage with `"outputType": {"typeId": "<type-id>", "typeVersion": 1}` and re-run `protocol stages`. Confirm the stage entry's `outputType` is a `TypeRef` object `{"typeId": ..., "typeVersion": ...}`, not a raw JSON string. Also confirm the stage without `outputType` does **not** have an `"outputType": null` key — the key must be absent entirely.
6b. **Optional-field absent check (srs-rust#352):** run `blueprint brief <id> --repo /tmp/dogfood-352`. Confirm the JSON output contains no `null` values and that optional keys (`aiGuidance`, `protocol`, `purpose`, `question`, `completionCriteria`, `contributesTo`) are absent rather than `"key": null` when the blueprint/stage/type has no values set for them.
7. `srs repo validate --repo /tmp/dogfood-protocols --pretty` → `ok: true`, `summary.errors: 0`.
8. `srs protocol find-by-target-type --type-id "com.example.dogfood/decision" --repo /tmp/dogfood-protocols --pretty` → `ok: true`, `payload.protocolId` = `"com.example.dogfood/extraction-protocol"`, `payload.stages` has 1 entry.

**Negative case.** `srs protocol get --repo /tmp/dogfood-protocols com.example.dogfood/nonexistent --pretty` → `ok: false`, `diagnostics[0]` contains `"not found"`. `srs protocol list` on a freshly-created repo (no protocols declared) → `ok: true`, `payload.protocols: []`. `srs protocol find-by-target-type --type-id "type-no-match" --repo /tmp/dogfood-protocols` → `ok: false`, `diagnostics[0]` contains `"No protocol found with target type"`.

**Done when.** `protocol list` returns the created protocol; `protocol get` returns the full definition including all stages; `protocol stages` returns the stage list; `protocol find-by-target-type` returns `{ protocolId, protocolName, stages, diagnostics }` for a known typeId and a clean `ok: false` envelope for an unknown typeId; a missing-ID get returns `ok: false` with a diagnostic naming the missing ID; `repo validate` shows 0 errors; `protocol list` on an empty repo returns an empty array without error. **TypeRef check (srs-rust#204):** a stage with `outputType` set serializes it as `{"typeId": ..., "typeVersion": ...}` (not raw JSON); a stage without `outputType` has no `outputType` key in the payload (not `null`). **Optional-field absent check (srs-rust#352):** `blueprint brief` output contains no `null` values; optional keys (`aiGuidance`, `protocol`, `purpose`, `question`, `completionCriteria`, `contributesTo`) are absent rather than `"key": null` when unset.

---

### S14 — Drive an editor member list from a DocumentView's field selection (`container resolve-view`)

**Intention.** *"I'm building an interactive, selectable list of a container's members in the editor. In one call I need the columns to show — driven by the DocumentView's field selection, not by my client knowing the types — plus each member's display label and full record, and the container's root for the header. I should never compute 'what columns' or 'what label' in the client."*

This is the issue-#254 capability: a single `resolve_container_view` projection (service → CLI payload → WASM binding, per `docs/architecture/capability-layering.md`) returning the container root record, the ordered Tier-2 member records (full `Record` + core-resolved `displayLabel` + `tier`), and the **column/field spec** resolved from a DocumentView section's `renderViewId → View.field_views`. Column-source precedence is [ADR-018](adr/018-container-view-column-source-precedence.md): the section targeting this container wins, else the first section by `order` with a `renderViewId`, else empty columns.

**Capabilities exercised.** Container membership (roots-first, deduped); DocumentView → View `field_views` column projection (visible-false exclusion, `order` sort, `displayLabel` override → field `name` fallback); core `record_display_label` reuse for member/root labels; Tier-gating (non-Tier-2 members skipped with a diagnostic); `--view-id` override vs. root-type matching; non-fatal diagnostics vs. hard errors; `ColumnSpec.isIdentityColumn` (RFC-020, ADR-023, #376) — the resolved column matching the single-Type-anchored DocumentView's effective `identityFieldId`. The anchor repo is the `rfc008-container-subset` fixture (heterogeneous container `…3500`, text-view `…3504`).

**CLI surface.** `container resolve-view` (`--view-id` flag), `document-view create`, `type create` (`identityFieldId`), `repo validate`.

**Steps.**
1. Orient and validate: copy the fixture to a scratch dir, then `srs repo validate --repo /tmp/dogfood-resolve-container-view` → `ok: true`, `summary.checked: 4`, 0 errors.
2. **Default (root-type matched) view** — `srs container resolve-view 00000000-0000-4000-8000-000000003500 --repo <repo>`. The fixture's container has no root binding and its views declare no `rootTypeRefs`, so no DocumentView matches: confirm `payload.containerView.documentViewId` is absent, `columns` is empty, **and** all four members still come back with core-resolved labels (`Text-One`, `Text-Two`, `Table-One`, `Table-Two`) — the member list never depends on a view resolving.
3. **Author a member-list view** — `document-view create` (stdin) a DocumentView whose `container-subset` section targets `…3500` and carries `renderViewId: …3504` (the text-view). `createdAt` is required in stdin. Capture the new `payload.documentView.id`.
4. **Column projection** — `srs container resolve-view 00000000-0000-4000-8000-000000003500 --view-id <new-dv-id> --repo <repo>`. Confirm `columns` is exactly one entry resolved from the text-view: `fieldName: "title"`, `displayLabel: "Text Title"` (the `FieldView.displayLabel` override), `order: 0`; `documentViewId` is the authored view; all four members carry a `displayLabel` and a full `record` object.
5. `srs repo validate --repo <repo>` → still `ok: true`, 0 errors (authoring the view did not corrupt the repo).
6. **`isIdentityColumn` — single-Type case (#376, ADR-023)** — in a fresh scratch repo (`identityFieldId` is not present in the `rfc008-container-subset` fixture's types), create a Field, a Type whose `identityFieldId` names that field, a Container, a matching L1 `View` exposing the field as a column, and a `DocumentView` whose `rootTypeRefs` has exactly that one `{ typeId, typeVersion }` entry with a `container-subset` section targeting the container. Confirm `container resolve-view`'s resolved column for that field carries `"isIdentityColumn": true`, and any other column on the same view carries `"isIdentityColumn": false`. Column **order** is unaffected — `isIdentityColumn` is a pure marker (ADR-023).
7. **`isIdentityColumn` — common-identity multi-Type case (#454, ADR-027)** — using the same scratch repo, add a second Type B with the same `identityFieldId` as Type A. Update (or create) a `DocumentView` whose `rootTypeRefs` lists both `{ typeId: A, typeVersion: 1 }` and `{ typeId: B, typeVersion: 1 }`. Run `container resolve-view`. Confirm the column for the shared identity field still carries `"isIdentityColumn": true` — even though two Types are listed. This is the common-identity case: all Types agree, so the column-level signal is still unambiguous. Then author a third Type C with a *different* `identityFieldId` and add it to `rootTypeRefs`; confirm every column reverts to `"isIdentityColumn": false` (types disagree → no signal).

**Negative case.** Two paths: (a) a nonexistent container — `srs container resolve-view 00000000-0000-0000-0000-deadbeef0000 --repo <repo>` → `ok: false`, top-level `diagnostics[0]` contains `"container not found"`, and `payload` is null (no partial/ghost result). (b) an unknown `--view-id` — `srs container resolve-view …3500 --view-id <missing> --repo <repo>` → `ok: true`, `documentViewId` absent, `columns` empty, `payload.containerView.diagnostics` contains `"documentView <missing> not found"`, and the four members are still returned (an unresolved view is a diagnostic, not a failure). (c) **disagreeing `isIdentityColumn`:** a `DocumentView.rootTypeRefs` with two or more entries where the Types have *different* `identityFieldId` values (or any entry lacks one) yields `isIdentityColumn: false` on every column, with **no diagnostic** (a disagreeing/absent identity signal is a normal outcome per ADR-023 and ADR-027, not an error). The `rfc008-container-subset` fixture's `cross-type-order-view` (`…3509`) has no `rootTypeRefs` at all and serves as an example of this silent fallback.

**Done when.** One call returns root + ordered members + per-member label + DocumentView-driven column spec; columns honour visibility, order, and the displayLabel override and resolve field names from the package; each member carries `isVisibleByDefault: bool` (see ADR-020 and S15 for how the governing section's `excludeLifecycleStates` drives this field); a non-Tier-2 member would be skipped with a diagnostic (not crash the call); an unknown `--view-id` degrades to empty columns + diagnostic while still returning members; a missing container is a clean `ok: false` with no payload; `repo validate` stays at 0 errors. The client computes no semantics — columns, labels, per-member visibility, and identity-column marking all come entirely from the payload. Exactly one column is `isIdentityColumn: true` when the DocumentView's `rootTypeRefs` all resolve to the same `identityFieldId` (whether one entry per ADR-023, or multiple entries all agreeing per ADR-027); every column is `false`, silently, when any entry is absent from the index, disagrees, or `rootTypeRefs` is absent/empty.

**Authored list defaults (ADR-020).** `payload.containerView.excludeLifecycleStates` carries the authored default-hidden lifecycle states, read from the same governing section that drives `columns`: `[]` when that section is a `container-subset` (the `…3500` fixture above), or the declared set when it is a `type-query` (see S15). Each member also carries `isVisibleByDefault: false` when its `lifecycleState` is in that exclusion list, and `true` otherwise (including when `lifecycleState` is absent). A web client can implement a "show all" toggle by reading `member.isVisibleByDefault` directly — it never re-derives the governing `DocumentSection` or re-queries the exclusion list.

---

### S15 — Interactive governance list: default-hidden states + show-all + search/tag (`srs-gov list`)

**Intention.** *"I'm running a governance decision log. By default the list should hide decisions that are `superseded` or `closed` — but that 'what's hidden' rule is **authored in the view**, not coded in my client — with a one-flag show-all toggle, plus search and tag narrowing. My client should only compose two services, never re-express the filter."*

This is issue #298 (parent plan §4): `srs-gov list` composes `container resolve-view` (authored columns + ordered members + authored `excludeLifecycleStates`, ADR-020) with `srs find` (the runtime discovery query, ADR-019). The default-hidden states come from the package's `type-query` DocumentView; `srs-gov` forwards them to `find` and intersects the hit set with the resolved members. No lifecycle/filter semantics live in the client.

**Capabilities exercised.** `srs-gov repo-create` (stamps the regenerated `type-query` governance seed; internally delegates to `srs-repository::governance_scaffold_service` and `srsj_migration_service` — #327); `container resolve-view` `excludeLifecycleStates` surface; `srs find` `--exclude-lifecycle-state` / `--text` / `--tag` / `--container`; the resolve-view ∩ find intersection; the `--all` show-all toggle; `--explain` printing both composed commands; `srs-gov tui --smoke` first-frame rendering; `record transition` (drive lifecycle states) and `record tag add`.

**CLI surface.** `srs-gov repo-create`, `srs repo navigation`, `srs-gov list` (`--all`, `--search`, `--tag`, `--explain`, `--json`), `srs-gov tui --smoke`, `srs record create/transition/tag add`, `srs container resolve-view`, `srs find`, `repo validate`.

**Steps.**
1. `srs-gov repo-create --output /tmp/dogfood-srs-gov-list.srsj --title "Acme Co-op"` → a fresh governance `.srsj`. Confirm the stamped seed's decision-log DocumentView is a `type-query` (regenerated asset): `srs container resolve-view <decisionLogId> --repo <repo>` → `payload.containerView.excludeLifecycleStates: ["superseded","closed"]`.
2. **Navigation** — `srs repo navigation --repo <repo>` → `payload.navigation.identity.instanceId` is non-empty (a `governance/article` record carrying the title); `payload.navigation.sections` has exactly 1 entry (the decision-log root container); `payload.navigation.diagnostics` is empty. This confirms the RFC-013 root container is correctly scaffolded with `memberInstanceIds` in the store.
3. Add four decisions in the decision-log container via `srs record create --type governance/decision --container <decisionLogId>` and drive their states with `srs record transition` (`{"to":"proposed"}` → `{"to":"ratified"}` → `{"to":"superseded"|"closed"}`): one left `draft`, one `ratified` (tag it `tooling`, statement contains a unique word like `budget` only in a non-title field), one `superseded`, one `closed`.
4. **Default** — `srs-gov list decision_log --repo <repo>` shows only the `draft` and `ratified` decisions; the `superseded` and `closed` ones are hidden.
5. **Show-all** — `srs-gov list decision_log --all` shows all four.
6. **Search** — `srs-gov list decision_log --all --search budget` returns only the decision whose `decision_statement` (a non-title field) contains `budget` — content recall, not a title match.
7. **Tag** — `srs-gov list decision_log --tag tooling` returns only the tagged decision.
8. **Explain** — `srs-gov --repo <repo> --explain list decision_log --search budget` prints three underlying commands: `repo navigation` (container resolution via RFC-009 type chain), `container resolve-view <id>` (authored columns + excludeLifecycleStates), and `--container <id> find --exclude-lifecycle-state superseded --exclude-lifecycle-state closed --text budget` (runtime query).
9. **TUI smoke** — `srs-gov --repo <repo> tui --smoke` exits `0` and reports a nonblank first frame with a nonzero record count (e.g. `1 sections, 8 records`). Run against the gallery repo and the fresh repo.
10. `srs repo validate --repo <repo>` → `ok: true`, 0 errors.

**Negative case.** `srs-gov list bogus_key` → a clean `error: unknown key 'bogus_key'. Known: decision_log` (non-zero exit), with no partial output. `--explain` placed *after* the subcommand (`list decision_log --explain`) is rejected by clap (it is a top-level flag) — the correct form is `srs-gov --explain list …`.

**Done when.** Navigation on a freshly created governance repo returns `ok: true` with a non-empty identity instanceId, exactly 1 section (decision-log), and no diagnostics. The default list hides exactly the authored states and `--all` reveals them; `--search` narrows by content over a non-title field and `--tag` by facet; `--explain` shows three commands: `repo navigation` (RFC-009 container resolution), `resolve-view`, and `find` carrying the authored excludes; `tui --smoke` renders a nonblank first frame with a nonzero record count for both existing and freshly scaffolded governance repos (confirming `sectionContainerId` is resolved correctly); `repo validate` stays at 0 errors. Crucially, the hidden-state set lives in the package `type-query` view (and is surfaced by `resolve-view`), never hardcoded in `srs-gov` — confirm with `rg "superseded|closed" crates/srs-gov/src` returning only `#[cfg(test)]` fixtures (and help text), never production filter logic.

### S16 — Initialise a new organisation repository from a governance seed

**Intention.** *"I've downloaded the governance seed package for my organisation. Before I start adding records I want to stamp it with our identity — our namespace, a title, and today's install date — so every future export carries the right provenance."*

**Capabilities exercised.** `repo init-new` re-stamps an `.srsj` seed's identity while preserving upstream package provenance; `meta.upstreamPackage.installedAt` is updated on stamp; `repositoryId` is auto-generated when omitted; `repo validate` confirms the store is structurally sound after stamping; `repo map` confirms the new identity is visible.

**CLI surface.** `repo init-new`, `repo validate`, `repo map`.

**Steps.**

1. Obtain a seed `.srsj` with `meta.upstreamPackage` provenance. The quickest source is the governance seed embedded in `crates/srs-repository/src/repository_lifecycle.rs` tests (the `seed_srsj()` helper). For a real test, use any `.srsj` produced by a governance-package install step that writes `meta.upstreamPackage`. Write it to `/tmp/dogfood-s16.srsj`:
   ```bash
   # Minimal seed in one step (requires jq):
   echo '{"srsj":"1","manifest":{"repositoryId":"seed-repo-id","srsVersion":"2.0-draft","namespace":"com.mudemocracy.governance","instanceIndex":[],"packageRef":{"mode":"local","path":"package"},"meta":{"upstreamPackage":{"packageId":"pkg-001","namespace":"com.mudemocracy.governance","name":"Governance","version":"1.0.0","installedAt":""}}},"data":{"package/package.json":{"id":"pkg-001","namespace":"com.mudemocracy.governance","name":"Governance","version":"1.0.0","fields":[],"types":[],"relationTypes":[],"views":[],"documentViews":[]}}}' > /tmp/dogfood-s16.srsj
   ```

2. Orient before stamping:
   ```bash
   srs repo map --repo /tmp/dogfood-s16.srsj --pretty
   ```
   Confirm `repositoryId: "seed-repo-id"` and `namespace: "com.mudemocracy.governance"` — this is the seed identity, not the organisation's.

3. Re-stamp with the organisation's identity:
   ```bash
   srs repo init-new --repo /tmp/dogfood-s16.srsj \
     --namespace com.example.myorg \
     --title "Example Org Governance" \
     --description "Governance repository for Example Org"
   ```
   Confirm the payload carries:
   - `repositoryId` — a new UUID (not `"seed-repo-id"`)
   - `namespace: "com.example.myorg"`
   - `packageId: "pkg-001"` and `packageVersion: "1.0.0"` (upstream provenance preserved)

4. Confirm the identity is visible in the store:
   ```bash
   srs repo map --repo /tmp/dogfood-s16.srsj --pretty
   ```
   The map's `namespace` field must be `com.example.myorg`.

5. Validate the stamped repository:
   ```bash
   srs repo validate --repo /tmp/dogfood-s16.srsj --pretty
   ```

**Negative case.** Run `repo init-new` against an `.srsj` that has no `meta.upstreamPackage` key (e.g. a repo created with `repo create`, which does not write upstream provenance):
```bash
srs repo create --repo /tmp/dogfood-s16-plain.srsj --namespace com.example --package-name test --package-version 1.0.0 --srs-version 2.0-draft
srs repo init-new --repo /tmp/dogfood-s16-plain.srsj --namespace com.example.new --title "New"
```
Must return `ok: false` with a message referencing the absent `meta` or `upstreamPackage`.

**Done when.**
- `payload.repositoryId` is a UUID that differs from `"seed-repo-id"`.
- `payload.namespace` is `"com.example.myorg"`.
- `payload.packageId` and `payload.packageVersion` match the upstream provenance from the seed.
- `srs repo map` shows the new namespace, title, and description.
- `srs repo validate` on a real governance seed returns `ok: true` with 0 errors. (The minimal one-liner fixture above will fail package schema validation because it omits required `$schema`/`title`/`status`/`createdAt` fields from `package.json` — this is a fixture limitation, not a stamping bug. A real governance seed ships with a complete package manifest.)
- The negative case returns `ok: false` with a message containing "manifest meta object is absent".

---

### S17 — Pin the repository's navigation root (`repo set-root-container`)

**Intention.** *"I've created the governance container that serves as my organisation's navigation entry point. I want to record that pointer in the manifest — so any client running `repo navigation` always knows where to start, without me having to tell it every time."*

**Capabilities exercised.** `set_manifest_root_container` service writing `manifest.container`; the RFC-013 contract that a valid repository always has `manifest.container` set; `repo navigation` reading back the pointer; `repo validate` asserting the invariant.

**CLI surface.** `repo create`, `container create`, `repo set-root-container`, `repo navigation`, `repo validate`.

**Steps.**

1. Create a throwaway repository:
   ```bash
   srs repo create --repo /tmp/dogfood-s17 --namespace com.example.s17 \
     --package-name s17-pkg --package-version 1.0.0 --srs-version 2.0-draft
   ```
2. Create the navigation root container (the "home" for records):
   ```bash
   echo '{"title":"Root","namespace":"com.example.s17"}' \
     | srs container create --repo /tmp/dogfood-s17
   ```
   Capture `payload.container.containerId` as `$ROOT_ID`.
3. Retrieve the identity instance (`repo create` always scaffolds a `com.semanticops.core/purpose` record and sets `manifest.container.identityInstanceId` to it since #424):
   ```bash
   srs repo navigation --repo /tmp/dogfood-s17 --pretty
   ```
   Capture `payload.navigation.identity.instanceId` as `$IDENTITY_ID`.
4. Pin the root container in the manifest:
   ```bash
   srs repo set-root-container --repo /tmp/dogfood-s17 \
     --container-id "$ROOT_ID" \
     --identity-instance-id "$IDENTITY_ID"
   ```
   Confirm the response: `ok: true`, `payload.containerId == $ROOT_ID`, `payload.identityInstanceId == $IDENTITY_ID`.
5. Inspect the manifest to confirm the pointer was written:
   ```bash
   cat /tmp/dogfood-s17/manifest.json | python3 -m json.tool
   ```
   Confirm `.container.containerId == $ROOT_ID` and `.container.identityInstanceId == $IDENTITY_ID`.
6. Validate:
   ```bash
   srs repo validate --repo /tmp/dogfood-s17 --pretty
   ```
   The repo must pass RFC-013 I-79 with 0 errors.

**Negative case.** Pass `--container-id ""` (empty string) — confirm `ok: false` with a message containing `"container_id must not be empty"` and that `manifest.container` is unchanged. Similarly, pass `--identity-instance-id ""` — same pattern.

**Done when.** `set-root-container` returns the two IDs in its payload; `manifest.json` on disk carries `.container.containerId` and `.container.identityInstanceId` matching what was passed; `repo validate` reports 0 errors; empty-flag inputs return `ok: false` with a clear diagnostic without corrupting the manifest.

---

### S18 — Graduate a Note to a typed Record (`note graduate`)

**Intention.** *"I captured a rough idea as a Note. Now I've thought it through and want to promote it into a proper decision record — without losing the original thinking, and in one step rather than three."*

**Capabilities exercised.** `graduate_note` service: atomic Record creation + `graduatedAt` stamp; re-graduation guard; Note identity preserved after promotion; `repo validate` confirms both entities are well-formed.

**CLI surface.** `note graduate`, `note get`, `record get`, `repo validate`.

**Steps.**

1. Create a throwaway repo with a field and a type:
   ```bash
   REPO=$(mktemp -d /tmp/dogfood-s18-XXXX)
   srs repo create --repo "$REPO" --namespace com.example.s18
   srs field create --repo "$REPO" <<'EOF'
   {"id":"","namespace":"com.example.s18","name":"summary","version":1,"valueType":"string","description":"Summary","aiGuidance":null,"createdAt":""}
   EOF
   FIELD_ID=$(srs field list --repo "$REPO" | jq -r '.payload.fields[0].id')
   srs type create --repo "$REPO" <<EOF
   {"id":"","namespace":"com.example.s18","name":"decision","version":1,"description":"A decision","fields":[{"fieldId":"$FIELD_ID","order":0,"required":false,"repeatable":false}],"createdAt":""}
   EOF
   ```
2. Capture a rough idea as a Note:
   ```bash
   NOTE_OUT=$(srs note create --repo "$REPO" <<'EOF'
   {"instanceId":"","sections":[{"name":"body","content":"We should standardise our deployment process."}]}
   EOF
   )
   NOTE_ID=$(echo "$NOTE_OUT" | jq -r '.payload.note.instanceId')
   ```
3. Graduate the Note to a decision Record:
   ```bash
   GRAD=$(srs note graduate --repo "$REPO" "$NOTE_ID" --type com.example.s18/decision <<EOF
   {"fieldValues":[{"fieldId":"$FIELD_ID","value":"Standardise deployments via CI/CD pipeline"}]}
   EOF
   )
   echo "$GRAD" | jq '{ok, graduatedAt: .payload.note.graduatedAt, recordId: .payload.record.instanceId}'
   RECORD_ID=$(echo "$GRAD" | jq -r '.payload.record.instanceId')
   ```
4. Confirm the note carries `graduatedAt` and the record exists:
   ```bash
   srs note get --repo "$REPO" "$NOTE_ID" | jq '.payload.note.graduatedAt'
   srs record get --repo "$REPO" "$RECORD_ID" | jq '{ok, instanceId: .payload.instanceId, displayLabel: .payload.displayLabel}'
   ```
5. Validate the repository:
   ```bash
   srs repo validate --repo "$REPO" --pretty
   ```

**Negative cases.**
- **Note not found:** `echo '{"fieldValues":[]}' | srs note graduate --repo "$REPO" "00000000-0000-0000-0000-000000000000" --type com.example.s18/decision` — expect `ok: false` with "note not found".
- **Unknown type:** `echo '{"fieldValues":[]}' | srs note graduate --repo "$REPO" "$NOTE_ID" --type com.example.s18/nonexistent` — expect `ok: false` with "type not found" (and note is still not graduated, because the error fires before the write).
- **Re-graduation:** after step 3, graduate the same note again — expect `ok: false` with "already graduated".

**Done when.** `ok: true`; `payload.note.graduatedAt` is a non-null ISO-8601 string; `payload.record.instanceId` is a non-empty UUID; the original Note is still present in `note list`; `repo validate` returns 0 diagnostics. All three negative cases return `ok: false` with clear diagnostics; no partial writes occur on error.

---

### S19 — Query allowed lifecycle transitions before advancing a record (ext:lifecycle)

**Intention.** *"Before I advance a decision's status I want to know what moves are allowed from where it is now — without guessing at the lifecycle definition or accidentally trying an invalid transition."*

**Repo.** `srs/docs/spec/examples/gallery-project-v2` (read-only; has `governance/decision` records with an inline lifecycle).

**CLI surface.** `record allowed-transitions`, `record list`.

**Steps.**

1. Pick an active (non-final) decision:
   ```bash
   REPO=<path-to-gallery-project-v2>
   RECORD_ID=$(srs record list --repo "$REPO" | \
     python3 -c "import json,sys; d=json.load(sys.stdin); print(next(r['record']['instanceId'] for r in d['payload']['records'] if r.get('record',{}).get('lifecycleState') == 'ratified'))")
   ```
2. Query allowed transitions:
   ```bash
   srs record allowed-transitions --repo "$REPO" --id "$RECORD_ID" --pretty
   ```
   Confirm `ok: true`, `payload.currentState == "ratified"`, `payload.transitions` is non-empty (names: `supersede`, `close`), `payload.isImmutable == false`.

3. Query a final-state record (e.g. one with `lifecycleState == "superseded"`):
   ```bash
   FINAL_ID=$(srs record list --repo "$REPO" | \
     python3 -c "import json,sys; d=json.load(sys.stdin); print(next(r['record']['instanceId'] for r in d['payload']['records'] if r.get('record',{}).get('lifecycleState') == 'superseded'))")
   srs record allowed-transitions --repo "$REPO" --id "$FINAL_ID" --pretty
   ```
   Confirm `payload.isImmutable == true` and `payload.transitions == []`.

**Negative case.** Non-existent ID:
```bash
srs record allowed-transitions --repo "$REPO" --id 00000000-0000-0000-0000-000000000000
```
Confirm `ok: false`, diagnostic mentions `not found`.

**Done when.** Active record returns the correct transition list with `isImmutable: false`; final-state record returns `isImmutable: true` and empty transitions; unknown ID returns `ok: false` with a clear error.

---

### S20 — Validate RFC-018 I-81: identityInstanceId type check

**Intention.** *"I want to confirm that my repository's identity pointer is set up correctly — and if it still points at a legacy note, get a clear migration hint without the repository being rejected as invalid."*

**Capabilities exercised.** RFC-018 I-81 extension in `validate_repository`: Advisory `Warning` when `identityInstanceId` resolves to a Tier-0 Note or a Tier-2 record of the wrong type; no diagnostic when it resolves to a `com.semanticops.core/purpose` Record; repo remains `ok: true` throughout.

**CLI surface.** `repo create`, `note create`, `container create`, `container members add`, `repo set-root-container`, `repo validate`.

**Note (post-#424).** `repo create` now always scaffolds a `com.semanticops.core/purpose` record and sets `identityInstanceId` to it (the no-warning happy path). This scenario tests the **legacy/override** case: after `repo create`, `set-root-container` deliberately points `identityInstanceId` at a Tier-0 note to trigger the RFC-018 I-81 warning. This path is still important for repos created before #424 landed, and for any user who manually overrides the pointer.

**Steps.**

```bash
SRS_BIN=target/debug/srs
SCRATCH=/tmp/dogfood-rfc018
rm -rf "$SCRATCH"

# repo create auto-scaffolds a purpose record and sets identityInstanceId (happy path, #424)
$SRS_BIN repo create --repo "$SCRATCH" --namespace com.example.rfc018test --pretty

NOTE_ID=$($SRS_BIN note create --repo "$SCRATCH" <<'EOF' | python3 -c "import sys,json; print(json.load(sys.stdin)['payload']['note']['instanceId'])"
{"title": "Placeholder identity", "sections": []}
EOF
)

CONTAINER_ID=$($SRS_BIN container create --repo "$SCRATCH" <<'EOF' | python3 -c "import sys,json; print(json.load(sys.stdin)['payload']['container']['containerId'])"
{"title": "Root", "memberInstanceIds": []}
EOF
)

# Override identityInstanceId to a Tier-0 note — simulates the legacy case that triggers I-81
$SRS_BIN container members add --repo "$SCRATCH" "$CONTAINER_ID" "$NOTE_ID" --pretty
$SRS_BIN repo set-root-container --repo "$SCRATCH" --container-id "$CONTAINER_ID" --identity-instance-id "$NOTE_ID" --pretty
$SRS_BIN repo validate --repo "$SCRATCH" --pretty
$SRS_BIN repo navigation --repo "$SCRATCH" --pretty
```

**Done when.** `repo validate` returns `ok: true` (not `false`), `summary.errors: 0`, `summary.warnings: 1`, and `diagnostics[0]` is a `"warning"` severity entry with `"path": "manifest.json"` whose message contains `"RFC-018 I-81"` and `"Tier-0 Note"`. The repo is loadable despite the warning — migration is needed (#426), not a hard rejection.

`repo navigation` also returns `ok: true` (not an error), `payload.navigation.identity.instanceId == $NOTE_ID`, `payload.navigation.identity.displayLabel == "Placeholder identity"` (the note title from the index), and `payload.navigation.diagnostics` contains exactly one entry whose message contains `"Tier-0"`. This confirms the graceful branch lands in the navigation payload rather than propagating a hard error (#427).

**Negative case (not applicable in isolation).** A Tier-2 record of the wrong type (e.g. `governance/article`) also emits an RFC-018 I-81 warning with the actual type in the message, and the repo still returns `ok: true`. The scaffold integration test `crates/srs-repository/tests/scaffold.rs` covers this path.

**Note.** Once the RFC-018 I-81 warning appears, use `repo migrate-identity` (S21) to resolve it.

---

### S21 — Graduate a Tier-0 identity note to a purpose record (`repo migrate-identity`)

**Intention.** *"My repository's identity is stored as a free-text note (Tier 0). I want to promote it to the formal `com.semanticops.core/purpose` record so the RFC-018 identity invariant is satisfied and repo validate is clean."*

**Capabilities exercised.** `migrate_identity_service`: extracts statement + title from the existing Tier-0 note, creates a `com.semanticops.core/purpose` Tier-2 Record via `create_record` (CFR validation runs at write time, ADR-002 #481), updates `manifest.container.identityInstanceId` and the persisted Container file's `identityInstanceId` in lockstep, adds the new record to the container's `memberInstanceIds`, all in a single ADR-021 batch. Old identity note is preserved (not deleted).

**CLI surface.** `repo create`, `note create`, `container create`, `container members add`, `repo set-root-container`, `repo validate`, `repo migrate-identity`.

**Steps.**

```bash
SRS_BIN=target/debug/srs
SCRATCH=/tmp/dogfood-migrate-identity-s21
rm -rf "$SCRATCH"

$SRS_BIN repo create --repo "$SCRATCH" --namespace com.example.dogfood --pretty

NOTE_ID=$($SRS_BIN note create --repo "$SCRATCH" <<'EOF' | python3 -c "import sys,json; print(json.load(sys.stdin)['payload']['note']['instanceId'])"
{
  "title": "I Build Better Knowledge Tools",
  "sections": [{"name": "body", "content": "I build tools that help teams govern and share knowledge across time and context."}]
}
EOF
)

CONTAINER_ID=$($SRS_BIN container create --repo "$SCRATCH" <<'EOF' | python3 -c "import sys,json; print(json.load(sys.stdin)['payload']['container']['containerId'])"
{"title": "My Repo", "namespace": "com.example.dogfood", "name": "root"}
EOF
)

# Identity must be in the container's member list before set-root-container
$SRS_BIN container members add --repo "$SCRATCH" "$CONTAINER_ID" "$NOTE_ID"
$SRS_BIN repo set-root-container --repo "$SCRATCH" \
  --container-id "$CONTAINER_ID" \
  --identity-instance-id "$NOTE_ID" --pretty

# Before migration: validate warns about Tier-0 identity (ok: true, warnings: 1)
$SRS_BIN repo validate --repo "$SCRATCH" --pretty

# Run migration
$SRS_BIN repo migrate-identity --repo "$SCRATCH" --pretty

# After migration: validate is clean (ok: true, warnings: 0)
$SRS_BIN repo validate --repo "$SCRATCH" --pretty
```

**Negative case.** Running `repo migrate-identity` a second time on the same repo returns `ok: false` with `"already a com.semanticops.core/purpose record; no migration needed"` in `diagnostics`.

```bash
$SRS_BIN repo migrate-identity --repo "$SCRATCH" --pretty   # second call: must error
```

**Done when.**
- First `repo validate`: `summary.warnings: 1`, message contains `"RFC-018 I-81"` and `"Tier-0 Note"`.
- `repo migrate-identity` payload: `oldIdentityTier: 0`, `statement` matches the note body, `title` matches the note title, `newIdentityId` is a valid UUID.
- Second `repo validate`: `summary.errors: 0`, `summary.warnings: 0`.
- Second `repo migrate-identity` call: `ok: false`, `diagnostics[0]` contains `"already"`.
- The persisted root Container file's `identityInstanceId` equals `newIdentityId` from the payload (confirms the #462 fix — the container file and `manifest.container` agree).

---

### S21b — Bootstrap identity for a pre-#424 repository with no identity pointer (`repo migrate-identity` None-branch, #432)

**Intention.** *"I have an older repository that was created before #424 shipped — it has no `identityInstanceId` in `manifest.container` at all. I want to run `repo migrate-identity` to derive a purpose record from the container's title and description so I can bring the repo into conformance without losing any existing content."*

**Capabilities exercised.** `migrate_identity_service` None-branch: when `manifest.container.identityInstanceId` is absent, derives a `com.semanticops.core/purpose` record directly from `container.title` / `container.description`; writes the record, updates `identityInstanceId`, persists container file, all in a single ADR-021 batch. `oldIdentityId` and `oldIdentityTier` are absent from the payload (pre-#424 repos have no prior identity to report).

**CLI surface.** `repo create`, `repo migrate-identity`, `repo validate`.

**Setup (simulate a pre-#424 repo).**

```bash
SRS_BIN=target/debug/srs
SCRATCH=/tmp/dogfood-migrate-identity-s21b
rm -rf "$SCRATCH"

$SRS_BIN repo create --repo "$SCRATCH" --namespace com.example.dogfood \
  --title "SRS Dog Food" --description "We build tools to govern knowledge." --pretty

# Strip identityInstanceId to simulate a repo created before #424.
# (In a real pre-#424 repo this field is simply absent from the manifest.)
python3 - <<'PYEOF'
import json, os, glob
SCRATCH = "/tmp/dogfood-migrate-identity-s21b"
with open(f"{SCRATCH}/manifest.json") as f:
    m = json.load(f)
iid = m["container"].pop("identityInstanceId", None)
m["container"]["memberInstanceIds"] = []
m["instanceIndex"] = [e for e in m.get("instanceIndex", []) if e.get("instanceId") != iid]
with open(f"{SCRATCH}/manifest.json", "w") as f:
    json.dump(m, f, indent=2)
for rf in glob.glob(f"{SCRATCH}/records/**/*.json", recursive=True):
    if json.loads(open(rf).read()).get("instanceId") == iid:
        os.remove(rf)
PYEOF
```

**Steps.**

```bash
# Before migration: validate is clean (no identity invariant errors for absent pointer)
$SRS_BIN repo validate --repo "$SCRATCH" --pretty

# Migrate: derives purpose record from container.title / container.description
$SRS_BIN repo migrate-identity --repo "$SCRATCH" --pretty

# After migration: validate must still be clean
$SRS_BIN repo validate --repo "$SCRATCH" --pretty
```

**Negative case.** Running a second time returns `ok: false` — second call hits the Some-branch because `identityInstanceId` is now set and the pointed record is already a purpose type.

```bash
$SRS_BIN repo migrate-identity --repo "$SCRATCH" --pretty   # second call: must error
```

**Negative case 2.** A repo whose container has an empty title **and** no description cannot derive a statement; migration must error.

```bash
# (Testing via unit tests: migrate_from_container_errors_if_empty_title_and_no_description)
cargo test -p srs-repository migrate_from_container_errors_if_empty_title_and_no_description
```

**Done when.**
- `repo migrate-identity` (first call): `ok: true`, `payload.newIdentityId` is a valid UUID, `payload.statement` equals `container.description` (preferred) or `container.title` (fallback), `payload.oldIdentityId` is absent, `payload.oldIdentityTier` is absent.
- `repo validate` after migration: `summary.errors: 0`, `summary.warnings: 0`.
- Second `repo migrate-identity` call: `ok: false`, `diagnostics[0]` contains `"already"`.
- `manifest.container.identityInstanceId` equals `payload.newIdentityId`.
- `manifest.container.memberInstanceIds` contains `payload.newIdentityId`.

---

## S22 — Keeping declared extensions in sync with repo content (#237)

**Intention.** I want to know whether my repo's `declaredExtensions` list accurately reflects which SRS extensions the content actually uses — so I can catch documentation gaps before sharing the repo with others.

**Prepare.**

```bash
SCRATCH=$(mktemp -d)
$SRS_BIN repo create --repo "$SCRATCH" --namespace com.example.dogfood
```

**Happy path — empty repo reports nothing used or declared.**

```bash
$SRS_BIN repo extensions conformance --repo "$SCRATCH" --pretty
```

`payload.declared`, `payload.usedButUndeclared`, and `payload.declaredButUnsupported` are all empty. `payload.supported` lists all 7 implemented extension IDs.

**Scenario — declare a supported extension, confirm it is no longer a gap.**

```bash
$SRS_BIN repo extensions enable --repo "$SCRATCH" ext:lifecycle --pretty
$SRS_BIN repo extensions conformance --repo "$SCRATCH" --pretty
```

`payload.declared` contains `"ext:lifecycle"`. `payload.usedButUndeclared` is empty (nothing detected in content yet). `payload.declaredButUnsupported` is empty.

**Negative case — declare an unsupported extension.**

```bash
$SRS_BIN repo extensions enable --repo "$SCRATCH" ext:federation --pretty
$SRS_BIN repo extensions conformance --repo "$SCRATCH" --pretty
```

`payload.declaredButUnsupported` contains `"ext:federation"`. This flags that the repo claims to rely on an extension the engine does not implement — a portability warning.

**Validate repo is still healthy.**

```bash
$SRS_BIN repo validate --repo "$SCRATCH" --pretty
```

`ok: true`, `diagnostics: []` — conformance mismatches are informational, not validation errors.

**Done when.**
- Empty repo: `declared: []`, `usedButUndeclared: []`, `declaredButUnsupported: []`, `supported` has 7 entries.
- After `extensions enable ext:lifecycle`: `declared` contains `"ext:lifecycle"`, `declaredButUnsupported` empty.
- After `extensions enable ext:federation`: `declaredButUnsupported` contains `"ext:federation"`.
- `repo validate` returns `ok: true` throughout.

**Capabilities exercised.** `repo extensions list/enable/conformance`; `SUPPORTED_EXTENSIONS` constant; `detect_used_extensions`; `declaredButUnsupported` and `usedButUndeclared` computation.

---

### S23 — Cross-field validation: governance decision must post-date deliberation

**Intention.** *"I'm recording a governance decision. Our process requires that the decision date always comes after the deliberation period began — I want the system to enforce that automatically so no one can accidentally log a decision that pre-dates the deliberation."*

**Capabilities exercised.** `ext:cross-field-validation` — `field-ordering` rule on a Type's `validationRules` enforced **at write time** (`record create` / `record update`, #437) and during `repo validate`; happy-path record with valid date ordering is accepted and produces no diagnostic; out-of-order `record create` is **hard-rejected** with a `RecordValidation` error (no record is persisted). The same rule also surfaces in `repo validate` for any pre-#437 records already in the repository.

**CLI surface.** `field create`, `type create` (with `validationRules`), `record create`, `repo validate`.

**Steps.**

```bash
SRS_BIN=target/debug/srs
SCRATCH=/tmp/dogfood-cfr-s23
rm -rf "$SCRATCH"
$SRS_BIN repo create --repo "$SCRATCH" --namespace com.example.dogfood

# Create the two date fields
FIELD1_ID=$($SRS_BIN field create --repo "$SCRATCH" <<'EOF' | python3 -c "import sys,json; print(json.load(sys.stdin)['payload']['field']['id'])"
{"name":"deliberation_date","namespace":"com.example.dogfood","version":1,"valueType":"date","description":"When deliberation began"}
EOF
)

FIELD2_ID=$($SRS_BIN field create --repo "$SCRATCH" <<'EOF' | python3 -c "import sys,json; print(json.load(sys.stdin)['payload']['field']['id'])"
{"name":"decision_date","namespace":"com.example.dogfood","version":1,"valueType":"date","description":"When the decision was made"}
EOF
)

# Create the type with a field-ordering validationRule: decision_date must-follow deliberation_date
TYPE_ID=$($SRS_BIN type create --repo "$SCRATCH" <<EOF | python3 -c "import sys,json; print(json.load(sys.stdin)['payload']['type']['id'])"
{
  "namespace": "com.example.dogfood",
  "name": "governance_decision",
  "version": 1,
  "description": "A governance decision record with ordering constraint",
  "createdAt": "2026-07-09T00:00:00Z",
  "fields": [
    {"fieldId":"$FIELD1_ID","order":1,"required":true},
    {"fieldId":"$FIELD2_ID","order":2,"required":true}
  ],
  "validationRules": [{
    "type": "field-ordering",
    "predicateFieldId": "$FIELD1_ID",
    "targetFieldId": "$FIELD2_ID",
    "effect": "must-follow",
    "message": "decision_date must follow deliberation_date"
  }]
}
EOF
)

# Happy path: decision_date (2026-07-10) follows deliberation_date (2026-07-01) — accepted
echo '{
  "fieldValues": [
    {"fieldId":"'"$FIELD1_ID"'","value":"2026-07-01"},
    {"fieldId":"'"$FIELD2_ID"'","value":"2026-07-10"}
  ]
}' | $SRS_BIN record create --repo "$SCRATCH" --type com.example.dogfood/governance_decision --version 1 --pretty

$SRS_BIN repo validate --repo "$SCRATCH" --pretty
```

**Negative case.** Attempt to create a record where decision_date (2026-06-01) precedes deliberation_date (2026-07-01). As of #437, this is now rejected **at write time** — the command returns `ok: false` and no record is persisted.

```bash
# This must fail at record create — not at repo validate — because CFR enforcement
# now lives in the write path (create_record_at_dir). No record is written.
echo '{
  "fieldValues": [
    {"fieldId":"'"$FIELD1_ID"'","value":"2026-07-01"},
    {"fieldId":"'"$FIELD2_ID"'","value":"2026-06-01"}
  ]
}' | $SRS_BIN record create --repo "$SCRATCH" --type com.example.dogfood/governance_decision --version 1 --pretty
# → ok: false; diagnostics contains "field-ordering" and references decision_date field ID

# repo validate still returns ok: true because the violating record was never written
$SRS_BIN repo validate --repo "$SCRATCH" --pretty
```

**Done when.**
- Happy-path `record create` returns `ok: true`; `repo validate` returns `ok: true`, `diagnostics` is empty.
- Negative-case `record create` returns `ok: false`; `diagnostics[0]` contains `"field-ordering"` and references `decision_date`'s field ID; no new record appears in `record list`.
- `repo validate` after the negative case still returns `ok: true` (violating record was never persisted).

> **Write-boundary change (#437):** Before #437, a violating `record create` would succeed and only be caught by a subsequent `repo validate`. Now it is a hard error at write time, consistent with required-field violations.

### S24 — Resolve a core type in a fresh repo with no package config (implicit core merge, #423)

**Intention.** *"I have a brand-new repository with no `packageRefs`. I want to create a `com.semanticops.core/purpose` record without declaring anything in the manifest — the runtime should make core types available automatically."*

**Capabilities exercised.** Implicit core package merge (`core_package::merge_core_into_package`): `FileStore::load_package` silently prepends the embedded `com.semanticops.core` fields and types to every package; `srs type list` surfaces core types; `srs repo map` reports a `corePackage` summary; a repo that tries to shadow a core type gets a loud error.

**CLI surface.** `repo create`, `repo map`, `type list`, `record create` (with a core type), `repo validate`.

**Steps.**

```bash
SRS_BIN=$(cargo build --bin srs 2>&1 | tail -1; echo "$(pwd)/target/debug/srs")
SCRATCH=/tmp/dogfood-s24
rm -rf "$SCRATCH"

# 1. Create a fresh repo with no packageRefs
$SRS_BIN repo create --repo "$SCRATCH" --namespace com.example.dogfood --pretty

# 2. Confirm repo map shows corePackage summary
$SRS_BIN repo map --repo "$SCRATCH" --pretty
# payload.repoMap.corePackage.name should be "core" (package name from the bundle)
# payload.repoMap.corePackage.types should include "com.semanticops.core/purpose"

# 3. Confirm type list shows core types despite zero packageRefs in manifest
$SRS_BIN type list --repo "$SCRATCH" --pretty
# Output must include com.semanticops.core/purpose

# 4. Create a purpose record using the core type directly
echo '{
  "fieldValues": [
    {"fieldId":"3b000001-0000-4000-a000-000000000001","value":"This repo proves implicit core merge"},
    {"fieldId":"3b000002-0000-4000-a000-000000000002","value":"Dogfood S24"}
  ]
}' | $SRS_BIN record create --repo "$SCRATCH" --type com.semanticops.core/purpose --pretty

# 5. Validate — must be clean
$SRS_BIN repo validate --repo "$SCRATCH" --pretty
```

**Negative case.** Attempt to define a field whose ID matches a core field ID but with a different namespace (simulating a repo that tries to shadow a core definition).

```bash
# The CLI does not support creating fields with an explicit UUID today — this case
# is covered by the unit test `load_package_repo_declaring_core_field_conflicts` in
# crates/srs-repository/src/store.rs, which injects the conflict at the MemoryStore level.
# File srs-rust#<follow-up> to expose a CLI negative-case path if the field-create command
# gains explicit-UUID support.
```

**Done when.**
- Step 2: `payload.repoMap.corePackage.name` is `"core"` and `payload.repoMap.corePackage.types` contains `"com.semanticops.core/purpose"`.
- Step 3: `srs type list` output includes at least one entry with `namespace: "com.semanticops.core"`.
- Step 4: `record create` returns `ok: true`; the returned record has `typeNamespace: "com.semanticops.core"` and `typeName: "purpose"`.
- Step 5: `repo validate` returns `ok: true`, `diagnostics` is empty.

---

## Coverage matrix

Maps each CLI command group to the scenario(s) that exercise it. A command group with **no scenario** is a dogfooding gap — adding or changing such a surface in a PR means extending a scenario or adding one (see below).

| Command group | Exercised by |
|---|---|
| `repo` (map, validate, init) | S1–S6 (orientation + validation in every scenario); `repo validate` now includes manifest.json schema validation — see S1 negative case; RFC-013 I-79/I-80/I-81/I-82 root-container invariants — see S1 negative case (I-79) and S15 step 10 (full happy path); blueprint semantic validation + protocol stage-dependency validation — see S13 (`repo validate` on a repo with a protocol); **RFC-018 I-81** identity type check (Warning when `identityInstanceId` resolves to a Tier-0 Note or wrong Tier-2 type) — see S20; **`repo create` always scaffolds a `com.semanticops.core/purpose` Tier-2 record and sets `identityInstanceId` unconditionally (#424)** — happy path covered by S17 step 3 (navigation reads back the purpose record); **ext:lifecycle V7/V8/V9 invariants now enforced at validate time (#239)**: V7 (type declares both `lifecycle` and `lifecycleRef`), V8 (lifecycleRef UUID does not resolve), V9 (inline TypeLifecycle structural errors + `initialState`/`isInitial` key mismatch) — see S6 negative case; **RFC-020 Rule [N+33]** identityFieldId effective-field-set check (a Type's `identityFieldId`, own or inherited, must resolve to a `fieldId` in that Type's effective field set; runs independent of whether any record of that Type exists, and one Type's resolution error does not block others) — see S1 negative case (#376); **`repo map` now includes `payload.corePackage` summary** (id, name, version, types, fields) from the embedded `com.semanticops.core` package (#423) — see S24 |
| `implicit core type availability` (`srs type list` shows `com.semanticops.core/*`; `srs repo map` shows `corePackage`; zero-config `com.semanticops.core/purpose` resolution, #423) | S24 |
| `repo init-new` (re-stamp seed identity) | S16 |
| `repo set-root-container` (write manifest.container pointer) | S17 |
| `repo copy` | S9, S10 |
| `repo upgrade` (in-place path normalisation) | S9b |
| `.srsj` write determinism (idempotent, minimal-diff) | S10 |
| `note` (create/get/list/update/delete) | S1, S10; `note create --container` (best-effort rollback on `add_member` failure, #455) — happy path verified end-to-end; error-path trigger deferred (requires FailStore test double, see ADR-024) |
| `note graduate` (atomic Note→Record promotion) | S18 |
| `field` (create/list/get/update/delete) | S2 |
| `type` (create/get/list/schema/update/delete) | S2 |
| `record` (create/get/list/update/delete) | S1, S2, S4 |
| `record update` with `typeVersion` migration (srs-rust#42) | S2 negative case — pass `typeVersion: 2` when package has advanced past stored version; confirm `ok: true` and returned record carries `typeVersion: 2`; pass `typeVersion: 99` and confirm `ok: false` with `"type version 99 not found"` diagnostic; `repo validate` must be 0 errors throughout |
| `record list`/`record get` core `displayLabel` (tree-parity, type_name fallback; RFC-020 `identityFieldId` priority + Rule [N+33] dangling-reference diagnostic, #376) | S1 (list), S18 (get — `payload.instanceId` + `payload.displayLabel` since #294) |
| `record validate` (no-write preflight) | S2 |
| `record transition` | S4, S6; WASM binding (`set_lifecycle_state`) now returns `{ record, warnings }` — verified via integration tests in `crates/srs-bindings/tests/relation_lifecycle.rs` (#367) |
| `record allowed-transitions` (ext:lifecycle query path, ADR-022) | S19 |
| `record successor` | S3, S4 |
| `record tag` | S6 |
| `relation` (create/list/get/delete) | S1, S3, S5 |
| `relation-type` | CLI: _gap — no scenario yet_; WASM read binding (`list_relation_types`) verified via integration tests in `crates/srs-bindings/tests/definition_browse.rs` (#411) |
| `container` (create/members/roots/validate/…) | S4; container CRUD on `.srsj` (create/list/delete) verified end-to-end on branch (#466); slug-named container path resolution (`.srsj` packed from FileStore with `manifest.containerIndex.path` entries) covered by `srs-repository` unit tests (`json_store_container_slug_path_resolution`, `json_store_save_container_writes_manifest_index`, etc.); **pre-#466 shadow containerIndex migration** (open-time derivation of missing `path` in `from_srsj`, #490) — dogfooded on branch: `container list` on a hand-crafted pre-#466 `.srsj` returns the container; `repo validate` produces no `[/containerIndex/N] "path" is a required property` error (the regressed behaviour); covered by unit tests `from_srsj_shadow_migration_derives_path_for_pathless_entry` and `from_srsj_shadow_migration_skips_entry_with_no_matching_data_key` |
| `container update` (`rootInstanceIds`/`memberInstanceIds`/`identityInstanceId` patch fields, `deny_unknown_fields`) | S4 (extended — patch membership fields, unknown-field error); verified end-to-end in worktree dogfood (#422) |
| `container resolve-view` (structured container view, `--view-id`; `ColumnSpec.isIdentityColumn`, ADR-023, #376) | S14 |
| `container resolve-view` authored `excludeLifecycleStates` (ADR-020) | S15 |
| `find` (ext:discovery query — type/tag/lifecycle/exclude/text) | S15 |
| `repo navigation` (RFC-013 root container + identity + sections) | S15, S17; WASM binding (`repository_navigation`) verified via integration tests in `crates/srs-bindings/tests/navigation.rs` (#268); Tier-0 note identity grace (returns Ok + diagnostic instead of erroring) — see S20 (#427); "root is also a member" shape (sub-container root in its own `memberInstanceIds`) — `repository_navigation_root_is_member_of_its_own_sub_container` in both unit and WASM integration tests (#460) |
| `srs-gov` (governance client: `repo-create`, `list` + `--all`/`--search`/`--tag`, `tui --smoke`) | S15; `SrsRepository::load` WASM binding applies RFC-014 migration automatically (#381); `crates/srs-repository/tests/scaffold.rs` covers the migrate→scaffold→validate chain; `migrate_rfc014` now unconditionally strips `contentHash` from already-promoted bundles (regression #428, see `migrate_rfc014_strips_content_hash_from_already_promoted_bundle`) |
| `document-view` (create/get/list/…) | S4, S5, S11 |
| `render document-view` | S4, S5, S8, S11; **RFC-020 Rule [N+37]** identity-field fallback heading (no `titleFieldId` → Type's `identityFieldId` → `### <value>` per record; structured mode NOT activated by fallback, #453) — see S5 step 8; **`{{container-title}}` fallback to container file when index entry has no title** (#484): `resolve_container_title` now calls `store.load_container` when the containerIndex entry is absent or has no title — dogfooded on branch: `srs render document-view --view <dv> --repo /tmp/dogfood-resolve-container-title --container <cid>` returns `"Container: Recognising decisions"` when manifest has a pathless-title containerIndex entry (previously returned repo title "DogfoodRepo"); negative case (no `--container`) returns `"Container: DogfoodRepo"` (manifest title fallback correct) |
| `container-subset` section + `typeFilter` / `typeDispatch` (RFC-008) | S11 |
| `type-query` lifecycle filter (`lifecycleStates`, `excludeLifecycleStates`, `containerScope`) (RFC-011) | S12 |
| `view` (L1 — `view list`, `view get`) | CLI surface: _gap — no CLI scenario yet_; WASM read bindings (`list_views`, `get_view`) verified via integration tests in `crates/srs-bindings/tests/definition_browse.rs` (#330) |
| `tree` | S5 |
| `vocabulary` (create/get/list/term-create/derive-tag-set/promote) | S6 |
| `term` (list/get) | S6 |
| `lifecycle` (list/get) | S4, S6 |
| `lifecycleRef` create/transition (referenceable lifecycle) | S6 (step 7 extended) |
| `blueprint` (list/get/validate/structure/schema/brief) | S7 |
| `protocol` (create/list/get/stages/find-by-target-type) | S13 |
| `theme` | S8 |
| `extension` | _gap — no scenario yet_ |
| `repo extensions` (list/enable/disable/conformance) | S22; WASM read binding (`declared_extensions_conformance`) verified via smoke test `declared_extensions_conformance_report_serialises` in `crates/srs-bindings/src/lib.rs` (#442) |
| `repo migrate-identity` (graduate Tier-0 identity note to purpose record, #426; bootstrap identity for pre-#424 repos with no `identityInstanceId`, #432) | S21 (Tier-0 note branch), S21b (None-branch: absent pointer); WASM binding (`migrate_identity`) verified via integration tests in `crates/srs-bindings/tests/migrate_identity.rs` (#434); `build_purpose_record` now uses `core_package::core_package()` lookups instead of hardcoded UUID constants (ADR-025, #434) |
| `type` `validationRules` (ext:cross-field-validation — conditional-required / field-ordering / mutual-exclusion, #242); **CFR violations are now hard errors at `record create`/`record update` write time (#437)** — `repo validate` still enforces for any pre-existing records | S23 |
| `tag` (definition) | _gap — being deprecated; see open issues_ |
| `package` | CLI: covered implicitly by field/type creation in S2; WASM read binding (`list_packages`) verified via integration tests in `crates/srs-bindings/tests/definition_browse.rs` (#330) |

Gaps are intentional and visible: they are the backlog of surfaces that need a meaningful scenario. Do not delete a gap row — fill it when a feature gives the surface a real workflow to demonstrate.

**WASM bindings** are not directly CLI-drivable and are not covered by scenarios here. WASM binding coverage lives in `crates/srs-bindings/tests/` and `crates/srs-repository/tests/` — integration tests that call `srs-repository` services via `srsj_migration_service::load_from_srsj` (the recommended entry point, which applies RFC-014 migration) or `JsonStore::from_srsj` (for already-migrated inputs). When a WASM binding PR adds or changes the binding surface, update the coverage matrix row for the underlying CLI command (or add a note like the `view` and `package` rows above).

## Maintaining this guide

`/ship` Stage 11 keeps this guide current. When a PR adds or changes a CLI command, flag, stdin shape, or observable behaviour:

1. **If an existing scenario already covers that surface**, run it against the change and, if the change alters the workflow, update the scenario's steps / done-when so they reflect reality.
2. **If the surface is a `gap` row (or entirely new)**, decide whether it belongs in an existing scenario (extend it) or needs a new one. A new scenario must lead with a *meaningful intention* — if you can't state the intention, the feature may not yet be ready to dogfood, and that itself is worth noting on the issue.
3. **Update the coverage matrix** in the same PR so it never drifts from the scenarios.
4. Keep scenarios runnable: every command block must work against a real repo. A scenario step that no longer runs is a regression in this guide.

Scenarios should stay few and meaningful. Prefer deepening an existing scenario over proliferating shallow ones.
