# ADR-023: `ColumnSpec.isIdentityColumn` marks the Type's effective identity field

- **Status:** accepted
- **Date:** 2026-07-09
- **Supersedes:** —
- **Superseded by:** —
- **Extends:** [ADR-018](018-container-view-column-source-precedence.md) — orthogonal to section/View
  resolution; this ADR only adds a semantic marker to columns ADR-018 has already resolved.
- **Extended by:** [ADR-027](027-identity-column-multi-type-extension.md) — adds common-identity
  resolution for multi-entry `root_type_refs`; the single-entry and absent cases are unchanged.

## Context

RFC-020 (srs#144, accepted) adds `Type.identityFieldId`, a schema-driven pointer to the field a
conformant implementation should treat as a record's identity/display field. `resolve_container_view`
(issue #376, srs-rust) needs to surface this fact on the `ColumnSpec` entries it returns, so that
consuming UIs (the editor's container list view) can render the identity column distinctly — e.g.
as a title/link cell — instead of guessing that `columns[0]` is the meaningful one, which was never
a documented contract (see ADR-018: `resolve_columns` orders columns strictly by the resolved
View's `FieldView.order`, an independently-authored presentational sequence).

Two prior decisions bound the design space here:

- ADR-018 already governs *which* section/View resolves the column set, and states that resolved
  columns are ordered by `FieldView.order` ascending — the View author's presentational order.
- RFC-015 (`ext:views-l2`, accepted) establishes a normative boundary: `precedes` is semantic-only,
  and *presentational* order (which includes View/`FieldView` ordering) is view-owned — no other
  ordering signal may silently override it.

A naive implementation of "default the primary column from `identityFieldId`" by reordering
`columns` so the identity field's column becomes `columns[0]` would silently override the View
author's `FieldView.order`, directly violating the RFC-015 principle above. Marking identity is a
distinct concern from ordering identity.

## Decision

Add `is_identity_column: bool` (serialized `isIdentityColumn`) to `ColumnSpec`
(`crates/srs-repository/src/container_view_service.rs`).

**Correction (pre-implementation review):** an earlier draft of this ADR assumed `resolve_columns`
already resolves — or that ADR-018 already resolves — a single governing Type for the column set.
It does not. Verified against the current code: neither `View`, `DocumentSection`, nor
`SectionSource` carries a `type_id`; `resolve_columns` (`container_view_service.rs:258-312`)
derives columns purely from a View's `field_views`, with no Type lookup anywhere in that path. The
only Type resolution that happens near this code is inside `view_service::document_views_for_container`
(matching the container's *root* record's `type_id`/`type_version` against a `DocumentView`'s
`root_type_refs` to pick which View applies) — that resolved Type is never returned to the caller,
and this resolution is skipped entirely when the caller supplies `input.view_id` explicitly
(`container_view_service.rs:126-135`). There is no existing "the Type" concept to reuse.

Given containers may project heterogeneous member Types (`ext:blueprint`, RFC-008
`typeFilter`/`typeDispatch`), and `DocumentView.root_type_refs: Option<Vec<ExactTypeRef>>` is
explicitly documented as OR-semantics over possibly multiple Types (`view.rs:232-236`), there is no
single correct Type to consult in the general case. Rather than invent a heterogeneous-container
resolution policy this ADR doesn't need to solve, the decision is scoped to the case where the
signal is unambiguous:

- If `dv.root_type_refs` is `Some(refs)` with exactly **one** entry, look up
  `(refs[0].type_id, refs[0].type_version)` in the `identity_field_index` that
  `resolve_container_view` already builds once per call (the same index `record_display_label`
  consumes elsewhere in this service — see `record_label::build_identity_field_index`). If the
  lookup hits, set `is_identity_column: true` on the column whose `field_id` matches, `false` on
  every other column. `resolve_columns` does **not** independently call
  `Package::resolve_type`/`Package::effective_identity_field_id` — doing so would both duplicate
  work the index build already did once for the whole `resolve_container_view` call, and risk a
  second `Package` load.
- In every other case — `root_type_refs` is `None`, empty, or has more than one entry; or the
  index has no entry for that `(type_id, type_version)` key (which covers "Type not found,"
  "Type declares no `identityFieldId`," and "resolution errored," since the index build already
  collapses all three to "no entry," by design — see `build_identity_field_index`) — every
  column's `is_identity_column` is `false`. This is not treated as a validation error or pushed
  as a diagnostic here: "no identity signal available" is a normal, expected outcome (e.g. before
  a repository adopts RFC-020, or for a genuinely multi-Type container), not a malformed-data
  condition. A genuine resolution error (e.g. an inheritance cycle) is surfaced once, non-silently,
  by Rule [N+33] validation — not re-surfaced as a second diagnostic here.

Column order (`FieldView.order`, per ADR-018) is **never** altered by this flag, regardless of
which branch above applies. This is a pure semantic marker layered onto columns ADR-018 has
already resolved and ordered — it carries no ordering information and must not be read as one.

The multi-Type-container case (marking identity columns per heterogeneous member, not per a single
resolved View Type) is explicitly **not** solved by this ADR — see Consequences.

## Consequences

**Positive:** Consumers get an explicit, schema-documented signal for "this column is the record's
identity" without inferring it from position, matching how `record_display_label` (srs-repository)
resolves the same fact for row labels elsewhere. Column order remains fully View-owned, so RFC-015
is not compromised. Both the identity marker and the display-label resolution algorithm (identity
field → name-ladder heuristic → type-name fallback) read from the exact same `identity_field_index`
(itself built from `Package::effective_identity_field_id`), so a UI's title column and its list-row
labels can never disagree about which field is "the" identity field, and the resolution work
happens exactly once per `resolve_container_view` call rather than once per consumer.

**Negative / trade-offs:** This is a new CLI/WASM payload field, but embedded opaquely
(`ContainerViewPayload.container_view` is `#[schemars(with = "serde_json::Value")]`), so no
golden-schema regeneration is required (verified against `payload.rs:522-524`) — narrower than a
typical ADR-011 payload change. A container view whose resolved columns don't happen to include the
identity field will show `isIdentityColumn: false` on every column with no positive signal at all;
consumers wanting to *always* show the identity value may still need to fall back to a separate
`record_display_label`-driven title outside the column set. **Heterogeneous containers get no
signal at all** (every column `false`) even when every individual member's Type has a well-defined
`identityFieldId` — this is the direct cost of scoping to the unambiguous single-Type case rather
than inventing a per-member resolution policy. A follow-up issue should be filed if per-member
identity marking for heterogeneous containers is wanted later; it is out of scope here.

**Neutral:** If a Type declares no `identityFieldId` (the common case until repositories adopt
RFC-020), or a container's View isn't anchored to exactly one Type via `root_type_refs`, every
column's `isIdentityColumn` is `false` — behaviourally identical to today, just with an explicit
`false` instead of an absent concept.
