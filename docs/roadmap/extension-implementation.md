# Extension Implementation Roadmap & Conformance Review

_As of 2026-06-27._

The SRS specification (`srs/srs/records/extensions/`) defines **15 extensions**. This
document records, for each, the current state of the `srs-rust` implementation; flags the
inconsistencies that have drifted between spec design, the ADRs, and the code; and lays out
the epics that drive the remaining work to full spec conformance.

It is the source of truth for the `Epic: …` issues in the `the-greenman/srs-rust` tracker.
When an extension's status changes, update the table here and the linked issues together.

## Status at a glance

| # | Extension | Spec dep | Status | Notes |
|---|---|---|---|---|
| 1 | `ext:repository` | — | ✅ Implemented | Live format, `.srs` archive, identity-based import |
| 2 | `ext:json-store` | repository | ✅ Implemented | `JsonStore`, `from_srsj`/`to_srsj_string` (ADR-013/017) |
| 3 | `ext:views-l1` | — | ✅ Implemented | RFC-001 |
| 4 | `ext:views-l2` | views-l1 | ✅ Implemented | RFC-001 |
| 5 | `ext:themes-l1` | views-l1 | ✅ Implemented | RFC-002 |
| 6 | `ext:repeatable-fields` | — | ✅ Implemented | `FieldValue.entries` + validation |
| 7 | `ext:field-groups` | — | ✅ Implemented | `GroupValue` + constraint validation |
| 8 | `ext:protocol` | lifecycle (rec.) | 🟢 Definitions done | Runs/execution + `AttentionState` deferred (ADR-016) |
| 9 | `ext:recommended-relations` | — | ✅ Retired → core | Compatibility label only (RFC-005); relation types are core |
| 10 | `ext:lifecycle` | — | 🟢 Substantially done | Transitions, initialState injection, V7–V9 enforced; needs a verification pass to close any remaining V-gaps |
| 11 | `ext:cross-field-validation` | — | ❌ Not implemented | No `CrossFieldRule`; 3 rule types missing |
| 12 | `ext:import-tracking` | — | ❌ Not implemented | `package import` registers a boundary only; no `ImportMode`/`ImportRecord`/`ImportSummary` |
| 13 | `ext:registry` | — | ❌ Not implemented | No `Registry`/`RegistryEntry` catalog |
| 14 | `ext:federation` | — | ❌ Not implemented | No cross-repo relation qualifiers, `RepositoryRegistry`, `FederationEvent` |
| 15 | `ext:addressability` | — | ❌ Not implemented | No `Address`/`AttentionState`/`Revision`; no context-query patterns |

**Remaining work = 6 extensions:** one verify-and-finish (`lifecycle`), one partial
(`protocol` runs), and four-to-five greenfield (`cross-field-validation`, `import-tracking`,
`registry`, `federation`, `addressability`).

## Conformance / drift review

Inconsistencies between spec intent, the ADRs, and the current code. Each is tracked.

- **D1 — README lifecycle row was stale.** The status table previously called
  `ext:lifecycle` a "stub … not enforced". In fact `record_store::transition_record_lifecycle`
  (`crates/srs-repository/src/record_store.rs:695`) validates transitions, initialState is
  injected at record creation (`record_store.rs:162`), and V9 invariants are enforced in
  `validation.rs`, all with passing tests. The README predated ADR-012 (vocabulary substrate).
  _Fixed in this change._

- **D2 — `ext:type-inheritance` is implemented but has no spec extension record.** Rust
  implements it (`record_type.rs` `extends_type_id`/`field_order`/`FieldAssignmentOverride`,
  cycle detection, `effective_fields()`), and the render path now resolves it via
  `effective_fields()` (`render_service.rs:474`) rather than ignoring `fieldOrder`. But
  `srs/srs/records/extensions/` has **no `ext-type-inheritance.json`** — it exists only as
  subsections (`07-5`, `08-20`). _Spec gap: author the extension record in `srs` (RFC) or
  reclassify._ README row also updated here.

