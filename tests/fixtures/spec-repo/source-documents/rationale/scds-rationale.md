# SCDS Rationale

**Version**: 2.0
**Status**: informative companion to `scds-spec.md`

This document is non-normative. It explains design decisions, provides usage guidance, and records the reasoning behind choices made in the SCDS specification. Nothing in this document overrides `scds-spec.md`. Language like "must" and "must not" does not appear here; all normative requirements are in the spec.

---

## 1. Core Thesis

Traditional document systems treat documents as primarily text.

This specification treats documents as **socially negotiated semantic state**. Text is one projection of that state.

Six principles follow from this:

**1. Semantic state is primary; documents are projections.**
The same semantic state may be rendered as a board paper, a governance record, a dashboard, or an AI context package. None of these projections is the source of truth.

**2. Fields are reusable semantic atoms.**
A Field defines a reusable slot of meaning with stable identity. It is not a form field. It is not tied to any specific Type or View. Its AI guidance, validation rules, and value type belong to the Field, not to the Type that uses it.

**3. Types are compositions, not owners of Field semantics.**
A Type selects and orders Fields for a specific semantic object type. It may provide session-level AI framing. It must not override or redefine the meaning of any Field it includes.

**4. Lineage and provenance are first-class.**
Definitions evolve. Forks happen. Upstream changes must be traceable. A definition without lineage is a definition that cannot be trusted to evolve cleanly.

**5. Records represent negotiated semantic state, not objective truth claims.**
A Record captures what a group understood, agreed, or committed to at a point in time. That understanding may be partial, contested, or later revised. The system preserves revision history and provenance precisely because the original state is worth keeping alongside its successors. Human prose and ambiguity are preserved, not collapsed.

**6. Understanding is mutable; historical semantic state has permanent value.**
SCDS assumes that understanding evolves. Records, Relations, and lifecycle states may be revised, superseded, refined, or contradicted without invalidating prior semantic state. A rough plan is a valid semantic object. A superseded decision is a valid semantic object. An abandoned hypothesis is a valid semantic object. Historical semantic state is not noise to be discarded — it is provenance, institutional memory, and the record of how understanding arrived at its current form.

---

## 2. Design Decisions

### 2.1 Why Field and Type are separate

A form system where each template defines its own fields produces semantic silos: the "decision statement" in the Technology template and the "decision statement" in the Budget template are unrelated strings. They cannot be searched together, compared, or composed.

In SCDS, a Field is defined once. Any number of Types may include it. When two Types share a Field, any AI extraction logic, validation rules, or downstream analysis written for that Field applies consistently across both. The Field's identity is stable across all the contexts it appears in.

This is a stronger constraint than it appears. It means a Type cannot secretly redefine what a Field means for its own purposes — it can only configure presentation. If a Type genuinely needs different semantics, it must use a different Field.

### 2.2 Why "Type" not "Module"

"Module" in v1 was accurate but implied a software analogy that didn't communicate the concept well to non-technical practitioners. "Module" suggests a composable software unit. "Type" says what it actually is: a type definition for a semantic object. A Decision is a Type. A Task is a Type. A Risk is a Type.

The rename also makes the Record/Type relationship legible by analogy: a Record is an instance of a Type, just as a value is an instance of a type in any typed system.

### 2.3 Why Record tiers exist (Note → Typed Record → Record)

Not all content arrives with full semantic formalisation. A meeting note, a brainstorm document, a rough plan — these are valid starting points that should be preserved and referenceable, even before anyone has decided what Types to extract from them.

The three tiers let a system capture content at whatever maturity level it has, and formalise later without losing provenance. The graduation path is one-way: Note → Typed Record → Record. It mirrors how understanding actually develops — rough first, then structured, then formally defined.

The tier model also makes SCDS progressively adoptable. A team can start at Tier 0 and arrive at Tier 2 as their understanding of the semantic structure matures, without ever having to restart from scratch.

### 2.4 Why Protocol replaces TemplateFacilitationStep

`TemplateFacilitationStep` in v1 was field-ordering with AI guidance attached. It could specify which fields to present in which order, with optional framing. This was sufficient for a linear form-filling workflow.

