# Plan: RFC-032 Revision 7 conformance predicates (I-120, I-94, `[T-9]`, `[N+1]`)

## Summary

`srs` PR #334 re-expressed four legacy `valueType`-subset conformance rules over RFC-032 `fieldType`
facets, as an **RFC-032 Revision 7 post-acceptance erratum**. `srs-rust` `master` implements none of
the four: `is_text_searchable` over-includes `format: "uuid"`, and `[T-9]`, `[N+1]` and I-94 perform
no eligibility check at all. This plan implements all four as named, testable predicates and widens
`validate_cross_field_rules` to carry the type information they need. It is a prerequisite for
srs#242 Phase B, whose Change-I removal gate condition 4 presupposes conformant implementations
exist to switch.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | this session |
| srs-core Worker | this session |
| srs-repository Worker | this session |
| Verification | Stage 7 review agents (`architecture-reviewer`, `verification-agent`) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs. This plan implements an accepted upstream spec erratum and respects existing
boundaries: predicates are canonical semantics and therefore live in `srs-core`
(ADR-001 library-first); the two render call sites consume them from `srs-repository` without
restating the rules (ADR-010 service boundary). No CLI surface changes, so ADR-011 is untouched.

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Canonical semantics live in the library, not the leaf client | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Business logic lives in `srs-repository`/`srs-core`, not handlers | accepted |

**Owner decision carried from the dispatch brief (not re-litigated):** the four predicates are
settled as of srs#284, 2026-08-01. Widening `validate_cross_field_rules`' signature is mandated by
the brief — `Datatype` alone cannot express `effective-single`, `format` or `valueDomain`.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. `crates/srs-cli/src/payload.rs` is untouched, so no
schema regeneration is required. `cargo test --test payload_contracts` is still run as a gate.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files are modified. `crates/srs-schema/schemas/2.0/` is a mirror resynced by PR #791
and is explicitly out of bounds for this issue.

---

## Scope

- Four predicates as named functions on `FieldType` in `crates/srs-core/src/types/field_type.rs`.
- A shared `effective-single` helper spanning **both** cardinality mechanisms.
- Widen `validate_cross_field_rules` from `HashMap<String, Datatype>` to a map carrying the full
  `FieldType` plus the effective `FieldAssignment.repeatable`, and update all five call sites.
- Apply `[T-9]` in `css_classes_for_record` and `[N+1]` in `resolve_heading_field_id`.
- Rewrite `is_text_searchable`'s doc comment.
- Invert `title_field_id_emits_record_heading`.
- One constructed **negative** test per predicate.

**Out of scope:**

