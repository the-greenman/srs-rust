# ADR-020: `resolve-view` surfaces a container's authored list defaults

- **Status:** proposed
- **Date:** 2026-06-28
- **Supersedes:** —
- **Superseded by:** —

## Context

An authored decision-log `DocumentView` declares default-hidden lifecycle states on its section
source (`SectionSource::TypeQuery { exclude_lifecycle_states, .. }`, RFC-012 Rev 7) — e.g.
`["superseded", "closed"]`. An interactive client (`srs-gov list`, and later the web/TUI) must apply
those defaults when listing a container's members, with a runtime "show all" override.

The client needs two things to render the list: the **columns + ordered members** (already provided
by `container resolve-view`, ADR-018) and the **authored default-hidden lifecycle states** to feed
into a `discovery_service::find` query. The exclusion list is *not* exposed today: a client would
have to call `document-view get <id>`, then re-implement the ADR-018 section-selection precedence to
pick which section's source applies, then read `exclude_lifecycle_states` itself. That re-derivation
is exactly the leaf-client-semantics divergence `docs/architecture/capability-layering.md` and
ADR-018 exist to prevent: two clients could disagree about which section governs, and therefore which
states are hidden.

## Decision

`container resolve-view` is the single surface that carries a container's authored **list defaults**.
Its `ContainerView` payload exposes `exclude_lifecycle_states: Vec<String>` alongside `columns`. Both
are sourced from the **same** governing `DocumentSection` selected by ADR-018's precedence (the
section that targets this container, else the first by `order` with a `render_view_id`); the
exclusion list is that section's `SectionSource::TypeQuery { exclude_lifecycle_states }` (or `[]` for
any non-`type-query` source or absent list). ADR-018's "targets this container" test is extended to
also recognise a `type-query` whose `container_ids` includes the container, since the canonical
decision-log section is now a `type-query` rather than a `container-subset`.

Clients consume `containerView.excludeLifecycleStates` and forward it to `find`
(`--exclude-lifecycle-state` per state), dropping it under a "show all" toggle. Clients MUST NOT
re-derive the governing section or read `DocumentView` sources directly to obtain list defaults.

## Consequences

**Positive:** One definition of "the container's authored list defaults," shared by CLI, WASM, and
future clients — consistent with ADR-018's single-section column rule and the capability-layering
guide. `srs-gov list` (and the TUI/web) stay thin: resolve-view → forward defaults → `find`.

**Negative / trade-offs:** `ContainerView` couples columns and lifecycle defaults to one governing
section. A DocumentView intending distinct per-section query defaults cannot express more than one
through this projection — the same single-section limitation ADR-018 already accepts; a richer
multi-section result would supersede both ADRs. Adding the field is an additive CLI payload-contract
change (golden schema regenerated, ADR-011).

**Neutral:** The field is a pure Layer-1 function of the persisted DocumentView/Container — it does
not itself run a query or consult lifecycle state of members; applying the exclusion is the client's
`find` call (ADR-019). Authored *searchable-fields* / *default-sort* defaults are out of scope here
(RFC #213); when added, they extend this same surface.