But the process of building a quality Record through group deliberation is not a form-filling workflow. It is an epistemically ordered process: you cannot meaningfully evaluate options before you have articulated criteria; you cannot propose a course of action before you have characterised the problem.

Protocol stages have:
- `dependsOn` — explicit epistemic dependencies, not just ordering. A stage may not proceed until its dependencies are sufficient.
- `completionCriteria` — how to know a stage is adequate to proceed.
- `outputType` — a stage may produce its own intermediate Record, not just fill fields in the final one.
- `question` — the core epistemic question this stage answers.

The distinction is between a View (which fields to show, in what order, for presentation purposes) and a Protocol (how to build understanding epistemically, stage by stage). These are separate concerns. Collapsing them into one construct produced a type that was adequate for neither.

A Record is the *compressed output* of a Protocol run. The Protocol is the process that produced the understanding; the Record is what that understanding looks like expressed in the standard vocabulary.

### 2.5 Why Schema is a new concept

In v1, there was no way to specify what a document type *is* — what needs to be extracted from source material in order to build it. `DocumentTemplate` (now Document View) handled *assembly* of existing Records into readable output. But nothing owned the prior question: "Given a transcript of a governance meeting, what Types should I extract, how should they relate to each other, and what does 'complete' mean?"

Schema fills that gap. A Schema is the artefact you hand to an extraction pipeline. It specifies root Types, expected Relations between extracted Records, and completeness criteria. The Extraction pipeline consults the Schema to know what to look for; the Document View consults existing Records to know what to render.

The two are complementary: Schema → Records → Document View.

### 2.6 Why Address and AttentionState are needed

v1 noted "focus links" as a session-layer concern without defining a mechanism. The mechanism was absent.

Without co-addressability, the transcript/SCDS separation is clean in principle but broken in practice. There is no way to say "this conversation happened while we were focused on this Field." Retrospective `SourceReference` links help, but they require someone to explicitly annotate which conversation produced which value. For real-time facilitation, that annotation needs to happen live.

`AttentionState` is the live cursor. Every transcript chunk produced while a Protocol stage is active carries the current `AttentionState` as a tag. Context assembly later queries by address: "all chunks where attention was on Field X in Record Y." The annotation is free because it was captured at production time.

`Address` is the addressing scheme that makes co-addressability possible. A transcript chunk and a Field Revision are in the same address space — they can reference each other because both have resolvable addresses.

**Multi-Container addressing**: A Record may belong to more than one Container simultaneously (a task may exist in both a project Container and a sprint Container). That Record therefore has multiple valid document-space Addresses — one per Container context. This is intentional: `containerId` in a document-space `Address` is not a uniqueness constraint, it is a *context specifier*. `AttentionState.containerId` records which Container was active during a live session, making the contextual anchor explicit. When a session-tagged transcript chunk is later queried, the Container in the `AttentionState` tells you not just *what Record* was being discussed but *in which context* it was being discussed.

### 2.7 Why Revision is addressable

In v1, field revision was an implementation concern. The spec described when to edit in-place versus create a new Record, but individual revisions were not addressable — you could not ask "what did this field say before the last Protocol run?" at the interoperability layer.

This matters for:
- **Governance challenge**: if a Record is challenged, you need to trace which conversation produced each field value and which version was in place when a downstream decision was made.
- **Context assembly**: when generating the next draft, knowing what changed between revision 2 and revision 3 — and what conversation produced that change — is more useful than knowing only the current value.
- **Audit**: a complete audit trail requires addressable history, not just current state.

`Revision` is the addressable audit trail. It does not replace the edit-in-place vs. new-Record judgment for minor corrections. That remains an implementation concern. Revision is the interoperability layer for cases where history itself is a first-class concern.

### 2.8 Why `valueType` and `editorHint` are separate

A Field with `valueType: "text"` might be edited via textarea in a web form, captured via voice in a mobile app, or extracted directly from a transcript with no editing UI. The semantic type is stable; the editing surface is a rendering decision.

AI extraction logic, validation rules, and export formatting depend only on `valueType`. `editorHint` is a default that implementations and Views may override. Conflating the two would mean that changing the preferred editor for a field could inadvertently break AI extraction rules.

### 2.9 Why `displayLabel` must not affect extraction

