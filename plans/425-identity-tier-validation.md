# Plan: Tighten I-81 — identityInstanceId must resolve to a com.semanticops.core/purpose Record

## Summary

RFC-013 invariant I-81 currently only checks that `identityInstanceId` is a member of the root container. RFC-018 tightens this: the identity instance MUST resolve to a Tier-2 `com.semanticops.core/purpose` Record. Un-migrated repos (with a Tier-0 note identity) must remain loadable, so the enforcement emits a Warning rather than an Error for the legacy case. This plan adds that validation to `validation.rs`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | `crates/srs-repository/src/validation.rs` |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Validation logic belongs in `srs-repository`, not the CLI | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | No business logic in CLI handlers | accepted |

No new ADRs required — this plan implements an accepted RFC-018 invariant within the existing validation pattern. The Warning-vs-Error choice for Tier-0 notes follows the established precedent of I-63 and I-64 (advisory warnings for transitional states, see validation.rs lines 624–699).

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. The validation diagnostics already live in `RepositoryValidationReport.diagnostics` — this plan adds new `ValidationDiagnostic` entries to that existing struct with no shape change. No action required; golden schemas stay as-is.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files under `srs/docs/schema/2.0/`. No action required.

---

## Scope

- Add RFC-018 I-81 type check to `validate_repository` in `crates/srs-repository/src/validation.rs`, immediately after the existing I-81 membership block (lines ~209–230).
- Emit `Warning` when `identityInstanceId` resolves to a Tier-0 note (legacy un-migrated repo).
- Emit `Warning` when `identityInstanceId` resolves to a Tier-2 record that is not `com.semanticops.core/purpose`.
- No diagnostic when identity is correctly a Tier-2 `com.semanticops.core/purpose` record.
- Add three targeted unit tests covering the three cases above.

**Out of scope:**

- The `repository_navigation_service.rs` transitional grace (issue #427 — that's the navigation side; this plan covers validation only).
- Core-type registry implementation (issue #423 — the type-check here uses `typeNamespace`/`typeName` string fields directly, not type resolution).
- Migration commands (issue #426).
- Changing I-81 from Warning to Error once migration is complete — that is a follow-up once all repos are migrated.

---

## Phases

### Phase 1: Add RFC-018 I-81 type check + tests

**Goal:** `validate_repository` emits a Warning when `identityInstanceId` resolves to a non-purpose identity, while un-migrated (Tier-0 note) repos stay valid.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `crates/srs-repository/src/validation.rs`, locate the I-81 membership check block (the `if let Some(ref identity_id) = root.identity_instance_id` block inside the `Some(root) =>` arm of `match manifest.container.as_ref()`).
- [x] Immediately after that block (before the closing brace of the `if let Some(ref full_container)` block), add the RFC-018 I-81 extension check:
  1. Find the `InstanceIndexEntry` for `identity_id` in `manifest.instance_index`.
  2. If `entry.tier() == 0` → push `Warning` with message `"RFC-018 I-81: identityInstanceId '<id>' resolves to a Tier-0 Note; must be migrated to a com.semanticops.core/purpose Record"`.
  3. If `entry.tier() == 2` → call `store.load_instance_json(entry.path())`:
     - On success: extract `typeNamespace` and `typeName` from the JSON value. If not `com.semanticops.core` / `purpose` → push `Warning` with message `"RFC-018 I-81: identityInstanceId '<id>' resolves to type '<ns>/<name>' but must be com.semanticops.core/purpose"`.
     - On load error → push `Warning` with message `"RFC-018 I-81: could not load identity instance '<id>' to verify type: <error>"`.
  4. Other tiers (none expected): skip without diagnostic.
  5. If the identity_id is not found in index: skip (the membership check above already emits an Error).
- [x] Add three unit tests in `#[cfg(test)] mod tests`:
  - `identity_tier0_note_emits_rfc018_warning`: repo with identity = Tier-0 note → assert exactly one Warning containing "RFC-018 I-81" and "Tier-0 Note"; assert `report.is_ok()` (warnings don't fail the report).
  - `identity_tier2_wrong_type_emits_rfc018_warning`: repo with identity = Tier-2 record of type `com.test`/`guide` → assert Warning containing "RFC-018 I-81" and "com.test/guide".
  - `identity_tier2_purpose_type_no_rfc018_diagnostic`: repo with identity = Tier-2 record of type `com.semanticops.core`/`purpose` → assert no Warning or Error containing "RFC-018 I-81".

#### Acceptance Criteria

- [x] A repo with `identityInstanceId` pointing to a Tier-0 note emits exactly one `Warning` matching "RFC-018 I-81" and does not emit an `Error` on that check.
- [x] A repo with `identityInstanceId` pointing to a Tier-2 record of wrong type emits exactly one `Warning` matching "RFC-018 I-81".
- [x] A repo with `identityInstanceId` pointing to a Tier-2 `com.semanticops.core/purpose` record emits no RFC-018 I-81 diagnostic.
- [x] `report.is_ok()` remains true for un-migrated repos (the Warning does not count as an error).
- [x] The existing `live_srs_repo_validates_cleanly` test still passes (that repo has a Tier-0 note identity; the Warning is acceptable since `is_ok()` only checks errors).
- [x] All pre-existing tests still pass.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:

- `identity_tier0_note_emits_rfc018_warning` — proves Warning is emitted for Tier-0 note identity and repo stays `is_ok()`
- `identity_tier2_wrong_type_emits_rfc018_warning` — proves Warning for wrong Tier-2 type
- `identity_tier2_purpose_type_no_rfc018_diagnostic` — proves no diagnostic for the correct case

#### Milestone gate

1. All three new tests pass.
2. All pre-existing tests in `srs-repository` pass.
3. Clippy clean.
4. Mark checkboxes `[x]` above.
5. Commit.

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] CLI output format unchanged (integration tests pass)
- [x] `cargo test --test payload_contracts` passes (no payload structs changed)
- [x] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [x] `live_srs_repo_validates_cleanly` still passes
- [x] New RFC-018 I-81 Warning appears in `srs repo validate` output for a repo with a Tier-0 note identity

## Coordination Rules

- All changes are confined to `crates/srs-repository/src/validation.rs`.
- No payload or schema changes; no `cargo run --bin generate-schemas` required.

## Assumptions

- The `com.semanticops.core` namespace and `purpose` type name are the correct identifiers as described in RFC-018 and issue #135 (srs repo).
- Type checking by `typeNamespace`/`typeName` string fields from the loaded JSON is sufficient and does not require the core package registry (issue #423) to be complete.
- The live `srs/srs` spec repo currently uses a Tier-0 note identity (not yet migrated per issue #426), so `live_srs_repo_validates_cleanly` will receive a Warning — which is acceptable since that test only asserts `is_ok()`.