- **D3 — `blueprint` naming drift.** README/manifest examples reference `ext:blueprint`, but
  the spec treats Blueprint as a **core package definition**, not an extension (no
  `ext-blueprint.json`). The Rust implementation is correct (parallel to protocols, ADR-016);
  only the labelling is wrong. _Reconcile naming in docs/examples, or confirm intent in spec._

- **D4 — `declaredExtensions` is tracked but never validated or gated.**
  `manifest_service.rs` stores `declaredExtensions[]` in `manifest.extra` with add/remove/list,
  but nothing checks that a declared extension is supported, and no feature is gated on a
  declaration. A repo can declare `ext:federation` with zero enforcement. _Add a
  conformance-report service (low priority)._

- **D5 — `AttentionState` is shared by `ext:protocol` (runs) and `ext:addressability`.**
  Both need the live cursor over container/record/field/stage; neither is implemented. Build
  `Address`/`AttentionState`/`Revision` once in the addressability epic and have protocol-runs
  consume them. _Drives sequencing: addressability before protocol execution._

- **D6 — `ext:recommended-relations` is retired but still declarable.** RFC-005 made it a
  compatibility label with no semantics; relation types are core `RelationTypeDefinition`
  records. Implementation is consistent. _Doc-only: ensure tooling notes it as a no-op and
  does not re-introduce semantics._

- **D7 — Verify the "conformant" set against current schema shapes.** Spot-check
  `views`/`themes`/`repeatable`/`field-groups` serde shapes against the schema mirror
  (`crates/srs-schema/schemas/2.0/`) for field-rename drift since RFC-001/002 rather than
  assuming completeness.

## Epics & sequencing

Sequenced by dependency + value: finish near-done work first, then self-contained validation,
then the package-distribution trio, then the large cross-repo and addressability work last.

Each epic follows the layering pattern established by Epic #212 → sub-issues #214–219:
**ADR → RFC (if a spec change) → `srs-core` types → `srs-repository` service (typed in/out,
all validation) → `srs-cli` payload + golden schema → `srs-bindings` method → client surface
(if needed)**, honoring `docs/architecture/capability-layering.md` and ADR-010/011.

| Epic | Scope | Size | Depends on |
|---|---|---|---|
| **0 — Conformance baseline** | This doc; README fix (D1/D2); `declaredExtensions` report (D4); spec-change tickets for D2/D3 in `srs` | small | — |
| **1 — Finish `ext:lifecycle`** | Audit V7–V9 coverage, close gaps, cross-store roundtrip tests, CLI transition surface, docs | small–medium | — |
| **2 — `ext:cross-field-validation`** | `CrossFieldRule` (conditional-required / field-ordering / mutual-exclusion), `Type.validationRules`; validation + schema sync | medium | — |
| **3 — `ext:registry` + `ext:import-tracking`** | `Registry`/`RegistryEntry`; `ImportMode`/`ImportRecord`/`ImportSummary`; extend `package import` provenance + divergence detection | medium + large | — |
| **4 — `ext:federation`** | Cross-repo relation qualifiers; `RepositoryRegistry` resolution (depth-first, cycle detection); `FederationEvent` log; manifest paths; graceful degradation | large | — |
| **5 — `ext:addressability` (+ `ext:protocol` runs)** | `Address`/`AttentionState`/`Revision`; four context-query patterns; then protocol execution consuming `AttentionState` | xl | Epic 5 core unblocks protocol-runs (D5) |

## Conventions for the tracked issues

- **Epics:** title `Epic: <name>`; labels `enhancement, plan` (+ `spec change` where a spec/RFC
  is involved, + `size: …`). Mirror #212 / #178.
- **Sub-issues:** one per layer, titled by scope — `Core: …`, `Service: …`, `CLI: …`,
  `Bindings: …`, `ADR-0NN: …`, `RFC: …`; labels `enhancement, size: …, complexity: …` plus
  `architecture`/`spec change`/`wasm` as applicable. Linked via the sub-issue relationship.
- Cross-repo spec items (D2/D3) are filed in `the-greenman/srs`, not here.
