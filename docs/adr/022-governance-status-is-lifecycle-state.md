# ADR-022: Governance status is SRS lifecycle state

- **Status:** accepted
- **Date:** 2026-07-07
- **Supersedes:** —
- **Superseded by:** —

## Context

The governance editor (`srs-web`) tracks decision state (draft, active, superseded, abandoned) as
a "status" field. Historically the transition graph was hardcoded as a TypeScript table
(`lifecycle.ts`: `LIFECYCLE_TRANSITIONS`, `IMMUTABLE_STATES`) and transitions were applied by
writing a raw field value via `updateRecord`, using a hardcoded field UUID (`STATUS_FIELD_ID`).

The SRS core provides a first-class `ext:lifecycle` extension that models exactly this: named
states, transition rules, immutability constraints, and a `set_lifecycle_state` service that
validates transitions before writing. The `set_lifecycle_state` WASM binding was already exported
from `srs-bindings` but unused by the governance editor.

The open question: is governance `status` the same thing as SRS lifecycle state (`ext:lifecycle`),
or is it an ordinary field that happens to have status-like semantics?

## Decision

Governance `status` **is** SRS lifecycle state. The two concepts are not analogous or parallel —
they are identical. A governance decision's "status" is its `ext:lifecycle` state, managed
through the `set_lifecycle_state` service and its WASM binding.

There is no separate "status" concept in the governance layer. Code that reads or writes
governance status reads or writes lifecycle state. Field writes that bypass the lifecycle service
are incorrect regardless of which field UUID they target.

## Consequences

**Positive:**
- Transition validation is enforced by the Rust core, not client-side TypeScript — the transition
  graph cannot diverge between the web editor, the CLI (`srs-gov`), and any future consumer.
- The allowed-transitions service (`get_allowed_lifecycle_transitions`, srs-rust#375) directly
  answers "what states can this decision move to next?" for the governance editor — no TS table
  required.
- Governance files written today remain valid as the tooling evolves: the data model is the
  authoritative transition graph, not a hardcoded constant.
- The TypeScript transition tables (`LIFECYCLE_TRANSITIONS`, `IMMUTABLE_STATES`) and the
  hardcoded `STATUS_FIELD_ID` constant are deleted entirely (srs-web ADR-001).

**Negative / trade-offs:**
- Record types intended as governance decisions must carry the `ext:lifecycle` extension. Types
  that do not declare `ext:lifecycle` cannot participate in governed status transitions.

**Neutral:**
- The hidden-status filter (srs-web#118) and immutability gating (srs-web#86) read lifecycle
  state, not a field value — their queries route through the lifecycle service, not `find` with
  a field-value predicate.
- `get_allowed_lifecycle_transitions` and `set_lifecycle_state` are the only correct query and
  write paths for governance status.
