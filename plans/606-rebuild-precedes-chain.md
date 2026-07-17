# Plan: WASM rebuild_precedes_chain binding

> **Issue:** srs-rust#606
> **Epic:** srs-rust#350 — Platform quality & hardening

## Summary

`srs-web`'s `GuidesShell.svelte` contains a ~10-line TypeScript orchestration (`rebuildPrecedesChain`) that calls `list_relations`, `delete_relation`, and `create_relation` in sequence to rebuild a linear `precedes` chain. This is domain knowledge about precedes-chain structure — not presentation logic — and violates ADR-001's rule that the web client must be free of SRS semantics. This plan adds a `rebuild_precedes_chain` service function to `srs-repository` and a corresponding WASM binding in `srs-bindings`, allowing srs-web to replace its ~10-line orchestration with a single WASM call.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | `agents.md#repository-service-worker` |
| Bindings Worker | `agents.md#bindings-worker` |
| Verification Agent | `agents.md#verification-agent` |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | New service function `rebuild_precedes_chain` takes typed input, performs all orchestration and validation, writes atomically in one collection write. Single write is inherently atomic; ADR-024 rollback machinery not needed. | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM binding is a thin `SrsRepository` method — deserialise JS input → one service call → serialise output | accepted |
| [ADR-015](../docs/adr/015-wasm-write-and-export.md) | Write mutation via in-memory `JsonStore`, no filesystem access | accepted |
| [CLAUDE.md capability-layering](../CLAUDE.md) | WASM-only exposure (no CLI command) is acceptable here because CLI callers retain full composability via the existing `relation delete` + `relation create` commands. The service itself follows the full capability-layering model. | accepted |

No new ADRs required — all decisions follow established patterns.

---

## Contracts

### CLI output contract (ADR-011)

No CLI command is added or changed. The `rebuild_precedes_chain` service is exposed only via the WASM binding. No payload struct changes, no `generate-schemas` run needed.

### Entity schema sync (check-schema-sync.sh)

No schema files in `srs/docs/schema/2.0/` are modified. No sync required.

---

## Scope

- Add `rebuild_precedes_chain(store, input)` to `crates/srs-repository/src/relation_service.rs`, with a `pub(crate)` helper `build_relation_validation_ctx` shared with `create_relation`
- Add `rebuild_precedes_chain(input_json)` WASM method to `crates/srs-bindings/src/lib.rs`
- Six unit tests for the service function (5 behavioural + 1 cross-store roundtrip) using `MemoryStore` / `JsonStore`
- WASM build smoke test verifying the binding compiles to `wasm32-unknown-unknown`

**Out of scope:**
- Any srs-web changes (follow-on work in the srs-web repo)
- CLI command for `rebuild_precedes_chain` (not needed — CLI callers compose existing `relation delete` + `relation create`)
- Non-`precedes` chain rebuilding

---

## Phases

### Phase 1: Service function in srs-repository

**Goal:** `relation_service::rebuild_precedes_chain` exists, passes all tests, and performs a single atomic collection write.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `crates/srs-repository/src/relation_service.rs`, define the input struct before `rebuild_precedes_chain`:
  ```rust
  #[derive(Debug, Clone)]
  pub struct RebuildPrecedesChainInput {
      /// Desired linear order — edges created as instance_ids[0]→[1]→…→[n-1].
      pub instance_ids: Vec<String>,
      /// IDs whose existing `precedes` edges (source OR target) are deleted first.
      pub clear_ids: Vec<String>,
  }
  ```

- [x] Define the result struct in the same file:
  ```rust
  #[derive(Debug, Clone, serde::Serialize)]
  #[serde(rename_all = "camelCase")]
  pub struct RebuildPrecedesChainResult {
      pub created: Vec<RelationSummary>,
  }
  ```

- [x] Extract `pub(crate) fn build_relation_validation_ctx` from the duplicated block in `create_relation`, placing it above `create_relation` in `relation_service.rs`. Signature:
  ```rust
  pub(crate) fn build_relation_validation_ctx<'a>(
      store: &dyn RepositoryStore,
      manifest: &'a srs_core::types::manifest::Manifest,
      definitions: &'a [srs_core::types::relation_type_definition::RelationTypeDefinition],
  ) -> RelationValidationContext<'a>
  ```
  Body: constructs `known_instance_ids` from `manifest.instance_index` and calls `crate::writer::build_instance_semantic_types(store, manifest)`, returning `RelationValidationContext { definitions, known_instance_ids: &known_instance_ids, instance_semantic_types: &instance_semantic_types }`.
  **Note:** `known_instance_ids` and `instance_semantic_types` are owned in the caller; this helper receives refs and returns a context that borrows them. Adjust lifetime bounds as needed — the existing `create_relation` body shows the exact pattern.
  Update `create_relation` to call `build_relation_validation_ctx` instead of duplicating the block.