- Dropping the legacy `FieldAssignment.repeatable` conjunct (CC-26 — removed only inside the atomic
  srs#242 Phase-B train, behind five evidenced conditions).
- Composite recursion for `ref`/`dependent`/`map` in Text Projection (CC-27 — undefined by the
  erratum, knowingly; that is RFC work).
- The `SRS_RUST_CLI_TAG` bump in `srs` (CC-38 — a sibling repo; noted in the PR body for the
  coordinator to book).
- srs-rust#789 (merged mirror resync) and srs-rust#792 (mirror-process docs).

---

## Phases

### Phase 1: Predicates in `srs-core`

**Goal:** All four rules exist as named, individually testable functions with correct allow-lists.

**Agent:** srs-core Worker

#### Tasks

- [x] Add `FieldType::is_effective_single(&self, assignment_repeatable: bool) -> bool` —
      `effective_cardinality() == Single && !assignment_repeatable`. Union across both mechanisms.
- [x] Add a private prose-format allow-list helper: `format` ∈ {absent, `plain`, `markdown`}.
- [x] Rewrite `is_text_searchable` (I-120 / `[R8]`): `datatype == string` AND `format` ∈
      {absent, `plain`, `markdown`, `uri`}. **Rewrite its doc comment** (CC-35).
- [x] Add `is_conditional_required_eligible` (I-94 / `[R6]`): effective-single AND `datatype` ∈
      {`string`, `date`, `date-time`}.
- [x] Add `is_theme_css_class_eligible` (`[T-9]`): effective-single AND `datatype == string` AND
      prose format. `valueDomain` unconstrained — open or closed both eligible.
- [x] Add `is_title_field_eligible` (`[N+1]`): effective-single AND `datatype == string` AND
      `valueDomain` ∈ {absent, `open`} AND prose format.

Every `format` test is an enumerated allow-list. **No `format !=` comparison anywhere** (CC-25).

#### Acceptance Criteria

- [x] All four predicates present as `pub fn` with rule ids in their doc comments
- [x] `is_text_searchable`'s superseded prose argument is gone
- [x] `rfc012_searchable_set_survives_the_rfc032_decomposition` still passes (legacy-eight parity)

#### Testing

- `i120_r8_text_projection_excludes_uuid_and_email_formats` — the **negative** case: a
  `datatype: string, format: uuid` field is not searchable, and nor is `email`. Neither occurs as a
  Tier-2 value in the corpus (CC-33).
- `i94_r6_conditional_required_rejects_list_cardinality` — negative: a list field is ineligible.
- `t9_theme_css_class_rejects_date_and_non_prose_formats` — negative: `date`, `uri`, `uuid`.
- `n1_title_field_rejects_closed_value_domain` — negative: closed-domain field ineligible.
- `effective_single_spans_both_cardinality_mechanisms` — `repeatable: true` alone defeats it, and
  `cardinality: list` alone defeats it (CC-26).

### Phase 2: Widen `validate_cross_field_rules`

**Goal:** I-94 is enforced, and the signature carries enough type information to express it.

**Agent:** srs-core Worker + srs-repository Worker

#### Tasks

- [x] Add `CrossFieldFieldType { field_type: FieldType, repeatable: bool }` to `srs-core`.
- [x] Add a builder that derives the map from `&[Field]` + `&RecordType` so the four construction
      sites do not each restate it.
- [x] Change `validate_cross_field_rules` to take `&HashMap<String, CrossFieldFieldType>`.
- [x] Apply the I-94 eligibility check in `evaluate_conditional_required`, mirroring how
      `evaluate_field_ordering` already guards. Absent field id ⇒ skip silently, as the sibling does.
- [x] Update call sites: `record_store.rs:254,398,475`, `validation.rs:696`, plus core tests.

#### Acceptance Criteria

- [x] `evaluate_conditional_required` rejects an ineligible predicate field
- [x] `evaluate_field_ordering` behaviour is unchanged
- [x] `cargo test -p srs-core -p srs-repository` green

### Phase 3: Apply `[T-9]` and `[N+1]` at the render sites

**Goal:** Both render predicates are enforced, and the test that locked in the forbidden behaviour
is inverted.

**Agent:** srs-repository Worker

#### Tasks

- [x] `css_classes_for_record`: resolve the record's Type, look up the `FieldAssignment`, and skip
      any field failing `is_theme_css_class_eligible`.
- [x] `resolve_heading_field_id`: apply `is_title_field_eligible` to `section.title_field_id`;
      when ineligible, fall through to the Type's identity field rather than honouring it.
- [x] **Invert `title_field_id_emits_record_heading`** — the fixture points `titleFieldId` at a
      repeatable field, which even the pre-erratum rule forbade. Rename to reflect the rule and
      assert no heading is emitted from that ineligible field. Explain the inversion in the PR.

#### Acceptance Criteria

- [x] All 23 first-party `titleFieldId` use sites still resolve (no corpus regression)
- [x] `srs repo validate --repo ../srs/srs` reports 0 errors
- [x] `cargo test` green

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] `cargo test --test payload_contracts` passes (no payload structs changed)
- [x] No file under `crates/srs-schema/schemas/2.0/` is modified
- [x] No sibling repository tree is modified
- [x] Each of the four predicates has a constructed negative test that fails without the predicate

## Coordination Rules

- Keep to crate write scopes; `srs-core` gains no I/O and no `schemars`.
- Do not revert the RFC-037 work from srs-rust#782 already on `master`.