`displayLabel` lets a View relabel a Field for a specific audience without altering the Field's meaning. "Strategic question" might be displayed as "The decision we're making" in a facilitated view aimed at non-specialist participants.

If `displayLabel` could affect extraction, two Views of the same Record could produce different extracted values for the same Field — because the AI was given different labels. Field semantics must be stable across views. The label controls what the human sees; the Field's `aiGuidance` controls what the AI does.

### 2.10 Why the directionality invariant matters

`sourceInstanceId` is the asserting instance; `targetInstanceId` is the related instance. "D-004 supersedes D-001" must always be represented as `source: D-004, target: D-001`.

Without this invariant, graph traversal breaks across system boundaries. If System A stores `supersedes` with the newer Record as source and System B stores it with the older Record as source, a federated query for "all Records that supersede D-001" returns different results from each system. The invariant is the minimum agreement required for semantic interoperability on Relation graphs.

The invariant does not assign agency or authority to the `source` slot — those are properties of the `relationType`. A `contains` Relation makes the source the container and the target the contained item. An `evidences` Relation makes the source the evidence and the target the claim it supports. Directionality is a slot convention; semantics come from the type.

### 2.11 Why Containers and Relations are complementary

A Relation graph answers "what is semantically connected to what?" but not "what should be exported or queried together?" These are different questions. A project may contain hundreds of Records connected by many Relations. The question "which Records are in scope for this export?" is a scoping question, not a semantic one.

Container provides the boundary. "These Records collectively form a unit for boundary purposes" is a scope claim. "Stage A contains Task B" is a semantic claim. A Container can hold Records that have no `contains` Relation between them — they are grouped for operational reasons, not because one is semantically inside the other.

Relationship-first implementations derive Container membership by traversing `contains` Relations from root instances. Container-first implementations use explicit `memberInstanceIds`. Both strategies are valid; neither replaces the other.

### 2.12 Why the conversation layer is a permanent boundary

SCDS captures negotiated semantic state. Transcripts capture raw material — speech, threads, annotations — from which semantic state is extracted or constructed. These are different things, and conflating them would harm both.

If SCDS tried to be a transcript standard, it would need to model speaker identity, timing, overlapping speech, and audio quality — none of which are semantic concerns. If the transcript standard tried to be a semantic state standard, it would need to version field definitions, track lineage, and manage inter-Record Relations — none of which are evidence concerns.

The boundary makes both layers better at what they do. The connection between them — `SourceReference` and `AttentionState` — is the bidirectional bridge. Each layer references the other; neither absorbs the other.

---

## 3. Usage Guidance

### 3.1 AI guidance composition order

When assembling an AI prompt from multiple `aiGuidance` blocks:

1. **Type framing** — establishes what semantic object type is being worked on
2. **View framing** (if using `ext:views-l1`) — workflow-specific context for this View
3. **Field extraction guidance** — specific instruction for populating each Field
4. **Negative guidance** — constraints applied after the extraction instruction
5. **Examples** — few-shot demonstrations last, as final grounding

This ordering ensures broad context (what kind of object this is) precedes narrow directives (how to populate this specific Field). Template framing narrows the Type context — it does not replace it.

This is a recommended default. Implementations that compose differently will produce different AI behaviour from the same definitions.

### 3.2 When to edit in-place vs create a new Record

The underlying question: *Would a reasonable reader, encountering this Record a year later, recognise it as the same understanding they would have read before the change?*

| Scenario | Guidance |
|---|---|
| Correcting how something is expressed (typo, phrasing) | Edit in-place |
| Adding context that reinforces the existing understanding | Edit in-place |
| Clarifying a detail that was ambiguous but understanding is unchanged | Edit in-place |
| Adding information that changes what was actually committed to | New Record + `refines` or `supersedes` |
| Reversing or materially replacing a prior commitment | New Record + `supersedes` |
| Producing a more detailed version from a rough original | New Record + `refines` |

Cross-check: if a `supersedes` Relation would feel misleading — as if the group reversed itself when it only clarified — it is probably an edit. If a silent edit would feel misleading — as if the record was silently revised after the fact — it is probably a new Record.

### 3.3 Choosing between repeatable fields, field groups, and separate Records

