# ADR-018: Column-source precedence for container-view projections

- **Status:** accepted
- **Date:** 2026-06-28
- **Supersedes:** —
- **Superseded by:** —

## Context

The `resolve_container_view` service (issue #254) projects a container's members into a
column/field spec for an interactive editor list. A `DocumentView` has no direct list of
columns; columns must be derived from a section's `render_view_id → View.field_views`. But a
`DocumentView` can carry **multiple** `DocumentSection`s, each with its own (optional)
`render_view_id`. The service must pick exactly one View to drive the member-list columns, and
that choice is a **semantic contract**: any future capability that consults a DocumentView to
drive a container-scoped UI (a second list projection, a grid export, a graph side-panel) must
make the *same* choice, or two clients will disagree about which columns a container's members
show — the exact divergence `docs/architecture/capability-layering.md` exists to prevent.

The choice cannot be left as an undocumented implementation detail inside one service, because
the rule is reusable and re-implementable. Plausible alternatives a future author might reach
for: (a) just use the first section; (b) merge/union the field_views of every section. Both
silently diverge from this service's behaviour.

## Decision

For a `(container, DocumentView)` pair, the columns are resolved from a single section chosen by
this precedence:

1. The section whose `source` is `SectionSource::ContainerSubset { container_id, .. }` equal to
   the requested container's id **and** whose `render_view_id` is `Some`. (The section that
   explicitly targets *this* container wins.)
2. Otherwise, the first section — ordered by `DocumentSection.order` ascending (lowest first) —
   whose `render_view_id` is `Some`.
3. Otherwise, no View resolves and the column spec is **empty** (the projection still returns
   the root and ordered members).

The resolved View's `field_views` become the columns: entries with `visible == Some(false)` are
excluded, the rest are ordered by `FieldView.order` ascending, and each column's display label is
`FieldView.display_label` when set, else the field's `name`.

Any future capability that derives columns from a DocumentView + container pair MUST use this
same precedence (or supersede this ADR).

## Consequences

**Positive:** One definition of "which section drives the columns," shared by every consumer —
CLI, WASM binding, and future projections agree. The container-targeting section taking priority
matches how `render_service` already dispatches per-section views, so behaviour is consistent
with rendering.

**Negative / trade-offs:** A DocumentView that intends several distinct column sets per section
cannot express more than one column set through this single-section projection; such a need
would require a richer multi-section result shape and would supersede this ADR. Merging field
sets across sections is explicitly rejected (ambiguous ordering, columns irrelevant to the
container).

**Neutral:** The precedence is computed over the DocumentView's declared sections only; it does
not consult lifecycle state, tags, or Layer-2 accelerators. It is a pure Layer-1 function of the
persisted DocumentView, View, and Container.
