# Plan: WASM list_tags binding (#303)

## Summary

`srs-web` needs to show all vocabulary terms (the available "tags") so users can browse by tag in the decision log. `tag_service::list_terms` already exists and `srs term list` already exposes it via the CLI. This plan adds `list_tags()` to the `SrsRepository` WASM binding — a thin wrapper that calls the service and returns the terms as a JS array. No business logic is introduced.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Bindings Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

_No new architectural decisions — this plan implements ADR-013 (WASM binding strategy) and ADR-010 (service boundary contract). The binding is a thin wrapper over an existing service, following the same pattern as `list_protocols`._

| ADR | Decision | Status |
|---|---|---|
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM bindings are thin wrappers — deserialize JS input, call one service, serialize output | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | No business logic in bindings; call the same services as the CLI | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. The WASM surface does not use `payload.rs`. `cargo test --test payload_contracts` is unaffected.

### Entity schema sync (check-schema-sync.sh)

No schema files changed. No sync action required.

---

## Scope

- Add `pub fn list_tags(&self) -> Result<JsValue, JsValue>` to `SrsRepository` in `crates/srs-bindings/src/lib.rs`.
- The method calls `tag_service::list_terms(&self.store)` and returns `Vec<Term>` as a JS array.
- Add an import for `srs_repository::tag_service` in `lib.rs`.
- Add a smoke test in `crates/srs-bindings/tests/` (or extend an existing integration test) confirming the binding returns valid JSON.

**Out of scope:**

- "All tags in use" (scanning `instanceIndex` for unique tag strings) — the issue explicitly asks to bind `tag_service::list_terms`; a distinct "tags in use" query is a separate concern.
- CLI changes — `srs term list` already covers this surface.
- Any change to `tag_service` itself — the service is complete.

---

## Phases

### Phase 1: Add `list_tags` WASM binding

**Goal:** `SrsRepository.list_tags()` is callable from JS and returns the vocabulary terms as a JSON-serializable array.

**Agent:** Bindings Worker

#### Tasks

- [ ] In `crates/srs-bindings/src/lib.rs`, add `use srs_repository::tag_service;` to the import block.
- [ ] Add the following method to `impl SrsRepository`:

  ```rust
  /// List all vocabulary Terms (RFC-006) defined in the package.
  /// Returns a JS array of `Term` objects — the same terms returned by `srs term list`.
  /// srs-web uses this to populate the tag picker / tag cloud.
  pub fn list_tags(&self) -> Result<JsValue, JsValue> {
      let terms = tag_service::list_terms(&self.store).map_err(js_err)?;
      to_js(&terms)
  }
  ```

- [ ] Verify `cargo build -p srs-bindings` passes.
- [ ] Create `crates/srs-bindings/tests/tags.rs` with two tests. **Important:** native tests must call the underlying service directly — `to_js()` calls `js_sys::JSON::parse` which panics outside a WASM runtime (see module-level comment in `blueprints.rs`). Do NOT call `list_tags()` from these native tests.

  **Test 1** — gallery fixture returns empty slice (gallery carries no vocabulary):
  ```rust
  use srs_repository::{tag_service, JsonStore};

  fn gallery_store() -> JsonStore {
      let srsj = include_str!("fixtures/gallery.srsj");
      JsonStore::from_srsj(srsj).expect("gallery srsj must load")
  }

  #[test]
  fn list_tags_empty_on_gallery() {
      let store = gallery_store();
      let terms = tag_service::list_terms(&store).expect("list_terms must succeed");
      assert!(terms.is_empty(), "gallery carries no vocabulary");
  }
  ```

  **Test 2** — inline `.srsj` with one vocabulary file carrying one Term:
  ```rust
  fn vocab_srsj() -> String {
      serde_json::json!({
          "srsj": "1",
          "manifest": {
              "repositoryId": "test-repo-vocab",
              "srsVersion": "2.0-draft",
              "namespace": "com.test",
              "instanceIndex": [],
              "packageRef": {"mode": "local", "path": "package"}
          },
          "data": {
              "package/package.json": {
                  "id": "pkg-vocab-001",
                  "namespace": "com.test",
                  "name": "test-pkg",
                  "version": "1.0.0",
                  "fields": [],
                  "types": [],
                  "relationTypes": [],
                  "views": [],
                  "documentViews": [],
                  "vocabularies": ["vocabularies/tags.json"]
              },
              "package/vocabularies/tags.json": {
                  "namespace": "com.test",
                  "name": "tags",
                  "version": "1.0.0",
                  "terms": [
                      {
                          "id": "term-001",
                          "version": 1,
                          "namespace": "com.test",
                          "key": "category:core"
                      }
                  ]
              }
          }
      }).to_string()
  }

  #[test]
  fn list_tags_returns_terms_from_vocabulary() {
      let store = JsonStore::from_srsj(&vocab_srsj()).expect("vocab srsj must load");
      let terms = tag_service::list_terms(&store).expect("list_terms must succeed");
      assert_eq!(terms.len(), 1, "one term registered");
      assert_eq!(terms[0].key, "category:core");
      assert_eq!(terms[0].namespace, "com.test");
  }
  ```

#### Acceptance Criteria

- [ ] `cargo build -p srs-bindings` passes.
- [ ] `cargo test -p srs-bindings` passes (including the new test).
- [ ] `cargo clippy -p srs-bindings -- -D warnings` passes.
- [ ] The `list_tags` method is the only change to `lib.rs` beyond the import.
- [ ] No business logic is introduced — the method body is one service call.

#### Testing

```bash
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

Specific tests:

- `bindings::list_tags_returns_json_array` — loads a `.srsj` fixture, calls `list_tags()`, verifies the JS value can be round-tripped through `serde_json` as an array.

#### Milestone gate

1. All acceptance criteria above are met.
2. `cargo test -p srs-bindings` passes.
3. `cargo clippy -p srs-bindings -- -D warnings` passes.
4. Commit: `feat(wasm): expose list_tags binding (#303)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `SrsRepository.list_tags()` is exposed in `crates/srs-bindings/src/lib.rs`
- [ ] New binding calls `tag_service::list_terms` with no in-binding business logic
- [ ] A test exercises the new binding against a fixture repo

## Coordination Rules

- Bindings Worker writes only to `crates/srs-bindings/`.
- No changes to `srs-repository`, `srs-core`, or `srs-cli`.
- Lead Integrator reviews the final diff for ADR-013 / ADR-010 compliance.

## Assumptions

- `tag_service::list_terms` is already re-exported from `srs_repository` (confirmed: `srs-repository/src/lib.rs` exports `tag_service`).
- Existing WASM tests use a fixture `.srsj` path — find the path used in existing tests to reuse it.
