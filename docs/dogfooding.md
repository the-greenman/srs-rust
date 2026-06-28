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

**CLI surface.** `note create`, `note get`, `note list`, `record create`, `record list` (per-record `displayLabel`), `tree`, `relation create`, `repo validate`.

**Steps.**
1. Orient: `srs repo map --repo <repo> --pretty`.
2. Capture raw thinking as a Note (Tier 0): `srs note create` with a free-text `sections[]` body.
3. Later, create a Tier 2 Record that structures the same idea against a real type (see S2 if the type doesn't exist yet).
4. Assert `derived-from` from the Record to the Note so the lineage survives.
5. List the records: `srs record list --repo <repo> --pretty`. Each item is `{ instanceId, displayLabel, record }` — confirm the new record's `displayLabel` is the core-resolved human label (its `title`/`name`/`label` field value, else the type name), **not** a raw UUID. Cross-check against `srs tree --repo <repo>`: the record's `displayLabel` must equal its `tree` node `label` (one core resolution, two surfaces — the client never re-derives a title).
6. `srs repo validate --repo <repo> --pretty`.

**Negative case.** Create a record referencing a `typeId` that isn't in the package — confirm `ok: false` with a diagnostic, and that no ghost file is left in `instanceIndex`. **Label fallback:** a record of a type with no `title`/`name`/`label` field still lists with a non-empty `displayLabel` equal to its `typeName` (the resolver falls back, never returns an empty label or a bare UUID).

**Done when.** Both instances appear in `record list` / `note list`; a `derived-from` relation connects them in `relation list`; validate returns zero diagnostics. The Note is *not* deleted when the Record is created — promotion preserves the origin. Every `record list` item carries a `displayLabel` equal to that record's `tree` label (verified across all records: title-bearing records show their title; field-less types fall back to `typeName`).

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

**Done when.** The Decision visibly progresses through its states; `derived-from` ties it to the Exercise; the ratified record is superseded (not mutated) on change; the decision-log view renders the ratified decision with its reasoning. The Exercise remains part of the record after a Decision is derived from it.

### S5 — Assemble and render a document (records as source of truth)

**Intention.** *"I have a set of records that together form a document. I want an ordered, human-readable rendering — and I want the rendering to follow the records, not a hand-maintained copy."*

This is the spec-as-repo pattern (`../srs/srs`): sections are records, order is a relation, the markdown is a projection.

**Capabilities exercised.** Ordering relations (`precedes`, or `members[]` sequence relations like `section-sequence`); document views (`ext:views-l2`); the RFC-009 typed anchor (`DocumentView.rootTypeRefs` — version-exact `ExactTypeRef`, the validated successor to the free-string `containerType` join); `render` as a pure projection of records + relations; `tree` as the hierarchy view.

**CLI surface.** `document-view create`, `document-view list` (incl. `--root-type <typeId>`), `document-view get`, `render document-view`, `relation create`, `tree`.

**Steps.**
1. Inspect the spec repo to see the target shape: `srs document-view list --repo ../srs/srs`, `srs render document-view --repo ../srs/srs --view <view>`.
2. In a working repo, define (or reuse) a document view that selects records by type and renders them. Anchor it to its root type with `rootTypeRefs: [{ "typeId": <type-uuid>, "typeVersion": <n> }]` (keep `containerType` as a human-readable hint if you like).
3. Establish order with `precedes` (or a `members[]` sequence relation).
4. `srs render document-view --view <view>` and read the output.
5. Reorder the records (change the `precedes`/`members` relation) and re-render — confirm the output order changed.
6. `srs tree --repo <repo>` to see the derived hierarchy.
7. Find views by anchor: `srs document-view list --root-type <type-uuid>` returns only views whose `rootTypeRefs` include that Type id; each summary carries `rootTypeRefs`.

**Negative case.** Render a view that references a type with no instances, or a view ID that doesn't exist — confirm an empty-but-valid render or a correct error envelope (not a crash). `srs document-view list --root-type <unknown-uuid>` returns an empty list with `ok: true` (not an error). RFC-009 validation diagnostics are **advisory `Warning`s** that never change `errors`/exit code: declare a `rootTypeRefs` entry that does not resolve to a package Type and confirm `repo validate` emits **I-63** (the entry is ignored for matching); give a rooted Container a `containerType` that differs from its root Record's resolved Type `name` and confirm **I-64** (the hint is stale; the container stays valid).

**Done when.** The render reflects record content and the ordering relation; **changing the relation changes the render** (proving the markdown is derived); `tree` shows the expected hierarchy; `document-view list --root-type` narrows to the anchored views, and a stale `containerType` surfaces as an advisory I-64 warning with `repo validate` still reporting `0 errors`.

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
8. **`$schema` loader tolerance (#117):** If your editor adds a top-level `"$schema"` key to lifecycle or vocabulary JSON files (the standard JSON Schema association hint), confirm `lifecycle list` and `vocabulary list` still succeed. Before #117, the Lifecycle loader rejected `$schema` with "unknown field". Note: adding `$schema` via CLI is not yet supported — you will encounter this in practice when an editor or schema-aware tool writes the file. The gap (no `lifecycle create` CLI command) is tracked in issue #116.

**Negative case.** (a) Promote with an unresolvable in-use key and confirm the structured block payload lists the same keys `derive-tag-set` classified `will-be-invalid`. (b) `derive-tag-set` on an unknown vocabulary id → `ok: false` with a diagnostic (no panic). (c) Attempt a `record transition` not present in the lifecycle's `transitions` and confirm rejection — this applies to both inline and `lifecycleRef`-bound Types. (d) Confirm `lifecycle list` succeeds even when a lifecycle file carries a `"$schema"` key (the old rejection error no longer occurs).

**Done when.** `open` accepts arbitrary keys; `closed` rejects unknown keys; **`derive-tag-set`'s `will-be-invalid` set equals `promote`'s `unresolvableKeys`** — the read-only pre-flight predicts the write outcome exactly; `promote` blocks with `unresolvableKeys` exactly when an in-use key lacks an active term (and succeeds within a grace `promotionWindow` if one is set); lifecycle transitions honour the declared state machine for both inline and `lifecycleRef`-bound Types; lifecycle and vocabulary files with a `$schema` key load without error.

### S7 — Verify a document type is correctly composed (Blueprint schema + brief)

**Intention.** *"I've declared a guide document type — a root record plus an ordered set of section types. Before building an editor, an extraction pipeline, or an AI prompt on top of it, I want to verify the composition is correct and machine-readable: all section types are reachable, each type's fields are discoverable, and composite groups (like data tables) surface with enough metadata for a generic authoring tool. I also want the layered AI guidance context — field semantics, extraction hints, and any targeting protocol — composed into a single brief I can hand directly to an agent."*

**Capabilities exercised.** Blueprint as a composition validator; `blueprint schema` as the machine contract for a multi-record document; the field-group (`x-srs-composite-renderer`) hint for composite sections; `blueprint brief` as the layered guidance context for AI extraction pipelines (blueprint `aiGuidance`, each root type's `aiGuidance` + fields in `order`, structure RelationSpecs, and any targeting Protocol); non-fatal diagnostics when a root type is unresolvable, no protocol is found, or a protocol stage `contributesTo` references a field ID that doesn't exist in the package.

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

**Done when.** `payload.schema.properties.contains.items.oneOf` has exactly the section types declared in the blueprint; the table section type's definition includes the `x-srs-composite-renderer: "table"` group property; removing a type from the blueprint's `structure[]` and re-projecting drops it from `items.oneOf` — the schema is derived, not cached; `blueprint validate` shows zero diagnostics. `blueprint brief` returns a non-empty `rendered` string and structured `types[]` with field-level `aiGuidance`; missing-blueprint input yields a correct `ok: false` envelope. A protocol stage whose `contributesTo` carries an unresolvable `fieldId` yields `ok: true` with a diagnostic — the rest of the brief is unaffected.

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

**Capabilities exercised.** `protocol create` (write path), `protocol list` (compiled-model read), `protocol get` (compiled-model read by ID), `protocol stages` (stage projection from compiled model), `protocol find-by-target-type` (lookup by target typeId). This scenario specifically verifies that the refactored read-side service functions source data from the compiled `Package.protocols` (populated at load time) rather than re-reading package files on every call.

**CLI surface.** `protocol create`, `protocol list`, `protocol get`, `protocol stages`, `protocol find-by-target-type`, `repo validate`.

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
7. `srs repo validate --repo /tmp/dogfood-protocols --pretty` → `ok: true`, `summary.errors: 0`.
8. `srs protocol find-by-target-type --type-id "com.example.dogfood/decision" --repo /tmp/dogfood-protocols --pretty` → `ok: true`, `payload.protocolId` = `"com.example.dogfood/extraction-protocol"`, `payload.stages` has 1 entry.

**Negative case.** `srs protocol get --repo /tmp/dogfood-protocols com.example.dogfood/nonexistent --pretty` → `ok: false`, `diagnostics[0]` contains `"not found"`. `srs protocol list` on a freshly-created repo (no protocols declared) → `ok: true`, `payload.protocols: []`. `srs protocol find-by-target-type --type-id "type-no-match" --repo /tmp/dogfood-protocols` → `ok: false`, `diagnostics[0]` contains `"No protocol found with target type"`.

**Done when.** `protocol list` returns the created protocol; `protocol get` returns the full definition including all stages; `protocol stages` returns the stage list; `protocol find-by-target-type` returns `{ protocolId, protocolName, stages, diagnostics }` for a known typeId and a clean `ok: false` envelope for an unknown typeId; a missing-ID get returns `ok: false` with a diagnostic naming the missing ID; `repo validate` shows 0 errors; `protocol list` on an empty repo returns an empty array without error.

---

### S14 — Drive an editor member list from a DocumentView's field selection (`container resolve-view`)

**Intention.** *"I'm building an interactive, selectable list of a container's members in the editor. In one call I need the columns to show — driven by the DocumentView's field selection, not by my client knowing the types — plus each member's display label and full record, and the container's root for the header. I should never compute 'what columns' or 'what label' in the client."*

This is the issue-#254 capability: a single `resolve_container_view` projection (service → CLI payload → WASM binding, per `docs/architecture/capability-layering.md`) returning the container root record, the ordered Tier-2 member records (full `Record` + core-resolved `displayLabel` + `tier`), and the **column/field spec** resolved from a DocumentView section's `renderViewId → View.field_views`. Column-source precedence is [ADR-018](adr/018-container-view-column-source-precedence.md): the section targeting this container wins, else the first section by `order` with a `renderViewId`, else empty columns.

**Capabilities exercised.** Container membership (roots-first, deduped); DocumentView → View `field_views` column projection (visible-false exclusion, `order` sort, `displayLabel` override → field `name` fallback); core `record_display_label` reuse for member/root labels; Tier-gating (non-Tier-2 members skipped with a diagnostic); `--view-id` override vs. root-type matching; non-fatal diagnostics vs. hard errors. The anchor repo is the `rfc008-container-subset` fixture (heterogeneous container `…3500`, text-view `…3504`).

**CLI surface.** `container resolve-view` (`--view-id` flag), `document-view create`, `repo validate`.

**Steps.**
1. Orient and validate: copy the fixture to a scratch dir, then `srs repo validate --repo /tmp/dogfood-resolve-container-view` → `ok: true`, `summary.checked: 4`, 0 errors.
2. **Default (root-type matched) view** — `srs container resolve-view 00000000-0000-4000-8000-000000003500 --repo <repo>`. The fixture's container has no root binding and its views declare no `rootTypeRefs`, so no DocumentView matches: confirm `payload.containerView.documentViewId` is absent, `columns` is empty, **and** all four members still come back with core-resolved labels (`Text-One`, `Text-Two`, `Table-One`, `Table-Two`) — the member list never depends on a view resolving.
3. **Author a member-list view** — `document-view create` (stdin) a DocumentView whose `container-subset` section targets `…3500` and carries `renderViewId: …3504` (the text-view). `createdAt` is required in stdin. Capture the new `payload.documentView.id`.
4. **Column projection** — `srs container resolve-view 00000000-0000-4000-8000-000000003500 --view-id <new-dv-id> --repo <repo>`. Confirm `columns` is exactly one entry resolved from the text-view: `fieldName: "title"`, `displayLabel: "Text Title"` (the `FieldView.displayLabel` override), `order: 0`; `documentViewId` is the authored view; all four members carry a `displayLabel` and a full `record` object.
5. `srs repo validate --repo <repo>` → still `ok: true`, 0 errors (authoring the view did not corrupt the repo).

**Negative case.** Two paths: (a) a nonexistent container — `srs container resolve-view 00000000-0000-0000-0000-deadbeef0000 --repo <repo>` → `ok: false`, top-level `diagnostics[0]` contains `"container not found"`, and `payload` is null (no partial/ghost result). (b) an unknown `--view-id` — `srs container resolve-view …3500 --view-id <missing> --repo <repo>` → `ok: true`, `documentViewId` absent, `columns` empty, `payload.containerView.diagnostics` contains `"documentView <missing> not found"`, and the four members are still returned (an unresolved view is a diagnostic, not a failure).

**Done when.** One call returns root + ordered members + per-member label + DocumentView-driven column spec; columns honour visibility, order, and the displayLabel override and resolve field names from the package; a non-Tier-2 member would be skipped with a diagnostic (not crash the call); an unknown `--view-id` degrades to empty columns + diagnostic while still returning members; a missing container is a clean `ok: false` with no payload; `repo validate` stays at 0 errors. The client computes no semantics — columns and labels come entirely from the payload.

**Authored list defaults (ADR-020).** `payload.containerView.excludeLifecycleStates` carries the authored default-hidden lifecycle states, read from the same governing section that drives `columns`: `[]` when that section is a `container-subset` (the `…3500` fixture above), or the declared set when it is a `type-query` (see S15). A client renders a default-hidden list by forwarding these to `find --exclude-lifecycle-state` — it never re-derives them from the DocumentView source.

---

### S15 — Interactive governance list: default-hidden states + show-all + search/tag (`srs-gov list`)

**Intention.** *"I'm running a governance decision log. By default the list should hide decisions that are `superseded` or `closed` — but that 'what's hidden' rule is **authored in the view**, not coded in my client — with a one-flag show-all toggle, plus search and tag narrowing. My client should only compose two services, never re-express the filter."*

This is issue #298 (parent plan §4): `srs-gov list` composes `container resolve-view` (authored columns + ordered members + authored `excludeLifecycleStates`, ADR-020) with `srs find` (the runtime discovery query, ADR-019). The default-hidden states come from the package's `type-query` DocumentView; `srs-gov` forwards them to `find` and intersects the hit set with the resolved members. No lifecycle/filter semantics live in the client.

**Capabilities exercised.** `srs-gov repo-create` (stamps the regenerated `type-query` governance seed); `container resolve-view` `excludeLifecycleStates` surface; `srs find` `--exclude-lifecycle-state` / `--text` / `--tag` / `--container`; the resolve-view ∩ find intersection; the `--all` show-all toggle; `--explain` printing both composed commands; `record transition` (drive lifecycle states) and `record tag add`.

**CLI surface.** `srs-gov repo-create`, `srs-gov list` (`--all`, `--search`, `--tag`, `--explain`, `--json`), `srs record create/transition/tag add`, `srs container resolve-view`, `srs find`, `repo validate`.

**Steps.**
1. `srs-gov repo-create --output /tmp/dogfood-srs-gov-list.srsj --title "Acme Co-op"` → a fresh governance `.srsj`. Confirm the stamped seed's decision-log DocumentView is a `type-query` (regenerated asset): `srs container resolve-view <decisionLogId> --repo <repo>` → `payload.containerView.excludeLifecycleStates: ["superseded","closed"]`.
2. Add four decisions in the decision-log container via `srs record create --type governance/decision --container <decisionLogId>` and drive their states with `srs record transition` (`{"to":"proposed"}` → `{"to":"ratified"}` → `{"to":"superseded"|"closed"}`): one left `draft`, one `ratified` (tag it `tooling`, statement contains a unique word like `budget` only in a non-title field), one `superseded`, one `closed`.
3. **Default** — `srs-gov list decision_log --repo <repo>` shows only the `draft` and `ratified` decisions; the `superseded` and `closed` ones are hidden.
4. **Show-all** — `srs-gov list decision_log --all` shows all four.
5. **Search** — `srs-gov list decision_log --all --search budget` returns only the decision whose `decision_statement` (a non-title field) contains `budget` — content recall, not a title match.
6. **Tag** — `srs-gov list decision_log --tag tooling` returns only the tagged decision.
7. **Explain** — `srs-gov --repo <repo> --explain list decision_log --search budget` prints the two underlying commands: `container resolve-view <id>` and `--container <id> find --exclude-lifecycle-state superseded --exclude-lifecycle-state closed --text budget`.
8. `srs repo validate --repo <repo>` → `ok: true`, 0 errors.

**Negative case.** `srs-gov list bogus_key` → a clean `error: unknown key 'bogus_key'. Known: articles, decision_log, roles` (non-zero exit), with no partial output. `--explain` placed *after* the subcommand (`list decision_log --explain`) is rejected by clap (it is a top-level flag) — the correct form is `srs-gov --explain list …`.

**Done when.** The default list hides exactly the authored states and `--all` reveals them; `--search` narrows by content over a non-title field and `--tag` by facet; `--explain` shows the composed `resolve-view` + `find` commands carrying the authored excludes; `repo validate` stays at 0 errors. Crucially, the hidden-state set lives in the package `type-query` view (and is surfaced by `resolve-view`), never hardcoded in `srs-gov` — confirm with `rg "superseded|closed" crates/srs-gov/src` returning only `#[cfg(test)]` fixtures (and help text), never production filter logic.

---

## Coverage matrix

Maps each CLI command group to the scenario(s) that exercise it. A command group with **no scenario** is a dogfooding gap — adding or changing such a surface in a PR means extending a scenario or adding one (see below).

| Command group | Exercised by |
|---|---|
| `repo` (map, validate, init) | S1–S6 (orientation + validation in every scenario) |
| `repo copy` | S9, S10 |
| `.srsj` write determinism (idempotent, minimal-diff) | S10 |
| `note` (create/get/list/update/delete) | S1, S10 |
| `field` (create/list/get/update/delete) | S2 |
| `type` (create/get/list/schema/update/delete) | S2 |
| `record` (create/get/list/update/delete) | S1, S2, S4 |
| `record list` core `displayLabel` (tree-parity, type_name fallback) | S1 |
| `record validate` (no-write preflight) | S2 |
| `record transition` | S4, S6 |
| `record successor` | S3, S4 |
| `record tag` | S6 |
| `relation` (create/list/get/delete) | S1, S3, S5 |
| `relation-type` | _gap — no scenario yet_ |
| `container` (create/members/roots/validate/…) | S4 |
| `container resolve-view` (structured container view, `--view-id`) | S14 |
| `container resolve-view` authored `excludeLifecycleStates` (ADR-020) | S15 |
| `find` (ext:discovery query — type/tag/lifecycle/exclude/text) | S15 |
| `srs-gov` (governance client: `repo-create`, `list` + `--all`/`--search`/`--tag`) | S15 |
| `document-view` (create/get/list/…) | S4, S5, S11 |
| `render document-view` | S4, S5, S8, S11 |
| `container-subset` section + `typeFilter` / `typeDispatch` (RFC-008) | S11 |
| `type-query` lifecycle filter (`lifecycleStates`, `excludeLifecycleStates`, `containerScope`) (RFC-011) | S12 |
| `view` (L1) | _gap — no scenario yet_ |
| `tree` | S5 |
| `vocabulary` (create/get/list/term-create/derive-tag-set/promote) | S6 |
| `term` (list/get) | S6 |
| `lifecycle` (list/get) | S4, S6 |
| `lifecycleRef` create/transition (referenceable lifecycle) | S6 (step 7 extended) |
| `blueprint` (list/get/validate/structure/schema/brief) | S7 |
| `protocol` (create/list/get/stages/find-by-target-type) | S13 |
| `theme` | S8 |
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
