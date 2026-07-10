# ADR-027: `isIdentityColumn` common-identity extension for multi-entry `root_type_refs`

- **Status:** accepted
- **Date:** 2026-07-10
- **Supersedes:** —
- **Superseded by:** —
- **Extends:** [ADR-023](023-columnspec-identity-column-marker.md) — adds a resolution rule for the multi-entry `root_type_refs` case ADR-023 explicitly deferred.

## Context

ADR-023 scoped `ColumnSpec.isIdentityColumn` to the unambiguous single-Type case: when `DocumentView.root_type_refs` has exactly one entry, the column matching that Type's effective `identityFieldId` is marked `true`; every other case yields `false` on all columns. This was a deliberate scope cut — quoted from ADR-023 Consequences:

> **Heterogeneous containers get no signal at all** (every column `false`) even when every individual member's Type has a well-defined `identityFieldId` — this is the direct cost of scoping to the unambiguous single-Type case rather than inventing a per-member resolution policy. A follow-up issue should be filed if per-member identity marking for heterogeneous containers is wanted later; it is out of scope here.

Issue #454 revisits this cut. The problem: a container may list multiple Types via `root_type_refs` (e.g. a blueprint-backed container with two related Types), yet every one of those Types may designate the same field as its identity field. In that case, the column-level signal remains perfectly unambiguous — the identity column is `f-title` regardless of which Type each row happens to be. ADR-023's blanket "multi-entry → all false" rule suppresses a valid, derivable signal.

The multi-Type-aware per-row alternative (adding `identity_field_id` to `ResolvedMember`) is deferred as a follow-up: it changes the `ResolvedMember` public API shape and no concrete consumer need for it has surfaced yet.

## Decision

Extend `resolve_columns` in `container_view_service.rs` with a **common-identity** rule:

- If `dv.root_type_refs` is `Some(refs)` with **more than one entry**, look up each entry's effective `identityFieldId` from the pre-built `identity_field_index`. If **all entries** resolve to the **same field ID** (including handling the case where any entry is absent from the index — that counts as "absent" / disagree), mark that column `is_identity_column: true`, all others `false`. This is the common-identity case: every referenced Type agrees on the same identity field, so the column-level signal is still unambiguous.

- If any Type in the multi-entry list is absent from the `identity_field_index` (no `identityFieldId` set, or resolution errored during index build) or disagrees with the others, `is_identity_column: false` on every column (same as ADR-023's behavior for ambiguous cases).

- The single-entry case from ADR-023 is unchanged: `Some([single]) → look up that one Type`.
- Absent or empty `root_type_refs` is unchanged: `None` on every column.

The implementation: extract a private helper `common_identity_field(dv, identity_field_index) -> Option<&String>` that encapsulates both the single-entry and multi-entry paths, and call it from `resolve_columns`. No second `Package` load; no independent call to `Package::effective_identity_field_id` — the same `identity_field_index` `resolve_container_view` already builds once is passed in.

Column order remains View-owned (per ADR-023, RFC-015). `is_identity_column` is never an ordering signal.

## Consequences

**Positive:**
- Containers with multi-entry `root_type_refs` where all Types share an identity field now get the expected column-level signal. The fix is minimal and contained within the existing `resolve_columns` function.
- No new indexes, no new Package loads, no new service boundaries. The `common_identity_field` helper is a pure function over already-available data.
- The existing "truly ambiguous" behavior (types that disagree, or types with no identity field) is preserved — no risk of a false-positive identity signal.

**Negative / trade-offs:**
- The per-row case (Types that each have a different identity field) remains unsolved at the column level — `isIdentityColumn: false` still. The follow-up issue tracks adding `identity_field_id: Option<String>` to `ResolvedMember` if a consumer need arises.
- The "all agree" rule can produce a false positive in theory: if Types A and B both list field `f-title` as their identity field but that field has no semantic relationship across types (e.g., a naming coincidence). In practice, `root_type_refs` entries are package-level design decisions and this scenario is considered unlikely enough to accept without a guard.

**Neutral:**
- The existing unit test `resolve_container_view_ambiguous_root_type_refs_all_columns_false` is renamed to `resolve_container_view_disagreeing_root_type_refs_all_columns_false` — the scenario (two entries where one has no identity field) still results in all-false, but the name now accurately describes the condition.
- ADR-023 continues to govern the overall `isIdentityColumn` concept; this ADR extends it for the multi-entry path only.
