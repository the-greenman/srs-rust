# Plan: Relation write/validate E4-parity (#556 + #548)

> Part of the relation-coherence epic **the-greenman/srs#171**, principles **R9** (one validated write path) and **R10** (every rule has an enforcement point). Write and validate must agree about what is acceptable.

## Summary

Two tightly-coupled implementation gaps let a relation that violates its type's semantic-type constraints (E4) slip through, and let at-rest validation miss the file the tooling actually writes:

- **#556 — E4 dead on create.** `create_relation` ([relation_service.rs:166](../crates/srs-repository/src/relation_service.rs#L166)) passes an **empty** `instance_semantic_types` map into `RelationValidationContext`, so E4 (`allowedSourceTypes` / `allowedTargetTypes` / `requireSameSemanticObjectType`) never fires on the write path. A relation that violates a type constraint is accepted on create.
- **#548 — validate reads the wrong file.** `repo validate` ([validation.rs:630](../crates/srs-repository/src/validation.rs#L630)) reads only the hardcoded `relations/relations.json`, never the authoritative file the relation service resolves (`manifest.relationsPath` → `relations/relations-collection.json` → `relations/relations.json`). In any repo whose relations live at `relations-collection.json` (everything written by `srs relation create` / WASM `create_relation` / `record successor` in a fresh repo) or a custom `relationsPath`, validate silently skips the entire relations block — E1–E4 are never enforced at rest and validate looks green.

The two bugs are the same defect seen from both ends of the write/validate contract, and both are rooted in **duplicated logic that has drifted**: the instance→semanticObjectType map is built in three places (one of them empty, one dead) and the relations-file resolution order is implemented in one place and ignored in another. The fix centralises each into a single `srs-repository` helper and routes every site through it, which repairs both bugs by construction.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Repository Service Worker | Claude (this session) — all edits in `crates/srs-repository/**` |
| Verification | Architecture Reviewer + Verification Agent (Stage 7) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

_No new architectural decisions._ This plan implements **ADR-010** (service-boundary contract — all validation lives in `srs-repository` services, typed in/out) and reinforces the existing crate-authority model (`srs-rust/CLAUDE.md` → Crate Authority): all business logic stays in `srs-repository`, `srs-core` remains I/O-free, no path strings leak outside the resolution helper. It is a bug-fix + DRY consolidation of logic that already exists; it does not establish, reject, or change any architectural constraint.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Validation belongs in the `srs-repository` service, not duplicated per call site | accepted (governs, not produced) |

---

## Contracts

### CLI output contract (ADR-011)

**No new/changed command output shapes.** No struct in `crates/srs-cli/src/payload.rs` changes. `repo validate` already returns `diagnostics[]`; this plan makes that array *populated correctly* for relations at rest and makes the `relative_path` on relation diagnostics reflect the file actually validated (e.g. `relations/relations-collection.json`) instead of the hardcoded `relations/relations.json`. That is a change to diagnostic **content**, not to the payload **shape** — no `payload.rs` edit, no `generate-schemas` run required. `create_relation` already returns `CreateRelationResult`; on an E4 violation it now returns the existing `RepositoryError::RelationValidation` error envelope (same shape, previously unreachable for E4).

Verification: `cargo test --test payload_contracts` must still pass (expected: unchanged).

### Entity schema sync (check-schema-sync.sh)

**No.** No JSON Schema files under `srs/docs/schema/2.0/` (or the mirrors) are added or modified.

---

## Scope