- [x] Implement `pub fn rebuild_precedes_chain(store: &dyn RepositoryStore, input: RebuildPrecedesChainInput) -> Result<RebuildPrecedesChainResult, RepositoryError>` in `relation_service.rs`. Algorithm — single atomic write:
  1. Load the relations collection once: `let (relative_path, mut collection) = load_relations_collection(store)?;`
  2. Delete all `precedes` edges where source OR target is in `clear_ids`: `collection.relations.retain(|r| !(r.relation_type == "precedes" && (clear_ids_set.contains(&r.source_instance_id) || clear_ids_set.contains(&r.target_instance_id))));`
  3. If `instance_ids.len() <= 1`, skip edge creation (0 new edges).
  4. Load the package: `let package = store.load_package()?;`
  5. Load the manifest: `let manifest = store.load_manifest()?;`
  6. Build the validation context once using `build_relation_validation_ctx(store, &manifest, &package.relation_type_definitions)`. (Own the `known_instance_ids` HashSet and `instance_semantic_types` map in the caller; pass refs to the helper as in `create_relation`.)
  7. For each adjacent pair `(instance_ids[i], instance_ids[i+1])`, create a `Relation { relation_id: new_instance_id(), relation_type: "precedes".to_string(), source_instance_id: instance_ids[i].clone(), target_instance_id: instance_ids[i+1].clone() }`. Call `validate_relation(&relation, &ctx, true)` — on error, return `RepositoryError::RelationValidation { relation_id, message }`. Collect validated relations.
  8. Append all new relations to `collection.relations`.
  9. Schema-validate the full collection: `SchemaRegistry::global().validate_by_id(RELATIONS_COLLECTION_SCHEMA_ID, &serde_json::to_value(&collection)?)`.
  10. Write exactly once: `write_relations_collection(store, &relative_path, &collection)?;`
  11. Return `RebuildPrecedesChainResult { created: new_relations.iter().map(|r| RelationSummary { … }).collect() }`.

- [x] Write unit tests in `relation_service.rs` `#[cfg(test)]` block using `MemoryStore` (and `JsonStore` for roundtrip). All 6 tests must pre-populate the manifest `instance_index` with the relevant IDs (so E1 validation passes):
  - `test_rebuild_precedes_chain_creates_n_minus_1_edges` — 3 IDs → 2 `precedes` edges, correct source/target order.
  - `test_rebuild_precedes_chain_clears_existing_precedes` — pre-populate store with existing `precedes` edges among `clear_ids`, call rebuild, confirm old edges removed and only new edges for `instance_ids` remain.
  - `test_rebuild_precedes_chain_empty_instance_ids` — `instance_ids: []` → `created: []`, no edges written.
  - `test_rebuild_precedes_chain_single_instance_id` — `instance_ids: [x]` → `created: []`, no edges written.
  - `test_rebuild_precedes_chain_does_not_clear_non_precedes` — non-`precedes` edges involving `clear_ids` IDs are preserved.
  - `test_rebuild_precedes_chain_roundtrip_json_store` — call `rebuild_precedes_chain` on `MemoryStore` with 3 IDs, export via `to_srsj_string()`, reload with `JsonStore::from_srsj`, call `list_relations` and assert exactly 2 `precedes` edges survive with correct source/target.

#### Acceptance Criteria

- [x] `pub struct RebuildPrecedesChainInput` with `#[derive(Debug, Clone)]` exists in `relation_service.rs`.
- [x] `pub struct RebuildPrecedesChainResult` with `#[derive(Debug, Clone, serde::Serialize)]` and `#[serde(rename_all = "camelCase")]` exists.
- [x] `pub(crate) fn load_validation_data` exists; `create_relation` is updated to use it (no duplication).
- [x] All 6 named tests pass.
- [x] The final relations collection after `rebuild_precedes_chain([a, b, c], clear_ids=[a,b,c])` contains exactly 2 `precedes` edges: `a→b` and `b→c`, and all pre-existing non-`precedes` edges are preserved.
- [x] If any relation fails E1–E4 validation, the call returns `RepositoryError::RelationValidation` and no write occurs (by virtue of the single-write-at-end structure).
- [x] Roundtrip test passes: edges written on a `JsonStore` survive serialize→deserialize via `to_srsj_string` / `from_srsj`.

#### Testing

```bash
cargo test -p srs-repository test_rebuild_precedes_chain
```

Specific tests to write or verify:
- `test_rebuild_precedes_chain_creates_n_minus_1_edges`
- `test_rebuild_precedes_chain_clears_existing_precedes`
- `test_rebuild_precedes_chain_empty_instance_ids`
- `test_rebuild_precedes_chain_single_instance_id`
- `test_rebuild_precedes_chain_does_not_clear_non_precedes`
- `test_rebuild_precedes_chain_roundtrip_json_store`

#### Milestone gate

1. All 7 acceptance criteria checked.
2. All 6 named tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Update plan checkboxes `[x]`.
5. Commit: `feat(relation-service): rebuild_precedes_chain service function (#606)`.

---

### Phase 2: WASM binding in srs-bindings

**Goal:** `SrsRepository::rebuild_precedes_chain(input_json)` exists in `crates/srs-bindings/src/lib.rs`, compiles to `wasm32-unknown-unknown`, and passes a smoke test.

