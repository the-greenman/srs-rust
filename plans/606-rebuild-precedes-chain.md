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
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | New service function `rebuild_precedes_chain` takes typed input struct, performs all orchestration, writes atomically in one collection write | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM binding is a thin `SrsRepository` method — deserialise JS input → one service call → serialise output | accepted |
| [ADR-015](../docs/adr/015-wasm-write-and-export.md) | Write mutation via in-memory `JsonStore`, no filesystem access | accepted |
| [ADR-024](../docs/adr/024-best-effort-rollback-multi-write-services.md) | Single collection write (load-mutate-write), naturally atomic | accepted |

No new ADRs required — all decisions follow established patterns.

---

## Contracts

### CLI output contract (ADR-011)

No CLI command is added or changed. The `rebuild_precedes_chain` service is exposed only via the WASM binding. No payload struct changes, no `generate-schemas` run needed.

### Entity schema sync (check-schema-sync.sh)

No schema files in `srs/docs/schema/2.0/` are modified. No sync required.

---

## Scope

- Add `rebuild_precedes_chain(store, input)` to `crates/srs-repository/src/relation_service.rs`
- Add `rebuild_precedes_chain(input_json)` WASM method to `crates/srs-bindings/src/lib.rs`
- Unit tests for the service function using `MemoryStore`
- Smoke test verifying WASM binding compiles (`cargo build --target wasm32-unknown-unknown -p srs-bindings`)

**Out of scope:**
- Any srs-web changes (that is follow-on work in the srs-web repo)
- CLI command for `rebuild_precedes_chain` (not needed — CLI callers can sequence `relation delete` + `relation create` themselves)
- Non-`precedes` chain rebuilding

---

## Phases

### Phase 1: Service function in srs-repository

**Goal:** `relation_service::rebuild_precedes_chain` exists, passes all tests, and performs a single atomic collection write.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Define `RebuildPrecedesChainInput` struct in `crates/srs-repository/src/relation_service.rs`:
  ```rust
  pub struct RebuildPrecedesChainInput {
      /// Desired linear order — edges created as instance_ids[0]→[1]→…→[n-1].
      pub instance_ids: Vec<String>,
      /// IDs whose existing `precedes` edges (source OR target) are deleted first.
      pub clear_ids: Vec<String>,
  }
  ```
- [ ] Define `RebuildPrecedesChainResult` struct in the same file:
  ```rust
  #[derive(Debug, Clone, serde::Serialize)]
  #[serde(rename_all = "camelCase")]
  pub struct RebuildPrecedesChainResult {
      pub created: Vec<RelationSummary>,
  }
  ```
- [ ] Implement `pub fn rebuild_precedes_chain(store: &dyn RepositoryStore, input: RebuildPrecedesChainInput) -> Result<RebuildPrecedesChainResult, RepositoryError>` in `relation_service.rs`. The function must:
  1. Load the relations collection once with `load_relations_collection(store)`.
  2. Retain all relations where: `relationType != "precedes"` OR neither source nor target is in `clear_ids` (i.e. only remove `precedes` edges that touch `clear_ids`).
  3. Build `n-1` new `Relation` objects: `instanceIds[i] precedes instanceIds[i+1]`, each with a fresh UUID from `new_instance_id()`.
  4. Validate each new relation using `create_relation`'s validation path: build `RelationValidationContext` from manifest instance index + `build_instance_semantic_types`, call `validate_relation` for each new relation.
  5. Append the new relations to the collection, run `SchemaRegistry::global().validate_by_id(RELATIONS_COLLECTION_SCHEMA_ID, ...)` on the full collection, then call `write_relations_collection(store, &relative_path, &collection)` exactly once.
  6. Return `RebuildPrecedesChainResult { created: Vec<RelationSummary> }` for the new edges.
- [ ] Write unit tests in `relation_service.rs` `#[cfg(test)]` block using `MemoryStore`:
  - `test_rebuild_precedes_chain_creates_n_minus_1_edges` — 3 IDs → 2 `precedes` edges, correct source/target order.
  - `test_rebuild_precedes_chain_clears_existing_precedes` — pre-populate store with existing `precedes` edges among `clear_ids`, call rebuild, confirm old edges gone and only new edges for `instance_ids` remain.
  - `test_rebuild_precedes_chain_empty_instance_ids` — `instance_ids: []` → `created: []`, no edges written.
  - `test_rebuild_precedes_chain_single_instance_id` — `instance_ids: [x]` → `created: []`, no edges written.
  - `test_rebuild_precedes_chain_does_not_clear_non_precedes` — non-`precedes` edges involving `clear_ids` are preserved.