- Add `pub(crate) fn build_instance_semantic_types(store: &dyn RepositoryStore, manifest: &Manifest) -> HashMap<String, String>` (in `writer.rs`, where the canonical copy of this loop already lives), reading each indexed instance's top-level `semanticObjectType` field.
- Route `create_relation` (#556 fix), `repo validate` (dedup), and the dead `validate_relation_before_write` through that one helper — eliminating the empty map and the two duplicate loops.
- Add `pub(crate) fn resolve_relations_source(store: &dyn RepositoryStore) -> Result<Option<(String, String)>, RepositoryError>` (in `relation_service.rs`) that resolves the authoritative relations file via the **same candidate order** as `load_relations_collection` and returns `(relative_path, raw_text)`, or `None` when no relations file exists.
- Extract the candidate-path list (`manifest.relationsPath` → `relations/relations-collection.json` → `relations/relations.json`) into one `fn relations_candidate_paths(store) -> Result<Vec<String>, RepositoryError>` used by both `load_relations_collection` and `resolve_relations_source`, so the two paths can never drift again.
- Rewire `validate_repository`'s relations block (#548 fix) to load via `resolve_relations_source`, and report every relation diagnostic against the **resolved** `relative_path`.
- Tests proving E4 fires on create, and that validate catches E1/E2/E4 violations in `relations-collection.json` and a custom `relationsPath`.

**Out of scope:**

- The new **diagnostics** in #557 (containerId endpoint, duplicate edges, precedes fan-out/cycles). This plan only makes validate *read the right file and build the right map*; #557 adds new checks on top of that foundation.
- The pre-existing latent bug where `load_relations_collection`'s `default_write_path` ignores a custom `manifest.relationsPath` when no relations file exists yet (the first relation is written to `relations/relations-collection.json` instead of the declared path). Filed as a follow-up issue parented under the epic (arch-review nit #4) — not fixed here. (`analysis.rs`'s duplicate candidate list, previously listed here, is now **in scope** — see Phase 2.)
- Any change to `srs-core` validation logic (`validate_relation`, E1–E4 semantics). The core rules are correct; only the repository-layer *inputs* to them are being fixed.
- Removing the `relationType == "precedes"` literal (#558) and inverse-form rejection (#559) — separate PRs.

---

## Phases

### Phase 1: Centralise the semantic-type map + fix E4 on create (#556)

**Goal:** `create_relation` enforces E4 identically to `repo validate`; the instance→semanticObjectType map is built by exactly one function.

**Agent:** Repository Service Worker

#### Tasks

- [x] **Confirm `validate_relation_before_write` is dead and remove it.** It has **zero callers** (`grep -rn "validate_relation_before_write" crates/` returns only its own definition at `writer.rs:11`; no `lib.rs` / `pub use` re-export). Deleting it removes one of the three copies of the semantic-type loop outright. _Fallback only if an unexpected reference appears:_ keep it, route it through the new helper, **and** downgrade `pub fn` → `pub(crate) fn` so it cannot become a second relation-write entry point. Record which path was taken in the commit message.
- [x] In `crates/srs-repository/src/writer.rs`, add `pub(crate) fn build_instance_semantic_types(store: &dyn RepositoryStore, manifest: &Manifest) -> HashMap<String, String>` containing the semantic-type loop (read each `manifest.instance_index` entry's file, insert `instance_id → semanticObjectType` when the top-level field is a string).
- [x] In `crates/srs-repository/src/relation_service.rs`, replace the empty-map construction at line 166 with `let instance_semantic_types = crate::writer::build_instance_semantic_types(store, &manifest);` (the `manifest` is already loaded at line 160 — do not double-load it, per Repository Service Worker constraints).
- [x] In `crates/srs-repository/src/validation.rs`, replace the inline map-build loop (~lines 679–687) with a call to the shared helper.

#### Acceptance Criteria

- [x] `create_relation` rejects a relation whose source/target `semanticObjectType` violates the resolved `RelationTypeDefinition`'s `allowedSourceTypes`/`allowedTargetTypes`/`requireSameSemanticObjectType`, returning `RepositoryError::RelationValidation` with an `E4`-coded message.
- [x] `create_relation` still accepts a relation whose endpoints carry no `semanticObjectType` or whose type has no E4 constraint (no regression — E4 only fires when both a constraint and a typed endpoint are present).
- [x] Exactly one function builds the instance→semanticObjectType map; no inline duplicate loop remains in `relation_service.rs` or `validation.rs`.
- [x] `validate_relation_before_write` is either removed (dead) or delegates to the helper — no third copy of the loop survives.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write (in `relation_service.rs` `#[cfg(test)]`, MemoryStore):

- `create_relation_enforces_e4_allowed_source_types` — a def with `allowedSourceTypes: ["com.x/allowed"]`, a source instance with `semanticObjectType: "com.x/forbidden"` → create returns `Err` with an E4 message; the relations file is **not** written.
- `create_relation_enforces_require_same_semantic_object_type` — def with `requireSameSemanticObjectType: true`, mismatched endpoint types → `Err`.
- `create_relation_allows_untyped_endpoints_under_constrained_type` — same constrained def but endpoints carry no `semanticObjectType` → create succeeds (proves the `if let Some` guard preserved, no false positive).

#### Milestone gate

`cargo test -p srs-repository` + `cargo clippy -p srs-repository -- -D warnings` green; plan checkboxes updated; commit `fix(relations): enforce E4 on relation create via shared semantic-type map (#556)`.

---

### Phase 2: Validate the authoritative relations file (#548)

**Goal:** `repo validate` loads relations through the same resolution the service writes through, so E1–E4 run at rest on whichever file is authoritative, with diagnostics attributed to the real path.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `relation_service.rs`, extract the candidate-path list into `pub(crate) fn relations_candidate_paths(store: &dyn RepositoryStore) -> Result<Vec<String>, RepositoryError>` (manifest `relationsPath` first, then `relations/relations-collection.json`, then `relations/relations.json`; NotFound/Io while reading the manifest → treat as no `relationsPath`, matching the current logic). Refactor `load_relations_collection` to consume it.
- [x] Add `pub(crate) fn resolve_relations_source(store: &dyn RepositoryStore) -> Result<Option<(String, String)>, RepositoryError>` (document the tuple as `(relative_path, raw_text)`): iterate `relations_candidate_paths`; for each, `store.load_text_file(path)` — first `Ok(raw)` returns `Some((path, raw))`; `Err(NotFound|Io)` continues; any other error propagates; exhausted → `Ok(None)`. (Raw text, not parsed, so validate can still report a JSON-parse diagnostic on a malformed file — matching current behaviour.)
- [x] In `crates/srs-repository/src/analysis.rs` (`summarize_relations`, ~L556–581), replace its inline relations-candidate-path array with a call to `relation_service::relations_candidate_paths(store)?`, keeping its own `try_load_relations_json` error-swallowing wrapper and local dedup. This removes the **third** copy of the resolution order so the Final-Acceptance "exactly one candidate-path list" holds honestly (arch-review finding #1).
- [x] In `validation.rs`, replace `if let Ok(relations_raw) = store.load_text_file("relations/relations.json")` with `if let Some((relations_path, relations_raw)) = crate::relation_service::resolve_relations_source(store)?`. Use `relations_path` (a binding) in place of every hardcoded `"relations/relations.json"` in that block: the schema-validate `relative_path` arg (~L635), the JSON-parse-error diagnostic (~L694), and the per-relation diagnostic (~L727).
- [x] Confirm the outer `validate_repository` signature already returns `Result<_, RepositoryError>` so the `?` on `resolve_relations_source` compiles (it does — see the early `return Ok(...)` blocks); the previous swallow-on-error behaviour is preserved because the resolver maps NotFound/Io → `None`.

#### Acceptance Criteria

- [x] With relations stored at `relations/relations-collection.json` (no `relations.json`), `repo validate` surfaces an E1 (unresolvable type) and an E2 (dangling endpoint) diagnostic that it previously missed.
- [x] With a manifest `relationsPath` pointing at a custom file, `repo validate` reads that file.
- [x] Relation diagnostics carry `relative_path` equal to the resolved file, not the hardcoded default.
- [x] A repo with relations only at the legacy `relations/relations.json` still validates exactly as before (back-compat).
- [x] A repo with no relations file at all still skips the relations block cleanly (no new error).

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write (in `validation.rs` `#[cfg(test)]`, MemoryStore):

- `validate_reads_relations_collection_json_for_e1` — bogus `relationType` in `relations-collection.json` → E1 diagnostic present (regression guard for #548; would report 0 diagnostics before the fix).
- `validate_reads_relations_collection_json_for_e2` — dangling `targetInstanceId` in `relations-collection.json` → E2 diagnostic present.
- `validate_honours_manifest_relations_path` — `relationsPath` set to a custom path holding a bad relation → diagnostic present, and its `relative_path` equals the custom path.
- `validate_still_reads_legacy_relations_json` — bad relation in `relations/relations.json` with no collection file → diagnostic present (back-compat).

#### Milestone gate

`cargo test -p srs-repository` + `cargo clippy -p srs-repository -- -D warnings` green; plan checkboxes updated; commit `fix(validate): resolve authoritative relations file for E1–E4 at rest (#548)`.

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] CLI output format unchanged (integration tests pass)
- [x] `cargo test --test payload_contracts` passes (no payload structs changed — expected green)
- [x] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed — expected green)
- [x] E4 fires on `create_relation` (dogfood: `srs relation create` rejects a type-constraint-violating edge)
- [x] `repo validate` reports E1/E2 diagnostics for a bad relation living in `relations-collection.json` (dogfood: the #548 repro sketch now shows diagnostics instead of 0)
- [x] Exactly one instance→semanticObjectType map builder and one relations-file candidate-path list remain in the crate

## Coordination Rules

- All edits confined to `crates/srs-repository/**` (writer.rs, relation_service.rs, validation.rs). No `srs-core`, `srs-cli`, or schema changes.
- Do not double-load the manifest — `create_relation` already holds it; pass it into the helper.
- Milestone gate at the end of each phase before proceeding.

## Assumptions

- The canonical core-package relation definitions (`contains`, `precedes`, `depends-on`, …) carry **no** E4 type constraints, so enabling E4 on create does not break existing fixtures/tests that create those relations between untyped instances. (Verify during Phase 1 by running the full `srs-repository` suite; if a fixture regresses, it was relying on the bug.)
- `store.load_text_file` returns `Err(NotFound|Io)` for a missing relations file (used as the existence probe in `resolve_relations_source`).
- `resolve_relations_source` must read via `store.load_relations_json` (returns a parsed `serde_json::Value`), **not** `store.load_text_file`. Code review found that `load_text_file` only surfaces `FileStore`'s on-disk text: for object-backed stores (`MemoryStore`, and critically `JsonStore` — the `.srsj`/WASM store behind srs-web) a relation written via `save_relations_json` is stored as a JSON object and `load_text_file` returns nothing, so validate would silently skip every relation there — leaving #548 unfixed for the actual WASM consumer. Reading via `load_relations_json` resolves uniformly across all three stores; a missing file maps to `Io`/`NotFound` (skip) and a malformed FileStore file maps to `Serialize` (propagated and surfaced by validate as a diagnostic). Proven by `validate_finds_relations_in_jsonstore_cross_store` (FileStore→snapshot→JsonStore).
