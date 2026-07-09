# ADR-023: Type-Schema Field Help Vendor Keys

- **Status:** accepted
- **Date:** 2026-07-09
- **Supersedes:** —
- **Superseded by:** —

## Context

The `type schema` projection (`type_schema_service::field_to_property`,
`crates/srs-repository/src/type_schema_service.rs`) emits a draft-07 JSON Schema property object per
field so a schema-driven editor (srs-web) can render an edit form. Each field carries three distinct
prose texts, and the projection needs to convey them without collision:

1. **Display label** — what the label reads. Already projected as `title` (the FieldAssignment
   `displayLabel`, falling back to the field's `description` when no label is set).
2. **`description`** — the field's own short caption ("Why this option over the alternatives.").
3. **`aiGuidance`** — machine-facing extraction guidance. A *string* `aiGuidance` is already
   projected into the JSON Schema `description` keyword; a *structured* `aiGuidance` goes to
   `x-srs-ai-guidance`.
4. **`instructions`** — fuller human "how to complete this field" guidance (SRS field schema,
   optional). Previously not projected at all.

The JSON Schema `title` and `description` keywords are therefore already spoken for (`title` = label,
`description` = string aiGuidance). Surfacing the field's own `description` and `instructions` as
human help in the editor requires a home that does not collide with either.

## Decision

The type-schema field property carries two **vendor extension keys**:

- **`x-srs-description`** ← the field's `description`, when non-empty.
- **`x-srs-instructions`** ← the field's `instructions`, when present and non-empty.

Both are emitted by `field_to_property`, so they apply uniformly to standalone fields and to the
sub-fields of a field group (ext:field-groups / RFC-007), which resolve through the same function.

This follows the semantic-role vendor-key convention established by **ADR-014**
(`x-srs-ordered-by`): a dedicated `x-srs-*` key names the datum's role rather than overloading a
standard JSON Schema keyword. Editors read `title` for the label, `x-srs-description` for the short
caption, `x-srs-instructions` for the fuller human help, and `x-srs-ai-guidance` for AI guidance —
four unambiguous slots.

The keys are additive and optional: absent when the source text is empty. The `type schema` payload
(`TypeSchemaPayload`) is an opaque `serde_json::Value`, so this change does not alter any committed
payload golden.

## Consequences

**Positive:**
- The editor can present a field's own `description` and `instructions` as distinct help without
  guessing, and without the label-fallback overloading that entangles `description` with `title`.
- Consistent with ADR-014; no new naming philosophy introduced.
- Backward compatible — consumers that ignore unknown `x-srs-*` keys are unaffected.

**Negative / trade-offs:**
- srs-web now depends on these key names as a projection contract. They are additive and controlled
  within the ecosystem; renaming later is a coordinated two-repo change (acceptable, and the reason
  this ADR records the choice).

**Neutral:**
- Governs only the `type schema` projection. Field `description`/`instructions` remain their normal
  selves everywhere else in the data model.
- Group-level `title`/`description` (a field group's own label/description) are unchanged; this ADR
  concerns per-field help text.
