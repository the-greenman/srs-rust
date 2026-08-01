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

- [ ] Add `FieldType::is_effective_single(&self, assignment_repeatable: bool) -> bool` —
      `effective_cardinality() == Single && !assignment_repeatable`. Union across both mechanisms.
- [ ] Add a private prose-format allow-list helper: `format` ∈ {absent, `plain`, `markdown`}.
- [ ] Rewrite `is_text_searchable` (I-120 / `[R8]`): `datatype == string` AND `format` ∈
      {absent, `plain`, `markdown`, `uri`}. **Rewrite its doc comment** (CC-35).
- [ ] Add `is_conditional_required_eligible` (I-94 / `[R6]`): effective-single AND `datatype` ∈
      {`string`, `date`, `date-time`}.
- [ ] Add `is_theme_css_class_eligible` (`[T-9]`): effective-single AND `datatype == string` AND
      prose format. `valueDomain` unconstrained — open or closed both eligible.
- [ ] Add `is_title_field_eligible` (`[N+1]`): effective-single AND `datatype == string` AND
      `valueDomain` ∈ {absent, `open`} AND prose format.

Every `format` test is an enumerated allow-list. **No `format !=` comparison anywhere** (CC-25).

#### Acceptance Criteria

- [ ] All four predicates present as `pub fn` with rule ids in their doc comments
- [ ] `is_text_searchable`'s superseded prose argument is gone
- [ ] `rfc012_searchable_set_survives_the_rfc032_decomposition` still passes (legacy-eight parity)

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

- [ ] Add `CrossFieldFieldType { field_type: FieldType, repeatable: bool }` to `srs-core`.
- [ ] Add a builder that derives the map from `&[Field]` + `&RecordType` so the four construction
      sites do not each restate it.
- [ ] Change `validate_cross_field_rules` to take `&HashMap<String, CrossFieldFieldType>`.
- [ ] Apply the I-94 eligibility check in `evaluate_conditional_required`, mirroring how
      `evaluate_field_ordering` already guards. Absent field id ⇒ skip silently, as the sibling does.
- [ ] Update call sites: `record_store.rs:254,398,475`, `validation.rs:696`, plus core tests.

#### Acceptance Criteria

- [ ] `evaluate_conditional_required` rejects an ineligible predicate field
- [ ] `evaluate_field_ordering` behaviour is unchanged
- [ ] `cargo test -p srs-core -p srs-repository` green

### Phase 3: Apply `[T-9]` and `[N+1]` at the render sites

**Goal:** Both render predicates are enforced, and the test that locked in the forbidden behaviour
is inverted.

**Agent:** srs-repository Worker

#### Tasks

- [ ] `css_classes_for_record`: resolve the record's Type, look up the `FieldAssignment`, and skip
      any field failing `is_theme_css_class_eligible`.
- [ ] `resolve_heading_field_id`: apply `is_title_field_eligible` to `section.title_field_id`;
      when ineligible, fall through to the Type's identity field rather than honouring it.
- [ ] **Invert `title_field_id_emits_record_heading`** — the fixture points `titleFieldId` at a
      repeatable field, which even the pre-erratum rule forbade. Rename to reflect the rule and
      assert no heading is emitted from that ineligible field. Explain the inversion in the PR.

#### Acceptance Criteria

- [ ] All 23 first-party `titleFieldId` use sites still resolve (no corpus regression)
- [ ] `srs repo validate --repo ../srs/srs` reports 0 errors
- [ ] `cargo test` green

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] No file under `crates/srs-schema/schemas/2.0/` is modified
- [ ] No sibling repository tree is modified
- [ ] Each of the four predicates has a constructed negative test that fails without the predicate

## Coordination Rules

- Keep to crate write scopes; `srs-core` gains no I/O and no `schemars`.
- Do not revert the RFC-037 work from srs-rust#782 already on `master`.

## Assumptions

- The erratum's predicate table in the issue body is authoritative and complete. Any question it
  does not settle stops implementation and goes to the owner (decision protocol), not to inference.
