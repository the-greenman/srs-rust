# ADR-014: Composite Schema Property Naming

- **Status:** accepted
- **Date:** 2026-06-07
- **Supersedes:** —
- **Superseded by:** —

## Context

The `blueprint schema` command (issue #61) emits a nested draft-07 JSON Schema that describes a
whole multi-record document. The schema's top-level `properties` object contains one entry for each
child-record collection, keyed by the Blueprint's `relationType` string (e.g. `"section-sequence"`,
`"contains-appendix"`).

JSON Schema property names must be valid identifiers in most code-generation toolchains
(TypeScript, Python dataclasses, JSON-LD framing). Relation types in SRS use kebab-case
(`"section-sequence"`) because they are semantic labels, not code identifiers. Using them verbatim
as property names would produce invalid identifiers in all major target languages.

Three alternatives were considered:

| Option | Example | Trade-off |
|--------|---------|-----------|
| Verbatim kebab-case | `"section-sequence"` | Invalid identifier in TS/Python/Java |
| snake_case | `"section_sequence"` | Valid identifier, but foreign to JS/JSON convention |
| lowerCamelCase | `"sectionSequence"` | Valid identifier, follows JSON/JS convention |

## Decision

Child-collection property keys in the composite schema use **lowerCamelCase** conversion of the
`relationType` string. Each word boundary in the kebab-case input is capitalised; the first word
is lower-cased.

Examples:
- `"section-sequence"` → `"sectionSequence"`
- `"contains-appendix"` → `"containsAppendix"`
- `"contains"` → `"contains"` (single word, unchanged)

The conversion is implemented in `blueprint_schema_service::relation_type_to_property_key` in
`crates/srs-repository/src/blueprint_schema_service.rs`.

The `root` property (representing the Blueprint's entry-point type(s)) is a fixed reserved key and
is not subject to this conversion.

## Consequences

**Positive:**
- Generated schemas are immediately usable for TypeScript interface generation, Python dataclass
  derivation, and similar tooling without post-processing.
- lowerCamelCase is the dominant convention in JSON APIs, so the output feels native.
- The mapping is deterministic and lossless — given a property key, the original `relationType`
  can be recovered by reversing the camelCase split.

**Negative / trade-offs:**
- A consumer inspecting the schema must know to apply the inverse transform when correlating a
  property key back to an SRS relation type. The `x-srs-ordered-by` extension field on each
  child-collection array property records the original relation type to make this mechanical.
  (`x-srs-ordered-by` was preferred over `x-srs-relation-type` because it communicates the
  semantic role — "this relation type governs ordering" — rather than just naming the raw data.)
- Two different relation types that normalise to the same camelCase key (e.g. a hypothetical
  `"sectionSequence"` and `"section-sequence"`) would collide. SRS relation types are expected to
  be distinct and non-ambiguous; this edge case is noted as a validation concern for future work.

**Neutral:**
- The `root` reserved key is in lowerCamelCase by convention but is not derived from a
  relation type.
- This decision governs only the `blueprint schema` projection. Relation type identifiers
  themselves remain kebab-case everywhere else in the SRS data model.
