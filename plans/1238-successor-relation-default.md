# Plan: Successor relation default from the core (#1238)

> DRAFT. Stage 2.4 design pause: the API shape in "Design decision" is awaiting the owner's ruling. Phases below describe the RECOMMENDED option (B); they change if the owner picks A.

## Summary

srs-web creates successor records via `record successor` / `create_record_successor` / MCP `record_successor`, whose input requires `relationType`. For final/immutable records there are no outgoing transitions, so the only core projection of `requiresRelation` (on `allowed_transitions`) is empty and the client hard-codes `"supersedes"` (srs-web#215). The core already holds the answer: the record's effective lifecycle declares a state (governance seed: `superseded`) with `requiresRelation {relationType: ["supersedes"]}`. Goal: the core supplies the relation so clients never name it. Also drop the stale "approved interim exception" note in `docs/architecture/capability-layering.md` (~L198-203).

## Spec gate (Stage 1.5)

No spec change required, under either option, provided the core only DERIVES the relation from a declared `requiresRelation` and never invents a default. Citations (srs origin/master):
- RFC-022 (`rfcs/rfc-022-relational-lifecycle-states.md`, Accepted Rev 4; invariants I-98/I-99/I-100). Note RFC-021 is the blueprint-optional-schema RFC; supersession is RFC-022 (issue srs#158).
- R6 / I-99: with an any-of `relationType` array, the relation defaults to the FIRST declared type. This is the spec's precedent for defaulting from the state declaration.
- Rationale, "No implicit relation-type default": "Defaulting to `supersedes` would re-encode the special case the object shape removes." So a core-hard-coded `"supersedes"` IS ruled out; a derived default is not.
- Change C / R9 / I-100: the obligation is projected only on allowed-transitions entries. Nothing forbids an additional read; the spec is silent on a lifecycle-level read (implementation-level, additive).
- Risk: RFC-022 Change B says `record successor` "is unchanged". Making `relationType` optional is backward compatible (explicit value behaves exactly as before), but an owner who reads "unchanged" strictly would want a spec note. Flagged in the design pause.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository worker (srs-repository) | — |
| CLI/bindings/MCP worker | — |
| Verification | — |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| ADR-010 service boundary | logic once in `srs-repository`, adapters thin | governs |
| ADR-011 CLI output contract | any payload change needs `payload.rs` + regenerated `schemas/payload/` | governs |
| ADR-013 / ADR-015 WASM strategy | binding mirrors the service signature | governs |
| ADR-022 governance status is lifecycle state | no client lifecycle vocabulary; core answers | governs |
| ADR-037 MCP adapter surface | `record_successor` tool mirrors the service | governs |
| ADR-048 implementation decision rules | one way per goal; no parallel mechanisms | governs |
| New ADR | none expected (implements RFC-022 R6 precedent); revisit if owner picks A | TBD |

(ADR check: read the ADRs above plus 002, 024 (rollback), 042; nothing contradicts. Interop register: not touched beyond existing agent-facing surfaces; no new format.)

## Design decision (owner ruling needed)

- Option A: lifecycle-state-level `requiresRelation` read. New read (service fn + CLI command + WASM binding + MCP tool/resource) returning, per record, the relation obligation(s) of the effective lifecycle. Client still passes the relation to `record successor`, so it still handles the value, and the "which state" selection is ambiguous for a lifecycle with several relational states. Adds a public surface in 4 places.
- Option B (recommended): `relationType` becomes optional on `CreateRecordSuccessorInput` (core), flowing through CLI stdin, WASM and MCP automatically. When omitted, the core derives it from the predecessor's effective lifecycle: collect the `requiresRelation` of states with `enforcement` hard and direction incoming (or omitted); if exactly one distinct first-declared type results, use it (R6 rule: first of an any-of array). Zero or several candidates: structured error naming the candidates and requiring explicit `relationType`. Explicit value unchanged (incl. `refines`). Never a hard-coded literal. No new command, no new read; the response already returns the relation, so clients can show it.
- Option C (rejected): hard-code `supersedes` in the core. Contradicts RFC-022 Rationale and ADR-048 (re-encodes the special case in code).

Trade-offs for B: minimal surface, thinnest client, single mechanism; cost is that the choice is implicit when a lifecycle has multiple relational states (mitigated by the explicit error), and the "record successor unchanged" wording above. Combining A and B would violate one-way-per-goal.

## Contracts

- CLI payload (ADR-011): input-only change; `RecordSuccessorPayload` shape unchanged, so no schema regeneration expected. Verify `cargo test --test payload_contracts`. Stdin docs updated.
- Entity schema sync: none (no spec schema change).

## Scope

- `crates/srs-repository/src/record_store.rs`: `relation_type: Option<String>` (serde default), derivation helper over `package.effective_lifecycle`, structured error variant for no/ambiguous candidates; validation via existing `validate_relation_type_for_write` after derivation.
- Adapters: `srs-cli` (`commands/record.rs`, help text), `srs-bindings` doc comment, `srs-mcp` `record_successor` input schema/description (optional field). No logic in adapters.
- Docs: `docs/architecture/capability-layering.md` drop interim-exception note; `srs/srs-usage.md` (record successor input) via a branch in `srs/`; `docs/dogfooding.md` scenario.
- srs-web follow-up (separate repo): remove the `"supersedes"` literal in GovernanceShell, tracked in srs-web#215.

**Out of scope:** Option A read; spec amendment; changing `refines` handling; retype (R11).

## Phases

### Phase 1: Service default
**Goal:** `create_record_successor` derives the relation when omitted.
**Agent:** Repository worker
#### Tasks
- [ ] Make `relation_type` optional; derive via effective lifecycle; structured errors
- [ ] Unit tests in `record_store.rs`
#### Acceptance Criteria
- [ ] Governance lifecycle: omitted -> `supersedes`; explicit `refines` still works
- [ ] Lifecycle with no relational state: error naming "specify relationType"
- [ ] Two distinct candidate types: error listing them
- [ ] any-of array: first declared type chosen (R6)
- [ ] No hard-coded `"supersedes"` in non-test code
#### Testing
`cargo test -p srs-repository successor` ; tests: `successor_default_from_lifecycle`, `successor_default_none_errors`, `successor_default_ambiguous_errors`, `successor_default_anyof_first`, `successor_explicit_unchanged`.
#### Milestone gate
`cargo test -p srs-repository`; `cargo clippy -p srs-repository -- -D warnings`; tick boxes; `git commit` (#1238).

### Phase 2: Adapters + docs
**Goal:** CLI, WASM, MCP accept omission; docs match.
**Agent:** CLI/bindings/MCP worker
#### Tasks
- [ ] CLI/MCP/binding docs and schemas; one CLI and one MCP test omitting `relationType`
- [ ] capability-layering note removed; srs-usage.md branch; dogfooding scenario (closed governance article -> successor without naming relation)
#### Acceptance Criteria
- [ ] Parity: CLI, WASM, MCP produce the same relation for the same input
- [ ] `cargo test --test payload_contracts` passes
#### Testing
`cargo test -p srs-cli -p srs-mcp -p srs-bindings`
#### Milestone gate
As above for each crate; commit.

## Final Acceptance

- [ ] `cargo test`, `cargo clippy -- -D warnings`, `cargo test --test payload_contracts` pass
- [ ] Spec-dependent tests run with `SRS_SPEC_DIR` at a fresh clone of srs origin/master (srs-rust#874)
- [ ] Dogfood scenario run on the branch build

## Coordination Rules

- Agents keep to write scopes; no reverting others' edits; Lead Integrator owns API naming.
- End of each phase: verify criteria, tick the plan, commit.

## Assumptions

- Owner picks B. If A, Phase 1 becomes a new read service fn plus CLI/WASM/MCP exposure and a likely ADR.
