# Plan: Resolve ADR-023 numbering collision

## Summary

Two ADR files both claim number 023: `023-columnspec-identity-column-marker.md` (merged 2026-07-09
17:04, PR #456, ~15 cross-references) and `023-type-schema-field-help-keys.md` (merged 2026-07-09
15:05, PR #452, 3 references). This plan renumbers the lower-reference-count file to ADR-026 (the
next free slot after `025-implicit-core-package-merge.md`) and updates its self-reference plus every
place that cites it, leaving the higher-traffic `023-columnspec-identity-column-marker.md` untouched.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | (this session) |
| Verification | (this session) |

## Architecture Decisions

No new architectural decisions — this plan is a pure documentation renumbering fix. It does not
change ADR-023 (`columnspec-identity-column-marker`)'s content or ADR-026
(`type-schema-field-help-keys`, formerly numbered 023)'s content, only its filename and number.

## Contracts

### CLI output contract (ADR-011)

No new/changed commands — no action required.

### Entity schema sync

No JSON Schema files touched — no action required.

## Scope

1. `git mv docs/adr/023-type-schema-field-help-keys.md docs/adr/026-type-schema-field-help-keys.md`
2. Update the renamed file's header (`# ADR-023: ...` → `# ADR-026: ...`) and any other internal
   self-references to "023"/"ADR-023" within that file.
3. Update `crates/srs-repository/src/type_schema_service.rs:202` comment: `See ADR-023.` → `See ADR-026.`
4. Update `plans/type-schema-field-help.md`: all `ADR-023` references (lines ~13, 31, 32, 34)
   including the markdown link `[ADR-023](../docs/adr/023-type-schema-field-help-keys.md)` →
   `[ADR-026](../docs/adr/026-type-schema-field-help-keys.md)`.
5. Do NOT modify `docs/adr/023-columnspec-identity-column-marker.md` or any of its referrers
   (`container_view_service.rs`, `docs/adr/018-...md`, `plans/376-...md`, `docs/dogfooding.md`).

## Out of scope

- The process gap that allowed two PRs to claim the same ADR number without checking — noted in
  the architecture review as a retro item, not a code fix.
- Any content changes to either ADR beyond the number/filename.

## Acceptance Criteria

- [ ] `docs/adr/026-type-schema-field-help-keys.md` exists; `023-type-schema-field-help-keys.md` no longer does.
- [ ] `grep -rn "ADR-023" --include="*.md" --include="*.rs" .` returns only references to columnspec-identity-column-marker.
- [ ] `grep -rn "ADR-026" --include="*.md" --include="*.rs" .` returns exactly the renumbered file's self-references plus the two updated referrer locations.
- [ ] `cargo test -p srs-repository` and `cargo clippy -- -D warnings` pass (no code logic touched, but confirm no breakage).

## Final Acceptance

```bash
cargo test
cargo clippy -- -D warnings
```
