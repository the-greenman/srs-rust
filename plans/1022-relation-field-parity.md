# Plan: Relation authoring surfaces expose fields rejected by canonical schema

> Small, well-scoped bug fix (Door 1 — executes the field removal already ruled by srs#441 on the schema side; no new architectural decision). Condensed plan per proportionality: no new ADR is warranted (see Architecture Decisions below).

## Summary

`srs_core::types::relation::Relation` still carries eight fields (`assertedBy`, `confidence`, `status`, `createdBy`, `validFrom`, `validUntil`, `sourceRepositoryId`, `targetRepositoryId`) that srs#441 already removed from the canonical schema (`relation.json`/`relations-collection.json`'s embedded `Relation` def — verified byte-identical mirrors of `srs/docs/schema/2.0/relation.json`) and that `docs/spec/srs-spec.md:6089` explicitly lists under "Removed surface (historical)". `srs relation create` therefore parses input containing these fields successfully, then the repository's schema gate (`schema_validate_relation`, validating against `RELATIONS_COLLECTION_SCHEMA_ID`'s embedded `Relation` def) rejects it with "Additional properties are not allowed". The public authoring surfaces (core struct, `srs-mcp` tool input, WASM binding — which documents its input as matching the `Relation` struct) advertise inputs they cannot persist.

Fix: remove all eight fields (and the two enums that exist only to type two of them, `AssertedBy`/`RelationStatus`) from every active authoring surface, and add a struct/schema property-parity test (the `srs_schema::conformance::assert_property_parity` mechanism from srs-rust#777) so this cannot silently drift back.

`source_repository_id`/`target_repository_id` were not named in the six fields #1022 lists, but they are the same defect family (schema-absent, spec-documented "Removed surface", and confirmed by exhaustive grep to be `None` at every one of their ~20 construction sites — genuinely dead). Including them is what lets the new parity guard actually close cleanly instead of needing a carve-out; leaving them in would mean claiming "the guard covers Relation" while two known-dead fields still escape it.

**Out of scope:** srs-rust#1021 (canonical `relation.json` requires `$schema`; nothing sets it) is a different, already-filed issue — do not touch. The new parity test targets `RELATIONS_COLLECTION_SCHEMA_ID`'s `$defs.Relation` (the schema `schema_validate_relation` actually enforces today, per its own code comment — the standalone `relation.json` mirror isn't wired into the live create-time gate yet) specifically so it does not collide with #1021's $schema gap. A comment in the test cites #1021 so the exemption isn't mysterious later.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | self (direct implementation — mechanical field removal, compiler-guided) |
| Verification | self (cargo build/test/clippy gates) |

## Architecture Decisions

No new ADRs. This plan executes the removal srs#441 already ruled on the canonical-schema side; it establishes no new constraint and does not revisit any prior decision. Mode: clear · Door: 1.

---

## Contracts

### CLI output contract (ADR-011)
No CLI payload struct in `crates/srs-cli/src/payload.rs` changes — `srs relation create`'s output envelope is unaffected; only the *input* struct (`srs_core::types::relation::Relation`, deserialized directly from stdin) shrinks. No `generate-schemas` run needed.

### Entity schema sync (check-schema-sync.sh)
No `docs/schema/2.0/` files change (the canonical schema already reflects the removal — that's the whole bug). `check-schema-sync.sh` is unaffected.

---

## Scope

- Remove `asserted_by`, `confidence`, `created_by`, `status`, `valid_from`, `valid_until`, `source_repository_id`, `target_repository_id` from `srs_core::types::relation::Relation`.
- Remove the now-unused `AssertedBy`/`RelationStatus` enums.
- Remove the corresponding fields from `srs-mcp`'s `RelationCreateToolInput` (and its `AssertedByInput`/`RelationStatusInput` shadow enums).
- Remove `render_service.rs`'s `is_active_relation` (dead once `status` can never be set) and its two call sites.
- Fix every other construction site across `srs-repository`/`srs-bindings` tests that sets the removed fields to `None` (compiler-guided).
- Add a struct/schema property-parity test for `Relation` (srs-rust#777 pattern).
- Update existing `relation.rs` unit tests that exercised the removed fields.

**Out of scope:** srs-rust#1021 ($schema requiredness); switching the live create-time gate from the legacy `relations-collection.json` embedded def to the standalone `relation.json` (unrelated pre-existing TODO noted in `relation_service.rs:211-214`); any change to `SourceReference.confidence` (a distinct, still-canonical field on a nested type).

---

## Phases

### Phase 1: Remove the fields and fix every call site

**Goal:** `cargo build --workspace` succeeds with the eight fields and two enums gone everywhere.

**Agent:** self

#### Tasks

- [ ] `crates/srs-core/src/types/relation.rs`: remove the 8 fields + `AssertedBy`/`RelationStatus` enums; update `relation_roundtrips_json` and `relation_with_optional_schema_fields_parses` tests.
- [ ] `crates/srs-mcp/src/tools.rs`: remove `AssertedByInput`, `RelationStatusInput`, their `From` impls, the corresponding `RelationCreateToolInput` fields, and the import of `AssertedBy`/`RelationStatus`; update its test.
- [ ] `crates/srs-repository/src/render_service.rs`: remove `is_active_relation` + the `RelationStatus` import + its two call sites (keep the surrounding filter logic, just drop the `&& is_active_relation(rel)` conjunct).
- [ ] Fix remaining construction sites (compiler-guided, `cargo build --workspace` until clean): `services.rs`, `repository_portability.rs`, `container_service.rs`, `record_store.rs`, `graduated_at_migration_service.rs`, `tree_service.rs`, `store.rs`, `okf_export_service.rs`, `relation_graph.rs`, `context_query_service.rs`, `relation_service.rs`, `srs-core/validation/relation.rs`, `srs-bindings/tests/relation_lifecycle.rs`, `srs-repository/tests/doctor_service.rs`.

#### Acceptance Criteria

- [ ] `cargo build --workspace` succeeds.
- [ ] No remaining references to `asserted_by`/`AssertedBy`/`confidence` (on `Relation`)/`created_by`/`status` (on `Relation`)/`RelationStatus`/`valid_from`/`valid_until`/`source_repository_id`/`target_repository_id` anywhere in `crates/`.

#### Testing

```bash
cargo build --workspace
```

### Phase 2: Regression test + parity guard

**Goal:** The bug's reproduction (creating a relation with `assertedBy` set) now fails at *parse* time (unknown field), not just at the schema gate — proving the authoring surface itself no longer advertises the dead field. A new struct/schema parity test locks the property set.

**Agent:** self

#### Tasks

- [ ] Add a test to `crates/srs-core/src/types/relation.rs` proving `serde_json::from_str::<Relation>(...)` with `"assertedBy": "human"` now fails to deserialize (`deny_unknown_fields`) — this is the actual regression test for #1022 (previously it parsed fine and only the separate schema gate rejected it; now the authoring surface itself rejects it, matching what it persists).
- [ ] Add `relation_struct_matches_relations_collection_relation_def_property_set` using `srs_schema::conformance::assert_property_parity(srs_schema::RELATIONS_COLLECTION_SCHEMA_ID, Some("Relation"), required, optional)`, paired with an exhaustive `let Relation { .. } = sample();` destructure (srs-rust#777 pattern) so the compiler forces this test to be touched if `Relation` ever gains/loses a field again.
- [ ] Confirm `srs-core`'s `Cargo.toml` already has a `srs-schema` dev-dependency (it does, via the existing `field.rs`/`record_type.rs` parity tests) — no dependency change needed.

#### Acceptance Criteria

- [ ] New tests pass; old `relation_with_optional_schema_fields_parses` no longer references the removed fields.
- [ ] `cargo test -p srs-core` green.

#### Testing

```bash
cargo test -p srs-core
```

### Phase 3: Full workspace gates

**Goal:** Same standard as every PR in this repo.

**Agent:** self

#### Tasks

- [ ] `cargo test --workspace` — zero failures.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`.

#### Acceptance Criteria

- [ ] Both commands exit 0.

#### Testing

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

---

## Final Acceptance

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace` (zero failures)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `srs relation create` with `assertedBy`/`confidence`/`status`/`createdBy`/`validFrom`/`validUntil` in the input now fails at parse time with an "unknown field" error, not a downstream schema-gate error.
- [ ] No stale reference to the removed fields/enums anywhere in `crates/`.