| Pattern | When to use | Example |
|---|---|---|
| Repeatable scalar (`ext:repeatable-fields`) | Multiple values of the same type, no pairing needed | Multiple assigned person names |
| Field Group (`ext:field-groups`) | Multiple structured entries that must be read together | Contacts with name + email |
| Separate Records + Relations | Repeated items need their own identity, lifecycle, or reuse | Tasks assigned to roles |

A Field Group entry does not have its own `instanceId`, lifecycle state, or Relation endpoints. If a group entry will ever need to be referenced independently, related to other Records, or reused across multiple Records, it should be a separate Record connected by a `contains` or `derived-from` Relation.

### 3.4 Graduation: when and how

Graduation is the act of replacing a lower-tier instance with a higher-tier equivalent as its structure stabilises.

**Identity continuity:**

| Scenario | `instanceId` | Relation |
|---|---|---|
| Pure formalisation (section names map directly to field names, content unchanged) | Keep | None required |
| Content interpreted or restructured during formalisation | New | `refines` from new to old |
| One Note splits into multiple Records | New IDs for all | `derived-from` from each new Record to the original |

**Graduation is not always one-to-one.** A single meeting Note may graduate into one Decision Record, three Task Records, and two Risk Records. Each resulting Record receives its own `instanceId` and links to the original via `derived-from`. The original Note is preserved as the semantic root of the derived graph.

Implementations may automate graduation suggestions by matching section or field names against `Field.name` values in available Type definitions.

### 3.5 Relation taxonomy usage

Use the canonical relation type strings from `ext:recommended-relations` for common relationships. Reserve custom `namespace/name` format for domain-specific relations.

**Composition example** (project planning):
```
Stage A  --contains-->  Task B
Task B   --contains-->  Subtask C
```

**Derivation example** (Protocol output):
```
Decision Record  --derived-from-->  Options Analysis Note
Options Analysis --derived-from-->  Brain Dump Note
```

**Governance example**:
```
Policy v2  --supersedes-->  Policy v1
Amendment  --amends-->      Policy v2
```

**Evidence example**:
```
Workshop photo  --evidences-->  Stage 1 completion claim
Transcript seg  --evidences-->  Decision rationale
```

Non-governance projects use the same Relation layer. `supersedes`, `delegates`, and `ratifies` apply when the semantic object type calls for them — they are one profile of the layer, not its primary purpose.

### 3.6 Protocol chaining and provenance traces

Loose Protocols produce open material. Tight Protocols converge on a specific Record. The output of one Protocol is the input context for the next.

Example chain for a governance decision:
```
Brain Dump Protocol → unstructured Notes
Decomposition Protocol → component Notes (derived-from Brain Dump Notes)
Options Analysis Protocol → Options Analysis Record (derived-from Decomposition Notes)
Decision Protocol → Decision Record (derived-from Options Analysis Record)
```

When a Decision Record is challenged, you can traverse back through the full chain: Decision ← Options Analysis ← Decomposition ← Brain Dump ← transcript chunks. The quality of the final Record is auditable because every stage of the process left addressable artefacts.

With `ext:addressability`, each stage's conversation chunks carry the `AttentionState` at the time they were produced. "What was being discussed when the options were evaluated?" is a queryable question.

### 3.7 Graceful degradation

In a federated ecosystem, implementations will often receive SCDS content that uses extensions they do not support. The useful default is: understand what you can, preserve what you cannot.

A conforming implementation should validate the core and extension content it recognises, surface unknown extension content clearly to users or downstream systems, and pass that unknown content through rather than silently discarding it. This is especially important for Records instantiated against a specializing Type: a system that knows only the base Type should still be able to read the inherited base fields correctly while preserving the specialization-specific fields.

---

## 4. Extension Design Notes

### 4.1 How to decide which extensions to implement

Start with the question: what does your implementation need to do?

| Need | Extensions |
|---|---|
| Define and exchange Field and Type definitions | Core only |
| Track definition origin and imports | `ext:import-tracking` |
| Publish a definition catalog | `ext:registry` |
| Governance with lifecycle states | `ext:lifecycle` |
| Present and export Records | `ext:views-l1` |
| Assemble multi-Record documents | `ext:views-l2` |
| Facilitate structured deliberation | `ext:protocol` |
| Live facilitation with context assembly | `ext:addressability` |
| Extraction from source material | `ext:schema` |
| Specialise Types while preserving base processability | `ext:type-inheritance` |
| Lists of values within a Record | `ext:repeatable-fields` |
| Structured repeatable context in a Record | `ext:field-groups` |
| Complex conditional validation | `ext:cross-field-validation` |
| Cross-system Relation interoperability | `ext:recommended-relations` |