**Agent:** Bindings Worker

#### Tasks

- [x] Add `RebuildPrecedesChainInput` to the `relation_service` import in `crates/srs-bindings/src/lib.rs` (amend the existing import line at line 36).

- [x] Add the WASM method to `#[wasm_bindgen] impl SrsRepository`:
  ```rust
  /// Atomically rebuild a linear `precedes` chain.
  ///
  /// `input_json` is `{ "instanceIds": ["uuid1", ...], "clearIds": ["uuid1", ...] }`.
  /// All `precedes` edges where source OR target is in `clearIds` are deleted first;
  /// then `n-1` new `precedes` edges connect `instanceIds[0]→[1]→…→[n-1]`.
  ///
  /// Returns `{ "created": [<RelationSummary>, ...] }` as a JS value where each
  /// `RelationSummary` is `{ "relationId", "relationType", "sourceId", "targetId" }`.
  pub fn rebuild_precedes_chain(&self, input_json: &str) -> Result<JsValue, JsValue> {
      #[derive(serde::Deserialize)]
      #[serde(rename_all = "camelCase")]
      struct Input {
          instance_ids: Vec<String>,
          clear_ids: Vec<String>,
      }
      let parsed: Input = serde_json::from_str(input_json)
          .map_err(|e| js_err(format!("invalid input: {e}")))?;
      let result = relation_service::rebuild_precedes_chain(
          &self.store,
          RebuildPrecedesChainInput {
              instance_ids: parsed.instance_ids,
              clear_ids: parsed.clear_ids,
          },
      )
      .map_err(js_err)?;
      to_js(&result)
  }
  ```

- [x] Add smoke test `test_rebuild_precedes_chain_binding_smoke` in `crates/srs-bindings/src/lib.rs` `#[cfg(test)]` block. The test must:
  1. Build a `JsonStore` from a minimal `.srsj` string that includes 3 instance IDs (`"id-a"`, `"id-b"`, `"id-c"`) in `manifest.instanceIndex` (follow the pattern of `srsj_with_note_and_type()` or construct a minimal seed inline).
  2. Call `relation_service::rebuild_precedes_chain(&store, RebuildPrecedesChainInput { instance_ids: vec!["id-a", "id-b", "id-c"], clear_ids: vec![] })`.
  3. Assert `result.created.len() == 2`.
  4. Assert `result.created[0].source_id == "id-a"` and `result.created[0].target_id == "id-b"`.
  5. Assert `result.created[1].source_id == "id-b"` and `result.created[1].target_id == "id-c"`.

- [x] Verify WASM compilation:
  ```bash
  cargo build --target wasm32-unknown-unknown -p srs-bindings
  ```

#### Acceptance Criteria

- [x] `rebuild_precedes_chain` is a `pub fn` on `impl SrsRepository` in `crates/srs-bindings/src/lib.rs`.
- [x] Input accepts `{ "instanceIds": [...], "clearIds": [...] }` (camelCase); invalid JSON returns a `JsValue` error via `js_err`.
- [x] Output shape is `{ "created": [{ "relationId", "relationType", "sourceId", "targetId" }, ...] }` (inherits `RelationSummary`'s camelCase serialisation).
- [x] `cargo build --target wasm32-unknown-unknown -p srs-bindings` exits 0.
- [x] Smoke test `test_rebuild_precedes_chain_binding_smoke` passes.

#### Testing

```bash
cargo build --target wasm32-unknown-unknown -p srs-bindings
cargo test -p srs-bindings test_rebuild_precedes_chain_binding_smoke
```

Specific tests to write or verify:
- `test_rebuild_precedes_chain_binding_smoke`

#### Milestone gate

1. All 5 acceptance criteria checked.
2. WASM build exits 0.
3. Run:
   ```bash
   cargo test -p srs-bindings
   cargo clippy -p srs-bindings -- -D warnings
   ```
4. Update plan checkboxes `[x]`.
5. Commit: `feat(srs-bindings): rebuild_precedes_chain WASM binding (#606)`.

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] `cargo test --test payload_contracts` passes (no payload structs changed)
- [x] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [x] `cargo build --target wasm32-unknown-unknown -p srs-bindings` succeeds
- [x] All 6 service-layer tests pass
- [x] Smoke test in srs-bindings passes

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Repository Service Worker writes only `crates/srs-repository/src/relation_service.rs`.
- Bindings Worker writes only `crates/srs-bindings/src/lib.rs`.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- The `precedes` relation type is registered in the repo's package (via the spec package). Validation via `validate_relation` will pass for repos that include the spec package (standard setup). For repos without any package definitions, E2 (unknown relation type) may fire; this is consistent with existing `create_relation_auto` behaviour.
- `clear_ids` semantics: edges where source OR target is in `clear_ids` AND `relationType == "precedes"` are removed. Non-`precedes` edges touching `clear_ids` are never removed.
- The WASM binding test validates service-level correctness; the `wasm32-unknown-unknown` build check validates wasm-bindgen correctness.