## Assumptions

- The erratum's predicate table in the issue body is authoritative and complete. Any question it
  does not settle stops implementation and goes to the owner (decision protocol), not to inference.

---

## Addendum: `[N+1]` ineligibility consequence, and I-94 Type-level enforcement

The initial pass (`74cbf18`..`76e993f`) implemented `[N+1]`'s eligibility predicate but left its
*consequence* an open question — the erratum states the predicate, not what happens when an
authored `titleFieldId` fails it. Spec research returned `UNRESOLVED`; the owner then settled it
in [srs PR #341](https://github.com/the-greenman/srs/pull/341) (merged 2026-08-02). This addendum
covers the rework that decision requires, plus the I-94 Type-level gap the owner separately
confirmed as ordinary implementation work (not an open principle).

### Phase 4: `[N+1]` consequence — omit, don't substitute; diagnose

**Goal:** implement the owner's two-plane disposition exactly, and prove it with a fixture that
can actually discriminate it from the rejected reading.

- [x] `resolve_heading_field_id` (render_service.rs): an **authored** `titleFieldId` that fails
      eligibility now omits the heading — it no longer falls through to the Type's
      `identityFieldId`. Absence still falls through (RFC-020 [N+37]'s literal scope; unaffected).
- [x] Negative test with a Type carrying both an ineligible `titleFieldId` *and* an
      `identityFieldId` (`n1_ineligible_title_field_id_omits_heading_without_identity_fallback`).
      The original fixture's Type has no `identityFieldId`, so it cannot tell omission and
      fall-through apart — this one can.
- [x] New validation diagnostic: `validate_title_field_id_eligibility` (validation.rs), the
      "diagnose" half. `Warning` severity, matching the existing I-63/I-64 package-validation style
      (advisory; a bad heading degrades gracefully, it does not invalidate the repository).
      Candidate Types resolve from `TypeQuery.semanticObjectType` / `ContainerSubset.typeFilter`;
      other section sources fall back to a field-only check (no assignment-`repeatable` context
      available), mirroring the render-time fallback for an unresolvable Type.
- [x] `title_field_id_is_eligible` made `pub(crate)` so validation and render share the one
      predicate rather than restating it (ADR-010).

### Phase 5: I-94 Type-level enforcement

**Goal:** close the gap the owner confirmed as ordinary work — I-92/94/95/96 all say "MUST be
reported as a Type-level validation error", but every existing call site only runs
`validate_cross_field_rules` against an actual Record, so a Type with a misconfigured rule and
zero Records was never flagged.

- [x] `validate_cross_field_rules_for_type` (srs-core, `validation/record_type.rs`): runs the
      existing `evaluate_*` functions against a field-value-less phantom Record of the Type, then
      filters to `CrossFieldRuleMisconfigured` only. Every value-comparison guard in the three
      `evaluate_*` bodies short-circuits on a missing field value, so no other error variant can
      surface from a phantom record — proven by dedicated tests, not just asserted.
  - This shape turned out tractable enough to fold in rather than deferring: it reuses the
    existing evaluators as-is (no logic duplicated or forked) and needed no signature change.
- [x] `validate_cross_field_rule_configuration` (srs-repository, validation.rs) wraps it as an
      `Error`-severity package diagnostic — matching the invariants' "MUST" — run once per Type,
      independent of record count.
- [x] Constructed negative/positive tests at both layers (srs-core unit tests with no `Record`
      constructed anywhere; srs-repository integration tests over a zero-record `FileStore` repo).

### Gates re-run after the addendum

| Gate | Result |
|---|---|
| `cargo fmt --check` | clean (CC-50 — the first pass on this branch never ran it) |
| `cargo test` (full workspace) | green |
| `cargo clippy --all-targets -- -D warnings` | exit 0 |
| `cargo test --test payload_contracts` | 114 passed |
| `srs repo validate --repo ../srs/srs` | 0 errors, 38 warnings (unchanged from baseline) |
| All 6 spec-repo document views, rendered with this branch vs. `origin/master` | byte-identical |