### 4.2 Addressability as a prerequisite for live facilitation

`ext:addressability` is not just about naming things. It is the mechanism that makes the conversation layer useful. Without `AttentionState`, transcript chunks have no address-time connection to the Records they inform. Without `Revision`, the history of a field's value is an implementation detail not visible at the interoperability layer.

Any implementation that facilitates live sessions — where conversation material is produced while people are working on specific Records and Fields — should implement `ext:addressability`. Without it, context assembly is purely retrospective, and the quality of AI assistance degrades accordingly.

**Diff rendering:** implementations rendering Revision history for governance review should support a diff view that shows field-level removals alongside additions, not only the current value. The Revision chain already provides the data needed for three useful modes: final (current value only), all markup (current value plus prior content shown as removed and new content as added), and original (the value at a specified Revision). This is a rendering pattern, not a separate data shape.

### 4.3 Schema vs View — the extraction gap

A View answers: given a Record that already exists, how do I render it for a specific audience?

A Schema answers: given source material, what Records should I extract, and how do they relate?

These are complementary but distinct. A Document View cannot serve as an extraction schema because it assumes Records already exist. A Schema cannot serve as a Document View because it does not specify how to render field values for an audience.

An extraction pipeline uses Schema + Field `aiGuidance` + Protocol to produce Records. A rendering pipeline uses View + Document View to project those Records into readable form.

### 4.4 `semanticObjectType` as a federation risk

`semanticObjectType` on `Type` and in `SectionSource.type-query` is a free-form string. The spec recommends `namespace/name` format for portable Document Views (Invariant 32) and treats bare strings as a single-system convention. This is the minimum rule needed to ship v2.

The risk: two systems can use the same bare string (`"decision"`, `"task"`) and mean different semantic Types. When graph traversal or document assembly crosses system boundaries, type-query portability becomes undefined wherever bare strings appear. This is where federation bugs will appear first.

The current design is deliberately light. Possible futures in order of increasing strictness:
- **Informative only** — `semanticObjectType` becomes advisory metadata with no query semantics; implementations must use explicit TypeRefs for cross-system queries
- **Typed vocabulary** — `semanticObjectType` becomes a typed reference to a Type definition (a `TypeRef` rather than a bare string), giving it the same identity guarantees as a Field or Type reference

The second option would require changing the type from `string` to `TypeRef | string` and a version bump. For now: prefer `namespace/name` format in any Type or SectionSource that will cross system boundaries, and treat bare strings as a scope boundary. Implementations should document which `semanticObjectType` values they recognise and what Types they map to.

### 4.5 Protocol loose-to-tight spectrum

The spectrum from loose to tight is not a quality ranking — it is a fitness question. A Brain Dump Protocol is the right tool when the problem space is not yet understood. A Decision Protocol is the right tool when the group is ready to converge. Starting with a tight Protocol before the problem is decomposed produces poor output because the epistemic prerequisites are not met.

The `dependsOn` field on `ProtocolStage` makes this explicit. A stage that depends on decomposition results cannot run before those results exist. This is not just sequencing — it is a statement about what understanding is required before the next stage is meaningful.

### 4.6 Why Type inheritance is conservative

`ext:type-inheritance` adds one formal mechanism: a Type may specialize one base Type and still be processable as that base Type. This solves the common case where a domain-specific Type needs to add fields to a shared Type without duplicating the whole definition.

The extension is intentionally narrow. It supports inherited fields, added fields, explicit ordering, and presentation/workflow overrides for inherited fields. It does not let a specializing Type change Field semantics or relax base requirements. That keeps the central promise intact: a system that understands the base Type can still process the base portion of a specialized Record.

`Type.fieldOrder` and `ExportConfig.fieldOrder` share a name but operate at different layers. `Type.fieldOrder` is a Type-level ordering declaration over the full effective field list, including inherited fields. `ExportConfig.fieldOrder` is a View export setting that controls rendered output order for a particular presentation. Validators should apply the `fieldAssignmentOverrides` inherited-field restriction only to `fieldAssignmentOverrides`, not to `Type.fieldOrder`.