#### Acceptance Criteria

- [ ] `rebuild_precedes_chain` takes `RebuildPrecedesChainInput` and returns `Result<RebuildPrecedesChainResult, RepositoryError>`.
- [ ] All 5 named tests pass.
- [ ] The function performs exactly **one** `write_relations_collection` call per invocation (verified by MemoryStore state being correct after one call).
- [ ] Non-`precedes` relations are never removed even when their IDs appear in `clear_ids`.
- [ ] `validate_relation` is called for each new edge; validation errors surface as `RepositoryError::RelationValidation`.

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

#### Milestone gate

1. All 5 acceptance criteria checked.
2. All 5 named tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Update plan checkboxes `[x]`.
5. Commit: `feat(relation-service): rebuild_precedes_chain service function (#606)`.

---

### Phase 2: WASM binding in srs-bindings

**Goal:** `SrsRepository::rebuild_precedes_chain(input_json)` exists in `crates/srs-bindings/src/lib.rs`, compiles to WASM, and is exercised by a smoke test.

**Agent:** Bindings Worker

#### Tasks

- [ ] Add `use srs_repository::relation_service::{self, ..., RebuildPrecedesChainInput, RebuildPrecedesChainResult}` import to `crates/srs-bindings/src/lib.rs` (amend the existing `relation_service` import line).
- [ ] Add the WASM method to `impl SrsRepository`:
  ```rust
  /// Atomically rebuild a linear `precedes` chain.
  ///
  /// `input_json` is `{ "instanceIds": ["uuid1", ...], "clearIds": ["uuid1", ...] }`.
  /// All `precedes` edges where source OR target is in `clearIds` are deleted; then
  /// `n-1` new `precedes` edges are created connecting `instanceIds[0]→[1]→…→[n-1]`.
  ///
  /// Returns `{ "created": [<RelationSummary>, ...] }` as a JS value.
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
- [ ] Verify the binding compiles for the WASM target:
  ```bash
  cargo build --target wasm32-unknown-unknown -p srs-bindings
  ```
- [ ] Add a smoke test in `crates/srs-bindings/src/lib.rs` or a dedicated test file that exercises the binding via the service function directly (WASM bindings are exercised by compiling; functional correctness is covered by Phase 1 service tests):
  - `test_rebuild_precedes_chain_binding_smoke` — create a `JsonStore` from a minimal `.srsj` seed, call `relation_service::rebuild_precedes_chain` with 3 IDs in `instance_ids`, verify 2 edges returned.

#### Acceptance Criteria

- [ ] `rebuild_precedes_chain` is a `#[wasm_bindgen]` method on `SrsRepository` in `crates/srs-bindings/src/lib.rs`.
- [ ] Input is `{ "instanceIds": [...], "clearIds": [...] }` (camelCase); invalid JSON returns a JS error.
- [ ] Output is `{ "created": [<RelationSummary>, ...] }` — same camelCase shape as `RelationSummary`.
- [ ] `cargo build --target wasm32-unknown-unknown -p srs-bindings` succeeds.
- [ ] Smoke test passes.

#### Testing

```bash
cargo build --target wasm32-unknown-unknown -p srs-bindings
cargo test -p srs-bindings
```

Specific tests to write or verify:
- `test_rebuild_precedes_chain_binding_smoke`

#### Milestone gate

1. All 5 acceptance criteria checked.
2. WASM build succeeds.
3. Run:
   ```bash
   cargo test -p srs-bindings
   cargo clippy -p srs-bindings -- -D warnings
   ```
4. Update plan checkboxes `[x]`.
5. Commit: `feat(srs-bindings): rebuild_precedes_chain WASM binding (#606)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `cargo build --target wasm32-unknown-unknown -p srs-bindings` succeeds
- [ ] All 5 service-layer tests pass
- [ ] Smoke test in srs-bindings passes

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Repository Service Worker writes only `crates/srs-repository/src/relation_service.rs`.
- Bindings Worker writes only `crates/srs-bindings/src/lib.rs`.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- The `precedes` relation type is already registered in the repo's package (via the spec package). Validation via `validate_relation` will pass for repos that include the spec package (standard setup). For repos without any package definitions, E1/E4 checks may fail; this is consistent with the existing `create_relation_auto` behaviour.
- `clear_ids` semantics: edges where source OR target is in `clear_ids` and `relationType == "precedes"` are removed. Non-`precedes` edges touching `clear_ids` are never removed.
- The WASM binding test (smoke) validates service-level correctness; the WASM-target build check validates bindgen correctness.
