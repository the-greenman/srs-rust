> From: https://claude.ai/chat/895d7f5e-f847-4ad6-9e52-e06a4ec04ab9

# you asked

message time: 2026-05-21 08:33:32

# SCDS Schema Specification

**Version**: 1.0-draft
**Status**: draft
**Scope**: field definitions, module definitions, template definitions, instances, distribution, import tracking, registry, relationships (provisional), containers (provisional), document projection (provisional)

---

## 1. Purpose and Scope

### What this specification defines

The Semantic Collaborative Document System (SCDS) schema layer defines an interoperable standard for semantic field and template definitions. It specifies the data shapes and invariants that allow field and module definitions to be created, shared, versioned, and imported across independent implementations.

This specification covers:

- **Field definitions** — atomic reusable semantic units
- **Module definitions** — named compositions of fields for a specific semantic object type
- **Template definitions** — versioned views, workflow configurations, and export layouts over module instances
- **Module instances** — the runtime shape of an instantiated module with field values
- **Freeform instances** — lightweight named-section content with no module binding; the entry point for progressive semantic adoption
- **Lifecycle** — optional state and transition vocabulary attached to module definitions
- **Distribution** — how definitions are packaged and distributed between repositories
- **Import tracking** — how a consumer tracks the origin, update state, and divergence of imported definitions
- **Registry** — a catalog format for discoverable definition packages
- **Relationships** — first-class semantic links between instances (Layer 7; schema provisional in this version)
- **Containers** — lightweight grouping boundaries for instance collections; the root scope for export, session, and graph traversal (Layer 8)

### What this specification is not

This specification does not define:

- **Session** — live collaborative process model. Defined in a future version.
- **Projection** — rendered output model (human documents, exports, AI context packages). Defined in a future version.
- **Registry protocol** — how registries communicate, authenticate, or federate. This specification defines the data shapes only.

### Non-goals

SCDS does not define a universal semantic ontology. The standard defines interoperability mechanics for semantic collaboration systems, not canonical domain vocabularies. Domain-specific vocabularies are the responsibility of namespace authors.

SCDS does not prescribe governance processes, authority structures, or decision-making rules. The standard supports structured deliberation without mandating how deliberation is conducted or what constitutes a valid decision.

SCDS does not require any particular AI capability. AI guidance fields describe intent; implementations may use them with any AI system or not at all.

SCDS relation types are not limited to governance or decision-making. Implementations may use relationships to model composition, project structure, evidence chains, asset references, dependencies, refinement history, and temporal sequence.

### Relationship to implementing systems

This specification is implementation-neutral. Multiple independent systems may implement the same standard. The specification does not constrain persistence technology, API design, UI rendering, or prompt assembly strategy. Implementations are expected to validate inputs against these schemas at their system boundaries.

---

## 2. Core Thesis

Traditional document systems treat documents as primarily text.

This specification treats documents as **socially negotiated semantic state**. Text is one projection of that state.

Six principles follow from this:

**1. Semantic state is primary; documents are projections.**
The same semantic state may be rendered as a board paper, a governance record, a dashboard, or an AI context package. None of these projections is the source of truth.

**2. Fields are reusable semantic atoms.**
A field definition describes a reusable slot of meaning with stable identity. It is not a form field. It is not tied to any specific template or document. Its AI guidance, validation rules, and value type belong to the field, not to the module that uses it.

**3. Modules are compositions, not owners of field semantics.**
A module definition selects and orders fields for a specific semantic object type. It may provide session-level AI framing. It must not override or redefine the meaning of any field it includes.

**4. Lineage and provenance are first-class.**
Definitions evolve. Forks happen. Upstream changes must be traceable. A definition without lineage is a definition that cannot be trusted to evolve cleanly.

**5. Instances represent negotiated semantic state, not objective truth claims.**
A module instance records what a group understood, agreed, or committed to at a point in time. That understanding may be partial, contested, or later revised. The system preserves revision history and provenance precisely because the original state is worth keeping alongside its successors. Human prose and ambiguity are preserved, not collapsed.

**6. Understanding is mutable; historical semantic state has permanent value.**
SCDS assumes that understanding evolves. Instances, relationships, and lifecycle states may be revised, superseded, refined, or contradicted without invalidating prior semantic state. A rough plan is a valid semantic object. A superseded decision is a valid semantic object. An abandoned hypothesis is a valid semantic object. Historical semantic state is not noise to be discarded — it is provenance, institutional memory, and the record of how understanding arrived at its current form.

---

## 3. Namespace Format

### Convention

Namespaces are dot-separated identifiers using lowercase alphanumeric characters and hyphens.

```
<component>[.<component>]*

component = [a-z0-9][a-z0-9-]*
```

Examples:
```
core
community.adr
com.acme.hr
org.cooperative-name
```

### Reserved namespaces

`core` is reserved for definitions maintained by the SCDS standard itself. Implementations must not allow user-created definitions in the `core` namespace.

### Reference format

A specific version of a definition is referenced using the canonical form:

```
namespace/name@version
```

Examples:
```
core/decision_statement@2
community.adr/review_rationale@1
com.acme.hr/headcount_impact@3
```

The `/` and `@` characters are reserved separators in this format. They must not appear within a namespace component or a name.

### Name convention

Field and module names are programmatic keys in `snake_case`. Names are stable within a namespace and version lineage. A new name means a new definition, not a renamed version of an existing one.

---

## 4. Schema Definitions

### Notation

Types are described using TypeScript-style notation. Optional fields are marked with `?`. Constraints are noted inline. All `UUID` values are RFC 4122 UUID strings. All `ISO8601` values are datetime strings with timezone offset.

### Version semantics

Version numbers are positive integers scoped to a definition's UUID lineage. The following guidance applies to when a version bump is required or optional:

| Change | Version action |
|---|---|
| Documentation, typo, formatting only | Optional bump |
| `description`, `instructions`, or `aiGuidance.purpose` reworded without semantic change | Minor bump recommended |
| `aiGuidance.extraction` or `aiGuidance.purpose` changed in meaning | Version bump required |
| `valueType`, `selectOptions`, or `validationRules` changed | Version bump required |
| `name` changed | New definition required (new UUID) |
| `namespace` changed | New definition required (new UUID) |

When in doubt: if a downstream consumer's AI extraction, validation, or governance logic would behave differently, a version bump is required.

---

### 4.1 Layer 1 — Field Content

#### `ValidationRule`

A constraint applied to a field value.

```typescript
{
  type: "required" | "minLength" | "maxLength" | "pattern" | "enum"
  value?: string | number | string[]  // required for minLength, maxLength, pattern, enum
  message?: string                    // human-readable error message
}
```

#### `AiGuidanceExample`

A single example for AI guidance. Used for few-shot prompting.

```typescript
{
  description?: string  // labels this example, e.g. "Implicit deferral decision"
  input?: string        // sample source text; omit for output-only examples
  output: string        // the ideal value the AI should produce for this field
}
```

`output` is required. `input` is optional — an example without input demonstrates the expected form of the output without requiring a specific source. An example with both `input` and `output` is a proper input→output pair for few-shot prompting.

#### `AiGuidance`

The structured AI guidance block for a field or module. Replaces a flat extraction prompt string.

```typescript
{
  purpose: string            // what this field/module captures (1-2 sentences)
  extraction?: string        // direct LLM instruction for how to extract or populate this field
  negativeGuidance?: string  // what the LLM must NOT include or do
  examples?: AiGuidanceExample[]
}
```

The minimum valid `AiGuidance` is `{ purpose: "..." }`. This allows fields that are always human-authored to carry a machine-readable intent statement without requiring a dummy extraction prompt.

**Why the split between `extraction` and `negativeGuidance`:**
Mixed positive/negative instructions in a single prompt string cause models to weight negative instructions unpredictably. Keeping them separate allows implementations to compose prompts deliberately and update negative guidance independently of the extraction instruction.

**Why `purpose` is separate from `extraction`:**
`purpose` is a human-readable statement of semantic intent. `extraction` is the LLM directive. They serve different audiences: `purpose` helps a human understand what a field is for; `extraction` tells the AI how to populate it. Conflating them produces prompts that are neither good human documentation nor good AI instructions.

---

### 4.2 Layer 2 — Definitions

#### `FieldDefinition`

The atomic reusable semantic unit.

```typescript
{
  // Stable identity
  id: UUID                   // stable across all versions; never reused
  namespace: string
  name: string               // snake_case programmatic key
  version: integer           // min: 1; increments within this id's lineage

  // Semantic content
  description: string        // one-sentence user-facing summary of what this field captures
  instructions?: string      // fuller guidance on how a human should complete this field
  aiGuidance: AiGuidance

  // Value semantics — stable across renderers
  valueType: "string" | "text" | "number" | "boolean" | "date" | "url" | "select" | "multiselect"

  selectOptions?: string[]   // required when valueType is "select" or "multiselect"
  validationRules?: ValidationRule[]

  // Editor hint — projection-specific default (implementations and templates may override)
  editorHint?: "singleline" | "textarea" | "rich-text" | "date-picker" | "dropdown" | "multi-select" | "voice"

  // Classification
  tags?: string[]            // free-form subject tags; no domain-specific enum

  // Metadata
  createdAt: ISO8601
  lineage?: DefinitionLineage
  provenance?: DefinitionProvenance
}
```

**`valueType` vs `editorHint` — why they are separate:**

| Concern | Field | Notes |
|---|---|---|
| Semantic data type | `valueType` | Stable. Used by AI guidance, validation, and export logic. |
| Preferred editing surface | `editorHint` | A default hint. Projections and templates may override. |

A field with `valueType: "text"` might be edited via textarea in a web form, captured via voice in a mobile app, or extracted directly from a transcript without any editing UI at all. The semantic type is stable; the editor is a rendering decision. AI extraction logic, validation rules, and export formatting should depend only on `valueType`.

**`valueType` semantics:**

| Value | Meaning |
|---|---|
| `"string"` | Short single-value text (typically one line) |
| `"text"` | Potentially long multi-paragraph prose |
| `"number"` | Numeric value |
| `"boolean"` | True/false |
| `"date"` | ISO 8601 date or datetime |
| `"url"` | A URL string |
| `"select"` | One value from `selectOptions` |
| `"multiselect"` | One or more values from `selectOptions` |

#### `LifecycleStateDefinition`

A named state in a module's lifecycle.

```typescript
{
  name: string          // e.g. "exercise", "proposal", "decision", "superseded"
  description?: string
  isInitial?: boolean   // this state is a valid starting state for new instances
  isFinal?: boolean     // no transitions out of this state; instance is settled
}
```

#### `LifecycleTransitionDefinition`

A named transition between lifecycle states.

```typescript
{
  name: string          // e.g. "promote", "approve", "supersede"
  from: string          // must match a state name in the enclosing lifecycle
  to: string            // must match a state name in the enclosing lifecycle
  description?: string
}
```

#### `ModuleFieldAssignment`

A field reference within a module definition. Configures presentation without redefining field semantics.

```typescript
{
  fieldId: UUID         // references FieldDefinition.id
  order: integer        // min: 0; display and processing order within the module
  required?: boolean    // default: true

  // Repeatability — module-scoped; the same field definition may be single in one module
  // and repeatable in another. Repeatability is a structural property of the assignment,
  // not of the field itself.
  repeatable?: boolean  // default: false; when true, multiple values are allowed
  minItems?: integer    // minimum entry count when repeatable; meaningful only when repeatable === true
  maxItems?: integer    // maximum entry count when repeatable; meaningful only when repeatable === true

  // Presentation-only — must NOT affect AI guidance, extraction, valueType, or validation
  displayLabel?: string // override display label for this assignment context only
  displayHint?: string  // additional short rendering context
}
```

`displayLabel` and `displayHint` are strictly for rendering. They are not visible to AI extraction logic. If a materially different label or meaning is needed, that requires a different `FieldDefinition` with its own lineage, not a presentation override.

#### `ModuleFieldGroup`

A named, ordered group of fields that repeat together as a unit within a module instance. Use when multiple fields are semantically paired — for example, a contact record with `name` + `email`. Preserves internal pairing across repeated items, avoiding the index-alignment problem of parallel `multiselect` arrays.

```typescript
{
  groupId: string              // stable key within the module, e.g. "assigned_contacts"
  label?: string               // e.g. "Assigned contacts"
  description?: string         // when to use this group; what each entry represents

  order: integer               // min: 0; position relative to other fields and groups in the module

  required?: boolean           // default: false; at least one entry must be present if true
  repeatable?: boolean         // default: false; when true, multiple entries are allowed
  minItems?: integer           // minimum entry count; meaningful only when repeatable === true
  maxItems?: integer           // maximum entry count; meaningful only when repeatable === true

  fields: ModuleFieldAssignment[]
  // The fields that constitute one entry in this group.
  // These are the same ModuleFieldAssignment shape as module-level fields.
  // Presentation-only overrides (displayLabel, displayHint) apply within the group context.
}
```

**On the semantic boundary:**
A field group records repeated structured context embedded within a module instance — it is not a collection of semantic objects. If each repeated item needs its own identity, lifecycle, relationships, provenance, or reuse across module instances, use separate `ModuleInstance` records connected by `contains` or other relationships instead.

| Pattern | Use | Example |
|---|---|---|
| Repeatable scalar | `ModuleFieldAssignment.repeatable` | Multiple assigned person names |
| Repeatable structured context | `ModuleFieldGroup` | Contacts with name + email pairs |
| Repeated semantic objects | Separate instances + relationships | Tasks assigned to roles |

#### `ModuleValidationRule`

A constraint that spans multiple fields within a module. Field-local `ValidationRule` handles single-field constraints; `ModuleValidationRule` handles the cases that require evaluating more than one field together.

```typescript
{
  type: "conditional-required" | "field-ordering" | "mutual-exclusion"
  message?: string  // human-readable error shown when the rule is violated

  // conditional-required: targetFieldId becomes required when predicateFieldId equals predicateValue
  predicateFieldId?: UUID
  predicateValue?: string  // exact string match; complex expressions are out of scope
  targetFieldId?: UUID

  // field-ordering: targetFieldId value must precede or follow predicateFieldId value
  // Applies only to fields with valueType "date" or "number".
  effect?: "must-precede" | "must-follow"

  // mutual-exclusion: at most one of the listed fields may have a non-empty value
  fieldIds?: UUID[]        // min: 2; for mutual-exclusion only
}
```

| Rule type | Use case | Required fields |
|---|---|---|
| `conditional-required` | `end_date` required only if `status === "Approved"` | `predicateFieldId`, `predicateValue`, `targetFieldId` |
| `field-ordering` | `end_date` must follow `start_date` | `predicateFieldId`, `targetFieldId`, `effect` |
| `mutual-exclusion` | Only one of `assigned_team` or `assigned_person` may be set | `fieldIds` |

This covers the most common cross-field constraints without introducing a general expression language. Complex conditional logic that cannot be expressed with these types belongs in implementation-layer business rules, not module definitions.

#### `ModuleDefinition`

A named, versioned composition of fields for a specific semantic object type. Modules are the composable units of a document or workspace — decisions, proposals, tasks, stages, risks, roles, budget lines, requirements, assets, and any other structured semantic object.

```typescript
{
  // Stable identity
  id: UUID
  namespace: string
  name: string
  version: integer      // min: 1

  // Content
  description: string   // when to use this module; what semantic object type it defines
  aiGuidance?: AiGuidance  // module-level LLM framing injected at session scope

  // Semantic object type (optional)
  semanticObjectType?: string
  // e.g. "decision", "proposal", "exercise", "task", "stage", "risk", "budget_line",
  //      "requirement", "milestone", "asset", "person", "open_question", "design_option"
  // Free-form; the standard does not enumerate allowed values.
  // Implementations may optionally use this as a lightweight semantic hint for rendering,
  // workflow, grouping, or reasoning. No conforming implementation is required to act on it.

  // Composition
  fields: ModuleFieldAssignment[]
  fieldGroups?: ModuleFieldGroup[]               // repeatable structured field groups; see ModuleFieldGroup
  moduleValidationRules?: ModuleValidationRule[]  // cross-field constraints; see ModuleValidationRule

  // Lifecycle (optional)
  lifecycle?: {
    states: LifecycleStateDefinition[]          // min 1 state
    transitions: LifecycleTransitionDefinition[]
    initialState: string  // must reference a state name in states[] where isInitial === true
  }

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: DefinitionLineage
  provenance?: DefinitionProvenance
}
```

**On `aiGuidance` at module level:**
`purpose` describes what type of semantic object this module captures. `extraction` provides session-level framing injected into the AI context (equivalent to a system prompt prefix that sets the decision category or document type). Examples are not typically used at module level.

**On `semanticObjectType`:**
This is a lightweight, optional semantic hint. It allows implementations to apply type-aware rendering, grouping, or reasoning without inspecting the full module definition. The value is free-form and the standard does not define an allowed vocabulary. Implementations are not required to act on it, and the standard deliberately avoids assigning normative meaning to any particular value — doing so would impose ontology where the field is meant to remain open.

**On `lifecycle`:**
Lifecycle is optional. Its presence declares the states and transitions a module supports. Implementations decide how strictly to enforce transitions. The specification is descriptive, not prescriptive. See Section 6 for lifecycle guidance.

**On instance migration when a module version changes:**
A `ModuleInstance` binds to a specific `moduleDefinitionVersion` at creation time. Instances created under version 2 remain valid version-2 instances after version 3 is published — they do not automatically migrate. If version 3 adds a required field, existing instances may lack that field; conformance is measured against the version they were instantiated under, not the latest version.

Implementations may offer migration tooling that re-instantiates existing instances against a newer module version, but the specification does not mandate this. When an instance is migrated and exchanged, it should carry the version it now conforms to. The original version-2 instance should be preserved and linked via a `supersedes` relationship if its authority is being replaced.

#### `TemplateFieldView`

A field reference within a template, configuring how a module field is presented in this view. Distinct from `ModuleFieldAssignment` — this is the template's projection of module fields, not the module's authoritative field list.

```typescript
{
  fieldId: UUID         // references FieldDefinition.id; must appear in the module's fields[]
  order: integer        // min: 0; display order within this template view
  required?: boolean    // template-level override; defaults to ModuleFieldAssignment.required
  visible?: boolean     // default: true; false to hide this field in this view

  // Presentation overrides — template scope only
  // Must NOT affect FieldDefinition AI guidance, extraction, valueType, or validation
  displayLabel?: string
  displayHint?: string
  editorHintOverride?: string  // overrides FieldDefinition.editorHint for this template view
}
```

`TemplateFieldView` controls presentation within this specific template and does not alter field semantics. A field hidden with `visible: false` remains in the module instance and may appear in other template views. A field marked `required: true` is a workflow-level constraint (this template's workflow requires this field); it does not alter the field's contract at the module or field definition level.

#### `TemplateFacilitationStep`

A named step in a guided facilitation sequence. Used when a template walks users or AI assistants through a structured process rather than presenting all fields simultaneously.

```typescript
{
  stepId: string        // stable key within this template
  order: integer        // min: 0; sequence position
  label: string         // human-readable step name, e.g. "Identify the options"
  description?: string  // guidance shown to the facilitator or participant for this step
  fieldIds: UUID[]      // subset of this template's fieldViews active in this step
  aiGuidance?: AiGuidance
  // purpose: what this step aims to achieve in the facilitation sequence
  // extraction: focused AI instruction for this step; supplements template-level aiGuidance
}
```

#### `TemplateExportConfig`

Configuration for rendering a module instance through this template as an exportable document.

```typescript
{
  format?: string            // target format hint, e.g. "markdown", "adoc", "json"
                             // Implementations may ignore or support additional formats.
  preamble?: string
  // A template string rendered before field values in the export output.
  // Variable substitution uses {{variable-name}} syntax.
  // Standard variables: {{instance-id}}, {{date}}, {{status}}, {{namespace}}, {{name}}
  // Implementations may support additional variables.
  // Example (ADR YAML frontmatter):
  //   "---\ndecision-id: {{instance-id}}\ndate: {{date}}\nstatus: {{status}}\n---"

  fieldOrder?: UUID[]        // explicit export field ordering; defaults to fieldViews[].order
  omitEmptyFields?: boolean  // default: false
}
```

#### Template levels

SCDS defines three levels of template, each operating at a different scope:

| Level | Type | Scope | Purpose |
|---|---|---|---|
| 1 — Module | `TemplateDefinition` | One module instance | Edit, facilitate, or export one semantic object |
| 2 — Document | `DocumentTemplate` | A container's instance collection | Assemble many instances into a coherent document |
| 3 — Session | *(future)* | Live collaborative process | Drive a facilitated session over a container's instances |

`TemplateDefinition` below is the Level 1 type. `DocumentTemplate` is defined in Layer 9. Session templates are a future extension (see Section 7).

The separation is intentional. Collapsing document composition into a large nested module definition — a "governance document" module with embedded fields — loses the semantic value of treating decisions, proposals, and articles as distinct, individually addressable, relatable objects. A document is a projection of a container's semantic graph, not a record type.

#### `TemplateDefinition` (Level 1 — Module Template)

A versioned view, workflow configuration, and optional export layout over a specific module definition. Templates provide audience- and purpose-specific presentations of module instances without altering their semantic content.

```typescript
{
  // Identity
  id: UUID
  namespace: string
  name: string           // snake_case programmatic key
  version: integer       // min: 1

  // Description
  description: string    // when to use this template; what workflow or audience it serves

  // Module binding — a template is always bound to a specific module version
  moduleDefinitionId: UUID
  moduleDefinitionVersion: integer

  // Workflow-level AI framing (optional)
  aiGuidance?: AiGuidance
  // purpose: describes the workflow context this template serves
  // extraction: session-level framing injected before field extraction;
  //             e.g. "You are facilitating a technology selection decision — emphasise
  //                   comparative analysis of alternatives against stated criteria."
  //             Complements module-level aiGuidance; does not replace it.

  // Field views
  // Only fields appearing in the referenced module's fields[] may appear here.
  // A template need not include all module fields — an omitted field is treated as visible: false.
  fieldViews: TemplateFieldView[]

  // Facilitation sequence (optional)
  facilitationSteps?: TemplateFacilitationStep[]

  // Export configuration (optional)
  exportConfig?: TemplateExportConfig

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: DefinitionLineage
  provenance?: DefinitionProvenance
}
```

**Why templates are a separate type from modules:**

| Concern | Module | Template |
|---|---|---|
| Owns semantic state | Yes — field definitions and their values | No |
| Owns field semantics | Yes — via `FieldDefinition.aiGuidance` | No |
| One per semantic object type | Yes | No — multiple templates may exist for one module |
| AI framing purpose | What kind of object this is | How to facilitate or export this workflow |
| Distributable independently | Yes | Yes — but always references a specific module version |

A module defines *what* a semantic object is. A template defines *how* to view, facilitate, or export that object for a specific purpose. The same module instance may be viewed through a full deliberation template, a summary template, and an ADR export template without any change to the instance data.

**On `aiGuidance` at template level:**
Template-level AI framing supplements module-level framing. When both are present, implementations should compose them — typically with module-level framing establishing what kind of object is being worked on, and template-level framing providing the workflow-specific context. Template framing should take precedence for session context when the two conflict.

**On the relationship between `TemplateFieldView` and `ModuleFieldAssignment`:**
`ModuleFieldAssignment` is authoritative — it determines which fields belong to a module and their baseline required status. `TemplateFieldView` is a view-layer projection — it can reorder, hide, and relabel fields for a specific audience, but cannot add fields that are not in the module, and any `required: true` override is a workflow constraint, not a semantic one.

**AI guidance composition — recommended assembly order:**
The spec defines AI guidance at three levels, each serving a different scope. When composing a prompt from multiple `aiGuidance` blocks, implementations should assemble context in the following order:

1. **Module framing** (`ModuleDefinition.aiGuidance.extraction`) — establishes what semantic object type is being worked on; sets the broadest context
2. **Template framing** (`TemplateDefinition.aiGuidance.extraction`) — adds workflow-specific context for this particular facilitation scenario; narrows the session context
3. **Field extraction guidance** (`FieldDefinition.aiGuidance.extraction`) — specific instruction for populating each individual field
4. **Negative guidance** (`FieldDefinition.aiGuidance.negativeGuidance`) — constraints applied after the extraction instruction
5. **Examples** (`FieldDefinition.aiGuidance.examples`) — few-shot demonstrations presented last, as final grounding

This ordering ensures that broad context (what kind of object is this) is established before narrow directives (how to populate this specific field). Template framing should be applied after module framing, not instead of it.

This is a recommended default, not a required invariant. Implementations may deviate for architectural reasons. The ordering recommendation exists because implementations that compose these levels differently will produce different AI behaviour from the same definitions — a form of behavioural divergence that structural validation alone cannot detect or prevent. Documenting a recommended order creates a convergence point without mandating it.

---

### 4.3 Layer 3 — Instances

These types define the runtime shape of an instantiated module — what a filled-in module looks like when an implementation creates or exchanges actual content. Without this layer the specification only defines schemas, not documents.

#### Semantic maturity tiers

SCDS is a progressively adoptable standard. Content does not need to be fully formalised from the start. Three tiers of semantic maturity are supported:

| Tier | Type | Structure | Semantics | Shareable |
|---|---|---|---|---|
| **0 — Freeform** | `FreeformInstance` | Named sections + free text | None | No |
| **1 — Typed** | `TypedInstance` | Named fields with types and values | Minimal | Partially |
| **2 — Defined** | `ModuleInstance` | Fields referencing `FieldDefinition` | Full | Yes |

The graduation path runs in one direction: freeform → typed → defined. Implementations are not required to support all three tiers; they may start at Tier 2. The tiers are defined to allow interoperability at the level of maturity a system has reached.

#### `FreeformSection`

A named text section within a freeform instance. The only structure required is a name and content — no UUID, no field type, no AI guidance.

```typescript
{
  name: string          // section key; unique within the instance; snake_case recommended
  label?: string        // human-readable display label, e.g. "What we decided"
  content: string       // free text; implementations may support markdown, plain, or rich text
  contentHint?: "text" | "markdown" | "plain"  // default: "text"; hint only
}
```

#### `FreeformInstance`

A lightweight content instance with no binding to a `ModuleDefinition` or `FieldDefinition`. Used when structure has not yet stabilised or semantic formalisation has not yet been warranted.

```typescript
{
  instanceId: UUID      // stable; carries forward if the instance is later graduated in place

  title?: string

  sections: FreeformSection[]

  graduatedAt?: ISO8601
  // When set, signals that this instance has been fully formalised into one or more
  // higher-tier instances. The original instance is preserved for provenance and graph
  // traversal. Implementations may exclude graduated instances from active queries.
  // The authoritative record of what this instance graduated into is the set of
  // derived-from relationships originating from the successor instances.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
  // Implementations may use meta to track graduation intent (e.g. target module)
  // before graduation is complete and derived-from links exist.
}
```

`FreeformInstance` carries no semantic guarantees. Section names are identifiers within this instance only — they are not stable, carry no lineage, and may be renamed freely.

#### `TypedField`

A field within a typed instance. Carries a name, an optional value type, and a value. No UUID, no AI guidance, no module binding.

```typescript
{
  name: string          // field key; unique within the instance; snake_case recommended
  label?: string        // human-readable display label
  valueType?: "string" | "text" | "number" | "boolean" | "date" | "url" | "select" | "multiselect"
  // When omitted, implementations may treat the field as "text".
  selectOptions?: string[]  // required when valueType is "select" or "multiselect"
  value: string | number | boolean | string[] | null
  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
}
```

#### `TypedInstance`

A structured instance with named, typed fields but no binding to a `ModuleDefinition` or `FieldDefinition`. Used when field names and types have stabilised but formal semantic definitions have not yet been created or adopted.

```typescript
{
  instanceId: UUID

  title?: string

  instanceType?: string
  // Lightweight semantic hint — analogous to ModuleDefinition.semanticObjectType.
  // Not a formal type declaration. Use namespace/name format for cross-system identification.

  fields: TypedField[]

  graduatedAt?: ISO8601
  // Same semantics as FreeformInstance.graduatedAt — signals full formalisation.
  // Authoritative record of successors is in derived-from relationships.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

`TypedField.name` values are the identity mechanism within a `TypedInstance`. They should be stable once the instance is being shared, but they are not versioned definitions — renaming a field does not create a lineage event.

**On naming collisions during automatic Tier 0 → Tier 1 transition:**
When parsing a `FreeformInstance` into a `TypedInstance` automatically — for example, via AI extraction from unstructured text — the same conceptual section may appear multiple times (e.g. three separate risk descriptions). Since `TypedField.name` values must be unique within an instance (Invariant 19), implementations must resolve collisions before producing a valid `TypedInstance`. Recommended strategy: append a numeric suffix to produce `risk`, `risk_2`, `risk_3`. Implementations should document their deduplication strategy, because the resulting names must match `FieldDefinition.name` values if the instance will later graduate to `ModuleInstance`. A collision-resolved name like `risk_2` will not automatically match a field definition named `risk` — the graduation mapping step must account for this.

#### On graduation between tiers

Graduation is the act of replacing a lower-tier instance with a higher-tier equivalent as its structure stabilises.

**Identity continuity during graduation:**

| Scenario | `instanceId` | Relationship |
|---|---|---|
| Same content, purely formalised (structural upgrade only) | Keep | None required |
| Content interpreted, restructured, or substantially changed during formalisation | New | `refines` from new to old |
| One instance split into multiple instances | New IDs for all | `derived-from` from each new instance to the original |

Pure formalisation — where section names map directly to field names and content is unchanged — is an in-place upgrade. The same `instanceId` continues to identify the same understood content, now expressed in a more formal shape. Implementations should preserve the original instance in their history even when graduating in place.

**Graduation is not always one-to-one.** A single `FreeformInstance` — a meeting note, a brainstorm, a rough plan — will often graduate into multiple `ModuleInstance` records. For example:

```text
one meeting note
→ one decision (ModuleInstance)
→ three tasks (ModuleInstance × 3)
→ two risks (ModuleInstance × 2)
```

In this case each resulting instance receives its own `instanceId` and should be linked to the original using `derived-from` relations. The original `FreeformInstance` is preserved and remains the semantic root of the derived graph.

**On graduated instance visibility:**
Graduated Tier 0/1 instances remain valid SCDS instances. They are not deleted. Setting `graduatedAt` signals to implementations that formalisation is complete. What to do with a graduated instance is an implementation decision:

- **Archive** — retain in history, exclude from default active queries; surfaced only when traversing the graph or searching historical state
- **Tombstone** — mark as inactive with a visible indicator; may appear in search results but clearly labelled as superseded
- **Hidden** — remove from user-facing queries entirely; retained for graph traversal and provenance only

The spec does not mandate a policy. The `derived-from` relationships and the `graduatedAt` timestamp together give implementations enough signal to make the right choice for their query model. What the spec does require: the instance must be preserved for graph traversal. Deleting a graduated instance breaks the provenance chain for any `derived-from` relationships that point to it.

Implementations may automate graduation suggestions by matching section or field names against `FieldDefinition.name` values in available module definitions.

#### `SourceReference`

A pointer from a field value (or instance) back to the external source material from which it was extracted or derived. Keeps SCDS independent of any specific transcript or evidence standard while enabling first-class provenance tracing.

```typescript
{
  sourceType: "transcript-chunk" | "transcript-segment" | "external-document"
  // "transcript-chunk"   — a chunk produced by a chunking pipeline; addressed by chunk ID
  // "transcript-segment" — a canonical, de-overlapped reading row visible to a human reviewer
  // "external-document"  — a document, file, or external resource outside the transcript layer

  sourceId: string
  // The stable identifier for this source within its system.
  // For transcript-chunk/segment: the chunk or reading-row UUID.
  // For external-document: a URL, DOI, file hash, or system-specific ID.

  sourceStandard?: string
  // The versioned standard or format the source conforms to, e.g.:
  // "canonical-transcript@1.0", "webvtt@1.0", "w3c-web-annotation@1.0"
  // Allows consumers to interpret sourceId correctly without coupling to a specific backend.

  streamId?: UUID
  // For transcript sources: the stream from which this chunk or segment originates.
  // Meaningful when a meeting has multiple concurrent streams (speech, chat, upload).

  relationType?: "evidence" | "derived-from" | "quoted-from" | "inspired-by" | "supersedes-context"
  // The relationship between the source and the value it supports.
  // "evidence"           — source material the value is based on; supports the claim
  // "derived-from"       — value was generated or synthesised from this source
  // "quoted-from"        — value directly quotes or reproduces content from the source
  // "inspired-by"        — source informed the value without being directly used
  // "supersedes-context" — this source replaces an earlier source for this value
  // When omitted, implementations may treat the link as generic evidence.

  confidence?: number     // 0.0–1.0; how confident the extractor was that this source supports the value
  note?: string           // optional human or AI annotation on the provenance link
}
```

**Focus links vs source references:**
These are distinct and must not be conflated:

| Type | When it is created | What it means |
|---|---|---|
| Focus link | Live, during a session — "this transcript happened while the group was focused on Article 5" | Contextual and attention-based; captured by session/connection state |
| Source reference (`SourceReference`) | During semantic refinement — "this wording in Article 5 was derived from chunks 120–140" | Editorial and provenance-based; recorded at extraction or editorial review time |

Focus links are a session-layer concern — they are captured from attention state (`activeMeetingId`, `activeDecisionId`, `activeField`) and stored in the transcript system as context tags. Source references are a SCDS concern — they are attached to field values, instances, or relations as permanent provenance assertions. The same chunk may appear in both: as a context-tagged segment (focus link) and as a `sourceRef` on the field value that was extracted from it. Both are correct; they answer different questions.

**On the transcript/SCDS boundary:**
Transcript chunks and reading segments are *source material* — addressable evidence. They do not become SCDS semantic instances automatically. A transcript chunk referenced in `sourceRefs` is evidence supporting a field value; it is not itself a `FreeformInstance` or `ModuleInstance` unless someone deliberately models it as one. This distinction preserves the layer separation:

```
Canonical Transcript Standard  →  captures evidence, streams, timing, provenance
SCDS                           →  captures negotiated meaning, structured records, relationships
AI context assembly            →  binds them when generating or reviewing semantic state
```

The `contentType` discriminator in transcript standards (`speech` vs `message`) is significant here. `speech` segments are duration-based and stream-relative; `message` events are point-in-time and wall-clock-originated. AI context assembly should not treat them identically — a chat message timestamp and a spoken utterance timestamp have different alignment semantics. `sourceStandard` is the mechanism that lets a consumer know which interpretation to apply.

#### `FieldValueEntry`

A single entry in a repeatable field. Used when `ModuleFieldAssignment.repeatable === true` and per-entry provenance is needed.

```typescript
{
  value: string | number | boolean | string[] | null
  // Type should conform to FieldDefinition.valueType.
  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
}
```

#### `FieldValue`

The current value of a field within a module instance. Supports both non-repeatable (single `value`) and repeatable (`entries`) fields.

```typescript
{
  fieldId: UUID                               // references FieldDefinition.id

  // Non-repeatable field — use value
  value?: string | number | boolean | string[] | null
  // Type of value should conform to FieldDefinition.valueType.
  // The specification does not prescribe internal storage — only exchange boundaries.

  // Repeatable field — use entries
  entries?: FieldValueEntry[]
  // One entry per repeated value. Use when ModuleFieldAssignment.repeatable === true.
  // entries respect minItems and maxItems from the assignment.

  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
  // For non-repeatable fields: source and editedAt describe the single value.
  // For repeatable fields: per-entry source and editedAt on FieldValueEntry take precedence;
  // source and editedAt here describe the field record as a whole (e.g. last AI sweep).

  sourceRefs?: SourceReference[]
  // Links from this value back to the source material from which it was extracted or derived.
  // Essential for explainability, governance challenge, and audit of AI-assisted extraction.
  // See SourceReference for the bridge between SCDS and transcript/document evidence systems.
}
```

#### `FieldGroupEntry`

One entry in a repeatable field group. Carries a set of field values that must be read together as a unit.

```typescript
{
  entryId?: UUID       // optional stable key for this entry; allows referencing or updating
                       // a specific entry without replacing the whole group
  fieldValues: FieldValue[]
  // One FieldValue per field in the group. Must cover at least the required fields
  // declared in ModuleFieldGroup.fields[].
}
```

#### `FieldGroupValue`

The current value of a field group within a module instance — a list of structured entries corresponding to one `ModuleFieldGroup`.

```typescript
{
  groupId: string           // references ModuleFieldGroup.groupId in the module definition
  entries: FieldGroupEntry[]
  // One entry per repeated item. Must satisfy ModuleFieldGroup.minItems and maxItems
  // when specified.
}
```

#### `ModuleInstance`

An instantiated module with field values.

```typescript
{
  instanceId: UUID
  moduleDefinitionId: UUID      // references ModuleDefinition.id
  moduleDefinitionVersion: integer
  moduleNamespace: string
  moduleName: string

  // Current lifecycle state (required if module has a lifecycle)
  lifecycleState?: string       // must match a state name in ModuleDefinition.lifecycle.states[]

  // Field values — flat fields
  fieldValues: FieldValue[]

  // Field group values — repeated structured groups (see ModuleDefinition.fieldGroups)
  groupValues?: FieldGroupValue[]

  // Instance-level source provenance (optional)
  sourceRefs?: SourceReference[]
  // Links from this instance as a whole back to its source material.
  // Use when provenance applies to the entire instance rather than individual fields —
  // e.g. "this decision instance was extracted from these three transcript chunks."
  // Field-level sourceRefs on FieldValue are preferred when provenance differs per field.

  // Timestamps
  createdAt?: ISO8601
  updatedAt?: ISO8601

  // Implementation-specific metadata
  meta?: Record<string, unknown>
  // Use meta for: lock state, visibility state, session references, participant records,
  // and any other implementation-local concerns that should not pollute the core shape.
}
```

**Why `meta` is permitted:**
Implementations have legitimate needs for implementation-local metadata — lock state, field visibility, session associations, participant snapshots. Rather than polluting the core instance shape with implementation-specific fields, `meta` provides a governed escape hatch. Implementations should document what they put in `meta` but are not required to interoperate on its contents.

**On instance revision and Principle 6 (mutable understanding):**
`ModuleInstance` defines the *exchange shape* for current semantic state. It does not prescribe internal storage. Principle 6 — "historical semantic state has permanent value" — applies at the exchange layer as follows:

- **In-place edits** (`updatedAt` advances, `fieldValues` mutate): appropriate for minor corrections, typos, and clarifications that do not alter the semantic meaning of the instance. The previous content is an implementation concern; the spec does not define a revision log format.
- **Semantic updates** (a group's understanding changed): these should produce a new instance linked to the prior by a `supersedes` or `refines` relationship. The prior instance remains valid and reachable via the relationship graph. This is what makes historical semantic state permanent at the interoperability layer.
- **Immutable instance + relationship graph**: an implementation may choose to treat all instances as append-only, producing a new instance for every change. This is a valid implementation strategy and naturally satisfies Principle 6.

The boundary is intentional. Implementations using event sourcing, collaborative editing, or CRDTs manage their own revision model internally and expose current state via `ModuleInstance` at exchange boundaries. A future version may define a standard format for exchanging revision history when that is itself a first-class interoperability concern.

**On semantic identity continuity:**
`instanceId` is stable across in-place edits, but the spec does not mechanically define when a change requires a new instance versus a mutation of the existing one. This is a semantic judgment. The underlying question is: *Would a reasonable reader, encountering this instance a year later, recognise it as the same understanding they would have read before the change?*

Practical guidance:

| Scenario | Guidance |
|---|---|
| Correcting how something is expressed (typo, phrasing) | Edit in-place |
| Adding context that reinforces the existing understanding | Edit in-place |
| Clarifying a detail that was ambiguous but the underlying understanding is unchanged | Edit in-place |
| Adding information that changes what was actually committed to or understood | New instance + `refines` or `supersedes` |
| Reversing or materially replacing a prior commitment | New instance + `supersedes` |
| Producing a more detailed version from a rough original | New instance + `refines` |

A useful cross-check: if a `supersedes` relationship would feel misleading — as if the group reversed itself when it only clarified — it is probably an edit. If a silent edit would feel misleading — as if the record was silently revised after the fact — it is probably a new instance.

Implementations should document their identity continuity policy and apply it consistently. Implementations that want stricter guarantees may treat all instances as append-only and rely entirely on the relationship graph for continuity.

---

### 4.4 Layer 4 — Distribution

#### `DefinitionLineage`

Upstream and fork tracking for a specific definition version. Applied to both `FieldDefinition` and `ModuleDefinition`.

```typescript
{
  sourceDefinitionId?: UUID     // UUID of the upstream definition in its originating registry
  sourceVersion?: integer       // upstream version at the time this version was derived
  forkedFromDefinitionId?: UUID // UUID of the definition this was deliberately forked from
  forkedFromVersion?: integer   // version at the fork point
}
```

**`sourceDefinition*` vs `forkedFrom*`:**

| Field pair | Meaning |
|---|---|
| `sourceDefinition*` | This is a tracked copy. The consumer expects to receive upstream updates. |
| `forkedFrom*` | This deliberately diverged. The consumer no longer expects to track upstream. |

Both may be present during a transition from tracking to forking. A definition with neither is locally authored with no upstream relationship.

#### `DefinitionProvenance`

Publisher and package origin metadata.

```typescript
{
  publisher?: string          // namespace or org of the original author, e.g. "com.acme"
  sourcePackage?: string      // package name that bundled this definition
  packageVersion?: string     // semver of the source package
  importedAt?: ISO8601        // when this was imported into the local registry
}
```

`packageVersion` is distinct from `FieldDefinition.version`. A package at `1.3.0` may contain `decision_statement@3` and `context@2`. Bumping the package to `1.4.0` (e.g. adding a new field) does not change the version of unchanged definitions.

#### `DefinitionReference`

A stable pointer to a specific definition version. Used in dependency manifests, import records, and cross-package references.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer    // min: 1
  definitionType?: "field" | "module" | "template"
  // When omitted, consuming systems may assume "field" for backward compatibility.
  // Required when dependencyRefs contains module or template references.
}
```

Canonical string form (derived from the scalar identity fields):

```
namespace/name@version
```

Implementations may expose the canonical string as a convenience property. The three scalar identity fields (`namespace`, `name`, `version`) are what must be stored and validated.

#### `DefinitionPackage`

The distributable artifact. Contains field, module, template, and relation type definitions with a complete dependency manifest.

```typescript
{
  // Package identity
  schemaVersion: string      // SCDS spec version this package conforms to, e.g. "1.0"
  packageId: UUID            // stable ID for this publication event
  packageName: string
  packageVersion: string     // semver, e.g. "1.2.0"
  publishedAt: ISO8601
  publisher?: string
  description?: string
  homepage?: string          // URL to documentation or registry page

  // Content (at least one of fields, modules, or templates must be non-empty)
  fields: FieldDefinition[]
  modules: ModuleDefinition[]
  templates?: TemplateDefinition[]
  relationTypes?: RelationTypeDefinition[]  // relation type definitions for consumers; see 4.7

  // Dependency mode
  mode: "bundled" | "standalone"

  // Complete dependency manifest (required in both modes)
  dependencyRefs: DefinitionReference[]
  // Contains field references (definitionType: "field") used by modules,
  // and module references (definitionType: "module") used by templates.
  // Consumers use this manifest to validate completeness without parsing content internals.
}
```

**`mode` semantics:**

| Mode | Meaning |
|---|---|
| `"bundled"` | All `FieldDefinition` records referenced by any module, and all `ModuleDefinition` records referenced by any template, are included in `fields[]` and `modules[]` respectively. Self-contained and portable. |
| `"standalone"` | Dependencies are expected to be pre-installed in the consumer's registry. `dependencyRefs` is the required manifest. |

**Package invariants** (see also Section 5):
- Every `fieldId` in any `ModuleFieldAssignment` within `modules[]` must appear in `dependencyRefs[].id`.
- Every `moduleDefinitionId` referenced by any `TemplateDefinition` in `templates[]` must appear in `dependencyRefs[].id` (with `definitionType: "module"`).
- If `mode === "bundled"`: every field `DefinitionReference` in `dependencyRefs` must have a matching `FieldDefinition` in `fields[]`; every module `DefinitionReference` must have a matching `ModuleDefinition` in `modules[]`.

`dependencyRefs` is required in both modes because consumers need the complete manifest to validate completeness without parsing content internals.

---

### 4.5 Layer 5 — Import Tracking

These types describe the state of imported definitions in a consumer's local registry. They enable consumers to track upstream lineage, manage updates, and surface conflicts.

#### `ImportMode`

```typescript
"upstream-tracked" | "local-copy" | "local-fork"
```

| Mode | Meaning |
|---|---|
| `"upstream-tracked"` | Consumer expects to receive updates from the source package. Conflicts are surfaced when local and upstream diverge. |
| `"local-copy"` | Imported as a snapshot. No update tracking. Consumer manages the definition independently. |
| `"local-fork"` | Deliberately diverged. Consumer maintains its own version history. Upstream lineage is preserved for reference but not tracked. |

#### `ImportRecord`

One record per imported definition (field or module) in a consumer's local registry.

```typescript
{
  // What was imported
  definitionId: UUID
  definitionType: "field" | "module" | "template"
  namespace: string
  name: string
  version: integer

  // Import configuration
  mode: ImportMode
  importedAt: ISO8601

  // Source package reference
  sourcePackageId: UUID
  sourcePackageName: string
  sourcePackageVersion: string

  // Upstream tracking (meaningful when mode is "upstream-tracked")
  latestKnownUpstreamVersion?: integer
  updateAvailable?: boolean
  updateCheckedAt?: ISO8601

  // Conflict state
  conflictState?: "clean" | "local-ahead" | "upstream-ahead" | "diverged"
  conflictDetectedAt?: ISO8601

  // Local fork state (meaningful when mode is "local-fork" or after local edit)
  localVersion?: integer     // local version counter, independent of upstream
  localEditedAt?: ISO8601
}
```

#### `ImportSummary`

A consumer's complete picture of its imported definitions.

```typescript
{
  generatedAt: ISO8601
  fields: ImportRecord[]
  modules: ImportRecord[]
  templates: ImportRecord[]
}
```

Implementations may compute derived counts (updates available, conflicts, tracked vs local) from the summary for display purposes.

---

### 4.6 Layer 6 — Registry

A registry catalog is a discoverable index of available definition packages. This layer defines the shape of a registry's published metadata — not the registry protocol, authentication, or federation mechanism.

#### `RegistryEntry`

One entry in a registry catalog.

```typescript
{
  packageId: UUID
  packageName: string
  packageVersion: string
  publisher: string
  description?: string
  publishedAt: ISO8601
  homepage?: string
  tags?: string[]           // searchable subject tags for the package
  fieldCount: integer       // min: 0
  moduleCount: integer      // min: 0
  templateCount?: integer   // min: 0
  relationTypeCount?: integer  // min: 0
  downloadUrl?: string      // URL to fetch the DefinitionPackage JSON
  checksum?: string         // SHA-256 hex digest of the package JSON for integrity verification
}
```

#### `RegistryCatalog`

A registry's published index.

```typescript
{
  schemaVersion: string     // SCDS spec version
  registryId: UUID          // stable ID for this registry
  registryName: string
  catalogVersion: string    // registry's own catalog version (semver)
  updatedAt: ISO8601
  homepage?: string
  entries: RegistryEntry[]
}
```

**On decentralisation:**
This specification intentionally does not define registry authority. Multiple registries may coexist. A consumer may index multiple catalogs. Organisational forks and community standards can coexist using the same schema without requiring a central authority.

---

### 4.7 Layer 7 — Relationships

> **Status**: Planned core layer. Schema is provisional in this version. The type shapes and directionality semantics are defined here; relation taxonomy conventions and lifecycle interaction require a dedicated design pass before this layer is considered stable.

Relationships are first-class semantic links between SCDS instances — `FreeformInstance`, `TypedInstance`, and `ModuleInstance`. They capture structure, dependency, sequence, derivation, refinement, and authority where relevant. They allow a document to become a navigable semantic model rather than a flat collection of text blocks.

Relationships allow implementations to construct **semantic graphs** over instances for navigation, analysis, projection, and reasoning. A collection of instances connected by typed relationships is a semantic graph — not merely a set of documents. Implementations may traverse, index, filter, and project these graphs to answer questions about structure, dependencies, derivation history, and semantic topology that flat document collections cannot answer.

Relationships span tiers. A `FreeformInstance` may be the source of a `derived-from` relation to the `ModuleInstance` records it graduated into. A `TypedInstance` may have a `refines` relation to a `FreeformInstance` it replaced. Freeform graduation is not representable without relationships that cross tier boundaries.

```
Stage A contains Task B
Task B depends-on Task C
Decision D-004 supersedes D-001
Budget line refines rough cost estimate
Workshop photo evidences Stage 1 completion
```

Without explicit relationships, this structure is implicit in human prose, invisible to AI reasoning, and lost to any system that needs to navigate, analyse, or render the semantic model.

#### References vs Relationships

These are distinct and should not be conflated:

| Type | Meaning | Semantic consequence |
|---|---|---|
| Reference | A citation — one instance mentions another in prose | None; informational only |
| Relationship | A semantic assertion — one instance stands in a defined relation to another | Adds navigable structure, dependency, derivation, or authority |

"This plan mentions the budget spreadsheet" is a reference. "This task depends-on that task" is a relationship. "This decision supersedes an earlier one" is a relationship. References add context; relationships add machine-navigable semantic structure. Implementations may support lightweight references via `meta` or free-text fields. The `SemanticRelation` type is reserved for assertions that carry semantic consequence beyond citation.

#### `SemanticRelation`

```typescript
{
  relationId: UUID

  relationType: string
  // The semantic type of this relation. Free-form string.
  // See Relation Taxonomy below for recommended conventions.

  // Directionality: source is the asserting instance; target is the related instance.
  // The relation reads: "source [relationType] target."
  // "D-004 supersedes D-001" → sourceInstanceId: D-004, targetInstanceId: D-001
  // "Stage A contains Task B" → sourceInstanceId: Stage-A, targetInstanceId: Task-B
  //
  // Instance IDs may reference any SCDS instance type:
  // FreeformInstance.instanceId, TypedInstance.instanceId, or ModuleInstance.instanceId.
  // Relationships may cross tier boundaries — a FreeformInstance may be the target
  // of a derived-from relation from the ModuleInstance records it graduated into.
  sourceInstanceId: UUID    // the asserting instance (any SCDS instance type)
  targetInstanceId: UUID    // the related instance (any SCDS instance type)

  // Assertion provenance
  assertedBy?: "human" | "ai" | "imported"
  confidence?: number       // 0.0–1.0; meaningful for ai-asserted relations
  createdAt?: ISO8601
  createdBy?: string        // human identifier or system agent

  // Relation status and temporal validity
  status?: "proposed" | "active" | "rejected" | "superseded"
  // "proposed" — candidate relation awaiting review (typical for ai-asserted)
  // "active"   — confirmed and in effect (default when omitted)
  // "rejected" — considered and explicitly declined; retained for audit
  // "superseded" — replaced by a later relation assertion
  validFrom?: ISO8601       // when this relation became or becomes valid
  validUntil?: ISO8601      // when this relation ceases to be valid; null means indefinite

  notes?: string

  // Source provenance for this relation assertion (optional)
  sourceRefs?: SourceReference[]
  // Links from this relation assertion back to the source material that motivated it.
  // Use when the relation was AI-asserted from transcript evidence, or when an editor
  // can point to the discussion that established the relationship.
  // Example: an AI-asserted "evidences" relation between a workshop photo and a stage
  // completion claim, with sourceRefs pointing to the transcript segment where that
  // evidence was discussed.

  meta?: Record<string, unknown>
}
```

#### Directionality convention

`source` is the asserting instance; `target` is the related instance. The relation reads: "`source` [relationType] `target`."

| Relation | source | target |
|---|---|---|
| `contains` | the stage | the task inside it |
| `depends-on` | the task that depends | the task it needs |
| `refines` | the detailed version | the rough version |
| `supersedes` | the newer decision | the older decision |
| `evidences` | the source material | the claim it supports |
| `blocks` | the blocking instance | the instance blocked |

This convention must be consistent across implementations for relationships to be interoperable. See Section 5, Invariant 19.

#### Relation taxonomy

The standard does not enumerate a fixed set of allowed `relationType` values. Instead it defines recommended semantic categories. Implementations should use these category conventions as a guide when choosing or defining relation types.

| Category | Examples | Notes |
|---|---|---|
| Composition | `contains`, `part-of`, `has-section` | Structural containment — project → stage → task |
| Refinement | `refines`, `expands`, `summarises`, `details` | One instance develops or condenses another |
| Dependency | `depends-on`, `requires`, `blocks`, `enables` | Execution or logical dependencies |
| Sequence | `precedes`, `follows`, `overlaps` | Temporal order where causal direction is not implied |
| Derivation | `derived-from`, `extracted-from`, `based-on` | One instance was generated or synthesised from another |
| Evidence | `evidences`, `supports`, `contradicts` | Source material or supporting/challenging claims |
| Governance | `supersedes`, `amends`, `ratifies`, `delegates` | Changes formal authority, validity, or delegation |
| Association | `relates-to`, `links-to` | Weak semantic links without stronger assertable category |

**On relation type identifiers and cross-system interoperability:**
`relationType` is a free-form string, which enables flexibility but creates a federation risk: System A uses `part-of`, System B uses `contains` for the same semantic relationship, and graph traversal across systems breaks.

The standard addresses this through two conventions:

1. **Standard types** — the canonical relation types listed in the taxonomy above (`contains`, `depends-on`, `supersedes`, `refines`, `derived-from`, `evidences`, `precedes`) are their own identifiers. Implementations should use these exact strings for common relation types to ensure cross-system interoperability.

2. **Custom types** — relation types not covered by the taxonomy should use the `namespace/name` format (e.g. `com.acme.hr/transferred-to`, `community.adr/motivated-by`). This prevents collision with standard types and between namespaces.

Implementations that wish to assert equivalence between their custom type and a standard type (e.g. `org.acme/supersedes` ≡ `supersedes`) may document this mapping in their registry entry, but the specification does not define a formal equivalence mechanism in this version.

**On `semanticObjectType` interoperability:**
When `semanticObjectType` is used for cross-system identification (e.g. "find all decision instances"), bare strings like `"decision"` or `"task"` interoperate only by convention — two systems using the same string may mean different things. Implementations that require reliable semantic object type identification across system boundaries should use the `namespace/name` format for custom types (e.g. `com.acme.pm/task`) while bare common values remain acceptable for single-system or convention-based use.

#### Canonical directions and inverse relations

The taxonomy above lists both forward and inverse forms in the same column (e.g. `contains` and `part-of`). These are not equivalent — they are converses of each other. Recording both would duplicate the edge. Implementations must choose a canonical direction and store only one edge per relationship.

The following table defines canonical forward forms for the most common cross-category relation types. Implementations should use the canonical form and derive the inverse when needed rather than storing both.

| Canonical (forward) | Converse (inverse) | Category |
|---|---|---|
| `contains` | `part-of` | Composition |
| `depends-on` | `required-by` | Dependency |
| `supersedes` | `superseded-by` | Governance |
| `refines` | `refined-by` | Refinement |
| `derived-from` | `source-of` | Derivation |
| `evidences` | `evidenced-by` | Evidence |
| `precedes` | `follows` | Sequence |

The canonical direction for the remaining taxonomy examples is left to namespace authors to establish within their own vocabularies. When creating a custom relation type, its canonical direction should be documented alongside its definition.

#### `RelationTypeDefinition`

A machine-readable definition for a `relationType` string. Registries may publish these alongside field and module definitions to enable relation type discovery, validation, and semantic interoperability.

```typescript
{
  relationType: string      // the exact string used in SemanticRelation.relationType
  namespace: string         // namespace owning this relation type definition
  label?: string            // human-readable label, e.g. "depends on"
  description?: string      // what this relation asserts semantically
  category?: "composition" | "refinement" | "dependency" | "sequence" | "derivation" | "evidence" | "governance" | "association"
  canonicalDirection?: string
  // Describes what source and target mean for this type, e.g.:
  // "source is the dependent task; target is the task it depends on"
  inverseType?: string      // the canonical inverse relation type string, if one exists
}
```

`RelationTypeDefinition` is optional metadata — implementations are not required to resolve `relationType` strings against a definition before accepting a `SemanticRelation`. However, registries that publish `RelationTypeDefinition` records allow consumers to validate, display, and reason about relation types without hard-coding them. Relation type definitions may be included in `DefinitionPackage` (see Layer 4) or published separately by a registry.

#### Instance revision versus replacement

Relationships make explicit a conceptual distinction that prose alone cannot:

| Action | Modelled as | Meaning |
|---|---|---|
| Edit | Mutation of the same instance | The group refined its understanding in-place |
| Refine | New instance + `refines` relation | A more detailed successor created from a rough original |
| Supersede | New instance + `supersedes` relation | The prior instance's authority is replaced; prior state preserved |
| Derive | New instance + `derived-from` relation | A related but semantically distinct object produced from a source |

These distinctions matter for AI workflows (what context to include), governance (what is still authoritative), graph rendering (which edges carry authority versus information), and audit trails (how understanding arrived at its current form).

#### Non-decision example: project planning

A project plan may begin with three high-level stage modules. Over time each stage is refined with task modules, budget modules, risk modules, evidence modules, and asset reference modules. Relationships express containment, dependency, refinement, evidence, and sequencing without requiring the project plan to be treated as a decision or governance process.

```json
{ "relationType": "contains",    "sourceInstanceId": "<plan>",    "targetInstanceId": "<stage-discovery>" }
{ "relationType": "contains",    "sourceInstanceId": "<stage>",   "targetInstanceId": "<task-interviews>" }
{ "relationType": "depends-on",  "sourceInstanceId": "<task-b>",  "targetInstanceId": "<task-a>" }
{ "relationType": "refines",     "sourceInstanceId": "<budget-detail>", "targetInstanceId": "<budget-rough>" }
{ "relationType": "evidences",   "sourceInstanceId": "<workshop-photo>", "targetInstanceId": "<stage-discovery>" }
```

The governance relations (`supersedes`, `delegates`, `ratifies`) apply when the semantic object type calls for them. They are one profile of the relationship layer, not its primary purpose.

#### AI-asserted relations

Relations with `assertedBy: "ai"` and a `confidence` value are candidate relations. Implementations should treat them as proposals requiring human review rather than authoritative assertions until ratified. The `confidence` field (0.0–1.0) allows consumers to filter or triage candidates by confidence threshold.

---

### 4.8 Layer 8 — Containers

A container is a lightweight grouping boundary over a collection of SCDS instances. It answers questions that the relationship graph cannot answer on its own: which instances belong together as a logical unit? What is the root object? What should be exported or queried together?

Containers are not semantic objects with fields — they are addressing and scoping boundaries. They do not own semantic state; instances do. Where a `contains` relationship asserts "A is part of B" (a semantic claim), a `SemanticContainer` asserts "these instances collectively form a unit for boundary purposes" (a scope claim). Both are useful; neither replaces the other.

Without a container, an implementation cannot cleanly answer:
- Which graph are these instances part of?
- What should be exported together?
- What is the root object for a session?
- What counts as "this project" or "this workspace"?

These questions arise regardless of whether a rich relationship graph has been constructed. A `SemanticContainer` provides the minimal boundary that allows instances to be grouped, addressed, and scoped before the full relationship layer is in place.

#### `SemanticContainer`

```typescript
{
  containerId: UUID

  namespace?: string         // namespace of the container definition, if formally named
  name?: string              // snake_case stable key within the namespace

  title: string              // human-readable label, e.g. "Q3 2025 Planning Cycle"

  containerType?: string
  // Optional lightweight semantic hint — analogous to ModuleDefinition.semanticObjectType.
  // Free-form; e.g. "project", "meeting", "sprint", "workspace", "portfolio".
  // No conforming implementation is required to act on it.

  rootInstanceIds?: UUID[]
  // The top-level instances this container was created to hold.
  // These are not necessarily the only members; memberInstanceIds may include transitively
  // contained instances. Implementations may derive contained sub-instances from the
  // relationship graph (traversing contains relations from roots).

  memberInstanceIds?: UUID[]
  // Explicit, queryable membership list for all instances in scope, including nested members.
  // When present, allows membership queries without graph traversal.
  // When omitted, membership is defined by traversing contains relations from rootInstanceIds.
  // Both strategies are valid; implementations should document which they use.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

**On containers and the relationship layer:**
Containers and `SemanticRelation` are complementary. Relationship-first implementations use `contains` relations and derive container membership by traversal. Container-first implementations use explicit `memberInstanceIds` and optionally layer in `contains` relations for finer navigation. Implementations may use either or both strategies.

**On container identity in relationships:**
`SemanticContainer.containerId` is not an instance ID. `SemanticRelation.sourceInstanceId` and `targetInstanceId` reference `FreeformInstance`, `TypedInstance`, and `ModuleInstance` records — not containers. Implementations that need to assert relationships between containers and instances should use the relationship `meta` field or a dedicated container-to-instance convention documented in their namespace. A future version may define `sourceType`/`targetType` fields on `SemanticRelation` to support container-level relationships explicitly.

---

### 4.9 Layer 9 — Document Projection

> **Status**: Provisional. The types and source query model are sketched here to establish the architectural boundary and avoid constraining lower layers. Detailed semantics, query language, and renderer integration require a dedicated design pass before this layer is considered stable.

A document projection template defines how a container's collection of semantic instances is assembled into a coherent, readable document. It is a Level 2 template — it operates over a container, not a single module instance.

This layer answers the question that lower layers cannot: given a `SemanticContainer` full of decisions, proposals, exercises, risks, and articles, how do they become a structured governance document, board paper, or project summary?

The key architectural principle: **a document is a projection, not a record type**. The instances remain individually addressable, relatable, and revisable. The document is a view assembled from them — not a container that absorbs their identity.

#### `DocumentSectionSource`

Defines how a section's instances are selected from the container. One of four strategies:

```typescript
type DocumentSectionSource =
  | {
      type: "fixed-instances"
      instanceIds: UUID[]
      // Explicit list. Use for preamble, cover page, or manually curated sections.
    }
  | {
      type: "module-type-query"
      semanticObjectType: string          // e.g. "decision", "article", "exercise"
      lifecycleState?: string             // filter by current lifecycle state
      containerIds?: UUID[]              // scope to specific containers; defaults to the document's container
      // Use for sections like "All decisions" or "Open exercises".
    }
  | {
      type: "relation-query"
      fromInstanceId: UUID               // traverse relations from this instance
      relationType: string               // e.g. "contains", "evidences", "derived-from"
      direction?: "forward" | "inverse"  // default: "forward"
      // Use for sections like "Tasks derived from this decision" or "Evidence for this stage".
    }
  | {
      type: "container-subset"
      containerId: UUID                  // a sub-container within the document's root container
      containerType?: string             // optional filter by container type
      // Use for multi-team or multi-project documents where each section covers a sub-scope.
    }
```

#### `DocumentSection`

One section in a document projection template.

```typescript
{
  sectionId: string        // stable key within this template
  title?: string           // rendered section heading
  description?: string     // authoring guidance; not typically rendered

  order: integer           // min: 0; section sequence in the rendered document

  source: DocumentSectionSource
  // How instances for this section are selected. See DocumentSectionSource.

  renderTemplateId?: UUID
  // Optional TemplateDefinition (Level 1) used to render each instance in this section.
  // When absent, implementations use a default rendering for the instance type.

  ordering?: {
    fieldId?: UUID          // sort by a specific field value within the module
    direction?: "asc" | "desc"  // default: "asc"
  }

  required?: boolean        // default: false; if true, an empty section is an error
  emptyBehavior?: "hide" | "show-placeholder"
  // "hide" — omit the section entirely if no instances match
  // "show-placeholder" — render the section heading with a placeholder message
}
```

#### `DocumentTemplate`

A versioned, container-level projection template. Defines how a container's instances are assembled into a readable document.

```typescript
{
  // Identity
  id: UUID
  namespace: string
  name: string           // snake_case programmatic key
  version: integer       // min: 1

  // Description
  description: string    // what kind of document this produces; intended audience

  // Container scope (optional)
  containerType?: string
  // When set, this template is intended for containers of this type.
  // Implementations may use this to suggest appropriate templates.
  // Not a hard constraint — implementations decide enforcement.

  // Document sections
  sections: DocumentSection[]

  // Document-level output configuration (optional)
  preamble?: string
  // Template string rendered before all sections.
  // Variable substitution uses {{variable-name}} syntax.
  // Standard variables: {{container-title}}, {{date}}, {{container-id}}

  format?: string        // target format hint, e.g. "markdown", "adoc", "html"

  // AI framing (optional)
  aiGuidance?: AiGuidance
  // purpose: what kind of document this template produces
  // extraction: context injected when AI assists with document-level tasks
  //             (e.g. generating an executive summary from the assembled sections)

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: DefinitionLineage
  provenance?: DefinitionProvenance
}
```

**Why this is separate from `TemplateDefinition`:**

| Concern | `TemplateDefinition` (Level 1) | `DocumentTemplate` (Level 2) |
|---|---|---|
| Operates on | One module instance | A container's instance collection |
| Defines | How fields are viewed, facilitated, or exported | How instances are selected, ordered, and assembled |
| Bound to | A specific `ModuleDefinition` | Optionally scoped to a `containerType` |
| AI framing | Workflow context for one object | Document-level context for assembled output |

A `DocumentTemplate` may reference multiple `TemplateDefinition` records (via `DocumentSection.renderTemplateId`) — one per instance type in the document. It orchestrates them; it does not replace them.

**On source query semantics:**
The `DocumentSectionSource` types intentionally avoid a general query language. The four strategies cover the most common document assembly patterns without requiring implementations to embed a full query engine. Complex filtering that cannot be expressed with these types belongs in implementation-layer document assembly logic, not in the template definition.

---

## 5. Key Invariants

Conforming implementations must uphold the following invariants.

### Field semantics

1. `ModuleFieldAssignment.displayLabel` and `ModuleFieldAssignment.displayHint` are for rendering only. They must not affect AI guidance, extraction logic, `valueType` interpretation, or validation.

2. A `ModuleDefinition` must not redefine, override, or duplicate the semantic content of any `FieldDefinition` it includes. If different semantics are needed for a field in a specific module context, a distinct `FieldDefinition` with its own identity and lineage must be created.

3. A `FieldDefinition`'s `aiGuidance` belongs to the field. Module-level `aiGuidance` provides session framing only.

### Lifecycle

4. `ModuleDefinition.lifecycle.initialState` must reference a `name` that appears in `lifecycle.states[]` and where `isInitial === true`.

5. Every `from` and `to` value in `lifecycle.transitions[]` must reference a `name` that appears in `lifecycle.states[]`.

6. A `ModuleInstance.lifecycleState` value, when present, must reference a `name` that appears in the associated `ModuleDefinition.lifecycle.states[]`.

### Distribution

7. Every `fieldId` referenced in any `ModuleFieldAssignment` within a `DefinitionPackage.modules[]` must appear as the `id` of an entry in `DefinitionPackage.dependencyRefs`.

8. If `DefinitionPackage.mode === "bundled"`: every `DefinitionReference` in `dependencyRefs` must have a matching `FieldDefinition` in `fields[]` (matched on `id` and `version`).

9. `FieldDefinition.id` is stable across versions. A new `id` means a new definition, not a new version of an existing one.

### Module validation

10. All `fieldId` values referenced in any `ModuleValidationRule` within `ModuleDefinition.moduleValidationRules[]` must appear in `ModuleDefinition.fields[].fieldId`. Cross-field rules cannot reference fields outside the module.

11. A `conditional-required` rule must supply `predicateFieldId`, `predicateValue`, and `targetFieldId`. A `field-ordering` rule must supply `predicateFieldId`, `targetFieldId`, and `effect`. A `mutual-exclusion` rule must supply `fieldIds` with at least two entries. Rules missing required fields are invalid.

### Templates

12. Every `fieldId` in `TemplateDefinition.fieldViews[]` must appear in the referenced module's `fields[].fieldId`. A template cannot introduce fields that are not part of its bound module.

13. Every `fieldId` in `TemplateFacilitationStep.fieldIds[]` must appear in `TemplateDefinition.fieldViews[].fieldId`. A step may reference fields that have `visible: false` in the default view — facilitation steps may reveal fields progressively that are otherwise hidden from the general layout.

14. `TemplateFieldView.displayLabel`, `TemplateFieldView.displayHint`, and `TemplateFieldView.editorHintOverride` are for rendering only. They must not affect AI guidance, extraction logic, `valueType` interpretation, or validation.

15. A `TemplateDefinition` must not override, redefine, or duplicate the semantic content of any `FieldDefinition` or `ModuleDefinition` it references. Template-level `aiGuidance` is workflow framing; it does not redefine field extraction semantics.

### Freeform and typed instances

16. `FreeformSection.name` values must be unique within a `FreeformInstance`. Section names are the only identity mechanism within a freeform instance; duplicates make sections unaddressable.

17. `TypedField.name` values must be unique within a `TypedInstance`. Field names are the only identity mechanism within a typed instance.

### Module instances

18. `ModuleInstance.moduleDefinitionId` and `ModuleInstance.moduleDefinitionVersion` are the authoritative module binding. `moduleNamespace` and `moduleName` are denormalised convenience fields for consumers that need them without resolving the UUID. If they conflict with the resolved `ModuleDefinition`, the definition identity takes precedence and the instance is considered invalid until corrected.

### Relationships

19. In a `SemanticRelation`, `sourceInstanceId` is the asserting instance and `targetInstanceId` is the related instance. The relation reads: "source [relationType] target." This convention must not be reversed. A consumer interpreting `D-004 supersedes D-001` must represent this as `sourceInstanceId: D-004, targetInstanceId: D-001`, not the inverse. Relation semantics emerge entirely from `relationType` — the directionality convention does not imply agency, authority, or causation.

20. `SemanticRelation` is reserved for assertions that carry semantic consequence beyond simple mention or citation. Lightweight prose references that do not assert a structural, causal, or governance relationship should not be modelled as `SemanticRelation` records.

### Distribution — templates

21. Every `moduleDefinitionId` referenced by any `TemplateDefinition` in `DefinitionPackage.templates[]` must appear in `DefinitionPackage.dependencyRefs` with `definitionType: "module"`. If `mode === "bundled"`, that `ModuleDefinition` must be present in `modules[]`.

### Containers

22. `SemanticContainer.rootInstanceIds` and `SemanticContainer.memberInstanceIds`, when present, must reference valid SCDS instance IDs (`FreeformInstance.instanceId`, `TypedInstance.instanceId`, or `ModuleInstance.instanceId`). A `containerId` is not an instance ID and must not appear in `rootInstanceIds`, `memberInstanceIds`, `SemanticRelation.sourceInstanceId`, or `SemanticRelation.targetInstanceId`.

### Repeatability

23. If a `ModuleFieldAssignment.repeatable` is false or absent, its corresponding `FieldValue` must use `value` and must not include `entries`.

24. If a `ModuleFieldAssignment.repeatable` is true, its corresponding `FieldValue` may use `entries`. If `minItems` is specified, `entries` must contain at least that many items. If `maxItems` is specified, `entries` must not exceed that count.

25. `ModuleFieldAssignment.minItems` and `maxItems` are valid only when `repeatable === true`. They must be ignored by validation logic when `repeatable` is false or absent.

26. A repeatable field entry (`FieldValueEntry`) does not create a new semantic instance. Use separate module instances connected by `contains` or other relationships when repeated items need their own identity, lifecycle, or graph position.

### Field groups

27. Every `groupId` in `ModuleInstance.groupValues[]` must reference a `groupId` declared in the associated `ModuleDefinition.fieldGroups[]`.

28. Within a `FieldGroupEntry.fieldValues[]`, every `fieldId` must appear in the enclosing `ModuleFieldGroup.fields[].fieldId`. Group entries cannot carry field values for fields outside their group definition.

29. A `FieldGroupValue.entries` list must satisfy `ModuleFieldGroup.minItems` and `maxItems` where specified. If `ModuleFieldGroup.required === true` and `repeatable === false`, exactly one entry must be present. If `required === true` and `repeatable === true`, at least `minItems` (or 1 if `minItems` is absent) entries must be present.

30. A `ModuleFieldGroup` does not create a new semantic instance. Its entries are embedded structured context within the enclosing `ModuleInstance`. Use separate `ModuleInstance` records connected by relationships when group entries need their own identity, lifecycle, provenance, or reuse across instances.

---

## 6. Lifecycle Note

### Descriptive, not prescriptive

The `lifecycle` block on a `ModuleDefinition` declares vocabulary. It does not mandate enforcement. Implementations may enforce transitions strictly, loosely, or not at all. The specification defines the schema; implementations define the policy.

### Typical progressions

State progressions depend on the semantic object type. Two examples:

**Decision-class module:**
```
exercise  →  proposal  →  decision  →  superseded
```
- `exercise` — open exploration; outcome unknown
- `proposal` — candidate commitment under consideration
- `decision` — agreed commitment
- `superseded` — replaced by a later decision

**Task-class module:**
```
backlog  →  active  →  blocked  →  complete
```
- `backlog` — not yet started
- `active` — in progress
- `blocked` — waiting on a dependency
- `complete` — settled; no further transitions

Both are illustrative. Module definitions should define state names appropriate to their own domain.

### Final states

A state with `isFinal: true` indicates no further transitions are expected. Implementations may use this to lock instance content or signal that the instance is settled for governance purposes.

### Relationships do not change lifecycle state

A `SemanticRelation` does not automatically change the `lifecycleState` of any instance it references. The `supersedes` relation indicates that a prior instance's authority is replaced, but it does not mutate the prior instance's `lifecycleState`. The `refines` relation indicates derivation, not progression. Lifecycle state changes are explicit acts — they must be performed by an implementation's transition mechanism, not inferred from relationship assertions.

This separation allows relationship graphs to express semantic structure without triggering unintended lifecycle side-effects. An implementation may choose to advance lifecycle state in response to a relationship assertion, but if it does, that policy must be explicit and documented.

---

## 7. Future Extensions

The following capabilities are planned but out of scope for this version of the specification.

### Session

A live collaborative process model. A session is a process-oriented container rather than a document-oriented one. Sessions may include transcript ingestion, live facilitation, AI assistance, collaborative editing, and semantic extraction. A session produces or enriches module instances but does not own them.

Session templates (Level 3 in the template hierarchy) would drive the facilitation process itself — which steps to follow, when to surface instances, when to request AI extraction. Deferred pending stabilisation of the session model and implementation experience with `DocumentTemplate` (Layer 9).

### Projection

Document-level projection is partially addressed by `DocumentTemplate` (Layer 9), which covers static assembled documents and governance records.

The broader projection surface — dashboards, timelines, AI context packages, real-time views, and composite renderings that are not document-shaped — remains a future concern. The full projection layer defines how semantic state is transformed into communicable form for an audience or purpose beyond what document templates cover. Projections are read-only views; they do not modify instance state.

### Revision history exchange format

A standard format for exchanging full revision history between implementations, for cases where the history itself — not just the current state — is a first-class interoperability concern. Deferred pending stabilisation of the container and relationship layers, which together provide the reference model for what constitutes a revisable unit.

### Graduation mapping record

A structured artefact recording how a freeform or typed instance was mapped to its module-level successors — including which section or field names were matched, merged, split, or interpreted. Useful for AI-assisted graduation review, audit, and provenance. Deferred pending implementation experience.

### Template inheritance and composition

As template libraries mature, inheritance and composition will become necessary. A lightweight ADR template and a governance ADR template logically share a base configuration — field selection, ordering, default `editorHint` overrides — while diverging on workflow framing and export layout.

A future version may define:
- `extendsTemplateId?: UUID` — single inheritance; the child template inherits all `fieldViews` and configuration from a parent and overrides selectively
- `composesTemplates?: UUID[]` — mixin composition; multiple templates contribute non-overlapping configuration

The current design deliberately omits both. `TemplateDefinition` is a leaf type. Implementations that need inheritance-like behaviour should currently achieve it by copying and modifying templates explicitly, using lineage tracking to record the relationship.

### Instance graph exchange format

A standard envelope for exchanging a container together with its full instance set, relationships, and source references — for cases where the complete graph, not just current state, is the interoperability concern. Natural successor to `DefinitionPackage` at the instance layer. Likely takes the form `{ container, instances[], relations[], sourceRefs[] }` and maps to RO-Crate for archive/dataset publication. Deferred pending stabilisation of Layer 9 and implementation experience.

This is my model for a system for collaborative creation

---

# claude response

This is a substantial spec — I've read it through. A few impressions and some genuine pushback, in roughly the order things landed for me.

**What's working**

The six principles do real load-bearing work. Specifically: separating semantic state from projection, treating fields as atoms with their own AI guidance, and Principle 6 (historical state has permanent value) — these aren't just framing, they shape concrete decisions later (the graduation tiers, the supersedes-vs-edit distinction, the instance-revision section). That's the test of whether a "principles" section is real or decorative, and yours passes it.

The Tier 0 → 1 → 2 progression is the most strategically important move in the document. Most collaborative-semantic systems die because they demand formalisation before the group has stabilised understanding. Allowing `FreeformInstance` as a first-class citizen with an `instanceId` that can carry through graduation is exactly right, and the explicit note that graduation is often one-to-many ("one meeting note → one decision + three tasks + two risks") is the kind of detail that signals you've actually watched this happen rather than imagined it.

The `valueType` vs `editorHint` separation is correct and the table makes it land in two lines. Same for `purpose` vs `extraction` in AI guidance — the rationale ("conflating them produces prompts that are neither good human documentation nor good AI instructions") is concrete enough to actually guide implementers.

The relationship layer's directionality invariant (#19) with the canonical-direction table is the thing that will save you from federation hell. Without that table, "contains" vs "part-of" duplicates the graph within a year.

**Where I'd push**

*Layer count.* Nine layers is a lot to hold. Some of them are doing genuinely different work (fields, modules, instances, distribution); others feel like they're layers because the document grew that way (import tracking is really a sub-concern of distribution; registry is a sub-concern of distribution too). I'd consider whether Layers 4/5/6 collapse into "Distribution" with three sections. The layer count is a cognitive cost paid by every reader of the spec, and right now it's higher than the structural distinctions warrant.

*Containers feel under-justified relative to their position.* Layer 8 introduces containers as "the root scope for export, session, and graph traversal" — but the relationship layer already gives you `contains`, and a session is going to need its own scope anyway. The text acknowledges the tension ("Containers and SemanticRelation are complementary") but doesn't fully resolve why both must exist as first-class types rather than containers being a derived view over `contains` relations from a designated root. The argument you make — "which graph are these instances part of? what should be exported together?" — is real, but it might be answered by a `Container` that's just `{containerId, rootInstanceId, title}` with everything else derived. The current shape with both `rootInstanceIds` and `memberInstanceIds` and an explicit note that implementations can do either suggests the abstraction isn't yet settled.

*The semantic-object-type interoperability hole.* You flag it honestly ("bare strings like 'decision' or 'task' interoperate only by convention") but then leave it as namespace/name being optional. Given that `DocumentSectionSource.module-type-query` filters on `semanticObjectType`, this is going to bite. Either commit to namespace/name for cross-system use or accept that document templates aren't portable across systems with different conventions. The middle ground will produce silent breakage.

*Provisional layers in a v1 spec.* Layers 7, 8, and 9 are marked provisional, but Layer 7 (relationships) is referenced extensively from earlier layers — the graduation story depends on `derived-from`, the supersedes/refines distinction depends on relationships, even the container layer references it. If relationships are provisional, much of what Layers 1-3 promise is also provisional. I'd either commit to Layer 7 as stable for v1 or be more explicit upfront that this is a v1-of-a-multi-version-rollout and the spec describes the target.

*Invariant 19 has a subtle bug.* "Relation semantics emerge entirely from `relationType` — the directionality convention does not imply agency, authority, or causation." This is true at the formal level but doesn't match the examples — `supersedes` clearly implies authority transfer, `depends-on` implies a directional constraint. I think you mean "the directionality convention alone does not imply..." — i.e. that source/target is a slot convention, not a semantic claim independent of `relationType`. Worth tightening.

**Smaller things**

The recipe-style AI guidance composition order (module → template → field extraction → negative → examples) is good but tucked inside the `TemplateDefinition` section. It deserves to be at the top of Layer 2 or in its own short section, because it's the rule that makes the whole multi-level AI guidance system coherent.

`SourceReference.sourceStandard` as a versioned string ("canonical-transcript@1.0") is the right call — it future-proofs the transcript boundary without binding you to a specific format now.

The `meta: Record<string, unknown>` pattern is pragmatic and you've justified it. One thing I'd add: a note that `meta` keys should be namespaced (`com.acme.locking.locked-by`) when they might be exchanged across systems, otherwise the escape hatch becomes a collision zone.

The freeform-section uniqueness invariant (#16) plus the AI-extraction collision footnote about `risk`/`risk_2`/`risk_3` is exactly the kind of detail that turns a spec from "looks reasonable" into "we've thought about what implementers will hit." More of these throughout would strengthen the document.

**The bigger question**

The spec is ambitious. The honest risk isn't that any individual layer is wrong — it's that the surface area is large enough that no implementation will conform to all of it, and you'll end up with "SCDS-compatible" meaning different things in different products. The provisional-status flags help, but if you want to ship this as v1.0 rather than 0.x, I'd consider whether there's a *minimum conforming subset* — say, Layers 1, 2, 3, and 7 — that you'd require for the SCDS label, with everything else as published extensions. That makes interoperability claims testable.

Overall: the conceptual architecture is the strongest part. The principle that documents are projections of negotiated semantic state, with explicit progressive formalisation and first-class historical state, is a thesis worth building on. The structural work — layer count, container justification, semantic-object-type federation — is where I'd put another editing pass before declaring v1.

---

# you asked

message time: 2026-05-21 08:43:52

Lets propose a core featureset with extensions. 

my initial use cases: 

* I want to create a system that allows for consistent output from conversation transcriptions. 
* I want to be able to conversationally refine a document, building it in iterations, feeding in multiple transcripts. 
* I am creating a governance tool that guides a group through a facilitated conversation to build up founding documents and a high quality decision log  (Although facilitation could be an extra layer ... I'd just build it first_

---

# claude response

Good. Let me work backwards from your three use cases to find the minimum spec, then propose extensions.

**What your use cases actually require**

Use case 1 — *consistent output from transcriptions*. You need: fields that tell an extractor what to populate and how, modules that group those fields into a recognisable object type, instances that hold the extracted values, and source references that link values back to transcript chunks. That's it. No relationships, no containers, no templates, no graduation tiers. A transcription comes in, an AI fills a module, you get a `ModuleInstance` with `sourceRefs`.

Use case 2 — *iterative refinement across multiple transcripts*. Now you need more. You need to update an existing instance with new transcript material (which the current spec handles via in-place edit or new instance + `refines`). You need to know which transcripts contributed to which fields, so `sourceRefs` becomes load-bearing rather than nice-to-have. You probably need the relationship layer for `refines` and `supersedes`, because "the second transcript clarified the budget" needs to be expressible without losing the first version. And you need *some* form of grouping — "this document I'm building up over multiple sessions" — which is where containers earn their place.

Use case 3 — *facilitated governance with founding docs + decision log*. This adds: lifecycle (a decision is in `exercise` → `proposal` → `decision`), the ability for facilitation to walk through fields in order (which the current `TemplateFacilitationStep` covers, but you've correctly noted facilitation could be a separate layer), and document projection (the founding document is assembled from many decision instances). Freeform → typed → defined graduation matters here too, because facilitated conversations often produce rough material first that gets formalised later.

**Proposed core**

Core SCDS — required for any conforming implementation:

- **Layer 1** (Field Content): `ValidationRule`, `AiGuidance`, `AiGuidanceExample`. Without these, fields can't carry semantics.
- **Layer 2** (Definitions): `FieldDefinition`, `ModuleDefinition`, `ModuleFieldAssignment`. *Without* `ModuleFieldGroup`, `ModuleValidationRule`, `TemplateDefinition`, or lifecycle — those become extensions.
- **Layer 3** (Instances): `ModuleInstance`, `FieldValue`, `SourceReference`. *Without* `FreeformInstance`, `TypedInstance`, `FieldGroupValue`, repeatable entries — extensions.
- **Layer 4** (Distribution): `DefinitionPackage`, `DefinitionReference`, `DefinitionLineage`, `DefinitionProvenance`. Needed if you ever share definitions; arguably could be an extension, but I'd keep it in core because without it you have no interoperability story at all.
- **Layer 7** (Relationships) — but only `SemanticRelation` itself, with the directionality invariant. The canonical-direction table moves to a recommended-types extension.

This core directly serves use case 1 and gets you 70% of use case 2. It's small enough that a conforming implementation is realistic to build and validate.

**Proposed extensions**

I'd group them by which use case forces them, so you can stage adoption:

*Extension: Progressive Formalisation* (Tier 0/1). Adds `FreeformInstance`, `TypedInstance`, graduation semantics, and `derived-from` as a recommended relation type. Use case 3 needs this for facilitated sessions that start rough.

*Extension: Lifecycle*. Adds `LifecycleStateDefinition`, `LifecycleTransitionDefinition`, and the `lifecycle` block on `ModuleDefinition` plus `lifecycleState` on `ModuleInstance`. Use case 3 needs this for the decision log.

*Extension: Containers*. Adds `SemanticContainer` and the related invariants. Use case 2 needs this for "the document I'm building up." Use case 3 needs it for the founding document scope.

*Extension: Templates Level 1*. Adds `TemplateDefinition`, `TemplateFieldView`, `TemplateFacilitationStep`, `TemplateExportConfig`. Use case 3 needs this for facilitation — though as you've noted, facilitation can be its own layer.

*Extension: Document Projection (Templates Level 2)*. Adds `DocumentTemplate` and `DocumentSection`. Use case 3 needs this for assembling the founding document.

*Extension: Field Groups*. Adds `ModuleFieldGroup`, `FieldGroupValue`, `FieldGroupEntry`. None of your three use cases requires this — it's a refinement for when repeatable scalars aren't enough. I'd defer it.

*Extension: Cross-Field Validation*. Adds `ModuleValidationRule`. Useful for governance (`end_date` required when `status === "Approved"`) but not in the critical path. Defer.

*Extension: Repeatable Fields*. Adds `repeatable`, `minItems`, `maxItems` on `ModuleFieldAssignment`, plus `FieldValueEntry` and `entries` on `FieldValue`. Probably needed in core actually — too many real modules have lists. Could go either way.

*Extension: Recommended Relation Vocabulary*. The canonical-direction table, the category taxonomy, `RelationTypeDefinition`. Without this, relation types don't federate; with it, they do.

*Extension: Import Tracking*. Layer 5 in your current spec. Only needed when you actually start consuming others' packages and tracking updates. Defer.

*Extension: Registry*. Layer 6. Defer until you have multiple publishers.

**The staging that falls out of this**

Phase 1 — Core only. Get use case 1 working end-to-end. Transcription → module instance with sourceRefs. This is testable, demonstrable, and the foundation everything else builds on.

Phase 2 — Add Containers, Relationships vocabulary, Repeatable Fields, Progressive Formalisation. Now use case 2 works properly — you can build documents iteratively across transcripts with provenance back to source material and rough-to-formal progression.

Phase 3 — Add Lifecycle, Templates L1, Document Projection. Use case 3 lights up. The governance tool can facilitate, run a decision log with states, and project a founding document.

Phase 4 — Field Groups, Cross-Field Validation, Import Tracking, Registry. Mature ecosystem features. Only when there's demand.

**One thing to reconsider**

You said facilitation "could be an extra layer ... I'd just build it first." I'd push back gently. If you build facilitation first inside the governance tool, you'll bake assumptions about facilitation into the core data model that don't belong there. The cleaner path is: build Phase 1 core, then build the facilitation tool on top of it as a *consumer* of templates with facilitation steps. The data model stays clean; the facilitation logic lives where it belongs, in the application layer. You'll know you've done it right when a second facilitation tool could be written against the same templates without modification.

The argument for "build it first anyway" is that you'll learn things from building facilitation that change what the core should look like. That's legitimate. But the discipline I'd suggest: build the facilitation tool, then before declaring the spec stable, ask which parts of what you built belong in the data model and which belong in the application. The current spec's `TemplateFacilitationStep` is on the right side of that line — it describes a step without mandating how facilitation drives it — and that's the test to apply to anything else facilitation needs.

---

# you asked

message time: 2026-05-21 09:12:19

Let's actually review what the elements of this are called. Now that we have developed it, we can take a bigger picture view.

I need to be able to define the semantic infrastructure of a document, and then pass a transcript (or 6) through it to create an output.

This feels like a document template, but that's not what our templates actually do in this model. Maybe that's a sign

---

# claude response

Your instinct is pointing at something real. Let me name the tension precisely.

**What "template" naturally means vs what it does in the spec**

In plain English, a document template is *the pattern you fill in* — it defines what a document is and what goes in it. In the current spec, templates are *view/workflow configuration over existing instances*. They're a presentation layer, not a definition layer. So when you say "I want to define the semantic infrastructure of a document and pass transcripts through it" — that IS a template in the natural sense, but it's not what `TemplateDefinition` or `DocumentTemplate` do.

That's a naming smell pointing at something structural.

**The actual concepts underneath**

Working from what you actually need, I think there are five distinct things:

1. **Field** — an atomic semantic unit. Knows what it captures and how to extract it. Currently `FieldDefinition` — name is fine, drop the suffix.

2. **Type** — a named composition of fields representing one kind of semantic object. Currently `ModuleDefinition`. "Module" is a weak name — it means something specific in software (a deployable unit) and something vague here. What it actually is: a *type definition* for a semantic object. `Decision`, `Action`, `Risk`, `Article`.

3. **Schema** — the definition of what a complete document consists of. Which Types appear in it, in what structure, with what relationships between them. **This concept is currently missing as a first-class object.** It's partially served by `DocumentTemplate` (the assembly step) and partially implied by the relationship layer, but neither one says "a Board Report contains: one Context, N Decisions, N Actions, one Summary." That definition — the semantic infrastructure — needs its own name.

4. **Record** — an instantiated Type with values. Currently `ModuleInstance`. "Instance" is technically correct but cold. "Record" carries the right connotation: a thing that was captured, that has provenance, that can be revised.

5. **View** — a presentation/export/facilitation configuration over a Record or Document. Currently both `TemplateDefinition` and `DocumentTemplate`. These are legitimately different scopes (one Record vs a whole Document) but they're both *views*, not templates in any meaningful sense.

**The missing concept is doing real work**

The thing you're calling "document template" in the natural sense — the Schema — is the thing you'd hand to an extraction pipeline and say "go find all of these in this transcript." It answers:

- What Types should I extract?
- What relationships should exist between the resulting Records?
- What does a complete, valid document of this type look like?

Without Schema as a first-class concept, your extraction pipeline has no single artifact to target. It has to be assembled from the relationship between DocumentTemplate + ModuleDefinitions + relationship vocabulary — which is architecturally correct but operationally awkward.

**Proposed vocabulary**

| Current | Proposed | Reason |
|---|---|---|
| `FieldDefinition` | `Field` | Suffix adds nothing |
| `ModuleDefinition` | `Type` | What it actually is |
| `ModuleInstance` | `Record` | Carries the right connotation |
| `TemplateDefinition` (L1) | `View` | What it actually does |
| `DocumentTemplate` (L2) | `Document View` | Consistent; assembly of Records into a readable form |
| *(missing)* | `Schema` | The semantic infrastructure of a document type |
| `SemanticContainer` | `Container` | Already fine, drop adjective |
| `SemanticRelation` | `Relation` | Already fine, drop adjective |

**What Schema looks like**

Roughly:

```typescript
{
  id: UUID
  name: string           // e.g. "board_report", "founding_document"
  description: string

  // What this document type consists of
  rootTypes: TypeRef[]   // the top-level Types to extract
  structure: [           // expected relationships between extracted Records
    { relationType: "contains", sourceType: "board_report", targetType: "decision" },
    { relationType: "contains", sourceType: "board_report", targetType: "action" }
  ]

  // AI framing for extraction
  aiGuidance: AiGuidance

  // Completeness rules — what does "done" look like?
  required: TypeRef[]    // document is incomplete without at least one Record of each
}
```

When you pass a transcript to a Schema, it has everything it needs: what to look for, how to extract it (from the Type/Field AI guidance), how the results should relate to each other, and what done looks like.

**The sign you noticed**

The reason it felt like "document template" is that you were reaching for the concept that sits *above* the view layer and *is* the definition. The spec has good machinery for extraction and good machinery for projection, but the thing that connects them — "here is what this document type fundamentally IS" — was implicit in the relationship between several other types rather than named and owned. Schema names it.

---

# you asked

message time: 2026-05-21 09:21:41

Now the next element is the "hidden" thinking steps that allowed me to get here. 

A decision record is the end of a process. The templates we designed allowed me to work through the semantic "build" process. 

Tell me the context, now explain why we need a decision, now what does good look like. What are your options, how do they all hold up against your expectations.... And only then... Great. .... What is the decision

---

# claude response

You've found the missing concept. And it's a significant one.

**What you're describing isn't a template or a view**

The facilitation steps in the current spec treat the process as "fill in these fields in this order." What you're pointing at is epistemically richer. Each stage isn't populating a field — it's building a piece of understanding that makes the next stage *possible and meaningful*.

You can't evaluate options meaningfully before you've stated what good looks like. You can't state what good looks like before you've established why a decision is needed. The stages have *epistemic dependencies*, not just sequential ordering. That's a fundamentally different structure.

**The Decision Record is compressed**

This is the key insight. The final Record contains: context, rationale, decision statement, options considered. But those fields only have *quality content* if the reasoning process was followed. The Record is the destination. The protocol is the path that guarantees you arrive with something trustworthy.

The "hidden" steps are hidden in the Record but they're not hidden in the process. They're their own semantic artifacts:

- Context → a Record
- Problem statement → a Record  
- Success criteria → a Record (the underrated step — most bad decisions skip this)
- Options → Records
- Evaluation → a Record (options × criteria)
- Decision → the final Record

The Decision Record doesn't contain the options analysis. It references it. The protocol run *produces a graph* of intermediate Records that the final Decision is derived from.

**This is the graduation story running in real time**

In the spec, graduation was framed as: rough freeform content formalises over time into proper Records. What you're describing is the protocol *generating* that progression live. Each stage produces intermediate semantic material — rough at first, then structured — and the final stage is the graduation event where all of it compresses into the authoritative Record.

The `derived-from` relationships aren't retrospective archaeology. They're the trace of the protocol itself.

**The concept: Protocol**

A Protocol is a first-class concept distinct from Schema, View, and Type.

```
Schema  — what a document consists of
View    — how to present Records for a purpose  
Type    — what fields one semantic object has
Protocol — the epistemically ordered process through which quality Records are built
```

A Protocol has:
- **Stages** with explicit epistemic dependencies (not just order)
- A **purpose** per stage — what understanding should exist when this stage is done
- A **completion criterion** per stage — how do you know it's good enough to proceed
- **Intermediate Record types** produced per stage
- **Contribution mapping** — which intermediate Records feed into which fields of the final Record
- AI guidance per stage that knows what has already been established

A Schema might have multiple valid Protocols. A quick individual decision and a full group deliberation both produce a Decision Record. The Record type is the same. The path is different.

**Why this couldn't be FacilitationStep**

`TemplateFacilitationStep` was a field-ordering mechanism dressed up as facilitation. It had no model of: what understanding this step builds, whether that understanding is sufficient before proceeding, or what semantic artifacts the step produces independently of the final Record. It treated facilitation as presentation. A Protocol treats facilitation as epistemology.

**The practical consequence**

When you run a Protocol against a Schema, you're not populating a form. You're conducting structured sense-making that produces a graph of Records with a clear derivation chain to the final authoritative one. The quality of the Decision Record is *auditable* — you can traverse `derived-from` relations back to the Options Evaluation record, the Success Criteria record, the Context record. You can see whether the criteria were actually applied to the options.

That auditability is exactly what a governance tool needs. And it falls out naturally once Protocol is a first-class concept rather than a feature of Views.

---

# you asked

message time: 2026-05-21 09:29:22

The other model is freeform. 

We need to work out how to achieve something.

"What does the process of designing a generative artist retreat for the cultivation of awe look like?"


Hmm
 Ok, first what are the big components?

We need to find the space
Organise the physical space
Invite and select artists 
Manage the practical elements
Design the programme

Ok, enough, what do these look like. 

To find a space we need to establish what we need, then work out the budgets.

Oh, and we might want a recording studio. We will need to get someone to set that up. 

Our final programme should end in a concert. Good, now let's go back to how we will do catering.


----

In practice this conversation evolves in stages and we gradually add structure

---

# claude response

You're showing two genuinely different epistemic modes. And they need different models.

**What's happening in that conversation**

The structure isn't being followed — it's being *discovered*. You don't start with "here are the components"; you start with a question and decompose it. The components emerge. Then you drill into one, discover something new inside it (recording studio), and it needs to find its place in the structure that didn't exist when you started. Then you jump somewhere else entirely.

This isn't a Protocol — a Protocol is *convergent*. You know you're building a Decision Record, the stages exist to get there. This mode is *divergent*. You don't know what you're building yet. The act of exploring IS the act of discovering what needs to be built.

And crucially — FreeformInstance doesn't handle it either. FreeformInstance assumes you have sections already. This conversation is discovering what the sections even are.

**The structure being built is a tree, not a document**

At any point in that conversation, what exists is something like:

```
Generative Artist Retreat
├── Find a space
│   ├── Establish requirements  ← being explored
│   └── Budget                  ← open
├── Organise physical space     ← open
├── Invite and select artists   ← open
├── Practical elements
│   └── Catering                ← being explored now
├── Programme design
│   └── Ends in a concert       ← established
└── Recording studio            ← appeared mid-conversation, needs placing
```

Each node has a status. Some are open questions. Some are being actively explored. Some have settled enough to become Records. The tree itself is navigable — you can jump to any node, drill down, come back. New nodes can appear at any depth.

This is an **Inquiry**. A living decomposition of an open question that gets progressively structured through conversation.

**How Inquiry relates to the other concepts**

| Mode | Starting state | Structure | Output |
|---|---|---|---|
| Inquiry | Open question | Emergent tree of components | Map of what needs to be built |
| Protocol | Known destination type | Epistemically ordered stages | A specific Record, with derivation chain |
| Extraction | Existing transcript | Schema-driven | Records |

The relationship between them is natural: an Inquiry discovers *what* needs to be resolved. A Protocol resolves *one node* of the Inquiry thoroughly. The Inquiry map eventually tells you what your Schema should contain.

In the retreat example: the Inquiry produces the tree. "Space selection" becomes a node developed enough to run a Protocol against — context, criteria, options, decision. "Recording studio" sits as an open node until someone decides it warrants the same treatment. The programme design develops iteratively through several conversations, pulling in transcripts.

**What Inquiry needs that nothing else provides**

- A *tree structure* not a flat section list — nodes can have children
- *Node status* — open, exploring, developed, delegated, parked
- *Non-linear navigation* — any node can be the current focus without the others being resolved
- *Emergent nodes* — something can appear at any depth and find its place
- *Selective graduation* — developed nodes graduate into Records independently, at their own pace, without the whole Inquiry being "done"
- *The tree itself as a persistent artifact* — not just the content of nodes, but the structure of how understanding was decomposed

The Inquiry IS the map of the territory. When it's done — or done enough — it tells you what your Schema should contain, what Protocols you need to run, and which Records already exist.

**The sign this points at in the spec**

FreeformInstance was positioned as "the entry point for progressive semantic adoption." But it's actually a *capture format* — it holds rough content that already has shape. The Inquiry is what happens *before* that — the generative process of discovering what shape the content should have. These are different enough to warrant distinct treatment.

The question for the spec: does Inquiry live inside SCDS, or is it the layer *above* SCDS that produces the material SCDS then structures? I lean toward it being inside — because the tree structure, node status, and derivation links from Inquiry nodes to eventual Records are exactly the kind of provenance SCDS cares about. But I'd hold that loosely.

---

# you asked

message time: 2026-05-21 09:35:03

Yes, I feel that the transcript and scds are not the same thing. I want them to reference each other. 

However there could be a general enquiry protocol... Even if it was just, let's do a brain dump. We are then abe to reference that in a let's define the major stages we are aware of protocol.

---

# claude response

Yes. And this clarifies the architecture quite cleanly.

**Transcript and SCDS are permanently separate layers**

The transcript is raw captured material. SCDS is negotiated semantic state extracted from it. They reference each other bidirectionally — a Record knows which transcript chunks it came from (SourceReference), a transcript chunk can be tagged with what SCDS context it was produced in. But neither owns the other. This isn't just an implementation detail — it's a first principle.

**"Brain dump" is a Protocol**

This is the key move. Even the most freeform process has Protocol structure, just very lightweight:

```
Brain Dump Protocol
  purpose: externalize all thinking on a topic without constraint
  method: open-ended, no structure imposed
  output: FreeformInstance (or Inquiry node)
  next: typically feeds into a Decomposition Protocol
```

```
Decomposition Protocol  
  purpose: from the raw material, identify the major components
  input: references Brain Dump output
  output: Inquiry tree, top-level nodes established
  next: one Protocol per node, as needed
```

The "general enquiry" you're describing is just these two Protocols chained. And the chain creates provenance — the Decomposition Protocol output references the Brain Dump output, which references its transcript. When you eventually make a decision about space selection, you can trace back through: Decision ← Space Selection Protocol ← Decomposition ← Brain Dump ← transcript.

**Protocol exists on a spectrum**

Rather than two modes (exploratory vs. convergent), there's one concept with a spectrum:

```
loose ←————————————————————————→ tight

Brain    Decompose   Prioritise   Options    Decision
Dump                             Analysis
```

Loose protocols produce open-ended material. Tight protocols converge on a specific Record type. The output of a loose protocol is the input context for something tighter. The whole spectrum is Protocol.

**What falls out: a Protocol library**

Generic protocols — reusable across any domain:
- Brain Dump
- Decomposition ("what are the major components?")
- Prioritisation ("which of these matters most right now?")
- Review ("what have we established so far, what's still open?")

Domain-specific protocols — target a specific Record type:
- Decision (context → criteria → options → evaluation → decision)
- Space Selection
- Budget
- Programme Design

The governance tool you're building is mostly a library of domain-specific Protocols, plus the generic ones that feed into them. The generic ones handle the exploratory/inquiry mode. The domain-specific ones handle convergence.

**The practical consequence**

You don't need a separate "Inquiry" concept distinct from Protocol. An inquiry *is* a sequence of generic Protocols — brain dump, decompose, maybe prioritise — applied to an open question before you know what domain-specific Protocols you'll need. The structure of the inquiry emerges from running those generic Protocols, and the Inquiry tree is just the accumulated output.

This also means the facilitation tool is simpler to reason about: it's a Protocol runner that can chain outputs, reference transcripts as source material, and knows which Protocol library entries are available. The intelligence is in the Protocol definitions, not in the runner.

---

# you asked

message time: 2026-05-21 09:38:08

This fits the origins of the current software.
This is the muDemocracy decision logger. The muDemocracy AI layer is the democracy coach...it aids a group in learning protocols for clear collaboration

---

# claude response

That reframes everything, and it all coheres.

**The layering becomes clear**

```
Democracy Coach (AI)     ← facilitates, teaches, adapts to group capability
μDemocracy Logger        ← captures structured semantic state (built on SCDS)
SCDS                     ← the semantic foundation, domain-neutral
Transcript layer         ← raw captured material, referenced but separate
```

The logger is the record-keeper. The coach is the process guide. They're doing different things and need to stay separate — the coach drives conversations that the logger captures, not the other way around.

**What "learning protocols" adds that "running protocols" doesn't**

A Protocol runner executes a process and produces a Record. A democracy coach does that AND tracks whether the group is developing capability. That's a different thing entirely.

A group using μDemocracy should eventually be able to run a Decision Protocol without the coach prompting every stage. The coach needs to know: has this group done this Protocol before? Did they skip the success criteria stage last time? Are they ready for a more complex variant?

That's not stored in SCDS Records — it's a capability model sitting alongside them. The Records are evidence the coach uses to assess capability, but the capability assessment is its own layer.

**The Protocol library is the democracy curriculum**

The generic protocols — brain dump, decomposition, check-in — are the entry level. Groups learn those first. Then consent-based decision making. Then more complex governance structures. The coach knows the learning sequence and where a group is within it.

This means Protocols aren't just process definitions. In the μDemocracy context they're also *curriculum objects* — they have prerequisites, they have a place in a learning arc, and completing them well enough to self-facilitate is a measurable milestone.

**The founding documents are the graduation artefact**

A group that has run enough Protocols — purpose-setting, value-definition, governance structure, decision-making method — has produced the raw material for its founding documents. The Document Schema for a founding document is essentially a map of which Protocols a group needs to have completed, and at what quality, before they can legitimately say they've constituted themselves.

The founding document isn't filled in. It's *assembled from Protocol outputs* that the group has genuinely worked through. That's a meaningful difference — it's constitutive, not administrative.

**The thing this reveals about the coach's role**

The democracy coach isn't trying to make itself indispensable. Its job is progressive handover — scaffold the process until the group has internalised it, then step back. That's a specific kind of AI design that most tools don't attempt. The coach succeeds when it's needed less.

Which means the coach needs to know when to scaffold and when to trust the group to run their own process and just watch. That's probably the hardest design problem in the whole system.

---

# you asked

message time: 2026-05-21 09:46:53

For now, I solve the coach problem by supporting a facilitator.

---

# claude response

That's the right call for v1. And it's not a compromise — it's the correct architecture for now.

Judgment, group-reading, relationship — those stay with the human. The AI handles what it's actually good at: structure, memory, extraction, and reducing cognitive load so the facilitator can be present with the group rather than managing process mechanics.

**What "supporting a facilitator" means concretely**

Before a session — the AI knows where the group is:
- Which Records are established, which are still open
- Which Protocols have been run and how complete they felt
- What the appropriate next step is, with options

During a session — the AI runs the scaffolding:
- Holds the Protocol sequence so the facilitator doesn't have to
- Flags when a stage seems thin before moving on
- Captures the transcript

After a session — the AI does the extraction work:
- Pulls Records from the transcript
- Updates the Inquiry tree
- Shows what's been established and what's still open

The facilitator makes every call. The AI makes sure no information is lost and no stage is accidentally skipped.

**The hidden benefit**

A human facilitator using these support tools is the prototype for what the democracy coach eventually becomes. Every time a facilitator decides "this group is ready for a harder Protocol" or "we need to go back and redo the criteria stage," that's a judgment the system can learn from. You don't have to solve the coach problem now — you gather the data to solve it later by watching what good facilitators do with the tools.

The facilitator-support version ships. The democracy coach emerges from it.

---

# you asked

message time: 2026-05-21 09:49:36

Now, let's create a remapping doc for scds. Providing some new names and showing how current elements fit the new iteration

---

# claude response

A few things worth noting about the decisions in the doc:

The μDemocracy mapping at the end (Section 7) felt important to include — it grounds the abstract renaming back in the actual application. Worth checking that the Type list there matches what you're actually building: Decision, Proposal, Action, Role, Value, Principle are my guesses at your core governance Types.

The Schema / Document View distinction (the table in Section 2) is probably the most important conceptual clarification in the whole remapping. Those two concepts were doing overlapping work in v1 and the rename forces them apart.

`TemplateFacilitationStep` disappearing entirely into Protocol Stage is a clean deletion — nothing is lost, the concept just lives somewhere more honest now. Worth flagging this explicitly when you revise the actual spec so it doesn't look like an accidental omission.

One thing I left open: whether `View` at L1 should also be split — there's a "facilitator view" and a "participant view" and an "export view" that might want to be distinguished. Held back from going there since you have an immediate v1 to build.

---

# you asked

message time: 2026-05-21 10:11:32

Here's a potentially missing piece. Addressability. 

There are three components to this in practice

* the document
* The conversation
* Attention

SCDS defines the document as a sementic object
The conversation is multimodal source material. It begins with meeting transcript. It could be online threaded conversation. It could be comments on text by a web ui. 

But all of these elements need to exist within a system. The attention mechanism links the contextual conversation to a specific element within the system. A transcript chunk needs to be adderressable, as must a specific revision of a field. 

a protocol will guide conversation from field to field, and we need to be able to query the system for relevant context - this particular instance of the field, a previous instance, this document

---

# claude response

You're identifying the connective tissue between the three layers. Without it, the transcript/SCDS separation is clean in principle but broken in practice — you have no mechanism to say "this conversation was about this thing."

**The three sub-problems**

These are related but distinct:

**Addressability** — every element in the system has a stable, resolvable identifier. Not just Records and Fields, but revisions of fields, chunks of conversation, stages of a Protocol run. If it can be referred to, it needs an address.

**Attention** — during a Protocol run there is a current focus: which Container, which Record, which Field, which Stage. Conversation material is tagged with that focus *as it's produced*, not retrospectively. This is the active mechanism.

**Context Assembly** — given an address and a purpose, retrieve the relevant material. Before filling a Field, assemble: current value, revision history, conversation chunks tagged to this Field, conversation chunks tagged to the parent Record, related Records. This is the query that feeds AI assistance.

They build in sequence. Attention tagging only works if everything has an address. Context assembly only works if attention was captured at production time.

---

**The address space**

Everything in the system needs to be co-addressable — a field revision and a transcript chunk need to live in the same reference space because you need to say "this chunk produced this revision."

```
Document space:
  Container / {containerId}
    Record / {recordId}
      Field / {fieldId}
        Revision / {revisionId}

Process space:
  Protocol Run / {runId}
    Stage / {stageId}

Conversation space:
  Session / {sessionId}
    Chunk / {chunkId}
      Annotation / {annotationId}   ← web UI comments, inline notes
```

The attention state at any moment is just a cursor across this space:

```
AttentionState {
  containerId:    UUID
  recordId?:      UUID
  fieldId?:       UUID
  protocolRunId?: UUID
  stageId?:       string
}
```

Every conversation element produced while attention is in a state gets that state as a tag. This is what makes context assembly possible later — you can query "all chunks produced while focus was on this Field."

---

**Revision as first-class**

This is the piece currently missing from the spec. Field revision is treated as an implementation concern (in-place edit vs. new Record) but for context assembly it needs to be addressable. You need to be able to ask:

- What did this field say before the last Protocol run?
- Which conversation produced the change from revision 2 to revision 3?
- What was the full state of this Record at the point the founding document was ratified?

A Revision needs: a stable address, the value at that point, the agent (human/AI), the timestamp, and the source references to the conversation that produced it.

---

**The multimodal conversation cases**

The three conversation types you name have different structures but all need the same addressing property — an anchor in the document space:

| Type | Structure | Anchor |
|---|---|---|
| Meeting transcript | Linear, time-ordered chunks | Tagged with AttentionState at production time |
| Threaded conversation | Tree of replies | Each thread root anchored to a document element |
| Web UI comments | Annotations on content | Attached to a specific Field, or sub-field text selection |

The web UI case is the most granular — a comment on specific text within a Field value. That's sub-field addressing, which requires richer identifiers. For v1 I'd scope to Field-level and Record-level addressing and treat sub-field as a later extension. The architecture accommodates it; you don't need to implement it first.

---

**What this adds to the spec**

Three new concepts:

**`Address`** — the universal identifier format across document, process, and conversation space. A stable path to any element. This is the foundation.

**`AttentionState`** — the current focus of an active Protocol run. Captured by the session, attached to outgoing conversation elements. The mechanism that connects live conversation to the document it's building.

**`Revision`** — a first-class, addressable snapshot of a Field value at a point in time. Carries value, agent, timestamp, and source references. Currently implicit; needs to be explicit for the context assembly queries to work.

**`ContextQuery`** — probably sits above the schema layer as an API concern rather than a schema type. But it needs to be specified: given an address + query purpose (fill this field / review this record / trace this revision), what gets assembled and in what order.

---

**The mechanism in operation**

During a Protocol run: the session holds an AttentionState. As conversation is produced — transcript chunks, threaded comments, inline annotations — each element is stamped with the current AttentionState. The Protocol advances attention from stage to stage, field to field.

When context assembly runs before filling a Field: it queries by address. Chunks tagged to this Field. Chunks tagged to the parent Record. Revision history for this Field. Related Records via Relation traversal. The AI guidance from the Type and Field definitions. All of it assembled in priority order.

The transcript/SCDS boundary stays clean. The address space and the attention mechanism are what make it *operational* rather than just architecturally correct.

---

# you asked

message time: 2026-05-21 10:24:16

lets update our remap doc

---

# claude response

Key things added:

**Four new concepts** under a new Addressability group — Address, AttentionState, Revision, Context Query. They're treated as a coherent group because they only make sense together: Address is the foundation, AttentionState uses it live, Revision makes history queryable by it, Context Query assembles across it.

**AttentionState vs SourceReference table** — this felt important to make explicit. They're both about connecting conversation to document, but in opposite directions and at different times. Without the table it's easy to conflate them.

**Transcript layer → Conversation layer** — the rename and expansion to cover threaded conversation and web UI comments. Sub-field addressing flagged as deferred but architecturally accommodated.

**Context Query scoped as a behavioural requirement** rather than a schema type — it's what a conforming implementation must produce, not a data shape it must conform to. That distinction matters when thinking about what the spec actually mandates.

One thing to check: Addressability is listed as an Extension in the table, but you could argue Address and AttentionState belong in Core for any live facilitation system. Worth a decision on whether μDemocracy v1 treats these as core or extension.

---

# you asked

message time: 2026-05-21 11:02:55

# SCDS Specification

**Version**: 2.0-draft
**Status**: active draft
**Scope**: field definitions (Field), type definitions (Type), records (Note / Typed Record / Record), relations, containers, distribution, and eleven optional extensions covering addressability, lifecycle, protocol, schema, views, repeatable fields, field groups, cross-field validation, recommended relations, import tracking, and registry.

> **Migration note**: This document supersedes `scds-schema.md` (v1.0-draft). A vocabulary and structural mapping from v1 to v2 is in `scds-shema-evolution.md`. Design rationale, usage guidance, and commentary are in `scds-rationale.md`.

---

## 1. Purpose and Scope

### What this specification defines

The Semantic Collaborative Document System (SCDS) specification defines an interoperable standard for semantic field and type definitions, records, relations, and the mechanisms by which these artefacts are created, shared, versioned, and distributed across independent implementations.

This specification covers:

- **Field** — atomic reusable semantic unit
- **Type** — named composition of fields for a specific semantic object type
- **Record** — instantiated type with field values; three semantic maturity tiers (Note, Typed Record, Record)
- **Relation** — first-class typed link between records
- **Container** — grouping boundary for record collections
- **Distribution** — Package, Reference, Lineage, Provenance
- **Extensions** — optional, independently adoptable capabilities declared by conforming implementations

### What this specification does not define

- **Session** — live collaborative process model (future version)
- **Registry protocol** — how registries communicate, authenticate, or federate; this specification defines data shapes only
- **Universal semantic ontology** — domain-specific vocabularies are the responsibility of namespace authors

### Relationship to implementing systems

This specification is implementation-neutral. Implementations are expected to validate inputs against these schemas at their system boundaries. The specification does not constrain persistence technology, API design, UI rendering, or prompt assembly strategy.

### Extension conformance model

Implementations declare conformance as:

```
SCDS Core [+ ext:<name> ...]
```

**Core** requires the Foundation group and Distribution group in full. No extension is required for core conformance. Extensions are independently adoptable; some declare dependencies on other extensions.

| Extension | Identifier | Depends on |
|---|---|---|
| Addressability | `ext:addressability` | — |
| Lifecycle | `ext:lifecycle` | — |
| Protocol | `ext:protocol` | `ext:lifecycle` (recommended) |
| Schema | `ext:schema` | — |
| Views L1 | `ext:views-l1` | — |
| Views L2 | `ext:views-l2` | `ext:views-l1` |
| Repeatable Fields | `ext:repeatable-fields` | — |
| Field Groups | `ext:field-groups` | — |
| Cross-Field Validation | `ext:cross-field-validation` | — |
| Recommended Relations | `ext:recommended-relations` | — |
| Import Tracking | `ext:import-tracking` | — |
| Registry | `ext:registry` | — |

Example declaration: `SCDS Core + ext:lifecycle + ext:protocol + ext:views-l1 + ext:addressability`

---

## 2. Namespace Format

### Convention

Namespaces are dot-separated identifiers using lowercase alphanumeric characters and hyphens.

```
<component>[.<component>]*

component = [a-z0-9][a-z0-9-]*
```

Examples:
```
core
community.adr
com.acme.hr
org.cooperative-name
```

### Reserved namespaces

`core` is reserved for definitions maintained by the SCDS standard. Implementations must not allow user-created definitions in the `core` namespace.

### Reference format

A specific version of a definition is referenced using the canonical form:

```
namespace/name@version
```

Examples:
```
core/decision_statement@2
community.adr/review_rationale@1
com.acme.hr/headcount_impact@3
```

The `/` and `@` characters are reserved separators. They must not appear within a namespace component or a name.

### Name convention

Field and Type names are programmatic keys in `snake_case`. Names are stable within a namespace and version lineage. A new name means a new definition.

---

## 3. Schema Notation

Types are described using TypeScript-style notation. Optional fields are marked with `?`. All `UUID` values are RFC 4122 UUID strings. All `ISO8601` values are datetime strings with timezone offset. `integer` means a positive integer unless otherwise noted.

### Version semantics

Version numbers are positive integers scoped to a definition's UUID lineage.

| Change | Version action |
|---|---|
| Documentation, typo, formatting only | Optional bump |
| `description`, `instructions`, or `aiGuidance.purpose` reworded without semantic change | Minor bump recommended |
| `aiGuidance.extraction` or `aiGuidance.purpose` changed in meaning | Version bump required |
| `valueType`, `selectOptions`, or `validationRules` changed | Version bump required |
| `name` changed | New definition required (new UUID) |
| `namespace` changed | New definition required (new UUID) |

When in doubt: if a downstream consumer's AI extraction, validation, or governance logic would behave differently, a version bump is required.

---

## 4. Foundation Group (Core)

The Foundation group is required for all conforming implementations.

### 4.1 Supporting types

#### `ValidationRule`

A constraint applied to a field value.

```typescript
{
  type: "required" | "minLength" | "maxLength" | "pattern" | "enum"
  value?: string | number | string[]  // required for minLength, maxLength, pattern, enum
  message?: string
}
```

#### `AiGuidanceExample`

A single example for AI guidance.

```typescript
{
  description?: string  // labels this example
  input?: string        // sample source text; omit for output-only examples
  output: string        // the ideal value the AI should produce
}
```

`output` is required. An example without `input` demonstrates expected output form without requiring a specific source.

#### `AiGuidance`

Structured AI guidance for a Field or Type.

```typescript
{
  purpose: string            // what this field/type captures (1-2 sentences)
  extraction?: string        // LLM instruction for how to extract or populate
  negativeGuidance?: string  // what the LLM must NOT include or do
  examples?: AiGuidanceExample[]
}
```

The minimum valid `AiGuidance` is `{ purpose: "..." }`.

---

### 4.2 Field

The atomic reusable semantic unit. Fields are defined once and composed into Types. A Field's `aiGuidance`, `validationRules`, and `valueType` belong to the Field, not to any Type that includes it.

```typescript
{
  // Stable identity
  id: UUID
  namespace: string
  name: string       // snake_case programmatic key
  version: integer   // min: 1; increments within this id's lineage

  // Semantic content
  description: string      // one-sentence user-facing summary
  instructions?: string    // fuller guidance for a human completing this field
  aiGuidance: AiGuidance

  // Value semantics — stable across renderers
  valueType: "string" | "text" | "number" | "boolean" | "date" | "url" | "select" | "multiselect"
  selectOptions?: string[]   // required when valueType is "select" or "multiselect"
  validationRules?: ValidationRule[]

  // Editor hint — projection-specific default; implementations and Views may override
  editorHint?: "singleline" | "textarea" | "rich-text" | "date-picker" | "dropdown" | "multi-select" | "voice"

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: Lineage      // see Distribution group
  provenance?: Provenance
}
```

**`valueType` semantics:**

| Value | Meaning |
|---|---|
| `"string"` | Short single-value text (typically one line) |
| `"text"` | Potentially long multi-paragraph prose |
| `"number"` | Numeric value |
| `"boolean"` | True/false |
| `"date"` | ISO 8601 date or datetime |
| `"url"` | A URL string |
| `"select"` | One value from `selectOptions` |
| `"multiselect"` | One or more values from `selectOptions` |

`valueType` is the stable semantic data type. `editorHint` is a rendering default. AI extraction, validation, and export formatting must depend only on `valueType`.

---

### 4.3 Type

A named, versioned composition of Fields for a specific semantic object type.

```typescript
{
  // Stable identity
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  // Content
  description: string        // when to use this Type; what semantic object it defines
  aiGuidance?: AiGuidance    // Type-level LLM framing; see AI guidance composition in rationale

  // Semantic object type (optional, informative)
  semanticObjectType?: string
  // e.g. "decision", "task", "risk", "budget_line", "requirement"
  // Free-form. Implementations may use as a rendering or grouping hint.
  // No conforming implementation is required to act on it.

  // Composition
  fields: FieldAssignment[]
  // fieldGroups and validationRules are extensions; see ext:field-groups and
  // ext:cross-field-validation

  // lifecycle is an extension; see ext:lifecycle

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

#### `FieldAssignment`

A Field reference within a Type. Configures presentation without redefining field semantics.

```typescript
{
  fieldId: UUID     // references Field.id
  order: integer    // min: 0; display and processing order within the Type
  required?: boolean  // default: true

  // Presentation-only — must NOT affect AI guidance, extraction, valueType, or validation
  displayLabel?: string
  displayHint?: string
}
```

`displayLabel` and `displayHint` are strictly for rendering. If a materially different label or meaning is needed, a distinct Field with its own lineage is required.

Repeatability fields (`repeatable`, `minItems`, `maxItems`) are defined in `ext:repeatable-fields`.

**AI guidance composition order** (recommended):

1. Type framing (`Type.aiGuidance.extraction`) — establishes the semantic object type
2. View framing (`View.aiGuidance.extraction`, if `ext:views-l1` is in use) — workflow-specific context
3. Field extraction guidance (`Field.aiGuidance.extraction`)
4. Negative guidance (`Field.aiGuidance.negativeGuidance`)
5. Examples (`Field.aiGuidance.examples`)

This is a recommended default, not a required invariant. Implementations that compose differently will produce different AI behaviour from the same definitions.

**On instance migration when a Type version changes:**
A Record binds to a specific `typeVersion` at creation time. Existing Records do not automatically migrate when a new Type version is published. Conformance is measured against the version the Record was instantiated under. When a Record is migrated and exchanged, it should carry the version it now conforms to, and the original Record should be preserved and linked via a `supersedes` Relation.

---

### 4.4 Record tiers

SCDS supports three semantic maturity tiers. Implementations are not required to support all three; they may begin at Tier 2.

| Tier | Type | Structure | Semantics |
|---|---|---|---|
| **0** | `Note` | Named sections + free text | None |
| **1** | `Typed Record` | Named fields with types and values | Minimal |
| **2** | `Record` | Fields bound to a `Type` definition | Full |

Graduation path: Note → Typed Record → Record.

#### `NoteSection`

A named text section within a Note.

```typescript
{
  name: string          // section key; unique within the Note; snake_case recommended
  label?: string
  content: string
  contentHint?: "text" | "markdown" | "plain"  // hint only; default: "text"
}
```

#### `Note`

A lightweight instance with no Type binding.

```typescript
{
  instanceId: UUID

  title?: string
  sections: NoteSection[]

  graduatedAt?: ISO8601
  // When set, signals full formalisation. Authoritative record of successors
  // is in derived-from Relations from the successor Records.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

#### `TypedField`

A field within a Typed Record.

```typescript
{
  name: string
  label?: string
  valueType?: "string" | "text" | "number" | "boolean" | "date" | "url" | "select" | "multiselect"
  selectOptions?: string[]
  value: string | number | boolean | string[] | null
  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
}
```

#### `Typed Record`

A structured instance with named, typed fields but no Type binding.

```typescript
{
  instanceId: UUID

  title?: string
  instanceType?: string  // lightweight semantic hint; not a formal type declaration

  fields: TypedField[]

  graduatedAt?: ISO8601

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

#### `SourceReference`

A pointer from a field value or instance back to source material.

```typescript
{
  sourceType: "transcript-chunk" | "transcript-segment" | "external-document"
  sourceId: string
  sourceStandard?: string   // versioned standard the source conforms to
  streamId?: UUID           // for transcript sources: originating stream

  relationType?: "evidence" | "derived-from" | "quoted-from" | "inspired-by" | "supersedes-context"

  confidence?: number       // 0.0–1.0
  note?: string
}
```

#### `FieldValue`

The current value of a Field within a Record.

```typescript
{
  fieldId: UUID

  // Non-repeatable — use value
  value?: string | number | boolean | string[] | null

  // Repeatable — use entries (ext:repeatable-fields)
  entries?: FieldValueEntry[]

  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601

  sourceRefs?: SourceReference[]
}
```

`FieldValueEntry` is defined in `ext:repeatable-fields`.

#### `Record`

An instantiated Type with field values.

```typescript
{
  instanceId: UUID
  typeId: UUID         // references Type.id
  typeVersion: integer
  typeNamespace: string
  typeName: string

  // lifecycleState is ext:lifecycle
  lifecycleState?: string

  fieldValues: FieldValue[]

  // groupValues is ext:field-groups
  groupValues?: FieldGroupValue[]

  sourceRefs?: SourceReference[]

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
  // Use meta for implementation-local concerns: lock state, visibility,
  // session references. Cross-system keys should be namespaced,
  // e.g. "com.acme.locking.locked-by".
}
```

`typeNamespace` and `typeName` are denormalised convenience fields. If they conflict with the resolved Type, the `typeId`/`typeVersion` identity takes precedence and the Record is considered invalid until corrected.

**On instance revision:**
- **In-place edits** (`updatedAt` advances, `fieldValues` mutate): for minor corrections that do not alter semantic meaning.
- **Semantic updates**: produce a new Record linked to the prior by a `supersedes` or `refines` Relation. The prior Record remains valid.
- **Immutable records + Relation graph**: all Records append-only; a new Record for every change. A valid implementation strategy that naturally preserves history.

---

### 4.5 Relation

A first-class typed link between instances. Relations allow implementations to construct semantic graphs for navigation, analysis, projection, and reasoning.

```typescript
{
  relationId: UUID

  relationType: string
  // Free-form. See ext:recommended-relations for canonical types and conventions.

  // source [relationType] target
  sourceInstanceId: UUID    // the asserting instance
  targetInstanceId: UUID    // the related instance

  assertedBy?: "human" | "ai" | "imported"
  confidence?: number       // 0.0–1.0; meaningful for ai-asserted
  createdAt?: ISO8601
  createdBy?: string

  status?: "proposed" | "active" | "rejected" | "superseded"
  validFrom?: ISO8601
  validUntil?: ISO8601

  notes?: string
  sourceRefs?: SourceReference[]
  meta?: Record<string, unknown>
}
```

**Directionality convention:**
`sourceInstanceId` is the asserting instance; `targetInstanceId` is the related instance. The Relation reads: "source [relationType] target."

| Relation | source | target |
|---|---|---|
| `supersedes` | the newer Record | the older Record |
| `contains` | the stage | the task inside it |
| `depends-on` | the dependent task | the task it needs |
| `refines` | the detailed version | the rough version |
| `derived-from` | the successor | the source Note or Record |
| `evidences` | the source material | the claim it supports |

This convention must be consistent across implementations. See Invariant 16.

Relations span tiers. A Note may be the target of `derived-from` Relations from the Records it graduated into.

**Canonical relation types** (use these exact strings for cross-system interoperability):

`contains`, `depends-on`, `supersedes`, `refines`, `derived-from`, `evidences`, `precedes`

Custom types not covered by these should use `namespace/name` format (e.g. `com.acme.hr/transferred-to`) to prevent collision. Extended relation type metadata is defined in `ext:recommended-relations`.

**Relations do not change lifecycle state.** A `supersedes` Relation does not mutate the prior Record's `lifecycleState`. Lifecycle state changes are explicit acts by an implementation's transition mechanism.

---

### 4.6 Container

A lightweight grouping boundary over a collection of instances. Containers answer scoping questions — which instances belong together, what constitutes "this project" — that the Relation graph alone cannot answer.

Containers are not semantic objects with Fields. They do not own semantic state; Records do. A `contains` Relation asserts "A is part of B" (a semantic claim); a Container asserts "these instances form a unit for boundary purposes" (a scope claim). Both are needed; neither replaces the other.

```typescript
{
  containerId: UUID

  namespace?: string
  name?: string

  title: string              // human-readable label

  containerType?: string     // free-form hint; e.g. "project", "meeting", "sprint"

  rootInstanceIds?: UUID[]
  // Top-level instances this Container was created to hold. Implementations may
  // derive nested members by traversing contains Relations from these roots.

  memberInstanceIds?: UUID[]
  // Explicit membership list for all instances in scope.
  // When present, allows membership queries without graph traversal.
  // When omitted, membership is defined by traversing contains Relations.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

`Container.containerId` is not an instance ID and must not appear in `Relation.sourceInstanceId` or `targetInstanceId`. See Invariant 19.

---

## 5. Distribution Group (Core)

The Distribution group is required for all conforming implementations.

### 5.1 Package

The distributable artefact. Contains Field, Type, View, and Relation type definitions with a complete dependency manifest.

```typescript
{
  schemaVersion: string      // SCDS spec version, e.g. "2.0"
  packageId: UUID
  packageName: string
  packageVersion: string     // semver, e.g. "1.2.0"
  publishedAt: ISO8601
  publisher?: string
  description?: string
  homepage?: string

  // Content (at least one of fields or types must be non-empty)
  fields: Field[]
  types: Type[]
  views?: View[]             // ext:views-l1; omit if not in use
  schemas?: Schema[]         // ext:schema; omit if not in use
  protocols?: Protocol[]     // ext:protocol; omit if not in use
  relationTypes?: RelationTypeDefinition[]  // ext:recommended-relations

  mode: "bundled" | "standalone"

  dependencyRefs: Reference[]
}
```

**`mode` semantics:**

| Mode | Meaning |
|---|---|
| `"bundled"` | All Field records referenced by any Type, and all Type records referenced by any View, are included in `fields[]` and `types[]`. Self-contained. |
| `"standalone"` | Dependencies are expected pre-installed in the consumer's registry. `dependencyRefs` is the required manifest. |

`dependencyRefs` is required in both modes. Consumers use it to validate completeness without parsing content internals.

---

### 5.2 Reference

A stable pointer to a specific definition version.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1
  definitionType?: "field" | "type" | "view" | "schema" | "protocol"
}
```

Canonical string form: `namespace/name@version`

---

### 5.3 Lineage

Upstream and fork tracking for a specific definition version.

```typescript
{
  sourceDefinitionId?: UUID     // UUID of the upstream definition
  sourceVersion?: integer       // upstream version at derivation time
  forkedFromDefinitionId?: UUID // UUID of the definition deliberately forked from
  forkedFromVersion?: integer   // version at the fork point
}
```

| Field pair | Meaning |
|---|---|
| `sourceDefinition*` | Tracked copy; consumer expects upstream updates |
| `forkedFrom*` | Deliberately diverged; no upstream tracking |

Both may be present during a transition from tracking to forking.

---

### 5.4 Provenance

Publisher and package origin metadata.

```typescript
{
  publisher?: string        // namespace or org of the original author
  sourcePackage?: string    // package name that bundled this definition
  packageVersion?: string   // semver of the source package
  importedAt?: ISO8601
}
```

`packageVersion` is distinct from `Field.version`. A package at `1.3.0` may contain `decision_statement@3` and `context@2`.

---

## 6. Conversation Layer

The conversation layer is a permanent architectural boundary distinct from SCDS. It captures raw multimodal source material; SCDS captures negotiated semantic state. They reference each other bidirectionally via `SourceReference` (document → conversation) and `AttentionState` tags (conversation → document, via `ext:addressability`).

```
Conversation layer  →  raw multimodal source material (speech, threads, annotations)
                        elements tagged with Address at production time
Protocol layer      →  structures the facilitation process; advances AttentionState
SCDS layer          →  captures negotiated semantic state; Records carry SourceReferences
Presentation layer  →  renders SCDS state via Views
```

Three conversation types are in scope:

| Type | Structure | Anchoring |
|---|---|---|
| Meeting transcript | Linear, time-ordered chunks | Tagged with AttentionState at production time |
| Threaded conversation | Tree of replies | Thread root anchored to a document element Address |
| Web UI annotations | Attached to content | Anchored to a Field or Record Address |

Transcript chunks referenced in `SourceReference` are source material — addressable evidence. They do not become Notes or Records automatically. A transcript chunk referenced in `sourceRefs` is evidence supporting a field value; it is not itself a Note unless someone deliberately models it as one.

---

## 7. Extensions

Extensions are optional, independently adoptable. Each extension section declares its identifier, dependencies, and the types it defines.

---

### ext:addressability

**Required for**: any implementation with live facilitation or multi-session extraction.

Defines a universal addressing scheme and the mechanisms that connect conversation material to document elements.

#### `Address`

A stable, resolvable identifier for any element across document space, process space, and conversation space.

```typescript
type Address =
  | {
      space: "document"
      containerId: UUID
      recordId?: UUID
      fieldId?: UUID
      revisionId?: UUID    // requires ext:addressability Revision
    }
  | {
      space: "process"
      runId: UUID          // Protocol run ID; requires ext:protocol
      stageId?: string
    }
  | {
      space: "conversation"
      sessionId: UUID
      chunkId?: UUID
      annotationId?: UUID
    }
```

Every element that can be referred to has an Address. A transcript chunk and a field Revision are co-addressable because assertions about one referencing the other require both to be resolvable.

#### `AttentionState`

The current focus of an active Protocol run. Conversation material is tagged with the active `AttentionState` as it is produced. This makes context assembly efficient: "all chunks produced while focus was on this Field" is a queryable address predicate.

```typescript
{
  containerId: UUID
  recordId?: UUID
  fieldId?: UUID
  protocolRunId?: UUID
  stageId?: string
}
```

`AttentionState` is set live by the session or Protocol runner. `SourceReference` is set retrospectively at extraction or editorial review time. Both are needed; they answer different questions.

#### `Revision`

A first-class, addressable snapshot of a `FieldValue` at a point in time. Carries the value, the agent, a timestamp, and source references to the conversation that produced the change.

```typescript
{
  revisionId: UUID
  fieldId: UUID
  recordId: UUID

  value: FieldValue
  agent: "human" | "ai" | "imported"
  createdAt: ISO8601

  sourceRefs?: SourceReference[]
  priorRevisionId?: UUID  // chain to the previous Revision for this field
}
```

Revision does not replace the edit-in-place vs. new-Record judgment. Minor corrections remain in-place edits at the implementation layer. Revision is the addressable audit trail for interoperability — it makes field history queryable: "what did this field say before the last Protocol run?", "which conversation produced the change from revision 2 to revision 3?"

#### Context Query (behavioural requirement)

A conforming `ext:addressability` implementation must be able to assemble relevant material given an address and a purpose. This is a behavioural requirement, not a data shape.

**Required query patterns:**

| Pattern | Address | Returns |
|---|---|---|
| Field context | `{recordId}/{fieldId}` | Current value, Revision history, chunks tagged to this Field, Field `aiGuidance` |
| Record context | `{recordId}` | All field values, chunks tagged to this Record, Relations, Protocol run history |
| Stage context | `{runId}/{stageId}` | All chunks produced during this stage, Fields active in this stage |
| Revision trace | `{fieldId}/{revisionId}` | Value at that Revision, the conversation that produced it, prior Revision chain |

**Recommended assembly order for AI assistance:**

1. Type and Field `aiGuidance` — what this field captures, how to extract it
2. Current value and recent Revision history — what has already been established
3. Chunks tagged to this Field via AttentionState — most focused context
4. Chunks tagged to the parent Record — broader session context
5. Related Records via Relations — structural context

---

### ext:lifecycle

**Required for**: governance tools, decision logs, any implementation where records progress through defined states.

Adds lifecycle state declarations to `Type` and lifecycle state tracking to `Record`.

#### `LifecycleState`

```typescript
{
  name: string
  description?: string
  isInitial?: boolean   // valid starting state for new Records
  isFinal?: boolean     // no transitions out; Record is settled
}
```

#### `LifecycleTransition`

```typescript
{
  name: string       // e.g. "promote", "approve", "supersede"
  from: string       // must match a state name in the enclosing lifecycle
  to: string
  description?: string
}
```

#### Type lifecycle block (added by this extension)

When `ext:lifecycle` is in use, `Type` gains:

```typescript
lifecycle?: {
  states: LifecycleState[]           // min 1 state
  transitions: LifecycleTransition[]
  initialState: string               // must reference a state name where isInitial === true
}
```

#### Record lifecycle state (added by this extension)

`Record.lifecycleState` becomes meaningful: must match a state name in the associated `Type.lifecycle.states[]` when the Type declares a lifecycle.

The `lifecycle` block declares vocabulary. Implementations decide enforcement strictness. A state with `isFinal: true` signals that no further transitions are expected; implementations may use this to lock Record content.

---

### ext:protocol

**Required for**: facilitation tools, structured deliberation, any implementation that guides users through epistemic stages.

Replaces `TemplateFacilitationStep` from v1. Protocol is epistemically richer: stages have explicit dependencies, completion criteria, and may produce intermediate Records.

#### `TypeRef`

A reference to a specific Type, used within Protocol and Schema.

```typescript
{
  typeId: UUID
  typeVersion?: integer
}
```

#### `FieldRef`

A reference to a Field within a Type.

```typescript
{
  fieldId: UUID
  typeId?: UUID    // which Type this Field appears in
}
```

#### `ProtocolStage`

A named stage in a Protocol. Stages have epistemic dependencies (`dependsOn`) — not just ordering. A stage may only proceed when its dependencies are sufficient.

```typescript
{
  stageId: string       // stable key within this Protocol
  order: integer        // min: 0; sequence position
  purpose: string       // what understanding this stage builds
  question: string      // the core question this stage answers
  dependsOn: string[]   // stageId values; epistemic dependencies, not just ordering
  completionCriteria: string   // how to know this stage is sufficient to proceed
  contributesTo: FieldRef[]    // which Record Fields this stage feeds
  outputType?: TypeRef         // if this stage produces its own intermediate Record
  aiGuidance: AiGuidance
}
```

#### `Protocol`

An epistemically ordered process for building quality Records through structured conversation or facilitation.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string

  targetType?: TypeRef
  // The Record type this Protocol produces. Absent for loose / exploratory Protocols
  // (Brain Dump, Decomposition) whose output is input context for a tighter Protocol.

  stages: ProtocolStage[]

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

**The Protocol spectrum:**

```
Loose                                                    Tight
─────────────────────────────────────────────────────────────
Brain Dump → Decomposition → Options Analysis → Decision
```

Loose Protocols produce open material. Tight Protocols converge on a specific Record type. The output of a loose Protocol is the input context for something tighter.

**Generic Protocols** (reusable across domains):
- Brain Dump — externalise all thinking without constraint
- Decomposition — identify major components from raw material
- Review — what is established, what is still open
- Prioritisation — which components to resolve first

**Domain-specific Protocols** (target a specific Record type):
- Decision — context → criteria → options → evaluation → decision
- Proposal — problem → solution shape → constraints → proposal

**Protocol chaining and provenance**: The output of one Protocol is the input context for the next. This derivation chain is traceable through `derived-from` Relations, making the quality and history of the final Record auditable.

Views (`ext:views-l1`) no longer contain facilitation logic. A View is a presentation concern; a Protocol is an epistemic one.

---

### ext:schema

**Required for**: extraction pipelines, founding document workflows, any system that needs to specify what a document type IS before assembling it.

#### `RelationSpec`

Declares an expected Relation between two Record types within a Schema.

```typescript
{
  relationType: string
  sourceType: TypeRef
  targetType: TypeRef
  cardinality?: "one-to-one" | "one-to-many" | "many-to-many"
  required?: boolean
}
```

#### `Schema`

The definition of a complete document type — which Types it contains, what Relations exist between resulting Records, and what "complete" means. A Schema is the artefact handed to an extraction pipeline.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string

  rootTypes: TypeRef[]        // Types to extract
  structure: RelationSpec[]   // expected Relations between extracted Records
  requiredTypes: TypeRef[]    // what "complete" means for this document type

  aiGuidance?: AiGuidance
  // purpose: what kind of document this Schema defines
  // extraction: framing for extraction pipelines

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

**Schema vs View:**

| | Schema | View / Document View |
|---|---|---|
| Question it answers | What IS this document type? What should be extracted? | How are existing Records assembled into readable output? |
| Operates at | Definition time | Projection time |
| Input | Source material (transcripts, conversations) | Existing Records in a Container |
| Output | Extraction instructions → Records | Rendered document |

---

### ext:views-l1

**Required for**: rendering and export workflows.

Defines Views — versioned presentations of a single Record through a specific Type.

#### `FieldView`

A field reference within a View. Controls presentation for this View without altering field semantics.

```typescript
{
  fieldId: UUID       // must appear in the Type's fields[]
  order: integer      // min: 0; display order within this View
  required?: boolean  // View-level workflow constraint; does not alter Field contract
  visible?: boolean   // default: true

  // Presentation overrides — View scope only
  displayLabel?: string
  displayHint?: string
  editorHintOverride?: string
}
```

A Field hidden with `visible: false` remains in the Record and may appear in other Views.

#### `ExportConfig`

Configuration for rendering a Record through this View as an exportable document.

```typescript
{
  format?: string        // target format hint, e.g. "markdown", "adoc", "json"
  preamble?: string
  // Template string rendered before field values.
  // Variable substitution uses {{variable-name}} syntax.
  // Standard variables: {{instance-id}}, {{date}}, {{status}}, {{namespace}}, {{name}}

  fieldOrder?: UUID[]    // explicit export field ordering; defaults to fieldViews[].order
  omitEmptyFields?: boolean  // default: false
}
```

#### `View`

A versioned presentation and export configuration over a specific Type. Multiple Views may exist for the same Type, serving different audiences or purposes.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string    // when to use this View; what workflow or audience it serves

  typeId: UUID           // references Type.id
  typeVersion: integer

  aiGuidance?: AiGuidance
  // purpose: the workflow context this View serves
  // extraction: session-level framing injected before field extraction

  fieldViews: FieldView[]

  exportConfig?: ExportConfig

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

A View may not introduce Fields that are not in the bound Type. Omitted Fields are treated as `visible: false`.

Facilitation steps have been removed from View. Use `ext:protocol` Protocol stages instead.

---

### ext:views-l2

**Depends on**: `ext:views-l1`

**Required for**: document projection — assembling multiple Records into a coherent document.

#### `SectionSource`

Defines how a section's instances are selected from a Container.

```typescript
type SectionSource =
  | {
      type: "fixed-instances"
      instanceIds: UUID[]
      // Explicit list. For preamble, cover page, or curated sections.
    }
  | {
      type: "type-query"
      semanticObjectType: string
      // For cross-system portability, use namespace/name format (e.g. "core/decision").
      // A bare string like "decision" is a single-system convention.
      lifecycleState?: string
      containerIds?: UUID[]
    }
  | {
      type: "relation-query"
      fromInstanceId: UUID
      relationType: string
      direction?: "forward" | "inverse"  // default: "forward"
    }
  | {
      type: "container-subset"
      containerId: UUID
      containerType?: string
    }
```

#### `DocumentSection`

One section in a Document View.

```typescript
{
  sectionId: string
  title?: string
  description?: string
  order: integer   // min: 0

  source: SectionSource

  renderViewId?: UUID    // View (ext:views-l1) used to render each instance in this section
  // When absent, implementations use a default rendering for the instance type.

  ordering?: {
    fieldId?: UUID
    direction?: "asc" | "desc"  // default: "asc"
  }

  required?: boolean
  emptyBehavior?: "hide" | "show-placeholder"
}
```

#### `Document View`

A versioned, Container-level projection. Defines how a Container's Records are assembled into a readable document.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string    // what kind of document this produces; intended audience

  containerType?: string  // when set, intended for Containers of this type

  sections: DocumentSection[]

  preamble?: string
  // Template string rendered before all sections.
  // Standard variables: {{container-title}}, {{date}}, {{container-id}}

  format?: string   // e.g. "markdown", "adoc", "html"

  aiGuidance?: AiGuidance
  // purpose: what kind of document this View produces
  // extraction: context for AI-assisted document-level tasks

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

A Document View may reference multiple Views (via `DocumentSection.renderViewId`) — one per instance type in the document. It orchestrates; it does not replace them.

---

### ext:repeatable-fields

**Required for**: any Record type that needs lists of values within a single Field.

Adds repeatability to `FieldAssignment` and defines `FieldValueEntry`.

#### `FieldValueEntry`

A single entry in a repeatable field.

```typescript
{
  value: string | number | boolean | string[] | null
  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
}
```

#### FieldAssignment additions

When `ext:repeatable-fields` is in use, `FieldAssignment` gains:

```typescript
repeatable?: boolean  // default: false; when true, multiple values are allowed
minItems?: integer    // meaningful only when repeatable === true
maxItems?: integer    // meaningful only when repeatable === true
```

And `FieldValue.entries` becomes active: use `entries` when `repeatable === true`, `value` otherwise.

A repeatable field entry does not create a new semantic instance. Use separate Records connected by Relations when repeated items need their own identity, lifecycle, or graph position.

---

### ext:field-groups

**Required for**: Record types where multiple Fields are semantically paired and repeat together as a unit.

Use when parallel `multiselect` arrays would lose pairing (e.g. a contact record with `name` + `email`). Preserves internal pairing across repeated items.

#### `FieldGroup`

A named, ordered group of Fields that repeat together as a unit within a Type.

```typescript
{
  groupId: string        // stable key within the Type
  label?: string
  description?: string

  order: integer         // min: 0; position relative to other Fields and Groups

  required?: boolean     // default: false
  repeatable?: boolean   // default: false
  minItems?: integer
  maxItems?: integer

  fields: FieldAssignment[]
}
```

#### `FieldGroupEntry`

One entry in a repeatable Field Group.

```typescript
{
  entryId?: UUID         // stable key for this entry; allows referencing or updating
  fieldValues: FieldValue[]
}
```

#### `FieldGroupValue`

The current value of a Field Group within a Record.

```typescript
{
  groupId: string           // references FieldGroup.groupId in the Type definition
  entries: FieldGroupEntry[]
}
```

A `FieldGroup` does not create a new semantic instance. Its entries are embedded structured context within the enclosing Record. Use separate Records connected by Relations when group entries need their own identity, lifecycle, provenance, or reuse across Records.

When `ext:field-groups` is in use, `Type` gains `fieldGroups?: FieldGroup[]` and `Record` gains `groupValues?: FieldGroupValue[]`.

**Repeatability pattern guide:**

| Pattern | Use | Example |
|---|---|---|
| Repeatable scalar | `FieldAssignment.repeatable` (ext:repeatable-fields) | Multiple assigned person names |
| Repeatable structured context | `FieldGroup` | Contacts with name + email pairs |
| Repeated semantic objects | Separate Records + Relations | Tasks assigned to roles |

---

### ext:cross-field-validation

**Required for**: Types with constraints that span multiple Fields.

`ValidationRule` handles single-field constraints. `CrossFieldRule` handles constraints that require evaluating more than one Field together.

#### `CrossFieldRule`

```typescript
{
  type: "conditional-required" | "field-ordering" | "mutual-exclusion"
  message?: string

  // conditional-required: targetFieldId becomes required when predicateFieldId equals predicateValue
  predicateFieldId?: UUID
  predicateValue?: string
  targetFieldId?: UUID

  // field-ordering: targetFieldId must precede or follow predicateFieldId
  // Applies only to fields with valueType "date" or "number".
  effect?: "must-precede" | "must-follow"

  // mutual-exclusion: at most one of the listed fields may have a non-empty value
  fieldIds?: UUID[]   // min: 2
}
```

| Rule type | Required fields |
|---|---|
| `conditional-required` | `predicateFieldId`, `predicateValue`, `targetFieldId` |
| `field-ordering` | `predicateFieldId`, `targetFieldId`, `effect` |
| `mutual-exclusion` | `fieldIds` (min 2) |

When `ext:cross-field-validation` is in use, `Type` gains `validationRules?: CrossFieldRule[]`.

---

### ext:recommended-relations

**Required for**: cross-system federation; multi-publisher ecosystems where Relation type semantics must be interoperable.

Canonical relation types and machine-readable Relation type definitions.

**Canonical relation types** (use exact strings):

| Canonical | Converse | Category |
|---|---|---|
| `contains` | `part-of` | Composition |
| `depends-on` | `required-by` | Dependency |
| `supersedes` | `superseded-by` | Governance |
| `refines` | `refined-by` | Refinement |
| `derived-from` | `source-of` | Derivation |
| `evidences` | `evidenced-by` | Evidence |
| `precedes` | `follows` | Sequence |

Implementations must store only the canonical (forward) form and derive the inverse when needed.

**Relation category taxonomy:**

| Category | Examples |
|---|---|
| Composition | `contains`, `part-of`, `has-section` |
| Refinement | `refines`, `expands`, `summarises` |
| Dependency | `depends-on`, `requires`, `blocks`, `enables` |
| Sequence | `precedes`, `follows`, `overlaps` |
| Derivation | `derived-from`, `extracted-from`, `based-on` |
| Evidence | `evidences`, `supports`, `contradicts` |
| Governance | `supersedes`, `amends`, `ratifies`, `delegates` |
| Association | `relates-to`, `links-to` |

#### `RelationTypeDefinition`

Machine-readable metadata for a `relationType` string.

```typescript
{
  relationType: string      // exact string used in Relation.relationType
  namespace: string
  label?: string
  description?: string
  category?: "composition" | "refinement" | "dependency" | "sequence" | "derivation" | "evidence" | "governance" | "association"
  canonicalDirection?: string   // e.g. "source is the dependent task; target is the task it depends on"
  inverseType?: string
}
```

`RelationTypeDefinition` is optional metadata. Implementations are not required to resolve `relationType` strings against a definition before accepting a Relation. Relation type definitions may be included in a Package or published separately.

---

### ext:import-tracking

**Required for**: implementations that receive packages from upstream publishers and need to track update and conflict state.

#### `ImportMode`

```typescript
"upstream-tracked" | "local-copy" | "local-fork"
```

| Mode | Meaning |
|---|---|
| `"upstream-tracked"` | Consumer expects updates from the source Package. Conflicts surfaced when local and upstream diverge. |
| `"local-copy"` | Imported as a snapshot. No update tracking. |
| `"local-fork"` | Deliberately diverged. Upstream lineage preserved for reference. |

#### `ImportRecord`

One record per imported definition in a consumer's local registry.

```typescript
{
  definitionId: UUID
  definitionType: "field" | "type" | "view" | "schema" | "protocol"
  namespace: string
  name: string
  version: integer

  mode: ImportMode
  importedAt: ISO8601

  sourcePackageId: UUID
  sourcePackageName: string
  sourcePackageVersion: string

  latestKnownUpstreamVersion?: integer
  updateAvailable?: boolean
  updateCheckedAt?: ISO8601

  conflictState?: "clean" | "local-ahead" | "upstream-ahead" | "diverged"
  conflictDetectedAt?: ISO8601

  localVersion?: integer
  localEditedAt?: ISO8601
}
```

#### `ImportSummary`

A consumer's complete picture of its imported definitions.

```typescript
{
  generatedAt: ISO8601
  fields: ImportRecord[]
  types: ImportRecord[]
  views: ImportRecord[]
  schemas: ImportRecord[]
  protocols: ImportRecord[]
}
```

---

### ext:registry

**Required for**: multi-publisher ecosystems; discoverable definition catalogs.

#### `RegistryEntry`

One entry in a Registry catalog.

```typescript
{
  packageId: UUID
  packageName: string
  packageVersion: string
  publisher: string
  description?: string
  publishedAt: ISO8601
  homepage?: string
  tags?: string[]
  fieldCount: integer       // min: 0
  typeCount: integer        // min: 0
  viewCount?: integer
  schemaCount?: integer
  protocolCount?: integer
  relationTypeCount?: integer
  downloadUrl?: string
  checksum?: string         // SHA-256 hex digest for integrity verification
}
```

#### `Registry`

A registry's published index.

```typescript
{
  schemaVersion: string
  registryId: UUID
  registryName: string
  catalogVersion: string    // registry's own version (semver)
  updatedAt: ISO8601
  homepage?: string
  entries: RegistryEntry[]
}
```

Multiple Registries may coexist. A consumer may index multiple catalogs. The specification does not define registry authority, authentication, or federation.

---

## 8. Key Invariants

Conforming implementations must uphold the following invariants.

### Field semantics

**1.** `FieldAssignment.displayLabel` and `FieldAssignment.displayHint` are for rendering only. They must not affect AI guidance, extraction logic, `valueType` interpretation, or validation.

**2.** A `Type` must not redefine, override, or duplicate the semantic content of any `Field` it includes. If different semantics are needed for a Field in a specific Type context, a distinct `Field` with its own identity and lineage must be created.

**3.** A `Field`'s `aiGuidance` belongs to the Field. Type-level `aiGuidance` provides session framing only.

### Lifecycle (ext:lifecycle)

**4.** `Type.lifecycle.initialState` must reference a `name` that appears in `lifecycle.states[]` and where `isInitial === true`.

**5.** Every `from` and `to` value in `lifecycle.transitions[]` must reference a `name` that appears in `lifecycle.states[]`.

**6.** `Record.lifecycleState`, when present, must reference a `name` in the associated `Type.lifecycle.states[]`.

### Distribution

**7.** Every `fieldId` referenced in any `FieldAssignment` within a `Package.types[]` must appear as the `id` of an entry in `Package.dependencyRefs`.

**8.** If `Package.mode === "bundled"`: every `Reference` in `dependencyRefs` must have a matching `Field` in `fields[]` (matched on `id` and `version`).

**9.** `Field.id` is stable across versions. A new `id` means a new definition, not a new version of an existing one.

### Cross-field validation (ext:cross-field-validation)

**10.** All `fieldId` values in any `CrossFieldRule` within `Type.validationRules[]` must appear in `Type.fields[].fieldId`. Cross-field rules cannot reference Fields outside the Type.

**11.** A `conditional-required` rule must supply `predicateFieldId`, `predicateValue`, and `targetFieldId`. A `field-ordering` rule must supply `predicateFieldId`, `targetFieldId`, and `effect`. A `mutual-exclusion` rule must supply `fieldIds` with at least two entries.

### Views (ext:views-l1)

**12.** Every `fieldId` in `View.fieldViews[]` must appear in the bound Type's `fields[].fieldId`. A View cannot introduce Fields not in its Type.

**13.** `FieldView.displayLabel`, `FieldView.displayHint`, and `FieldView.editorHintOverride` are for rendering only. They must not affect AI guidance, extraction logic, `valueType` interpretation, or validation.

**14.** A `View` must not override, redefine, or duplicate the semantic content of any `Field` or `Type` it references. View-level `aiGuidance` is workflow framing; it does not redefine Field extraction semantics.

### Distribution — Views (ext:views-l1)

**15.** Every `typeId` referenced by any `View` in `Package.views[]` must appear in `Package.dependencyRefs` with `definitionType: "type"`. If `mode === "bundled"`, that `Type` must be present in `types[]`.

### Relations

**16.** In a `Relation`, `sourceInstanceId` is the asserting instance and `targetInstanceId` is the related instance. The Relation reads: "source [relationType] target." This convention must not be reversed.

**17.** `Relation` is reserved for assertions that carry semantic consequence beyond simple mention or citation. Lightweight prose references that do not assert structural, causal, or governance relationships must not be modelled as `Relation` records.

### Notes and Typed Records

**18.** `NoteSection.name` values must be unique within a `Note`.

**19.** `TypedField.name` values must be unique within a `Typed Record`.

### Containers

**20.** `Container.containerId` is not an instance ID. It must not appear in `Container.rootInstanceIds`, `Container.memberInstanceIds`, `Relation.sourceInstanceId`, or `Relation.targetInstanceId`.

**21.** `Container.rootInstanceIds` and `Container.memberInstanceIds`, when present, must reference valid SCDS instance IDs (`Note.instanceId`, `Typed Record.instanceId`, or `Record.instanceId`).

### Repeatability (ext:repeatable-fields)

**22.** If `FieldAssignment.repeatable` is false or absent, its corresponding `FieldValue` must use `value` and must not include `entries`.

**23.** If `FieldAssignment.repeatable` is true, its corresponding `FieldValue` may use `entries`. If `minItems` is specified, `entries` must contain at least that many items. If `maxItems` is specified, `entries` must not exceed that count.

**24.** `FieldAssignment.minItems` and `maxItems` are valid only when `repeatable === true`. They must be ignored when `repeatable` is false or absent.

### Field groups (ext:field-groups)

**25.** Every `groupId` in `Record.groupValues[]` must reference a `groupId` declared in the associated `Type.fieldGroups[]`.

**26.** Within a `FieldGroupEntry.fieldValues[]`, every `fieldId` must appear in the enclosing `FieldGroup.fields[].fieldId`.

**27.** A `FieldGroupValue.entries` list must satisfy `FieldGroup.minItems` and `maxItems` where specified.

### Records

**28.** `Record.typeId` and `Record.typeVersion` are the authoritative Type binding. `typeNamespace` and `typeName` are denormalised convenience fields. If they conflict with the resolved `Type`, the `typeId`/`typeVersion` identity takes precedence and the Record is considered invalid until corrected.

### Protocol (ext:protocol)

**29.** Every `stageId` in `ProtocolStage.dependsOn[]` must reference a `stageId` declared in the enclosing `Protocol.stages[]`. A stage may not declare a dependency on itself.

**30.** Every `fieldId` in `ProtocolStage.contributesTo[]` must reference a `fieldId` that appears in `Protocol.targetType`'s Field list. When `targetType` is absent (loose Protocol), `contributesTo` must be empty.

### Addressability (ext:addressability)

**31.** `Revision.priorRevisionId`, when present, must reference a `Revision.revisionId` for the same `fieldId` and `recordId`. Revision chains must be acyclic.

**32.** `AttentionState.containerId` must reference a valid `Container.containerId`. Other Address components (`recordId`, `fieldId`, `protocolRunId`, `stageId`) are optional and may be absent when focus has not yet narrowed.

---

## 9. Conformance

An implementation declares conformance using the following form:

```
SCDS <version> Core [+ ext:<name> ...]
```

Example:
```
SCDS 2.0 Core + ext:lifecycle + ext:protocol + ext:views-l1 + ext:addressability + ext:recommended-relations
```

### Core conformance requirements

A core-conformant implementation must:
- Accept and validate `Field`, `Type`, `Record` (Tier 2), `Relation`, and `Container` inputs against this specification
- Enforce Invariants 1–3, 7–9, 16–21, 28
- Support the Foundation and Distribution groups in full
- Implement the namespace format and reference format correctly
- Not accept `relationType` strings that include `/` except in `namespace/name` format

Support for `Note` (Tier 0) and `Typed Record` (Tier 1) is optional at core conformance level.

### Extension conformance requirements

An implementation declaring a given extension must:
- Accept and validate all types defined by that extension
- Enforce all invariants assigned to that extension
- Respect the declared dependency chain (e.g., `ext:views-l2` requires `ext:views-l1` to also be declared)

### Interoperability note

Two implementations at the same conformance level will produce compatible definitions for exchange. An implementation receiving a Package that includes types from an extension it does not support should surface the unknown content rather than silently discard it.

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
| Lists of values within a Record | `ext:repeatable-fields` |
| Structured repeatable context in a Record | `ext:field-groups` |
| Complex conditional validation | `ext:cross-field-validation` |
| Cross-system Relation interoperability | `ext:recommended-relations` |

### 4.2 Addressability as a prerequisite for live facilitation

`ext:addressability` is not just about naming things. It is the mechanism that makes the conversation layer useful. Without `AttentionState`, transcript chunks have no address-time connection to the Records they inform. Without `Revision`, the history of a field's value is an implementation detail not visible at the interoperability layer.

Any implementation that facilitates live sessions — where conversation material is produced while people are working on specific Records and Fields — should implement `ext:addressability`. Without it, context assembly is purely retrospective, and the quality of AI assistance degrades accordingly.

### 4.3 Schema vs View — the extraction gap

A View answers: given a Record that already exists, how do I render it for a specific audience?

A Schema answers: given source material, what Records should I extract, and how do they relate?

These are complementary but distinct. A Document View cannot serve as an extraction schema because it assumes Records already exist. A Schema cannot serve as a Document View because it does not specify how to render field values for an audience.

An extraction pipeline uses Schema + Field `aiGuidance` + Protocol to produce Records. A rendering pipeline uses View + Document View to project those Records into readable form.

### 4.4 Protocol loose-to-tight spectrum

The spectrum from loose to tight is not a quality ranking — it is a fitness question. A Brain Dump Protocol is the right tool when the problem space is not yet understood. A Decision Protocol is the right tool when the group is ready to converge. Starting with a tight Protocol before the problem is decomposed produces poor output because the epistemic prerequisites are not met.

The `dependsOn` field on `ProtocolStage` makes this explicit. A stage that depends on decomposition results cannot run before those results exist. This is not just sequencing — it is a statement about what understanding is required before the next stage is meaningful.

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

### View inheritance and composition

As View libraries mature, inheritance will become necessary. A lightweight ADR View and a governance ADR View share base configuration — field selection, ordering, `editorHint` overrides — while diverging on workflow framing and export layout.

A future version may define:
- `extendsViewId?: UUID` — single inheritance; child View inherits all `fieldViews` from parent and overrides selectively
- `composesViews?: UUID[]` — mixin composition; multiple Views contribute non-overlapping configuration

Current design: `View` is a leaf type. Use Lineage tracking to record inheritance relationships.

### Instance graph exchange format

A standard envelope for exchanging a Container together with its full Record set, Relations, and source references. Natural successor to `Package` at the instance layer. Likely shape: `{ container, instances[], relations[], sourceRefs[] }`. Deferred pending stabilisation of `ext:views-l2` and implementation experience.

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

Here's an evolution

---

# claude response

Clean evolution. The structure holds together well and the two-document split (spec + rationale) is the right call — it keeps the normative content lean while giving the rationale room to actually explain things.

**What's genuinely improved**

The four-group structure reads much better than nine layers. The extension conformance model with identifiers and dependency declarations is exactly right — `SCDS 2.0 Core + ext:lifecycle + ext:protocol` is legible and testable. The `meta` namespacing note ("com.acme.locking.locked-by") closes the concern from the original review. `RelationSpec.cardinality` is a good addition that wasn't in our conversation — it's useful for Schema completeness validation.

**Four things worth fixing before calling this stable**

*1. ProtocolStage has both `order` and `dependsOn` with no conflict resolution rule.*

If stage B has `order: 2` but `dependsOn: ["stage-c"]` which has `order: 3`, the spec is silent on what happens. Either `order` is display order only (execution determined by dependency resolution), or there's a requirement that `order` must be consistent with `dependsOn`. Needs a sentence in the spec and an invariant. My recommendation: `order` is display/presentation order; execution sequence is determined by `dependsOn` resolution. Add Invariant 33.

*2. `AttentionState` and `Address` overlap structurally.*

`AttentionState` is essentially a document-space `Address` with an optional process-space component stapled on. Right now they're two separate types that carry much of the same information. Either unify them (`AttentionState` is an `Address` tuple) or make the relationship explicit ("AttentionState is the live cursor; an Address is the stable identifier it resolves to"). As written, an implementer has to decide independently how to store and relate them.

*3. The `type-query` semanticObjectType federation hole is acknowledged but not closed.*

The note says "for cross-system portability, use namespace/name format" — but `semanticObjectType` is still `string` with no validation. Any Document View using `type-query` is only portable if authors follow the convention, and there's no invariant to enforce it. Either add an invariant ("when `SectionSource.type === 'type-query'` and the Document View is published in a Package, `semanticObjectType` must use `namespace/name` format") or accept the limitation explicitly in the rationale and note that `type-query` portability is a convention, not a guarantee.

*4. `Document View` needs a TypeScript programmatic name.*

The ext:views-l2 section defines the type inline under a heading but never gives it a formal `DocumentView` identifier. Every other type has a code-block name. This matters for the `renderViewId` field in `DocumentSection` — it says `View (ext:views-l1)` but a Document View could theoretically also be nested. Minor, but worth being explicit.

**One softer concern**

`ext:protocol` and `ext:addressability` are declared as independent extensions, but in practice they're a co-dependency for live facilitation. A Protocol without AttentionState produces no live tagging; AttentionState without Protocol runs has no stage context to capture. The spec handles this via the soft comments in Address (`runId — requires ext:protocol`) and the rationale section 4.2, but it might be worth a note in the extension table: "for live facilitation, both extensions should be declared together."

**The rationale document**

Sections 2.4 (Protocol vs TemplateFacilitationStep) and 2.5 (Schema gap) are the strongest — they explain the *why* clearly enough that a new implementer would understand the design intent without having been in the conversation. Section 3.6 (Protocol chaining trace) is the most practically useful thing in the whole rationale doc. Consider moving a version of it into the spec itself as a non-normative example, since it illustrates how Address + Protocol + Relation work together in a way that no single section does alone.

---

# you asked

message time: 2026-05-21 11:13:46

# SCDS Schema Specification

> **Superseded.** This is the v1.0-draft. It has been superseded by:
> - [`scds-spec.md`](scds-spec.md) — normative v2 specification (Field, Type, Record, Relation, extensions)
> - [`scds-rationale.md`](scds-rationale.md) — design rationale and usage guidance
>
> The v1→v2 vocabulary and structural mapping is in [`scds-shema-evolution.md`](scds-shema-evolution.md).
>
> This document is preserved as a v1 reference. Do not edit it.

**Version**: 1.0-draft
**Status**: superseded by scds-spec.md v2.0-draft
**Scope**: field definitions, module definitions, template definitions, instances, distribution (packages, import tracking, registry), relationships, containers (provisional), document projection (provisional)

---

## 1. Purpose and Scope

### What this specification defines

The Semantic Collaborative Document System (SCDS) schema layer defines an interoperable standard for semantic field and template definitions. It specifies the data shapes and invariants that allow field and module definitions to be created, shared, versioned, and imported across independent implementations.

This specification covers:

- **Field definitions** — atomic reusable semantic units
- **Module definitions** — named compositions of fields for a specific semantic object type
- **Template definitions** — versioned views, workflow configurations, and export layouts over module instances
- **Module instances** — the runtime shape of an instantiated module with field values
- **Freeform instances** — lightweight named-section content with no module binding; the entry point for progressive semantic adoption
- **Lifecycle** — optional state and transition vocabulary attached to module definitions
- **Distribution** — how definitions are packaged and distributed between repositories
- **Import tracking** — how a consumer tracks the origin, update state, and divergence of imported definitions
- **Registry** — a catalog format for discoverable definition packages
- **Relationships** — first-class semantic links between instances (Layer 5; stable core)
- **Containers** — lightweight grouping boundaries for instance collections; the root scope for export, session, and graph traversal (Layer 6)

### What this specification is not

This specification does not define:

- **Session** — live collaborative process model. Defined in a future version.
- **Projection** — rendered output model (human documents, exports, AI context packages). Defined in a future version.
- **Registry protocol** — how registries communicate, authenticate, or federate. This specification defines the data shapes only.

### Non-goals

SCDS does not define a universal semantic ontology. The standard defines interoperability mechanics for semantic collaboration systems, not canonical domain vocabularies. Domain-specific vocabularies are the responsibility of namespace authors.

SCDS does not prescribe governance processes, authority structures, or decision-making rules. The standard supports structured deliberation without mandating how deliberation is conducted or what constitutes a valid decision.

SCDS does not require any particular AI capability. AI guidance fields describe intent; implementations may use them with any AI system or not at all.

SCDS relation types are not limited to governance or decision-making. Implementations may use relationships to model composition, project structure, evidence chains, asset references, dependencies, refinement history, and temporal sequence.

### Relationship to implementing systems

This specification is implementation-neutral. Multiple independent systems may implement the same standard. The specification does not constrain persistence technology, API design, UI rendering, or prompt assembly strategy. Implementations are expected to validate inputs against these schemas at their system boundaries.

---

## 2. Core Thesis

Traditional document systems treat documents as primarily text.

This specification treats documents as **socially negotiated semantic state**. Text is one projection of that state.

Six principles follow from this:

**1. Semantic state is primary; documents are projections.**
The same semantic state may be rendered as a board paper, a governance record, a dashboard, or an AI context package. None of these projections is the source of truth.

**2. Fields are reusable semantic atoms.**
A field definition describes a reusable slot of meaning with stable identity. It is not a form field. It is not tied to any specific template or document. Its AI guidance, validation rules, and value type belong to the field, not to the module that uses it.

**3. Modules are compositions, not owners of field semantics.**
A module definition selects and orders fields for a specific semantic object type. It may provide session-level AI framing. It must not override or redefine the meaning of any field it includes.

**4. Lineage and provenance are first-class.**
Definitions evolve. Forks happen. Upstream changes must be traceable. A definition without lineage is a definition that cannot be trusted to evolve cleanly.

**5. Instances represent negotiated semantic state, not objective truth claims.**
A module instance records what a group understood, agreed, or committed to at a point in time. That understanding may be partial, contested, or later revised. The system preserves revision history and provenance precisely because the original state is worth keeping alongside its successors. Human prose and ambiguity are preserved, not collapsed.

**6. Understanding is mutable; historical semantic state has permanent value.**
SCDS assumes that understanding evolves. Instances, relationships, and lifecycle states may be revised, superseded, refined, or contradicted without invalidating prior semantic state. A rough plan is a valid semantic object. A superseded decision is a valid semantic object. An abandoned hypothesis is a valid semantic object. Historical semantic state is not noise to be discarded — it is provenance, institutional memory, and the record of how understanding arrived at its current form.

---

## 3. Namespace Format

### Convention

Namespaces are dot-separated identifiers using lowercase alphanumeric characters and hyphens.

```
<component>[.<component>]*

component = [a-z0-9][a-z0-9-]*
```

Examples:
```
core
community.adr
com.acme.hr
org.cooperative-name
```

### Reserved namespaces

`core` is reserved for definitions maintained by the SCDS standard itself. Implementations must not allow user-created definitions in the `core` namespace.

### Reference format

A specific version of a definition is referenced using the canonical form:

```
namespace/name@version
```

Examples:
```
core/decision_statement@2
community.adr/review_rationale@1
com.acme.hr/headcount_impact@3
```

The `/` and `@` characters are reserved separators in this format. They must not appear within a namespace component or a name.

### Name convention

Field and module names are programmatic keys in `snake_case`. Names are stable within a namespace and version lineage. A new name means a new definition, not a renamed version of an existing one.

---

## 4. Schema Definitions

### Notation

Types are described using TypeScript-style notation. Optional fields are marked with `?`. Constraints are noted inline. All `UUID` values are RFC 4122 UUID strings. All `ISO8601` values are datetime strings with timezone offset.

### Version semantics

Version numbers are positive integers scoped to a definition's UUID lineage. The following guidance applies to when a version bump is required or optional:

| Change | Version action |
|---|---|
| Documentation, typo, formatting only | Optional bump |
| `description`, `instructions`, or `aiGuidance.purpose` reworded without semantic change | Minor bump recommended |
| `aiGuidance.extraction` or `aiGuidance.purpose` changed in meaning | Version bump required |
| `valueType`, `selectOptions`, or `validationRules` changed | Version bump required |
| `name` changed | New definition required (new UUID) |
| `namespace` changed | New definition required (new UUID) |

When in doubt: if a downstream consumer's AI extraction, validation, or governance logic would behave differently, a version bump is required.

---

### 4.1 Layer 1 — Field Content

#### `ValidationRule`

A constraint applied to a field value.

```typescript
{
  type: "required" | "minLength" | "maxLength" | "pattern" | "enum"
  value?: string | number | string[]  // required for minLength, maxLength, pattern, enum
  message?: string                    // human-readable error message
}
```

#### `AiGuidanceExample`

A single example for AI guidance. Used for few-shot prompting.

```typescript
{
  description?: string  // labels this example, e.g. "Implicit deferral decision"
  input?: string        // sample source text; omit for output-only examples
  output: string        // the ideal value the AI should produce for this field
}
```

`output` is required. `input` is optional — an example without input demonstrates the expected form of the output without requiring a specific source. An example with both `input` and `output` is a proper input→output pair for few-shot prompting.

#### `AiGuidance`

The structured AI guidance block for a field or module. Replaces a flat extraction prompt string.

```typescript
{
  purpose: string            // what this field/module captures (1-2 sentences)
  extraction?: string        // direct LLM instruction for how to extract or populate this field
  negativeGuidance?: string  // what the LLM must NOT include or do
  examples?: AiGuidanceExample[]
}
```

The minimum valid `AiGuidance` is `{ purpose: "..." }`. This allows fields that are always human-authored to carry a machine-readable intent statement without requiring a dummy extraction prompt.

**Why the split between `extraction` and `negativeGuidance`:**
Mixed positive/negative instructions in a single prompt string cause models to weight negative instructions unpredictably. Keeping them separate allows implementations to compose prompts deliberately and update negative guidance independently of the extraction instruction.

**Why `purpose` is separate from `extraction`:**
`purpose` is a human-readable statement of semantic intent. `extraction` is the LLM directive. They serve different audiences: `purpose` helps a human understand what a field is for; `extraction` tells the AI how to populate it. Conflating them produces prompts that are neither good human documentation nor good AI instructions.

---

### 4.2 Layer 2 — Definitions

#### AI guidance composition — recommended assembly order

The spec defines `aiGuidance` at three levels, each serving a different scope. When composing a prompt from multiple `aiGuidance` blocks, implementations should assemble context in the following order:

1. **Module framing** (`ModuleDefinition.aiGuidance.extraction`) — establishes what semantic object type is being worked on; sets the broadest context
2. **Template framing** (`TemplateDefinition.aiGuidance.extraction`) — adds workflow-specific context for this particular facilitation scenario; narrows the session context
3. **Field extraction guidance** (`FieldDefinition.aiGuidance.extraction`) — specific instruction for populating each individual field
4. **Negative guidance** (`FieldDefinition.aiGuidance.negativeGuidance`) — constraints applied after the extraction instruction
5. **Examples** (`FieldDefinition.aiGuidance.examples`) — few-shot demonstrations presented last, as final grounding

This ordering ensures that broad context (what kind of object this is) is established before narrow directives (how to populate this specific field). Template framing narrows the module context — it does not replace it.

This is a recommended default, not a required invariant. Implementations may deviate for architectural reasons, but implementations that compose these levels differently will produce different AI behaviour from the same definitions. Documenting a recommended order creates a convergence point without mandating it.

---

#### `FieldDefinition`

The atomic reusable semantic unit.

```typescript
{
  // Stable identity
  id: UUID                   // stable across all versions; never reused
  namespace: string
  name: string               // snake_case programmatic key
  version: integer           // min: 1; increments within this id's lineage

  // Semantic content
  description: string        // one-sentence user-facing summary of what this field captures
  instructions?: string      // fuller guidance on how a human should complete this field
  aiGuidance: AiGuidance

  // Value semantics — stable across renderers
  valueType: "string" | "text" | "number" | "boolean" | "date" | "url" | "select" | "multiselect"

  selectOptions?: string[]   // required when valueType is "select" or "multiselect"
  validationRules?: ValidationRule[]

  // Editor hint — projection-specific default (implementations and templates may override)
  editorHint?: "singleline" | "textarea" | "rich-text" | "date-picker" | "dropdown" | "multi-select" | "voice"

  // Classification
  tags?: string[]            // free-form subject tags; no domain-specific enum

  // Metadata
  createdAt: ISO8601
  lineage?: DefinitionLineage
  provenance?: DefinitionProvenance
}
```

**`valueType` vs `editorHint` — why they are separate:**

| Concern | Field | Notes |
|---|---|---|
| Semantic data type | `valueType` | Stable. Used by AI guidance, validation, and export logic. |
| Preferred editing surface | `editorHint` | A default hint. Projections and templates may override. |

A field with `valueType: "text"` might be edited via textarea in a web form, captured via voice in a mobile app, or extracted directly from a transcript without any editing UI at all. The semantic type is stable; the editor is a rendering decision. AI extraction logic, validation rules, and export formatting should depend only on `valueType`.

**`valueType` semantics:**

| Value | Meaning |
|---|---|
| `"string"` | Short single-value text (typically one line) |
| `"text"` | Potentially long multi-paragraph prose |
| `"number"` | Numeric value |
| `"boolean"` | True/false |
| `"date"` | ISO 8601 date or datetime |
| `"url"` | A URL string |
| `"select"` | One value from `selectOptions` |
| `"multiselect"` | One or more values from `selectOptions` |

#### `LifecycleStateDefinition`

A named state in a module's lifecycle.

```typescript
{
  name: string          // e.g. "exercise", "proposal", "decision", "superseded"
  description?: string
  isInitial?: boolean   // this state is a valid starting state for new instances
  isFinal?: boolean     // no transitions out of this state; instance is settled
}
```

#### `LifecycleTransitionDefinition`

A named transition between lifecycle states.

```typescript
{
  name: string          // e.g. "promote", "approve", "supersede"
  from: string          // must match a state name in the enclosing lifecycle
  to: string            // must match a state name in the enclosing lifecycle
  description?: string
}
```

#### `ModuleFieldAssignment`

A field reference within a module definition. Configures presentation without redefining field semantics.

```typescript
{
  fieldId: UUID         // references FieldDefinition.id
  order: integer        // min: 0; display and processing order within the module
  required?: boolean    // default: true

  // Repeatability — module-scoped; the same field definition may be single in one module
  // and repeatable in another. Repeatability is a structural property of the assignment,
  // not of the field itself.
  repeatable?: boolean  // default: false; when true, multiple values are allowed
  minItems?: integer    // minimum entry count when repeatable; meaningful only when repeatable === true
  maxItems?: integer    // maximum entry count when repeatable; meaningful only when repeatable === true

  // Presentation-only — must NOT affect AI guidance, extraction, valueType, or validation
  displayLabel?: string // override display label for this assignment context only
  displayHint?: string  // additional short rendering context
}
```

`displayLabel` and `displayHint` are strictly for rendering. They are not visible to AI extraction logic. If a materially different label or meaning is needed, that requires a different `FieldDefinition` with its own lineage, not a presentation override.

#### `ModuleFieldGroup`

A named, ordered group of fields that repeat together as a unit within a module instance. Use when multiple fields are semantically paired — for example, a contact record with `name` + `email`. Preserves internal pairing across repeated items, avoiding the index-alignment problem of parallel `multiselect` arrays.

```typescript
{
  groupId: string              // stable key within the module, e.g. "assigned_contacts"
  label?: string               // e.g. "Assigned contacts"
  description?: string         // when to use this group; what each entry represents

  order: integer               // min: 0; position relative to other fields and groups in the module

  required?: boolean           // default: false; at least one entry must be present if true
  repeatable?: boolean         // default: false; when true, multiple entries are allowed
  minItems?: integer           // minimum entry count; meaningful only when repeatable === true
  maxItems?: integer           // maximum entry count; meaningful only when repeatable === true

  fields: ModuleFieldAssignment[]
  // The fields that constitute one entry in this group.
  // These are the same ModuleFieldAssignment shape as module-level fields.
  // Presentation-only overrides (displayLabel, displayHint) apply within the group context.
}
```

**On the semantic boundary:**
A field group records repeated structured context embedded within a module instance — it is not a collection of semantic objects. If each repeated item needs its own identity, lifecycle, relationships, provenance, or reuse across module instances, use separate `ModuleInstance` records connected by `contains` or other relationships instead.

| Pattern | Use | Example |
|---|---|---|
| Repeatable scalar | `ModuleFieldAssignment.repeatable` | Multiple assigned person names |
| Repeatable structured context | `ModuleFieldGroup` | Contacts with name + email pairs |
| Repeated semantic objects | Separate instances + relationships | Tasks assigned to roles |

#### `ModuleValidationRule`

A constraint that spans multiple fields within a module. Field-local `ValidationRule` handles single-field constraints; `ModuleValidationRule` handles the cases that require evaluating more than one field together.

```typescript
{
  type: "conditional-required" | "field-ordering" | "mutual-exclusion"
  message?: string  // human-readable error shown when the rule is violated

  // conditional-required: targetFieldId becomes required when predicateFieldId equals predicateValue
  predicateFieldId?: UUID
  predicateValue?: string  // exact string match; complex expressions are out of scope
  targetFieldId?: UUID

  // field-ordering: targetFieldId value must precede or follow predicateFieldId value
  // Applies only to fields with valueType "date" or "number".
  effect?: "must-precede" | "must-follow"

  // mutual-exclusion: at most one of the listed fields may have a non-empty value
  fieldIds?: UUID[]        // min: 2; for mutual-exclusion only
}
```

| Rule type | Use case | Required fields |
|---|---|---|
| `conditional-required` | `end_date` required only if `status === "Approved"` | `predicateFieldId`, `predicateValue`, `targetFieldId` |
| `field-ordering` | `end_date` must follow `start_date` | `predicateFieldId`, `targetFieldId`, `effect` |
| `mutual-exclusion` | Only one of `assigned_team` or `assigned_person` may be set | `fieldIds` |

This covers the most common cross-field constraints without introducing a general expression language. Complex conditional logic that cannot be expressed with these types belongs in implementation-layer business rules, not module definitions.

#### `ModuleDefinition`

A named, versioned composition of fields for a specific semantic object type. Modules are the composable units of a document or workspace — decisions, proposals, tasks, stages, risks, roles, budget lines, requirements, assets, and any other structured semantic object.

```typescript
{
  // Stable identity
  id: UUID
  namespace: string
  name: string
  version: integer      // min: 1

  // Content
  description: string   // when to use this module; what semantic object type it defines
  aiGuidance?: AiGuidance  // module-level LLM framing injected at session scope

  // Semantic object type (optional)
  semanticObjectType?: string
  // e.g. "decision", "proposal", "exercise", "task", "stage", "risk", "budget_line",
  //      "requirement", "milestone", "asset", "person", "open_question", "design_option"
  // Free-form; the standard does not enumerate allowed values.
  // Implementations may optionally use this as a lightweight semantic hint for rendering,
  // workflow, grouping, or reasoning. No conforming implementation is required to act on it.

  // Composition
  fields: ModuleFieldAssignment[]
  fieldGroups?: ModuleFieldGroup[]               // repeatable structured field groups; see ModuleFieldGroup
  moduleValidationRules?: ModuleValidationRule[]  // cross-field constraints; see ModuleValidationRule

  // Lifecycle (optional)
  lifecycle?: {
    states: LifecycleStateDefinition[]          // min 1 state
    transitions: LifecycleTransitionDefinition[]
    initialState: string  // must reference a state name in states[] where isInitial === true
  }

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: DefinitionLineage
  provenance?: DefinitionProvenance
}
```

**On `aiGuidance` at module level:**
`purpose` describes what type of semantic object this module captures. `extraction` provides session-level framing injected into the AI context (equivalent to a system prompt prefix that sets the decision category or document type). Examples are not typically used at module level.

**On `semanticObjectType`:**
This is a lightweight, optional semantic hint. It allows implementations to apply type-aware rendering, grouping, or reasoning without inspecting the full module definition. The value is free-form and the standard does not define an allowed vocabulary. Implementations are not required to act on it, and the standard deliberately avoids assigning normative meaning to any particular value — doing so would impose ontology where the field is meant to remain open.

**On `lifecycle`:**
Lifecycle is optional. Its presence declares the states and transitions a module supports. Implementations decide how strictly to enforce transitions. The specification is descriptive, not prescriptive. See Section 6 for lifecycle guidance.

**On instance migration when a module version changes:**
A `ModuleInstance` binds to a specific `moduleDefinitionVersion` at creation time. Instances created under version 2 remain valid version-2 instances after version 3 is published — they do not automatically migrate. If version 3 adds a required field, existing instances may lack that field; conformance is measured against the version they were instantiated under, not the latest version.

Implementations may offer migration tooling that re-instantiates existing instances against a newer module version, but the specification does not mandate this. When an instance is migrated and exchanged, it should carry the version it now conforms to. The original version-2 instance should be preserved and linked via a `supersedes` relationship if its authority is being replaced.

#### `TemplateFieldView`

A field reference within a template, configuring how a module field is presented in this view. Distinct from `ModuleFieldAssignment` — this is the template's projection of module fields, not the module's authoritative field list.

```typescript
{
  fieldId: UUID         // references FieldDefinition.id; must appear in the module's fields[]
  order: integer        // min: 0; display order within this template view
  required?: boolean    // template-level override; defaults to ModuleFieldAssignment.required
  visible?: boolean     // default: true; false to hide this field in this view

  // Presentation overrides — template scope only
  // Must NOT affect FieldDefinition AI guidance, extraction, valueType, or validation
  displayLabel?: string
  displayHint?: string
  editorHintOverride?: string  // overrides FieldDefinition.editorHint for this template view
}
```

`TemplateFieldView` controls presentation within this specific template and does not alter field semantics. A field hidden with `visible: false` remains in the module instance and may appear in other template views. A field marked `required: true` is a workflow-level constraint (this template's workflow requires this field); it does not alter the field's contract at the module or field definition level.

#### `TemplateFacilitationStep`

A named step in a guided facilitation sequence. Used when a template walks users or AI assistants through a structured process rather than presenting all fields simultaneously.

```typescript
{
  stepId: string        // stable key within this template
  order: integer        // min: 0; sequence position
  label: string         // human-readable step name, e.g. "Identify the options"
  description?: string  // guidance shown to the facilitator or participant for this step
  fieldIds: UUID[]      // subset of this template's fieldViews active in this step
  aiGuidance?: AiGuidance
  // purpose: what this step aims to achieve in the facilitation sequence
  // extraction: focused AI instruction for this step; supplements template-level aiGuidance
}
```

#### `TemplateExportConfig`

Configuration for rendering a module instance through this template as an exportable document.

```typescript
{
  format?: string            // target format hint, e.g. "markdown", "adoc", "json"
                             // Implementations may ignore or support additional formats.
  preamble?: string
  // A template string rendered before field values in the export output.
  // Variable substitution uses {{variable-name}} syntax.
  // Standard variables: {{instance-id}}, {{date}}, {{status}}, {{namespace}}, {{name}}
  // Implementations may support additional variables.
  // Example (ADR YAML frontmatter):
  //   "---\ndecision-id: {{instance-id}}\ndate: {{date}}\nstatus: {{status}}\n---"

  fieldOrder?: UUID[]        // explicit export field ordering; defaults to fieldViews[].order
  omitEmptyFields?: boolean  // default: false
}
```

#### Template levels

SCDS defines three levels of template, each operating at a different scope:

| Level | Type | Scope | Purpose |
|---|---|---|---|
| 1 — Module | `TemplateDefinition` | One module instance | Edit, facilitate, or export one semantic object |
| 2 — Document | `DocumentTemplate` | A container's instance collection | Assemble many instances into a coherent document |
| 3 — Session | *(future)* | Live collaborative process | Drive a facilitated session over a container's instances |

`TemplateDefinition` below is the Level 1 type. `DocumentTemplate` is defined in Layer 7. Session templates are a future extension (see Section 7).

The separation is intentional. Collapsing document composition into a large nested module definition — a "governance document" module with embedded fields — loses the semantic value of treating decisions, proposals, and articles as distinct, individually addressable, relatable objects. A document is a projection of a container's semantic graph, not a record type.

#### `TemplateDefinition` (Level 1 — Module Template)

A versioned view, workflow configuration, and optional export layout over a specific module definition. Templates provide audience- and purpose-specific presentations of module instances without altering their semantic content.

```typescript
{
  // Identity
  id: UUID
  namespace: string
  name: string           // snake_case programmatic key
  version: integer       // min: 1

  // Description
  description: string    // when to use this template; what workflow or audience it serves

  // Module binding — a template is always bound to a specific module version
  moduleDefinitionId: UUID
  moduleDefinitionVersion: integer

  // Workflow-level AI framing (optional)
  aiGuidance?: AiGuidance
  // purpose: describes the workflow context this template serves
  // extraction: session-level framing injected before field extraction;
  //             e.g. "You are facilitating a technology selection decision — emphasise
  //                   comparative analysis of alternatives against stated criteria."
  //             Complements module-level aiGuidance; does not replace it.

  // Field views
  // Only fields appearing in the referenced module's fields[] may appear here.
  // A template need not include all module fields — an omitted field is treated as visible: false.
  fieldViews: TemplateFieldView[]

  // Facilitation sequence (optional)
  facilitationSteps?: TemplateFacilitationStep[]

  // Export configuration (optional)
  exportConfig?: TemplateExportConfig

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: DefinitionLineage
  provenance?: DefinitionProvenance
}
```

**Why templates are a separate type from modules:**

| Concern | Module | Template |
|---|---|---|
| Owns semantic state | Yes — field definitions and their values | No |
| Owns field semantics | Yes — via `FieldDefinition.aiGuidance` | No |
| One per semantic object type | Yes | No — multiple templates may exist for one module |
| AI framing purpose | What kind of object this is | How to facilitate or export this workflow |
| Distributable independently | Yes | Yes — but always references a specific module version |

A module defines *what* a semantic object is. A template defines *how* to view, facilitate, or export that object for a specific purpose. The same module instance may be viewed through a full deliberation template, a summary template, and an ADR export template without any change to the instance data.

**On `aiGuidance` at template level:**
Template-level AI framing supplements module-level framing. When both are present, implementations should compose them — typically with module-level framing establishing what kind of object is being worked on, and template-level framing providing the workflow-specific context. Template framing should take precedence for session context when the two conflict.

**On the relationship between `TemplateFieldView` and `ModuleFieldAssignment`:**
`ModuleFieldAssignment` is authoritative — it determines which fields belong to a module and their baseline required status. `TemplateFieldView` is a view-layer projection — it can reorder, hide, and relabel fields for a specific audience, but cannot add fields that are not in the module, and any `required: true` override is a workflow constraint, not a semantic one.

The recommended AI guidance assembly order is defined at the top of Layer 2.

---

### 4.3 Layer 3 — Instances

These types define the runtime shape of an instantiated module — what a filled-in module looks like when an implementation creates or exchanges actual content. Without this layer the specification only defines schemas, not documents.

#### Semantic maturity tiers

SCDS is a progressively adoptable standard. Content does not need to be fully formalised from the start. Three tiers of semantic maturity are supported:

| Tier | Type | Structure | Semantics | Shareable |
|---|---|---|---|---|
| **0 — Freeform** | `FreeformInstance` | Named sections + free text | None | No |
| **1 — Typed** | `TypedInstance` | Named fields with types and values | Minimal | Partially |
| **2 — Defined** | `ModuleInstance` | Fields referencing `FieldDefinition` | Full | Yes |

The graduation path runs in one direction: freeform → typed → defined. Implementations are not required to support all three tiers; they may start at Tier 2. The tiers are defined to allow interoperability at the level of maturity a system has reached.

#### `FreeformSection`

A named text section within a freeform instance. The only structure required is a name and content — no UUID, no field type, no AI guidance.

```typescript
{
  name: string          // section key; unique within the instance; snake_case recommended
  label?: string        // human-readable display label, e.g. "What we decided"
  content: string       // free text; implementations may support markdown, plain, or rich text
  contentHint?: "text" | "markdown" | "plain"  // default: "text"; hint only
}
```

#### `FreeformInstance`

A lightweight content instance with no binding to a `ModuleDefinition` or `FieldDefinition`. Used when structure has not yet stabilised or semantic formalisation has not yet been warranted.

```typescript
{
  instanceId: UUID      // stable; carries forward if the instance is later graduated in place

  title?: string

  sections: FreeformSection[]

  graduatedAt?: ISO8601
  // When set, signals that this instance has been fully formalised into one or more
  // higher-tier instances. The original instance is preserved for provenance and graph
  // traversal. Implementations may exclude graduated instances from active queries.
  // The authoritative record of what this instance graduated into is the set of
  // derived-from relationships originating from the successor instances.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
  // Implementations may use meta to track graduation intent (e.g. target module)
  // before graduation is complete and derived-from links exist.
}
```

`FreeformInstance` carries no semantic guarantees. Section names are identifiers within this instance only — they are not stable, carry no lineage, and may be renamed freely.

#### `TypedField`

A field within a typed instance. Carries a name, an optional value type, and a value. No UUID, no AI guidance, no module binding.

```typescript
{
  name: string          // field key; unique within the instance; snake_case recommended
  label?: string        // human-readable display label
  valueType?: "string" | "text" | "number" | "boolean" | "date" | "url" | "select" | "multiselect"
  // When omitted, implementations may treat the field as "text".
  selectOptions?: string[]  // required when valueType is "select" or "multiselect"
  value: string | number | boolean | string[] | null
  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
}
```

#### `TypedInstance`

A structured instance with named, typed fields but no binding to a `ModuleDefinition` or `FieldDefinition`. Used when field names and types have stabilised but formal semantic definitions have not yet been created or adopted.

```typescript
{
  instanceId: UUID

  title?: string

  instanceType?: string
  // Lightweight semantic hint — analogous to ModuleDefinition.semanticObjectType.
  // Not a formal type declaration. Use namespace/name format for cross-system identification.

  fields: TypedField[]

  graduatedAt?: ISO8601
  // Same semantics as FreeformInstance.graduatedAt — signals full formalisation.
  // Authoritative record of successors is in derived-from relationships.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

`TypedField.name` values are the identity mechanism within a `TypedInstance`. They should be stable once the instance is being shared, but they are not versioned definitions — renaming a field does not create a lineage event.

**On naming collisions during automatic Tier 0 → Tier 1 transition:**
When parsing a `FreeformInstance` into a `TypedInstance` automatically — for example, via AI extraction from unstructured text — the same conceptual section may appear multiple times (e.g. three separate risk descriptions). Since `TypedField.name` values must be unique within an instance (Invariant 17), implementations must resolve collisions before producing a valid `TypedInstance`. Recommended strategy: append a numeric suffix to produce `risk`, `risk_2`, `risk_3`. Implementations should document their deduplication strategy, because the resulting names must match `FieldDefinition.name` values if the instance will later graduate to `ModuleInstance`. A collision-resolved name like `risk_2` will not automatically match a field definition named `risk` — the graduation mapping step must account for this.

#### On graduation between tiers

Graduation is the act of replacing a lower-tier instance with a higher-tier equivalent as its structure stabilises.

**Identity continuity during graduation:**

| Scenario | `instanceId` | Relationship |
|---|---|---|
| Same content, purely formalised (structural upgrade only) | Keep | None required |
| Content interpreted, restructured, or substantially changed during formalisation | New | `refines` from new to old |
| One instance split into multiple instances | New IDs for all | `derived-from` from each new instance to the original |

Pure formalisation — where section names map directly to field names and content is unchanged — is an in-place upgrade. The same `instanceId` continues to identify the same understood content, now expressed in a more formal shape. Implementations should preserve the original instance in their history even when graduating in place.

**Graduation is not always one-to-one.** A single `FreeformInstance` — a meeting note, a brainstorm, a rough plan — will often graduate into multiple `ModuleInstance` records. For example:

```text
one meeting note
→ one decision (ModuleInstance)
→ three tasks (ModuleInstance × 3)
→ two risks (ModuleInstance × 2)
```

In this case each resulting instance receives its own `instanceId` and should be linked to the original using `derived-from` relations. The original `FreeformInstance` is preserved and remains the semantic root of the derived graph.

**On graduated instance visibility:**
Graduated Tier 0/1 instances remain valid SCDS instances. They are not deleted. Setting `graduatedAt` signals to implementations that formalisation is complete. What to do with a graduated instance is an implementation decision:

- **Archive** — retain in history, exclude from default active queries; surfaced only when traversing the graph or searching historical state
- **Tombstone** — mark as inactive with a visible indicator; may appear in search results but clearly labelled as superseded
- **Hidden** — remove from user-facing queries entirely; retained for graph traversal and provenance only

The spec does not mandate a policy. The `derived-from` relationships and the `graduatedAt` timestamp together give implementations enough signal to make the right choice for their query model. What the spec does require: the instance must be preserved for graph traversal. Deleting a graduated instance breaks the provenance chain for any `derived-from` relationships that point to it.

Implementations may automate graduation suggestions by matching section or field names against `FieldDefinition.name` values in available module definitions.

#### `SourceReference`

A pointer from a field value (or instance) back to the external source material from which it was extracted or derived. Keeps SCDS independent of any specific transcript or evidence standard while enabling first-class provenance tracing.

```typescript
{
  sourceType: "transcript-chunk" | "transcript-segment" | "external-document"
  // "transcript-chunk"   — a chunk produced by a chunking pipeline; addressed by chunk ID
  // "transcript-segment" — a canonical, de-overlapped reading row visible to a human reviewer
  // "external-document"  — a document, file, or external resource outside the transcript layer

  sourceId: string
  // The stable identifier for this source within its system.
  // For transcript-chunk/segment: the chunk or reading-row UUID.
  // For external-document: a URL, DOI, file hash, or system-specific ID.

  sourceStandard?: string
  // The versioned standard or format the source conforms to, e.g.:
  // "canonical-transcript@1.0", "webvtt@1.0", "w3c-web-annotation@1.0"
  // Allows consumers to interpret sourceId correctly without coupling to a specific backend.

  streamId?: UUID
  // For transcript sources: the stream from which this chunk or segment originates.
  // Meaningful when a meeting has multiple concurrent streams (speech, chat, upload).

  relationType?: "evidence" | "derived-from" | "quoted-from" | "inspired-by" | "supersedes-context"
  // The relationship between the source and the value it supports.
  // "evidence"           — source material the value is based on; supports the claim
  // "derived-from"       — value was generated or synthesised from this source
  // "quoted-from"        — value directly quotes or reproduces content from the source
  // "inspired-by"        — source informed the value without being directly used
  // "supersedes-context" — this source replaces an earlier source for this value
  // When omitted, implementations may treat the link as generic evidence.

  confidence?: number     // 0.0–1.0; how confident the extractor was that this source supports the value
  note?: string           // optional human or AI annotation on the provenance link
}
```

**Focus links vs source references:**
These are distinct and must not be conflated:

| Type | When it is created | What it means |
|---|---|---|
| Focus link | Live, during a session — "this transcript happened while the group was focused on Article 5" | Contextual and attention-based; captured by session/connection state |
| Source reference (`SourceReference`) | During semantic refinement — "this wording in Article 5 was derived from chunks 120–140" | Editorial and provenance-based; recorded at extraction or editorial review time |

Focus links are a session-layer concern — they are captured from attention state (`activeMeetingId`, `activeDecisionId`, `activeField`) and stored in the transcript system as context tags. Source references are a SCDS concern — they are attached to field values, instances, or relations as permanent provenance assertions. The same chunk may appear in both: as a context-tagged segment (focus link) and as a `sourceRef` on the field value that was extracted from it. Both are correct; they answer different questions.

**On the transcript/SCDS boundary:**
Transcript chunks and reading segments are *source material* — addressable evidence. They do not become SCDS semantic instances automatically. A transcript chunk referenced in `sourceRefs` is evidence supporting a field value; it is not itself a `FreeformInstance` or `ModuleInstance` unless someone deliberately models it as one. This distinction preserves the layer separation:

```
Canonical Transcript Standard  →  captures evidence, streams, timing, provenance
SCDS                           →  captures negotiated meaning, structured records, relationships
AI context assembly            →  binds them when generating or reviewing semantic state
```

The `contentType` discriminator in transcript standards (`speech` vs `message`) is significant here. `speech` segments are duration-based and stream-relative; `message` events are point-in-time and wall-clock-originated. AI context assembly should not treat them identically — a chat message timestamp and a spoken utterance timestamp have different alignment semantics. `sourceStandard` is the mechanism that lets a consumer know which interpretation to apply.

#### `FieldValueEntry`

A single entry in a repeatable field. Used when `ModuleFieldAssignment.repeatable === true` and per-entry provenance is needed.

```typescript
{
  value: string | number | boolean | string[] | null
  // Type should conform to FieldDefinition.valueType.
  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
}
```

#### `FieldValue`

The current value of a field within a module instance. Supports both non-repeatable (single `value`) and repeatable (`entries`) fields.

```typescript
{
  fieldId: UUID                               // references FieldDefinition.id

  // Non-repeatable field — use value
  value?: string | number | boolean | string[] | null
  // Type of value should conform to FieldDefinition.valueType.
  // The specification does not prescribe internal storage — only exchange boundaries.

  // Repeatable field — use entries
  entries?: FieldValueEntry[]
  // One entry per repeated value. Use when ModuleFieldAssignment.repeatable === true.
  // entries respect minItems and maxItems from the assignment.

  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
  // For non-repeatable fields: source and editedAt describe the single value.
  // For repeatable fields: per-entry source and editedAt on FieldValueEntry take precedence;
  // source and editedAt here describe the field record as a whole (e.g. last AI sweep).

  sourceRefs?: SourceReference[]
  // Links from this value back to the source material from which it was extracted or derived.
  // Essential for explainability, governance challenge, and audit of AI-assisted extraction.
  // See SourceReference for the bridge between SCDS and transcript/document evidence systems.
}
```

#### `FieldGroupEntry`

One entry in a repeatable field group. Carries a set of field values that must be read together as a unit.

```typescript
{
  entryId?: UUID       // optional stable key for this entry; allows referencing or updating
                       // a specific entry without replacing the whole group
  fieldValues: FieldValue[]
  // One FieldValue per field in the group. Must cover at least the required fields
  // declared in ModuleFieldGroup.fields[].
}
```

#### `FieldGroupValue`

The current value of a field group within a module instance — a list of structured entries corresponding to one `ModuleFieldGroup`.

```typescript
{
  groupId: string           // references ModuleFieldGroup.groupId in the module definition
  entries: FieldGroupEntry[]
  // One entry per repeated item. Must satisfy ModuleFieldGroup.minItems and maxItems
  // when specified.
}
```

#### `ModuleInstance`

An instantiated module with field values.

```typescript
{
  instanceId: UUID
  moduleDefinitionId: UUID      // references ModuleDefinition.id
  moduleDefinitionVersion: integer
  moduleNamespace: string
  moduleName: string

  // Current lifecycle state (required if module has a lifecycle)
  lifecycleState?: string       // must match a state name in ModuleDefinition.lifecycle.states[]

  // Field values — flat fields
  fieldValues: FieldValue[]

  // Field group values — repeated structured groups (see ModuleDefinition.fieldGroups)
  groupValues?: FieldGroupValue[]

  // Instance-level source provenance (optional)
  sourceRefs?: SourceReference[]
  // Links from this instance as a whole back to its source material.
  // Use when provenance applies to the entire instance rather than individual fields —
  // e.g. "this decision instance was extracted from these three transcript chunks."
  // Field-level sourceRefs on FieldValue are preferred when provenance differs per field.

  // Timestamps
  createdAt?: ISO8601
  updatedAt?: ISO8601

  // Implementation-specific metadata
  meta?: Record<string, unknown>
  // Use meta for: lock state, visibility state, session references, participant records,
  // and any other implementation-local concerns that should not pollute the core shape.
  // Keys intended for cross-system exchange should be namespaced (e.g. "com.acme.locking.locked-by")
  // to avoid collision. Bare key names are acceptable for single-system internal use only.
}
```

**Why `meta` is permitted:**
Implementations have legitimate needs for implementation-local metadata — lock state, field visibility, session associations, participant snapshots. Rather than polluting the core instance shape with implementation-specific fields, `meta` provides a governed escape hatch. Implementations should document what they put in `meta` but are not required to interoperate on its contents.

**On instance revision and Principle 6 (mutable understanding):**
`ModuleInstance` defines the *exchange shape* for current semantic state. It does not prescribe internal storage. Principle 6 — "historical semantic state has permanent value" — applies at the exchange layer as follows:

- **In-place edits** (`updatedAt` advances, `fieldValues` mutate): appropriate for minor corrections, typos, and clarifications that do not alter the semantic meaning of the instance. The previous content is an implementation concern; the spec does not define a revision log format.
- **Semantic updates** (a group's understanding changed): these should produce a new instance linked to the prior by a `supersedes` or `refines` relationship. The prior instance remains valid and reachable via the relationship graph. This is what makes historical semantic state permanent at the interoperability layer.
- **Immutable instance + relationship graph**: an implementation may choose to treat all instances as append-only, producing a new instance for every change. This is a valid implementation strategy and naturally satisfies Principle 6.

The boundary is intentional. Implementations using event sourcing, collaborative editing, or CRDTs manage their own revision model internally and expose current state via `ModuleInstance` at exchange boundaries. A future version may define a standard format for exchanging revision history when that is itself a first-class interoperability concern.

**On semantic identity continuity:**
`instanceId` is stable across in-place edits, but the spec does not mechanically define when a change requires a new instance versus a mutation of the existing one. This is a semantic judgment. The underlying question is: *Would a reasonable reader, encountering this instance a year later, recognise it as the same understanding they would have read before the change?*

Practical guidance:

| Scenario | Guidance |
|---|---|
| Correcting how something is expressed (typo, phrasing) | Edit in-place |
| Adding context that reinforces the existing understanding | Edit in-place |
| Clarifying a detail that was ambiguous but the underlying understanding is unchanged | Edit in-place |
| Adding information that changes what was actually committed to or understood | New instance + `refines` or `supersedes` |
| Reversing or materially replacing a prior commitment | New instance + `supersedes` |
| Producing a more detailed version from a rough original | New instance + `refines` |

A useful cross-check: if a `supersedes` relationship would feel misleading — as if the group reversed itself when it only clarified — it is probably an edit. If a silent edit would feel misleading — as if the record was silently revised after the fact — it is probably a new instance.

Implementations should document their identity continuity policy and apply it consistently. Implementations that want stricter guarantees may treat all instances as append-only and rely entirely on the relationship graph for continuity.

---

### 4.4 Layer 4 — Distribution

#### `DefinitionLineage`

Upstream and fork tracking for a specific definition version. Applied to both `FieldDefinition` and `ModuleDefinition`.

```typescript
{
  sourceDefinitionId?: UUID     // UUID of the upstream definition in its originating registry
  sourceVersion?: integer       // upstream version at the time this version was derived
  forkedFromDefinitionId?: UUID // UUID of the definition this was deliberately forked from
  forkedFromVersion?: integer   // version at the fork point
}
```

**`sourceDefinition*` vs `forkedFrom*`:**

| Field pair | Meaning |
|---|---|
| `sourceDefinition*` | This is a tracked copy. The consumer expects to receive upstream updates. |
| `forkedFrom*` | This deliberately diverged. The consumer no longer expects to track upstream. |

Both may be present during a transition from tracking to forking. A definition with neither is locally authored with no upstream relationship.

#### `DefinitionProvenance`

Publisher and package origin metadata.

```typescript
{
  publisher?: string          // namespace or org of the original author, e.g. "com.acme"
  sourcePackage?: string      // package name that bundled this definition
  packageVersion?: string     // semver of the source package
  importedAt?: ISO8601        // when this was imported into the local registry
}
```

`packageVersion` is distinct from `FieldDefinition.version`. A package at `1.3.0` may contain `decision_statement@3` and `context@2`. Bumping the package to `1.4.0` (e.g. adding a new field) does not change the version of unchanged definitions.

#### `DefinitionReference`

A stable pointer to a specific definition version. Used in dependency manifests, import records, and cross-package references.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer    // min: 1
  definitionType?: "field" | "module" | "template"
  // When omitted, consuming systems may assume "field" for backward compatibility.
  // Required when dependencyRefs contains module or template references.
}
```

Canonical string form (derived from the scalar identity fields):

```
namespace/name@version
```

Implementations may expose the canonical string as a convenience property. The three scalar identity fields (`namespace`, `name`, `version`) are what must be stored and validated.

#### `DefinitionPackage`

The distributable artifact. Contains field, module, template, and relation type definitions with a complete dependency manifest.

```typescript
{
  // Package identity
  schemaVersion: string      // SCDS spec version this package conforms to, e.g. "1.0"
  packageId: UUID            // stable ID for this publication event
  packageName: string
  packageVersion: string     // semver, e.g. "1.2.0"
  publishedAt: ISO8601
  publisher?: string
  description?: string
  homepage?: string          // URL to documentation or registry page

  // Content (at least one of fields, modules, or templates must be non-empty)
  fields: FieldDefinition[]
  modules: ModuleDefinition[]
  templates?: TemplateDefinition[]
  relationTypes?: RelationTypeDefinition[]  // relation type definitions for consumers; see 4.7

  // Dependency mode
  mode: "bundled" | "standalone"

  // Complete dependency manifest (required in both modes)
  dependencyRefs: DefinitionReference[]
  // Contains field references (definitionType: "field") used by modules,
  // and module references (definitionType: "module") used by templates.
  // Consumers use this manifest to validate completeness without parsing content internals.
}
```

**`mode` semantics:**

| Mode | Meaning |
|---|---|
| `"bundled"` | All `FieldDefinition` records referenced by any module, and all `ModuleDefinition` records referenced by any template, are included in `fields[]` and `modules[]` respectively. Self-contained and portable. |
| `"standalone"` | Dependencies are expected to be pre-installed in the consumer's registry. `dependencyRefs` is the required manifest. |

**Package invariants** (see also Section 5):
- Every `fieldId` in any `ModuleFieldAssignment` within `modules[]` must appear in `dependencyRefs[].id`.
- Every `moduleDefinitionId` referenced by any `TemplateDefinition` in `templates[]` must appear in `dependencyRefs[].id` (with `definitionType: "module"`).
- If `mode === "bundled"`: every field `DefinitionReference` in `dependencyRefs` must have a matching `FieldDefinition` in `fields[]`; every module `DefinitionReference` must have a matching `ModuleDefinition` in `modules[]`.

`dependencyRefs` is required in both modes because consumers need the complete manifest to validate completeness without parsing content internals.

---

#### Import Tracking

These types describe the state of imported definitions in a consumer's local registry. They enable consumers to track upstream lineage, manage updates, and surface conflicts.

#### `ImportMode`

```typescript
"upstream-tracked" | "local-copy" | "local-fork"
```

| Mode | Meaning |
|---|---|
| `"upstream-tracked"` | Consumer expects to receive updates from the source package. Conflicts are surfaced when local and upstream diverge. |
| `"local-copy"` | Imported as a snapshot. No update tracking. Consumer manages the definition independently. |
| `"local-fork"` | Deliberately diverged. Consumer maintains its own version history. Upstream lineage is preserved for reference but not tracked. |

#### `ImportRecord`

One record per imported definition (field or module) in a consumer's local registry.

```typescript
{
  // What was imported
  definitionId: UUID
  definitionType: "field" | "module" | "template"
  namespace: string
  name: string
  version: integer

  // Import configuration
  mode: ImportMode
  importedAt: ISO8601

  // Source package reference
  sourcePackageId: UUID
  sourcePackageName: string
  sourcePackageVersion: string

  // Upstream tracking (meaningful when mode is "upstream-tracked")
  latestKnownUpstreamVersion?: integer
  updateAvailable?: boolean
  updateCheckedAt?: ISO8601

  // Conflict state
  conflictState?: "clean" | "local-ahead" | "upstream-ahead" | "diverged"
  conflictDetectedAt?: ISO8601

  // Local fork state (meaningful when mode is "local-fork" or after local edit)
  localVersion?: integer     // local version counter, independent of upstream
  localEditedAt?: ISO8601
}
```

#### `ImportSummary`

A consumer's complete picture of its imported definitions.

```typescript
{
  generatedAt: ISO8601
  fields: ImportRecord[]
  modules: ImportRecord[]
  templates: ImportRecord[]
}
```

Implementations may compute derived counts (updates available, conflicts, tracked vs local) from the summary for display purposes.

---

#### Registry

A registry catalog is a discoverable index of available definition packages. This section defines the shape of a registry's published metadata — not the registry protocol, authentication, or federation mechanism.

#### `RegistryEntry`

One entry in a registry catalog.

```typescript
{
  packageId: UUID
  packageName: string
  packageVersion: string
  publisher: string
  description?: string
  publishedAt: ISO8601
  homepage?: string
  tags?: string[]           // searchable subject tags for the package
  fieldCount: integer       // min: 0
  moduleCount: integer      // min: 0
  templateCount?: integer   // min: 0
  relationTypeCount?: integer  // min: 0
  downloadUrl?: string      // URL to fetch the DefinitionPackage JSON
  checksum?: string         // SHA-256 hex digest of the package JSON for integrity verification
}
```

#### `RegistryCatalog`

A registry's published index.

```typescript
{
  schemaVersion: string     // SCDS spec version
  registryId: UUID          // stable ID for this registry
  registryName: string
  catalogVersion: string    // registry's own catalog version (semver)
  updatedAt: ISO8601
  homepage?: string
  entries: RegistryEntry[]
}
```

**On decentralisation:**
This specification intentionally does not define registry authority. Multiple registries may coexist. A consumer may index multiple catalogs. Organisational forks and community standards can coexist using the same schema without requiring a central authority.

---

### 4.5 Layer 5 — Relationships

> **Status**: Stable core layer. The `SemanticRelation` type shape, directionality convention, canonical relation types, and `RelationTypeDefinition` are normative in this version. The full relation taxonomy and extended lifecycle interaction are descriptive guidance; conforming implementations are not required to support every relation category.

Relationships are first-class semantic links between SCDS instances — `FreeformInstance`, `TypedInstance`, and `ModuleInstance`. They capture structure, dependency, sequence, derivation, refinement, and authority where relevant. They allow a document to become a navigable semantic model rather than a flat collection of text blocks.

Relationships allow implementations to construct **semantic graphs** over instances for navigation, analysis, projection, and reasoning. A collection of instances connected by typed relationships is a semantic graph — not merely a set of documents. Implementations may traverse, index, filter, and project these graphs to answer questions about structure, dependencies, derivation history, and semantic topology that flat document collections cannot answer.

Relationships span tiers. A `FreeformInstance` may be the source of a `derived-from` relation to the `ModuleInstance` records it graduated into. A `TypedInstance` may have a `refines` relation to a `FreeformInstance` it replaced. Freeform graduation is not representable without relationships that cross tier boundaries.

```
Stage A contains Task B
Task B depends-on Task C
Decision D-004 supersedes D-001
Budget line refines rough cost estimate
Workshop photo evidences Stage 1 completion
```

Without explicit relationships, this structure is implicit in human prose, invisible to AI reasoning, and lost to any system that needs to navigate, analyse, or render the semantic model.

#### References vs Relationships

These are distinct and should not be conflated:

| Type | Meaning | Semantic consequence |
|---|---|---|
| Reference | A citation — one instance mentions another in prose | None; informational only |
| Relationship | A semantic assertion — one instance stands in a defined relation to another | Adds navigable structure, dependency, derivation, or authority |

"This plan mentions the budget spreadsheet" is a reference. "This task depends-on that task" is a relationship. "This decision supersedes an earlier one" is a relationship. References add context; relationships add machine-navigable semantic structure. Implementations may support lightweight references via `meta` or free-text fields. The `SemanticRelation` type is reserved for assertions that carry semantic consequence beyond citation.

#### `SemanticRelation`

```typescript
{
  relationId: UUID

  relationType: string
  // The semantic type of this relation. Free-form string.
  // See Relation Taxonomy below for recommended conventions.

  // Directionality: source is the asserting instance; target is the related instance.
  // The relation reads: "source [relationType] target."
  // "D-004 supersedes D-001" → sourceInstanceId: D-004, targetInstanceId: D-001
  // "Stage A contains Task B" → sourceInstanceId: Stage-A, targetInstanceId: Task-B
  //
  // Instance IDs may reference any SCDS instance type:
  // FreeformInstance.instanceId, TypedInstance.instanceId, or ModuleInstance.instanceId.
  // Relationships may cross tier boundaries — a FreeformInstance may be the target
  // of a derived-from relation from the ModuleInstance records it graduated into.
  sourceInstanceId: UUID    // the asserting instance (any SCDS instance type)
  targetInstanceId: UUID    // the related instance (any SCDS instance type)

  // Assertion provenance
  assertedBy?: "human" | "ai" | "imported"
  confidence?: number       // 0.0–1.0; meaningful for ai-asserted relations
  createdAt?: ISO8601
  createdBy?: string        // human identifier or system agent

  // Relation status and temporal validity
  status?: "proposed" | "active" | "rejected" | "superseded"
  // "proposed" — candidate relation awaiting review (typical for ai-asserted)
  // "active"   — confirmed and in effect (default when omitted)
  // "rejected" — considered and explicitly declined; retained for audit
  // "superseded" — replaced by a later relation assertion
  validFrom?: ISO8601       // when this relation became or becomes valid
  validUntil?: ISO8601      // when this relation ceases to be valid; null means indefinite

  notes?: string

  // Source provenance for this relation assertion (optional)
  sourceRefs?: SourceReference[]
  // Links from this relation assertion back to the source material that motivated it.
  // Use when the relation was AI-asserted from transcript evidence, or when an editor
  // can point to the discussion that established the relationship.
  // Example: an AI-asserted "evidences" relation between a workshop photo and a stage
  // completion claim, with sourceRefs pointing to the transcript segment where that
  // evidence was discussed.

  meta?: Record<string, unknown>
}
```

#### Directionality convention

`source` is the asserting instance; `target` is the related instance. The relation reads: "`source` [relationType] `target`."

| Relation | source | target |
|---|---|---|
| `contains` | the stage | the task inside it |
| `depends-on` | the task that depends | the task it needs |
| `refines` | the detailed version | the rough version |
| `supersedes` | the newer decision | the older decision |
| `evidences` | the source material | the claim it supports |
| `blocks` | the blocking instance | the instance blocked |

This convention must be consistent across implementations for relationships to be interoperable. See Section 5, Invariant 19.

#### Relation taxonomy

The standard does not enumerate a fixed set of allowed `relationType` values. Instead it defines recommended semantic categories. Implementations should use these category conventions as a guide when choosing or defining relation types.

| Category | Examples | Notes |
|---|---|---|
| Composition | `contains`, `part-of`, `has-section` | Structural containment — project → stage → task |
| Refinement | `refines`, `expands`, `summarises`, `details` | One instance develops or condenses another |
| Dependency | `depends-on`, `requires`, `blocks`, `enables` | Execution or logical dependencies |
| Sequence | `precedes`, `follows`, `overlaps` | Temporal order where causal direction is not implied |
| Derivation | `derived-from`, `extracted-from`, `based-on` | One instance was generated or synthesised from another |
| Evidence | `evidences`, `supports`, `contradicts` | Source material or supporting/challenging claims |
| Governance | `supersedes`, `amends`, `ratifies`, `delegates` | Changes formal authority, validity, or delegation |
| Association | `relates-to`, `links-to` | Weak semantic links without stronger assertable category |

**On relation type identifiers and cross-system interoperability:**
`relationType` is a free-form string, which enables flexibility but creates a federation risk: System A uses `part-of`, System B uses `contains` for the same semantic relationship, and graph traversal across systems breaks.

The standard addresses this through two conventions:

1. **Standard types** — the canonical relation types listed in the taxonomy above (`contains`, `depends-on`, `supersedes`, `refines`, `derived-from`, `evidences`, `precedes`) are their own identifiers. Implementations should use these exact strings for common relation types to ensure cross-system interoperability.

2. **Custom types** — relation types not covered by the taxonomy should use the `namespace/name` format (e.g. `com.acme.hr/transferred-to`, `community.adr/motivated-by`). This prevents collision with standard types and between namespaces.

Implementations that wish to assert equivalence between their custom type and a standard type (e.g. `org.acme/supersedes` ≡ `supersedes`) may document this mapping in their registry entry, but the specification does not define a formal equivalence mechanism in this version.

**On `semanticObjectType` interoperability:**
When `semanticObjectType` is used for cross-system identification (e.g. "find all decision instances"), bare strings like `"decision"` or `"task"` interoperate only by convention — two systems using the same string may mean different things. Implementations that require reliable semantic object type identification across system boundaries should use the `namespace/name` format for custom types (e.g. `com.acme.pm/task`) while bare common values remain acceptable for single-system or convention-based use.

#### Canonical directions and inverse relations

The taxonomy above lists both forward and inverse forms in the same column (e.g. `contains` and `part-of`). These are not equivalent — they are converses of each other. Recording both would duplicate the edge. Implementations must choose a canonical direction and store only one edge per relationship.

The following table defines canonical forward forms for the most common cross-category relation types. Implementations should use the canonical form and derive the inverse when needed rather than storing both.

| Canonical (forward) | Converse (inverse) | Category |
|---|---|---|
| `contains` | `part-of` | Composition |
| `depends-on` | `required-by` | Dependency |
| `supersedes` | `superseded-by` | Governance |
| `refines` | `refined-by` | Refinement |
| `derived-from` | `source-of` | Derivation |
| `evidences` | `evidenced-by` | Evidence |
| `precedes` | `follows` | Sequence |

The canonical direction for the remaining taxonomy examples is left to namespace authors to establish within their own vocabularies. When creating a custom relation type, its canonical direction should be documented alongside its definition.

#### `RelationTypeDefinition`

A machine-readable definition for a `relationType` string. Registries may publish these alongside field and module definitions to enable relation type discovery, validation, and semantic interoperability.

```typescript
{
  relationType: string      // the exact string used in SemanticRelation.relationType
  namespace: string         // namespace owning this relation type definition
  label?: string            // human-readable label, e.g. "depends on"
  description?: string      // what this relation asserts semantically
  category?: "composition" | "refinement" | "dependency" | "sequence" | "derivation" | "evidence" | "governance" | "association"
  canonicalDirection?: string
  // Describes what source and target mean for this type, e.g.:
  // "source is the dependent task; target is the task it depends on"
  inverseType?: string      // the canonical inverse relation type string, if one exists
}
```

`RelationTypeDefinition` is optional metadata — implementations are not required to resolve `relationType` strings against a definition before accepting a `SemanticRelation`. However, registries that publish `RelationTypeDefinition` records allow consumers to validate, display, and reason about relation types without hard-coding them. Relation type definitions may be included in `DefinitionPackage` (see Layer 4) or published separately by a registry.

#### Instance revision versus replacement

Relationships make explicit a conceptual distinction that prose alone cannot:

| Action | Modelled as | Meaning |
|---|---|---|
| Edit | Mutation of the same instance | The group refined its understanding in-place |
| Refine | New instance + `refines` relation | A more detailed successor created from a rough original |
| Supersede | New instance + `supersedes` relation | The prior instance's authority is replaced; prior state preserved |
| Derive | New instance + `derived-from` relation | A related but semantically distinct object produced from a source |

These distinctions matter for AI workflows (what context to include), governance (what is still authoritative), graph rendering (which edges carry authority versus information), and audit trails (how understanding arrived at its current form).

#### Non-decision example: project planning

A project plan may begin with three high-level stage modules. Over time each stage is refined with task modules, budget modules, risk modules, evidence modules, and asset reference modules. Relationships express containment, dependency, refinement, evidence, and sequencing without requiring the project plan to be treated as a decision or governance process.

```json
{ "relationType": "contains",    "sourceInstanceId": "<plan>",    "targetInstanceId": "<stage-discovery>" }
{ "relationType": "contains",    "sourceInstanceId": "<stage>",   "targetInstanceId": "<task-interviews>" }
{ "relationType": "depends-on",  "sourceInstanceId": "<task-b>",  "targetInstanceId": "<task-a>" }
{ "relationType": "refines",     "sourceInstanceId": "<budget-detail>", "targetInstanceId": "<budget-rough>" }
{ "relationType": "evidences",   "sourceInstanceId": "<workshop-photo>", "targetInstanceId": "<stage-discovery>" }
```

The governance relations (`supersedes`, `delegates`, `ratifies`) apply when the semantic object type calls for them. They are one profile of the relationship layer, not its primary purpose.

#### AI-asserted relations

Relations with `assertedBy: "ai"` and a `confidence` value are candidate relations. Implementations should treat them as proposals requiring human review rather than authoritative assertions until ratified. The `confidence` field (0.0–1.0) allows consumers to filter or triage candidates by confidence threshold.

---

### 4.6 Layer 6 — Containers

A container is a lightweight grouping boundary over a collection of SCDS instances. It answers questions that the relationship graph cannot answer on its own: which instances belong together as a logical unit? What is the root object? What should be exported or queried together?

Containers are not semantic objects with fields — they are addressing and scoping boundaries. They do not own semantic state; instances do. Where a `contains` relationship asserts "A is part of B" (a semantic claim), a `SemanticContainer` asserts "these instances collectively form a unit for boundary purposes" (a scope claim). Both are useful; neither replaces the other.

Without a container, an implementation cannot cleanly answer:
- Which graph are these instances part of?
- What should be exported together?
- What is the root object for a session?
- What counts as "this project" or "this workspace"?

These questions arise regardless of whether a rich relationship graph has been constructed. A `SemanticContainer` provides the minimal boundary that allows instances to be grouped, addressed, and scoped before the full relationship layer is in place.

#### `SemanticContainer`

```typescript
{
  containerId: UUID

  namespace?: string         // namespace of the container definition, if formally named
  name?: string              // snake_case stable key within the namespace

  title: string              // human-readable label, e.g. "Q3 2025 Planning Cycle"

  containerType?: string
  // Optional lightweight semantic hint — analogous to ModuleDefinition.semanticObjectType.
  // Free-form; e.g. "project", "meeting", "sprint", "workspace", "portfolio".
  // No conforming implementation is required to act on it.

  rootInstanceIds?: UUID[]
  // The top-level instances this container was created to hold.
  // These are not necessarily the only members; memberInstanceIds may include transitively
  // contained instances. Implementations may derive contained sub-instances from the
  // relationship graph (traversing contains relations from roots).

  memberInstanceIds?: UUID[]
  // Explicit, queryable membership list for all instances in scope, including nested members.
  // When present, allows membership queries without graph traversal.
  // When omitted, membership is defined by traversing contains relations from rootInstanceIds.
  // Both strategies are valid; implementations should document which they use.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

**On containers and the relationship layer:**
Containers and `SemanticRelation` are complementary. Relationship-first implementations use `contains` relations and derive container membership by traversal. Container-first implementations use explicit `memberInstanceIds` and optionally layer in `contains` relations for finer navigation. Implementations may use either or both strategies.

**On container identity in relationships:**
`SemanticContainer.containerId` is not an instance ID. `SemanticRelation.sourceInstanceId` and `targetInstanceId` reference `FreeformInstance`, `TypedInstance`, and `ModuleInstance` records — not containers. Implementations that need to assert relationships between containers and instances should use the relationship `meta` field or a dedicated container-to-instance convention documented in their namespace. A future version may define `sourceType`/`targetType` fields on `SemanticRelation` to support container-level relationships explicitly.

---

### 4.7 Layer 7 — Document Projection

> **Status**: Provisional. The types and source query model are sketched here to establish the architectural boundary and avoid constraining lower layers. Detailed semantics, query language, and renderer integration require a dedicated design pass before this layer is considered stable.

A document projection template defines how a container's collection of semantic instances is assembled into a coherent, readable document. It is a Level 2 template — it operates over a container, not a single module instance.

This layer answers the question that lower layers cannot: given a `SemanticContainer` full of decisions, proposals, exercises, risks, and articles, how do they become a structured governance document, board paper, or project summary?

The key architectural principle: **a document is a projection, not a record type**. The instances remain individually addressable, relatable, and revisable. The document is a view assembled from them — not a container that absorbs their identity.

#### `DocumentSectionSource`

Defines how a section's instances are selected from the container. One of four strategies:

```typescript
type DocumentSectionSource =
  | {
      type: "fixed-instances"
      instanceIds: UUID[]
      // Explicit list. Use for preamble, cover page, or manually curated sections.
    }
  | {
      type: "module-type-query"
      semanticObjectType: string
      // e.g. "decision", "article", "exercise"
      // For cross-system portability, use namespace/name format (e.g. "core/decision",
      // "com.acme.pm/task"). A bare string like "decision" is an implicit convention —
      // two systems using the same bare string may mean different semantic types.
      // A DocumentTemplate that uses bare semanticObjectType values is single-system only;
      // one that uses namespace/name values can be distributed and consumed reliably.
      lifecycleState?: string             // filter by current lifecycle state
      containerIds?: UUID[]              // scope to specific containers; defaults to the document's container
      // Use for sections like "All decisions" or "Open exercises".
    }
  | {
      type: "relation-query"
      fromInstanceId: UUID               // traverse relations from this instance
      relationType: string               // e.g. "contains", "evidences", "derived-from"
      direction?: "forward" | "inverse"  // default: "forward"
      // Use for sections like "Tasks derived from this decision" or "Evidence for this stage".
    }
  | {
      type: "container-subset"
      containerId: UUID                  // a sub-container within the document's root container
      containerType?: string             // optional filter by container type
      // Use for multi-team or multi-project documents where each section covers a sub-scope.
    }
```

#### `DocumentSection`

One section in a document projection template.

```typescript
{
  sectionId: string        // stable key within this template
  title?: string           // rendered section heading
  description?: string     // authoring guidance; not typically rendered

  order: integer           // min: 0; section sequence in the rendered document

  source: DocumentSectionSource
  // How instances for this section are selected. See DocumentSectionSource.

  renderTemplateId?: UUID
  // Optional TemplateDefinition (Level 1) used to render each instance in this section.
  // When absent, implementations use a default rendering for the instance type.

  ordering?: {
    fieldId?: UUID          // sort by a specific field value within the module
    direction?: "asc" | "desc"  // default: "asc"
  }

  required?: boolean        // default: false; if true, an empty section is an error
  emptyBehavior?: "hide" | "show-placeholder"
  // "hide" — omit the section entirely if no instances match
  // "show-placeholder" — render the section heading with a placeholder message
}
```

#### `DocumentTemplate`

A versioned, container-level projection template. Defines how a container's instances are assembled into a readable document.

```typescript
{
  // Identity
  id: UUID
  namespace: string
  name: string           // snake_case programmatic key
  version: integer       // min: 1

  // Description
  description: string    // what kind of document this produces; intended audience

  // Container scope (optional)
  containerType?: string
  // When set, this template is intended for containers of this type.
  // Implementations may use this to suggest appropriate templates.
  // Not a hard constraint — implementations decide enforcement.

  // Document sections
  sections: DocumentSection[]

  // Document-level output configuration (optional)
  preamble?: string
  // Template string rendered before all sections.
  // Variable substitution uses {{variable-name}} syntax.
  // Standard variables: {{container-title}}, {{date}}, {{container-id}}

  format?: string        // target format hint, e.g. "markdown", "adoc", "html"

  // AI framing (optional)
  aiGuidance?: AiGuidance
  // purpose: what kind of document this template produces
  // extraction: context injected when AI assists with document-level tasks
  //             (e.g. generating an executive summary from the assembled sections)

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: DefinitionLineage
  provenance?: DefinitionProvenance
}
```

**Why this is separate from `TemplateDefinition`:**

| Concern | `TemplateDefinition` (Level 1) | `DocumentTemplate` (Level 2) |
|---|---|---|
| Operates on | One module instance | A container's instance collection |
| Defines | How fields are viewed, facilitated, or exported | How instances are selected, ordered, and assembled |
| Bound to | A specific `ModuleDefinition` | Optionally scoped to a `containerType` |
| AI framing | Workflow context for one object | Document-level context for assembled output |

A `DocumentTemplate` may reference multiple `TemplateDefinition` records (via `DocumentSection.renderTemplateId`) — one per instance type in the document. It orchestrates them; it does not replace them.

**On source query semantics:**
The `DocumentSectionSource` types intentionally avoid a general query language. The four strategies cover the most common document assembly patterns without requiring implementations to embed a full query engine. Complex filtering that cannot be expressed with these types belongs in implementation-layer document assembly logic, not in the template definition.

---

## 5. Key Invariants

Conforming implementations must uphold the following invariants.

### Field semantics

1. `ModuleFieldAssignment.displayLabel` and `ModuleFieldAssignment.displayHint` are for rendering only. They must not affect AI guidance, extraction logic, `valueType` interpretation, or validation.

2. A `ModuleDefinition` must not redefine, override, or duplicate the semantic content of any `FieldDefinition` it includes. If different semantics are needed for a field in a specific module context, a distinct `FieldDefinition` with its own identity and lineage must be created.

3. A `FieldDefinition`'s `aiGuidance` belongs to the field. Module-level `aiGuidance` provides session framing only.

### Lifecycle

4. `ModuleDefinition.lifecycle.initialState` must reference a `name` that appears in `lifecycle.states[]` and where `isInitial === true`.

5. Every `from` and `to` value in `lifecycle.transitions[]` must reference a `name` that appears in `lifecycle.states[]`.

6. A `ModuleInstance.lifecycleState` value, when present, must reference a `name` that appears in the associated `ModuleDefinition.lifecycle.states[]`.

### Distribution

7. Every `fieldId` referenced in any `ModuleFieldAssignment` within a `DefinitionPackage.modules[]` must appear as the `id` of an entry in `DefinitionPackage.dependencyRefs`.

8. If `DefinitionPackage.mode === "bundled"`: every `DefinitionReference` in `dependencyRefs` must have a matching `FieldDefinition` in `fields[]` (matched on `id` and `version`).

9. `FieldDefinition.id` is stable across versions. A new `id` means a new definition, not a new version of an existing one.

### Module validation

10. All `fieldId` values referenced in any `ModuleValidationRule` within `ModuleDefinition.moduleValidationRules[]` must appear in `ModuleDefinition.fields[].fieldId`. Cross-field rules cannot reference fields outside the module.

11. A `conditional-required` rule must supply `predicateFieldId`, `predicateValue`, and `targetFieldId`. A `field-ordering` rule must supply `predicateFieldId`, `targetFieldId`, and `effect`. A `mutual-exclusion` rule must supply `fieldIds` with at least two entries. Rules missing required fields are invalid.

### Templates

12. Every `fieldId` in `TemplateDefinition.fieldViews[]` must appear in the referenced module's `fields[].fieldId`. A template cannot introduce fields that are not part of its bound module.

13. Every `fieldId` in `TemplateFacilitationStep.fieldIds[]` must appear in `TemplateDefinition.fieldViews[].fieldId`. A step may reference fields that have `visible: false` in the default view — facilitation steps may reveal fields progressively that are otherwise hidden from the general layout.

14. `TemplateFieldView.displayLabel`, `TemplateFieldView.displayHint`, and `TemplateFieldView.editorHintOverride` are for rendering only. They must not affect AI guidance, extraction logic, `valueType` interpretation, or validation.

15. A `TemplateDefinition` must not override, redefine, or duplicate the semantic content of any `FieldDefinition` or `ModuleDefinition` it references. Template-level `aiGuidance` is workflow framing; it does not redefine field extraction semantics.

### Freeform and typed instances

16. `FreeformSection.name` values must be unique within a `FreeformInstance`. Section names are the only identity mechanism within a freeform instance; duplicates make sections unaddressable.

17. `TypedField.name` values must be unique within a `TypedInstance`. Field names are the only identity mechanism within a typed instance.

### Module instances

18. `ModuleInstance.moduleDefinitionId` and `ModuleInstance.moduleDefinitionVersion` are the authoritative module binding. `moduleNamespace` and `moduleName` are denormalised convenience fields for consumers that need them without resolving the UUID. If they conflict with the resolved `ModuleDefinition`, the definition identity takes precedence and the instance is considered invalid until corrected.

### Relationships

19. In a `SemanticRelation`, `sourceInstanceId` is the asserting instance and `targetInstanceId` is the related instance. The relation reads: "source [relationType] target." This convention must not be reversed. A consumer interpreting `D-004 supersedes D-001` must represent this as `sourceInstanceId: D-004, targetInstanceId: D-001`, not the inverse. Relation semantics emerge from the combination of `relationType` and directionality — the `source`/`target` slots are a convention that establishes which side is which, but they do not carry agency, authority, or causation *independent of `relationType`*. `supersedes` implies authority transfer because that is the semantics of `supersedes`, not because `source` is always the authoritative party.

20. `SemanticRelation` is reserved for assertions that carry semantic consequence beyond simple mention or citation. Lightweight prose references that do not assert a structural, causal, or governance relationship should not be modelled as `SemanticRelation` records.

### Distribution — templates

21. Every `moduleDefinitionId` referenced by any `TemplateDefinition` in `DefinitionPackage.templates[]` must appear in `DefinitionPackage.dependencyRefs` with `definitionType: "module"`. If `mode === "bundled"`, that `ModuleDefinition` must be present in `modules[]`.

### Containers

22. `SemanticContainer.rootInstanceIds` and `SemanticContainer.memberInstanceIds`, when present, must reference valid SCDS instance IDs (`FreeformInstance.instanceId`, `TypedInstance.instanceId`, or `ModuleInstance.instanceId`). A `containerId` is not an instance ID and must not appear in `rootInstanceIds`, `memberInstanceIds`, `SemanticRelation.sourceInstanceId`, or `SemanticRelation.targetInstanceId`.

### Repeatability

23. If a `ModuleFieldAssignment.repeatable` is false or absent, its corresponding `FieldValue` must use `value` and must not include `entries`.

24. If a `ModuleFieldAssignment.repeatable` is true, its corresponding `FieldValue` may use `entries`. If `minItems` is specified, `entries` must contain at least that many items. If `maxItems` is specified, `entries` must not exceed that count.

25. `ModuleFieldAssignment.minItems` and `maxItems` are valid only when `repeatable === true`. They must be ignored by validation logic when `repeatable` is false or absent.

26. A repeatable field entry (`FieldValueEntry`) does not create a new semantic instance. Use separate module instances connected by `contains` or other relationships when repeated items need their own identity, lifecycle, or graph position.

### Field groups

27. Every `groupId` in `ModuleInstance.groupValues[]` must reference a `groupId` declared in the associated `ModuleDefinition.fieldGroups[]`.

28. Within a `FieldGroupEntry.fieldValues[]`, every `fieldId` must appear in the enclosing `ModuleFieldGroup.fields[].fieldId`. Group entries cannot carry field values for fields outside their group definition.

29. A `FieldGroupValue.entries` list must satisfy `ModuleFieldGroup.minItems` and `maxItems` where specified. If `ModuleFieldGroup.required === true` and `repeatable === false`, exactly one entry must be present. If `required === true` and `repeatable === true`, at least `minItems` (or 1 if `minItems` is absent) entries must be present.

30. A `ModuleFieldGroup` does not create a new semantic instance. Its entries are embedded structured context within the enclosing `ModuleInstance`. Use separate `ModuleInstance` records connected by relationships when group entries need their own identity, lifecycle, provenance, or reuse across instances.

---

## 6. Lifecycle Note

### Descriptive, not prescriptive

The `lifecycle` block on a `ModuleDefinition` declares vocabulary. It does not mandate enforcement. Implementations may enforce transitions strictly, loosely, or not at all. The specification defines the schema; implementations define the policy.

### Typical progressions

State progressions depend on the semantic object type. Two examples:

**Decision-class module:**
```
exercise  →  proposal  →  decision  →  superseded
```
- `exercise` — open exploration; outcome unknown
- `proposal` — candidate commitment under consideration
- `decision` — agreed commitment
- `superseded` — replaced by a later decision

**Task-class module:**
```
backlog  →  active  →  blocked  →  complete
```
- `backlog` — not yet started
- `active` — in progress
- `blocked` — waiting on a dependency
- `complete` — settled; no further transitions

Both are illustrative. Module definitions should define state names appropriate to their own domain.

### Final states

A state with `isFinal: true` indicates no further transitions are expected. Implementations may use this to lock instance content or signal that the instance is settled for governance purposes.

### Relationships do not change lifecycle state

A `SemanticRelation` does not automatically change the `lifecycleState` of any instance it references. The `supersedes` relation indicates that a prior instance's authority is replaced, but it does not mutate the prior instance's `lifecycleState`. The `refines` relation indicates derivation, not progression. Lifecycle state changes are explicit acts — they must be performed by an implementation's transition mechanism, not inferred from relationship assertions.

This separation allows relationship graphs to express semantic structure without triggering unintended lifecycle side-effects. An implementation may choose to advance lifecycle state in response to a relationship assertion, but if it does, that policy must be explicit and documented.

---

## 7. Future Extensions

The following capabilities are planned but out of scope for this version of the specification.

### Session

A live collaborative process model. A session is a process-oriented container rather than a document-oriented one. Sessions may include transcript ingestion, live facilitation, AI assistance, collaborative editing, and semantic extraction. A session produces or enriches module instances but does not own them.

Session templates (Level 3 in the template hierarchy) would drive the facilitation process itself — which steps to follow, when to surface instances, when to request AI extraction. Deferred pending stabilisation of the session model and implementation experience with `DocumentTemplate` (Layer 7).

### Projection

Document-level projection is partially addressed by `DocumentTemplate` (Layer 7), which covers static assembled documents and governance records.

The broader projection surface — dashboards, timelines, AI context packages, real-time views, and composite renderings that are not document-shaped — remains a future concern. The full projection layer defines how semantic state is transformed into communicable form for an audience or purpose beyond what document templates cover. Projections are read-only views; they do not modify instance state.

### Revision history exchange format

A standard format for exchanging full revision history between implementations, for cases where the history itself — not just the current state — is a first-class interoperability concern. Deferred pending stabilisation of the container and relationship layers, which together provide the reference model for what constitutes a revisable unit.

### Graduation mapping record

A structured artefact recording how a freeform or typed instance was mapped to its module-level successors — including which section or field names were matched, merged, split, or interpreted. Useful for AI-assisted graduation review, audit, and provenance. Deferred pending implementation experience.

### Template inheritance and composition

As template libraries mature, inheritance and composition will become necessary. A lightweight ADR template and a governance ADR template logically share a base configuration — field selection, ordering, default `editorHint` overrides — while diverging on workflow framing and export layout.

A future version may define:
- `extendsTemplateId?: UUID` — single inheritance; the child template inherits all `fieldViews` and configuration from a parent and overrides selectively
- `composesTemplates?: UUID[]` — mixin composition; multiple templates contribute non-overlapping configuration

The current design deliberately omits both. `TemplateDefinition` is a leaf type. Implementations that need inheritance-like behaviour should currently achieve it by copying and modifying templates explicitly, using lineage tracking to record the relationship.

### Instance graph exchange format

A standard envelope for exchanging a container together with its full instance set, relationships, and source references — for cases where the complete graph, not just current state, is the interoperability concern. Natural successor to `DefinitionPackage` at the instance layer. Likely takes the form `{ container, instances[], relations[], sourceRefs[] }` and maps to RO-Crate for archive/dataset publication. Deferred pending stabilisation of Layer 7 and implementation experience.

update

---

# claude response

Now I can see several things to fix. The v1 final has 7 layers not 9, several type renames are missing, the layer source references in Section 3 are wrong, and there's a meaningful structural change (inline-optional → named extension) that deserves its own section.

What changed and why:

**Section 1 — vocabulary table** gained eight missing entries: `FreeformSection` → `NoteSection`, `TemplateFieldView` → `FieldView`, `TemplateExportConfig` → `ExportConfig`, `TemplateFacilitationStep` → absorbed, `ModuleValidationRule` → `CrossFieldRule`, `LifecycleStateDefinition` → `LifecycleState`, `LifecycleTransitionDefinition` → `LifecycleTransition`. These were all named types in v1 with no prior mapping entry.

**Section 3 — layer list corrected** from nine to seven. The v1 final had consolidated Import Tracking and Registry into Layer 4 before being superseded. This also fixed the layer source references in the v2 group table — Relation was Layer 5 not 7, Container was Layer 6 not 8, Document View was Layer 7 not 9.

**Section 4 — new** on inline-optional to named extension. This is the most structurally significant v1→v2 change that wasn't captured before: lifecycle, repeatability, field groups, and cross-field validation all moved from optional inline fields on `ModuleDefinition`/`FieldAssignment` to independently declared extensions. The table makes the mapping explicit.

**Section 5 — new** on invariant changes. Captures the numbering shift, the directionality wording fix (which was actually corrected in v1 final before supersession), and the removal of the `TemplateFacilitationStep` invariant.

---

# you asked

message time: 2026-05-21 11:18:01

# SCDS Specification

**Version**: 2.0-draft
**Status**: active draft
**Scope**: field definitions (Field), type definitions (Type), records (Note / Typed Record / Record), relations, containers, distribution, and eleven optional extensions covering addressability, lifecycle, protocol, schema, views, repeatable fields, field groups, cross-field validation, recommended relations, import tracking, and registry.

> **Migration note**: This document supersedes `scds-schema.md` (v1.0-draft). A vocabulary and structural mapping from v1 to v2 is in `scds-shema-evolution.md`. Design rationale, usage guidance, and commentary are in `scds-rationale.md`.

---

## 1. Purpose and Scope

### What this specification defines

The Semantic Collaborative Document System (SCDS) specification defines an interoperable standard for semantic field and type definitions, records, relations, and the mechanisms by which these artefacts are created, shared, versioned, and distributed across independent implementations.

This specification covers:

- **Field** — atomic reusable semantic unit
- **Type** — named composition of fields for a specific semantic object type
- **Record** — instantiated type with field values; three semantic maturity tiers (Note, Typed Record, Record)
- **Relation** — first-class typed link between records
- **Container** — grouping boundary for record collections
- **Distribution** — Package, Reference, Lineage, Provenance
- **Extensions** — optional, independently adoptable capabilities declared by conforming implementations

### What this specification does not define

- **Session** — live collaborative process model (future version)
- **Registry protocol** — how registries communicate, authenticate, or federate; this specification defines data shapes only
- **Universal semantic ontology** — domain-specific vocabularies are the responsibility of namespace authors

### Relationship to implementing systems

This specification is implementation-neutral. Implementations are expected to validate inputs against these schemas at their system boundaries. The specification does not constrain persistence technology, API design, UI rendering, or prompt assembly strategy.

### Extension conformance model

Implementations declare conformance as:

```
SCDS Core [+ ext:<name> ...]
```

**Core** requires the Foundation group and Distribution group in full. No extension is required for core conformance. Extensions are independently adoptable; some declare dependencies on other extensions.

| Extension | Identifier | Depends on | Notes |
|---|---|---|---|
| Addressability | `ext:addressability` | — | For live facilitation, declare together with `ext:protocol` |
| Lifecycle | `ext:lifecycle` | — | |
| Protocol | `ext:protocol` | `ext:lifecycle` (recommended) | For live facilitation, declare together with `ext:addressability` |
| Schema | `ext:schema` | — | |
| Views L1 | `ext:views-l1` | — | |
| Views L2 | `ext:views-l2` | `ext:views-l1` | |
| Repeatable Fields | `ext:repeatable-fields` | — | |
| Field Groups | `ext:field-groups` | — | |
| Cross-Field Validation | `ext:cross-field-validation` | — | |
| Recommended Relations | `ext:recommended-relations` | — | |
| Import Tracking | `ext:import-tracking` | — | |
| Registry | `ext:registry` | — | |

`ext:protocol` and `ext:addressability` are formally independent but are a functional co-dependency for live facilitation: a Protocol without `AttentionState` produces no live conversation tagging; `AttentionState` without Protocol stages has no stage context to capture. Implementations supporting live facilitation should declare both.

Example declaration: `SCDS Core + ext:lifecycle + ext:protocol + ext:views-l1 + ext:addressability`

---

## 2. Namespace Format

### Convention

Namespaces are dot-separated identifiers using lowercase alphanumeric characters and hyphens.

```
<component>[.<component>]*

component = [a-z0-9][a-z0-9-]*
```

Examples:
```
core
community.adr
com.acme.hr
org.cooperative-name
```

### Reserved namespaces

`core` is reserved for definitions maintained by the SCDS standard. Implementations must not allow user-created definitions in the `core` namespace.

### Reference format

A specific version of a definition is referenced using the canonical form:

```
namespace/name@version
```

Examples:
```
core/decision_statement@2
community.adr/review_rationale@1
com.acme.hr/headcount_impact@3
```

The `/` and `@` characters are reserved separators. They must not appear within a namespace component or a name.

### Name convention

Field and Type names are programmatic keys in `snake_case`. Names are stable within a namespace and version lineage. A new name means a new definition.

---

## 3. Schema Notation

Types are described using TypeScript-style notation. Optional fields are marked with `?`. All `UUID` values are RFC 4122 UUID strings. All `ISO8601` values are datetime strings with timezone offset. `integer` means a positive integer unless otherwise noted.

### Version semantics

Version numbers are positive integers scoped to a definition's UUID lineage.

| Change | Version action |
|---|---|
| Documentation, typo, formatting only | Optional bump |
| `description`, `instructions`, or `aiGuidance.purpose` reworded without semantic change | Minor bump recommended |
| `aiGuidance.extraction` or `aiGuidance.purpose` changed in meaning | Version bump required |
| `valueType`, `selectOptions`, or `validationRules` changed | Version bump required |
| `name` changed | New definition required (new UUID) |
| `namespace` changed | New definition required (new UUID) |

When in doubt: if a downstream consumer's AI extraction, validation, or governance logic would behave differently, a version bump is required.

---

## 4. Foundation Group (Core)

The Foundation group is required for all conforming implementations.

### 4.1 Supporting types

#### `ValidationRule`

A constraint applied to a field value.

```typescript
{
  type: "required" | "minLength" | "maxLength" | "pattern" | "enum"
  value?: string | number | string[]  // required for minLength, maxLength, pattern, enum
  message?: string
}
```

#### `AiGuidanceExample`

A single example for AI guidance.

```typescript
{
  description?: string  // labels this example
  input?: string        // sample source text; omit for output-only examples
  output: string        // the ideal value the AI should produce
}
```

`output` is required. An example without `input` demonstrates expected output form without requiring a specific source.

#### `AiGuidance`

Structured AI guidance for a Field or Type.

```typescript
{
  purpose: string            // what this field/type captures (1-2 sentences)
  extraction?: string        // LLM instruction for how to extract or populate
  negativeGuidance?: string  // what the LLM must NOT include or do
  examples?: AiGuidanceExample[]
}
```

The minimum valid `AiGuidance` is `{ purpose: "..." }`.

---

### 4.2 Field

The atomic reusable semantic unit. Fields are defined once and composed into Types. A Field's `aiGuidance`, `validationRules`, and `valueType` belong to the Field, not to any Type that includes it.

```typescript
{
  // Stable identity
  id: UUID
  namespace: string
  name: string       // snake_case programmatic key
  version: integer   // min: 1; increments within this id's lineage

  // Semantic content
  description: string      // one-sentence user-facing summary
  instructions?: string    // fuller guidance for a human completing this field
  aiGuidance: AiGuidance

  // Value semantics — stable across renderers
  valueType: "string" | "text" | "number" | "boolean" | "date" | "url" | "select" | "multiselect"
  selectOptions?: string[]   // required when valueType is "select" or "multiselect"
  validationRules?: ValidationRule[]

  // Editor hint — projection-specific default; implementations and Views may override
  editorHint?: "singleline" | "textarea" | "rich-text" | "date-picker" | "dropdown" | "multi-select" | "voice"

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: Lineage      // see Distribution group
  provenance?: Provenance
}
```

**`valueType` semantics:**

| Value | Meaning |
|---|---|
| `"string"` | Short single-value text (typically one line) |
| `"text"` | Potentially long multi-paragraph prose |
| `"number"` | Numeric value |
| `"boolean"` | True/false |
| `"date"` | ISO 8601 date or datetime |
| `"url"` | A URL string |
| `"select"` | One value from `selectOptions` |
| `"multiselect"` | One or more values from `selectOptions` |

`valueType` is the stable semantic data type. `editorHint` is a rendering default. AI extraction, validation, and export formatting must depend only on `valueType`.

---

### 4.3 Type

A named, versioned composition of Fields for a specific semantic object type.

```typescript
{
  // Stable identity
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  // Content
  description: string        // when to use this Type; what semantic object it defines
  aiGuidance?: AiGuidance    // Type-level LLM framing; see AI guidance composition in rationale

  // Semantic object type (optional, informative)
  semanticObjectType?: string
  // e.g. "decision", "task", "risk", "budget_line", "requirement"
  // Free-form. Implementations may use as a rendering or grouping hint.
  // No conforming implementation is required to act on it.

  // Composition
  fields: FieldAssignment[]
  // fieldGroups and validationRules are extensions; see ext:field-groups and
  // ext:cross-field-validation

  // lifecycle is an extension; see ext:lifecycle

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

#### `FieldAssignment`

A Field reference within a Type. Configures presentation without redefining field semantics.

```typescript
{
  fieldId: UUID     // references Field.id
  order: integer    // min: 0; display and processing order within the Type
  required?: boolean  // default: true

  // Presentation-only — must NOT affect AI guidance, extraction, valueType, or validation
  displayLabel?: string
  displayHint?: string
}
```

`displayLabel` and `displayHint` are strictly for rendering. If a materially different label or meaning is needed, a distinct Field with its own lineage is required.

Repeatability fields (`repeatable`, `minItems`, `maxItems`) are defined in `ext:repeatable-fields`.

**AI guidance composition order** (recommended):

1. Type framing (`Type.aiGuidance.extraction`) — establishes the semantic object type
2. View framing (`View.aiGuidance.extraction`, if `ext:views-l1` is in use) — workflow-specific context
3. Field extraction guidance (`Field.aiGuidance.extraction`)
4. Negative guidance (`Field.aiGuidance.negativeGuidance`)
5. Examples (`Field.aiGuidance.examples`)

This is a recommended default, not a required invariant. Implementations that compose differently will produce different AI behaviour from the same definitions.

**On instance migration when a Type version changes:**
A Record binds to a specific `typeVersion` at creation time. Existing Records do not automatically migrate when a new Type version is published. Conformance is measured against the version the Record was instantiated under. When a Record is migrated and exchanged, it should carry the version it now conforms to, and the original Record should be preserved and linked via a `supersedes` Relation.

---

### 4.4 Record tiers

SCDS supports three semantic maturity tiers. Implementations are not required to support all three; they may begin at Tier 2.

| Tier | Type | Structure | Semantics |
|---|---|---|---|
| **0** | `Note` | Named sections + free text | None |
| **1** | `Typed Record` | Named fields with types and values | Minimal |
| **2** | `Record` | Fields bound to a `Type` definition | Full |

Graduation path: Note → Typed Record → Record.

#### `NoteSection`

A named text section within a Note.

```typescript
{
  name: string          // section key; unique within the Note; snake_case recommended
  label?: string
  content: string
  contentHint?: "text" | "markdown" | "plain"  // hint only; default: "text"
}
```

#### `Note`

A lightweight instance with no Type binding.

```typescript
{
  instanceId: UUID

  title?: string
  sections: NoteSection[]

  graduatedAt?: ISO8601
  // When set, signals full formalisation. Authoritative record of successors
  // is in derived-from Relations from the successor Records.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

#### `TypedField`

A field within a Typed Record.

```typescript
{
  name: string
  label?: string
  valueType?: "string" | "text" | "number" | "boolean" | "date" | "url" | "select" | "multiselect"
  selectOptions?: string[]
  value: string | number | boolean | string[] | null
  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
}
```

#### `Typed Record`

A structured instance with named, typed fields but no Type binding.

```typescript
{
  instanceId: UUID

  title?: string
  instanceType?: string  // lightweight semantic hint; not a formal type declaration

  fields: TypedField[]

  graduatedAt?: ISO8601

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

#### `SourceReference`

A pointer from a field value or instance back to source material.

```typescript
{
  sourceType: "transcript-chunk" | "transcript-segment" | "external-document"
  sourceId: string
  sourceStandard?: string   // versioned standard the source conforms to
  streamId?: UUID           // for transcript sources: originating stream

  relationType?: "evidence" | "derived-from" | "quoted-from" | "inspired-by" | "supersedes-context"

  confidence?: number       // 0.0–1.0
  note?: string
}
```

#### `FieldValue`

The current value of a Field within a Record.

```typescript
{
  fieldId: UUID

  // Non-repeatable — use value
  value?: string | number | boolean | string[] | null

  // Repeatable — use entries (ext:repeatable-fields)
  entries?: FieldValueEntry[]

  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601

  sourceRefs?: SourceReference[]
}
```

`FieldValueEntry` is defined in `ext:repeatable-fields`.

#### `Record`

An instantiated Type with field values.

```typescript
{
  instanceId: UUID
  typeId: UUID         // references Type.id
  typeVersion: integer
  typeNamespace: string
  typeName: string

  // lifecycleState is ext:lifecycle
  lifecycleState?: string

  fieldValues: FieldValue[]

  // groupValues is ext:field-groups
  groupValues?: FieldGroupValue[]

  sourceRefs?: SourceReference[]

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
  // Use meta for implementation-local concerns: lock state, visibility,
  // session references. Cross-system keys should be namespaced,
  // e.g. "com.acme.locking.locked-by".
}
```

`typeNamespace` and `typeName` are denormalised convenience fields. If they conflict with the resolved Type, the `typeId`/`typeVersion` identity takes precedence and the Record is considered invalid until corrected.

**On instance revision:**
- **In-place edits** (`updatedAt` advances, `fieldValues` mutate): for minor corrections that do not alter semantic meaning.
- **Semantic updates**: produce a new Record linked to the prior by a `supersedes` or `refines` Relation. The prior Record remains valid.
- **Immutable records + Relation graph**: all Records append-only; a new Record for every change. A valid implementation strategy that naturally preserves history.

---

### 4.5 Relation

A first-class typed link between instances. Relations allow implementations to construct semantic graphs for navigation, analysis, projection, and reasoning.

```typescript
{
  relationId: UUID

  relationType: string
  // Free-form. See ext:recommended-relations for canonical types and conventions.

  // source [relationType] target
  sourceInstanceId: UUID    // the asserting instance
  targetInstanceId: UUID    // the related instance

  assertedBy?: "human" | "ai" | "imported"
  confidence?: number       // 0.0–1.0; meaningful for ai-asserted
  createdAt?: ISO8601
  createdBy?: string

  status?: "proposed" | "active" | "rejected" | "superseded"
  validFrom?: ISO8601
  validUntil?: ISO8601

  notes?: string
  sourceRefs?: SourceReference[]
  meta?: Record<string, unknown>
}
```

**Directionality convention:**
`sourceInstanceId` is the asserting instance; `targetInstanceId` is the related instance. The Relation reads: "source [relationType] target."

| Relation | source | target |
|---|---|---|
| `supersedes` | the newer Record | the older Record |
| `contains` | the stage | the task inside it |
| `depends-on` | the dependent task | the task it needs |
| `refines` | the detailed version | the rough version |
| `derived-from` | the successor | the source Note or Record |
| `evidences` | the source material | the claim it supports |

This convention must be consistent across implementations. See Invariant 16.

Relations span tiers. A Note may be the target of `derived-from` Relations from the Records it graduated into.

**Canonical relation types** (use these exact strings for cross-system interoperability):

`contains`, `depends-on`, `supersedes`, `refines`, `derived-from`, `evidences`, `precedes`

Custom types not covered by these should use `namespace/name` format (e.g. `com.acme.hr/transferred-to`) to prevent collision. Extended relation type metadata is defined in `ext:recommended-relations`.

**Relations do not change lifecycle state.** A `supersedes` Relation does not mutate the prior Record's `lifecycleState`. Lifecycle state changes are explicit acts by an implementation's transition mechanism.

---

### 4.6 Container

A lightweight grouping boundary over a collection of instances. Containers answer scoping questions — which instances belong together, what constitutes "this project" — that the Relation graph alone cannot answer.

Containers are not semantic objects with Fields. They do not own semantic state; Records do. A `contains` Relation asserts "A is part of B" (a semantic claim); a Container asserts "these instances form a unit for boundary purposes" (a scope claim). Both are needed; neither replaces the other.

```typescript
{
  containerId: UUID

  namespace?: string
  name?: string

  title: string              // human-readable label

  containerType?: string     // free-form hint; e.g. "project", "meeting", "sprint"

  rootInstanceIds?: UUID[]
  // Top-level instances this Container was created to hold. Implementations may
  // derive nested members by traversing contains Relations from these roots.

  memberInstanceIds?: UUID[]
  // Explicit membership list for all instances in scope.
  // When present, allows membership queries without graph traversal.
  // When omitted, membership is defined by traversing contains Relations.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

`Container.containerId` is not an instance ID and must not appear in `Relation.sourceInstanceId` or `targetInstanceId`. See Invariant 19.

---

## 5. Distribution Group (Core)

The Distribution group is required for all conforming implementations.

### 5.1 Package

The distributable artefact. Contains Field, Type, View, and Relation type definitions with a complete dependency manifest.

```typescript
{
  schemaVersion: string      // SCDS spec version, e.g. "2.0"
  packageId: UUID
  packageName: string
  packageVersion: string     // semver, e.g. "1.2.0"
  publishedAt: ISO8601
  publisher?: string
  description?: string
  homepage?: string

  // Content (at least one of fields or types must be non-empty)
  fields: Field[]
  types: Type[]
  views?: View[]             // ext:views-l1; omit if not in use
  schemas?: Schema[]         // ext:schema; omit if not in use
  protocols?: Protocol[]     // ext:protocol; omit if not in use
  relationTypes?: RelationTypeDefinition[]  // ext:recommended-relations

  mode: "bundled" | "standalone"

  dependencyRefs: Reference[]
}
```

**`mode` semantics:**

| Mode | Meaning |
|---|---|
| `"bundled"` | All Field records referenced by any Type, and all Type records referenced by any View, are included in `fields[]` and `types[]`. Self-contained. |
| `"standalone"` | Dependencies are expected pre-installed in the consumer's registry. `dependencyRefs` is the required manifest. |

`dependencyRefs` is required in both modes. Consumers use it to validate completeness without parsing content internals.

---

### 5.2 Reference

A stable pointer to a specific definition version.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1
  definitionType?: "field" | "type" | "view" | "schema" | "protocol"
}
```

Canonical string form: `namespace/name@version`

---

### 5.3 Lineage

Upstream and fork tracking for a specific definition version.

```typescript
{
  sourceDefinitionId?: UUID     // UUID of the upstream definition
  sourceVersion?: integer       // upstream version at derivation time
  forkedFromDefinitionId?: UUID // UUID of the definition deliberately forked from
  forkedFromVersion?: integer   // version at the fork point
}
```

| Field pair | Meaning |
|---|---|
| `sourceDefinition*` | Tracked copy; consumer expects upstream updates |
| `forkedFrom*` | Deliberately diverged; no upstream tracking |

Both may be present during a transition from tracking to forking.

---

### 5.4 Provenance

Publisher and package origin metadata.

```typescript
{
  publisher?: string        // namespace or org of the original author
  sourcePackage?: string    // package name that bundled this definition
  packageVersion?: string   // semver of the source package
  importedAt?: ISO8601
}
```

`packageVersion` is distinct from `Field.version`. A package at `1.3.0` may contain `decision_statement@3` and `context@2`.

---

## 6. Conversation Layer

The conversation layer is a permanent architectural boundary distinct from SCDS. It captures raw multimodal source material; SCDS captures negotiated semantic state. They reference each other bidirectionally via `SourceReference` (document → conversation) and `AttentionState` tags (conversation → document, via `ext:addressability`).

```
Conversation layer  →  raw multimodal source material (speech, threads, annotations)
                        elements tagged with Address at production time
Protocol layer      →  structures the facilitation process; advances AttentionState
SCDS layer          →  captures negotiated semantic state; Records carry SourceReferences
Presentation layer  →  renders SCDS state via Views
```

Three conversation types are in scope:

| Type | Structure | Anchoring |
|---|---|---|
| Meeting transcript | Linear, time-ordered chunks | Tagged with AttentionState at production time |
| Threaded conversation | Tree of replies | Thread root anchored to a document element Address |
| Web UI annotations | Attached to content | Anchored to a Field or Record Address |

Transcript chunks referenced in `SourceReference` are source material — addressable evidence. They do not become Notes or Records automatically. A transcript chunk referenced in `sourceRefs` is evidence supporting a field value; it is not itself a Note unless someone deliberately models it as one.

---

## 7. Extensions

Extensions are optional, independently adoptable. Each extension section declares its identifier, dependencies, and the types it defines.

---

### ext:addressability

**Required for**: any implementation with live facilitation or multi-session extraction.

Defines a universal addressing scheme and the mechanisms that connect conversation material to document elements.

#### `Address`

A stable, resolvable identifier for any element across document space, process space, and conversation space.

```typescript
type Address =
  | {
      space: "document"
      containerId: UUID
      recordId?: UUID
      fieldId?: UUID
      revisionId?: UUID    // requires ext:addressability Revision
    }
  | {
      space: "process"
      runId: UUID          // Protocol run ID; requires ext:protocol
      stageId?: string
    }
  | {
      space: "conversation"
      sessionId: UUID
      chunkId?: UUID
      annotationId?: UUID
    }
```

Every element that can be referred to has an Address. A transcript chunk and a field Revision are co-addressable because assertions about one referencing the other require both to be resolvable.

#### `AttentionState`

The current focus of an active Protocol run — a live cursor across the address space. `AttentionState` and `Address` are structurally related but serve distinct roles: an `Address` is a stable, resolvable identifier for a specific element; `AttentionState` is the mutable cursor that records *where focus currently is* during an active session. An `AttentionState` value at a point in time resolves to a document-space `Address`, but it is stored separately because it changes continuously as the Protocol advances.

Conversation material is tagged with the active `AttentionState` as it is produced. This makes context assembly efficient: "all chunks produced while focus was on this Field" is a queryable address predicate.

```typescript
{
  containerId: UUID
  recordId?: UUID
  fieldId?: UUID
  protocolRunId?: UUID
  stageId?: string
}
```

`AttentionState` is set live by the session or Protocol runner. `SourceReference` is set retrospectively at extraction or editorial review time. Both are needed; they answer different questions.

#### `Revision`

A first-class, addressable snapshot of a `FieldValue` at a point in time. Carries the value, the agent, a timestamp, and source references to the conversation that produced the change.

```typescript
{
  revisionId: UUID
  fieldId: UUID
  recordId: UUID

  value: FieldValue
  agent: "human" | "ai" | "imported"
  createdAt: ISO8601

  sourceRefs?: SourceReference[]
  priorRevisionId?: UUID  // chain to the previous Revision for this field
}
```

Revision does not replace the edit-in-place vs. new-Record judgment. Minor corrections remain in-place edits at the implementation layer. Revision is the addressable audit trail for interoperability — it makes field history queryable: "what did this field say before the last Protocol run?", "which conversation produced the change from revision 2 to revision 3?"

#### Context Query (behavioural requirement)

A conforming `ext:addressability` implementation must be able to assemble relevant material given an address and a purpose. This is a behavioural requirement, not a data shape.

**Required query patterns:**

| Pattern | Address | Returns |
|---|---|---|
| Field context | `{recordId}/{fieldId}` | Current value, Revision history, chunks tagged to this Field, Field `aiGuidance` |
| Record context | `{recordId}` | All field values, chunks tagged to this Record, Relations, Protocol run history |
| Stage context | `{runId}/{stageId}` | All chunks produced during this stage, Fields active in this stage |
| Revision trace | `{fieldId}/{revisionId}` | Value at that Revision, the conversation that produced it, prior Revision chain |

**Recommended assembly order for AI assistance:**

1. Type and Field `aiGuidance` — what this field captures, how to extract it
2. Current value and recent Revision history — what has already been established
3. Chunks tagged to this Field via AttentionState — most focused context
4. Chunks tagged to the parent Record — broader session context
5. Related Records via Relations — structural context

---

### ext:lifecycle

**Required for**: governance tools, decision logs, any implementation where records progress through defined states.

Adds lifecycle state declarations to `Type` and lifecycle state tracking to `Record`.

#### `LifecycleState`

```typescript
{
  name: string
  description?: string
  isInitial?: boolean   // valid starting state for new Records
  isFinal?: boolean     // no transitions out; Record is settled
}
```

#### `LifecycleTransition`

```typescript
{
  name: string       // e.g. "promote", "approve", "supersede"
  from: string       // must match a state name in the enclosing lifecycle
  to: string
  description?: string
}
```

#### Type lifecycle block (added by this extension)

When `ext:lifecycle` is in use, `Type` gains:

```typescript
lifecycle?: {
  states: LifecycleState[]           // min 1 state
  transitions: LifecycleTransition[]
  initialState: string               // must reference a state name where isInitial === true
}
```

#### Record lifecycle state (added by this extension)

`Record.lifecycleState` becomes meaningful: must match a state name in the associated `Type.lifecycle.states[]` when the Type declares a lifecycle.

The `lifecycle` block declares vocabulary. Implementations decide enforcement strictness. A state with `isFinal: true` signals that no further transitions are expected; implementations may use this to lock Record content.

---

### ext:protocol

**Required for**: facilitation tools, structured deliberation, any implementation that guides users through epistemic stages.

Replaces `TemplateFacilitationStep` from v1. Protocol is epistemically richer: stages have explicit dependencies, completion criteria, and may produce intermediate Records.

#### `TypeRef`

A reference to a specific Type, used within Protocol and Schema.

```typescript
{
  typeId: UUID
  typeVersion?: integer
}
```

#### `FieldRef`

A reference to a Field within a Type.

```typescript
{
  fieldId: UUID
  typeId?: UUID    // which Type this Field appears in
}
```

#### `ProtocolStage`

A named stage in a Protocol. Stages have epistemic dependencies (`dependsOn`) — not just ordering. A stage may only proceed when its dependencies are sufficient.

```typescript
{
  stageId: string       // stable key within this Protocol
  order: integer        // min: 0; display/presentation order only — see note below
  purpose: string       // what understanding this stage builds
  question: string      // the core question this stage answers
  dependsOn: string[]   // stageId values; epistemic dependencies, not just ordering
  completionCriteria: string   // how to know this stage is sufficient to proceed
  contributesTo: FieldRef[]    // which Record Fields this stage feeds
  outputType?: TypeRef         // if this stage produces its own intermediate Record
  aiGuidance: AiGuidance
}
```

**`order` vs `dependsOn`:** `order` is the display and presentation sequence — how stages are shown in a UI or facilitation guide. Execution sequence is determined by `dependsOn` resolution: a stage runs when all its declared dependencies are satisfied, regardless of its `order` value. Authors must ensure `order` is consistent with the partial order implied by `dependsOn` (i.e. a stage's `order` value should be greater than the `order` of any stage it depends on). See Invariant 31.

#### `Protocol`

An epistemically ordered process for building quality Records through structured conversation or facilitation.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string

  targetType?: TypeRef
  // The Record type this Protocol produces. Absent for loose / exploratory Protocols
  // (Brain Dump, Decomposition) whose output is input context for a tighter Protocol.

  stages: ProtocolStage[]

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

**The Protocol spectrum:**

```
Loose                                                    Tight
─────────────────────────────────────────────────────────────
Brain Dump → Decomposition → Options Analysis → Decision
```

Loose Protocols produce open material. Tight Protocols converge on a specific Record type. The output of a loose Protocol is the input context for something tighter.

**Generic Protocols** (reusable across domains):
- Brain Dump — externalise all thinking without constraint
- Decomposition — identify major components from raw material
- Review — what is established, what is still open
- Prioritisation — which components to resolve first

**Domain-specific Protocols** (target a specific Record type):
- Decision — context → criteria → options → evaluation → decision
- Proposal — problem → solution shape → constraints → proposal

**Protocol chaining and provenance**: The output of one Protocol is the input context for the next. This derivation chain is traceable through `derived-from` Relations, making the quality and history of the final Record auditable.

**Non-normative example — Protocol chain for a governance decision:**

```
Brain Dump Protocol (loose, no targetType)
  → AttentionState: { containerId: C1 }
  → Produces: Note N1 (unstructured brainstorm)

Decomposition Protocol (loose, targetType: Component)
  → AttentionState: { containerId: C1, recordId: N1 }
  → Produces: Notes N2, N3, N4  [derived-from N1]

Decision Protocol (tight, targetType: Decision)
  → AttentionState: { containerId: C1, protocolRunId: R1, stageId: "criteria" }
  → Stage "criteria" produces: Options Analysis Record R-OA  [derived-from N2, N3]
  → Stage "decision" produces: Decision Record R-D           [derived-from R-OA]

Conversation chunks produced during Decision stage:
  chunk-42: { AttentionState: { containerId: C1, recordId: R-OA, fieldId: F-criteria, ... } }
  chunk-43: { AttentionState: { containerId: C1, recordId: R-D, fieldId: F-outcome, ... } }

Context query for R-D / F-outcome:
  → Field aiGuidance from Decision Type + outcome Field
  → Current value + Revision history for F-outcome
  → Chunks tagged with { recordId: R-D, fieldId: F-outcome } — chunk-43
  → Chunks tagged with { recordId: R-D } — broader session context
  → Related Records via Relations — R-OA via derived-from
```

The final Decision Record is auditable because every Protocol stage left addressable artefacts. The quality of the outcome is traceable to the conversation that produced it.

Views (`ext:views-l1`) no longer contain facilitation logic. A View is a presentation concern; a Protocol is an epistemic one.

---

### ext:schema

**Required for**: extraction pipelines, founding document workflows, any system that needs to specify what a document type IS before assembling it.

#### `RelationSpec`

Declares an expected Relation between two Record types within a Schema.

```typescript
{
  relationType: string
  sourceType: TypeRef
  targetType: TypeRef
  cardinality?: "one-to-one" | "one-to-many" | "many-to-many"
  required?: boolean
}
```

#### `Schema`

The definition of a complete document type — which Types it contains, what Relations exist between resulting Records, and what "complete" means. A Schema is the artefact handed to an extraction pipeline.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string

  rootTypes: TypeRef[]        // Types to extract
  structure: RelationSpec[]   // expected Relations between extracted Records
  requiredTypes: TypeRef[]    // what "complete" means for this document type

  aiGuidance?: AiGuidance
  // purpose: what kind of document this Schema defines
  // extraction: framing for extraction pipelines

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

**Schema vs View:**

| | Schema | View / Document View |
|---|---|---|
| Question it answers | What IS this document type? What should be extracted? | How are existing Records assembled into readable output? |
| Operates at | Definition time | Projection time |
| Input | Source material (transcripts, conversations) | Existing Records in a Container |
| Output | Extraction instructions → Records | Rendered document |

---

### ext:views-l1

**Required for**: rendering and export workflows.

Defines Views — versioned presentations of a single Record through a specific Type.

#### `FieldView`

A field reference within a View. Controls presentation for this View without altering field semantics.

```typescript
{
  fieldId: UUID       // must appear in the Type's fields[]
  order: integer      // min: 0; display order within this View
  required?: boolean  // View-level workflow constraint; does not alter Field contract
  visible?: boolean   // default: true

  // Presentation overrides — View scope only
  displayLabel?: string
  displayHint?: string
  editorHintOverride?: string
}
```

A Field hidden with `visible: false` remains in the Record and may appear in other Views.

#### `ExportConfig`

Configuration for rendering a Record through this View as an exportable document.

```typescript
{
  format?: string        // target format hint, e.g. "markdown", "adoc", "json"
  preamble?: string
  // Template string rendered before field values.
  // Variable substitution uses {{variable-name}} syntax.
  // Standard variables: {{instance-id}}, {{date}}, {{status}}, {{namespace}}, {{name}}

  fieldOrder?: UUID[]    // explicit export field ordering; defaults to fieldViews[].order
  omitEmptyFields?: boolean  // default: false
}
```

#### `View`

A versioned presentation and export configuration over a specific Type. Multiple Views may exist for the same Type, serving different audiences or purposes.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string    // when to use this View; what workflow or audience it serves

  typeId: UUID           // references Type.id
  typeVersion: integer

  aiGuidance?: AiGuidance
  // purpose: the workflow context this View serves
  // extraction: session-level framing injected before field extraction

  fieldViews: FieldView[]

  exportConfig?: ExportConfig

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

A View may not introduce Fields that are not in the bound Type. Omitted Fields are treated as `visible: false`.

Facilitation steps have been removed from View. Use `ext:protocol` Protocol stages instead.

---

### ext:views-l2

**Depends on**: `ext:views-l1`

**Required for**: document projection — assembling multiple Records into a coherent document.

#### `SectionSource`

Defines how a section's instances are selected from a Container.

```typescript
type SectionSource =
  | {
      type: "fixed-instances"
      instanceIds: UUID[]
      // Explicit list. For preamble, cover page, or curated sections.
    }
  | {
      type: "type-query"
      semanticObjectType: string
      // For cross-system portability, use namespace/name format (e.g. "core/decision").
      // A bare string like "decision" is a single-system convention.
      lifecycleState?: string
      containerIds?: UUID[]
    }
  | {
      type: "relation-query"
      fromInstanceId: UUID
      relationType: string
      direction?: "forward" | "inverse"  // default: "forward"
    }
  | {
      type: "container-subset"
      containerId: UUID
      containerType?: string
    }
```

#### `DocumentSection`

One section in a Document View.

```typescript
{
  sectionId: string
  title?: string
  description?: string
  order: integer   // min: 0

  source: SectionSource

  renderViewId?: UUID    // View (ext:views-l1) used to render each instance in this section
  // When absent, implementations use a default rendering for the instance type.

  ordering?: {
    fieldId?: UUID
    direction?: "asc" | "desc"  // default: "asc"
  }

  required?: boolean
  emptyBehavior?: "hide" | "show-placeholder"
}
```

#### `DocumentView`

A versioned, Container-level projection. Defines how a Container's Records are assembled into a readable document.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string    // what kind of document this produces; intended audience

  containerType?: string  // when set, intended for Containers of this type

  sections: DocumentSection[]

  preamble?: string
  // Template string rendered before all sections.
  // Standard variables: {{container-title}}, {{date}}, {{container-id}}

  format?: string   // e.g. "markdown", "adoc", "html"

  aiGuidance?: AiGuidance
  // purpose: what kind of document this View produces
  // extraction: context for AI-assisted document-level tasks

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

A `DocumentView` may reference multiple `View` records (via `DocumentSection.renderViewId`) — one per instance type in the document. It orchestrates; it does not replace them.

`DocumentSection.renderViewId` references a `View.id` (from `ext:views-l1`). A `DocumentView.id` is not a valid value for `renderViewId` — Document Views are not nestable.

---

### ext:repeatable-fields

**Required for**: any Record type that needs lists of values within a single Field.

Adds repeatability to `FieldAssignment` and defines `FieldValueEntry`.

#### `FieldValueEntry`

A single entry in a repeatable field.

```typescript
{
  value: string | number | boolean | string[] | null
  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
}
```

#### FieldAssignment additions

When `ext:repeatable-fields` is in use, `FieldAssignment` gains:

```typescript
repeatable?: boolean  // default: false; when true, multiple values are allowed
minItems?: integer    // meaningful only when repeatable === true
maxItems?: integer    // meaningful only when repeatable === true
```

And `FieldValue.entries` becomes active: use `entries` when `repeatable === true`, `value` otherwise.

A repeatable field entry does not create a new semantic instance. Use separate Records connected by Relations when repeated items need their own identity, lifecycle, or graph position.

---

### ext:field-groups

**Required for**: Record types where multiple Fields are semantically paired and repeat together as a unit.

Use when parallel `multiselect` arrays would lose pairing (e.g. a contact record with `name` + `email`). Preserves internal pairing across repeated items.

#### `FieldGroup`

A named, ordered group of Fields that repeat together as a unit within a Type.

```typescript
{
  groupId: string        // stable key within the Type
  label?: string
  description?: string

  order: integer         // min: 0; position relative to other Fields and Groups

  required?: boolean     // default: false
  repeatable?: boolean   // default: false
  minItems?: integer
  maxItems?: integer

  fields: FieldAssignment[]
}
```

#### `FieldGroupEntry`

One entry in a repeatable Field Group.

```typescript
{
  entryId?: UUID         // stable key for this entry; allows referencing or updating
  fieldValues: FieldValue[]
}
```

#### `FieldGroupValue`

The current value of a Field Group within a Record.

```typescript
{
  groupId: string           // references FieldGroup.groupId in the Type definition
  entries: FieldGroupEntry[]
}
```

A `FieldGroup` does not create a new semantic instance. Its entries are embedded structured context within the enclosing Record. Use separate Records connected by Relations when group entries need their own identity, lifecycle, provenance, or reuse across Records.

When `ext:field-groups` is in use, `Type` gains `fieldGroups?: FieldGroup[]` and `Record` gains `groupValues?: FieldGroupValue[]`.

**Repeatability pattern guide:**

| Pattern | Use | Example |
|---|---|---|
| Repeatable scalar | `FieldAssignment.repeatable` (ext:repeatable-fields) | Multiple assigned person names |
| Repeatable structured context | `FieldGroup` | Contacts with name + email pairs |
| Repeated semantic objects | Separate Records + Relations | Tasks assigned to roles |

---

### ext:cross-field-validation

**Required for**: Types with constraints that span multiple Fields.

`ValidationRule` handles single-field constraints. `CrossFieldRule` handles constraints that require evaluating more than one Field together.

#### `CrossFieldRule`

```typescript
{
  type: "conditional-required" | "field-ordering" | "mutual-exclusion"
  message?: string

  // conditional-required: targetFieldId becomes required when predicateFieldId equals predicateValue
  predicateFieldId?: UUID
  predicateValue?: string
  targetFieldId?: UUID

  // field-ordering: targetFieldId must precede or follow predicateFieldId
  // Applies only to fields with valueType "date" or "number".
  effect?: "must-precede" | "must-follow"

  // mutual-exclusion: at most one of the listed fields may have a non-empty value
  fieldIds?: UUID[]   // min: 2
}
```

| Rule type | Required fields |
|---|---|
| `conditional-required` | `predicateFieldId`, `predicateValue`, `targetFieldId` |
| `field-ordering` | `predicateFieldId`, `targetFieldId`, `effect` |
| `mutual-exclusion` | `fieldIds` (min 2) |

When `ext:cross-field-validation` is in use, `Type` gains `validationRules?: CrossFieldRule[]`.

---

### ext:recommended-relations

**Required for**: cross-system federation; multi-publisher ecosystems where Relation type semantics must be interoperable.

Canonical relation types and machine-readable Relation type definitions.

**Canonical relation types** (use exact strings):

| Canonical | Converse | Category |
|---|---|---|
| `contains` | `part-of` | Composition |
| `depends-on` | `required-by` | Dependency |
| `supersedes` | `superseded-by` | Governance |
| `refines` | `refined-by` | Refinement |
| `derived-from` | `source-of` | Derivation |
| `evidences` | `evidenced-by` | Evidence |
| `precedes` | `follows` | Sequence |

Implementations must store only the canonical (forward) form and derive the inverse when needed.

**Relation category taxonomy:**

| Category | Examples |
|---|---|
| Composition | `contains`, `part-of`, `has-section` |
| Refinement | `refines`, `expands`, `summarises` |
| Dependency | `depends-on`, `requires`, `blocks`, `enables` |
| Sequence | `precedes`, `follows`, `overlaps` |
| Derivation | `derived-from`, `extracted-from`, `based-on` |
| Evidence | `evidences`, `supports`, `contradicts` |
| Governance | `supersedes`, `amends`, `ratifies`, `delegates` |
| Association | `relates-to`, `links-to` |

#### `RelationTypeDefinition`

Machine-readable metadata for a `relationType` string.

```typescript
{
  relationType: string      // exact string used in Relation.relationType
  namespace: string
  label?: string
  description?: string
  category?: "composition" | "refinement" | "dependency" | "sequence" | "derivation" | "evidence" | "governance" | "association"
  canonicalDirection?: string   // e.g. "source is the dependent task; target is the task it depends on"
  inverseType?: string
}
```

`RelationTypeDefinition` is optional metadata. Implementations are not required to resolve `relationType` strings against a definition before accepting a Relation. Relation type definitions may be included in a Package or published separately.

---

### ext:import-tracking

**Required for**: implementations that receive packages from upstream publishers and need to track update and conflict state.

#### `ImportMode`

```typescript
"upstream-tracked" | "local-copy" | "local-fork"
```

| Mode | Meaning |
|---|---|
| `"upstream-tracked"` | Consumer expects updates from the source Package. Conflicts surfaced when local and upstream diverge. |
| `"local-copy"` | Imported as a snapshot. No update tracking. |
| `"local-fork"` | Deliberately diverged. Upstream lineage preserved for reference. |

#### `ImportRecord`

One record per imported definition in a consumer's local registry.

```typescript
{
  definitionId: UUID
  definitionType: "field" | "type" | "view" | "schema" | "protocol"
  namespace: string
  name: string
  version: integer

  mode: ImportMode
  importedAt: ISO8601

  sourcePackageId: UUID
  sourcePackageName: string
  sourcePackageVersion: string

  latestKnownUpstreamVersion?: integer
  updateAvailable?: boolean
  updateCheckedAt?: ISO8601

  conflictState?: "clean" | "local-ahead" | "upstream-ahead" | "diverged"
  conflictDetectedAt?: ISO8601

  localVersion?: integer
  localEditedAt?: ISO8601
}
```

#### `ImportSummary`

A consumer's complete picture of its imported definitions.

```typescript
{
  generatedAt: ISO8601
  fields: ImportRecord[]
  types: ImportRecord[]
  views: ImportRecord[]
  schemas: ImportRecord[]
  protocols: ImportRecord[]
}
```

---

### ext:registry

**Required for**: multi-publisher ecosystems; discoverable definition catalogs.

#### `RegistryEntry`

One entry in a Registry catalog.

```typescript
{
  packageId: UUID
  packageName: string
  packageVersion: string
  publisher: string
  description?: string
  publishedAt: ISO8601
  homepage?: string
  tags?: string[]
  fieldCount: integer       // min: 0
  typeCount: integer        // min: 0
  viewCount?: integer
  schemaCount?: integer
  protocolCount?: integer
  relationTypeCount?: integer
  downloadUrl?: string
  checksum?: string         // SHA-256 hex digest for integrity verification
}
```

#### `Registry`

A registry's published index.

```typescript
{
  schemaVersion: string
  registryId: UUID
  registryName: string
  catalogVersion: string    // registry's own version (semver)
  updatedAt: ISO8601
  homepage?: string
  entries: RegistryEntry[]
}
```

Multiple Registries may coexist. A consumer may index multiple catalogs. The specification does not define registry authority, authentication, or federation.

---

## 8. Key Invariants

Conforming implementations must uphold the following invariants.

### Field semantics

**1.** `FieldAssignment.displayLabel` and `FieldAssignment.displayHint` are for rendering only. They must not affect AI guidance, extraction logic, `valueType` interpretation, or validation.

**2.** A `Type` must not redefine, override, or duplicate the semantic content of any `Field` it includes. If different semantics are needed for a Field in a specific Type context, a distinct `Field` with its own identity and lineage must be created.

**3.** A `Field`'s `aiGuidance` belongs to the Field. Type-level `aiGuidance` provides session framing only.

### Lifecycle (ext:lifecycle)

**4.** `Type.lifecycle.initialState` must reference a `name` that appears in `lifecycle.states[]` and where `isInitial === true`.

**5.** Every `from` and `to` value in `lifecycle.transitions[]` must reference a `name` that appears in `lifecycle.states[]`.

**6.** `Record.lifecycleState`, when present, must reference a `name` in the associated `Type.lifecycle.states[]`.

### Distribution

**7.** Every `fieldId` referenced in any `FieldAssignment` within a `Package.types[]` must appear as the `id` of an entry in `Package.dependencyRefs`.

**8.** If `Package.mode === "bundled"`: every `Reference` in `dependencyRefs` must have a matching `Field` in `fields[]` (matched on `id` and `version`).

**9.** `Field.id` is stable across versions. A new `id` means a new definition, not a new version of an existing one.

### Cross-field validation (ext:cross-field-validation)

**10.** All `fieldId` values in any `CrossFieldRule` within `Type.validationRules[]` must appear in `Type.fields[].fieldId`. Cross-field rules cannot reference Fields outside the Type.

**11.** A `conditional-required` rule must supply `predicateFieldId`, `predicateValue`, and `targetFieldId`. A `field-ordering` rule must supply `predicateFieldId`, `targetFieldId`, and `effect`. A `mutual-exclusion` rule must supply `fieldIds` with at least two entries.

### Views (ext:views-l1)

**12.** Every `fieldId` in `View.fieldViews[]` must appear in the bound Type's `fields[].fieldId`. A View cannot introduce Fields not in its Type.

**13.** `FieldView.displayLabel`, `FieldView.displayHint`, and `FieldView.editorHintOverride` are for rendering only. They must not affect AI guidance, extraction logic, `valueType` interpretation, or validation.

**14.** A `View` must not override, redefine, or duplicate the semantic content of any `Field` or `Type` it references. View-level `aiGuidance` is workflow framing; it does not redefine Field extraction semantics.

### Distribution — Views (ext:views-l1)

**15.** Every `typeId` referenced by any `View` in `Package.views[]` must appear in `Package.dependencyRefs` with `definitionType: "type"`. If `mode === "bundled"`, that `Type` must be present in `types[]`.

### Relations

**16.** In a `Relation`, `sourceInstanceId` is the asserting instance and `targetInstanceId` is the related instance. The Relation reads: "source [relationType] target." This convention must not be reversed.

**17.** `Relation` is reserved for assertions that carry semantic consequence beyond simple mention or citation. Lightweight prose references that do not assert structural, causal, or governance relationships must not be modelled as `Relation` records.

### Notes and Typed Records

**18.** `NoteSection.name` values must be unique within a `Note`.

**19.** `TypedField.name` values must be unique within a `Typed Record`.

### Containers

**20.** `Container.containerId` is not an instance ID. It must not appear in `Container.rootInstanceIds`, `Container.memberInstanceIds`, `Relation.sourceInstanceId`, or `Relation.targetInstanceId`.

**21.** `Container.rootInstanceIds` and `Container.memberInstanceIds`, when present, must reference valid SCDS instance IDs (`Note.instanceId`, `Typed Record.instanceId`, or `Record.instanceId`).

### Repeatability (ext:repeatable-fields)

**22.** If `FieldAssignment.repeatable` is false or absent, its corresponding `FieldValue` must use `value` and must not include `entries`.

**23.** If `FieldAssignment.repeatable` is true, its corresponding `FieldValue` may use `entries`. If `minItems` is specified, `entries` must contain at least that many items. If `maxItems` is specified, `entries` must not exceed that count.

**24.** `FieldAssignment.minItems` and `maxItems` are valid only when `repeatable === true`. They must be ignored when `repeatable` is false or absent.

### Field groups (ext:field-groups)

**25.** Every `groupId` in `Record.groupValues[]` must reference a `groupId` declared in the associated `Type.fieldGroups[]`.

**26.** Within a `FieldGroupEntry.fieldValues[]`, every `fieldId` must appear in the enclosing `FieldGroup.fields[].fieldId`.

**27.** A `FieldGroupValue.entries` list must satisfy `FieldGroup.minItems` and `maxItems` where specified.

### Records

**28.** `Record.typeId` and `Record.typeVersion` are the authoritative Type binding. `typeNamespace` and `typeName` are denormalised convenience fields. If they conflict with the resolved `Type`, the `typeId`/`typeVersion` identity takes precedence and the Record is considered invalid until corrected.

### Protocol (ext:protocol)

**29.** Every `stageId` in `ProtocolStage.dependsOn[]` must reference a `stageId` declared in the enclosing `Protocol.stages[]`. A stage may not declare a dependency on itself.

**30.** Every `fieldId` in `ProtocolStage.contributesTo[]` must reference a `fieldId` that appears in `Protocol.targetType`'s Field list. When `targetType` is absent (loose Protocol), `contributesTo` must be empty.

**31.** For every pair of stages A and B within a `Protocol` where B.dependsOn includes A.stageId, B.order must be greater than A.order. `order` is display order; execution sequence is determined by `dependsOn` resolution. The two must not contradict each other.

### Views L2 (ext:views-l2)

**32.** When a `DocumentView` is included in a `Package` (i.e. published for distribution), any `SectionSource` with `type === "type-query"` must use `namespace/name` format for `semanticObjectType` (e.g. `"core/decision"`, not `"decision"`). Bare strings are acceptable only in single-system `DocumentView` records not intended for distribution. Implementations receiving a distributed `DocumentView` with a bare `semanticObjectType` in a `type-query` section should treat the portability of that section as undefined.

### Addressability (ext:addressability)

**33.** `Revision.priorRevisionId`, when present, must reference a `Revision.revisionId` for the same `fieldId` and `recordId`. Revision chains must be acyclic.

**34.** `AttentionState.containerId` must reference a valid `Container.containerId`. Other Address components (`recordId`, `fieldId`, `protocolRunId`, `stageId`) are optional and may be absent when focus has not yet narrowed.

---

## 9. Conformance

An implementation declares conformance using the following form:

```
SCDS <version> Core [+ ext:<name> ...]
```

Example:
```
SCDS 2.0 Core + ext:lifecycle + ext:protocol + ext:views-l1 + ext:addressability + ext:recommended-relations
```

### Core conformance requirements

A core-conformant implementation must:
- Accept and validate `Field`, `Type`, `Record` (Tier 2), `Relation`, and `Container` inputs against this specification
- Enforce Invariants 1–3, 7–9, 16–21, 28
- Support the Foundation and Distribution groups in full
- Implement the namespace format and reference format correctly
- Not accept `relationType` strings that include `/` except in `namespace/name` format

Support for `Note` (Tier 0) and `Typed Record` (Tier 1) is optional at core conformance level.

### Extension conformance requirements

An implementation declaring a given extension must:
- Accept and validate all types defined by that extension
- Enforce all invariants assigned to that extension
- Respect the declared dependency chain (e.g., `ext:views-l2` requires `ext:views-l1` to also be declared)

### Interoperability note

Two implementations at the same conformance level will produce compatible definitions for exchange. An implementation receiving a Package that includes types from an extension it does not support should surface the unknown content rather than silently discard it.

whoops. that was the old version.

---

# claude response

Actually this looks like the *newer* version, not the old one. Comparing against what you shared earlier, this incorporates all four of the fixes from the last review plus a few additions:

**Fixes from review:**

- **Invariant 31** — `order` vs `dependsOn` conflict resolution is now explicit: "`order` is display and presentation sequence... execution sequence is determined by `dependsOn` resolution" with the consistency requirement
- **AttentionState/Address overlap** — now clarified directly in the `AttentionState` definition: "an `Address` is a stable, resolvable identifier... `AttentionState` is the mutable cursor that records *where focus currently is*"
- **`type-query` federation hole** — closed by **Invariant 32**, which requires `namespace/name` format for `semanticObjectType` when a `DocumentView` is distributed in a Package
- **`DocumentView`** — now has a proper TypeScript type name instead of "Document View" in a heading

**New additions not in the previous version:**

- The co-dependency note for `ext:protocol` + `ext:addressability` is now in the conformance table with a plain-English explanation, not just buried in the rationale
- A non-normative Protocol chain example under `ext:protocol` showing AttentionState tagging, chunk context queries, and the full derivation trace — this is the most operationally useful addition
- `DocumentSection.renderViewId` nesting clarification: "A `DocumentView.id` is not a valid value for `renderViewId` — Document Views are not nestable"

The invariant count went from 32 to 34 (adding the `order`/`dependsOn` consistency rule and the `type-query` portability rule).

So if you have a newer version to share, go ahead — but this one is already more evolved than what came before. Nothing to worry about.

---

# you asked

message time: 2026-05-21 11:19:19

# SCDS Specification

**Version**: 2.0-draft
**Status**: active draft
**Scope**: field definitions (Field), type definitions (Type), records (Note / Typed Record / Record), relations, containers, distribution, and eleven optional extensions covering addressability, lifecycle, protocol, schema, views, repeatable fields, field groups, cross-field validation, recommended relations, import tracking, and registry.

> **Migration note**: This document supersedes `scds-schema.md` (v1.0-draft). A vocabulary and structural mapping from v1 to v2 is in `scds-shema-evolution.md`. Design rationale, usage guidance, and commentary are in `scds-rationale.md`.

---

## 1. Purpose and Scope

### What this specification defines

The Semantic Collaborative Document System (SCDS) specification defines an interoperable standard for semantic field and type definitions, records, relations, and the mechanisms by which these artefacts are created, shared, versioned, and distributed across independent implementations.

This specification covers:

- **Field** — atomic reusable semantic unit
- **Type** — named composition of fields for a specific semantic object type
- **Record** — instantiated type with field values; three semantic maturity tiers (Note, Typed Record, Record)
- **Relation** — first-class typed link between records
- **Container** — grouping boundary for record collections
- **Distribution** — Package, Reference, Lineage, Provenance
- **Extensions** — optional, independently adoptable capabilities declared by conforming implementations

### What this specification does not define

- **Session** — live collaborative process model (future version)
- **Registry protocol** — how registries communicate, authenticate, or federate; this specification defines data shapes only
- **Universal semantic ontology** — domain-specific vocabularies are the responsibility of namespace authors

### Relationship to implementing systems

This specification is implementation-neutral. Implementations are expected to validate inputs against these schemas at their system boundaries. The specification does not constrain persistence technology, API design, UI rendering, or prompt assembly strategy.

### Extension conformance model

Implementations declare conformance as:

```
SCDS Core [+ ext:<name> ...]
```

**Core** requires the Foundation group and Distribution group in full. No extension is required for core conformance. Extensions are independently adoptable; some declare dependencies on other extensions.

| Extension | Identifier | Depends on | Notes |
|---|---|---|---|
| Addressability | `ext:addressability` | — | For live facilitation, declare together with `ext:protocol` |
| Lifecycle | `ext:lifecycle` | — | |
| Protocol | `ext:protocol` | `ext:lifecycle` (recommended) | For live facilitation, declare together with `ext:addressability` |
| Schema | `ext:schema` | — | |
| Views L1 | `ext:views-l1` | — | |
| Views L2 | `ext:views-l2` | `ext:views-l1` | |
| Repeatable Fields | `ext:repeatable-fields` | — | |
| Field Groups | `ext:field-groups` | — | |
| Cross-Field Validation | `ext:cross-field-validation` | — | |
| Recommended Relations | `ext:recommended-relations` | — | |
| Import Tracking | `ext:import-tracking` | — | |
| Registry | `ext:registry` | — | |

`ext:protocol` and `ext:addressability` are formally independent but are a functional co-dependency for live facilitation: a Protocol without `AttentionState` produces no live conversation tagging; `AttentionState` without Protocol stages has no stage context to capture. Implementations supporting live facilitation should declare both.

Example declaration: `SCDS Core + ext:lifecycle + ext:protocol + ext:views-l1 + ext:addressability`

---

## 2. Namespace Format

### Convention

Namespaces are dot-separated identifiers using lowercase alphanumeric characters and hyphens.

```
<component>[.<component>]*

component = [a-z0-9][a-z0-9-]*
```

Examples:
```
core
community.adr
com.acme.hr
org.cooperative-name
```

### Reserved namespaces

`core` is reserved for definitions maintained by the SCDS standard. Implementations must not allow user-created definitions in the `core` namespace.

### Reference format

A specific version of a definition is referenced using the canonical form:

```
namespace/name@version
```

Examples:
```
core/decision_statement@2
community.adr/review_rationale@1
com.acme.hr/headcount_impact@3
```

The `/` and `@` characters are reserved separators. They must not appear within a namespace component or a name.

### Name convention

Field and Type names are programmatic keys in `snake_case`. Names are stable within a namespace and version lineage. A new name means a new definition.

---

## 3. Schema Notation

Types are described using TypeScript-style notation. Optional fields are marked with `?`. All `UUID` values are RFC 4122 UUID strings. All `ISO8601` values are datetime strings with timezone offset. `integer` means a positive integer unless otherwise noted.

### Version semantics

Version numbers are positive integers scoped to a definition's UUID lineage.

| Change | Version action |
|---|---|
| Documentation, typo, formatting only | Optional bump |
| `description`, `instructions`, or `aiGuidance.purpose` reworded without semantic change | Minor bump recommended |
| `aiGuidance.extraction` or `aiGuidance.purpose` changed in meaning | Version bump required |
| `valueType`, `selectOptions`, or `validationRules` changed | Version bump required |
| `name` changed | New definition required (new UUID) |
| `namespace` changed | New definition required (new UUID) |

When in doubt: if a downstream consumer's AI extraction, validation, or governance logic would behave differently, a version bump is required.

---

## 4. Foundation Group (Core)

The Foundation group is required for all conforming implementations.

### 4.1 Supporting types

#### `ValidationRule`

A constraint applied to a field value.

```typescript
{
  type: "required" | "minLength" | "maxLength" | "pattern" | "enum"
  value?: string | number | string[]  // required for minLength, maxLength, pattern, enum
  message?: string
}
```

#### `AiGuidanceExample`

A single example for AI guidance.

```typescript
{
  description?: string  // labels this example
  input?: string        // sample source text; omit for output-only examples
  output: string        // the ideal value the AI should produce
}
```

`output` is required. An example without `input` demonstrates expected output form without requiring a specific source.

#### `AiGuidance`

Structured AI guidance for a Field or Type.

```typescript
{
  purpose: string            // what this field/type captures (1-2 sentences)
  extraction?: string        // LLM instruction for how to extract or populate
  negativeGuidance?: string  // what the LLM must NOT include or do
  examples?: AiGuidanceExample[]
}
```

The minimum valid `AiGuidance` is `{ purpose: "..." }`.

---

### 4.2 Field

The atomic reusable semantic unit. Fields are defined once and composed into Types. A Field's `aiGuidance`, `validationRules`, and `valueType` belong to the Field, not to any Type that includes it.

```typescript
{
  // Stable identity
  id: UUID
  namespace: string
  name: string       // snake_case programmatic key
  version: integer   // min: 1; increments within this id's lineage

  // Semantic content
  description: string      // one-sentence user-facing summary
  instructions?: string    // fuller guidance for a human completing this field
  aiGuidance: AiGuidance

  // Value semantics — stable across renderers
  valueType: "string" | "text" | "number" | "boolean" | "date" | "url" | "select" | "multiselect"
  selectOptions?: string[]   // required when valueType is "select" or "multiselect"
  validationRules?: ValidationRule[]

  // Editor hint — projection-specific default; implementations and Views may override
  editorHint?: "singleline" | "textarea" | "rich-text" | "date-picker" | "dropdown" | "multi-select" | "voice"

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: Lineage      // see Distribution group
  provenance?: Provenance
}
```

**`valueType` semantics:**

| Value | Meaning |
|---|---|
| `"string"` | Short single-value text (typically one line) |
| `"text"` | Potentially long multi-paragraph prose |
| `"number"` | Numeric value |
| `"boolean"` | True/false |
| `"date"` | ISO 8601 date or datetime |
| `"url"` | A URL string |
| `"select"` | One value from `selectOptions` |
| `"multiselect"` | One or more values from `selectOptions` |

`valueType` is the stable semantic data type. `editorHint` is a rendering default. AI extraction, validation, and export formatting must depend only on `valueType`.

---

### 4.3 Type

A named, versioned composition of Fields for a specific semantic object type.

```typescript
{
  // Stable identity
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  // Content
  description: string        // when to use this Type; what semantic object it defines
  aiGuidance?: AiGuidance    // Type-level LLM framing; see AI guidance composition in rationale

  // Semantic object type (optional, informative)
  semanticObjectType?: string
  // e.g. "decision", "task", "risk", "budget_line", "requirement"
  // Free-form. Implementations may use as a rendering or grouping hint.
  // No conforming implementation is required to act on it.

  // Composition
  fields: FieldAssignment[]
  // fieldGroups and validationRules are extensions; see ext:field-groups and
  // ext:cross-field-validation

  // lifecycle is an extension; see ext:lifecycle

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

#### `FieldAssignment`

A Field reference within a Type. Configures presentation without redefining field semantics.

```typescript
{
  fieldId: UUID     // references Field.id
  order: integer    // min: 0; display and processing order within the Type
  required?: boolean  // default: true

  // Presentation-only — must NOT affect AI guidance, extraction, valueType, or validation
  displayLabel?: string
  displayHint?: string
}
```

`displayLabel` and `displayHint` are strictly for rendering. If a materially different label or meaning is needed, a distinct Field with its own lineage is required.

Repeatability fields (`repeatable`, `minItems`, `maxItems`) are defined in `ext:repeatable-fields`.

**AI guidance composition order** (recommended):

1. Type framing (`Type.aiGuidance.extraction`) — establishes the semantic object type
2. View framing (`View.aiGuidance.extraction`, if `ext:views-l1` is in use) — workflow-specific context
3. Field extraction guidance (`Field.aiGuidance.extraction`)
4. Negative guidance (`Field.aiGuidance.negativeGuidance`)
5. Examples (`Field.aiGuidance.examples`)

This is a recommended default, not a required invariant. Implementations that compose differently will produce different AI behaviour from the same definitions.

**On instance migration when a Type version changes:**
A Record binds to a specific `typeVersion` at creation time. Existing Records do not automatically migrate when a new Type version is published. Conformance is measured against the version the Record was instantiated under. When a Record is migrated and exchanged, it should carry the version it now conforms to, and the original Record should be preserved and linked via a `supersedes` Relation.

---

### 4.4 Record tiers

SCDS supports three semantic maturity tiers. Implementations are not required to support all three; they may begin at Tier 2.

| Tier | Type | Structure | Semantics |
|---|---|---|---|
| **0** | `Note` | Named sections + free text | None |
| **1** | `Typed Record` | Named fields with types and values | Minimal |
| **2** | `Record` | Fields bound to a `Type` definition | Full |

Graduation path: Note → Typed Record → Record.

#### `NoteSection`

A named text section within a Note.

```typescript
{
  name: string          // section key; unique within the Note; snake_case recommended
  label?: string
  content: string
  contentHint?: "text" | "markdown" | "plain"  // hint only; default: "text"
}
```

#### `Note`

A lightweight instance with no Type binding.

```typescript
{
  instanceId: UUID

  title?: string
  sections: NoteSection[]

  graduatedAt?: ISO8601
  // When set, signals full formalisation. Authoritative record of successors
  // is in derived-from Relations from the successor Records.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

#### `TypedField`

A field within a Typed Record.

```typescript
{
  name: string
  label?: string
  valueType?: "string" | "text" | "number" | "boolean" | "date" | "url" | "select" | "multiselect"
  selectOptions?: string[]
  value: string | number | boolean | string[] | null
  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
}
```

#### `Typed Record`

A structured instance with named, typed fields but no Type binding.

```typescript
{
  instanceId: UUID

  title?: string
  instanceType?: string  // lightweight semantic hint; not a formal type declaration

  fields: TypedField[]

  graduatedAt?: ISO8601

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

#### `SourceReference`

A pointer from a field value or instance back to source material.

```typescript
{
  sourceType: "transcript-chunk" | "transcript-segment" | "external-document"
  sourceId: string
  sourceStandard?: string   // versioned standard the source conforms to
  streamId?: UUID           // for transcript sources: originating stream

  relationType?: "evidence" | "derived-from" | "quoted-from" | "inspired-by" | "supersedes-context"

  confidence?: number       // 0.0–1.0
  note?: string
}
```

#### `FieldValue`

The current value of a Field within a Record.

```typescript
{
  fieldId: UUID

  // Non-repeatable — use value
  value?: string | number | boolean | string[] | null

  // Repeatable — use entries (ext:repeatable-fields)
  entries?: FieldValueEntry[]

  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601

  sourceRefs?: SourceReference[]
}
```

`FieldValueEntry` is defined in `ext:repeatable-fields`.

#### `Record`

An instantiated Type with field values.

```typescript
{
  instanceId: UUID
  typeId: UUID         // references Type.id
  typeVersion: integer
  typeNamespace: string
  typeName: string

  // lifecycleState is ext:lifecycle
  lifecycleState?: string

  fieldValues: FieldValue[]

  // groupValues is ext:field-groups
  groupValues?: FieldGroupValue[]

  sourceRefs?: SourceReference[]

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
  // Use meta for implementation-local concerns: lock state, visibility,
  // session references. Cross-system keys should be namespaced,
  // e.g. "com.acme.locking.locked-by".
}
```

`typeNamespace` and `typeName` are denormalised convenience fields. If they conflict with the resolved Type, the `typeId`/`typeVersion` identity takes precedence and the Record is considered invalid until corrected.

**On instance revision:**
- **In-place edits** (`updatedAt` advances, `fieldValues` mutate): for minor corrections that do not alter semantic meaning.
- **Semantic updates**: produce a new Record linked to the prior by a `supersedes` or `refines` Relation. The prior Record remains valid.
- **Immutable records + Relation graph**: all Records append-only; a new Record for every change. A valid implementation strategy that naturally preserves history.

---

### 4.5 Relation

A first-class typed link between instances. Relations allow implementations to construct semantic graphs for navigation, analysis, projection, and reasoning.

```typescript
{
  relationId: UUID

  relationType: string
  // Free-form. See ext:recommended-relations for canonical types and conventions.

  // source [relationType] target
  sourceInstanceId: UUID    // the asserting instance
  targetInstanceId: UUID    // the related instance

  assertedBy?: "human" | "ai" | "imported"
  confidence?: number       // 0.0–1.0; meaningful for ai-asserted
  createdAt?: ISO8601
  createdBy?: string

  status?: "proposed" | "active" | "rejected" | "superseded"
  validFrom?: ISO8601
  validUntil?: ISO8601

  notes?: string
  sourceRefs?: SourceReference[]
  meta?: Record<string, unknown>
}
```

**Directionality convention:**
`sourceInstanceId` is the asserting instance; `targetInstanceId` is the related instance. The Relation reads: "source [relationType] target."

| Relation | source | target |
|---|---|---|
| `supersedes` | the newer Record | the older Record |
| `contains` | the stage | the task inside it |
| `depends-on` | the dependent task | the task it needs |
| `refines` | the detailed version | the rough version |
| `derived-from` | the successor | the source Note or Record |
| `evidences` | the source material | the claim it supports |

This convention must be consistent across implementations. See Invariant 16.

Relations span tiers. A Note may be the target of `derived-from` Relations from the Records it graduated into.

**Canonical relation types** (use these exact strings for cross-system interoperability):

`contains`, `depends-on`, `supersedes`, `refines`, `derived-from`, `evidences`, `precedes`

Custom types not covered by these should use `namespace/name` format (e.g. `com.acme.hr/transferred-to`) to prevent collision. Extended relation type metadata is defined in `ext:recommended-relations`.

**Relations do not change lifecycle state.** A `supersedes` Relation does not mutate the prior Record's `lifecycleState`. Lifecycle state changes are explicit acts by an implementation's transition mechanism.

---

### 4.6 Container

A lightweight grouping boundary over a collection of instances. Containers answer scoping questions — which instances belong together, what constitutes "this project" — that the Relation graph alone cannot answer.

Containers are not semantic objects with Fields. They do not own semantic state; Records do. A `contains` Relation asserts "A is part of B" (a semantic claim); a Container asserts "these instances form a unit for boundary purposes" (a scope claim). Both are needed; neither replaces the other.

```typescript
{
  containerId: UUID

  namespace?: string
  name?: string

  title: string              // human-readable label

  containerType?: string     // free-form hint; e.g. "project", "meeting", "sprint"

  rootInstanceIds?: UUID[]
  // Top-level instances this Container was created to hold. Implementations may
  // derive nested members by traversing contains Relations from these roots.

  memberInstanceIds?: UUID[]
  // Explicit membership list for all instances in scope.
  // When present, allows membership queries without graph traversal.
  // When omitted, membership is defined by traversing contains Relations.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

`Container.containerId` is not an instance ID and must not appear in `Relation.sourceInstanceId` or `targetInstanceId`. See Invariant 19.

---

## 5. Distribution Group (Core)

The Distribution group is required for all conforming implementations.

### 5.1 Package

The distributable artefact. Contains Field, Type, View, and Relation type definitions with a complete dependency manifest.

```typescript
{
  schemaVersion: string      // SCDS spec version, e.g. "2.0"
  packageId: UUID
  packageName: string
  packageVersion: string     // semver, e.g. "1.2.0"
  publishedAt: ISO8601
  publisher?: string
  description?: string
  homepage?: string

  // Content (at least one of fields or types must be non-empty)
  fields: Field[]
  types: Type[]
  views?: View[]             // ext:views-l1; omit if not in use
  schemas?: Schema[]         // ext:schema; omit if not in use
  protocols?: Protocol[]     // ext:protocol; omit if not in use
  relationTypes?: RelationTypeDefinition[]  // ext:recommended-relations

  mode: "bundled" | "standalone"

  dependencyRefs: Reference[]
}
```

**`mode` semantics:**

| Mode | Meaning |
|---|---|
| `"bundled"` | All Field records referenced by any Type, and all Type records referenced by any View, are included in `fields[]` and `types[]`. Self-contained. |
| `"standalone"` | Dependencies are expected pre-installed in the consumer's registry. `dependencyRefs` is the required manifest. |

`dependencyRefs` is required in both modes. Consumers use it to validate completeness without parsing content internals.

---

### 5.2 Reference

A stable pointer to a specific definition version.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1
  definitionType?: "field" | "type" | "view" | "schema" | "protocol"
}
```

Canonical string form: `namespace/name@version`

---

### 5.3 Lineage

Upstream and fork tracking for a specific definition version.

```typescript
{
  sourceDefinitionId?: UUID     // UUID of the upstream definition
  sourceVersion?: integer       // upstream version at derivation time
  forkedFromDefinitionId?: UUID // UUID of the definition deliberately forked from
  forkedFromVersion?: integer   // version at the fork point
}
```

| Field pair | Meaning |
|---|---|
| `sourceDefinition*` | Tracked copy; consumer expects upstream updates |
| `forkedFrom*` | Deliberately diverged; no upstream tracking |

Both may be present during a transition from tracking to forking.

---

### 5.4 Provenance

Publisher and package origin metadata.

```typescript
{
  publisher?: string        // namespace or org of the original author
  sourcePackage?: string    // package name that bundled this definition
  packageVersion?: string   // semver of the source package
  importedAt?: ISO8601
}
```

`packageVersion` is distinct from `Field.version`. A package at `1.3.0` may contain `decision_statement@3` and `context@2`.

---

## 6. Conversation Layer

The conversation layer is a permanent architectural boundary distinct from SCDS. It captures raw multimodal source material; SCDS captures negotiated semantic state. They reference each other bidirectionally via `SourceReference` (document → conversation) and `AttentionState` tags (conversation → document, via `ext:addressability`).

```
Conversation layer  →  raw multimodal source material (speech, threads, annotations)
                        elements tagged with Address at production time
Protocol layer      →  structures the facilitation process; advances AttentionState
SCDS layer          →  captures negotiated semantic state; Records carry SourceReferences
Presentation layer  →  renders SCDS state via Views
```

Three conversation types are in scope:

| Type | Structure | Anchoring |
|---|---|---|
| Meeting transcript | Linear, time-ordered chunks | Tagged with AttentionState at production time |
| Threaded conversation | Tree of replies | Thread root anchored to a document element Address |
| Web UI annotations | Attached to content | Anchored to a Field or Record Address |

Transcript chunks referenced in `SourceReference` are source material — addressable evidence. They do not become Notes or Records automatically. A transcript chunk referenced in `sourceRefs` is evidence supporting a field value; it is not itself a Note unless someone deliberately models it as one.

---

## 7. Extensions

Extensions are optional, independently adoptable. Each extension section declares its identifier, dependencies, and the types it defines.

---

### ext:addressability

**Required for**: any implementation with live facilitation or multi-session extraction.

Defines a universal addressing scheme and the mechanisms that connect conversation material to document elements.

#### `Address`

A stable, resolvable identifier for any element across document space, process space, and conversation space.

```typescript
type Address =
  | {
      space: "document"
      containerId: UUID
      recordId?: UUID
      fieldId?: UUID
      revisionId?: UUID    // requires ext:addressability Revision
    }
  | {
      space: "process"
      runId: UUID          // Protocol run ID; requires ext:protocol
      stageId?: string
    }
  | {
      space: "conversation"
      sessionId: UUID
      chunkId?: UUID
      annotationId?: UUID
    }
```

Every element that can be referred to has an Address. A transcript chunk and a field Revision are co-addressable because assertions about one referencing the other require both to be resolvable.

#### `AttentionState`

The current focus of an active Protocol run — a live cursor across the address space. `AttentionState` and `Address` are structurally related but serve distinct roles: an `Address` is a stable, resolvable identifier for a specific element; `AttentionState` is the mutable cursor that records *where focus currently is* during an active session. An `AttentionState` value at a point in time resolves to a document-space `Address`, but it is stored separately because it changes continuously as the Protocol advances.

Conversation material is tagged with the active `AttentionState` as it is produced. This makes context assembly efficient: "all chunks produced while focus was on this Field" is a queryable address predicate.

```typescript
{
  containerId: UUID
  recordId?: UUID
  fieldId?: UUID
  protocolRunId?: UUID
  stageId?: string
}
```

`AttentionState` is set live by the session or Protocol runner. `SourceReference` is set retrospectively at extraction or editorial review time. Both are needed; they answer different questions.

#### `Revision`

A first-class, addressable snapshot of a `FieldValue` at a point in time. Carries the value, the agent, a timestamp, and source references to the conversation that produced the change.

```typescript
{
  revisionId: UUID
  fieldId: UUID
  recordId: UUID

  value: FieldValue
  agent: "human" | "ai" | "imported"
  createdAt: ISO8601

  sourceRefs?: SourceReference[]
  priorRevisionId?: UUID  // chain to the previous Revision for this field
}
```

Revision does not replace the edit-in-place vs. new-Record judgment. Minor corrections remain in-place edits at the implementation layer. Revision is the addressable audit trail for interoperability — it makes field history queryable: "what did this field say before the last Protocol run?", "which conversation produced the change from revision 2 to revision 3?"

#### Context Query (behavioural requirement)

A conforming `ext:addressability` implementation must be able to assemble relevant material given an address and a purpose. This is a behavioural requirement, not a data shape.

**Required query patterns:**

| Pattern | Address | Returns |
|---|---|---|
| Field context | `{recordId}/{fieldId}` | Current value, Revision history, chunks tagged to this Field, Field `aiGuidance` |
| Record context | `{recordId}` | All field values, chunks tagged to this Record, Relations, Protocol run history |
| Stage context | `{runId}/{stageId}` | All chunks produced during this stage, Fields active in this stage |
| Revision trace | `{fieldId}/{revisionId}` | Value at that Revision, the conversation that produced it, prior Revision chain |

**Recommended assembly order for AI assistance:**

1. Type and Field `aiGuidance` — what this field captures, how to extract it
2. Current value and recent Revision history — what has already been established
3. Chunks tagged to this Field via AttentionState — most focused context
4. Chunks tagged to the parent Record — broader session context
5. Related Records via Relations — structural context

---

### ext:lifecycle

**Required for**: governance tools, decision logs, any implementation where records progress through defined states.

Adds lifecycle state declarations to `Type` and lifecycle state tracking to `Record`.

#### `LifecycleState`

```typescript
{
  name: string
  description?: string
  isInitial?: boolean   // valid starting state for new Records
  isFinal?: boolean     // no transitions out; Record is settled
}
```

#### `LifecycleTransition`

```typescript
{
  name: string       // e.g. "promote", "approve", "supersede"
  from: string       // must match a state name in the enclosing lifecycle
  to: string
  description?: string
}
```

#### Type lifecycle block (added by this extension)

When `ext:lifecycle` is in use, `Type` gains:

```typescript
lifecycle?: {
  states: LifecycleState[]           // min 1 state
  transitions: LifecycleTransition[]
  initialState: string               // must reference a state name where isInitial === true
}
```

#### Record lifecycle state (added by this extension)

`Record.lifecycleState` becomes meaningful: must match a state name in the associated `Type.lifecycle.states[]` when the Type declares a lifecycle.

The `lifecycle` block declares vocabulary. Implementations decide enforcement strictness. A state with `isFinal: true` signals that no further transitions are expected; implementations may use this to lock Record content.

---

### ext:protocol

**Required for**: facilitation tools, structured deliberation, any implementation that guides users through epistemic stages.

Replaces `TemplateFacilitationStep` from v1. Protocol is epistemically richer: stages have explicit dependencies, completion criteria, and may produce intermediate Records.

#### `TypeRef`

A reference to a specific Type, used within Protocol and Schema.

```typescript
{
  typeId: UUID
  typeVersion?: integer
}
```

#### `FieldRef`

A reference to a Field within a Type.

```typescript
{
  fieldId: UUID
  typeId?: UUID    // which Type this Field appears in
}
```

#### `ProtocolStage`

A named stage in a Protocol. Stages have epistemic dependencies (`dependsOn`) — not just ordering. A stage may only proceed when its dependencies are sufficient.

```typescript
{
  stageId: string       // stable key within this Protocol
  order: integer        // min: 0; display/presentation order only — see note below
  purpose: string       // what understanding this stage builds
  question: string      // the core question this stage answers
  dependsOn: string[]   // stageId values; epistemic dependencies, not just ordering
  completionCriteria: string   // how to know this stage is sufficient to proceed
  contributesTo: FieldRef[]    // which Record Fields this stage feeds
  outputType?: TypeRef         // if this stage produces its own intermediate Record
  aiGuidance: AiGuidance
}
```

**`order` vs `dependsOn`:** `order` is the display and presentation sequence — how stages are shown in a UI or facilitation guide. Execution sequence is determined by `dependsOn` resolution: a stage runs when all its declared dependencies are satisfied, regardless of its `order` value. Authors must ensure `order` is consistent with the partial order implied by `dependsOn` (i.e. a stage's `order` value should be greater than the `order` of any stage it depends on). See Invariant 31.

#### `Protocol`

An epistemically ordered process for building quality Records through structured conversation or facilitation.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string

  targetType?: TypeRef
  // The Record type this Protocol produces. Absent for loose / exploratory Protocols
  // (Brain Dump, Decomposition) whose output is input context for a tighter Protocol.

  stages: ProtocolStage[]

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

**The Protocol spectrum:**

```
Loose                                                    Tight
─────────────────────────────────────────────────────────────
Brain Dump → Decomposition → Options Analysis → Decision
```

Loose Protocols produce open material. Tight Protocols converge on a specific Record type. The output of a loose Protocol is the input context for something tighter.

**Generic Protocols** (reusable across domains):
- Brain Dump — externalise all thinking without constraint
- Decomposition — identify major components from raw material
- Review — what is established, what is still open
- Prioritisation — which components to resolve first

**Domain-specific Protocols** (target a specific Record type):
- Decision — context → criteria → options → evaluation → decision
- Proposal — problem → solution shape → constraints → proposal

**Protocol chaining and provenance**: The output of one Protocol is the input context for the next. This derivation chain is traceable through `derived-from` Relations, making the quality and history of the final Record auditable.

**Non-normative example — Protocol chain for a governance decision:**

```
Brain Dump Protocol (loose, no targetType)
  → AttentionState: { containerId: C1 }
  → Produces: Note N1 (unstructured brainstorm)

Decomposition Protocol (loose, targetType: Component)
  → AttentionState: { containerId: C1, recordId: N1 }
  → Produces: Notes N2, N3, N4  [derived-from N1]

Decision Protocol (tight, targetType: Decision)
  → AttentionState: { containerId: C1, protocolRunId: R1, stageId: "criteria" }
  → Stage "criteria" produces: Options Analysis Record R-OA  [derived-from N2, N3]
  → Stage "decision" produces: Decision Record R-D           [derived-from R-OA]

Conversation chunks produced during Decision stage:
  chunk-42: { AttentionState: { containerId: C1, recordId: R-OA, fieldId: F-criteria, ... } }
  chunk-43: { AttentionState: { containerId: C1, recordId: R-D, fieldId: F-outcome, ... } }

Context query for R-D / F-outcome:
  → Field aiGuidance from Decision Type + outcome Field
  → Current value + Revision history for F-outcome
  → Chunks tagged with { recordId: R-D, fieldId: F-outcome } — chunk-43
  → Chunks tagged with { recordId: R-D } — broader session context
  → Related Records via Relations — R-OA via derived-from
```

The final Decision Record is auditable because every Protocol stage left addressable artefacts. The quality of the outcome is traceable to the conversation that produced it.

Views (`ext:views-l1`) no longer contain facilitation logic. A View is a presentation concern; a Protocol is an epistemic one.

---

### ext:schema

**Required for**: extraction pipelines, founding document workflows, any system that needs to specify what a document type IS before assembling it.

#### `RelationSpec`

Declares an expected Relation between two Record types within a Schema.

```typescript
{
  relationType: string
  sourceType: TypeRef
  targetType: TypeRef
  cardinality?: "one-to-one" | "one-to-many" | "many-to-many"
  required?: boolean
}
```

#### `Schema`

The definition of a complete document type — which Types it contains, what Relations exist between resulting Records, and what "complete" means. A Schema is the artefact handed to an extraction pipeline.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string

  rootTypes: TypeRef[]        // Types to extract
  structure: RelationSpec[]   // expected Relations between extracted Records
  requiredTypes: TypeRef[]    // what "complete" means for this document type

  aiGuidance?: AiGuidance
  // purpose: what kind of document this Schema defines
  // extraction: framing for extraction pipelines

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

**Schema vs View:**

| | Schema | View / Document View |
|---|---|---|
| Question it answers | What IS this document type? What should be extracted? | How are existing Records assembled into readable output? |
| Operates at | Definition time | Projection time |
| Input | Source material (transcripts, conversations) | Existing Records in a Container |
| Output | Extraction instructions → Records | Rendered document |

---

### ext:views-l1

**Required for**: rendering and export workflows.

Defines Views — versioned presentations of a single Record through a specific Type.

#### `FieldView`

A field reference within a View. Controls presentation for this View without altering field semantics.

```typescript
{
  fieldId: UUID       // must appear in the Type's fields[]
  order: integer      // min: 0; display order within this View
  required?: boolean  // View-level workflow constraint; does not alter Field contract
  visible?: boolean   // default: true

  // Presentation overrides — View scope only
  displayLabel?: string
  displayHint?: string
  editorHintOverride?: string
}
```

A Field hidden with `visible: false` remains in the Record and may appear in other Views.

#### `ExportConfig`

Configuration for rendering a Record through this View as an exportable document.

```typescript
{
  format?: string        // target format hint, e.g. "markdown", "adoc", "json"
  preamble?: string
  // Template string rendered before field values.
  // Variable substitution uses {{variable-name}} syntax.
  // Standard variables: {{instance-id}}, {{date}}, {{status}}, {{namespace}}, {{name}}

  fieldOrder?: UUID[]    // explicit export field ordering; defaults to fieldViews[].order
  omitEmptyFields?: boolean  // default: false
}
```

#### `View`

A versioned presentation and export configuration over a specific Type. Multiple Views may exist for the same Type, serving different audiences or purposes.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string    // when to use this View; what workflow or audience it serves

  typeId: UUID           // references Type.id
  typeVersion: integer

  aiGuidance?: AiGuidance
  // purpose: the workflow context this View serves
  // extraction: session-level framing injected before field extraction

  fieldViews: FieldView[]

  exportConfig?: ExportConfig

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

A View may not introduce Fields that are not in the bound Type. Omitted Fields are treated as `visible: false`.

Facilitation steps have been removed from View. Use `ext:protocol` Protocol stages instead.

---

### ext:views-l2

**Depends on**: `ext:views-l1`

**Required for**: document projection — assembling multiple Records into a coherent document.

#### `SectionSource`

Defines how a section's instances are selected from a Container.

```typescript
type SectionSource =
  | {
      type: "fixed-instances"
      instanceIds: UUID[]
      // Explicit list. For preamble, cover page, or curated sections.
    }
  | {
      type: "type-query"
      semanticObjectType: string
      // For cross-system portability, use namespace/name format (e.g. "core/decision").
      // A bare string like "decision" is a single-system convention.
      lifecycleState?: string
      containerIds?: UUID[]
    }
  | {
      type: "relation-query"
      fromInstanceId: UUID
      relationType: string
      direction?: "forward" | "inverse"  // default: "forward"
    }
  | {
      type: "container-subset"
      containerId: UUID
      containerType?: string
    }
```

#### `DocumentSection`

One section in a Document View.

```typescript
{
  sectionId: string
  title?: string
  description?: string
  order: integer   // min: 0

  source: SectionSource

  renderViewId?: UUID    // View (ext:views-l1) used to render each instance in this section
  // When absent, implementations use a default rendering for the instance type.

  ordering?: {
    fieldId?: UUID
    direction?: "asc" | "desc"  // default: "asc"
  }

  required?: boolean
  emptyBehavior?: "hide" | "show-placeholder"
}
```

#### `DocumentView`

A versioned, Container-level projection. Defines how a Container's Records are assembled into a readable document.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string    // what kind of document this produces; intended audience

  containerType?: string  // when set, intended for Containers of this type

  sections: DocumentSection[]

  preamble?: string
  // Template string rendered before all sections.
  // Standard variables: {{container-title}}, {{date}}, {{container-id}}

  format?: string   // e.g. "markdown", "adoc", "html"

  aiGuidance?: AiGuidance
  // purpose: what kind of document this View produces
  // extraction: context for AI-assisted document-level tasks

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

A `DocumentView` may reference multiple `View` records (via `DocumentSection.renderViewId`) — one per instance type in the document. It orchestrates; it does not replace them.

`DocumentSection.renderViewId` references a `View.id` (from `ext:views-l1`). A `DocumentView.id` is not a valid value for `renderViewId` — Document Views are not nestable.

---

### ext:repeatable-fields

**Required for**: any Record type that needs lists of values within a single Field.

Adds repeatability to `FieldAssignment` and defines `FieldValueEntry`.

#### `FieldValueEntry`

A single entry in a repeatable field.

```typescript
{
  value: string | number | boolean | string[] | null
  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
}
```

#### FieldAssignment additions

When `ext:repeatable-fields` is in use, `FieldAssignment` gains:

```typescript
repeatable?: boolean  // default: false; when true, multiple values are allowed
minItems?: integer    // meaningful only when repeatable === true
maxItems?: integer    // meaningful only when repeatable === true
```

And `FieldValue.entries` becomes active: use `entries` when `repeatable === true`, `value` otherwise.

A repeatable field entry does not create a new semantic instance. Use separate Records connected by Relations when repeated items need their own identity, lifecycle, or graph position.

---

### ext:field-groups

**Required for**: Record types where multiple Fields are semantically paired and repeat together as a unit.

Use when parallel `multiselect` arrays would lose pairing (e.g. a contact record with `name` + `email`). Preserves internal pairing across repeated items.

#### `FieldGroup`

A named, ordered group of Fields that repeat together as a unit within a Type.

```typescript
{
  groupId: string        // stable key within the Type
  label?: string
  description?: string

  order: integer         // min: 0; position relative to other Fields and Groups

  required?: boolean     // default: false
  repeatable?: boolean   // default: false
  minItems?: integer
  maxItems?: integer

  fields: FieldAssignment[]
}
```

#### `FieldGroupEntry`

One entry in a repeatable Field Group.

```typescript
{
  entryId?: UUID         // stable key for this entry; allows referencing or updating
  fieldValues: FieldValue[]
}
```

#### `FieldGroupValue`

The current value of a Field Group within a Record.

```typescript
{
  groupId: string           // references FieldGroup.groupId in the Type definition
  entries: FieldGroupEntry[]
}
```

A `FieldGroup` does not create a new semantic instance. Its entries are embedded structured context within the enclosing Record. Use separate Records connected by Relations when group entries need their own identity, lifecycle, provenance, or reuse across Records.

When `ext:field-groups` is in use, `Type` gains `fieldGroups?: FieldGroup[]` and `Record` gains `groupValues?: FieldGroupValue[]`.

**Repeatability pattern guide:**

| Pattern | Use | Example |
|---|---|---|
| Repeatable scalar | `FieldAssignment.repeatable` (ext:repeatable-fields) | Multiple assigned person names |
| Repeatable structured context | `FieldGroup` | Contacts with name + email pairs |
| Repeated semantic objects | Separate Records + Relations | Tasks assigned to roles |

---

### ext:cross-field-validation

**Required for**: Types with constraints that span multiple Fields.

`ValidationRule` handles single-field constraints. `CrossFieldRule` handles constraints that require evaluating more than one Field together.

#### `CrossFieldRule`

```typescript
{
  type: "conditional-required" | "field-ordering" | "mutual-exclusion"
  message?: string

  // conditional-required: targetFieldId becomes required when predicateFieldId equals predicateValue
  predicateFieldId?: UUID
  predicateValue?: string
  targetFieldId?: UUID

  // field-ordering: targetFieldId must precede or follow predicateFieldId
  // Applies only to fields with valueType "date" or "number".
  effect?: "must-precede" | "must-follow"

  // mutual-exclusion: at most one of the listed fields may have a non-empty value
  fieldIds?: UUID[]   // min: 2
}
```

| Rule type | Required fields |
|---|---|
| `conditional-required` | `predicateFieldId`, `predicateValue`, `targetFieldId` |
| `field-ordering` | `predicateFieldId`, `targetFieldId`, `effect` |
| `mutual-exclusion` | `fieldIds` (min 2) |

When `ext:cross-field-validation` is in use, `Type` gains `validationRules?: CrossFieldRule[]`.

---

### ext:recommended-relations

**Required for**: cross-system federation; multi-publisher ecosystems where Relation type semantics must be interoperable.

Canonical relation types and machine-readable Relation type definitions.

**Canonical relation types** (use exact strings):

| Canonical | Converse | Category |
|---|---|---|
| `contains` | `part-of` | Composition |
| `depends-on` | `required-by` | Dependency |
| `supersedes` | `superseded-by` | Governance |
| `refines` | `refined-by` | Refinement |
| `derived-from` | `source-of` | Derivation |
| `evidences` | `evidenced-by` | Evidence |
| `precedes` | `follows` | Sequence |

Implementations must store only the canonical (forward) form and derive the inverse when needed.

**Relation category taxonomy:**

| Category | Examples |
|---|---|
| Composition | `contains`, `part-of`, `has-section` |
| Refinement | `refines`, `expands`, `summarises` |
| Dependency | `depends-on`, `requires`, `blocks`, `enables` |
| Sequence | `precedes`, `follows`, `overlaps` |
| Derivation | `derived-from`, `extracted-from`, `based-on` |
| Evidence | `evidences`, `supports`, `contradicts` |
| Governance | `supersedes`, `amends`, `ratifies`, `delegates` |
| Association | `relates-to`, `links-to` |

#### `RelationTypeDefinition`

Machine-readable metadata for a `relationType` string.

```typescript
{
  relationType: string      // exact string used in Relation.relationType
  namespace: string
  label?: string
  description?: string
  category?: "composition" | "refinement" | "dependency" | "sequence" | "derivation" | "evidence" | "governance" | "association"
  canonicalDirection?: string   // e.g. "source is the dependent task; target is the task it depends on"
  inverseType?: string
}
```

`RelationTypeDefinition` is optional metadata. Implementations are not required to resolve `relationType` strings against a definition before accepting a Relation. Relation type definitions may be included in a Package or published separately.

---

### ext:import-tracking

**Required for**: implementations that receive packages from upstream publishers and need to track update and conflict state.

#### `ImportMode`

```typescript
"upstream-tracked" | "local-copy" | "local-fork"
```

| Mode | Meaning |
|---|---|
| `"upstream-tracked"` | Consumer expects updates from the source Package. Conflicts surfaced when local and upstream diverge. |
| `"local-copy"` | Imported as a snapshot. No update tracking. |
| `"local-fork"` | Deliberately diverged. Upstream lineage preserved for reference. |

#### `ImportRecord`

One record per imported definition in a consumer's local registry.

```typescript
{
  definitionId: UUID
  definitionType: "field" | "type" | "view" | "schema" | "protocol"
  namespace: string
  name: string
  version: integer

  mode: ImportMode
  importedAt: ISO8601

  sourcePackageId: UUID
  sourcePackageName: string
  sourcePackageVersion: string

  latestKnownUpstreamVersion?: integer
  updateAvailable?: boolean
  updateCheckedAt?: ISO8601

  conflictState?: "clean" | "local-ahead" | "upstream-ahead" | "diverged"
  conflictDetectedAt?: ISO8601

  localVersion?: integer
  localEditedAt?: ISO8601
}
```

#### `ImportSummary`

A consumer's complete picture of its imported definitions.

```typescript
{
  generatedAt: ISO8601
  fields: ImportRecord[]
  types: ImportRecord[]
  views: ImportRecord[]
  schemas: ImportRecord[]
  protocols: ImportRecord[]
}
```

---

### ext:registry

**Required for**: multi-publisher ecosystems; discoverable definition catalogs.

#### `RegistryEntry`

One entry in a Registry catalog.

```typescript
{
  packageId: UUID
  packageName: string
  packageVersion: string
  publisher: string
  description?: string
  publishedAt: ISO8601
  homepage?: string
  tags?: string[]
  fieldCount: integer       // min: 0
  typeCount: integer        // min: 0
  viewCount?: integer
  schemaCount?: integer
  protocolCount?: integer
  relationTypeCount?: integer
  downloadUrl?: string
  checksum?: string         // SHA-256 hex digest for integrity verification
}
```

#### `Registry`

A registry's published index.

```typescript
{
  schemaVersion: string
  registryId: UUID
  registryName: string
  catalogVersion: string    // registry's own version (semver)
  updatedAt: ISO8601
  homepage?: string
  entries: RegistryEntry[]
}
```

Multiple Registries may coexist. A consumer may index multiple catalogs. The specification does not define registry authority, authentication, or federation.

---

## 8. Key Invariants

Conforming implementations must uphold the following invariants.

### Field semantics

**1.** `FieldAssignment.displayLabel` and `FieldAssignment.displayHint` are for rendering only. They must not affect AI guidance, extraction logic, `valueType` interpretation, or validation.

**2.** A `Type` must not redefine, override, or duplicate the semantic content of any `Field` it includes. If different semantics are needed for a Field in a specific Type context, a distinct `Field` with its own identity and lineage must be created.

**3.** A `Field`'s `aiGuidance` belongs to the Field. Type-level `aiGuidance` provides session framing only.

### Lifecycle (ext:lifecycle)

**4.** `Type.lifecycle.initialState` must reference a `name` that appears in `lifecycle.states[]` and where `isInitial === true`.

**5.** Every `from` and `to` value in `lifecycle.transitions[]` must reference a `name` that appears in `lifecycle.states[]`.

**6.** `Record.lifecycleState`, when present, must reference a `name` in the associated `Type.lifecycle.states[]`.

### Distribution

**7.** Every `fieldId` referenced in any `FieldAssignment` within a `Package.types[]` must appear as the `id` of an entry in `Package.dependencyRefs`.

**8.** If `Package.mode === "bundled"`: every `Reference` in `dependencyRefs` must have a matching `Field` in `fields[]` (matched on `id` and `version`).

**9.** `Field.id` is stable across versions. A new `id` means a new definition, not a new version of an existing one.

### Cross-field validation (ext:cross-field-validation)

**10.** All `fieldId` values in any `CrossFieldRule` within `Type.validationRules[]` must appear in `Type.fields[].fieldId`. Cross-field rules cannot reference Fields outside the Type.

**11.** A `conditional-required` rule must supply `predicateFieldId`, `predicateValue`, and `targetFieldId`. A `field-ordering` rule must supply `predicateFieldId`, `targetFieldId`, and `effect`. A `mutual-exclusion` rule must supply `fieldIds` with at least two entries.

### Views (ext:views-l1)

**12.** Every `fieldId` in `View.fieldViews[]` must appear in the bound Type's `fields[].fieldId`. A View cannot introduce Fields not in its Type.

**13.** `FieldView.displayLabel`, `FieldView.displayHint`, and `FieldView.editorHintOverride` are for rendering only. They must not affect AI guidance, extraction logic, `valueType` interpretation, or validation.

**14.** A `View` must not override, redefine, or duplicate the semantic content of any `Field` or `Type` it references. View-level `aiGuidance` is workflow framing; it does not redefine Field extraction semantics.

### Distribution — Views (ext:views-l1)

**15.** Every `typeId` referenced by any `View` in `Package.views[]` must appear in `Package.dependencyRefs` with `definitionType: "type"`. If `mode === "bundled"`, that `Type` must be present in `types[]`.

### Relations

**16.** In a `Relation`, `sourceInstanceId` is the asserting instance and `targetInstanceId` is the related instance. The Relation reads: "source [relationType] target." This convention must not be reversed.

**17.** `Relation` is reserved for assertions that carry semantic consequence beyond simple mention or citation. Lightweight prose references that do not assert structural, causal, or governance relationships must not be modelled as `Relation` records.

### Notes and Typed Records

**18.** `NoteSection.name` values must be unique within a `Note`.

**19.** `TypedField.name` values must be unique within a `Typed Record`.

### Containers

**20.** `Container.containerId` is not an instance ID. It must not appear in `Container.rootInstanceIds`, `Container.memberInstanceIds`, `Relation.sourceInstanceId`, or `Relation.targetInstanceId`.

**21.** `Container.rootInstanceIds` and `Container.memberInstanceIds`, when present, must reference valid SCDS instance IDs (`Note.instanceId`, `Typed Record.instanceId`, or `Record.instanceId`).

### Repeatability (ext:repeatable-fields)

**22.** If `FieldAssignment.repeatable` is false or absent, its corresponding `FieldValue` must use `value` and must not include `entries`.

**23.** If `FieldAssignment.repeatable` is true, its corresponding `FieldValue` may use `entries`. If `minItems` is specified, `entries` must contain at least that many items. If `maxItems` is specified, `entries` must not exceed that count.

**24.** `FieldAssignment.minItems` and `maxItems` are valid only when `repeatable === true`. They must be ignored when `repeatable` is false or absent.

### Field groups (ext:field-groups)

**25.** Every `groupId` in `Record.groupValues[]` must reference a `groupId` declared in the associated `Type.fieldGroups[]`.

**26.** Within a `FieldGroupEntry.fieldValues[]`, every `fieldId` must appear in the enclosing `FieldGroup.fields[].fieldId`.

**27.** A `FieldGroupValue.entries` list must satisfy `FieldGroup.minItems` and `maxItems` where specified.

### Records

**28.** `Record.typeId` and `Record.typeVersion` are the authoritative Type binding. `typeNamespace` and `typeName` are denormalised convenience fields. If they conflict with the resolved `Type`, the `typeId`/`typeVersion` identity takes precedence and the Record is considered invalid until corrected.

### Protocol (ext:protocol)

**29.** Every `stageId` in `ProtocolStage.dependsOn[]` must reference a `stageId` declared in the enclosing `Protocol.stages[]`. A stage may not declare a dependency on itself.

**30.** Every `fieldId` in `ProtocolStage.contributesTo[]` must reference a `fieldId` that appears in `Protocol.targetType`'s Field list. When `targetType` is absent (loose Protocol), `contributesTo` must be empty.

**31.** For every pair of stages A and B within a `Protocol` where B.dependsOn includes A.stageId, B.order must be greater than A.order. `order` is display order; execution sequence is determined by `dependsOn` resolution. The two must not contradict each other.

### Views L2 (ext:views-l2)

**32.** When a `DocumentView` is included in a `Package` (i.e. published for distribution), any `SectionSource` with `type === "type-query"` must use `namespace/name` format for `semanticObjectType` (e.g. `"core/decision"`, not `"decision"`). Bare strings are acceptable only in single-system `DocumentView` records not intended for distribution. Implementations receiving a distributed `DocumentView` with a bare `semanticObjectType` in a `type-query` section should treat the portability of that section as undefined.

### Addressability (ext:addressability)

**33.** `Revision.priorRevisionId`, when present, must reference a `Revision.revisionId` for the same `fieldId` and `recordId`. Revision chains must be acyclic.

**34.** `AttentionState.containerId` must reference a valid `Container.containerId`. Other Address components (`recordId`, `fieldId`, `protocolRunId`, `stageId`) are optional and may be absent when focus has not yet narrowed.

---

## 9. Conformance

An implementation declares conformance using the following form:

```
SCDS <version> Core [+ ext:<name> ...]
```

Example:
```
SCDS 2.0 Core + ext:lifecycle + ext:protocol + ext:views-l1 + ext:addressability + ext:recommended-relations
```

### Core conformance requirements

A core-conformant implementation must:
- Accept and validate `Field`, `Type`, `Record` (Tier 2), `Relation`, and `Container` inputs against this specification
- Enforce Invariants 1–3, 7–9, 16–21, 28
- Support the Foundation and Distribution groups in full
- Implement the namespace format and reference format correctly
- Not accept `relationType` strings that include `/` except in `namespace/name` format

Support for `Note` (Tier 0) and `Typed Record` (Tier 1) is optional at core conformance level.

### Extension conformance requirements

An implementation declaring a given extension must:
- Accept and validate all types defined by that extension
- Enforce all invariants assigned to that extension
- Respect the declared dependency chain (e.g., `ext:views-l2` requires `ext:views-l1` to also be declared)

### Interoperability note

Two implementations at the same conformance level will produce compatible definitions for exchange. An implementation receiving a Package that includes types from an extension it does not support should surface the unknown content rather than silently discard it.

this is the new one

---

# claude response

Confirmed — same document I already reviewed, all four fixes are in. The spec is in good shape.

One gap I missed in the previous review, now that I'm reading more carefully:

**`Package` is missing `documentViews`**

The Package type has:
```typescript
views?: View[]             // ext:views-l1
schemas?: Schema[]         // ext:schema
protocols?: Protocol[]     // ext:protocol
```

But no `documentViews?: DocumentView[]` for `ext:views-l2`. This means DocumentViews can't be distributed via Package — which quietly breaks the distribution story for that extension.

This makes Invariant 32 self-contradictory: it says "When a `DocumentView` is included in a `Package`..." but there's currently no field in Package where a DocumentView can appear.

The fix is straightforward — add to Package:

```typescript
documentViews?: DocumentView[]   // ext:views-l2; omit if not in use
```

And add a corresponding distribution invariant (Invariant 35 or between 15 and 16):

> When a `DocumentView` is included in a `Package`, any `DocumentSection.renderViewId` must reference a `View.id` that appears in `Package.views[]` or `Package.dependencyRefs`. If `mode === "bundled"`, that `View` must be present in `Package.views[]`.

That's the only gap I can find. Everything else holds together.

---

# you asked

message time: 2026-05-24 13:55:05

Now, RAG pipeline. Scds is a way to maintain context and lineage. But does it aid in retrieval?

---

# claude response

This is a genuinely interesting question because the answer is: SCDS produces exactly the right metadata for better-than-naive RAG, but doesn't currently define how that metadata is used for retrieval. There's a gap worth naming.

**What SCDS already enables**

AttentionState tagging is the most valuable piece. Every chunk produced during a Protocol run carries: containerId, recordId, fieldId, stageId. At index time, attach this as chunk metadata. At query time, you can do scope-filtered retrieval rather than flat vector similarity:

```
Query: "what did people say about success criteria?"
Naive RAG: search all chunks → lots of noise
SCDS-scoped: search chunks where { recordId: R1, fieldId: F-criteria } → high signal
```

The scope hierarchy is natural: exact field match → same Record → same Container → related Containers via Relations → global. You traverse outward until you have enough context.

Field-level AI guidance is the second piece. `Field.aiGuidance.purpose` is a natural language description of what a field captures. This seeds retrieval queries precisely — you're not just embedding the user's query, you're embedding "what this field is looking for" and finding chunks that match that intent.

The Relation graph enables multi-hop retrieval. Before querying, traverse: a Decision Record's context should include chunks from the Options Analysis Record it was derived-from, which includes chunks from the Decomposition Notes those were derived-from. The graph tells you what's semantically upstream.

**The gap**

SCDS doesn't define:

1. **Chunk indexing schema** — what SCDS metadata to attach to a chunk at index time. This is the bridge. Right now AttentionState tagging is described as a tagging mechanism but the actual metadata fields that land on a chunk aren't specified.

2. **Retrieval query shape** — how to express a context-scoped query. "Give me chunks relevant to filling Field X in Record Y, scoped to Container Z, with graph traversal to depth N." This is implicit in the Context Query behavioural requirement but not formalised.

3. **Scope resolution order** — when to widen scope. Start at field level, fall back to record, then container, then related containers. This needs to be defined for consistent behaviour across implementations.

**The architectural question**

There are two ways to think about this:

SCDS as a *metadata producer* — it generates the tagging and structure that a retrieval system consumes, but retrieval is someone else's concern. Clean separation, SCDS stays as a semantic state standard.

SCDS as a *retrieval context definer* — it specifies what the chunk metadata schema looks like and what a well-formed retrieval query over SCDS-tagged material looks like. This makes retrieval interoperable across implementations.

For μDemocracy specifically, you probably want the second. If the democracy coach is assembling context for AI assistance, you want a defined behaviour: "given this AttentionState, here is what gets retrieved and in what priority order." That's testable and reproducible. Without it, two implementations of the context assembly step produce different quality AI assistance from the same Records.

**What a retrieval extension might look like**

A minimal `ext:retrieval` or just a defined chunk metadata schema in `ext:addressability`:

```
ChunkMetadata {
  chunkId: UUID
  sessionId: UUID

  // SCDS context at production time (from AttentionState)
  containerId?: UUID
  recordId?: UUID
  fieldId?: UUID
  protocolRunId?: UUID
  stageId?: string

  // Scope level — derived, for efficient filtering
  scope: "field" | "record" | "container" | "session"
}
```

And a retrieval scope specification:

```
RetrievalScope {
  primary:   Address    // exact match — highest relevance
  fallback:  Address[]  // widen in order until sufficient context
  related:   UUID[]     // instance IDs to include via Relation traversal
  maxDepth?: integer    // how many Relation hops to traverse
}
```

The Context Query behavioural requirement already implies this — it just needs to be made concrete enough that implementations produce compatible results.

**The practical consequence for you**

Right now, without this defined, your RAG pipeline for μDemocracy will work but its quality depends entirely on how you implement context assembly. If you define it once well, you get consistent AI assistance quality across all Protocol runs and all facilitated sessions. The semantic structure is doing the heavy lifting — you just need to formalise how it connects to the retrieval layer.

Worth either adding a minimal retrieval schema to `ext:addressability` (since AttentionState is already there) or flagging it as a near-future extension, because it's the piece that makes the AI assistance layer actually reliable.

---

# you asked

message time: 2026-05-24 13:59:44

Scds provides depth. But when used for decisions, the primary use case is finding relevant previous decisions. Scds makes the rich history possible... But the first search target is just the decision log

---

# claude response

Right. And that reframes the retrieval problem significantly.

**Two distinct retrieval modes**

The primary use case isn't RAG at all — it's structured search over a semantic log. You're not fishing through raw transcript chunks. You're querying a collection of Decision Records that already have typed, structured fields.

```
Mode 1 — Decision log search (primary)
  Input: what we're currently deciding
  Target: Decision Records in the log
  Output: relevant past Decisions, ranked

Mode 2 — Source material retrieval (secondary)
  Input: a past Decision you want to understand deeply
  Target: transcript chunks via SourceReference + AttentionState
  Output: the conversation that produced it
```

Mode 2 only activates when someone asks "why did we decide that?" or "what were we thinking when we chose X over Y?" That's the deep RAG use case. It's powerful but it's not the first thing anyone needs.

**SCDS is the index for Mode 1**

This is actually a cleaner problem than RAG. You're doing semantic search over structured Records. Each Decision Record has typed fields — decision_statement, rationale, context, options_considered. You know exactly what to embed because the Type definition tells you which fields carry the meaning.

The search becomes:
- Embed the `decision_statement` field (and maybe a composite of key fields)
- Filter by: lifecycleState (active decisions, not superseded ones), Container scope, date range
- Rank by semantic similarity
- Return whole Records, not chunks

And because Relations are first-class, you get graph-augmented search for free. "Find decisions related to what we're deciding" can traverse `supersedes`, `depends-on`, and `contains` chains — not just similarity. That's something no raw chunk index can do.

**The practical architecture**

```
User is about to make a decision
  ↓
Search decision log
  → semantic similarity on decision_statement
  → filter: lifecycleState = "decision" (ratified, not superseded)
  → filter: Container = this workspace (or all)
  → graph: traverse depends-on / related-to from current context
  ↓
Surface relevant past Decisions as structured Records
  ↓
If user needs deeper context on a specific past Decision:
  → follow SourceReferences into transcript chunks
  → Mode 2 activates
```

**What this means for SCDS**

The Decision Log is already expressible as a Schema (`ext:schema`) — a Container with Decision Records inside it, with `requiredTypes` ensuring completeness. The Container's `memberInstanceIds` is the log index. The Relation graph over those Records is the structural retrieval layer.

What SCDS doesn't currently specify is what gets embedded from a Record for semantic search. This is the indexing question: for a Decision Type, which field or combination of fields produces the best retrieval signal? That's probably a per-Type concern — a Decision embeds differently from a Risk or a Principle.

Worth a lightweight addition: an optional `indexHint` on Type or Field, indicating whether a field should be included in the semantic index for this type. Something like:

```typescript
// On FieldAssignment, optional
indexWeight?: "primary" | "secondary" | "exclude"
// primary: embed this field, high weight
// secondary: embed, lower weight  
// exclude: don't include in semantic index
```

That way the Decision Type definition can say: `decision_statement` is primary, `rationale` is secondary, `admin_notes` is excluded. The retrieval system doesn't have to guess.

The RAG depth is then a feature you add later, not a prerequisite. Ship Mode 1 first — it solves the real problem. Mode 2 is the thing that makes μDemocracy genuinely distinctive over time.

---

# you asked

message time: 2026-05-24 14:03:59

Mode 2 provides an interesting but vital decision making element... Context. Different decisions, even opposite decisions, can be the expression of the same values and ethics when context is different

---

# claude response

This is one of the most important insights in the whole system. Let me unpack why.

**The decision record is the tip of an iceberg**

A decision statement — "we will prioritise depth over breadth" — means almost nothing without knowing: what was the pressure that forced the choice, what alternatives were considered, what constraints existed, what the group understood about its own situation at that moment. Two groups could make opposite decisions and both be acting with integrity, because their contexts differ. Two decisions by the same group could look contradictory and actually represent the same values applied to changed circumstances.

The Protocol chain preserves this. Brain Dump → Decomposition → Options Analysis → Decision isn't just process overhead. It's the context record. The early loose Protocols carry the situational understanding; the final Record carries the conclusion. Mode 2 retrieval doesn't just fetch "why we decided that" — it fetches "what was true about the world when we decided that."

**This makes context a retrieval dimension, not just a field**

The retrieval model needs to search on context similarity, not just decision similarity. When a group faces a new choice, the most useful past decisions aren't necessarily the ones with similar outcomes — they're the ones made in similar contexts. And sometimes the most illuminating past decisions are ones made in different contexts that produced different outcomes from the same values, because that teaches you something about how your values actually behave under different conditions.

```
Context-aware retrieval:
  "We're deciding whether to slow expansion"
  
  Mode 1 finds: decisions about expansion pace
  
  But also valuable:
  - decisions made under resource constraint (similar context, different topic)
  - decisions where the group chose sustainability over growth (same values expression)
  - decisions that look opposite but were made when conditions differed
```

**Values become the stable layer**

In μDemocracy, Principles and Values are their own Record Types in the founding documents. This creates something powerful: Decision Records can relate to Value Records via Relations. `expresses → Principle`, `derived-from → Value`.

Over time the decision log becomes a values evidence base. "Show me all decisions that express the principle of adaptive response" is a graph query. "Has our behaviour been consistent with our stated values?" becomes answerable — not by assertion but by traversal.

Two apparently contradictory decisions, both linked via `expresses` to the same Principle, with different contexts explaining the divergence — that's institutional integrity made legible. The democracy coach can surface this: "these look like contradictions but they're both expressions of Principle P applied to different situations."

**What this means architecturally**

The Protocol chain is the context preservation mechanism. Intermediate Records — the Decomposition Notes, the Options Analysis, the Success Criteria — are not just process artefacts. They're the context layer that makes a Decision Record meaningful as a precedent.

Mode 2 retrieval should traverse the derivation chain, not just fetch source chunks:

```
Past Decision D
  ← derived-from Options Analysis R-OA
    ← derived-from Decomposition Notes N2, N3
      ← derived-from Brain Dump N1
        ← transcript chunks (raw context)
```

Each level of that chain answers a different question. The transcript chunks answer "what was said." The Brain Dump answers "what was the raw situation." The Decomposition answers "how did the group understand the problem structure." The Options Analysis answers "what was evaluated against what criteria." The Decision answers "what was committed to."

Context-aware retrieval can target any level of that chain depending on what you're trying to understand.

**The institutional memory this creates**

Most governance systems lose context within months. The decision log becomes a list of outcomes with no reasoning, no situational grounding, no values trace. New members can't learn from it. Old members can't interrogate their own consistency. External parties can't assess integrity.

SCDS with Protocol chains solves this structurally. The context isn't a narrative summary someone writes after the fact — it's preserved in the derivation graph as it was built, with conversation evidence attached, at the moment the group actually understood it. That's a fundamentally different quality of institutional memory.

For μDemocracy this is central to the whole proposition. The democracy coach isn't just helping groups make better individual decisions. It's helping them build a coherent, queryable record of how they think — one that teaches them about themselves over time.

---

# you asked

message time: 2026-05-24 14:08:41

https://github.com/the-greenman/muDemocracy-decisions/issues/30

This is the beginning of the plan to formalise the transcription process

---

# claude response

This is further along than I expected, and it's converging on the same concepts as SCDS from the opposite direction. Let me map the connections precisely.

**What this issue IS in SCDS terms**

The `CanonicalTranscriptSegment` schema here is the transcript standard that `SourceReference.sourceStandard` should reference — `"canonical-transcript@1.0"` is exactly what Phase 5 publishes. These two specifications need to cite each other formally when that happens.

The context broadcast mechanism (`feature/connection-context-sync`) — facilitator pushes active decision/field context to all streams — is `AttentionState` implemented. The SCDS concept didn't lead that work, it formalises what already exists. The field names even converge: `activeDecisionId`, `activeField` in the implementation; `recordId`, `fieldId` in `AttentionState`.

The chunk addressing API (`GET /api/meetings/{id}/chunks/{chunkId}`) is the conversation-space Address in `ext:addressability`. Already built.

**The merge layer is new for SCDS to acknowledge**

This is the piece SCDS doesn't model. When a Decision Record's `SourceReference` points to `chunkId: abc123`, that chunk is the canonical selection from a merge of potentially several equivalent captures (room mic + Zoom audio). The merge selection itself is provenance — it's a decision about which capture best represents what was said.

SCDS should probably acknowledge this in `SourceReference`:

```typescript
mergeSelectionId?: UUID
// When the source chunk is a merge-selected canonical, this references
// the merge selection record for full capture provenance
```

Thin enough not to impose on the transcript system, but preserves the chain.

**The contentType discrimination lands correctly**

The speech vs message distinction — duration-based vs point-in-time, stream-relative vs wall-clock — is exactly the detail SCDS noted but left to the transcript standard to define. This issue defines it rigorously. The SCDS note about "a chat message timestamp and a spoken utterance timestamp have different alignment semantics" is validated by this schema.

**The stream relationship types are ahead of SCDS**

`equivalent`, `parallel`, `derived` is a sophisticated model SCDS doesn't address. It matters for `SourceReference` interpretation: a source chunk from a derived stream (AI synthesis, translation) has different evidential weight than one from an equivalent stream canonical selection. Worth a `streamRelationship` field on `SourceReference` eventually, or at least acknowledgment that `sourceStandard` carries this information.

**Phase 6 is where the two systems become deeply intertwined**

The graph backend — `FOLLOWS`, `RESPONDS_TO` conversation edges — is SCDS Relations applied at the conversation layer. A `RESPONDS_TO` edge between two chunks is structurally the same as a `derived-from` Relation between two Records. When Phase 6 arrives, the question is whether the conversation graph and the SCDS semantic graph are unified or remain separate layers with bridges.

My instinct: keep them separate but make the bridges first-class. A chunk-level `RESPONDS_TO` is a conversation structure assertion. A Record-level `derived-from` is a semantic provenance assertion. They answer different questions. The bridge is `SourceReference` + `AttentionState` — already defined.

**The phasing matches the SCDS core/extension structure**

Phases 1–3 are building the foundation that makes `ext:addressability` possible. Phase 4 (merge workflow) enriches provenance. Phase 5 is the open standard publication that gives `SourceReference.sourceStandard` something stable to reference. Phase 6 is the graph work that starts to look like SCDS Relations at conversation depth.

The practical observation: you can ship Mode 1 retrieval (decision log search) against SCDS Records without waiting for Phase 4 or beyond. Mode 2 retrieval (source material depth) becomes richer with each phase. The two build in parallel without blocking each other.

---

# you asked

message time: 2026-05-24 14:14:02

I plan to define both of these as clear standards that can be implemented together... They need each other.

But my reference implementation will probably build them into the same platform rather than developing the implementation completely separately.

These require a systemic relationship. They need the cross-addrressable architecture.

But, is this the right path? Should I actually build them separately?

---

# claude response

The honest answer: build them in the same repository with the discipline of genuine separation. Not the same thing as building them as one system.

**Why not fully separate**

The interface between them is still being discovered. `AttentionState`, `SourceReference`, the addressing scheme — these look right on paper, but you'll only find out what they actually need to be by building both sides simultaneously and feeling the friction. If they're in separate repos with a defined API contract, every iteration on the interface costs you twice. You'll be negotiating across a gap while the design is still fluid.

The facilitator support tool needs both working at the same time. You can't test the full flow — Protocol stage advances AttentionState → chunks get tagged → extraction produces Records with SourceReferences → context query assembles them — without both systems running. Separate repos means running two things locally and maintaining synchronised versions just to develop the part that matters most.

**Why not a monolith**

The open standard goal requires that each system is independently implementable. If the transcript package imports SCDS types directly, or if SCDS has transcript-specific logic baked in, you've made the claim of two separable standards false. Another tool should be able to implement the canonical transcript format without knowing SCDS exists, and vice versa.

The risk of a monorepo isn't the shared repository — it's the temptation to make the packages depend on each other in ways that aren't part of the defined interface.

**The structure that resolves this**

```
packages/transcript    — canonical transcript standard implementation
                         no SCDS imports, no knowledge of Records or Types

packages/scds          — SCDS implementation
                         no transcript imports, references chunks only by ID

packages/bridge        — the interface between them
                         Address, AttentionState, SourceReference
                         this is the cross-addressable architecture, owned by neither

packages/platform      — μDemocracy application
                         uses all three, orchestrates the full flow
```

The bridge package is the key move. It's where the cross-addressable architecture lives — the Address type spans both systems, so neither system owns it. The bridge package is thin, well-defined, and can be published separately as the interoperability specification. It's the formal statement of how the two standards relate.

The discipline: any direct import between `transcript` and `scds` packages is a design review event. They should only communicate through `bridge` types.

**Why this is also right for the dataset goal**

The GitHub issue frames the deliberation dataset as a significant long-term asset — deliberation data essentially doesn't exist for AI training. Exporting that dataset requires both systems packaged together coherently: transcript streams, SCDS Records, Relations, AttentionState traces, SourceReference chains. That's a joint export format that requires both to be present. A monorepo makes this natural; fully separate repos would require a third coordination project.

**The test for whether you've got it right**

At any point, someone should be able to take `packages/transcript` and implement a meeting capture tool with no knowledge of SCDS. Someone else should be able to take `packages/scds` and implement a governance record system that references external source material by ID. And a third party implementing both should be able to use `packages/bridge` to connect them without writing the interface themselves.

If that's true, the open standard goal is met regardless of whether they live in one repo or two.

---

# you asked

message time: 2026-05-24 14:45:09

Now. A deeper question. Typescript?
This will be an entirely AI built system. None of the existing system needs to be kept. 

What Language really makes sense for this

---

# claude response

This is a genuinely open question and the "none of the existing code needs to be kept" freedom changes the calculus significantly.

**The case against TypeScript as the foundation**

TypeScript is excellent for defining schemas — Zod-as-SSOT is a clean pattern. But look at what this system actually does: transcript ingestion, multi-stream merging, semantic extraction, embedding generation, vector search, RAG pipelines, the democracy coach AI layer. The computational profile of this system leans heavily into territory where Python's ecosystem is years ahead.

If you want to run Whisper locally, Python. Local embeddings, Python. LangChain, LlamaIndex, sentence-transformers — all Python-first. The Anthropic and OpenAI clients exist in both, but the tooling built on top of them is predominantly Python.

**The case for Python**

Pydantic v2 is Python's Zod. It's become the foundation of the modern Python AI stack — FastAPI, LangChain, LlamaIndex all build on it. The SCDS schemas as Pydantic models is a natural translation. You lose nothing from the Zod-as-SSOT approach; you gain a language that doesn't fight you when the AI processing starts.

For an entirely AI-built system, Python reduces impedance mismatch. The parts that need AI tooling, the parts that process transcripts, the parts that do retrieval — all in the same language, all with first-class library support.

```
FastAPI          — async HTTP, Pydantic-native, excellent for this
Pydantic v2      — SCDS schemas, canonical transcript schemas
SQLAlchemy 2.0   — async ORM, proven at scale
pgvector         — vector search in Postgres, Python client excellent
Celery / ARQ     — async job processing for transcript pipeline
Anthropic SDK    — Python client is first-class
```

**The spec layer question**

Here's the deeper consideration. SCDS and the canonical transcript format are meant to be open standards — implementable by any tool in any language. If the schemas live as Zod or Pydantic, TypeScript or Python becomes the reference implementation language, which biases adoption.

The right move for open standards: define them as JSON Schema. Generate Pydantic models for the Python reference implementation, TypeScript types for anyone building frontends or TypeScript tools. The spec is language-neutral; the implementations derive from it.

```
schemas/
  scds/
    field.schema.json
    type.schema.json
    record.schema.json
    ...
  transcript/
    canonical-segment.schema.json
    stream.schema.json
    ...

→ generates Python Pydantic models
→ generates TypeScript types
→ could generate Rust structs, Go structs, anything
```

This is also what makes the "permanent addressable open dataset" goal real. If the deliberation dataset is going to be consumed by researchers and training pipelines, JSON Schema as the spec foundation is more durable than "you need to understand Zod" or "you need Pydantic."

**The frontend is still TypeScript**

The facilitation tool — the thing a facilitator actually uses — is a web UI. TypeScript there is essentially mandatory. So the stack becomes Python backend + TypeScript frontend, which is a completely standard split. The bridge package (Address, AttentionState, SourceReference) lives as JSON Schema and generates types for both sides.

**For an entirely AI-built system specifically**

Python reduces the accidental complexity of the ML/AI integration. When Claude is building the transcript extraction pipeline, it doesn't need to navigate TypeScript's awkward ML ecosystem or npm packages that are thin wrappers around Python libraries anyway. The AI tooling is native.

The honest answer is Python backend with JSON Schema as the spec layer, TypeScript only where the browser demands it. The schema generation step (JSON Schema → Pydantic, JSON Schema → TypeScript) is mechanical and can itself be AI-built and maintained.

---

# you asked

message time: 2026-05-24 14:47:23

What about a compiled language like rust?

---

# claude response

Genuinely worth considering, not just dismissing as too hard. Let me think through it honestly.

**Where Rust is compelling for this system**

The transcript pipeline — time alignment, multi-stream deduplication, similarity scoring — is CPU-intensive with well-defined inputs and outputs and high correctness requirements. Rust handles this naturally. The borrow checker and ownership model prevent whole classes of concurrency bugs in exactly the kind of concurrent multi-stream processing this pipeline does.

The SCDS validation engine is an even stronger case. It's pure logic, no external dependencies, performance-sensitive, and the invariants need to be enforced correctly forever. More importantly: Rust compiles to WASM. A Rust validation engine could be embedded in any environment — browsers, Python runtimes via PyO3, Node.js via WASM, edge compute. That would make SCDS validation genuinely language-neutral in a way that a Python or TypeScript reference implementation cannot be. Any tool implementing SCDS in any language could embed the same canonical validator.

This is architecturally significant for an open standard.

**The honest problem with Rust for an AI-built system**

AI code generation for Rust is meaningfully worse than for Python or TypeScript. Not because models don't know Rust — they do — but because the borrow checker creates a feedback loop that lengthens iteration. Write → compile → lifetime error → fix → recompile is slower than write → run → fix for the parts of the system that are still in design flux.

The democracy coach layer, the Protocol definitions, the context assembly logic — these will iterate rapidly while the design stabilises. Rust's type system fights you when you're still figuring out the shape of things. You want fluidity there.

**The interesting inversion**

There's a counterargument specifically for AI-built systems. When Claude writes Rust code that compiles and passes the borrow checker, it's more likely to be correct than equivalent Python that runs but has subtle concurrency or state bugs. The compiler is a second reviewer. For a system maintained entirely by AI over years, Rust's enforced correctness might matter more than development velocity.

The invariants in SCDS — the 34 rules, the addressing scheme, the lineage chains — are exactly the kind of thing Rust's type system can encode so that future AI maintainers literally cannot violate them. `invalid states unrepresentable` is a genuine long-term maintenance argument.

**The architecture this suggests**

Not Rust for everything. But Rust for the components that are:
- Well-defined (no design flux)
- Performance or correctness critical
- Intended to be long-lived and stable
- Worth embedding in other environments

```
Rust (core):
  scds-validator      — compiles to WASM, embeds anywhere
  transcript-core     — time alignment, merge, deduplication
  address-engine      — Address resolution, AttentionState

Python (application):
  AI layer            — democracy coach, extraction, RAG
  API server          — FastAPI, orchestration
  facilitation logic  — Protocol runner, context assembly

TypeScript (frontend):
  facilitation UI     — real-time, collaborative
  (uses WASM-compiled scds-validator directly in browser)
```

The bridge package becomes Rust code that generates bindings for Python (PyO3) and TypeScript (wasm-bindgen). The spec is implemented once, correctly, and the other layers use it.

**The practical verdict**

Full Rust for the whole system: too much friction for the AI layer specifically. The ML ecosystem isn't there and the iteration speed on design-fluid components is too slow.

Rust for the core spec engine and transcript processing, Python for the AI application layer, TypeScript for the browser: this is harder to set up but architecturally sounder for the long term. The WASM compilation path for SCDS validation is genuinely compelling as the foundation of an open standard.

The question is whether you want to take that complexity on now, or start with Python throughout and migrate the stable components to Rust once they've settled. The second path is more pragmatic. The first is more correct.

---

# you asked

message time: 2026-05-24 15:34:16

AI development with different languages means very clear separation of concerns. Policing system boundaries with AI code is its own workload

---

# claude response

That's the sharpest practical constraint in this whole decision. And it reframes the question.

With human developers, architectural elegance can survive polyglot because experienced engineers intuit boundaries and self-police. With AI as the primary developer, every boundary is a tax on every session. The AI needs to know which language applies to which change, maintain FFI bindings when interfaces evolve, context-switch between ecosystems, and resist the gravitational pull of "just putting this here because it's easier." That's not architecture overhead — it's prompting overhead, and it compounds.

The natural seams for this system reduce to one unavoidable boundary: the browser requires JavaScript. Everything else is a choice.

**The minimum-seam architecture**

Python backend, TypeScript frontend, HTTP/JSON between them. One seam, at the API boundary, which is already explicit and enforced by the network. No FFI, no WASM bindings to maintain, no build system complexity. The AI working on the backend speaks Python; the AI working on the frontend speaks TypeScript; they meet at an OpenAPI schema.

**The case for collapsing further**

If you commit to API services for all AI processing — Anthropic for generation, Voyage or similar for embeddings, Postgres with pgvector for retrieval — you never actually need Python's ML ecosystem. The backend is making HTTP calls. TypeScript does that as well as Python.

Then the stack is TypeScript everywhere: Bun for the backend, React or similar for the frontend, Zod already in place for SCDS schemas, shared types across the API boundary. One language, one package manager, one linter, one build system. The AI working on the system never needs to context-switch.

This is probably the honest answer if you're willing to commit to API-service-based AI processing. The existing Zod + Drizzle foundation is already right for this.

**The one question that tips it**

Will you ever want to run anything locally — Whisper for transcription, local embeddings, a local model for the democracy coach? If yes, Python wins because the ecosystem advantage is real and you'll fight TypeScript the whole way. If no — if the system is entirely built on hosted AI services — TypeScript is cleaner, simpler, and the seam count is lower.

For μDemocracy specifically: the open dataset goal and the long-term democracy coach ambition both point toward eventually wanting local processing capability. But that's future Greenman's problem. Present Greenman needs to build a working system.

The pragmatic call is TypeScript now with the AI services approach, with the understanding that if local processing becomes necessary, that component gets isolated cleanly then — not pre-emptively.

---

# you asked

message time: 2026-05-24 15:39:49

Interesting. I was actually tending in the opposite direction. When I have completely different languages it's much easier to know what goes where. I can  maintain a clear library because it's in rust, while the python code around it is implementation and the typescript is front-end. AI agents will often end up making a library pull code from implementation so I always need to keep watching

---

# claude response

You're right, and I was thinking about the wrong kind of overhead.

My concern was development-time overhead — the AI needs to know which language to use for each change. Your concern is monitoring-time overhead — catching when the AI has quietly blurred abstraction layers. These are different problems, and yours is harder.

With a single language, an AI agent working on a feature will reach for whatever function is nearby and useful. The import works, the tests pass, and you've now got application logic leaking into library code, or a reverse dependency nobody intended. It's invisible until something breaks in a confusing way. You have to watch for it constantly.

With polyglot, that violation requires an explicit, deliberate act. To make Rust depend on Python, you'd have to write FFI bindings the other way around — which is obviously wrong and structurally prevented. The compiler is the enforcer, not the reviewer.

**The boundary becomes self-documenting**

"This is Rust" carries meaning beyond implementation detail. It means: canonical, stable, correct, no application concerns. "This is Python" means: orchestration, application logic, things that change as the product evolves. "This is TypeScript" means: browser, presentation. An AI agent — or a human — can orient immediately from the language alone.

The library doesn't just happen to be in Rust. It's in Rust *because* it's the library. The language choice is the architectural statement.

**What this means for what lives in Rust**

Given this framing, the Rust layer should contain exactly what you want to be permanently correct and permanently stable:

- SCDS schema validation — all 34 invariants, enforced by the type system where possible
- Address resolution — the canonical Address type, resolvable across all three spaces
- Transcript processing core — time alignment mathematics, deduplication logic
- The bridge types — AttentionState, SourceReference, the interface between the two standards

These are the things that have precise specifications, don't depend on application state, and need to be correct indefinitely. The Rust compiler verifies them at build time. The WASM compilation path means they can be embedded anywhere.

The Python around it is genuinely implementation — how you call the library, how you orchestrate the AI services, how you run Protocols. It changes as the product evolves. It's allowed to be messy in ways the library isn't.

**The monitoring trade-off**

You're trading development-time friction (maintaining FFI bindings, build system complexity) for monitoring-time reliability (the compiler catches boundary violations automatically). For a system maintained primarily by AI agents over a long time horizon, that trade is probably worth it. The compiler never misses a violation. Human or AI code review does.

---

# you asked

message time: 2026-05-24 15:55:20

Scds... Pronounced Seeds? 

Need some good names

---

# claude response

Seeds works — there's something true in it. Fields are semantic seeds. They contain the genetics (AI guidance, validation rules). Records are what grows from them. The lineage is literally botanical.

But let me throw some directions at you.

**For the semantic layer (SCDS)**

*Sema* — Greek for sign/meaning. Clean, short, pronounceable, doesn't over-explain itself. "The sema layer." Works as a technical name.

*Grain* — fundamental unit, granular, also agricultural. Fields are grains of meaning composed into Types. Has the right register for something foundational.

*Sediment* — meaning accumulates in layers, each preserving what came before. Institutional memory as geology. A superseded decision isn't deleted, it's sediment. Might be too heavy.

*Accord* — negotiated agreement, the output of deliberation. Resonates with the democracy context. "Records of accord."

**For the transcript layer**

*Stream* — already implicit in the spec. Simple, accurate.

*Voice* — what it captures. The voice layer beneath the semantic layer.

**The pairing that feels most right**

*Voice and Grain* — or *Stream and Sediment*. The conversation flows; meaning settles out. One captures what was said, the other captures what it meant. The relationship between them is almost geological.

**For the overall platform**

*Ground* — common ground, grounding decisions in evidence, the ground beneath the building. Also: groundwork. Has democratic resonance without being precious about it.

*Commons* — shared infrastructure for collective sense-making. Deliberately political, fits μDemocracy.

**The honest favourite**

Sema for the spec. It's doing the right work — semantic, concise, not trying too hard. Seeds is charming but slightly too playful for founding documents and governance records.

What register are you aiming for — technical standard, or something that needs to speak to non-technical participants in a deliberative democracy context?

---

# you asked

message time: 2026-05-24 17:57:44

Create me a visualization of the scds element and layers

---

# claude response

Four tabs:

**Architecture** — the full layer stack from conversation at the bottom to distribution at the top. Click any layer to expand and explore its types. The right panel shows the cross-layer connections (AttentionState, SourceReference, Protocol → Record, etc.) and the six core principles.

**Relations** — the canonical relation types with their converses, categories, and concrete examples. Plus the namespace/name convention for custom types.

**Protocol Spectrum** — the loose→tight spectrum visualised, with generic and μDemocracy domain protocols, plus ProtocolStage's three key properties (dependsOn, completionCriteria, outputType).

**Record Tiers** — the Note → Typed Record → Record graduation path with per-tier detail, and the one-to-many graduation pattern (one meeting note → multiple Records via derived-from).

EXT badges mark extension-only concepts. Let me know if you want any layer or concept emphasised differently.

---

# you asked

message time: 2026-05-24 18:22:37

this might be one more piece for the spec. IN practice, a field that is just a lump of text cannot really be read. While we may have typed records, we may need tiny bit of markdown format in content

---

# claude response

Right. And it's a clean gap — the spec currently has no way to say "this field produces markdown" versus "this field produces plain prose." The two need to be distinguishable at the definition level.

**Why it's distinct from `editorHint`**

`editorHint` controls the editing surface. `contentFormat` describes what the value actually contains. These are orthogonal:

```
contentFormat: "markdown" + editorHint: "textarea"     
→ produces markdown, edited in a plain textarea

contentFormat: "markdown" + editorHint: "rich-text"    
→ produces markdown, edited with a WYSIWYG surface

contentFormat: "plain"    + editorHint: "textarea"     
→ just prose, no formatting expected
```

A renderer needs `contentFormat` to know whether to parse the value. An AI extractor needs it to know what to produce. Neither is answered by `editorHint`.

**The addition**

On `FieldDefinition`:

```typescript
contentFormat?: "plain" | "markdown"
// Applies only when valueType is "string" or "text"
// Default: "plain" when absent
// "plain"    — unformatted prose; renderers must not interpret markup
// "markdown" — CommonMark subset; renderers should parse and display formatting
//
// Describes the VALUE, not the editing surface (see editorHint)
// AI extractors should produce output conforming to this format —
// a field with contentFormat: "markdown" should receive structured
// markdown from extraction, not flat prose
```

**The NoteSection alignment**

`NoteSection` already has `contentHint: "text" | "markdown" | "plain"`. This addition makes Field-level content format consistent with that vocabulary, just with a cleaner name and tighter scope. The `"plain"` and `"text"` distinction in NoteSection was always slightly redundant — collapsing to `"plain" | "markdown"` at the Field level is cleaner.

**The invariant**

Add to Section 8:

> **35.** `Field.contentFormat`, when present, is only meaningful when `valueType` is `"string"` or `"text"`. Implementations must ignore `contentFormat` on fields with other `valueType` values.

**The AI guidance implication**

This is worth noting explicitly in the spec, not just the rationale: an AI extractor should treat `contentFormat: "markdown"` as an instruction to produce structured output — bullet points, bold, headers where appropriate — rather than flowing prose. The `contentFormat` is as much guidance for the extraction pipeline as it is for the renderer.

**Scope boundary**

`"plain" | "markdown"` is the right limit for the spec. "Rich text" (HTML, custom markup) is too implementation-specific to standardise here. Implementations using rich text editors can map to/from markdown at their boundaries; the spec stays clean.

---

# you asked

message time: 2026-05-24 18:48:09

# Decision Feedback Architecture

**Status**: authoritative
**Owns**: feedback chain model, feedback-to-regeneration contract, feedback source types, MCP feedback tool surface
**Must sync with**: `packages/schema`, `docs/field-regeneration-strategy.md`, `docs/expert-system-architecture.md`, `docs/mcp-architecture-strategy.md`, `docs/plans/iterative-implementation-plan.md`

## Purpose

Structured feedback replaces the former transient `GuidanceSegment` system.

Feedback is a durable, rated, source-attributed record linked to a specific decision context, optionally scoped to a specific field and draft version. The feedback chain is assembled automatically during regeneration and rendered into the LLM prompt.

## Core concepts

### Feedback item

A feedback item captures a human or agent's evaluation of a field value or whole draft:

- **rating** — `approved | needs_work | rejected`
- **source** — `user | expert_agent | peer_user`
- **authorId** — user id or agent name (stable identity for attribution)
- **comment** — free-text assessment
- **textReference** — optional verbatim quote from the field value being annotated (like a Google Docs comment anchor)
- **referenceId / referenceUrl** — optional cross-reference to external artefacts; semantics deliberately open for future use (Jira, Slack, document links, etc.)
- **excludeFromRegeneration** — boolean; when true this item is omitted from the LLM prompt on next regeneration but remains in the historical chain
- **fieldId** — nullable UUID; `null` means whole-draft feedback, non-null means feedback scoped to a specific field
- **draftVersionNumber** — nullable integer; links to a specific snapshot in `decision_contexts.draftVersions`
- **fieldVersionId** — nullable UUID; forward-compatible FK for when the `field_versions` table is introduced (see `docs/plans/field-versioning-schema-proposal.md`); always `null` until then

Feedback does not replace template guidance. Template guidance remains the canonical statement of intent for the active template and its field definitions. Feedback provides contextual steering toward that intent for a specific draft, field, or review cycle.

### Feedback chain

All non-excluded feedback items for a context (or scoped to a specific field) form the **feedback chain**.

During regeneration:

- Template guidance from the active template and field definitions remains part of prompt construction
- `DraftGenerationService` fetches the feedback chain from the database automatically — no inline guidance in the request body
- For field regeneration, field-specific feedback is prioritised, followed by whole-draft feedback (mirrors transcript chunk priority)
- Items with `excludeFromRegeneration = true` are omitted from the assembled chain
- The chain is rendered in the LLM prompt after supplementary content and before the field extraction block

### Sources

- `user` — a human facilitator or meeting participant
- `expert_agent` — an AI agent (connected via the MCP server registry or the core expert system)
- `peer_user` — another user in a review or approval flow

### Relationship to ExpertAdvice

`ExpertAdvice` and `DecisionFeedback` are distinct tables serving different purposes:

| | ExpertAdvice | DecisionFeedback |
|---|---|---|
| Scope | Broad structured analysis | Targeted rating + comment on a specific field or draft |
| Shape | `advice`, `concerns`, `recommendations`, `mcpToolsUsed` | `rating`, `comment`, `textReference` |
| Author | Expert agents (via expert system) | Users, agents, or peers |
| Influence on regeneration | Not directly | Yes — assembled automatically into the LLM prompt |

Expert agents may produce both: an `ExpertAdvice` record for their full analysis and one or more `DecisionFeedback` records for specific field ratings that should guide the next regeneration.

## MCP tool surface

The following MCP tools are exposed from `apps/mcp`:

- **`add_field_feedback`** — create a feedback item (field-scoped or whole-draft)
- **`get_feedback_chain`** — read the non-excluded feedback chain for a context or field
- **`exclude_feedback_item`** — toggle `excludeFromRegeneration` on a specific item

This allows expert agents to submit structured feedback programmatically via the MCP protocol without requiring REST API access. Both REST and MCP surfaces delegate to the same `FeedbackService` (Shared Core pattern).

## Behavioral contract

- Regeneration fetches the persisted feedback chain from the database — no ad-hoc guidance in the request body
- Regeneration preserves template guidance as a distinct prompt layer; feedback augments but does not replace template intent
- Items with `excludeFromRegeneration = true` are silently omitted from the LLM prompt
- Excluded items are never deleted — they remain in the chain and can be re-included by toggling the flag
- Field-specific items (non-null `fieldId`) outrank whole-draft items in field regeneration context assembly
- Feedback is append-only from the user's perspective; deletion is supported but not required for normal workflows

## Prompt rendering

Feedback is rendered between supplementary content and the field extraction block. Field identifiers are UUIDs in storage and APIs; human-readable field names may be derived for prompt labels, but UUID remains the canonical identifier.

```
=== FEEDBACK ON PREVIOUS DRAFT ===
[approved | user | Alice] The decision statement is accurate and concise.
  > "We will migrate to PostgreSQL 16 by Q3." (text reference)

=== FEEDBACK (applies to: options) ===
[needs_work | expert_agent | TechReviewer] Missing the vendor-managed option — Azure Database for PostgreSQL was discussed.
```

Whole-draft feedback (null `fieldId`) goes in the first section. Field-scoped feedback goes in labelled sections per field.

## Persistence

| Layer | Location |
|---|---|
| Canonical Zod schema | `packages/schema/src/index.ts` — `DecisionFeedbackSchema`, `FeedbackRatingSchema`, `FeedbackSourceSchema` |
| Drizzle table | `packages/db/src/schema.ts` — `decisionFeedback` |
| Repository interface | `packages/core/src/interfaces/i-feedback-repository.ts` — `IFeedbackRepository` |
| Repository implementation | `packages/db/src/repositories/feedback-repository.ts` — `DrizzleFeedbackRepository` |
| Service | `packages/core/src/services/feedback-service.ts` — `FeedbackService` |

## REST API surface

| Method | Path | Purpose |
|--------|------|---------|
| `GET`    | `/api/decision-contexts/:id/feedback`                | All feedback for context (all fields) |
| `GET`    | `/api/decision-contexts/:id/feedback/field/:fieldId` | Feedback scoped to one field |
| `POST`   | `/api/decision-contexts/:id/feedback`                | Add a feedback item |
| `PATCH`  | `/api/decision-feedback/:feedbackId/exclude`         | Toggle `excludeFromRegeneration` |
| `DELETE` | `/api/decision-feedback/:feedbackId`                 | Delete a feedback item |

## UI

The `FieldZoom` component renders the feedback chain per field:

- Rating badge (colour-coded: approved=green, needs_work=amber, rejected=red), source label, author, timestamp
- Optional quoted text reference rendered as a styled blockquote
- Per-item toggle to exclude/include from next regeneration (visually muted when excluded)
- "Add feedback" form: rating radio (`approved | needs_work | rejected`), comment textarea, optional quote textarea for `textReference`

## Versioning notes

`fieldVersionId` is stored as a nullable UUID column with no FK constraint at this time. When the `field_versions` table is introduced (see `docs/plans/field-versioning-schema-proposal.md`), an `ALTER TABLE` migration will add the FK constraint. Feedback records created before that migration can be associated retroactively via `draftVersionNumber` linkage.

Here;s another element of the original plan. 
How does this relate to scds?

---

# claude response

Several distinct things are happening here that map to SCDS differently.

**What's already covered**

`ExpertAdvice` — broad structured analysis with advice, concerns, recommendations — is a Record. It's a semantic object produced by an agent, with provenance, that relates to a decision via `evidences` or a custom relation type. It belongs in the SCDS layer as a Type.

The context assembly priority (field-specific feedback outranks whole-draft, excluded items omitted) is a specialised instance of the Context Query pattern in `ext:addressability`. The feedback chain is one component of the broader assembly order — it slots between Revision history and broader session context.

**What points at a gap**

`DecisionFeedback` — targeted rating + comment on a specific field value — isn't quite a Record and isn't quite a Relation. It's an annotation: structured, provenance-bearing, anchored to a specific field value rather than to a whole instance. SCDS doesn't currently have this concept.

The `textReference` field makes the gap sharper. It's a verbatim quote from the field value being annotated — which is precisely the sub-field addressing we deferred from `ext:addressability`. The feedback item needs to say "this specific piece of text in this field value is what I'm commenting on." That requires addressing below the Field level.

**The proposed concept: Annotation**

```typescript
// ext:annotations (new extension)

Annotation {
  annotationId: UUID

  // Scope — what this is about
  recordId: UUID
  fieldId?: UUID          // null = whole-record annotation
  revisionId?: UUID       // which Revision this applies to
  textSelector?: string   // specific text within the field value
                          // (sub-field addressing — see ext:addressability future)

  // Body
  body: string            // the comment / assessment
  motivation?: string     // open string — domain defines vocabulary
                          // e.g. "reviewing", "approving", "flagging"
                          // μDemocracy uses "approved | needs_work | rejected"

  // Attribution
  authorId: string
  authorType: "human" | "ai" | "peer"
  createdAt: ISO8601

  // Context assembly control
  status?: "active" | "resolved" | "excluded"
  // "excluded" = omit from context assembly (maps to excludeFromRegeneration)
  // Excluded annotations are never deleted — historical chain preserved

  sourceRefs?: SourceReference[]
}
```

**What stays application layer**

The rating vocabulary (`approved | needs_work | rejected`) is μDemocracy-specific. SCDS provides the `motivation` slot as an open string; the application defines its own vocabulary. This is the same pattern as `semanticObjectType` on Type and `containerType` on Container — SCDS provides the structure, the domain fills the values.

The `excludeFromRegeneration` boolean maps to `status: "excluded"` — the logic is the same, the name is more general.

The REST API surface, UI components, and the ExpertAdvice/DecisionFeedback distinction are entirely application layer.

**The relationship between the two**

```
ExpertAdvice    → Record (Type: ExpertAnalysis)
                  relates to the decision via evidences or custom relation
                  full semantic object with its own lifecycle

DecisionFeedback → Annotation
                   anchored to a specific field in a specific Record
                   participates in context assembly
                   vocabulary defined by the application (approved/needs_work/rejected)
```

An expert agent can produce both — a Record capturing the full analysis, and one or more Annotations on specific fields for targeted guidance that feeds the next regeneration. The two serve different purposes and live at different levels of the data model.

**The `textSelector` as the sub-field bridge**

This is the piece that finally makes sub-field addressing necessary rather than optional. An annotation without `textSelector` is fine — "this whole field needs work." But the `textReference` use case — anchoring a comment to a specific quoted phrase — requires knowing where in the field value the annotation applies. The W3C Web Annotation model calls this a selector; the mechanism is the same. When sub-field addressing comes into the spec, `Annotation.textSelector` is the first concrete use case.

---

# you asked

message time: 2026-05-24 19:02:16

Here are the tensions and gaps I see in this proposal:
1. `authorType: "peer"` is inconsistent with existing vocabulary
Every other attribution field in SCDS uses `"human" | "ai" | "imported"`. "peer" is a role distinction within the human category, not a type. The μDemocracy domain concern should live in a separate `authorRole` field, keeping `authorType` aligned with `Revision.agent` and `FieldValue.source`.
2. Annotations are instance data, not definitions — no Package home
`Package` holds distributable definitions (Fields, Types, Views, Protocols). Annotations are instance-level state attached to specific Records. They wouldn't appear in `Package.fields[]` or a new `annotations[]` array. The extension should clarify this explicitly — it's a runtime/persistence concern, not a distribution concern.
3. `revisionId` creates an undeclared dependency on `ext:addressability`
`Revision` is defined entirely within `ext:addressability`. If an annotation can reference a `revisionId`, that path requires the addressability extension. This dependency should be stated (as a conditional — "when `revisionId` is present, `ext:addressability` must be declared").
4. `textSelector` — either specify it or drop it
Marking it "future" but including it in the schema leaves the field semantically undefined. Implementations can't validate or interoperate on it. Either remove it now and add it when sub-field addressing is specified, or put it in a clearly-labeled non-normative note.
5. `status: "excluded"` introduces implicit context assembly semantics
The spec is careful to keep context assembly as a behavioural requirement in `ext:addressability`, not in data shapes. `"excluded"` implicitly says "implementations must check this during context assembly" — but that contract isn't declared. Either the extension should formally state the context assembly requirement, or this maps to a `Relation` with `status: "rejected"` to the field/record context.
6. Missing: what invariants does this extension add?
Following the spec's Invariants pattern (Section 8), this extension needs:

* `annotationId` unique
* `recordId` must reference a valid instance
* `fieldId`, when present, must appear in the referenced instance's Type fields
* `revisionId`, when present, must reference a Revision for the same `recordId`/`fieldId`
One structural question: should annotations be first-class queryable objects in the context query patterns (Section 7, ext:addressability), or are they purely a storage concern? If the μDemocracy use case requires "assemble all annotations on this field before AI regeneration," that's a context assembly query pattern that ext:addressability would need to define.

---

# claude response

All six are correct. Working through each:

**1. authorType vocabulary**

Drop `"peer"`, align with existing attribution vocabulary:

```typescript
authorType: "human" | "ai" | "imported"
authorRole?: string
// Domain-specific role within the authorType.
// μDemocracy defines: "facilitator" | "participant" | "expert_agent" | "peer_reviewer"
// Open string — the spec does not enumerate values.
```

Same pattern as `semanticObjectType` and `containerType`. The spec provides the slot; the domain fills the vocabulary.

**2. Annotations as instance data**

Add explicitly to the extension header:

> `ext:annotations` defines a runtime data type, not a distributable definition. Annotations are instance-level state attached to specific Records. They do not appear in `Package` and are not subject to dependency manifest rules. Persistence, querying, and exchange of Annotations is an implementation concern; this extension defines only the data shape and invariants.

Same category as Records themselves — the spec defines the shape of a Record but doesn't include Records in Package.

**3. Conditional dependency on ext:addressability**

```typescript
revisionId?: UUID
// When present, references a Revision for the same recordId and fieldId.
// Requires ext:addressability to be declared.
// See Invariant 38.
```

And in the invariants:

> **38.** When `Annotation.revisionId` is present, the implementation must have declared `ext:addressability`. `revisionId` must reference a `Revision.revisionId` where `Revision.recordId === Annotation.recordId` and `Revision.fieldId === Annotation.fieldId`.

**4. textSelector — drop it**

Remove from the schema. Non-normative note instead:

> Sub-field text selection — anchoring an annotation to a specific span of text within a field value — is reserved for a future extension that will define `textSelector` semantics in coordination with `ext:addressability`. Implementations needing this capability before standardisation should use `meta` for forward-compatible storage.

**5. excluded status**

Split into a storage flag and a declared assembly contract:

```typescript
excludeFromAssembly?: boolean
// default: false
// When true, this annotation is omitted from context assembly.
// The annotation is never deleted — the historical chain is preserved.
// The assembly contract is declared in ext:annotations × ext:addressability interaction below.
```

Remove `status` entirely or keep it as `"active" | "resolved"` for review workflow state, which is separate from the assembly concern. These are orthogonal: a resolved annotation might still be included in assembly; an active annotation might be excluded.

The assembly contract moves to a declared interaction section:

> **When both `ext:annotations` and `ext:addressability` are declared**, the Context Query for a field must include the annotation assembly step: retrieve all Annotations where `recordId` and `fieldId` match, `excludeFromAssembly !== true`, ordered by `createdAt`. Field-scoped annotations (non-null `fieldId`) are assembled before whole-record annotations (null `fieldId`). This is the formal declaration of the assembly semantics implicit in `excludeFromAssembly`.

**6. Invariants**

```
35. Annotation.recordId must reference a valid instance ID 
    (Note.instanceId, Typed Record.instanceId, or Record.instanceId).

36. Annotation.fieldId, when present, must reference a fieldId that appears 
    in the referenced Record's bound Type fields[].

37. An Annotation with null fieldId is a whole-record annotation. 
    An Annotation with non-null fieldId is field-scoped.
    Both are valid; they have different assembly priority (Invariant per ext:addressability interaction).

38. Annotation.revisionId, when present, requires ext:addressability 
    and must reference a Revision for the same recordId and fieldId.
```

**The structural question**

Yes — Annotations should be first-class in Context Query, but as an augmentation declared by the interaction of the two extensions. The assembly order becomes:

```
1. Type and Field aiGuidance
2. Current value and Revision history         ← ext:addressability
3. Active annotations on this Field           ← ext:annotations + ext:addressability
4. Active whole-record annotations            ← ext:annotations + ext:addressability
5. Chunks tagged to this Field via AttentionState
6. Chunks tagged to parent Record
7. Related Records via Relations
```

Steps 3 and 4 only appear when both extensions are declared. Neither extension alone claims this position — it belongs to their interaction. This keeps ext:addressability clean (it defines the assembly framework) and ext:annotations clean (it defines the annotation shape), while the interaction section formally declares the contract that makes the μDemocracy use case work.

**The cleaned-up schema**

```typescript
// ext:annotations

{
  annotationId: UUID

  recordId: UUID
  fieldId?: UUID          // null = whole-record annotation
  revisionId?: UUID       // requires ext:addressability; see Invariant 38

  body: string
  motivation?: string     // open string; domain defines vocabulary
                          // e.g. "reviewing" | "approving" | "flagging"
                          // μDemocracy: "approved" | "needs_work" | "rejected"

  authorId: string
  authorType: "human" | "ai" | "imported"
  authorRole?: string     // domain-specific role; open string
  createdAt: ISO8601

  excludeFromAssembly?: boolean   // default: false; see assembly contract above
  status?: "active" | "resolved"  // review workflow state; orthogonal to assembly

  sourceRefs?: SourceReference[]
  meta?: Record<string, unknown>
}
```

---

# you asked

message time: 2026-05-24 20:57:32

So, this is a document format for tracking semantics change. How does this compare with something like docx

---

# claude response

They're operating at fundamentally different levels, which makes the comparison revealing.

**What DOCX represents**

DOCX is a rendering format. It captures how a document looks — typography, layout, flow, visual hierarchy. Structure is implied by formatting: "Heading 1" is a style, not a semantic declaration. The meaning lives in the human-readable text; the machine only sees presentation.

Track changes in DOCX captures text-level edits — which words changed, who changed them, when. Comments anchor free text to a specific selection. These are the closest DOCX equivalents to SCDS's revision and annotation concepts, but they answer different questions.

**The level difference**

| Concern | DOCX | SCDS |
|---|---|---|
| Primary artifact | The rendered document | The semantic state |
| Structure | Implied by formatting (Heading 1, Normal, etc.) | Declared as Types and Fields |
| Change tracking | Text diff — what words changed | Semantic diff — what the group understood differently |
| Comments | Text-anchored free notes | Field-anchored structured annotations with provenance |
| History | Who changed what text | What conversation produced what understanding |
| Reuse | Paragraphs copy-paste | Records referenced from multiple Document Views |
| Queryability | Parse text to find meaning | Query fields directly |

**The crucial distinction**

DOCX assumes the document is the thing. You author it, edit it, publish it. The document is the artifact.

SCDS assumes meaning is the thing. You converse, extract, refine, and project. The document is one possible view of the semantic state — not the source of truth.

A DOCX heading says: "this text looks like a heading."
An SCDS Type says: "this object *is* a Decision — here are its fields, their extraction guidance, their validation rules, their provenance."

**Track changes vs Revision**

DOCX track changes: "Alice changed 'will migrate' to 'will not migrate' on Thursday."
SCDS Revision: "the decision_statement field changed from V2 to V3 during the Decision Protocol's evaluation stage, sourced from transcript chunks 42–47, which captured the group's consensus after the risk discussion."

One records text mutation. The other records semantic evolution with its full epistemic context.

**Comments vs Annotations**

DOCX comment: a sticky note on selected text, reviewed and resolved.
SCDS Annotation: a structured evaluation of a specific field value with rating, attribution, motivation, and formal participation in context assembly for the next AI generation pass.

**Where DOCX is genuinely better**

Rich formatting, tables, images, page layout, print-readiness. Universal tool support — everyone has Word. Free-form authoring with no schema. Decades of backward compatibility.

SCDS can't do any of that. A Document View can *export* to something DOCX-shaped, but the inverse — extracting SCDS semantics from a DOCX file — requires AI interpretation and loses provenance entirely.

**The relationship**

DOCX is a valid output format for SCDS. A Document View assembles Records into a readable document and could render to DOCX. But DOCX is not a valid input format for SCDS — you'd need the full extraction pipeline to recover semantic structure from it, and you'd still lose the deliberation provenance that makes SCDS meaningful.

The honest comparison: SCDS is to meaning what DOCX is to formatting. They're not competing — they're different layers. DOCX is what you'd hand to a printer. SCDS is what you'd hand to a democracy coach.

---

# you asked

message time: 2026-05-24 21:07:25

Are there things we could learn or adopt from docx?

---

# claude response

Several genuinely useful things. Working through the most interesting ones.

**Threaded annotations**

DOCX comments support reply chains — a reviewer flags something, the author responds, the reviewer resolves it. Our Annotation model is currently flat: each annotation is independent. For the governance review workflow, that's insufficient. A deliberation about whether a decision statement is accurate should be traceable as a thread, not a set of independent remarks pointing at the same field.

The fix is simple:

```typescript
replyToAnnotationId?: UUID
// When present, this annotation is a reply within a thread.
// The root of the thread has no replyToAnnotationId.
// Invariant: replyToAnnotationId must reference an Annotation 
// with the same recordId and fieldId.
```

**Track changes visibility model**

DOCX doesn't just record what changed — it keeps the deleted content visible until explicitly accepted or rejected. You can view "original", "all markup", or "final." This matters for governance: seeing what was *removed* from a decision rationale, and by whom, can be as important as seeing what was added.

SCDS's Revision captures the before and after state, but doesn't address the visibility question. The governance use case suggests a `showRemoved` concept for Revision display — render a diff view showing field-level deletions alongside additions when reviewing a Record's history. This is a View-layer concern, not a data shape concern, but it's worth adding to the ext:views-l1 rationale as an expected rendering pattern.

**Custom XML parts as the direct ancestor**

DOCX introduced custom XML parts in Office 2007 — arbitrary structured XML data embedded in a document, bindable to content controls in the visual layer. This is conceptually identical to what SCDS does: semantic content separated from its visual representation, with typed slots that bind to display elements.

The difference: DOCX scoped it to a single document, never built proper tooling for it, and the binding model was brittle. SCDS takes the same fundamental idea and implements it correctly — field definitions owned independently, multiple Views over the same Records, distribution via Package. Custom XML parts were the right idea at the wrong scale.

Worth acknowledging in the SCDS rationale as prior art that validated the approach.

**Content controls as validation of the field/view split**

DOCX content controls are typed fillable regions — text, date, dropdown, rich text, checkbox — with placeholder text and validation. They sit in the visual layout but bind to structured data. This validates the SCDS model at a practical level: the industry recognised the need for typed semantic slots in documents years ago. The difference is that DOCX content controls are owned by the document template, while SCDS Fields are owned independently and reused across Types.

**Style inheritance as the right model for View inheritance**

DOCX styles inherit from each other cleanly. "Heading 2" inherits from "Heading 1" and overrides specific properties. Child styles carry parent properties unless explicitly overridden. This is exactly the right model for View inheritance — a summary View inherits from the full deliberation View and hides certain fields, without redefining any field semantics. The `extendsViewId` future extension should follow this model explicitly.

**Document protection levels**

DOCX protection has distinct modes: read-only, comments-only (can annotate but not edit), form-filling only (can fill content controls only), tracked changes only. The comments-only mode is particularly interesting — a Record that has reached a `isFinal` lifecycle state but where Annotations are still permitted maps directly to this. Right now SCDS handles this through implementation-layer lock state in `meta`. A formal protection model would make this interoperable.

**The closer relative worth studying**

DITA (Darwin Information Typing Architecture) is actually a closer architectural relative than DOCX. It defines typed topics (concept, task, reference), supports content reuse via references, and assembles documents via maps. Type = DITA topic type, Record = DITA topic, Document View = DITA map. The main difference is that DITA is designed for technical documentation by individual authors, with no deliberation layer, no provenance tracing, and no context assembly for AI. But DITA's content reuse model is mature — specifically the `conref` mechanism for referencing specific elements across topics — and worth examining before finalising how SCDS handles cross-Container Record references.

**What not to bring across**

The rendering-centric model, page layout concepts, and OOXML's notorious complexity. DOCX became extraordinarily complicated because it tried to preserve backward compatibility with thirty years of document formatting behaviour. SCDS has the advantage of starting without that burden.

---

# you asked

message time: 2026-05-24 21:46:47

Ok, let's look at dita and see what we can learn

---

# claude response

DITA is worth studying carefully because it's the most architecturally similar standard to SCDS — and it's had twenty years of production use to reveal what works and what doesn't.

**What DITA is**

DITA (Darwin Information Typing Architecture) is an XML-based standard for technical documentation, originally IBM, now OASIS. Its core model: content is authored as independent typed topics, assembled into publications via maps. A topic is self-contained and reusable. A map is an assembly definition. Sound familiar?

```
DITA topic type  ≈  SCDS Type
DITA topic       ≈  SCDS Record
DITA map         ≈  SCDS Document View
DITA specialization ≈  SCDS Type inheritance (which doesn't exist yet)
```

**Lesson 1: Specialization / Type inheritance**

This is the most significant thing SCDS is missing. DITA's "Darwin" name comes from its specialization model — you can create new topic types that formally inherit from base types, add new elements, constrain existing ones, and still be processable as the base type by systems that don't know the specialization.

```
topic (base)
  ↑ specializes
concept (adds: "what is this" structure)
  ↑ specializes
org.mu-democracy/principle (adds: expression_domain, tension_with)
```

A system that understands `concept` can process `principle` — it just won't see the specialized fields. The specialization is formal, declared, and traceable.

SCDS needs `extendsTypeId?: UUID` on Type. A GovernanceDecision specializes Decision and adds `ratification_method`, `quorum_threshold`. A ConstitutionalDecision specializes GovernanceDecision and adds `amendment_procedure`. Each level is independently valid. Downstream systems that know only Decision can still process ConstitutionalDecision — they see the base fields.

The invariant that DITA enforces strictly: a specialization cannot remove base type fields, only add or constrain them. This is what makes graceful degradation work.

**Lesson 2: Field domains**

DITA has "domains" — named sets of elements that can be mixed into any topic type. A "programming domain" adds code blocks and syntax elements. A "governance domain" could add ratification and quorum elements. Domains are designed to travel together as coherent groups.

SCDS has FieldGroups (repeated structured context within an instance) but no equivalent concept at definition time — no named grouping of Fields that are designed to be included together.

A `FieldDomain` concept:

```typescript
// ext:field-domains (new extension)
FieldDomain {
  id: UUID
  namespace: string
  name: string
  description: string
  fields: FieldRef[]
  // A named set of semantically coherent Fields
  // intended to be included as a group in Type definitions
  // e.g. "governance-domain" → ratification_method, quorum_count, voting_record
  //      "provenance-domain" → source_meeting, decision_date, facilitator
}
```

Types can then declare `includesDomains?: FieldDomainRef[]` alongside their explicit field list. This makes Type composition cleaner for domain authors and makes it easier to query "which Types include governance domain fields."

**Lesson 3: Relationship tables**

DITA maps have relationship tables — a separate declaration of relationships between topics that doesn't live inside the topic content itself. Topics stay self-contained; navigational relationships are declared at the map level.

This reveals an important distinction SCDS conflates: semantic relations versus navigational relations. 

```
Semantic:   "Decision D supersedes Decision D-001"
            → belongs on the Records themselves
            → SCDS Relation is correct here

Navigational: "The founding document links to the values list"  
              → belongs at Document View level
              → shouldn't require a Relation between Records
```

Document View currently handles this implicitly via SectionSource queries, but there's no explicit mechanism for declaring navigational relationships at the Document View level without requiring semantic Relations on the Records. Worth adding to the Document View a `navigationLinks?: RelationRef[]` or similar — "when rendering this document, surface these cross-section links" — separate from the semantic Relation graph.

**Lesson 4: The conref transclusion model**

DITA's `conref` allows pulling specific content from another topic inline — transclusion rather than just reference. You reference a specific element in another file and it appears as if it's in the current document, maintained in one place.

For SCDS Document Views, SectionSource includes whole Records in sections. But there's no mechanism for transcluding specific field values from a Record into prose in another section. The founding document might want to include the exact text of a key decision statement inline in the preamble rather than as a separate section.

A `FieldTransclusion` in ExportConfig or preamble templates:

```
{{field:recordId/fieldId}}
```

Pull the current value of a specific field from a specific Record inline into a template string. This is a small addition to the preamble variable substitution system (`{{container-title}}`, `{{date}}` already exist) that makes Document Views significantly more expressive.

**Lesson 5: Conditional processing**

DITA has `@audience`, `@platform`, `@product` attributes on any element, and a filtering step that includes or excludes content based on these conditions. The same source produces different outputs for different audiences.

SCDS has `visible?: boolean` on FieldView and `emptyBehavior` on DocumentSection. But there's no conditional filtering based on Record properties or audience context. A governance document for the full membership might include the deliberation history; a summary for external stakeholders might exclude it.

The SectionSource `lifecycleState` filter does some of this. But a more general `conditions?: Record<string, string>` on DocumentSection — evaluated against Container or Record metadata — would enable proper audience-targeted projection. Worth flagging as a future extension rather than adding now.

**What DITA's failures tell us**

DITA has been around twenty years and is still primarily used by large enterprises with dedicated technical writing teams and tooling budgets. The learning curve is steep. The XML is verbose. The specialization model, while powerful, requires deep expertise to use correctly. Tooling fragmentation means different DITA processors interpret the spec differently.

Three specific warnings:

First, *complexity creep*. DITA 1.3 added branch filtering, scoped keys, and troubleshooting topics. Each addition was justified individually. The cumulative result is a spec that takes months to learn. SCDS's extension model is the right guard against this — each extension is independently adoptable and the core stays lean.

Second, *the map/topic impedance mismatch*. Topics are supposed to be standalone but they frequently end up containing context that only makes sense within a specific map. The "context-free topic" ideal is hard to maintain in practice. For SCDS, this is the warning about Records that are only meaningful within a specific Container — the spec should encourage Records that carry sufficient context to be meaningful in isolation.

Third, *specialization without tooling is theory*. DITA specialization is powerful on paper. In practice, most organizations define one or two custom topic types and stop, because the tooling cost is high. SCDS Type inheritance should be designed to work well with minimal tooling — the basic case (one level of inheritance, a few added fields) should require nothing beyond declaring `extendsTypeId` and listing the additional fields.

**What SCDS already does better**

DITA has no Protocol layer — content is manually authored with no model of deliberation. It has no provenance tracing to source conversations. No AI guidance on fields. No lifecycle states on topics. No Annotation/feedback layer. No addressability for context assembly. No progressive formalisation tiers. The deliberation-first architecture of SCDS has no analog in DITA.

DITA is what you build when you need structured reusable content for documentation. SCDS is what you build when you need structured reusable meaning from collaborative sense-making. The technical documentation use case and the governance use case have enough in common that DITA's twenty years of lessons are directly applicable — particularly specialization, domains, and the semantic/navigational relation distinction. The delivery is very different.

---

# you asked

message time: 2026-05-24 21:55:26

Create a proposal for how scds might adopt based on this review of dita and docx

---

# claude response

Eleven adoptions across four priority tiers. A few things worth flagging:

**The biggest structural addition** is `ext:type-inheritance` (C). It's the most architecturally significant gap and the hardest to add later once a large Type library exists — retrofitting inheritance onto established Types is painful. Worth prioritising even though the implementation cost is non-trivial.

**The most immediately useful** for μDemocracy are B (annotation threading — directly solves the review workflow), E (view protection levels — governance records need formal read-only states), and A (contentFormat — every text field needs this).

**The DITA warning in the complexity budget section** is genuine. DITA specialisation is theoretically powerful and practically underused because tooling never caught up. The proposal scopes `ext:type-inheritance` to the common case (one level, a few added fields) rather than optimising for deep hierarchies. Worth holding that line.

**Three open questions** are flagged — the bundling rules for multi-level inheritance, domain versioning guidance, and the protection/lifecycle interaction. These need worked examples in the rationale before the spec can claim these extensions are stable.

---

# you asked

message time: 2026-05-24 22:05:31

here's some feedback from our last annotation schema:  

Lets blend in this annotation with our adoption proposal

---

# you asked

message time: 2026-05-24 22:05:52

Invariant numbering collision
The spec currently ends at Invariant 38 (`contentFormat` / `valueType`). The proposed invariants numbered 35–38 collide with existing ones. They should be 39–42.
Where does the interaction section live?
The spec has no precedent for a cross-extension interaction section. Two options:

* Append it to the end of `ext:annotations` as a conditional block: "When both `ext:annotations` and `ext:addressability` are declared..."
* Add a new Section 8.5 or similar: "Extension interactions", covering any pair whose combined behaviour requires explicit declaration.
The second is more maintainable as interactions accumulate (e.g., ext:protocol × ext:addressability already has implicit interaction in the non-normative example). If the spec is going to grow, Section 8-and-a-half is the right home for this class of content.
`status: "active" | "resolved"` — does it need "superseded"?
If annotations can be superseded by a later annotation (e.g., a reviewer changes their assessment), there are two paths:

* Relation graph: new annotation + `supersedes` Relation from new to old — consistent with how Records handle revision, and the old annotation stays untouched
* `status: "superseded"`: mutate the old annotation's status in-place
The Relation graph path is more consistent with the rest of the spec and avoids introducing mutable status. But it requires `Annotation` to be a valid target for a `supersedes` Relation — which requires that `annotationId` can appear in `Relation.sourceInstanceId` or `targetInstanceId`. Right now `annotationId` is not an instance ID in the SCDS sense (Notes, Typed Records, and Records are the instance tier). Worth deciding whether Annotations are promotable to the instance tier or whether supersession is handled by `status`

---

# claude response

Now I have the full picture. Four targeted changes needed: invariant renumbering, annotation supersession resolution, extension interactions section, and blending the annotation feedback into Adoption B.

All three feedback points resolved and blended in:

**Invariant numbering** — the summary table now states the baseline (spec ends at 38) explicitly. Adoption A notes its invariant is already at 38 in the spec. All new invariants run 39–49. The numbering logic is traceable: 35–38 covers the annotation schema + contentFormat already in spec; 39–40 covers B (threading + supersession); 41–44 covers C (type inheritance); 45 covers E (view protection); 46–47 covers F (field domains); 48–49 covers G and H.

**Annotation supersession** — Adoption B now has three sub-sections. B.2 explicitly argues through the Relation-graph path, states the decision (status mutation, not instance tier promotion), explains the rationale (Annotations are quality signals not semantic state, Principle 6 applies differently), and proposes the `supersedingAnnotationFor` field. The instance tier boundary is formalised with an explicit spec statement that must be added to Section 4.

**Section 8.5** — Added as a full proposed section at the end, with both known interactions formally stated: ext:annotations × ext:addressability (context assembly order with steps 3-4 explicitly positioned) and ext:protocol × ext:addressability (AttentionState advancement contract, formalising what was previously only in the non-normative example). Open Question 4 proposes the working definition for what qualifies as a Section 8.5 entry.

---

# you asked

message time: 2026-05-24 22:18:02

1. Invariant baseline note is wrong
"Invariants 35–38 in the spec cover the annotation schema"
This is incorrect. The spec's Invariants 35–38 cover Distribution (Views L2, Schema, Protocol) and contentFormat. The annotation invariants from our conversation are proposed additions starting at 39, not yet in the spec. The baseline note should read: "This proposal adds Invariants 39–49, including the annotation schema invariants proposed in the ext:annotations review."
2. `overrideFieldOrder` directly contradicts Invariant 42
The `overrideFieldOrder: true` description says:
"Authors must include all inherited field UUIDs in `fields[]` when using this option."
Invariant 42 says:
"A specializing Type must not declare a `fieldId` in `fields[]` that duplicates any `fieldId` inherited from the base Type."
These are mutually exclusive. `overrideFieldOrder` requires inherited UUIDs in `fields[]`; Invariant 42 prohibits it. Either the mechanism is wrong or the invariant is wrong. One resolution: instead of reusing `fields[]`, introduce a separate `fieldOrder?: UUID[]` array on the specializing Type that is only an ordering declaration and is explicitly exempt from Invariant 42. Or drop `overrideFieldOrder` entirely — it's an edge case that adds implementation cost disproportionate to value.
3. Invariant 43 is checking something that can never happen
"A specializing Type must not increase the `required` constraint of an inherited field."
Under the current rules (no re-declaration of inherited fields), a specializing Type has no mechanism to set `required` on an inherited field at all. Invariant 42 prevents the field from appearing in `fields[]`. So the constraint being guarded against is structurally impossible, and Invariant 43 is vacuously true.
This exposes a real design gap: if specializations can't override any `FieldAssignment` properties of inherited fields, that's a significant limitation (no relabelling, no ordering adjustment, no `required` relaxation). If they can, there needs to be an explicit override mechanism — e.g., `fieldAssignmentOverrides?: { fieldId: UUID; displayLabel?: string; required?: boolean }[]` — with Invariant 43 then guarding the `required` property within that override. The current draft is silent on this question.
4. `transclusionFallback` is on the wrong type
`ExportConfig` is defined in `ext:views-l1` and lives on `View` — a single-Record presentation. Field transclusion `{{field:...}}` is being introduced in `ext:views-l2` and applies to `DocumentView.preamble`, `DocumentSection.title`, and `DocumentSection.description`. `DocumentView` has no `ExportConfig`. Either add `transclusionFallback` directly to `DocumentView`, or define a `DocumentViewExportConfig` that `DocumentView` gets alongside `format`. Putting it on the `View`-level `ExportConfig` is the wrong attachment point.
5. Open Question 1 needs a decision, not just documentation
The transitive closure question for multi-level inheritance has a straightforward answer: yes, `Package.dependencyRefs` must include the full transitive closure of base Types for any Type in `types[]`. This should be stated as a corollary to Invariant 44 (or as Invariant 44 itself, made more precise) rather than left as an open question — it's an invariant the spec needs to enforce, not optional guidance.

---

# claude response

Five precise fixes. Let me work through them in order.

All five points addressed:

**1. Baseline note** — corrected to state that invariants 35–38 cover Distribution/Views L2/Schema/Protocol/contentFormat, and that annotation schema invariants are proposed additions starting at 39.

**2. `overrideFieldOrder` contradiction** — removed entirely. Replaced with `fieldOrder?: UUID[]` (ordering-only declaration, exempt from Invariant 42) and `fieldAssignmentOverrides?: FieldAssignmentOverride[]` (presentation overrides for inherited fields: displayLabel, displayHint, required relaxation only). These solve the two real problems `overrideFieldOrder` was trying to address without the contradiction.

**3. Invariant 43 vacuous truth** — removed. Replaced by two meaningful invariants: 43 (fieldOrder must contain exactly the effective field set) and 44 (fieldAssignmentOverrides.required may not tighten). Invariant 44 now guards something that can actually happen.

**4. `transclusionFallback` attachment** — moved from `ExportConfig` to `DocumentView` directly, with an explicit note explaining why: `ExportConfig` lives on `View` in ext:views-l1 and doesn't apply to `DocumentView`.

**5. Open Question 1** — closed as resolved. Invariant 45 now states the transitive closure rule explicitly. Open Question 1 is struck through and annotated. A new Open Question 5 was added (the `fieldAssignmentOverrides` × `ext:views-l1` precedence rule) which surfaced as a consequence of the fix to point 3.

---

# you asked

message time: 2026-05-24 23:15:23

# SCDS Specification

**Version**: 2.0-draft
**Status**: active draft
**Scope**: field definitions (Field), type definitions (Type), records (Note / Typed Record / Record), relations, containers, distribution, and optional extensions covering addressability, lifecycle, protocol, schema, type inheritance, views, repeatable fields, field groups, cross-field validation, recommended relations, import tracking, and registry.

> **Migration note**: This document supersedes `scds-schema.md` (v1.0-draft). A vocabulary and structural mapping from v1 to v2 is in `scds-shema-evolution.md`. Design rationale, usage guidance, and commentary are in `scds-rationale.md`.

---

## 1. Purpose and Scope

### What this specification defines

The Semantic Collaborative Document System (SCDS) specification defines an interoperable standard for semantic field and type definitions, records, relations, and the mechanisms by which these artefacts are created, shared, versioned, and distributed across independent implementations.

This specification covers:

- **Field** — atomic reusable semantic unit
- **Type** — named composition of fields for a specific semantic object type
- **Record** — instantiated type with field values; three semantic maturity tiers (Note, Typed Record, Record)
- **Relation** — first-class typed link between records
- **Container** — grouping boundary for record collections
- **Distribution** — Package, Reference, Lineage, Provenance
- **Extensions** — optional, independently adoptable capabilities declared by conforming implementations

### What this specification does not define

- **Session** — live collaborative process model (future version)
- **Registry protocol** — how registries communicate, authenticate, or federate; this specification defines data shapes only
- **Universal semantic ontology** — domain-specific vocabularies are the responsibility of namespace authors

### Relationship to implementing systems

This specification is implementation-neutral. Implementations are expected to validate inputs against these schemas at their system boundaries. The specification does not constrain persistence technology, API design, UI rendering, or prompt assembly strategy.

### Extension conformance model

Implementations declare conformance as:

```
SCDS Core [+ ext:<name> ...]
```

**Core** requires the Foundation group and Distribution group in full. No extension is required for core conformance. Extensions are independently adoptable; some declare dependencies on other extensions.

| Extension | Identifier | Depends on | Notes |
|---|---|---|---|
| Addressability | `ext:addressability` | — | For live facilitation, declare together with `ext:protocol` |
| Lifecycle | `ext:lifecycle` | — | |
| Protocol | `ext:protocol` | `ext:lifecycle` (recommended) | For live facilitation, declare together with `ext:addressability` |
| Schema | `ext:schema` | — | |
| Type Inheritance | `ext:type-inheritance` | — | |
| Views L1 | `ext:views-l1` | — | |
| Views L2 | `ext:views-l2` | `ext:views-l1` | |
| Repeatable Fields | `ext:repeatable-fields` | — | |
| Field Groups | `ext:field-groups` | — | Group repeatability is self-contained; `ext:repeatable-fields` is not required |
| Cross-Field Validation | `ext:cross-field-validation` | — | |
| Recommended Relations | `ext:recommended-relations` | — | |
| Import Tracking | `ext:import-tracking` | — | |
| Registry | `ext:registry` | — | |

`ext:protocol` and `ext:addressability` are formally independent but are a functional co-dependency for live facilitation: a Protocol without `AttentionState` produces no live conversation tagging; `AttentionState` without Protocol stages has no stage context to capture. Implementations supporting live facilitation should declare both.

Example declaration: `SCDS Core + ext:lifecycle + ext:protocol + ext:views-l1 + ext:addressability`

---

## 2. Namespace Format

### Convention

Namespaces are dot-separated identifiers using lowercase alphanumeric characters and hyphens.

```
<component>[.<component>]*

component = [a-z0-9][a-z0-9-]*
```

Examples:
```
core
community.adr
com.acme.hr
org.cooperative-name
```

### Reserved namespaces

`core` is reserved for definitions maintained by the SCDS standard. Implementations must not allow user-created definitions in the `core` namespace.

### Reference format

A specific version of a definition is referenced using the canonical form:

```
namespace/name@version
```

Examples:
```
core/decision_statement@2
community.adr/review_rationale@1
com.acme.hr/headcount_impact@3
```

The `/` and `@` characters are reserved separators. They must not appear within a namespace component or a name.

### Name convention

Field and Type names are programmatic keys in `snake_case`. Names are stable within a namespace and version lineage. A new name means a new definition.

---

## 3. Schema Notation

Types are described using TypeScript-style notation. Optional fields are marked with `?`. All `UUID` values are RFC 4122 UUID strings. All `ISO8601` values are datetime strings with timezone offset. `integer` means a positive integer unless otherwise noted.

### Version semantics

Version numbers are positive integers scoped to a definition's UUID lineage.

| Change | Version action |
|---|---|
| Documentation, typo, formatting only | Optional bump |
| `description`, `instructions`, or `aiGuidance.purpose` reworded without semantic change | Minor bump recommended |
| `aiGuidance.extraction` or `aiGuidance.purpose` changed in meaning | Version bump required |
| `valueType`, `selectOptions`, or `validationRules` changed | Version bump required |
| `name` changed | New definition required (new UUID) |
| `namespace` changed | New definition required (new UUID) |

When in doubt: if a downstream consumer's AI extraction, validation, or governance logic would behave differently, a version bump is required.

---

## 4. Foundation Group (Core)

The Foundation group is required for all conforming implementations.

### 4.1 Supporting types

#### `ValidationRule`

A constraint applied to a field value.

```typescript
{
  type: "required" | "minLength" | "maxLength" | "pattern" | "enum"
  value?: string | number | string[]  // required for minLength, maxLength, pattern, enum
  message?: string
}
```

#### `AiGuidanceExample`

A single example for AI guidance.

```typescript
{
  description?: string  // labels this example
  input?: string        // sample source text; omit for output-only examples
  output: string        // the ideal value the AI should produce
}
```

`output` is required. An example without `input` demonstrates expected output form without requiring a specific source.

#### `AiGuidance`

Structured AI guidance for a Field or Type.

```typescript
{
  purpose: string            // what this field/type captures (1-2 sentences)
  extraction?: string        // LLM instruction for how to extract or populate
  negativeGuidance?: string  // what the LLM must NOT include or do
  examples?: AiGuidanceExample[]
}
```

The minimum valid `AiGuidance` is `{ purpose: "..." }`.

---

### 4.2 Field

The atomic reusable semantic unit. Fields are defined once and composed into Types. A Field's `aiGuidance`, `validationRules`, and `valueType` belong to the Field, not to any Type that includes it.

```typescript
{
  // Stable identity
  id: UUID
  namespace: string
  name: string       // snake_case programmatic key
  version: integer   // min: 1; increments within this id's lineage

  // Semantic content
  description: string      // one-sentence user-facing summary
  instructions?: string    // fuller guidance for a human completing this field
  aiGuidance: AiGuidance

  // Value semantics — stable across renderers
  valueType: "string" | "text" | "number" | "boolean" | "date" | "url" | "select" | "multiselect"
  selectOptions?: string[]   // required when valueType is "select" or "multiselect"
  validationRules?: ValidationRule[]
  contentFormat?: "plain" | "markdown"
  // Meaningful only when valueType is "string" or "text". Default: "plain".
  // Describes the content of the value, not the editing surface (see editorHint).
  // "plain"    — unformatted prose; renderers must not interpret markup
  // "markdown" — CommonMark subset; renderers should parse and display formatting
  // AI extractors must produce output conforming to this format: a field with
  // contentFormat "markdown" should receive structured markdown from extraction.

  // Editor hint — projection-specific default; implementations and Views may override
  editorHint?: "singleline" | "textarea" | "rich-text" | "date-picker" | "dropdown" | "multi-select" | "voice"

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: Lineage      // see Distribution group
  provenance?: Provenance
}
```

**`valueType` semantics:**

| Value | Meaning |
|---|---|
| `"string"` | Short single-value text (typically one line) |
| `"text"` | Potentially long multi-paragraph prose |
| `"number"` | Numeric value |
| `"boolean"` | True/false |
| `"date"` | ISO 8601 date or datetime |
| `"url"` | A URL string |
| `"select"` | One value from `selectOptions` |
| `"multiselect"` | One or more values from `selectOptions` |

`valueType` is the stable semantic data type. `editorHint` is a rendering default. AI extraction, validation, and export formatting must depend only on `valueType`. `contentFormat` refines how `string` and `text` values should be produced and rendered, but does not alter the `valueType`.

---

### 4.3 Type

A named, versioned composition of Fields for a specific semantic object type.

```typescript
{
  // Stable identity
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  // Content
  description: string        // when to use this Type; what semantic object it defines
  aiGuidance?: AiGuidance    // Type-level LLM framing; see AI guidance composition in rationale

  // Semantic object type (optional, informative)
  semanticObjectType?: string
  // e.g. "decision", "task", "risk", "budget_line", "requirement"
  // Free-form. Implementations may use as a rendering or grouping hint.
  // No conforming implementation is required to act on it.

  // Composition
  fields: FieldAssignment[]
  // type inheritance, fieldGroups, and validationRules are extensions; see
  // ext:type-inheritance, ext:field-groups, and ext:cross-field-validation

  // lifecycle is an extension; see ext:lifecycle

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

#### `FieldAssignment`

A Field reference within a Type. Configures presentation without redefining field semantics.

```typescript
{
  fieldId: UUID     // references Field.id
  order: integer    // min: 0; display and processing order within the Type
  required?: boolean  // default: true

  // Presentation-only — must NOT affect AI guidance, extraction, valueType, or validation
  displayLabel?: string
  displayHint?: string
}
```

`displayLabel` and `displayHint` are strictly for rendering. If a materially different label or meaning is needed, a distinct Field with its own lineage is required.

Repeatability fields (`repeatable`, `minItems`, `maxItems`) are defined in `ext:repeatable-fields`.

The Type's effective field list is `fields[]` unless `ext:type-inheritance` is declared and the Type extends another Type. In that case, the effective field list also includes inherited fields as defined by `ext:type-inheritance`.

**AI guidance composition order** (recommended):

1. Type framing (`Type.aiGuidance.extraction`) — establishes the semantic object type
2. View framing (`View.aiGuidance.extraction`, if `ext:views-l1` is in use) — workflow-specific context
3. Field extraction guidance (`Field.aiGuidance.extraction`)
4. Negative guidance (`Field.aiGuidance.negativeGuidance`)
5. Examples (`Field.aiGuidance.examples`)

This is a recommended default, not a required invariant. Implementations that compose differently will produce different AI behaviour from the same definitions.

**On instance migration when a Type version changes:**
A Record binds to a specific `typeVersion` at creation time. Existing Records do not automatically migrate when a new Type version is published. Conformance is measured against the version the Record was instantiated under. When a Record is migrated and exchanged, it should carry the version it now conforms to, and the original Record should be preserved and linked via a `supersedes` Relation.

---

### 4.4 Record tiers

SCDS supports three semantic maturity tiers. Implementations are not required to support all three; they may begin at Tier 2.

| Tier | Type | Structure | Semantics |
|---|---|---|---|
| **0** | `Note` | Named sections + free text | None |
| **1** | `Typed Record` | Named fields with types and values | Minimal |
| **2** | `Record` | Fields bound to a `Type` definition | Full |

Graduation path: Note → Typed Record → Record.

#### `NoteSection`

A named text section within a Note.

```typescript
{
  name: string          // section key; unique within the Note; snake_case recommended
  label?: string
  content: string
  contentHint?: "text" | "markdown" | "plain"  // hint only; default: "text"
}
```

#### `Note`

A lightweight instance with no Type binding.

```typescript
{
  instanceId: UUID

  title?: string
  sections: NoteSection[]

  graduatedAt?: ISO8601
  // When set, signals full formalisation. Authoritative record of successors
  // is in derived-from Relations from the successor Records.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

#### `TypedField`

A field within a Typed Record.

```typescript
{
  name: string
  label?: string
  valueType?: "string" | "text" | "number" | "boolean" | "date" | "url" | "select" | "multiselect"
  selectOptions?: string[]
  value: string | number | boolean | string[] | null
  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
}
```

#### `Typed Record`

A structured instance with named, typed fields but no Type binding.

```typescript
{
  instanceId: UUID

  title?: string
  instanceType?: string  // lightweight semantic hint; not a formal type declaration

  fields: TypedField[]

  graduatedAt?: ISO8601

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

#### `SourceReference`

A pointer from a field value or instance back to source material.

```typescript
{
  sourceType: "transcript-chunk" | "transcript-segment" | "external-document"
  sourceId: string
  sourceStandard?: string   // versioned standard the source conforms to
  streamId?: UUID           // for transcript sources: originating stream

  relationType?: "evidence" | "derived-from" | "quoted-from" | "inspired-by" | "supersedes-context"

  confidence?: number       // 0.0–1.0
  note?: string
}
```

#### `FieldValue`

The current value of a Field within a Record.

```typescript
{
  fieldId: UUID

  // Non-repeatable — use value
  value?: string | number | boolean | string[] | null

  // Repeatable — use entries (ext:repeatable-fields)
  entries?: FieldValueEntry[]

  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601

  sourceRefs?: SourceReference[]
}
```

`FieldValueEntry` is defined in `ext:repeatable-fields`.

#### `Record`

An instantiated Type with field values.

```typescript
{
  instanceId: UUID
  typeId: UUID         // references Type.id
  typeVersion: integer
  typeNamespace: string
  typeName: string

  // lifecycleState is ext:lifecycle
  lifecycleState?: string

  fieldValues: FieldValue[]

  // groupValues is ext:field-groups
  groupValues?: FieldGroupValue[]

  sourceRefs?: SourceReference[]

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
  // Use meta for implementation-local concerns: lock state, visibility,
  // session references. Cross-system keys should be namespaced,
  // e.g. "com.acme.locking.locked-by".
}
```

`typeNamespace` and `typeName` are denormalised convenience fields. If they conflict with the resolved Type, the `typeId`/`typeVersion` identity takes precedence and the Record is considered invalid until corrected.

**On instance revision:**
- **In-place edits** (`updatedAt` advances, `fieldValues` mutate): for minor corrections that do not alter semantic meaning.
- **Semantic updates**: produce a new Record linked to the prior by a `supersedes` or `refines` Relation. The prior Record remains valid.
- **Immutable records + Relation graph**: all Records append-only; a new Record for every change. A valid implementation strategy that naturally preserves history.

---

### 4.5 Relation

A first-class typed link between instances. Relations allow implementations to construct semantic graphs for navigation, analysis, projection, and reasoning.

```typescript
{
  relationId: UUID

  relationType: string
  // Free-form. See ext:recommended-relations for canonical types and conventions.

  // source [relationType] target
  sourceInstanceId: UUID    // the asserting instance
  targetInstanceId: UUID    // the related instance

  assertedBy?: "human" | "ai" | "imported"
  confidence?: number       // 0.0–1.0; meaningful for ai-asserted
  createdAt?: ISO8601
  createdBy?: string

  status?: "proposed" | "active" | "rejected" | "superseded"
  validFrom?: ISO8601
  validUntil?: ISO8601

  notes?: string
  sourceRefs?: SourceReference[]
  meta?: Record<string, unknown>
}
```

**Directionality convention:**
`sourceInstanceId` is the asserting instance; `targetInstanceId` is the related instance. The Relation reads: "source [relationType] target."

| Relation | source | target |
|---|---|---|
| `supersedes` | the newer Record | the older Record |
| `contains` | the stage | the task inside it |
| `depends-on` | the dependent task | the task it needs |
| `refines` | the detailed version | the rough version |
| `derived-from` | the successor | the source Note or Record |
| `evidences` | the source material | the claim it supports |

This convention must be consistent across implementations. See Invariant 16.

Relations span tiers. A Note may be the target of `derived-from` Relations from the Records it graduated into.

**Canonical relation types** (use these exact strings for cross-system interoperability):

`contains`, `depends-on`, `supersedes`, `refines`, `derived-from`, `evidences`, `precedes`

Custom types not covered by these should use `namespace/name` format (e.g. `com.acme.hr/transferred-to`) to prevent collision. Extended relation type metadata is defined in `ext:recommended-relations`.

**Relations do not change lifecycle state.** A `supersedes` Relation does not mutate the prior Record's `lifecycleState`. Lifecycle state changes are explicit acts by an implementation's transition mechanism.

---

### 4.6 Container

A lightweight grouping boundary over a collection of instances. Containers answer scoping questions — which instances belong together, what constitutes "this project" — that the Relation graph alone cannot answer.

Containers are not semantic objects with Fields. They do not own semantic state; Records do. A `contains` Relation asserts "A is part of B" (a semantic claim); a Container asserts "these instances form a unit for boundary purposes" (a scope claim). Both are needed; neither replaces the other.

```typescript
{
  containerId: UUID

  namespace?: string
  name?: string

  title: string              // human-readable label

  containerType?: string     // free-form hint; e.g. "project", "meeting", "sprint"

  rootInstanceIds?: UUID[]
  // Top-level instances this Container was created to hold. Implementations may
  // derive nested members by traversing contains Relations from these roots.

  memberInstanceIds?: UUID[]
  // Explicit membership list for all instances in scope.
  // When present, allows membership queries without graph traversal.
  // When omitted, membership is defined by traversing contains Relations.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

`Container.containerId` is not an instance ID and must not appear in `Relation.sourceInstanceId` or `targetInstanceId`. See Invariant 19.

---

## 5. Distribution Group (Core)

The Distribution group is required for all conforming implementations.

### 5.1 Package

The distributable artefact. Contains Field, Type, View, and Relation type definitions with a complete dependency manifest.

```typescript
{
  schemaVersion: string      // SCDS spec version, e.g. "2.0"
  packageId: UUID
  packageName: string
  packageVersion: string     // semver, e.g. "1.2.0"
  publishedAt: ISO8601
  publisher?: string
  description?: string
  homepage?: string

  // Content (at least one of fields or types must be non-empty)
  fields: Field[]
  types: Type[]
  views?: View[]             // ext:views-l1; omit if not in use
  documentViews?: DocumentView[]  // ext:views-l2; omit if not in use
  schemas?: Schema[]         // ext:schema; omit if not in use
  protocols?: Protocol[]     // ext:protocol; omit if not in use
  relationTypes?: RelationTypeDefinition[]  // ext:recommended-relations

  mode: "bundled" | "standalone"

  dependencyRefs: Reference[]
}
```

**`mode` semantics:**

| Mode | Meaning |
|---|---|
| `"bundled"` | All Field records referenced by any Type, all Type records referenced by any Type or View, and all View records referenced by any DocumentView are included in their respective arrays. Self-contained. |
| `"standalone"` | Dependencies are expected pre-installed in the consumer's registry. `dependencyRefs` is the required manifest. |

`dependencyRefs` is required in both modes. Consumers use it to validate completeness without parsing content internals.

---

### 5.2 Reference

A stable pointer to a specific definition version.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1
  definitionType?: "field" | "type" | "view" | "schema" | "protocol"
}
```

Canonical string form: `namespace/name@version`

---

### 5.3 Lineage

Upstream and fork tracking for a specific definition version.

```typescript
{
  sourceDefinitionId?: UUID     // UUID of the upstream definition
  sourceVersion?: integer       // upstream version at derivation time
  forkedFromDefinitionId?: UUID // UUID of the definition deliberately forked from
  forkedFromVersion?: integer   // version at the fork point
}
```

| Field pair | Meaning |
|---|---|
| `sourceDefinition*` | Tracked copy; consumer expects upstream updates |
| `forkedFrom*` | Deliberately diverged; no upstream tracking |

Both may be present during a transition from tracking to forking.

---

### 5.4 Provenance

Publisher and package origin metadata.

```typescript
{
  publisher?: string        // namespace or org of the original author
  sourcePackage?: string    // package name that bundled this definition
  packageVersion?: string   // semver of the source package
  importedAt?: ISO8601
}
```

`packageVersion` is distinct from `Field.version`. A package at `1.3.0` may contain `decision_statement@3` and `context@2`.

---

## 6. Conversation Layer

The conversation layer is a permanent architectural boundary distinct from SCDS. It captures raw multimodal source material; SCDS captures negotiated semantic state. They reference each other bidirectionally via `SourceReference` (document → conversation) and `AttentionState` tags (conversation → document, via `ext:addressability`).

```
Conversation layer  →  raw multimodal source material (speech, threads, annotations)
                        elements tagged with Address at production time
Protocol layer      →  structures the facilitation process; advances AttentionState
SCDS layer          →  captures negotiated semantic state; Records carry SourceReferences
Presentation layer  →  renders SCDS state via Views
```

Three conversation types are in scope:

| Type | Structure | Anchoring |
|---|---|---|
| Meeting transcript | Linear, time-ordered chunks | Tagged with AttentionState at production time |
| Threaded conversation | Tree of replies | Thread root anchored to a document element Address |
| Web UI annotations | Attached to content | Anchored to a Field or Record Address |

Transcript chunks referenced in `SourceReference` are source material — addressable evidence. They do not become Notes or Records automatically. A transcript chunk referenced in `sourceRefs` is evidence supporting a field value; it is not itself a Note unless someone deliberately models it as one.

---

## 7. Extensions

Extensions are optional, independently adoptable. Each extension section declares its identifier, dependencies, and the types it defines.

---

### ext:addressability

**Required for**: any implementation with live facilitation or multi-session extraction.

Defines a universal addressing scheme and the mechanisms that connect conversation material to document elements.

#### `Address`

A stable, resolvable identifier for any element across document space, process space, and conversation space.

```typescript
type Address =
  | {
      space: "document"
      containerId: UUID
      recordId?: UUID
      fieldId?: UUID
      revisionId?: UUID    // requires ext:addressability Revision
    }
  | {
      space: "process"
      runId: UUID          // Protocol run ID; requires ext:protocol
      stageId?: string
    }
  | {
      space: "conversation"
      sessionId: UUID
      chunkId?: UUID
      annotationId?: UUID
    }
```

Every element that can be referred to has an Address. A transcript chunk and a field Revision are co-addressable because assertions about one referencing the other require both to be resolvable.

#### `AttentionState`

The current focus of an active Protocol run — a live cursor across the address space. `AttentionState` and `Address` are structurally related but serve distinct roles: an `Address` is a stable, resolvable identifier for a specific element; `AttentionState` is the mutable cursor that records *where focus currently is* during an active session. An `AttentionState` value at a point in time resolves to a document-space `Address`, but it is stored separately because it changes continuously as the Protocol advances.

Conversation material is tagged with the active `AttentionState` as it is produced. This makes context assembly efficient: "all chunks produced while focus was on this Field" is a queryable address predicate.

```typescript
{
  containerId: UUID
  recordId?: UUID
  fieldId?: UUID
  protocolRunId?: UUID
  stageId?: string
}
```

`AttentionState` is set live by the session or Protocol runner. `SourceReference` is set retrospectively at extraction or editorial review time. Both are needed; they answer different questions.

#### `Revision`

A first-class, addressable snapshot of a `FieldValue` at a point in time. Carries the value, the agent, a timestamp, and source references to the conversation that produced the change.

```typescript
{
  revisionId: UUID
  fieldId: UUID
  recordId: UUID

  value: FieldValue
  agent: "human" | "ai" | "imported"
  createdAt: ISO8601

  sourceRefs?: SourceReference[]
  priorRevisionId?: UUID  // chain to the previous Revision for this field
}
```

Revision does not replace the edit-in-place vs. new-Record judgment. Minor corrections remain in-place edits at the implementation layer. Revision is the addressable audit trail for interoperability — it makes field history queryable: "what did this field say before the last Protocol run?", "which conversation produced the change from revision 2 to revision 3?"

#### Context Query (behavioural requirement)

A conforming `ext:addressability` implementation must be able to assemble relevant material given an address and a purpose. This is a behavioural requirement, not a data shape.

**Required query patterns:**

| Pattern | Address | Returns |
|---|---|---|
| Field context | `{recordId}/{fieldId}` | Current value, Revision history, chunks tagged to this Field, Field `aiGuidance` |
| Record context | `{recordId}` | All field values, chunks tagged to this Record, Relations, Protocol run history |
| Stage context | `{runId}/{stageId}` | All chunks produced during this stage, Fields active in this stage |
| Revision trace | `{fieldId}/{revisionId}` | Value at that Revision, the conversation that produced it, prior Revision chain |

**Recommended assembly order for AI assistance:**

1. Type and Field `aiGuidance` — what this field captures, how to extract it
2. Current value and recent Revision history — what has already been established
3. Chunks tagged to this Field via AttentionState — most focused context
4. Chunks tagged to the parent Record — broader session context
5. Related Records via Relations — structural context

---

### ext:lifecycle

**Required for**: governance tools, decision logs, any implementation where records progress through defined states.

Adds lifecycle state declarations to `Type` and lifecycle state tracking to `Record`.

#### `LifecycleState`

```typescript
{
  name: string
  description?: string
  isInitial?: boolean   // valid starting state for new Records
  isFinal?: boolean     // no transitions out; Record is settled
}
```

#### `LifecycleTransition`

```typescript
{
  name: string       // e.g. "promote", "approve", "supersede"
  from: string       // must match a state name in the enclosing lifecycle
  to: string
  description?: string
}
```

#### Type lifecycle block (added by this extension)

When `ext:lifecycle` is in use, `Type` gains:

```typescript
lifecycle?: {
  states: LifecycleState[]           // min 1 state
  transitions: LifecycleTransition[]
  initialState: string               // must reference a state name where isInitial === true
}
```

#### Record lifecycle state (added by this extension)

`Record.lifecycleState` becomes meaningful: must match a state name in the associated `Type.lifecycle.states[]` when the Type declares a lifecycle.

The `lifecycle` block declares vocabulary. Implementations decide enforcement strictness. A state with `isFinal: true` signals that no further transitions are expected; implementations may use this to lock Record content.

---

### ext:protocol

**Required for**: facilitation tools, structured deliberation, any implementation that guides users through epistemic stages.

Replaces `TemplateFacilitationStep` from v1. Protocol is epistemically richer: stages have explicit dependencies, completion criteria, and may produce intermediate Records.

#### `TypeRef`

A reference to a specific Type, used within Protocol and Schema.

```typescript
{
  typeId: UUID
  typeVersion?: integer
}
```

#### `FieldRef`

A reference to a Field within a Type.

```typescript
{
  fieldId: UUID
  typeId?: UUID    // which Type this Field appears in
}
```

#### `ProtocolStage`

A named stage in a Protocol. Stages have epistemic dependencies (`dependsOn`) — not just ordering. A stage may only proceed when its dependencies are sufficient.

```typescript
{
  stageId: string       // stable key within this Protocol
  order: integer        // min: 0; display/presentation order only — see note below
  purpose: string       // what understanding this stage builds
  question: string      // the core question this stage answers
  dependsOn: string[]   // stageId values; epistemic dependencies, not just ordering
  completionCriteria: string   // how to know this stage is sufficient to proceed
  contributesTo: FieldRef[]    // which Record Fields this stage feeds
  outputType?: TypeRef         // if this stage produces its own intermediate Record
  aiGuidance: AiGuidance
}
```

**`order` vs `dependsOn`:** `order` is the display and presentation sequence — how stages are shown in a UI or facilitation guide. Execution sequence is determined by `dependsOn` resolution: a stage runs when all its declared dependencies are satisfied, regardless of its `order` value. Authors must ensure `order` is consistent with the partial order implied by `dependsOn` (i.e. a stage's `order` value should be greater than the `order` of any stage it depends on). See Invariant 31.

#### `Protocol`

An epistemically ordered process for building quality Records through structured conversation or facilitation.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string

  targetType?: TypeRef
  // The Record type this Protocol produces. Absent for loose / exploratory Protocols
  // (Brain Dump, Decomposition) whose output is input context for a tighter Protocol.

  stages: ProtocolStage[]

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

**The Protocol spectrum:**

```
Loose                                                    Tight
─────────────────────────────────────────────────────────────
Brain Dump → Decomposition → Options Analysis → Decision
```

Loose Protocols produce open material. Tight Protocols converge on a specific Record type. The output of a loose Protocol is the input context for something tighter.

**Generic Protocols** (reusable across domains):
- Brain Dump — externalise all thinking without constraint
- Decomposition — identify major components from raw material
- Review — what is established, what is still open
- Prioritisation — which components to resolve first

**Domain-specific Protocols** (target a specific Record type):
- Decision — context → criteria → options → evaluation → decision
- Proposal — problem → solution shape → constraints → proposal

**Protocol chaining and provenance**: The output of one Protocol is the input context for the next. This derivation chain is traceable through `derived-from` Relations, making the quality and history of the final Record auditable.

**Non-normative example — Protocol chain for a governance decision:**

```
Brain Dump Protocol (loose, no targetType)
  → AttentionState: { containerId: C1 }
  → Produces: Note N1 (unstructured brainstorm)

Decomposition Protocol (loose, targetType: Component)
  → AttentionState: { containerId: C1, recordId: N1 }
  → Produces: Notes N2, N3, N4  [derived-from N1]

Decision Protocol (tight, targetType: Decision)
  → AttentionState: { containerId: C1, protocolRunId: R1, stageId: "criteria" }
  → Stage "criteria" produces: Options Analysis Record R-OA  [derived-from N2, N3]
  → Stage "decision" produces: Decision Record R-D           [derived-from R-OA]

Conversation chunks produced during Decision stage:
  chunk-42: { AttentionState: { containerId: C1, recordId: R-OA, fieldId: F-criteria, ... } }
  chunk-43: { AttentionState: { containerId: C1, recordId: R-D, fieldId: F-outcome, ... } }

Context query for R-D / F-outcome:
  → Field aiGuidance from Decision Type + outcome Field
  → Current value + Revision history for F-outcome
  → Chunks tagged with { recordId: R-D, fieldId: F-outcome } — chunk-43
  → Chunks tagged with { recordId: R-D } — broader session context
  → Related Records via Relations — R-OA via derived-from
```

The final Decision Record is auditable because every Protocol stage left addressable artefacts. The quality of the outcome is traceable to the conversation that produced it.

Views (`ext:views-l1`) no longer contain facilitation logic. A View is a presentation concern; a Protocol is an epistemic one.

---

### ext:schema

**Required for**: extraction pipelines, founding document workflows, any system that needs to specify what a document type IS before assembling it.

#### `RelationSpec`

Declares an expected Relation between two Record types within a Schema.

```typescript
{
  relationType: string
  sourceType: TypeRef
  targetType: TypeRef
  cardinality?: "one-to-one" | "one-to-many" | "many-to-many"
  required?: boolean
}
```

#### `Schema`

The definition of a complete document type — which Types it contains, what Relations exist between resulting Records, and what "complete" means. A Schema is the artefact handed to an extraction pipeline.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string

  rootTypes: TypeRef[]        // Types to extract
  structure: RelationSpec[]   // expected Relations between extracted Records
  requiredTypes: TypeRef[]    // what "complete" means for this document type

  aiGuidance?: AiGuidance
  // purpose: what kind of document this Schema defines
  // extraction: framing for extraction pipelines

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

**Schema vs View:**

| | Schema | View / Document View |
|---|---|---|
| Question it answers | What IS this document type? What should be extracted? | How are existing Records assembled into readable output? |
| Operates at | Definition time | Projection time |
| Input | Source material (transcripts, conversations) | Existing Records in a Container |
| Output | Extraction instructions → Records | Rendered document |

---

### ext:type-inheritance

**Required for**: Type libraries that need formal specialization while preserving base-Type processability.

Defines single inheritance for Types. A specializing Type inherits the fields and semantics of a base Type, may add fields, and remains processable as the base Type by systems that know the base Type but not the specialization.

When `ext:type-inheritance` is in use, `Type` gains:

```typescript
{
  extendsTypeId?: UUID
  // UUID of the base Type this Type specializes.
  // When present, the effective field list consists of inherited fields
  // followed by this Type's own fields[], unless fieldOrder is present.

  extendsTypeVersion?: integer
  // Version of the base Type targeted by this specialization.

  fieldOrder?: UUID[]
  // Optional explicit ordering of all fields in the effective field list:
  // inherited fields plus this Type's own fields[].
  // This is an ordering declaration only; it does not re-declare field
  // assignments or change Field semantics.

  fieldAssignmentOverrides?: FieldAssignmentOverride[]
  // Presentation and workflow overrides for inherited fields only.
}
```

#### `FieldAssignmentOverride`

Overrides presentation or workflow constraints for an inherited Field in a specializing Type. It does not change the Field's semantics.

```typescript
{
  fieldId: UUID
  displayLabel?: string
  displayHint?: string
  required?: boolean
}
```

`displayLabel` and `displayHint` are presentation-only. `required` may tighten an inherited optional field (`false` to `true`) for the specializing Type. It must not relax an inherited required field (`true` to `false`), because a Record instantiated against the specializing Type must remain valid when processed as the base Type.

The effective field list for a specializing Type is the inherited effective field list of its base Type plus the specializing Type's own `fields[]`. A specializing Type must not duplicate an inherited `fieldId` in its own `fields[]`.

Example:

```text
Type: core/decision
  fields: decision_statement, context, rationale, options_considered

Type: org.example/governance_decision
  extendsTypeId: core/decision
  adds: ratification_method, quorum_threshold, voting_record
```

A system that knows `core/decision` but not `org.example/governance_decision` can still read the inherited decision fields. The specializing fields are unknown extension content to that system and should be preserved rather than discarded.

---

### ext:views-l1

**Required for**: rendering and export workflows.

Defines Views — versioned presentations of a single Record through a specific Type.

#### `FieldView`

A field reference within a View. Controls presentation for this View without altering field semantics.

```typescript
{
  fieldId: UUID       // must appear in the Type's effective field list
  order: integer      // min: 0; display order within this View
  required?: boolean  // View-level workflow constraint; does not alter Field contract
  visible?: boolean   // default: true

  // Presentation overrides — View scope only
  displayLabel?: string
  displayHint?: string
  editorHintOverride?: string
}
```

A Field hidden with `visible: false` remains in the Record and may appear in other Views.

#### `ExportConfig`

Configuration for rendering a Record through this View as an exportable document.

```typescript
{
  format?: string        // target format hint, e.g. "markdown", "adoc", "json"
  preamble?: string
  // Template string rendered before field values.
  // Variable substitution uses {{variable-name}} syntax.
  // Standard variables: {{instance-id}}, {{date}}, {{status}}, {{namespace}}, {{name}}

  fieldOrder?: UUID[]    // explicit export field ordering; defaults to fieldViews[].order
  omitEmptyFields?: boolean  // default: false
}
```

#### `View`

A versioned presentation and export configuration over a specific Type. Multiple Views may exist for the same Type, serving different audiences or purposes.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string    // when to use this View; what workflow or audience it serves

  typeId: UUID           // references Type.id
  typeVersion: integer

  aiGuidance?: AiGuidance
  // purpose: the workflow context this View serves
  // extraction: session-level framing injected before field extraction

  fieldViews: FieldView[]

  protection?: "none" | "read-only" | "fill-in"
  // Default: "none".
  // "read-only" — Records rendered through this View cannot be edited.
  // "fill-in"   — only null or empty Field values may be populated.
  // Protection is a View-level workflow constraint. It does not modify
  // the Record or replace lifecycle states.

  exportConfig?: ExportConfig

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

A View may not introduce Fields that are not in the bound Type's effective field list. Omitted Fields are treated as `visible: false`.

`View.protection` applies only to interactions through that View. A Record may be editable through one View and read-only through another. For record-level settlement, use `ext:lifecycle` states such as `isFinal`.

Facilitation steps have been removed from View. Use `ext:protocol` Protocol stages instead.

---

### ext:views-l2

**Depends on**: `ext:views-l1`

**Required for**: document projection — assembling multiple Records into a coherent document.

#### `SectionSource`

Defines how a section's instances are selected from a Container.

```typescript
type SectionSource =
  | {
      type: "fixed-instances"
      instanceIds: UUID[]
      // Explicit list. For preamble, cover page, or curated sections.
    }
  | {
      type: "type-query"
      semanticObjectType: string
      // For cross-system portability, use namespace/name format (e.g. "core/decision").
      // A bare string like "decision" is a single-system convention.
      lifecycleState?: string
      containerIds?: UUID[]
    }
  | {
      type: "relation-query"
      fromInstanceId: UUID
      relationType: string
      direction?: "forward" | "inverse"  // default: "forward"
    }
  | {
      type: "container-subset"
      containerId: UUID
      containerType?: string
    }
```

#### `DocumentSection`

One section in a Document View.

```typescript
{
  sectionId: string
  title?: string
  description?: string
  order: integer   // min: 0

  source: SectionSource

  renderViewId?: UUID    // View (ext:views-l1) used to render each instance in this section
  // When absent, implementations use a default rendering for the instance type.

  ordering?: {
    fieldId?: UUID
    direction?: "asc" | "desc"  // default: "asc"
  }

  required?: boolean
  emptyBehavior?: "hide" | "show-placeholder"
}
```

#### `NavigationLink`

An assembly-time cross-section link in a Document View. Navigation links are reading aids for the rendered document, not semantic assertions about Records. They do not appear in the Relation graph.

```typescript
{
  fromSectionId: string
  toSectionId: string
  label?: string
  bidirectional?: boolean  // default: false
}
```

#### `DocumentView`

A versioned, Container-level projection. Defines how a Container's Records are assembled into a readable document.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string    // what kind of document this produces; intended audience

  containerType?: string  // when set, intended for Containers of this type

  sections: DocumentSection[]

  navigationLinks?: NavigationLink[]

  preamble?: string
  // Template string rendered before all sections.
  // Standard variables: {{container-title}}, {{date}}, {{container-id}}

  format?: string   // e.g. "markdown", "adoc", "html"

  aiGuidance?: AiGuidance
  // purpose: what kind of document this View produces
  // extraction: context for AI-assisted document-level tasks

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

A `DocumentView` may reference multiple `View` records (via `DocumentSection.renderViewId`) — one per instance type in the document. It orchestrates; it does not replace them.

`DocumentSection.renderViewId` references a `View.id` (from `ext:views-l1`). A `DocumentView.id` is not a valid value for `renderViewId` — Document Views are not nestable.

Use `navigationLinks` when a rendered document should include "see also" or related-section links. Use `Relation` only when the relationship is a semantic assertion about Records.

---

### ext:repeatable-fields

**Required for**: any Record type that needs lists of values within a single Field.

Adds repeatability to `FieldAssignment` and defines `FieldValueEntry`.

#### `FieldValueEntry`

A single entry in a repeatable field.

```typescript
{
  value: string | number | boolean | string[] | null
  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
}
```

#### FieldAssignment additions

When `ext:repeatable-fields` is in use, `FieldAssignment` gains:

```typescript
repeatable?: boolean  // default: false; when true, multiple values are allowed
minItems?: integer    // meaningful only when repeatable === true
maxItems?: integer    // meaningful only when repeatable === true
```

And `FieldValue.entries` becomes active: use `entries` when `repeatable === true`, `value` otherwise.

A repeatable field entry does not create a new semantic instance. Use separate Records connected by Relations when repeated items need their own identity, lifecycle, or graph position.

---

### ext:field-groups

**Required for**: Record types where multiple Fields are semantically paired and repeat together as a unit.

Use when parallel `multiselect` arrays would lose pairing (e.g. a contact record with `name` + `email`). Preserves internal pairing across repeated items.

#### `FieldGroup`

A named, ordered group of Fields that repeat together as a unit within a Type.

```typescript
{
  groupId: string        // stable key within the Type
  label?: string
  description?: string

  order: integer         // min: 0; position relative to other Fields and Groups

  required?: boolean     // default: false
  repeatable?: boolean   // default: false
  minItems?: integer
  maxItems?: integer

  fields: FieldAssignment[]
}
```

#### `FieldGroupEntry`

One entry in a repeatable Field Group.

```typescript
{
  entryId?: UUID         // stable key for this entry; allows referencing or updating
  fieldValues: FieldValue[]
}
```

#### `FieldGroupValue`

The current value of a Field Group within a Record.

```typescript
{
  groupId: string           // references FieldGroup.groupId in the Type definition
  entries: FieldGroupEntry[]
}
```

A `FieldGroup` does not create a new semantic instance. Its entries are embedded structured context within the enclosing Record. Use separate Records connected by Relations when group entries need their own identity, lifecycle, provenance, or reuse across Records.

`FieldGroup.repeatable`, `minItems`, and `maxItems` define group-level repeatability — whether the group as a whole can appear multiple times within a Record. This is structurally independent from `ext:repeatable-fields`, which adds scalar repeatability to individual Fields. An implementation may adopt `ext:field-groups` without `ext:repeatable-fields`; the repeatability mechanics in each extension are self-contained.

When `ext:field-groups` is in use, `Type` gains `fieldGroups?: FieldGroup[]` and `Record` gains `groupValues?: FieldGroupValue[]`.

**Repeatability pattern guide:**

| Pattern | Use | Example |
|---|---|---|
| Repeatable scalar | `FieldAssignment.repeatable` (ext:repeatable-fields) | Multiple assigned person names |
| Repeatable structured context | `FieldGroup` | Contacts with name + email pairs |
| Repeated semantic objects | Separate Records + Relations | Tasks assigned to roles |

---

### ext:cross-field-validation

**Required for**: Types with constraints that span multiple Fields.

`ValidationRule` handles single-field constraints. `CrossFieldRule` handles constraints that require evaluating more than one Field together.

#### `CrossFieldRule`

```typescript
{
  type: "conditional-required" | "field-ordering" | "mutual-exclusion"
  message?: string

  // conditional-required: targetFieldId becomes required when predicateFieldId equals predicateValue
  predicateFieldId?: UUID
  predicateValue?: string
  targetFieldId?: UUID

  // field-ordering: targetFieldId must precede or follow predicateFieldId
  // Applies only to fields with valueType "date" or "number".
  effect?: "must-precede" | "must-follow"

  // mutual-exclusion: at most one of the listed fields may have a non-empty value
  fieldIds?: UUID[]   // min: 2
}
```

| Rule type | Required fields |
|---|---|
| `conditional-required` | `predicateFieldId`, `predicateValue`, `targetFieldId` |
| `field-ordering` | `predicateFieldId`, `targetFieldId`, `effect` |
| `mutual-exclusion` | `fieldIds` (min 2) |

When `ext:cross-field-validation` is in use, `Type` gains `validationRules?: CrossFieldRule[]`.

---

### ext:recommended-relations

**Required for**: cross-system federation; multi-publisher ecosystems where Relation type semantics must be interoperable.

Canonical relation types and machine-readable Relation type definitions.

**Canonical relation types** (use exact strings):

| Canonical | Converse | Category |
|---|---|---|
| `contains` | `part-of` | Composition |
| `depends-on` | `required-by` | Dependency |
| `supersedes` | `superseded-by` | Governance |
| `refines` | `refined-by` | Refinement |
| `derived-from` | `source-of` | Derivation |
| `evidences` | `evidenced-by` | Evidence |
| `precedes` | `follows` | Sequence |

Implementations must store only the canonical (forward) form and derive the inverse when needed.

**Relation category taxonomy:**

| Category | Examples |
|---|---|
| Composition | `contains`, `part-of`, `has-section` |
| Refinement | `refines`, `expands`, `summarises` |
| Dependency | `depends-on`, `requires`, `blocks`, `enables` |
| Sequence | `precedes`, `follows`, `overlaps` |
| Derivation | `derived-from`, `extracted-from`, `based-on` |
| Evidence | `evidences`, `supports`, `contradicts` |
| Governance | `supersedes`, `amends`, `ratifies`, `delegates` |
| Association | `relates-to`, `links-to` |

#### `RelationTypeDefinition`

Machine-readable metadata for a `relationType` string.

```typescript
{
  relationType: string      // exact string used in Relation.relationType
  namespace: string
  label?: string
  description?: string
  category?: "composition" | "refinement" | "dependency" | "sequence" | "derivation" | "evidence" | "governance" | "association"
  canonicalDirection?: string   // e.g. "source is the dependent task; target is the task it depends on"
  inverseType?: string
}
```

`RelationTypeDefinition` is optional metadata. Implementations are not required to resolve `relationType` strings against a definition before accepting a Relation. Relation type definitions may be included in a Package or published separately.

---

### ext:import-tracking

**Required for**: implementations that receive packages from upstream publishers and need to track update and conflict state.

#### `ImportMode`

```typescript
"upstream-tracked" | "local-copy" | "local-fork"
```

| Mode | Meaning |
|---|---|
| `"upstream-tracked"` | Consumer expects updates from the source Package. Conflicts surfaced when local and upstream diverge. |
| `"local-copy"` | Imported as a snapshot. No update tracking. |
| `"local-fork"` | Deliberately diverged. Upstream lineage preserved for reference. |

#### `ImportRecord`

One record per imported definition in a consumer's local registry.

```typescript
{
  definitionId: UUID
  definitionType: "field" | "type" | "view" | "schema" | "protocol"
  namespace: string
  name: string
  version: integer

  mode: ImportMode
  importedAt: ISO8601

  sourcePackageId: UUID
  sourcePackageName: string
  sourcePackageVersion: string

  latestKnownUpstreamVersion?: integer
  updateAvailable?: boolean
  updateCheckedAt?: ISO8601

  conflictState?: "clean" | "local-ahead" | "upstream-ahead" | "diverged"
  conflictDetectedAt?: ISO8601

  localVersion?: integer
  localEditedAt?: ISO8601
}
```

#### `ImportSummary`

A consumer's complete picture of its imported definitions.

```typescript
{
  generatedAt: ISO8601
  fields: ImportRecord[]
  types: ImportRecord[]
  views: ImportRecord[]
  schemas: ImportRecord[]
  protocols: ImportRecord[]
}
```

---

### ext:registry

**Required for**: multi-publisher ecosystems; discoverable definition catalogs.

#### `RegistryEntry`

One entry in a Registry catalog.

```typescript
{
  packageId: UUID
  packageName: string
  packageVersion: string
  publisher: string
  description?: string
  publishedAt: ISO8601
  homepage?: string
  tags?: string[]
  fieldCount: integer       // min: 0
  typeCount: integer        // min: 0
  viewCount?: integer
  schemaCount?: integer
  protocolCount?: integer
  relationTypeCount?: integer
  downloadUrl?: string
  checksum?: string         // SHA-256 hex digest for integrity verification
}
```

#### `Registry`

A registry's published index.

```typescript
{
  schemaVersion: string
  registryId: UUID
  registryName: string
  catalogVersion: string    // registry's own version (semver)
  updatedAt: ISO8601
  homepage?: string
  entries: RegistryEntry[]
}
```

Multiple Registries may coexist. A consumer may index multiple catalogs. The specification does not define registry authority, authentication, or federation.

---

## 8. Key Invariants

Conforming implementations must uphold the following invariants.

### Field semantics

**1.** `FieldAssignment.displayLabel` and `FieldAssignment.displayHint` are for rendering only. They must not affect AI guidance, extraction logic, `valueType` interpretation, or validation.

**2.** A `Type` must not redefine, override, or duplicate the semantic content of any `Field` it includes. If different semantics are needed for a Field in a specific Type context, a distinct `Field` with its own identity and lineage must be created.

**3.** A `Field`'s `aiGuidance` belongs to the Field. Type-level `aiGuidance` provides session framing only.

### Lifecycle (ext:lifecycle)

**4.** `Type.lifecycle.initialState` must reference a `name` that appears in `lifecycle.states[]` and where `isInitial === true`.

**5.** Every `from` and `to` value in `lifecycle.transitions[]` must reference a `name` that appears in `lifecycle.states[]`.

**6.** `Record.lifecycleState`, when present, must reference a `name` in the associated `Type.lifecycle.states[]`.

### Distribution

**7.** Every `fieldId` referenced in any `FieldAssignment` within a `Package.types[]` must appear as the `id` of an entry in `Package.dependencyRefs`.

**8.** If `Package.mode === "bundled"`: every `Reference` in `dependencyRefs` must have a matching `Field` in `fields[]` (matched on `id` and `version`).

**9.** `Field.id` is stable across versions. A new `id` means a new definition, not a new version of an existing one.

### Cross-field validation (ext:cross-field-validation)

**10.** All `fieldId` values in any `CrossFieldRule` within `Type.validationRules[]` must appear in the Type's effective field list. Cross-field rules cannot reference Fields outside the Type.

**11.** A `conditional-required` rule must supply `predicateFieldId`, `predicateValue`, and `targetFieldId`. A `field-ordering` rule must supply `predicateFieldId`, `targetFieldId`, and `effect`. A `mutual-exclusion` rule must supply `fieldIds` with at least two entries.

### Views (ext:views-l1)

**12.** Every `fieldId` in `View.fieldViews[]` must appear in the bound Type's effective field list. A View cannot introduce Fields not in its Type.

**13.** `FieldView.displayLabel`, `FieldView.displayHint`, and `FieldView.editorHintOverride` are for rendering only. They must not affect AI guidance, extraction logic, `valueType` interpretation, or validation.

**14.** A `View` must not override, redefine, or duplicate the semantic content of any `Field` or `Type` it references. View-level `aiGuidance` is workflow framing; it does not redefine Field extraction semantics.

### Distribution — Views (ext:views-l1)

**15.** Every `typeId` referenced by any `View` in `Package.views[]` must appear in `Package.dependencyRefs` with `definitionType: "type"`. If `mode === "bundled"`, that `Type` must be present in `types[]`.

### Relations

**16.** In a `Relation`, `sourceInstanceId` is the asserting instance and `targetInstanceId` is the related instance. The Relation reads: "source [relationType] target." This convention must not be reversed.

**17.** `Relation` is reserved for assertions that carry semantic consequence beyond simple mention or citation. Lightweight prose references that do not assert structural, causal, or governance relationships must not be modelled as `Relation` records.

### Notes and Typed Records

**18.** `NoteSection.name` values must be unique within a `Note`.

**19.** `TypedField.name` values must be unique within a `Typed Record`.

### Containers

**20.** `Container.containerId` is not an instance ID. It must not appear in `Container.rootInstanceIds`, `Container.memberInstanceIds`, `Relation.sourceInstanceId`, or `Relation.targetInstanceId`.

**21.** `Container.rootInstanceIds` and `Container.memberInstanceIds`, when present, must reference valid SCDS instance IDs (`Note.instanceId`, `Typed Record.instanceId`, or `Record.instanceId`).

### Repeatability (ext:repeatable-fields)

**22.** If `FieldAssignment.repeatable` is false or absent, its corresponding `FieldValue` must use `value` and must not include `entries`.

**23.** If `FieldAssignment.repeatable` is true, its corresponding `FieldValue` may use `entries`. If `minItems` is specified, `entries` must contain at least that many items. If `maxItems` is specified, `entries` must not exceed that count. For repeatable fields, `Field.validationRules` are evaluated against each `FieldValueEntry.value` individually, not against the array as a whole.

**24.** `FieldAssignment.minItems` and `maxItems` are valid only when `repeatable === true`. They must be ignored when `repeatable` is false or absent.

### Field groups (ext:field-groups)

**25.** Every `groupId` in `Record.groupValues[]` must reference a `groupId` declared in the associated `Type.fieldGroups[]`.

**26.** Within a `FieldGroupEntry.fieldValues[]`, every `fieldId` must appear in the enclosing `FieldGroup.fields[].fieldId`.

**27.** A `FieldGroupValue.entries` list must satisfy `FieldGroup.minItems` and `maxItems` where specified.

### Records

**28.** `Record.typeId` and `Record.typeVersion` are the authoritative Type binding. `typeNamespace` and `typeName` are denormalised convenience fields. If they conflict with the resolved `Type`, the `typeId`/`typeVersion` identity takes precedence and the Record is considered invalid until corrected.

### Protocol (ext:protocol)

**29.** Every `stageId` in `ProtocolStage.dependsOn[]` must reference a `stageId` declared in the enclosing `Protocol.stages[]`. A stage may not declare a dependency on itself.

**30.** Every `fieldId` in `ProtocolStage.contributesTo[]` must reference a `fieldId` that appears in the stage's own `outputType`'s effective field list (when `outputType` is declared), or in `Protocol.targetType`'s effective field list (when `outputType` is absent). A single stage must not contribute to both its own `outputType` and the enclosing `Protocol.targetType`. When neither `outputType` nor `Protocol.targetType` is declared, `contributesTo` must be empty.

**31.** For every pair of stages A and B within a `Protocol` where B.dependsOn includes A.stageId, B.order must be greater than A.order. `order` is display order; execution sequence is determined by `dependsOn` resolution. The two must not contradict each other.

### Views L2 (ext:views-l2)

**32.** Any `DocumentView` in `Package.documentViews[]` that contains a `SectionSource` with `type === "type-query"` must use `namespace/name` format for `semanticObjectType` (e.g. `"core/decision"`, not `"decision"`). Bare strings are acceptable only in single-system `DocumentView` records not included in a Package. Implementations receiving a `DocumentView` from a Package with a bare `semanticObjectType` in a `type-query` section should treat the portability of that section as undefined.

### Addressability (ext:addressability)

**33.** `Revision.priorRevisionId`, when present, must reference a `Revision.revisionId` for the same `fieldId` and `recordId`. Revision chains must be acyclic.

**34.** `AttentionState.containerId` must reference a valid `Container.containerId`. Other Address components (`recordId`, `fieldId`, `protocolRunId`, `stageId`) are optional and may be absent when focus has not yet narrowed.

### Distribution — Views L2 (ext:views-l2)

**35.** Every `DocumentSection.renderViewId` in any `DocumentView` within `Package.documentViews[]` must reference a `View.id` that appears in `Package.views[]` or `Package.dependencyRefs`. If `mode === "bundled"`, that `View` must be present in `Package.views[]`.

### Distribution — Schema (ext:schema)

**36.** Every `TypeRef.typeId` referenced in any `Schema.rootTypes[]`, `Schema.requiredTypes[]`, or in any `RelationSpec.sourceType` or `RelationSpec.targetType` within `Schema.structure[]`, for each Schema in `Package.schemas[]`, must appear in `Package.dependencyRefs` with `definitionType: "type"`. If `mode === "bundled"`, each such Type must be present in `Package.types[]`.

### Distribution — Protocol (ext:protocol)

**37.** Every `TypeRef.typeId` referenced in `Protocol.targetType` or in any `ProtocolStage.outputType`, for each Protocol in `Package.protocols[]`, must appear in `Package.dependencyRefs` with `definitionType: "type"`. Every `FieldRef.fieldId` in any `ProtocolStage.contributesTo[]` must appear in `Package.dependencyRefs` with `definitionType: "field"`. If `mode === "bundled"`, those Types must be in `Package.types[]` and those Fields in `Package.fields[]`.

### Field semantics — content format

**38.** `Field.contentFormat`, when present, is only meaningful when `valueType` is `"string"` or `"text"`. Implementations must ignore `contentFormat` on fields with any other `valueType`.

### Type inheritance (ext:type-inheritance)

**39.** `Type.extendsTypeId`, when present, must reference a valid `Type.id`. Inheritance chains must be acyclic; a Type may not directly or transitively extend itself.

**40.** A specializing Type must not declare a `fieldId` in its own `fields[]` that duplicates any `fieldId` inherited from its base Type or any ancestor Type.

**41.** When `Type.fieldOrder` is present, it must contain exactly the set of field UUIDs in the Type's effective field list. No UUID may appear more than once, and no UUID from the effective field list may be absent.

**42.** Every `fieldId` in `Type.fieldAssignmentOverrides[]` must reference a field inherited from the base Type or an ancestor Type. Overrides must not reference fields declared in the specializing Type's own `fields[]`, must not alter Field semantics, and must not relax an inherited required field from `true` to `false`.

**43.** When `ext:type-inheritance` is declared, `Package.dependencyRefs` must include a `Reference` for every Type in the transitive closure of base Types for any Type in `Package.types[]`. If `mode === "bundled"`, all such base Types must be present in `types[]`.

### Views L2 navigation (ext:views-l2)

**44.** Every `NavigationLink.fromSectionId` and `NavigationLink.toSectionId` must reference a `sectionId` declared in the enclosing `DocumentView.sections[]`.

---

## 8.5 Extension Interactions

Cross-extension interactions are behavioural requirements that apply only when an implementation declares both named extensions.

### ext:protocol × ext:addressability

**Trigger**: an implementation declares both `ext:protocol` and `ext:addressability`.

**Required behaviour**: Protocol stage advancement updates `AttentionState`. When a Protocol run advances from one stage to another, the active `AttentionState` must reflect the new stage before any conversation material is tagged.

Specifically:

- `AttentionState.protocolRunId` references the active Protocol run
- `AttentionState.stageId` reflects the current stage
- `AttentionState.fieldId`, when a specific field is the current focus within a stage, is set accordingly

Conversation chunks produced while `AttentionState.stageId` is set are associated with that stage. This makes stage-level Context Queries (`{runId}/{stageId}`) return the correct material.

---

## 9. Conformance

An implementation declares conformance using the following form:

```
SCDS <version> Core [+ ext:<name> ...]
```

Example:
```
SCDS 2.0 Core + ext:lifecycle + ext:protocol + ext:views-l1 + ext:addressability + ext:recommended-relations
```

### Core conformance requirements

A core-conformant implementation must:
- Accept and validate `Field`, `Type`, `Record` (Tier 2), `Relation`, and `Container` inputs against this specification
- Enforce Invariants 1–3, 7–9, 16–21, 28, 38
- Support the Foundation and Distribution groups in full
- Implement the namespace format and reference format correctly
- Not accept `relationType` strings that include `/` except in `namespace/name` format

Support for `Note` (Tier 0) and `Typed Record` (Tier 1) is optional at core conformance level.

### Extension conformance requirements

An implementation declaring a given extension must:
- Accept and validate all types defined by that extension
- Enforce all invariants assigned to that extension
- Respect the declared dependency chain (e.g., `ext:views-l2` requires `ext:views-l1` to also be declared)

### Interoperability note

Two implementations at the same conformance level will produce compatible definitions for exchange. An implementation receiving a Package that includes types or fields from an extension it does not support should surface the unknown content, preserve it where possible, and pass it through rather than silently discard it.

---

# claude response

Good evolution. Let me give a precise read against what was proposed.

**What's incorporated cleanly**

`ext:type-inheritance` is in. `fieldOrder` and `fieldAssignmentOverrides` replaced `overrideFieldOrder` correctly. Distribution invariants 35–37 close the DocumentView/Schema/Protocol bundling gaps that were flagged. `documentViews` is now in Package with the bundled mode semantics updated to cover the DocumentView → View closure. NavigationLink is in ext:views-l2. Section 8.5 exists. Invariant 30 is significantly improved — the `outputType` vs `targetType` split is now explicit and the vacuous case (neither declared) is handled. Invariant 10 and 23 have been tightened. Field Groups independence from ext:repeatable-fields is explicitly documented.

**One substantive correction: the spec improved on the proposal**

The adoption proposal had `FieldAssignmentOverride.required` backwards. The proposal said:

> relaxation is permitted (set false where base is true); tightening is a conformance error

The spec correctly reverses this:

> `required` may tighten an inherited optional field (`false` to `true`). It must not relax an inherited required field (`true` to `false`), because a Record instantiated against the specializing Type must remain valid when processed as the base Type.

The spec's version is correct for DITA-style graceful degradation. If a specialization makes a required base field optional, a specialization Record could be created without that field — which would then fail validation when a base-Type-aware system processes it. Tightening is safe (the specialization adds a constraint the base didn't have; base-Type systems ignore it). Relaxation is not safe (the specialization removes a constraint; base-Type systems depend on it). Invariant 42 states this correctly. The adoption proposal should be updated to match.

**What's intentionally deferred**

`ext:annotations` is not yet in the spec. This cascades correctly — `View.protection "comments-only"` is absent (it depends on ext:annotations), the ext:annotations × ext:addressability Section 8.5 entry is absent, and the annotation invariants (39–40 in the proposal) are not present. The spec ends at Invariant 44. This is the right staging — add ext:annotations as a discrete step.

`ext:field-domains` is not yet in the spec. No field transclusion (`{{field:...}}` syntax, `DocumentView.transclusionFallback`). Both sensibly deferred.

**One minor naming collision to track**

`fieldOrder` appears in two places with different meanings: `Type.fieldOrder` (in ext:type-inheritance — ordering declaration for the effective field list) and `ExportConfig.fieldOrder` (in ext:views-l1 — export field ordering for a View). Different contexts, same name. Not a correctness issue, but implementers reading both sections may be momentarily confused. Worth a disambiguation note in the rationale.

**Section 8.5 status**

Currently has only ext:protocol × ext:addressability. The ext:annotations × ext:addressability interaction (context assembly order, annotation query pattern) should be added when ext:annotations lands. The section structure is correct; it just needs the second entry.

**One thing to verify**

Invariant 42 says `fieldAssignmentOverrides` entries "must not reference fields declared in the specializing Type's own `fields[]`." The `fieldOrder` array, however, must reference *all* fields including own fields (Invariant 41). These are consistent — `fieldOrder` is an ordering declaration exempt from Invariant 40's duplication prohibition, while `fieldAssignmentOverrides` specifically targets inherited fields only. The spec states this but the two invariants are close enough together that an implementer validating 40/41/42 together should check that their validator doesn't incorrectly apply the `fields[]` prohibition to `fieldOrder`.

---

# you asked

message time: 2026-05-24 23:20:36

# SCDS Specification

**Version**: 2.0-draft
**Status**: active draft
**Scope**: field definitions (Field), type definitions (Type), records (Note / Typed Record / Record), relations, containers, distribution, and optional extensions covering addressability, lifecycle, protocol, schema, type inheritance, views, repeatable fields, field groups, cross-field validation, recommended relations, import tracking, and registry.

> **Migration note**: This document supersedes `scds-schema.md` (v1.0-draft). A vocabulary and structural mapping from v1 to v2 is in `scds-shema-evolution.md`. Design rationale, usage guidance, and commentary are in `scds-rationale.md`.

---

## 1. Purpose and Scope

### What this specification defines

The Semantic Collaborative Document System (SCDS) specification defines an interoperable standard for semantic field and type definitions, records, relations, and the mechanisms by which these artefacts are created, shared, versioned, and distributed across independent implementations.

This specification covers:

- **Field** — atomic reusable semantic unit
- **Type** — named composition of fields for a specific semantic object type
- **Record** — instantiated type with field values; three semantic maturity tiers (Note, Typed Record, Record)
- **Relation** — first-class typed link between records
- **Container** — grouping boundary for record collections
- **Distribution** — Package, Reference, Lineage, Provenance
- **Extensions** — optional, independently adoptable capabilities declared by conforming implementations

### What this specification does not define

- **Session** — live collaborative process model (future version)
- **Registry protocol** — how registries communicate, authenticate, or federate; this specification defines data shapes only
- **Universal semantic ontology** — domain-specific vocabularies are the responsibility of namespace authors

### Relationship to implementing systems

This specification is implementation-neutral. Implementations are expected to validate inputs against these schemas at their system boundaries. The specification does not constrain persistence technology, API design, UI rendering, or prompt assembly strategy.

### Extension conformance model

Implementations declare conformance as:

```
SCDS Core [+ ext:<name> ...]
```

**Core** requires the Foundation group and Distribution group in full. No extension is required for core conformance. Extensions are independently adoptable; some declare dependencies on other extensions.

| Extension | Identifier | Depends on | Notes |
|---|---|---|---|
| Addressability | `ext:addressability` | — | For live facilitation, declare together with `ext:protocol` |
| Lifecycle | `ext:lifecycle` | — | |
| Protocol | `ext:protocol` | `ext:lifecycle` (recommended) | For live facilitation, declare together with `ext:addressability` |
| Schema | `ext:schema` | — | |
| Type Inheritance | `ext:type-inheritance` | — | |
| Views L1 | `ext:views-l1` | — | |
| Views L2 | `ext:views-l2` | `ext:views-l1` | |
| Repeatable Fields | `ext:repeatable-fields` | — | |
| Field Groups | `ext:field-groups` | — | Group repeatability is self-contained; `ext:repeatable-fields` is not required |
| Cross-Field Validation | `ext:cross-field-validation` | — | |
| Recommended Relations | `ext:recommended-relations` | — | |
| Import Tracking | `ext:import-tracking` | — | |
| Registry | `ext:registry` | — | |

`ext:protocol` and `ext:addressability` are formally independent but are a functional co-dependency for live facilitation: a Protocol without `AttentionState` produces no live conversation tagging; `AttentionState` without Protocol stages has no stage context to capture. Implementations supporting live facilitation should declare both.

Example declaration: `SCDS Core + ext:lifecycle + ext:protocol + ext:views-l1 + ext:addressability`

---

## 2. Namespace Format

### Convention

Namespaces are dot-separated identifiers using lowercase alphanumeric characters and hyphens.

```
<component>[.<component>]*

component = [a-z0-9][a-z0-9-]*
```

Examples:
```
core
community.adr
com.acme.hr
org.cooperative-name
```

### Reserved namespaces

`core` is reserved for definitions maintained by the SCDS standard. Implementations must not allow user-created definitions in the `core` namespace.

### Reference format

A specific version of a definition is referenced using the canonical form:

```
namespace/name@version
```

Examples:
```
core/decision_statement@2
community.adr/review_rationale@1
com.acme.hr/headcount_impact@3
```

The `/` and `@` characters are reserved separators. They must not appear within a namespace component or a name.

### Name convention

Field and Type names are programmatic keys in `snake_case`. Names are stable within a namespace and version lineage. A new name means a new definition.

---

## 3. Schema Notation

Types are described using TypeScript-style notation. Optional fields are marked with `?`. All `UUID` values are RFC 4122 UUID strings. All `ISO8601` values are datetime strings with timezone offset. `integer` means a positive integer unless otherwise noted.

### Version semantics

Version numbers are positive integers scoped to a definition's UUID lineage.

| Change | Version action |
|---|---|
| Documentation, typo, formatting only | Optional bump |
| `description`, `instructions`, or `aiGuidance.purpose` reworded without semantic change | Minor bump recommended |
| `aiGuidance.extraction` or `aiGuidance.purpose` changed in meaning | Version bump required |
| `valueType`, `selectOptions`, or `validationRules` changed | Version bump required |
| `name` changed | New definition required (new UUID) |
| `namespace` changed | New definition required (new UUID) |

When in doubt: if a downstream consumer's AI extraction, validation, or governance logic would behave differently, a version bump is required.

---

## 4. Foundation Group (Core)

The Foundation group is required for all conforming implementations.

### 4.1 Supporting types

#### `ValidationRule`

A constraint applied to a field value.

```typescript
{
  type: "required" | "minLength" | "maxLength" | "pattern" | "enum"
  value?: string | number | string[]  // required for minLength, maxLength, pattern, enum
  message?: string
}
```

#### `AiGuidanceExample`

A single example for AI guidance.

```typescript
{
  description?: string  // labels this example
  input?: string        // sample source text; omit for output-only examples
  output: string        // the ideal value the AI should produce
}
```

`output` is required. An example without `input` demonstrates expected output form without requiring a specific source.

#### `AiGuidance`

Structured AI guidance for a Field or Type.

```typescript
{
  purpose: string            // what this field/type captures (1-2 sentences)
  extraction?: string        // LLM instruction for how to extract or populate
  negativeGuidance?: string  // what the LLM must NOT include or do
  examples?: AiGuidanceExample[]
}
```

The minimum valid `AiGuidance` is `{ purpose: "..." }`.

---

### 4.2 Field

The atomic reusable semantic unit. Fields are defined once and composed into Types. A Field's `aiGuidance`, `validationRules`, and `valueType` belong to the Field, not to any Type that includes it.

```typescript
{
  // Stable identity
  id: UUID
  namespace: string
  name: string       // snake_case programmatic key
  version: integer   // min: 1; increments within this id's lineage

  // Semantic content
  description: string      // one-sentence user-facing summary
  instructions?: string    // fuller guidance for a human completing this field
  aiGuidance: AiGuidance

  // Value semantics — stable across renderers
  valueType: "string" | "text" | "number" | "boolean" | "date" | "url" | "select" | "multiselect"
  selectOptions?: string[]   // required when valueType is "select" or "multiselect"
  validationRules?: ValidationRule[]
  contentFormat?: "plain" | "markdown"
  // Meaningful only when valueType is "string" or "text". Default: "plain".
  // Describes the content of the value, not the editing surface (see editorHint).
  // "plain"    — unformatted prose; renderers must not interpret markup
  // "markdown" — CommonMark subset; renderers should parse and display formatting
  // AI extractors must produce output conforming to this format: a field with
  // contentFormat "markdown" should receive structured markdown from extraction.

  // Editor hint — projection-specific default; implementations and Views may override
  editorHint?: "singleline" | "textarea" | "rich-text" | "date-picker" | "dropdown" | "multi-select" | "voice"

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: Lineage      // see Distribution group
  provenance?: Provenance
}
```

**`valueType` semantics:**

| Value | Meaning |
|---|---|
| `"string"` | Short single-value text (typically one line) |
| `"text"` | Potentially long multi-paragraph prose |
| `"number"` | Numeric value |
| `"boolean"` | True/false |
| `"date"` | ISO 8601 date or datetime |
| `"url"` | A URL string |
| `"select"` | One value from `selectOptions` |
| `"multiselect"` | One or more values from `selectOptions` |

`valueType` is the stable semantic data type. `editorHint` is a rendering default. AI extraction, validation, and export formatting must depend only on `valueType`. `contentFormat` refines how `string` and `text` values should be produced and rendered, but does not alter the `valueType`.

---

### 4.3 Type

A named, versioned composition of Fields for a specific semantic object type.

```typescript
{
  // Stable identity
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  // Content
  description: string        // when to use this Type; what semantic object it defines
  aiGuidance?: AiGuidance    // Type-level LLM framing; see AI guidance composition in rationale

  // Semantic object type (optional, informative)
  semanticObjectType?: string
  // e.g. "decision", "task", "risk", "budget_line", "requirement"
  // Free-form. Implementations may use as a rendering or grouping hint.
  // No conforming implementation is required to act on it.

  // Composition
  fields: FieldAssignment[]
  // type inheritance, fieldGroups, and validationRules are extensions; see
  // ext:type-inheritance, ext:field-groups, and ext:cross-field-validation

  // lifecycle is an extension; see ext:lifecycle

  // Classification
  tags?: string[]

  // Metadata
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

#### `FieldAssignment`

A Field reference within a Type. Configures presentation without redefining field semantics.

```typescript
{
  fieldId: UUID     // references Field.id
  order: integer    // min: 0; display and processing order within the Type
  required?: boolean  // default: true

  // Presentation-only — must NOT affect AI guidance, extraction, valueType, or validation
  displayLabel?: string
  displayHint?: string
}
```

`displayLabel` and `displayHint` are strictly for rendering. If a materially different label or meaning is needed, a distinct Field with its own lineage is required.

Repeatability fields (`repeatable`, `minItems`, `maxItems`) are defined in `ext:repeatable-fields`.

The Type's effective field list is `fields[]` unless `ext:type-inheritance` is declared and the Type extends another Type. In that case, the effective field list also includes inherited fields as defined by `ext:type-inheritance`.

**AI guidance composition order** (recommended):

1. Type framing (`Type.aiGuidance.extraction`) — establishes the semantic object type
2. View framing (`View.aiGuidance.extraction`, if `ext:views-l1` is in use) — workflow-specific context
3. Field extraction guidance (`Field.aiGuidance.extraction`)
4. Negative guidance (`Field.aiGuidance.negativeGuidance`)
5. Examples (`Field.aiGuidance.examples`)

This is a recommended default, not a required invariant. Implementations that compose differently will produce different AI behaviour from the same definitions.

**On instance migration when a Type version changes:**
A Record binds to a specific `typeVersion` at creation time. Existing Records do not automatically migrate when a new Type version is published. Conformance is measured against the version the Record was instantiated under. When a Record is migrated and exchanged, it should carry the version it now conforms to, and the original Record should be preserved and linked via a `supersedes` Relation.

---

### 4.4 Record tiers

SCDS supports three semantic maturity tiers. Implementations are not required to support all three; they may begin at Tier 2.

| Tier | Type | Structure | Semantics |
|---|---|---|---|
| **0** | `Note` | Named sections + free text | None |
| **1** | `Typed Record` | Named fields with types and values | Minimal |
| **2** | `Record` | Fields bound to a `Type` definition | Full |

Graduation path: Note → Typed Record → Record.

#### `NoteSection`

A named text section within a Note.

```typescript
{
  name: string          // section key; unique within the Note; snake_case recommended
  label?: string
  content: string
  contentHint?: "text" | "markdown" | "plain"  // hint only; default: "text"
}
```

#### `Note`

A lightweight instance with no Type binding.

```typescript
{
  instanceId: UUID

  title?: string
  sections: NoteSection[]

  graduatedAt?: ISO8601
  // When set, signals full formalisation. Authoritative record of successors
  // is in derived-from Relations from the successor Records.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

#### `TypedField`

A field within a Typed Record.

```typescript
{
  name: string
  label?: string
  valueType?: "string" | "text" | "number" | "boolean" | "date" | "url" | "select" | "multiselect"
  selectOptions?: string[]
  value: string | number | boolean | string[] | null
  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
}
```

#### `Typed Record`

A structured instance with named, typed fields but no Type binding.

```typescript
{
  instanceId: UUID

  title?: string
  instanceType?: string  // lightweight semantic hint; not a formal type declaration

  fields: TypedField[]

  graduatedAt?: ISO8601

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

#### `SourceReference`

A pointer from a field value or instance back to source material.

```typescript
{
  sourceType: "transcript-chunk" | "transcript-segment" | "external-document"
  sourceId: string
  sourceStandard?: string   // versioned standard the source conforms to
  streamId?: UUID           // for transcript sources: originating stream

  relationType?: "evidence" | "derived-from" | "quoted-from" | "inspired-by" | "supersedes-context"

  confidence?: number       // 0.0–1.0
  note?: string
}
```

#### `FieldValue`

The current value of a Field within a Record.

```typescript
{
  fieldId: UUID

  // Non-repeatable — use value
  value?: string | number | boolean | string[] | null

  // Repeatable — use entries (ext:repeatable-fields)
  entries?: FieldValueEntry[]

  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601

  sourceRefs?: SourceReference[]
}
```

`FieldValueEntry` is defined in `ext:repeatable-fields`.

#### `Record`

An instantiated Type with field values.

```typescript
{
  instanceId: UUID
  typeId: UUID         // references Type.id
  typeVersion: integer
  typeNamespace: string
  typeName: string

  // lifecycleState is ext:lifecycle
  lifecycleState?: string

  fieldValues: FieldValue[]

  // groupValues is ext:field-groups
  groupValues?: FieldGroupValue[]

  sourceRefs?: SourceReference[]

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
  // Use meta for implementation-local concerns: lock state, visibility,
  // session references. Cross-system keys should be namespaced,
  // e.g. "com.acme.locking.locked-by".
}
```

`typeNamespace` and `typeName` are denormalised convenience fields. If they conflict with the resolved Type, the `typeId`/`typeVersion` identity takes precedence and the Record is considered invalid until corrected.

**On instance revision:**
- **In-place edits** (`updatedAt` advances, `fieldValues` mutate): for minor corrections that do not alter semantic meaning.
- **Semantic updates**: produce a new Record linked to the prior by a `supersedes` or `refines` Relation. The prior Record remains valid.
- **Immutable records + Relation graph**: all Records append-only; a new Record for every change. A valid implementation strategy that naturally preserves history.

---

### 4.5 Relation

A first-class typed link between instances. Relations allow implementations to construct semantic graphs for navigation, analysis, projection, and reasoning.

```typescript
{
  relationId: UUID

  relationType: string
  // Free-form. See ext:recommended-relations for canonical types and conventions.

  // source [relationType] target
  sourceInstanceId: UUID    // the asserting instance
  targetInstanceId: UUID    // the related instance

  assertedBy?: "human" | "ai" | "imported"
  confidence?: number       // 0.0–1.0; meaningful for ai-asserted
  createdAt?: ISO8601
  createdBy?: string

  status?: "proposed" | "active" | "rejected" | "superseded"
  validFrom?: ISO8601
  validUntil?: ISO8601

  notes?: string
  sourceRefs?: SourceReference[]
  meta?: Record<string, unknown>
}
```

**Directionality convention:**
`sourceInstanceId` is the asserting instance; `targetInstanceId` is the related instance. The Relation reads: "source [relationType] target."

| Relation | source | target |
|---|---|---|
| `supersedes` | the newer Record | the older Record |
| `contains` | the stage | the task inside it |
| `depends-on` | the dependent task | the task it needs |
| `refines` | the detailed version | the rough version |
| `derived-from` | the successor | the source Note or Record |
| `evidences` | the source material | the claim it supports |

This convention must be consistent across implementations. See Invariant 16.

Relations span tiers. A Note may be the target of `derived-from` Relations from the Records it graduated into.

**Canonical relation types** (use these exact strings for cross-system interoperability):

`contains`, `depends-on`, `supersedes`, `refines`, `derived-from`, `evidences`, `precedes`

Custom types not covered by these should use `namespace/name` format (e.g. `com.acme.hr/transferred-to`) to prevent collision. Extended relation type metadata is defined in `ext:recommended-relations`.

**Relations do not change lifecycle state.** A `supersedes` Relation does not mutate the prior Record's `lifecycleState`. Lifecycle state changes are explicit acts by an implementation's transition mechanism.

---

### 4.6 Container

A lightweight grouping boundary over a collection of instances. Containers answer scoping questions — which instances belong together, what constitutes "this project" — that the Relation graph alone cannot answer.

Containers are not semantic objects with Fields. They do not own semantic state; Records do. A `contains` Relation asserts "A is part of B" (a semantic claim); a Container asserts "these instances form a unit for boundary purposes" (a scope claim). Both are needed; neither replaces the other.

```typescript
{
  containerId: UUID

  namespace?: string
  name?: string

  title: string              // human-readable label

  containerType?: string     // free-form hint; e.g. "project", "meeting", "sprint"

  rootInstanceIds?: UUID[]
  // Top-level instances this Container was created to hold. Implementations may
  // derive nested members by traversing contains Relations from these roots.

  memberInstanceIds?: UUID[]
  // Explicit membership list for all instances in scope.
  // When present, allows membership queries without graph traversal.
  // When omitted, membership is defined by traversing contains Relations.

  createdAt?: ISO8601
  updatedAt?: ISO8601
  meta?: Record<string, unknown>
}
```

`Container.containerId` is not an instance ID and must not appear in `Relation.sourceInstanceId` or `targetInstanceId`. See Invariant 19.

---

## 5. Distribution Group (Core)

The Distribution group is required for all conforming implementations.

### 5.1 Package

The distributable artefact. Contains Field, Type, View, and Relation type definitions with a complete dependency manifest.

```typescript
{
  schemaVersion: string      // SCDS spec version, e.g. "2.0"
  packageId: UUID
  packageName: string
  packageVersion: string     // semver, e.g. "1.2.0"
  publishedAt: ISO8601
  publisher?: string
  description?: string
  homepage?: string

  // Content (at least one of fields or types must be non-empty)
  fields: Field[]
  types: Type[]
  views?: View[]             // ext:views-l1; omit if not in use
  documentViews?: DocumentView[]  // ext:views-l2; omit if not in use
  schemas?: Schema[]         // ext:schema; omit if not in use
  protocols?: Protocol[]     // ext:protocol; omit if not in use
  relationTypes?: RelationTypeDefinition[]  // ext:recommended-relations

  mode: "bundled" | "standalone"

  dependencyRefs: Reference[]
}
```

**`mode` semantics:**

| Mode | Meaning |
|---|---|
| `"bundled"` | All Field records referenced by any Type, all Type records referenced by any Type or View, and all View records referenced by any DocumentView are included in their respective arrays. Self-contained. |
| `"standalone"` | Dependencies are expected pre-installed in the consumer's registry. `dependencyRefs` is the required manifest. |

`dependencyRefs` is required in both modes. Consumers use it to validate completeness without parsing content internals.

---

### 5.2 Reference

A stable pointer to a specific definition version.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1
  definitionType?: "field" | "type" | "view" | "schema" | "protocol"
}
```

Canonical string form: `namespace/name@version`

---

### 5.3 Lineage

Upstream and fork tracking for a specific definition version.

```typescript
{
  sourceDefinitionId?: UUID     // UUID of the upstream definition
  sourceVersion?: integer       // upstream version at derivation time
  forkedFromDefinitionId?: UUID // UUID of the definition deliberately forked from
  forkedFromVersion?: integer   // version at the fork point
}
```

| Field pair | Meaning |
|---|---|
| `sourceDefinition*` | Tracked copy; consumer expects upstream updates |
| `forkedFrom*` | Deliberately diverged; no upstream tracking |

Both may be present during a transition from tracking to forking.

---

### 5.4 Provenance

Publisher and package origin metadata.

```typescript
{
  publisher?: string        // namespace or org of the original author
  sourcePackage?: string    // package name that bundled this definition
  packageVersion?: string   // semver of the source package
  importedAt?: ISO8601
}
```

`packageVersion` is distinct from `Field.version`. A package at `1.3.0` may contain `decision_statement@3` and `context@2`.

---

## 6. Conversation Layer

The conversation layer is a permanent architectural boundary distinct from SCDS. It captures raw multimodal source material; SCDS captures negotiated semantic state. They reference each other bidirectionally via `SourceReference` (document → conversation) and `AttentionState` tags (conversation → document, via `ext:addressability`).

```
Conversation layer  →  raw multimodal source material (speech, threads, annotations)
                        elements tagged with Address at production time
Protocol layer      →  structures the facilitation process; advances AttentionState
SCDS layer          →  captures negotiated semantic state; Records carry SourceReferences
Presentation layer  →  renders SCDS state via Views
```

Three conversation types are in scope:

| Type | Structure | Anchoring |
|---|---|---|
| Meeting transcript | Linear, time-ordered chunks | Tagged with AttentionState at production time |
| Threaded conversation | Tree of replies | Thread root anchored to a document element Address |
| Web UI annotations | Attached to content | Anchored to a Field or Record Address |

Transcript chunks referenced in `SourceReference` are source material — addressable evidence. They do not become Notes or Records automatically. A transcript chunk referenced in `sourceRefs` is evidence supporting a field value; it is not itself a Note unless someone deliberately models it as one.

---

## 7. Extensions

Extensions are optional, independently adoptable. Each extension section declares its identifier, dependencies, and the types it defines.

---

### ext:addressability

**Required for**: any implementation with live facilitation or multi-session extraction.

Defines a universal addressing scheme and the mechanisms that connect conversation material to document elements.

#### `Address`

A stable, resolvable identifier for any element across document space, process space, and conversation space.

```typescript
type Address =
  | {
      space: "document"
      containerId: UUID
      recordId?: UUID
      fieldId?: UUID
      revisionId?: UUID    // requires ext:addressability Revision
    }
  | {
      space: "process"
      runId: UUID          // Protocol run ID; requires ext:protocol
      stageId?: string
    }
  | {
      space: "conversation"
      sessionId: UUID
      chunkId?: UUID
      annotationId?: UUID
    }
```

Every element that can be referred to has an Address. A transcript chunk and a field Revision are co-addressable because assertions about one referencing the other require both to be resolvable.

#### `AttentionState`

The current focus of an active Protocol run — a live cursor across the address space. `AttentionState` and `Address` are structurally related but serve distinct roles: an `Address` is a stable, resolvable identifier for a specific element; `AttentionState` is the mutable cursor that records *where focus currently is* during an active session. An `AttentionState` value at a point in time resolves to a document-space `Address`, but it is stored separately because it changes continuously as the Protocol advances.

Conversation material is tagged with the active `AttentionState` as it is produced. This makes context assembly efficient: "all chunks produced while focus was on this Field" is a queryable address predicate.

```typescript
{
  containerId: UUID
  recordId?: UUID
  fieldId?: UUID
  protocolRunId?: UUID
  stageId?: string
}
```

`AttentionState` is set live by the session or Protocol runner. `SourceReference` is set retrospectively at extraction or editorial review time. Both are needed; they answer different questions.

#### `Revision`

A first-class, addressable snapshot of a `FieldValue` at a point in time. Carries the value, the agent, a timestamp, and source references to the conversation that produced the change.

```typescript
{
  revisionId: UUID
  fieldId: UUID
  recordId: UUID

  value: FieldValue
  agent: "human" | "ai" | "imported"
  createdAt: ISO8601

  sourceRefs?: SourceReference[]
  priorRevisionId?: UUID  // chain to the previous Revision for this field
}
```

Revision does not replace the edit-in-place vs. new-Record judgment. Minor corrections remain in-place edits at the implementation layer. Revision is the addressable audit trail for interoperability — it makes field history queryable: "what did this field say before the last Protocol run?", "which conversation produced the change from revision 2 to revision 3?"

#### Context Query (behavioural requirement)

A conforming `ext:addressability` implementation must be able to assemble relevant material given an address and a purpose. This is a behavioural requirement, not a data shape.

**Required query patterns:**

| Pattern | Address | Returns |
|---|---|---|
| Field context | `{recordId}/{fieldId}` | Current value, Revision history, chunks tagged to this Field, Field `aiGuidance` |
| Record context | `{recordId}` | All field values, chunks tagged to this Record, Relations, Protocol run history |
| Stage context | `{runId}/{stageId}` | All chunks produced during this stage, Fields active in this stage |
| Revision trace | `{fieldId}/{revisionId}` | Value at that Revision, the conversation that produced it, prior Revision chain |

**Recommended assembly order for AI assistance:**

1. Type and Field `aiGuidance` — what this field captures, how to extract it
2. Current value and recent Revision history — what has already been established
3. Chunks tagged to this Field via AttentionState — most focused context
4. Chunks tagged to the parent Record — broader session context
5. Related Records via Relations — structural context

---

### ext:lifecycle

**Required for**: governance tools, decision logs, any implementation where records progress through defined states.

Adds lifecycle state declarations to `Type` and lifecycle state tracking to `Record`.

#### `LifecycleState`

```typescript
{
  name: string
  description?: string
  isInitial?: boolean   // valid starting state for new Records
  isFinal?: boolean     // no transitions out; Record is settled
}
```

#### `LifecycleTransition`

```typescript
{
  name: string       // e.g. "promote", "approve", "supersede"
  from: string       // must match a state name in the enclosing lifecycle
  to: string
  description?: string
}
```

#### Type lifecycle block (added by this extension)

When `ext:lifecycle` is in use, `Type` gains:

```typescript
lifecycle?: {
  states: LifecycleState[]           // min 1 state
  transitions: LifecycleTransition[]
  initialState: string               // must reference a state name where isInitial === true
}
```

#### Record lifecycle state (added by this extension)

`Record.lifecycleState` becomes meaningful: must match a state name in the associated `Type.lifecycle.states[]` when the Type declares a lifecycle.

The `lifecycle` block declares vocabulary. Implementations decide enforcement strictness. A state with `isFinal: true` signals that no further transitions are expected; implementations may use this to lock Record content.

---

### ext:protocol

**Required for**: facilitation tools, structured deliberation, any implementation that guides users through epistemic stages.

Replaces `TemplateFacilitationStep` from v1. Protocol is epistemically richer: stages have explicit dependencies, completion criteria, and may produce intermediate Records.

#### `TypeRef`

A reference to a specific Type, used within Protocol and Schema.

```typescript
{
  typeId: UUID
  typeVersion?: integer
}
```

#### `FieldRef`

A reference to a Field within a Type.

```typescript
{
  fieldId: UUID
  typeId?: UUID    // which Type this Field appears in
}
```

#### `ProtocolStage`

A named stage in a Protocol. Stages have epistemic dependencies (`dependsOn`) — not just ordering. A stage may only proceed when its dependencies are sufficient.

```typescript
{
  stageId: string       // stable key within this Protocol
  order: integer        // min: 0; display/presentation order only — see note below
  purpose: string       // what understanding this stage builds
  question: string      // the core question this stage answers
  dependsOn: string[]   // stageId values; epistemic dependencies, not just ordering
  completionCriteria: string   // how to know this stage is sufficient to proceed
  contributesTo: FieldRef[]    // which Record Fields this stage feeds
  outputType?: TypeRef         // if this stage produces its own intermediate Record
  aiGuidance: AiGuidance
}
```

**`order` vs `dependsOn`:** `order` is the display and presentation sequence — how stages are shown in a UI or facilitation guide. Execution sequence is determined by `dependsOn` resolution: a stage runs when all its declared dependencies are satisfied, regardless of its `order` value. Authors must ensure `order` is consistent with the partial order implied by `dependsOn` (i.e. a stage's `order` value should be greater than the `order` of any stage it depends on). See Invariant 31.

#### `Protocol`

An epistemically ordered process for building quality Records through structured conversation or facilitation.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string

  targetType?: TypeRef
  // The Record type this Protocol produces. Absent for loose / exploratory Protocols
  // (Brain Dump, Decomposition) whose output is input context for a tighter Protocol.

  stages: ProtocolStage[]

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

**The Protocol spectrum:**

```
Loose                                                    Tight
─────────────────────────────────────────────────────────────
Brain Dump → Decomposition → Options Analysis → Decision
```

Loose Protocols produce open material. Tight Protocols converge on a specific Record type. The output of a loose Protocol is the input context for something tighter.

**Generic Protocols** (reusable across domains):
- Brain Dump — externalise all thinking without constraint
- Decomposition — identify major components from raw material
- Review — what is established, what is still open
- Prioritisation — which components to resolve first

**Domain-specific Protocols** (target a specific Record type):
- Decision — context → criteria → options → evaluation → decision
- Proposal — problem → solution shape → constraints → proposal

**Protocol chaining and provenance**: The output of one Protocol is the input context for the next. This derivation chain is traceable through `derived-from` Relations, making the quality and history of the final Record auditable.

**Non-normative example — Protocol chain for a governance decision:**

```
Brain Dump Protocol (loose, no targetType)
  → AttentionState: { containerId: C1 }
  → Produces: Note N1 (unstructured brainstorm)

Decomposition Protocol (loose, targetType: Component)
  → AttentionState: { containerId: C1, recordId: N1 }
  → Produces: Notes N2, N3, N4  [derived-from N1]

Decision Protocol (tight, targetType: Decision)
  → AttentionState: { containerId: C1, protocolRunId: R1, stageId: "criteria" }
  → Stage "criteria" produces: Options Analysis Record R-OA  [derived-from N2, N3]
  → Stage "decision" produces: Decision Record R-D           [derived-from R-OA]

Conversation chunks produced during Decision stage:
  chunk-42: { AttentionState: { containerId: C1, recordId: R-OA, fieldId: F-criteria, ... } }
  chunk-43: { AttentionState: { containerId: C1, recordId: R-D, fieldId: F-outcome, ... } }

Context query for R-D / F-outcome:
  → Field aiGuidance from Decision Type + outcome Field
  → Current value + Revision history for F-outcome
  → Chunks tagged with { recordId: R-D, fieldId: F-outcome } — chunk-43
  → Chunks tagged with { recordId: R-D } — broader session context
  → Related Records via Relations — R-OA via derived-from
```

The final Decision Record is auditable because every Protocol stage left addressable artefacts. The quality of the outcome is traceable to the conversation that produced it.

Views (`ext:views-l1`) no longer contain facilitation logic. A View is a presentation concern; a Protocol is an epistemic one.

---

### ext:schema

**Required for**: extraction pipelines, founding document workflows, any system that needs to specify what a document type IS before assembling it.

#### `RelationSpec`

Declares an expected Relation between two Record types within a Schema.

```typescript
{
  relationType: string
  sourceType: TypeRef
  targetType: TypeRef
  cardinality?: "one-to-one" | "one-to-many" | "many-to-many"
  required?: boolean
}
```

#### `Schema`

The definition of a complete document type — which Types it contains, what Relations exist between resulting Records, and what "complete" means. A Schema is the artefact handed to an extraction pipeline.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string

  rootTypes: TypeRef[]        // Types to extract
  structure: RelationSpec[]   // expected Relations between extracted Records
  requiredTypes: TypeRef[]    // what "complete" means for this document type

  aiGuidance?: AiGuidance
  // purpose: what kind of document this Schema defines
  // extraction: framing for extraction pipelines

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

**Schema vs View:**

| | Schema | View / Document View |
|---|---|---|
| Question it answers | What IS this document type? What should be extracted? | How are existing Records assembled into readable output? |
| Operates at | Definition time | Projection time |
| Input | Source material (transcripts, conversations) | Existing Records in a Container |
| Output | Extraction instructions → Records | Rendered document |

---

### ext:type-inheritance

**Required for**: Type libraries that need formal specialization while preserving base-Type processability.

Defines single inheritance for Types. A specializing Type inherits the fields and semantics of a base Type, may add fields, and remains processable as the base Type by systems that know the base Type but not the specialization.

When `ext:type-inheritance` is in use, `Type` gains:

```typescript
{
  extendsTypeId?: UUID
  // UUID of the base Type this Type specializes.
  // When present, the effective field list consists of inherited fields
  // followed by this Type's own fields[], unless fieldOrder is present.

  extendsTypeVersion?: integer
  // Version of the base Type targeted by this specialization.

  fieldOrder?: UUID[]
  // Optional explicit ordering of all fields in the effective field list:
  // inherited fields plus this Type's own fields[].
  // This is an ordering declaration only; it does not re-declare field
  // assignments or change Field semantics.

  fieldAssignmentOverrides?: FieldAssignmentOverride[]
  // Presentation and workflow overrides for inherited fields only.
}
```

#### `FieldAssignmentOverride`

Overrides presentation or workflow constraints for an inherited Field in a specializing Type. It does not change the Field's semantics.

```typescript
{
  fieldId: UUID
  displayLabel?: string
  displayHint?: string
  required?: boolean
}
```

`displayLabel` and `displayHint` are presentation-only. `required` may tighten an inherited optional field (`false` to `true`) for the specializing Type. It must not relax an inherited required field (`true` to `false`), because a Record instantiated against the specializing Type must remain valid when processed as the base Type.

The effective field list for a specializing Type is the inherited effective field list of its base Type plus the specializing Type's own `fields[]`. A specializing Type must not duplicate an inherited `fieldId` in its own `fields[]`.

Example:

```text
Type: core/decision
  fields: decision_statement, context, rationale, options_considered

Type: org.example/governance_decision
  extendsTypeId: core/decision
  adds: ratification_method, quorum_threshold, voting_record
```

A system that knows `core/decision` but not `org.example/governance_decision` can still read the inherited decision fields. The specializing fields are unknown extension content to that system and should be preserved rather than discarded.

---

### ext:views-l1

**Required for**: rendering and export workflows.

Defines Views — versioned presentations of a single Record through a specific Type.

#### `FieldView`

A field reference within a View. Controls presentation for this View without altering field semantics.

```typescript
{
  fieldId: UUID       // must appear in the Type's effective field list
  order: integer      // min: 0; display order within this View
  required?: boolean  // View-level workflow constraint; does not alter Field contract
  visible?: boolean   // default: true

  // Presentation overrides — View scope only
  displayLabel?: string
  displayHint?: string
  editorHintOverride?: string
}
```

A Field hidden with `visible: false` remains in the Record and may appear in other Views.

#### `ExportConfig`

Configuration for rendering a Record through this View as an exportable document.

```typescript
{
  format?: string        // target format hint, e.g. "markdown", "adoc", "json"
  preamble?: string
  // Template string rendered before field values.
  // Variable substitution uses {{variable-name}} syntax.
  // Standard variables: {{instance-id}}, {{date}}, {{status}}, {{namespace}}, {{name}}

  fieldOrder?: UUID[]    // explicit export field ordering; defaults to fieldViews[].order
  omitEmptyFields?: boolean  // default: false
}
```

#### `View`

A versioned presentation and export configuration over a specific Type. Multiple Views may exist for the same Type, serving different audiences or purposes.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string    // when to use this View; what workflow or audience it serves

  typeId: UUID           // references Type.id
  typeVersion: integer

  aiGuidance?: AiGuidance
  // purpose: the workflow context this View serves
  // extraction: session-level framing injected before field extraction

  fieldViews: FieldView[]

  protection?: "none" | "read-only" | "fill-in"
  // Default: "none".
  // "read-only" — Records rendered through this View cannot be edited.
  // "fill-in"   — only null or empty Field values may be populated.
  // Protection is a View-level workflow constraint. It does not modify
  // the Record or replace lifecycle states.

  exportConfig?: ExportConfig

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

A View may not introduce Fields that are not in the bound Type's effective field list. Omitted Fields are treated as `visible: false`.

`View.protection` applies only to interactions through that View. A Record may be editable through one View and read-only through another. For record-level settlement, use `ext:lifecycle` states such as `isFinal`.

Facilitation steps have been removed from View. Use `ext:protocol` Protocol stages instead.

---

### ext:views-l2

**Depends on**: `ext:views-l1`

**Required for**: document projection — assembling multiple Records into a coherent document.

#### `SectionSource`

Defines how a section's instances are selected from a Container.

```typescript
type SectionSource =
  | {
      type: "fixed-instances"
      instanceIds: UUID[]
      // Explicit list. For preamble, cover page, or curated sections.
    }
  | {
      type: "type-query"
      semanticObjectType: string
      // For cross-system portability, use namespace/name format (e.g. "core/decision").
      // A bare string like "decision" is a single-system convention.
      lifecycleState?: string
      containerIds?: UUID[]
    }
  | {
      type: "relation-query"
      fromInstanceId: UUID
      relationType: string
      direction?: "forward" | "inverse"  // default: "forward"
    }
  | {
      type: "container-subset"
      containerId: UUID
      containerType?: string
    }
```

#### `DocumentSection`

One section in a Document View.

```typescript
{
  sectionId: string
  title?: string
  description?: string
  order: integer   // min: 0

  source: SectionSource

  renderViewId?: UUID    // View (ext:views-l1) used to render each instance in this section
  // When absent, implementations use a default rendering for the instance type.

  ordering?: {
    fieldId?: UUID
    direction?: "asc" | "desc"  // default: "asc"
  }

  required?: boolean
  emptyBehavior?: "hide" | "show-placeholder"
}
```

#### `NavigationLink`

An assembly-time cross-section link in a Document View. Navigation links are reading aids for the rendered document, not semantic assertions about Records. They do not appear in the Relation graph.

```typescript
{
  fromSectionId: string
  toSectionId: string
  label?: string
  bidirectional?: boolean  // default: false
}
```

#### `DocumentView`

A versioned, Container-level projection. Defines how a Container's Records are assembled into a readable document.

```typescript
{
  id: UUID
  namespace: string
  name: string
  version: integer   // min: 1

  description: string    // what kind of document this produces; intended audience

  containerType?: string  // when set, intended for Containers of this type

  sections: DocumentSection[]

  navigationLinks?: NavigationLink[]

  preamble?: string
  // Template string rendered before all sections.
  // Standard variables: {{container-title}}, {{date}}, {{container-id}}

  format?: string   // e.g. "markdown", "adoc", "html"

  aiGuidance?: AiGuidance
  // purpose: what kind of document this View produces
  // extraction: context for AI-assisted document-level tasks

  tags?: string[]
  createdAt: ISO8601
  lineage?: Lineage
  provenance?: Provenance
}
```

A `DocumentView` may reference multiple `View` records (via `DocumentSection.renderViewId`) — one per instance type in the document. It orchestrates; it does not replace them.

`DocumentSection.renderViewId` references a `View.id` (from `ext:views-l1`). A `DocumentView.id` is not a valid value for `renderViewId` — Document Views are not nestable.

Use `navigationLinks` when a rendered document should include "see also" or related-section links. Use `Relation` only when the relationship is a semantic assertion about Records.

---

### ext:repeatable-fields

**Required for**: any Record type that needs lists of values within a single Field.

Adds repeatability to `FieldAssignment` and defines `FieldValueEntry`.

#### `FieldValueEntry`

A single entry in a repeatable field.

```typescript
{
  value: string | number | boolean | string[] | null
  source?: "human" | "ai" | "imported" | "derived"
  editedAt?: ISO8601
}
```

#### FieldAssignment additions

When `ext:repeatable-fields` is in use, `FieldAssignment` gains:

```typescript
repeatable?: boolean  // default: false; when true, multiple values are allowed
minItems?: integer    // meaningful only when repeatable === true
maxItems?: integer    // meaningful only when repeatable === true
```

And `FieldValue.entries` becomes active: use `entries` when `repeatable === true`, `value` otherwise.

A repeatable field entry does not create a new semantic instance. Use separate Records connected by Relations when repeated items need their own identity, lifecycle, or graph position.

---

### ext:field-groups

**Required for**: Record types where multiple Fields are semantically paired and repeat together as a unit.

Use when parallel `multiselect` arrays would lose pairing (e.g. a contact record with `name` + `email`). Preserves internal pairing across repeated items.

#### `FieldGroup`

A named, ordered group of Fields that repeat together as a unit within a Type.

```typescript
{
  groupId: string        // stable key within the Type
  label?: string
  description?: string

  order: integer         // min: 0; position relative to other Fields and Groups

  required?: boolean     // default: false
  repeatable?: boolean   // default: false
  minItems?: integer
  maxItems?: integer

  fields: FieldAssignment[]
}
```

#### `FieldGroupEntry`

One entry in a repeatable Field Group.

```typescript
{
  entryId?: UUID         // stable key for this entry; allows referencing or updating
  fieldValues: FieldValue[]
}
```

#### `FieldGroupValue`

The current value of a Field Group within a Record.

```typescript
{
  groupId: string           // references FieldGroup.groupId in the Type definition
  entries: FieldGroupEntry[]
}
```

A `FieldGroup` does not create a new semantic instance. Its entries are embedded structured context within the enclosing Record. Use separate Records connected by Relations when group entries need their own identity, lifecycle, provenance, or reuse across Records.

`FieldGroup.repeatable`, `minItems`, and `maxItems` define group-level repeatability — whether the group as a whole can appear multiple times within a Record. This is structurally independent from `ext:repeatable-fields`, which adds scalar repeatability to individual Fields. An implementation may adopt `ext:field-groups` without `ext:repeatable-fields`; the repeatability mechanics in each extension are self-contained.

When `ext:field-groups` is in use, `Type` gains `fieldGroups?: FieldGroup[]` and `Record` gains `groupValues?: FieldGroupValue[]`.

**Repeatability pattern guide:**

| Pattern | Use | Example |
|---|---|---|
| Repeatable scalar | `FieldAssignment.repeatable` (ext:repeatable-fields) | Multiple assigned person names |
| Repeatable structured context | `FieldGroup` | Contacts with name + email pairs |
| Repeated semantic objects | Separate Records + Relations | Tasks assigned to roles |

---

### ext:cross-field-validation

**Required for**: Types with constraints that span multiple Fields.

`ValidationRule` handles single-field constraints. `CrossFieldRule` handles constraints that require evaluating more than one Field together.

#### `CrossFieldRule`

```typescript
{
  type: "conditional-required" | "field-ordering" | "mutual-exclusion"
  message?: string

  // conditional-required: targetFieldId becomes required when predicateFieldId equals predicateValue
  predicateFieldId?: UUID
  predicateValue?: string
  targetFieldId?: UUID

  // field-ordering: targetFieldId must precede or follow predicateFieldId
  // Applies only to fields with valueType "date" or "number".
  effect?: "must-precede" | "must-follow"

  // mutual-exclusion: at most one of the listed fields may have a non-empty value
  fieldIds?: UUID[]   // min: 2
}
```

| Rule type | Required fields |
|---|---|
| `conditional-required` | `predicateFieldId`, `predicateValue`, `targetFieldId` |
| `field-ordering` | `predicateFieldId`, `targetFieldId`, `effect` |
| `mutual-exclusion` | `fieldIds` (min 2) |

When `ext:cross-field-validation` is in use, `Type` gains `validationRules?: CrossFieldRule[]`.

---

### ext:recommended-relations

**Required for**: cross-system federation; multi-publisher ecosystems where Relation type semantics must be interoperable.

Canonical relation types and machine-readable Relation type definitions.

**Canonical relation types** (use exact strings):

| Canonical | Converse | Category |
|---|---|---|
| `contains` | `part-of` | Composition |
| `depends-on` | `required-by` | Dependency |
| `supersedes` | `superseded-by` | Governance |
| `refines` | `refined-by` | Refinement |
| `derived-from` | `source-of` | Derivation |
| `evidences` | `evidenced-by` | Evidence |
| `precedes` | `follows` | Sequence |

Implementations must store only the canonical (forward) form and derive the inverse when needed.

**Relation category taxonomy:**

| Category | Examples |
|---|---|
| Composition | `contains`, `part-of`, `has-section` |
| Refinement | `refines`, `expands`, `summarises` |
| Dependency | `depends-on`, `requires`, `blocks`, `enables` |
| Sequence | `precedes`, `follows`, `overlaps` |
| Derivation | `derived-from`, `extracted-from`, `based-on` |
| Evidence | `evidences`, `supports`, `contradicts` |
| Governance | `supersedes`, `amends`, `ratifies`, `delegates` |
| Association | `relates-to`, `links-to` |

#### `RelationTypeDefinition`

Machine-readable metadata for a `relationType` string.

```typescript
{
  relationType: string      // exact string used in Relation.relationType
  namespace: string
  label?: string
  description?: string
  category?: "composition" | "refinement" | "dependency" | "sequence" | "derivation" | "evidence" | "governance" | "association"
  canonicalDirection?: string   // e.g. "source is the dependent task; target is the task it depends on"
  inverseType?: string
}
```

`RelationTypeDefinition` is optional metadata. Implementations are not required to resolve `relationType` strings against a definition before accepting a Relation. Relation type definitions may be included in a Package or published separately.

---

### ext:import-tracking

**Required for**: implementations that receive packages from upstream publishers and need to track update and conflict state.

#### `ImportMode`

```typescript
"upstream-tracked" | "local-copy" | "local-fork"
```

| Mode | Meaning |
|---|---|
| `"upstream-tracked"` | Consumer expects updates from the source Package. Conflicts surfaced when local and upstream diverge. |
| `"local-copy"` | Imported as a snapshot. No update tracking. |
| `"local-fork"` | Deliberately diverged. Upstream lineage preserved for reference. |

#### `ImportRecord`

One record per imported definition in a consumer's local registry.

```typescript
{
  definitionId: UUID
  definitionType: "field" | "type" | "view" | "schema" | "protocol"
  namespace: string
  name: string
  version: integer

  mode: ImportMode
  importedAt: ISO8601

  sourcePackageId: UUID
  sourcePackageName: string
  sourcePackageVersion: string

  latestKnownUpstreamVersion?: integer
  updateAvailable?: boolean
  updateCheckedAt?: ISO8601

  conflictState?: "clean" | "local-ahead" | "upstream-ahead" | "diverged"
  conflictDetectedAt?: ISO8601

  localVersion?: integer
  localEditedAt?: ISO8601
}
```

#### `ImportSummary`

A consumer's complete picture of its imported definitions.

```typescript
{
  generatedAt: ISO8601
  fields: ImportRecord[]
  types: ImportRecord[]
  views: ImportRecord[]
  schemas: ImportRecord[]
  protocols: ImportRecord[]
}
```

---

### ext:registry

**Required for**: multi-publisher ecosystems; discoverable definition catalogs.

#### `RegistryEntry`

One entry in a Registry catalog.

```typescript
{
  packageId: UUID
  packageName: string
  packageVersion: string
  publisher: string
  description?: string
  publishedAt: ISO8601
  homepage?: string
  tags?: string[]
  fieldCount: integer       // min: 0
  typeCount: integer        // min: 0
  viewCount?: integer
  schemaCount?: integer
  protocolCount?: integer
  relationTypeCount?: integer
  downloadUrl?: string
  checksum?: string         // SHA-256 hex digest for integrity verification
}
```

#### `Registry`

A registry's published index.

```typescript
{
  schemaVersion: string
  registryId: UUID
  registryName: string
  catalogVersion: string    // registry's own version (semver)
  updatedAt: ISO8601
  homepage?: string
  entries: RegistryEntry[]
}
```

Multiple Registries may coexist. A consumer may index multiple catalogs. The specification does not define registry authority, authentication, or federation.

---

## 8. Key Invariants

Conforming implementations must uphold the following invariants.

### Field semantics

**1.** `FieldAssignment.displayLabel` and `FieldAssignment.displayHint` are for rendering only. They must not affect AI guidance, extraction logic, `valueType` interpretation, or validation.

**2.** A `Type` must not redefine, override, or duplicate the semantic content of any `Field` it includes. If different semantics are needed for a Field in a specific Type context, a distinct `Field` with its own identity and lineage must be created.

**3.** A `Field`'s `aiGuidance` belongs to the Field. Type-level `aiGuidance` provides session framing only.

### Lifecycle (ext:lifecycle)

**4.** `Type.lifecycle.initialState` must reference a `name` that appears in `lifecycle.states[]` and where `isInitial === true`.

**5.** Every `from` and `to` value in `lifecycle.transitions[]` must reference a `name` that appears in `lifecycle.states[]`.

**6.** `Record.lifecycleState`, when present, must reference a `name` in the associated `Type.lifecycle.states[]`.

### Distribution

**7.** Every `fieldId` referenced in any `FieldAssignment` within a `Package.types[]` must appear as the `id` of an entry in `Package.dependencyRefs`.

**8.** If `Package.mode === "bundled"`: every `Reference` in `dependencyRefs` must have a matching `Field` in `fields[]` (matched on `id` and `version`).

**9.** `Field.id` is stable across versions. A new `id` means a new definition, not a new version of an existing one.

### Cross-field validation (ext:cross-field-validation)

**10.** All `fieldId` values in any `CrossFieldRule` within `Type.validationRules[]` must appear in the Type's effective field list. Cross-field rules cannot reference Fields outside the Type.

**11.** A `conditional-required` rule must supply `predicateFieldId`, `predicateValue`, and `targetFieldId`. A `field-ordering` rule must supply `predicateFieldId`, `targetFieldId`, and `effect`. A `mutual-exclusion` rule must supply `fieldIds` with at least two entries.

### Views (ext:views-l1)

**12.** Every `fieldId` in `View.fieldViews[]` must appear in the bound Type's effective field list. A View cannot introduce Fields not in its Type.

**13.** `FieldView.displayLabel`, `FieldView.displayHint`, and `FieldView.editorHintOverride` are for rendering only. They must not affect AI guidance, extraction logic, `valueType` interpretation, or validation.

**14.** A `View` must not override, redefine, or duplicate the semantic content of any `Field` or `Type` it references. View-level `aiGuidance` is workflow framing; it does not redefine Field extraction semantics.

### Distribution — Views (ext:views-l1)

**15.** Every `typeId` referenced by any `View` in `Package.views[]` must appear in `Package.dependencyRefs` with `definitionType: "type"`. If `mode === "bundled"`, that `Type` must be present in `types[]`.

### Relations

**16.** In a `Relation`, `sourceInstanceId` is the asserting instance and `targetInstanceId` is the related instance. The Relation reads: "source [relationType] target." This convention must not be reversed.

**17.** `Relation` is reserved for assertions that carry semantic consequence beyond simple mention or citation. Lightweight prose references that do not assert structural, causal, or governance relationships must not be modelled as `Relation` records.

### Notes and Typed Records

**18.** `NoteSection.name` values must be unique within a `Note`.

**19.** `TypedField.name` values must be unique within a `Typed Record`.

### Containers

**20.** `Container.containerId` is not an instance ID. It must not appear in `Container.rootInstanceIds`, `Container.memberInstanceIds`, `Relation.sourceInstanceId`, or `Relation.targetInstanceId`.

**21.** `Container.rootInstanceIds` and `Container.memberInstanceIds`, when present, must reference valid SCDS instance IDs (`Note.instanceId`, `Typed Record.instanceId`, or `Record.instanceId`).

### Repeatability (ext:repeatable-fields)

**22.** If `FieldAssignment.repeatable` is false or absent, its corresponding `FieldValue` must use `value` and must not include `entries`.

**23.** If `FieldAssignment.repeatable` is true, its corresponding `FieldValue` may use `entries`. If `minItems` is specified, `entries` must contain at least that many items. If `maxItems` is specified, `entries` must not exceed that count. For repeatable fields, `Field.validationRules` are evaluated against each `FieldValueEntry.value` individually, not against the array as a whole.

**24.** `FieldAssignment.minItems` and `maxItems` are valid only when `repeatable === true`. They must be ignored when `repeatable` is false or absent.

### Field groups (ext:field-groups)

**25.** Every `groupId` in `Record.groupValues[]` must reference a `groupId` declared in the associated `Type.fieldGroups[]`.

**26.** Within a `FieldGroupEntry.fieldValues[]`, every `fieldId` must appear in the enclosing `FieldGroup.fields[].fieldId`.

**27.** A `FieldGroupValue.entries` list must satisfy `FieldGroup.minItems` and `maxItems` where specified.

### Records

**28.** `Record.typeId` and `Record.typeVersion` are the authoritative Type binding. `typeNamespace` and `typeName` are denormalised convenience fields. If they conflict with the resolved `Type`, the `typeId`/`typeVersion` identity takes precedence and the Record is considered invalid until corrected.

### Protocol (ext:protocol)

**29.** Every `stageId` in `ProtocolStage.dependsOn[]` must reference a `stageId` declared in the enclosing `Protocol.stages[]`. A stage may not declare a dependency on itself.

**30.** Every `fieldId` in `ProtocolStage.contributesTo[]` must reference a `fieldId` that appears in the stage's own `outputType`'s effective field list (when `outputType` is declared), or in `Protocol.targetType`'s effective field list (when `outputType` is absent). A single stage must not contribute to both its own `outputType` and the enclosing `Protocol.targetType`. When neither `outputType` nor `Protocol.targetType` is declared, `contributesTo` must be empty.

**31.** For every pair of stages A and B within a `Protocol` where B.dependsOn includes A.stageId, B.order must be greater than A.order. `order` is display order; execution sequence is determined by `dependsOn` resolution. The two must not contradict each other.

### Views L2 (ext:views-l2)

**32.** Any `DocumentView` in `Package.documentViews[]` that contains a `SectionSource` with `type === "type-query"` must use `namespace/name` format for `semanticObjectType` (e.g. `"core/decision"`, not `"decision"`). Bare strings are acceptable only in single-system `DocumentView` records not included in a Package. Implementations receiving a `DocumentView` from a Package with a bare `semanticObjectType` in a `type-query` section should treat the portability of that section as undefined.

### Addressability (ext:addressability)

**33.** `Revision.priorRevisionId`, when present, must reference a `Revision.revisionId` for the same `fieldId` and `recordId`. Revision chains must be acyclic.

**34.** `AttentionState.containerId` must reference a valid `Container.containerId`. Other Address components (`recordId`, `fieldId`, `protocolRunId`, `stageId`) are optional and may be absent when focus has not yet narrowed.

### Distribution — Views L2 (ext:views-l2)

**35.** Every `DocumentSection.renderViewId` in any `DocumentView` within `Package.documentViews[]` must reference a `View.id` that appears in `Package.views[]` or `Package.dependencyRefs`. If `mode === "bundled"`, that `View` must be present in `Package.views[]`.

### Distribution — Schema (ext:schema)

**36.** Every `TypeRef.typeId` referenced in any `Schema.rootTypes[]`, `Schema.requiredTypes[]`, or in any `RelationSpec.sourceType` or `RelationSpec.targetType` within `Schema.structure[]`, for each Schema in `Package.schemas[]`, must appear in `Package.dependencyRefs` with `definitionType: "type"`. If `mode === "bundled"`, each such Type must be present in `Package.types[]`.

### Distribution — Protocol (ext:protocol)

**37.** Every `TypeRef.typeId` referenced in `Protocol.targetType` or in any `ProtocolStage.outputType`, for each Protocol in `Package.protocols[]`, must appear in `Package.dependencyRefs` with `definitionType: "type"`. Every `FieldRef.fieldId` in any `ProtocolStage.contributesTo[]` must appear in `Package.dependencyRefs` with `definitionType: "field"`. If `mode === "bundled"`, those Types must be in `Package.types[]` and those Fields in `Package.fields[]`.

### Field semantics — content format

**38.** `Field.contentFormat`, when present, is only meaningful when `valueType` is `"string"` or `"text"`. Implementations must ignore `contentFormat` on fields with any other `valueType`.

### Type inheritance (ext:type-inheritance)

**39.** `Type.extendsTypeId`, when present, must reference a valid `Type.id`. Inheritance chains must be acyclic; a Type may not directly or transitively extend itself.

**40.** A specializing Type must not declare a `fieldId` in its own `fields[]` that duplicates any `fieldId` inherited from its base Type or any ancestor Type.

**41.** When `Type.fieldOrder` is present, it must contain exactly the set of field UUIDs in the Type's effective field list. No UUID may appear more than once, and no UUID from the effective field list may be absent.

**42.** Every `fieldId` in `Type.fieldAssignmentOverrides[]` must reference a field inherited from the base Type or an ancestor Type. Overrides must not reference fields declared in the specializing Type's own `fields[]`, must not alter Field semantics, and must not relax an inherited required field from `true` to `false`.

**43.** When `ext:type-inheritance` is declared, `Package.dependencyRefs` must include a `Reference` for every Type in the transitive closure of base Types for any Type in `Package.types[]`. If `mode === "bundled"`, all such base Types must be present in `types[]`.

### Views L2 navigation (ext:views-l2)

**44.** Every `NavigationLink.fromSectionId` and `NavigationLink.toSectionId` must reference a `sectionId` declared in the enclosing `DocumentView.sections[]`.

---

## 8.5 Extension Interactions

Cross-extension interactions are behavioural requirements that apply only when an implementation declares both named extensions.

### ext:protocol × ext:addressability

**Trigger**: an implementation declares both `ext:protocol` and `ext:addressability`.

**Required behaviour**: Protocol stage advancement updates `AttentionState`. When a Protocol run advances from one stage to another, the active `AttentionState` must reflect the new stage before any conversation material is tagged.

Specifically:

- `AttentionState.protocolRunId` references the active Protocol run
- `AttentionState.stageId` reflects the current stage
- `AttentionState.fieldId`, when a specific field is the current focus within a stage, is set accordingly

Conversation chunks produced while `AttentionState.stageId` is set are associated with that stage. This makes stage-level Context Queries (`{runId}/{stageId}`) return the correct material.

---

## 9. Conformance

An implementation declares conformance using the following form:

```
SCDS <version> Core [+ ext:<name> ...]
```

Example:
```
SCDS 2.0 Core + ext:lifecycle + ext:protocol + ext:views-l1 + ext:addressability + ext:recommended-relations
```

### Core conformance requirements

A core-conformant implementation must:
- Accept and validate `Field`, `Type`, `Record` (Tier 2), `Relation`, and `Container` inputs against this specification
- Enforce Invariants 1–3, 7–9, 16–21, 28, 38
- Support the Foundation and Distribution groups in full
- Implement the namespace format and reference format correctly
- Not accept `relationType` strings that include `/` except in `namespace/name` format

Support for `Note` (Tier 0) and `Typed Record` (Tier 1) is optional at core conformance level.

### Extension conformance requirements

An implementation declaring a given extension must:
- Accept and validate all types defined by that extension
- Enforce all invariants assigned to that extension
- Respect the declared dependency chain (e.g., `ext:views-l2` requires `ext:views-l1` to also be declared)

### Interoperability note

Two implementations at the same conformance level will produce compatible definitions for exchange. An implementation receiving a Package that includes types or fields from an extension it does not support should surface the unknown content, preserve it where possible, and pass it through rather than silently discard it.

---

# claude response

This appears identical to the version you shared in the previous message — same invariants 1–44, same Section 8.5, same `ext:type-inheritance` section including the corrected `FieldAssignmentOverride.required` direction.

Did you intend to share a newer version, or is there a specific section or question you'd like me to focus on?

---

# you asked

message time: 2026-05-25 08:23:54

Hmm. I may also need a document element to this system. 

In my governance context, decisions may often be supported by documents. 

Ideally I'd have another component of the system that managed documents and provided an addressable RAG that allowed them to be included like conversations. 

But I don't want to build my own custom generic rag if I can avoid it

---

# claude response

The architecture already half-accommodates this. `SourceReference.sourceType: "external-document"` is in the spec precisely for this case — a field value or Record that was informed by a PDF, policy document, or prior report. The mechanism exists; what's missing is the retrieval infrastructure behind it.

**The key question: what does "not building custom RAG" actually mean here?**

There's a spectrum:

*Managed service (least build)* — AWS Bedrock Knowledge Bases, Azure AI Search, or similar. Point at a document store, get a retrieval API back. No chunking, embedding, or index management. The downside: they're generic and don't know about SCDS addressing. You'd use them as a black-box retrieval layer, get chunks back, and wrap them in `SourceReference` records manually. For a governance context, this probably loses too much structural awareness.

*LlamaIndex or LangChain (library, not custom build)* — This is the pragmatic middle ground. You're not writing a RAG system; you're configuring an existing one. LlamaIndex handles PDF/DOCX/HTML parsing, chunking strategies, embedding, and retrieval. You add SCDS-compatible metadata (documentId, section, page, containerId) to chunks at index time. The chunks land in the same pgvector Postgres table as transcript chunks. Context Query then treats document chunks and conversation chunks identically. This is assembling tools, not building a system.

*MCP route (potentially zero build)* — You already have Google Drive connected. If governance documents live there, an MCP document retrieval server could handle search without any additional infrastructure. Less integrated with SCDS addressing but dramatically lower maintenance burden. Worth checking what's available before building anything.

**The governance-specific nuance**

Governance documents aren't just retrievable chunks — they're often themselves sources of Records. A policy document might contain principles you want to extract into SCDS Type instances. A previous decision log in PDF form might contain Decisions worth graduating into proper Records. A founding document from another organisation might contain Values you want to compare against your own.

So document ingestion has two modes:

*Mode A — Evidence retrieval*: document is chunked and searchable, chunks surface as context during deliberation ("this policy says X, which is relevant to your decision"). Same pipeline as transcripts. `SourceReference.sourceType: "external-document"` covers this.

*Mode B — Record extraction*: document is processed by the Schema + Protocol extraction pipeline, producing SCDS Records from the document content. The document becomes a source for graduation rather than just a retrieval target.

Both modes can use the same document processing backend. The difference is what you do with the output.

**Practical recommendation**

Don't add document management to the SCDS spec itself — the `SourceReference` bridge is sufficient. Instead, treat documents as a third source type in the same infrastructure as transcripts:

```
Conversation layer
  ├── Meeting transcripts    (chunks with AttentionState tags)
  ├── Threaded conversations (chunks anchored to document elements)  
  └── Supporting documents   (chunks with document address metadata)
        ↕ SourceReference
SCDS layer
```

All three land in the same pgvector index with compatible metadata. Context Query doesn't care whether a chunk came from a speech segment or a PDF paragraph — it cares about `recordId`, `fieldId`, and confidence score.

For the implementation: LlamaIndex as the document processing layer, pgvector for storage, Unstructured.io if you're dealing with complex PDFs (tables, scanned documents). The SCDS addressing doesn't change — `sourceType: "external-document"` with a `sourceId` that's the document's stable identifier plus chunk offset. Nothing custom; everything assembled.

The one thing worth adding to the spec is a disambiguation note in Section 6 (Conversation Layer) clarifying that "external-document" source references participate in the same context assembly order as conversation chunks, and that implementations should index document chunks alongside conversation chunks rather than maintaining a separate retrieval index. That's a rationale note, not a schema change.

---

# you asked

message time: 2026-05-25 08:45:38

I can get addressable chunks?

At an implementation later, I will also need to have a reliable, long term data store for docs. Decisions need their supporting documents available for 10 years if they are organisational records.
Not scds, but related

---

# claude response

Yes — addressable chunks are well solved. The key is stable addressing at ingest time: assign a UUID to each chunk when you first process a document, store chunk metadata (document ID, page, section, character offset) alongside the UUID, and put that UUID in `SourceReference.sourceId`. The content doesn't move; the UUID is the permanent handle. Re-indexing for better embeddings doesn't change the UUID — it just updates the vector, not the identity.

The `sourceStandard` field already in the spec is where you'd declare what the `sourceId` format means, so consumers know how to resolve it.

**But the long-term storage question is the more important one.**

This is a records management problem, not a RAG problem. They're completely separate concerns and should be separate systems:

```
Document Archive    ← durability, retention, integrity, legal standing
       ↓
Document Index     ← chunking, embeddings, semantic retrieval (rebuildable)
       ↓
SCDS layer         ← SourceReference points into the archive
```

The index can be rebuilt from the archive at any time. The archive is what cannot be lost.

**What a 10-year document archive actually needs:**

*Format* — PDF/A (ISO 19005), not regular PDF. Archival PDF embeds all fonts and colour profiles and is designed for format longevity. If incoming documents are DOCX or other formats, convert to PDF/A at ingest. This is non-negotiable for genuine organisational records.

*Storage durability* — S3 or equivalent object storage with versioning enabled. S3's durability is 99.999999999% (11 nines). Enable Object Lock for WORM (Write Once Read Many) compliance — documents genuinely cannot be modified or deleted during the retention period. Lifecycle policies move documents to Glacier after active use while preserving them. Cost for 10-year cold storage is negligible.

*Integrity verification* — store SHA-256 hash of the document in the SCDS `SourceReference.note` field or `meta`. Verify on every retrieval. If hash doesn't match, the document has been tampered with.

*Stable addressing* — the `sourceId` in SourceReference should be the document's UUID, not its S3 URL. A separate small registry maps UUID → current storage location. This survives storage migration (if you move from S3 to something else in year 6, the UUIDs in all your SourceReferences remain valid — only the registry mapping changes).

*Immutability proof* — if decisions have genuine legal or governance standing, consider anchoring the document hash to Arweave or a similar permanent ledger at ingest time. You then have cryptographic proof that this document existed, unmodified, at a specific point in time. The Arweave transaction ID goes alongside the hash in `meta`. This was already in your thinking from earlier.

**What this looks like as a service:**

A small document archive service, separate from everything else:

```
POST /documents          → ingest, assign UUID, convert to PDF/A,
                           store in S3 with versioning + WORM,
                           hash, optionally anchor to Arweave,
                           return { documentId, hash, arweaveId? }

GET  /documents/{id}     → retrieve by UUID (resolves to current storage location)

GET  /documents/{id}/verify → returns { hash, matches: true/false }
```

The document index (LlamaIndex / pgvector) sits alongside this, reading from the archive to build its retrieval index. If the index is ever lost or needs rebuilding, you re-ingest from the archive. The archive is the source of truth; the index is derived from it.

**The SCDS SourceReference for a document would look like:**

```typescript
{
  sourceType: "external-document",
  sourceId: "doc-uuid-here",           // stable UUID from the archive
  sourceStandard: "mu-democracy-archive@1.0",  // tells consumers how to resolve
  note: "SHA256:abc123...",            // integrity check
  // arweave tx in meta if anchored
}
```

**What NOT to build:** a custom document management UI, version control for documents (S3 versioning handles this), search within documents (that's the index layer), or access control beyond bucket-level (for now). Those are future concerns.

The archive service is small, boring, and critically important. The RAG sits on top of it and is replaceable. The archive is not replaceable — which is exactly why it should be simple and standard.

---

# you asked

message time: 2026-05-25 08:49:48

Another governance element that is vital in some contexts: attendance. People entering and leaving meetings

---

# claude response

This is a multi-layered governance requirement. Let me unpack the distinct concerns:

**Attendance has two separate problems**

*Recording* — who was present, when they joined, when they left. This is a conversation-layer concern. It's a stream of events, like speech or chat, with timestamps and identity. The canonical transcript standard's stream model already accommodates this — attendance is a third stream type alongside speech and message.

*Validity* — whether a specific decision was valid given who was present at the moment it was made. This is a governance-layer concern. It requires knowing quorum rules for the decision type, matching them against attendance at decision time, and recording the result.

These are different enough that conflating them causes problems.

**Attendance in the conversation layer**

An attendance stream alongside speech and chat:

```
Stream: attendance
  contentType: "presence"
  
  Events:
    join:  { participantId, displayName, role?, timestamp }
    leave: { participantId, timestamp, reason?: "voluntary" | "disconnect" | "ejected" }
```

These events are chunked into addressable units like any other stream. An attendance chunk carries a stable address — so a SCDS Record can carry a SourceReference pointing to "participant X was present from chunk 42 to chunk 89 in this session."

The AttentionState already tracks what's being worked on. Attendance tracks who was there while it was being worked on. They're complementary.

**Quorum in the governance layer**

This is where it gets interesting. Quorum isn't just a headcount — it's a constraint on decision validity. In different governance frameworks:

- Simple majority of registered members
- All named role-holders present
- The proposer must be present for their proposal
- Specific individuals (board members, trustees) must be present regardless of numbers

For μDemocracy, quorum belongs as a field on the governance decision Type and as a validation in the Protocol — specifically in the decision stage's `completionCriteria`. The Protocol can check "were the required participants present when this stage ran?" before allowing the stage to complete.

A `QuorumRecord` as a Type captures this at decision time:

```
Type: org.mu-democracy/quorum_record
  fields:
    participants_present: [participant IDs]  
    quorum_threshold: number or rule expression
    quorum_met: boolean
    participants_required_absent: [participant IDs]  ← people who should have been there
    decision_valid: boolean
```

This Record relates to the Decision via `evidences` — the quorum record is evidence supporting the decision's validity. If the decision is ever challenged, you can traverse to the quorum record and from there to the attendance stream chunks that verify it.

**The governance edge cases that matter**

*Someone leaves before the decision is made* — they were present for context but not the vote. The quorum check needs a time window, not just presence at any point during the meeting. "Present at the moment of decision" is different from "attended the meeting."

*Someone joins after context was discussed* — they consented to the decision without full context. Some governance frameworks treat this as valid; others don't. The Protocol's `completionCriteria` can express this: "all present participants must have been present since stage X."

*Quorum breaks mid-Protocol* — a decision in progress when quorum breaks is a real governance problem. The Protocol stage completion check should verify quorum at the point of completion, not just at the start.

*Proxy attendance* — one person voting on behalf of an absent member. This needs explicit modelling: a `Proxy` record linking the representative to the represented, valid for a specific session or decision scope.

**What this adds to the system**

The conversation layer gains an attendance stream type. The transcript standard (Issue #30) adds presence events alongside speech and message events.

SCDS gains nothing at the spec level — the attendance data flows into Records via normal field values and SourceReferences. But the μDemocracy Type library gains:

```
Type: quorum_record
Type: participant  
Type: proxy_authority
```

And the Decision Protocol gains a quorum-verification step between "evaluation" and "decision" stages — the stage that checks attendance, confirms quorum, records it, and gates the final decision.

The `dependsOn` mechanism in ProtocolStage is exactly right for this: the decision stage depends on the quorum_verification stage, which cannot complete unless quorum is confirmed. The completionCriteria for quorum_verification is "participants_required_present are in the current attendance stream AND quorum_threshold is met."

The democracy coach's job here is to flag in real time when quorum is at risk — "three people have left, you're now at the minimum. If one more person leaves before the decision stage, the decision cannot proceed."

---

# you asked

message time: 2026-05-25 08:53:52

This increases the scope of the conversation stream to a full time stream.

---

# claude response

Exactly. And naming it correctly changes the architecture.

What started as "transcript" was already becoming something broader the moment chat messages joined speech chunks. Attendance events complete the picture. What you're actually building is a **session event stream** — a complete temporal record of everything that happens in a collaborative session, not just what was said.

The event types that belong in a full session stream:

```
speech       — audio-derived text, speaker attribution, timing
message      — chat, threaded conversation  
presence     — join / leave / role change
attention    — AttentionState changes (facilitator advances focus)
stage        — Protocol stage transitions
document     — file shares, document opens
reaction     — hand raises, emoji reactions, status signals
vote         — structured decision input
recording    — consent events, recording start/stop
```

These are all typed, timestamped, identity-attributed events in a single ordered stream. The same addressing model works for all of them: `sessionId / eventId`. A speech chunk, a join event, and a stage transition are all addressable by the same scheme.

**What this means for the canonical stream standard**

The name "canonical transcript standard" is now wrong. It implies speech-and-text. What Issue #30 is actually defining is a **canonical session event stream** — a standard for the complete temporal record of a collaborative session.

The stream types in Issue #30 (equivalent, parallel, derived) still apply — they describe relationships between concurrent capture sources. The merge pipeline still applies. The chunk addressing still applies. But the event vocabulary is broader than "speech" and "message."

**The architectural consequence**

The conversation layer in SCDS should be recharacterised. It's not "where speech content lives" — it's "where session events live." The boundary is temporal, not modal.

```
Session event stream
  speech events        → transcript chunks with AttentionState tags
  presence events      → attendance record for quorum verification
  attention events     → AttentionState change log (auditable)
  stage events         → Protocol run history
  document events      → file share provenance
  
  all addressable as sessionId/eventId
  all referenceable from SCDS via SourceReference
```

**The interesting consequence: full session replayability**

If everything is a typed event in a temporal stream, you can reconstruct the full state of a session at any point in time. At 14:32, who was present, what was being discussed, which Protocol stage was active, what documents were visible, what had just been said. This is essentially event sourcing for collaborative governance.

That's not just useful for provenance — it's the foundation for the democracy coach's ability to say "when this decision was made, quorum was X, the context discussion had been running for Y minutes, and three people had arrived after the success criteria were set."

**The naming shift worth making now**

The GitHub issue and any documentation should rename "canonical transcript" to "canonical session stream" or "session event log." The word "transcript" will constrain thinking about what the standard covers every time someone reads it. The scope is genuinely larger, and the name should reflect it.

---

# you asked

message time: 2026-05-25 09:22:17

I think it may be time to map the spec for the temporal stream now. 

It's ambitious.

A generic, multilayered temporal stream that can capture time stream events, multiple concurrent conversations in multiple formats.

---

# claude response

A full first draft. The structural decisions worth noting:

**The `attentionState` field on every event** is the most important architectural choice. Rather than tagging events retrospectively, every event carries the current SCDS focus context at production time. This makes the context assembly query "all events produced while working on Field F" trivially efficient — it's a single metadata filter, not a join against a separate attention log.

**`StageEvent.participantsPresent`** closes the quorum loop cleanly. You don't need to replay the presence event stream to know who was in the room when a Protocol stage completed — the stage event carries the snapshot.

**Session vs Container** is an explicit boundary. The session is a capture boundary (one video call, one bounded period). The SCDS Container is the semantic boundary (the governance workspace, which may span many sessions). They reference each other but neither absorbs the other.

**Three things this draft deliberately leaves open** pending your review:
- Whether `timingModel: "session-relative"` integers and `timingModel: "wall-clock"` ISO8601 strings in the same `timestamp` field is the right union, or whether they should be separate fields
- Whether `VoteEvent.voteContextId` is enough to group a voting round, or whether a `VoteContext` type is needed
- The async session question — the model handles it but the guidance isn't written yet

---

# you asked

message time: 2026-05-25 09:52:02

Would we benefit from any kind of inheritance, like scds? 
We have lots of context specific elements here, something like quorum and participant tracking points to another identity system

---

# claude response

Both observations point at the same underlying pattern — and yes, TSS benefits from the same inheritance model SCDS has, for the same reasons.

**Event type inheritance**

The core event types in TSS are generic. A `PresenceEvent` knows someone joined; it knows nothing about whether that person's presence satisfies a quorum requirement, what their governance eligibility is, or whether they're attending as a proxy. A `VoteEvent` captures a value; it doesn't know whether the vote is binding, what decision rule applies, or what constitutes a valid ballot.

Domain specialization follows naturally:

```
PresenceEvent (core)
  ↑ extends
org.mu-democracy/governance_presence
  adds: quorumStateAtJoin, eligibilityStatus, proxyForMemberId
  
VoteEvent (core)
  ↑ extends
org.mu-democracy/consent_vote
  adds: hasObjection, concernStatement, proposalId
  ↑ extends
org.mu-democracy/consent_vote_with_amendment
  adds: proposedAmendment

StageEvent (core)
  ↑ extends
org.mu-democracy/governance_stage
  adds: quorumRequirement, quorumMetAtTransition, eligibleVotersPresent
```

The same invariants as SCDS type inheritance apply: specialized events inherit all base fields, may add fields, may not remove them, and remain processable as their base type by systems that don't know the specialization. A generic TSS consumer reading `org.mu-democracy/governance_presence` still sees a valid `PresenceEvent` — the governance fields are unknown extension content it preserves and passes through.

The namespace convention is identical to SCDS. Core event types live in `core/`; domain specializations use `namespace/name` format.

**The identity system is the larger observation**

Participant tracking in TSS already implies a fourth distinct system you haven't named yet. The current spec's `Participant` is doing two different things:

*Session presence* — who was in this session, under what display name, joining when. This belongs in TSS. It's temporal and session-scoped.

*Organizational identity* — who this person is, their membership status, their governance rights (voting eligibility, chairing authority, proxy authority, veto rights). This belongs nowhere in TSS or SCDS. It's a separate identity and membership registry.

The same separation applies that runs through the whole system:

```
Identity Registry    ←  who people are, their stable rights and roles
       ↓ memberId reference
TSS                  ←  who was present in a session and when
       ↓ participantId reference
SCDS                 ←  what decisions they collectively produced
       ↓ SourceReference
Document Archive     ←  the documents that supported those decisions
```

Each system references the one above it; none owns another's concerns.

**What this means for `Participant` in TSS**

The current `Participant` type carries too much. It should be thinned to session-scope only:

```typescript
Participant {
  participantId: UUID       // stable identity reference — resolved against
                            // external Identity Registry
  memberId?: UUID           // the stable cross-session membership ID in the registry
                            // null for guests, observers, system agents
  
  // Session-scope only from here:
  displayName: string       // name as shown during this session
  sessionRole?: string      // role in this specific session
  platformHandle?: string   // their name in the meeting platform
  isRemote?: boolean
}
```

`memberId` is the bridge to the identity system. Everything that was `role` — voting rights, eligibility, proxy authorities — lives in the Identity Registry and is looked up via `memberId` when quorum rules need to be checked.

**What the Identity Registry owns**

This is a fifth component in the overall architecture — not a spec you need to write today, but worth naming:

```
Identity Registry
  Member {
    memberId: UUID
    displayNames: [...]     // names over time
    memberSince: ISO8601
    status: "active" | "inactive" | "suspended"
    
    roles: [...]            // governance roles with effective dates
    votingWeight?: number   // if weighted voting is used
    canProxy?: boolean      // may act as proxy for others
    proxyAuthorities: [{ forMemberId, scope, validUntil }]
  }
```

Quorum rules then say "decision type X requires members with role Y to be present" — checked by matching the Identity Registry's role data against TSS's `PresenceEvent.participantId` → `memberId` chain.

**The practical staging**

For v1 of μDemocracy, `memberId` can just be a UUID you control and store locally — the Identity Registry is a simple members table. You declare the field now; you build the registry later. The same approach you've taken throughout: name the boundary now so it can be respected, implement the full system when it's needed.

The TSS spec should add the event inheritance model alongside a note that `Participant.memberId` references an external identity system whose schema is outside TSS scope — the same pattern as `SourceReference.sourceId` referencing an external document archive.

---

# you asked

message time: 2026-05-25 10:01:50

Identity and presence is something that will take time to evolve. For many transcripts identity is unavailable or potentially inaccurate. Sometimes...like a chat, it's canonical... But the concept of multiple layers of derived conversation leans that even a chat transcript ends with blurry attrubution

---

# claude response

This is one of the most honest design constraints in the whole system. Attribution isn't a property — it's a chain, and every link in that chain introduces uncertainty.

**The spectrum that actually exists**

```
Authenticated     — platform-guaranteed: "account X sent this chat message"
Attributed        — probabilistic: "voice print suggests this is Alice, confidence 0.87"
Inferred          — contextual: "immediately follows Alice's last turn, probably her"
Anonymous         — human but unidentifiable: "someone in the room said this"
Collective        — individual dissolved: "the group discussed..." 
System            — no human origin: AI-generated, transformation output
```

Even "authenticated" has a crack in it — a chat message is attributed to an account, not a person. Someone else may be typing on Alice's account.

**Derivation compounds the blur**

The derivation chain is the real problem. Attribution confidence doesn't just degrade — it can collapse entirely:

```
Audio stream
  speaker: Alice, confidence: 0.91 (voice print)
  
  ↓ Whisper transcript (derived)
  speaker: Alice, confidence: 0.88 (some degradation through ASR)
  
  ↓ AI summary (derived)
  "The group explored three options..."   ← individual attribution gone
  attributionType: collective
  
  ↓ SCDS extraction (derived)
  Record.options_considered = "..."
  sourceRef → transcript chunk
  but who said what: unrecoverable
```

The SCDS Record cites the transcript chunk as its source, but the individual who framed option 2 is invisible in the final record. The chain preserved the *what* but dissolved the *who*. This is fine for context assembly; it's a problem for governance accountability.

**What the spec needs to acknowledge**

Attribution should be a structured type, not a bare UUID:

```typescript
Attribution {
  participantId?: UUID          // resolved identity, when known
  memberId?: UUID               // stable governance identity, when authenticated
  
  confidence: number            // 0.0–1.0
  
  basis: "platform-authenticated"   // chat sender, platform login — highest integrity
        | "voice-print"             // biometric attribution
        | "session-context"         // inferred from conversational flow
        | "manual-review"           // human reviewer confirmed attribution
        | "inferred"                // probabilistic, basis not specified
        | "unknown"                 // human origin but unattributable
  
  isCollective: boolean         // true = individual attribution not possible or meaningful
                                // "the group decided", "a participant asked"
  
  derivedFrom?: UUID            // if attribution was propagated from another event
  attributionDegradedBy?: string // "summarisation", "translation", "aggregation"
}
```

And the derivation loss should be explicit: when a derived stream event has `attributionDegradedBy: "summarisation"`, consumers know the individual-level attribution from the source is not recoverable from this event.

**The governance-critical split**

This actually argues for a clean architectural separation that the spec should make explicit:

*Governance-critical events must come from authenticated channels.* Presence events, vote events, formal role changes — these should never be derived from speech attribution. They must originate from the meeting platform's authenticated participant data, an attendance system, or a facilitator's explicit action. If you're relying on speech recognition to determine who was present, your quorum record has shaky foundations.

*Context-providing events can be probabilistic.* Speech chunks, message threads, reactions — these feed context assembly and understanding. Attribution uncertainty is tolerable because the governance weight of this material is in its content, not its individual provenance.

The spec should say this explicitly, probably in a section on attribution integrity:

> **Attribution integrity tiers**:
> - *Governance tier*: presence, votes, stage transitions. Must originate from authenticated sources. `attribution.basis` must be `"platform-authenticated"` or `"manual-review"`. Should not appear in derived streams.
> - *Context tier*: speech, messages, reactions. May be probabilistic. Governance consumers should treat individual attribution in this tier as indicative, not conclusive.

**The identity system implication**

This is also why the Identity Registry should be a late concern, not an early one. You don't need stable member IDs to build a working system — you need them for governance accountability. And the places where that accountability matters (quorum, voting) are exactly the places where you have authenticated attribution. The places where identity is blurry (speech, summaries) are the places where member IDs wouldn't help anyway.

Start with: authenticated events carry a `platformHandle` and eventually a `memberId` when the registry exists. Probabilistic events carry a `participantId` with `confidence < 1.0`. Collective events carry `isCollective: true`. The system works at every stage of identity maturity; it just becomes more auditable as the registry develops.

**The honest framing for the spec**

The spec should open its identity section with something like: "Attribution in a temporal stream is not binary — it exists on a spectrum from cryptographically authenticated to collectively anonymous. The spec provides vocabulary for the full spectrum; implementations should record attribution at the highest fidelity available and never claim higher confidence than the underlying evidence supports."

That's architecturally honest in a way most systems aren't. Most systems pretend identity is known when it's actually guessed. Making the uncertainty first-class is both more correct and more useful for governance — you can always ask "how confident was the attribution on this quorum record?" rather than discovering the answer was "not very" after the fact.

---

# you asked

message time: 2026-05-25 10:08:22

Identity management is definitely a completely different component of this and we need a hard boundary on our time stream to not attempt it. 

This full system extends in a different direction for governance, it also addresses fiscal governance and uses the same identity to manage payments in  our collaboration environment... So we cannot mess about there

---

# claude response

Clean call. The hard boundary for TSS should be stated explicitly in the spec:

> `participantId` is an opaque reference to an external identity system. TSS assigns no meaning to this identifier beyond event attribution. The rights, roles, membership status, and payment relationships of the entity behind this identifier are out of scope. TSS records that an event was attributed to this UUID; everything else about that UUID is someone else's concern.

That reduces `Participant` in TSS to exactly what TSS needs:

```typescript
Participant {
  participantId: UUID       // foreign key to external identity system — opaque
  displayName: string       // session-scoped display name only
  sessionRole?: string      // role in this session, not in the organisation
  isRemote?: boolean
  platformHandle?: string   // how the platform knows them this session
}
```

Nothing about rights, nothing about voting eligibility, nothing about payment routing. A foreign key with a session-scope display label.

**What's emerging is a five-component architecture**

You've now named all the components:

```
Identity System       ←  who people are; governance rights; payment identity
       ↕ participantId
TSS                   ←  what happened in a session; temporal evidence
       ↕ SourceReference / AttentionState
SCDS                  ←  what it meant; negotiated semantic state
       ↕ documentId
Document Archive      ←  supporting evidence; 10-year retention
       ↕ memberId / financial identity
Fiscal System         ←  financial governance; payments; cooperative economics
```

The Identity System sits at the centre of all four others. Everything references it by `memberId` or `participantId`; none of the four owns it. It carries the highest stakes in the whole architecture — the intersection of governance rights, financial rights, and legal identity means getting it wrong affects actual money and actual accountability.

**The consequence for the other specs**

SCDS already handles this correctly — it has `createdBy?: string` on Relations and `agent` on Revisions, but makes no claims about what those strings mean. TSS needs to follow the same discipline: carry the UUID, say nothing about what it identifies.

The fiscal system is its own spec eventually. When governance decisions involve financial commitments — approved budgets, cooperative distributions, contract authorisations — those Records in SCDS will carry `memberId` references in their field values and SourceReferences pointing to fiscal system records. Same bridge model, different domain.

The identity system is the one component that you genuinely cannot cut corners on and cannot scope-creep the others into covering. Everything else can be rebuilt or migrated. The identity system, once it has financial and governance authority, is load-bearing infrastructure.

---

# you asked

message time: 2026-05-25 10:13:07

Initially, our decision management will deal with identity throughbthe clerk/facilitator. 

12 people around the table in the coop meeting are not going to be signed in. Unless there is a specific reason, no transcript needs to be attributed. The committee makes the decisions collectively. Dissent may be recorded by specific request, but even then, not necessarily named.


In delegation contexts, decision and statement ownership will be very cleanly named. 

Let's not assume that attribution is even desired

---

# claude response

This inverts the spec's current defaults — and it's the right inversion.

The spec currently reads as if attribution is the normal state and collective is the edge case. That's backwards for most real governance. The correct model is:

**Collective is the default. Attribution is deliberate and exceptional.**

**The two distinct contexts**

*In-person collective session*: 12 people around a table. The committee decides. The record says "the committee resolved to..." The clerk notes headcount for quorum. No individual attribution unless explicitly requested. If Alice wants her dissent on the record, she asks for it. Even then she might want it recorded anonymously — "one member dissented." The clerk mediates all of this.

*Digital delegated context*: someone is acting in a named role, making a commitment on behalf of themselves or as a delegate. Chat messages are inherently attributed. A digital vote is attributed. A formal delegation is attributed. Here attribution is inherent and clean.

These aren't just different confidence levels of the same thing — they're different attribution regimes with different defaults.

**What changes in the spec**

`Session.participants` should be optional. The default case has no participant list — just a clerk, a quorum headcount, and a collective record.

Attribution defaults to collective unless explicitly set:

```typescript
Attribution {
  mode: "collective"        // default — the committee, the group, the body
        | "role"            // attributed to a role, not a named individual
                            // e.g. "the Chair ruled that..."
        | "named"           // explicit, deliberate, facilitator-recorded
        | "platform"        // inherent from the digital channel (chat, authenticated vote)
  
  participantId?: UUID      // only present when mode is "named" or "platform"
  roleName?: string         // only present when mode is "role"
  
  recordedBy?: UUID         // the clerk/facilitator who made the attribution decision
                            // absent for platform-guaranteed attribution
}
```

`SpeechEvent` in a collective session doesn't carry a `participantId` at all. It carries `attribution.mode: "collective"`. The content is the group's voice.

**The clerk/facilitator as the attribution mechanism**

This is historically accurate — the company secretary, the minutes clerk, the scribe. Their job is:

- Recording what the body decided (collective)
- Recording who proposed or seconded when procedure requires it (role)
- Recording named dissent when a member specifically requests it (named)
- Certifying that quorum was met without necessarily listing every name

The clerk is a named participant in the session with a specific role. Their attributions are authoritative. The spec should acknowledge this role explicitly.

**The quorum implication**

Even quorum doesn't require named individuals in most cases. "Twelve members present, threshold is seven, quorum met" is valid. The only exception is when specific role-holders must be present — "the Chair and Treasurer must be present for financial decisions" — and even then you're tracking roles, not necessarily individuals.

**What the spec should say**

> Attribution is opt-in. Collective attribution is the default for in-person sessions and requires no participant identity infrastructure. Named attribution is recorded deliberately by the facilitator or clerk. The system must not infer individual attribution from speech patterns, seating position, or other indirect signals unless attribution capture is explicitly configured. Platform-guaranteed attribution (chat, authenticated digital vote) is inherently attributed by the channel.

This is also the correct privacy posture. Twelve people in a room did not necessarily consent to individual attribution of their contributions. The collective voice is what they're offering to the record.

**The simplification this creates for TSS**

The spec gets significantly simpler in its default form. No participant list required. No identity system dependency for the common case. The clerk's judgment mediates everything. The identity system integration only becomes necessary when:

- Digital participation channels are in use
- Specific role delegation needs to be tracked
- Financial governance connects individual identity to decision authority

Start without it. The attribution model accommodates its eventual arrival without requiring it now.

---

# you asked

message time: 2026-05-25 10:15:51

Let's update the spec doc

---

# claude response

Given the changes touch most sections, I'll rewrite the full file cleanly.

The key structural changes:

**Attribution is now a first-class type** (Section 4) with four modes: `collective`, `role`, `named`, `platform`. It's the primary identity mechanism throughout the spec — replacing the flat `participantId` on events.

**Collective is the default at every level.** `Session.participants` is optional. `Stream.defaultAttribution` sets the default for a whole stream. When both are absent, events are treated as collective. Named attribution never appears unless explicitly recorded.

**`count_update` on PresenceEvent** handles collective session attendance changes — "three more members arrived" — without requiring any named participants. Quorum works on headcounts, not name lists.

**VoteEvent has a `collectiveResult`** for show-of-hands and voice votes, separate from individual `value` which only appears with named/platform attribution.

**Invariant 11 and 12** formalise the attribution discipline: named attribution without identity reference requires explicit anonymity invocation; individual attribution must never be inferred from acoustic signals.

**Named attendance is explicitly deferred** to a future extension, making clear it's opt-in rather than a baseline assumption.

The hard identity boundary is stated in Section 1 ("What this specification does not define") and Section 6 (Participant) without ambiguity.

---

# you asked

message time: 2026-05-25 10:54:38

I'm not entirely sure that I participants can get a uuid. 

A clerk will log in. But my meeting may just record a named participant as attending, but it may just be a name. This needs to be able to work without a full identity system

---

# claude response

Right. The spec currently requires a UUID for named attribution, which presupposes an identity system that may not exist. The real spectrum is:

```
Clerk          → logged in, has UUID
Named member   → "Alice Jones" or "The Treasurer" — just a name
Collective     → no identification at all
```

A name is a valid identifier for governance records. Minutes have worked this way for centuries. The UUID is an upgrade when an identity system exists, not a prerequisite.

Two targeted fixes — `Attribution` and `Participant`:

Three precise changes:

**Attribution** now accepts `participantId?: UUID` OR `participantName?: string` OR both. The table shows how this plays out: a clerk ruling uses their UUID; a named member dissent uses their name string; a collective decision uses neither.

**Participant** has `name: string` as the required field and `participantId?: UUID` as optional. The three-stage evolution table makes the upgrade path explicit without making the UUID a requirement.

**Invariant 11** now says named attribution requires at least a `participantId` or a `participantName` — a name is sufficient. Anonymity invocation is still valid with neither.

The practical consequence: the system works on day one with just a clerk login and participant names. Nothing breaks. The UUID becomes available when the identity system arrives, and the spec accommodates it without requiring any migration of existing records.

---

# you asked

message time: 2026-05-25 11:09:57

Areas for Refinement and Edge Cases
There are three specific mechanical gaps in the current draft regarding timing, inheritance, and derivation that need to be addressed.
1. The Event Type Inheritance Missing Link
Location: Section 15 (Event Type Inheritance) & Section 9.1 (Event base) The Issue: Section 15 states that custom event types (e.g., `org.mu-democracy/consent_vote`) extend core types, and that consumers who don't recognize the custom type should "process it as the base type it extends." The Gap: There is no mechanism in the `Event` base schema to tell the consumer what base type is being extended. If I receive `eventType: "org.mu-democracy/consent_vote"`, I don't know if I should fall back to processing it as a `vote` or a `message` unless I have an external schema definition. Resolution: Add a `baseEventType?: string` field to the Event base schema, or require the custom `eventType` string to embed the base type (e.g., `vote:org.mu-democracy/consent_vote`).
2. Interleaving Mixed Timing Models
Location: Section 7 (`timingModel`) & Section 11 (`Canonical view`) The Issue: You allow streams to use either `wall-clock` (ISO8601) or `session-relative` (integer ms) timestamps. Section 11 states that the Canonical view is "ordered by timestamp." The Gap: You cannot directly sort a mix of ISO8601 strings and integers. A consumer trying to interleave a `speech` stream (session-relative) with a `chat` stream (wall-clock) needs to calculate the absolute time for the speech events. Resolution: Add a strict invariant stating: "For cross-stream ordering and View generation, session-relative timestamps must be converted to absolute time by adding the integer milliseconds to `Session.startTime`."
3. Retroactive Derivation and the Session Boundary
Location: Section 5 (`Session.endTime`) & Section 8 (`Stream Relationships`) The Issue: A session has a defined `endTime`. However, a `derived` stream (like an AI summary or a cleaned-up manual transcript) might be generated hours or days after the session concludes. The Gap: Do these retroactively generated events get appended to the original closed `Session` payload? If an AI takes 20 minutes to process a 2-hour meeting, it will emit a new `Stream` of derived events. If `Session.endTime` has already passed, is it legal to append a new `Stream` to that session package? Resolution: Explicitly clarify in Section 8 or 13 that `derived` streams and `MergeRecords` may be appended to a Session Package after `endTime` has passed, provided the `timestamp` of the events within that derived stream logically map to the time bounds of the original session.
The way this specification isolates the chaotic, multi-stream reality of human communication from the strict semantic state of SCDS is fantastic.
Given that `ext:addressability` relies heavily on live `AttentionState` tagging, how do you envision handling asynchronous sessions (like a 3-day threaded discussion)? Will every reply in the thread generate a new `AttentionEvent` to yank the "cursor" to that specific topic, or do async sessions behave differently than live meetings?

---

# claude response

Three clean fixes and a genuinely interesting design question. Let me handle them in order.

The three gaps addressed:

**Gap 1 — Inheritance missing link**: `baseEventType` added to Event base. Invariant 13 makes it required for any custom `namespace/name` eventType and specifies fallback behaviour. `eventType: "org.mu-democracy/consent_vote"` with `baseEventType: "vote"` — unambiguous.

**Gap 2 — Mixed timing interleave**: Invariant 14 states the normalisation formula explicitly (`absoluteTime = Session.startTime + event.timestamp`) and requires it before any cross-stream ordering. The canonical view must use normalised absolute time.

**Gap 3 — Post-session derived streams**: Session Package gains `updatedAt` and `packageVersion`. The clarification section states the rules clearly: derived streams may be appended after endTime, their events must fall within session bounds, original streams are immutable, and the checksum is versioned.

**On the async AttentionState question**: thread-level rather than event-level. One `AttentionEvent` at thread creation; all replies inherit via the `replyToEventId` chain from the root. The critical property is that context assembly at the SCDS layer doesn't need to know which regime applies — the query "all events where attentionState.fieldId = F" returns both live speech chunks and async thread messages identically. The temporal layer absorbs the difference.

---

# you asked

message time: 2026-05-25 11:19:06

ah. here's an anlgle. Non-human actors. 
This is an AI native format, agents and LLMs can interact on this stream too. 

our SCDS allows for annotations, but we need to allow agents to operate here too.  A decision agent will track the stream and could mark potential decisions as events? 
A culture agent could highlight interruptions and aggressive behaviour

---

# claude response

This is a substantial addition and it's architecturally right. Agents are first-class stream participants — they observe, analyse, and annotate in real time or retroactively. The spec needs to accommodate them properly.

Now add the two new event types after VoteEvent:

Now fix the RecordingEvent section number and add the agent invariants:

Good — RecordingEvent is already at 9.12. Now add the agent invariants:

What's now in the spec:

**Attribution `mode: "agent"`** with `agentId`, `agentName`, `modelVersion`, and `requiresHumanReview`. The `requiresHumanReview` flag is the governance gate — agents propose, humans confirm.

**`ObservationEvent`** (9.10) with `observationCategory` in namespace/name format, `observedEventIds`/`observedTimeRange` pointing at the stream evidence, and a `draftScdsInstanceId` bridge for when the agent also creates a SCDS draft. The two worked flows — decision agent and culture agent — are shown explicitly.

**`SuggestionEvent`** (9.11) for directed recommendations to the facilitator, with priority and `directedTo`. The distinction from ObservationEvent: observations describe what the agent noticed; suggestions prescribe what to do.

**`status` on both types** (`proposed → confirmed/dismissed` for observations, `pending → acted_on/dismissed/deferred` for suggestions) enables the human review workflow without requiring a separate review event type.

**Invariants 15–17** close the agent contract: agent attribution requires `agentId`, ObservationEvent and SuggestionEvent are agent-only, and the subtyping invariant is clarified for custom observation categories.

The architecture means agents are fully first-class stream participants: they produce derived streams just like Whisper produces a transcript stream, and their events are addressable, referenceable from SCDS via SourceReference, and queryable through the same attention-state filter that everything else uses.

---

# you asked

message time: 2026-05-25 11:39:43

A Critical Friction Point: Append-Only vs. In-Place Updates
There is a direct mechanical contradiction in the current draft regarding how human reviews of AI events are recorded.
Location: Section 9.10 (`ObservationEvent`) vs. Section 12 (`Addressing`)
The Issue: In Section 12, the specification clearly states a fundamental rule of the temporal stream:
"Events are append-only and never modified."
However, in Section 9.10, the `status` field of an `ObservationEvent` dictates:
"Status is updated in-place on the event... original observation is preserved."
The Gap: You cannot have an append-only, cryptographically checksummed stream of events where individual events are updated in-place. If the clerk reviews an `ObservationEvent` and changes its status from `"proposed"` to `"confirmed"`, mutating the original event payload will break the stream's historical integrity and invalidate the package checksum.
Resolution: To maintain the append-only nature of the TSS, the human review of an agent's observation must be logged as a new temporal event that points back to the original observation. For example, a `ReviewEvent` or a specific `ReactionEvent` (e.g., `reactionType: "observation_confirmed"`, `targetEventId: <ObservationEvent-UUID>`). This accurately models the temporal reality: the agent made the observation at 10:14 AM, and the clerk confirmed it at 10:22 AM.
Given that maintaining an append-only stream is critical for auditability, how would you prefer to model the facilitator's confirmation or dismissal of AI suggestions—would you like to introduce a dedicated `ReviewEvent` to the core types, or overload the existing `ReactionEvent` to handle agent status transitions?

---

# claude response

The contradiction is real and the resolution is clear: `ReviewEvent` as a dedicated type. Here's why overloading `ReactionEvent` is wrong before fixing it.

`ReactionEvent` is a social signal — "hand raise", "thumbs up", "go slower." A clerk's governance confirmation of a potential decision observation is not a reaction. It's a formal act with a reviewer, a decision, and an optional rationale that becomes part of the audit trail. Conflating them would muddy the separation between participation signals and governance actions the spec has maintained throughout.

`ReviewEvent` also applies to both `ObservationEvent` and `SuggestionEvent`, which further argues for a dedicated type over extending either existing type.

Now update the invariants — add `"review"` and `"observation"` and `"suggestion"` to the core type list, and add the ReviewEvent invariant:

The contradiction is resolved. What changed and why:

**`status` removed from ObservationEvent and SuggestionEvent.** Replaced by a derivation note: implied status is `"proposed"`/`"pending"` when no ReviewEvent exists; the most recent ReviewEvent determines current status. Implementations may cache this as a materialized view for query performance — the spec doesn't prohibit that — but the canonical record is the append-only event stream.

**`ReviewEvent` added as 9.13** with `targetEventId`, `targetEventType`, `decision: "accepted" | "dismissed" | "deferred"`, and optional `rationale`. The worked example shows the correct temporal picture: agent observation at 10:14, clerk review at 10:22, two permanent events with correct timestamps.

**Section 12** now explicitly states the append-only rule and names the contradiction it resolves: "Status changes for ObservationEvent and SuggestionEvent are recorded as new ReviewEvent records pointing at the original — never as in-place mutations."

**Invariant 15** adds the downstream action gate: a governance-critical observation must not trigger Protocol advancement or Record creation until a ReviewEvent with `decision: "accepted"` has been recorded. This is the human-in-the-loop guarantee at the spec level.

**Invariant 16** enforces the attribution separation: ObservationEvent and SuggestionEvent are agent-only; ReviewEvent must not carry agent attribution. The boundary between analysis (agents) and decision (humans) is normative.

The `packageVersion` + `checksum` model in Section 13 now also correctly accommodates ReviewEvents as appended events that increment the version rather than mutating existing content.

---

# you asked

message time: 2026-05-26 20:09:14

Scds as a standard. I've designed it originally to support governance. But this is pretty open now. It does however explicitly focus on contexts where meaning evolves and becomes more structured


How would I apply this to contract negotiation... And perhaps even ending up with smart contracts.

---

# claude response

SCDS fits contract negotiation almost perfectly — arguably better than governance, because contracts make the "meaning evolving toward structure" process explicit and formal.

**Why contract negotiation is native SCDS territory**

A contract is not a document. It's an agreed set of semantic commitments between parties, with a document as one of its projections. SCDS's core thesis — "documents are projections of socially negotiated semantic state" — describes contracts precisely.

The negotiation process is exactly the Tier 0 → 2 graduation path:

```
"We want payment on completion" (Note — rough position)
  ↓
payment_term: { trigger: "on_delivery", amount: 50000 } (Typed Record)
  ↓
ContractTerm Record bound to a formal Type with lifecycle (Record — Tier 2)
```

The revision history isn't administrative overhead — it's the provenance trail that answers "what was Party B's original position on this clause and why did they concede?" That's legally significant.

**The Types a contract domain needs**

```
Party              — person or legal entity
ContractTerm       — an individual agreed (or disputed) term
Obligation         — a duty one party owes another
Condition          — a trigger that activates an obligation
Remedy             — what happens if an obligation is breached
ContractMilestone  — a defined point at which obligations trigger
ContractClause     — grouping of related terms (IP, Liability, Payment)
```

These are SCDS Types with their own Fields, AI guidance (for extraction from negotiation conversations), and lifecycle states.

**The lifecycle is richer than governance**

```
draft → proposed → counter-proposed → agreed → executed → active → closed
                                                          ↓
                                                       disputed → resolved | terminated
```

The `counter-proposed` state is what makes negotiation distinctive — a term can be in multiple proposal rounds. Each counter-proposal produces a new Record with a `supersedes` Relation to the prior version, keeping the full negotiation history navigable.

**The Relations contracts need**

Most are already in the spec. One new one:

- `depends-on` — payment obligation depends on delivery milestone
- `supersedes` — new term version replaces prior position
- `refines` — narrowed scope version of a broader clause
- `conflicts-with` — *(new canonical type worth adding)* two active terms cannot coexist; detected at agreement stage

`conflicts-with` is the term that flags when negotiation has produced an internal inconsistency. The AI layer can detect these across related ContractTerm Records and surface them as ObservationEvents in the TSS stream.

**The Protocol for negotiation**

```
Stage 1: Term identification (loose Protocol)
  Brain Dump → what does each party need?
  Decomposition → what are the term categories?

Stage 2: Initial positions (per-term Protocols)
  Each party proposes their version of each ContractTerm
  completionCriteria: "positions recorded for all required terms"

Stage 3: Gap analysis
  Compare positions → identify: agreed, gap, conflict
  ObservationEvent: "conflict detected on IP clause"

Stage 4: Resolution rounds (repeatable)
  Counter-proposal → Revision → new Record with supersedes chain
  completionCriteria: "all conflicts resolved to agreed status"

Stage 5: Agreement + legal review
  All ContractTerms reach lifecycleState: "agreed"
  Annotation thread: lawyer review per clause
  completionCriteria: "no open annotations, all terms agreed"

Stage 6: Execution
  Signature events (in TSS or external signature platform)
  Contract Schema marked complete
```

**The smart contract connection**

This is where the architecture becomes genuinely interesting. A smart contract is a third projection of the same semantic state — alongside the human-readable document and the negotiation record.

```
SCDS ContractTerm Records (agreed semantic state)
  ↓ DocumentView
  Human-readable contract document
  ↓ SmartContractView  ← new projection type
  Executable code (Solidity, Michelson, etc.)
```

A `SmartContractView` is architecturally the same as `DocumentView` but produces code instead of prose. It maps:

```
ContractTerm.amount (number) → uint256
ContractTerm.trigger (select: on_milestone) → function call condition
ContractMilestone.completed (boolean) → require() check
Party.wallet_address (url/string) → payable address
```

The SCDS Records are the source of truth. The smart contract is derived. If terms change — an amendment, an addendum — you update the Records with full provenance, a `supersedes` Relation, and regenerate. The amendment audit trail is free.

**What makes this architecturally clean**

The negotiation conversation lives in TSS as source material. Proposed terms are SCDS Records with SourceReferences back to the conversation. The final agreed terms are Records with complete revision history — every counter-proposal, every concession, every annotation from legal review. The smart contract is a projection of the final agreed state.

Disputes are answerable at every level:
- What was agreed? → SCDS Records, agreed lifecycle state
- Why was it agreed that way? → Revision history + SourceReferences to negotiation conversation
- What did Party B originally propose? → Earlier Record versions via supersedes chain
- Did the smart contract execution match the agreed terms? → Requires integration with blockchain execution data, but the comparison target (SCDS Record) is always available

**What SCDS doesn't replace**

Legal review, digital signature infrastructure, smart contract security audits, regulatory compliance, and the dispute resolution mechanism itself. SCDS provides the semantic record that feeds those processes — it doesn't substitute for them.

**The open question worth sitting with**

At what point does a SCDS ContractTerm Record have enough formal structure to be automatically projected to smart contract code without human translation? The Type definition with `aiGuidance`, `valueType`, and `validationRules` already constrains what a term can contain. If the Types are designed with executable projection in mind — each field mapped to a code equivalent, each condition expressible as a predicate — the projection becomes mechanical rather than interpretive.

That's the path from collaborative negotiation to trustless execution. The human work is in the negotiation and agreement; the machine work is in the projection.

---

# you asked

message time: 2026-05-26 20:17:42

Another context. Scds itself. I have enough to make this self referencing. The scds spec should now begin to maintain it's semantic architecture

---

# claude response

This is the closing of a loop. SCDS describes socially negotiated semantic state where meaning evolves and becomes structured. The spec itself is exactly that — and we've been running an informal Protocol to produce it throughout this conversation.

**What SCDS needs to describe itself**

The spec is a Container of Records. Four Types cover it:

```
Extension          — one Record per extension; status, identifier, dependencies
Invariant          — one Record per invariant; statement, assigned extension, rationale
DesignDecision     — why a choice was made; the decisions that shaped the spec
OpenQuestion       — unresolved questions; lifecycle: open → resolved | deferred
```

With standard lifecycle states, Relations, and Annotations, the spec becomes a navigable semantic graph rather than a static document.

**What the Relations express**

```
ext:views-l2  depends-on  ext:views-l1         ← already in the spec table
Invariant 42  part-of     ext:type-inheritance  ← which extension owns which invariants
DesignDecision "fieldOrder"  supersedes  DesignDecision "overrideFieldOrder"
OpenQuestion "fieldOrder × ext:views-l1 precedence"  derived-from  DesignDecision "fieldAssignmentOverrides"
```

The extension dependency graph becomes machine-navigable. The invariant-to-extension mapping is explicit. Design decisions link to the invariants they produced.

**The document becomes a projection**

The markdown spec file is currently the source of truth. In the self-describing architecture it becomes a DocumentView — a projection from the Records. Invariant 42's statement lives in an Invariant Record; the spec document is generated from it. Changes to the spec are Record-first: update the Record with full provenance, regenerate the document.

This has a practical consequence: internal consistency becomes verifiable. Does every `dependsOn` extension reference in the conformance table correspond to a `depends-on` Relation between Extension Records? Checkable. Does every invariant belong to exactly one extension or core? Checkable.

**This conversation was the Protocol run**

The design decisions made in this conversation are the source material for the initial batch of DesignDecision Records. The key ones that should be captured:

- Why collective attribution is default not named (decision + rationale)
- Why `required` in `fieldAssignmentOverrides` is tightening-only (corrected from the proposal)
- Why `ReviewEvent` rather than overloading `ReactionEvent`
- Why `participantId` is optional and `name` is the primary identifier
- Why `status` was removed from `ObservationEvent` in favour of derived state

Each has a SourceReference back to this conversation via TSS. The rationale document is currently prose — it becomes linked Records.

**The governance of the standard uses its own tools**

Once the spec is self-describing, proposals to change it are SCDS Proposal Records. Reviews are Annotations. Decisions are Decision Records with full provenance. Open questions have lifecycle states. "Why does invariant 42 say what it says?" is a graph traversal, not a prose search.

For an open standard, this matters. Community members proposing changes follow the same Protocol as any other deliberative process. The democracy coach can facilitate spec evolution sessions. The history of why the standard is what it is is permanently auditable — not in a change log, but in a semantic graph where every design choice is linked to the conversation that produced it.

**The bootstrapping sequence**

1. Define the four Types above (Extension, Invariant, DesignDecision, OpenQuestion) in the `core.scds` namespace
2. Define the Schema: the SCDS spec is a Container with these root Types
3. Retrospectively capture design decisions from this conversation
4. Capture the current 44 invariants as Invariant Records
5. Capture the 13 extensions as Extension Records with their dependency Relations
6. Capture the six open questions from the adoptions proposal
7. From here forward: all spec changes are Record-first

**The DesignDecision type is worth noting**

A DesignDecision in the spec context has a specific structure:

```
decision_statement: what was decided
context: what problem or question prompted it
options_considered: what alternatives were evaluated  
rationale: why this option was chosen
supersedes: previous decision or approach it replaces
affects: which Invariants or Extensions this decision shaped
```

This is almost identical to the governance Decision Type the spec already defines. Which is exactly the point — the standard describes decisions, and its own evolution is a sequence of decisions. The meta-architecture and the object architecture are the same.