---

## 5. Future Extensions

The following capabilities are planned but out of scope for this version.

### Session

A live collaborative process model with real-time facilitation, AI assistance, and collaborative editing. A Session produces or enriches Records but does not own them. Session-level Protocol management (tracking active stage, managing participant attention) is a natural successor to `ext:protocol` and `ext:addressability`. Deferred pending implementation experience.

### Full projection surface

Document-level projection is addressed by `ext:views-l2`. The broader projection surface — dashboards, timelines, AI context packages, real-time views, and composite renderings that are not document-shaped — remains a future concern. Projections are read-only views; they do not modify Record state.

### Revision history exchange format

A standard format for exchanging full Revision history between implementations, for cases where the history itself is a first-class interoperability concern. Natural extension of `ext:addressability`. Deferred pending stabilisation of the Container and Relation layers.

### Graduation mapping record

A structured artefact recording how a Note or Typed Record was mapped to its Record successors — which section or field names were matched, merged, split, or interpreted. Useful for AI-assisted graduation review and audit. Deferred pending implementation experience.

### Field domains

Named sets of Fields that travel together may become useful as Type libraries grow. For v2, ordinary shared base Types plus `ext:type-inheritance` cover the immediate reuse need with less machinery. Field domains are deferred until there is stronger evidence that reusable field sets need their own identity, versioning, and package dependency rules independent of Types.

### View inheritance and composition

As View libraries mature, inheritance will become necessary. A lightweight ADR View and a governance ADR View share base configuration — field selection, ordering, `editorHint` overrides — while diverging on workflow framing and export layout.

A future version may define:
- `extendsViewId?: UUID` — single inheritance; child View inherits all `fieldViews` from parent and overrides selectively
- `composesViews?: UUID[]` — mixin composition; multiple Views contribute non-overlapping configuration

Current design: `View` is a leaf type. Use Lineage tracking to record inheritance relationships.

### Instance graph exchange format

A standard envelope for exchanging a Container together with its full Record set, Relations, and source references. Natural successor to `Package` at the instance layer. Likely shape: `{ container, instances[], relations[], sourceRefs[] }`. Deferred pending stabilisation of `ext:views-l2` and implementation experience.

### Field transclusion in Document Views

Pulling a specific Field value inline into a Document View is useful, but a syntax such as `{{field:{recordId}/{fieldId}}}` makes a reusable Document View depend on concrete instance IDs. That weakens portability and should wait for an addressing model that can express reusable selection rules rather than binding a definition to one Record.

### Conditional processing

Audience, platform, and output filtering may eventually allow one source Container to produce different projections for different readers. This is deferred because SectionSource queries already cover common projection differences, while a general condition evaluation model would add substantial complexity.

### Sub-field addressing

Web UI comments and annotations attached to specific text within a Field value require addressing below the Field level. `ext:addressability` currently addresses at Field granularity. Sub-field text selection addressing is architecturally possible (the Address space accommodates it) but is deferred as a separate extension.

---

## 6. μDemocracy Mapping

How the SCDS v2 vocabulary maps to the μDemocracy application layer. Reproduced from the v1→v2 conceptual remapping document for reference.

| SCDS concept | μDemocracy application |
|---|---|
| Field | Semantic atom in a governance record |
| Type | Decision, Proposal, Action, Role, Value, Principle, ... |
| Record | A captured governance artefact with provenance |
| Schema | Founding Document type; Decision Log type |
| Protocol | Democracy protocol: Brain Dump, Decomposition, Decision, Proposal, ... |
| Container | A group's governance workspace; a founding process scope |
| Relation | `supersedes`, `derived-from`, `ratifies`, `depends-on`, ... |
| View | Facilitator view; summary view; export for ratification |
| Document View | Assembled founding document; full decision log |
| Address | Stable identifier for any governance element — Field, Record, stage, chunk |
| Attention State | Current focus of an active facilitated session |
| Revision | Auditable history of how a governance field arrived at its current value |
| Conversation layer | Session transcript; threaded discussion; facilitator annotations |
