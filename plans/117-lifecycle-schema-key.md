# Plan: Accept `$schema` in Lifecycle and Vocabulary loaders (#117)

## Summary

The `Lifecycle` deserializer uses `deny_unknown_fields` and rejects a top-level
`$schema` key (the conventional JSON Schema editor-hint field), while `Field`
and `RecordType` use `#[serde(flatten)] pub extra: HashMap<…>` and silently
absorb it. This inconsistency forces users to omit `$schema` from lifecycle
files even though they use it on field/type files. The fix extends the same
tolerance pattern to every standalone package entity file that currently has
`deny_unknown_fields`: `Lifecycle` and `Vocabulary`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Core Model Worker | — |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-004](../docs/adr/004-schemas-embedded-at-compile-time.md) | `$schema` is a valid editor hint on entity files; Rust types must not reject it | accepted |
| [ADR-009](../docs/adr/009-package-boundary-model.md) | Standalone package entity files (`Field`, `RecordType`, `Lifecycle`, `Vocabulary`) are loaded by the package boundary model | accepted |

No new ADRs are needed. The change is an implementation consistency fix: it aligns `Lifecycle` and `Vocabulary` with the existing tolerance pattern already established by `Field` and `RecordType`.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands, flags, stdin shapes, or payload structs.
No action required. `cargo test --test payload_contracts` must still pass.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/` entity schemas. The `$schema` key is
already permitted in the canonical JSON Schema files (it is `additionalProperties`
safe there). No sync action required.

---

## Scope

- Remove `deny_unknown_fields` from `Lifecycle` struct in
  `crates/srs-core/src/types/lifecycle.rs`.
- Add `#[serde(flatten)] pub extra: HashMap<String, serde_json::Value>` to
  `Lifecycle`, matching the `Field` and `RecordType` pattern exactly (no
  `skip_serializing_if` — the flatten mechanism automatically omits empty maps).
- Remove `deny_unknown_fields` from `Vocabulary` struct in
  `crates/srs-core/src/types/vocabulary.rs`.
- Add `use std::collections::HashMap;` import to `vocabulary.rs` (not currently
  present; lifecycle.rs already has it).
- Add `#[serde(flatten)] pub extra: HashMap<String, serde_json::Value>` to
  `Vocabulary`.
- Remove or rewrite the `vocabulary_deny_unknown_fields` test (vocabulary.rs:201)
  which currently asserts unknown fields are rejected — this expectation inverts
  after the fix.
- Add regression tests for both types confirming `$schema` is accepted and
  that serialized output does not emit an `extra` key.

**Out of scope:**
- `Term` (`term.rs:29`) — has `deny_unknown_fields` but is an embedded
  sub-structure within `Vocabulary.terms[]`; top-level `$schema` cannot appear
  on Term objects (they are array items, not standalone entity files). No change.
- `PromotionWindow` (`vocabulary.rs:12`) — embedded within `Vocabulary` as
  `Vocabulary.promotion_window`; never appears as a standalone entity file.
  `deny_unknown_fields` is left in place intentionally.
- `Relation` — items within a relations collection array; `$schema` appears at
  the collection level, not on individual Relation objects.
- Any CLI command changes, payload changes, or entity schema changes.

---

## Phases

### Phase 1: Fix `Lifecycle` and `Vocabulary` serde shapes

**Goal:** Both types accept and silently absorb `$schema` (and any other
unknown key) when deserializing from JSON; serialized output contains no
spurious `extra` key.

**Agent:** Core Model Worker

#### Tasks

Pre-task scope check:
- [x] Run `grep -rn 'deny_unknown_fields' crates/srs-core/src/types/` and verify all hits
  are accounted for in this plan's scope: `Lifecycle` (fix), `Vocabulary` (fix),
  `Term` (out of scope — embedded), `PromotionWindow` (out of scope — embedded),
  `Relation` (out of scope — collection item). Document any unexpected hits.

`crates/srs-core/src/types/lifecycle.rs`:
- [x] Change `#[serde(rename_all = "camelCase", deny_unknown_fields)]` on `Lifecycle`
  to `#[serde(rename_all = "camelCase")]` (remove `deny_unknown_fields`).
- [x] `HashMap` is already imported at the top of this file (used by
  `LifecycleState.properties`) — no import change needed. Verify.
- [x] Add field at the end of `Lifecycle`:
  ```rust
  #[serde(flatten)]
  pub extra: HashMap<String, serde_json::Value>,
  ```
  Match `Field` (field.rs:22-23) exactly: no `skip_serializing_if`. The flatten
  mechanism automatically emits nothing for an empty map.
- [x] Add test `lifecycle_accepts_schema_key` in the `#[cfg(test)]` block:
  ```rust
  #[test]
  fn lifecycle_accepts_schema_key() {
      let json = r#"{
          "$schema": "https://srs.semanticops.com/schema/2.0/lifecycle.json",
          "id": "lc-test",
          "version": 1,
          "namespace": "com.test",
          "name": "test-lc",
          "states": [],
          "transitions": [],
          "initialState": "draft",
          "createdAt": "2026-01-01T00:00:00Z"
      }"#;
      let lc: Lifecycle = serde_json::from_str(json).expect("must accept $schema");
      assert_eq!(lc.id, "lc-test");
      // $schema is silently absorbed; serialized form must not emit "extra"
      let serialized = serde_json::to_string(&lc).unwrap();
      assert!(!serialized.contains("\"extra\""),
          "flatten must not emit an 'extra' key");
  }
  ```

`crates/srs-core/src/types/vocabulary.rs`:
- [x] Add `use std::collections::HashMap;` import at the top (currently absent).
- [x] Change `#[serde(rename_all = "camelCase", deny_unknown_fields)]` on `Vocabulary`
  to `#[serde(rename_all = "camelCase")]`.
- [x] Add field at the end of `Vocabulary`:
  ```rust
  #[serde(flatten)]
  pub extra: HashMap<String, serde_json::Value>,
  ```
- [x] Remove the `vocabulary_deny_unknown_fields` test (vocabulary.rs:201-204):
  it currently asserts unknown fields cause an error, which is the opposite of
  the intended post-fix behaviour. Delete the entire test function.
- [x] Add test `vocabulary_accepts_schema_key` in the `#[cfg(test)]` block:
  ```rust
  #[test]
  fn vocabulary_accepts_schema_key() {
      let json = r#"{
          "$schema": "https://srs.semanticops.com/schema/2.0/vocabulary.json",
          "id": "v-test",
          "version": 1,
          "namespace": "com.test",
          "name": "test-vocab",
          "mode": "open",
          "terms": [],
          "createdAt": "2026-01-01T00:00:00Z"
      }"#;
      let v: Vocabulary = serde_json::from_str(json).expect("must accept $schema");
      assert_eq!(v.id, "v-test");
      let serialized = serde_json::to_string(&v).unwrap();
      assert!(!serialized.contains("\"extra\""),
          "flatten must not emit an 'extra' key");
  }
  ```
- [x] Add test `vocabulary_absorbs_unknown_fields` to confirm non-`$schema`
  unknown fields are also tolerated (symmetric with `field_extra_fields_survive_roundtrip`):
  ```rust
  #[test]
  fn vocabulary_absorbs_unknown_fields() {
      let json = r#"{
          "id": "v-test",
          "version": 1,
          "namespace": "com.test",
          "name": "test-vocab",
          "mode": "open",
          "terms": [],
          "createdAt": "2026-01-01T00:00:00Z",
          "futureExtension": "some-value"
      }"#;
      let v: Vocabulary = serde_json::from_str(json).expect("unknown fields must be absorbed");
      assert_eq!(v.id, "v-test");
  }
  ```
- [x] Confirm `PromotionWindow` in vocabulary.rs still has `deny_unknown_fields`
  (must not have been changed).

#### Acceptance Criteria

- [x] `cargo test -p srs-core` passes with no failures.
- [x] `lifecycle_accepts_schema_key` test exists and passes.
- [x] `vocabulary_accepts_schema_key` test exists and passes.
- [x] `vocabulary_absorbs_unknown_fields` test exists and passes.
- [x] `vocabulary_deny_unknown_fields` test has been removed.
- [x] Existing `lifecycle_roundtrips_json` and `lifecycle_state_substrate_fields_roundtrip` tests still pass.
- [x] `cargo clippy -p srs-core -- -D warnings` passes.
- [x] `PromotionWindow` in vocabulary.rs still has `deny_unknown_fields` (unchanged).
- [x] Serialized `Lifecycle` from JSON containing `$schema` does not emit an `extra` key.
- [x] Serialized `Vocabulary` from JSON containing `$schema` does not emit an `extra` key.

#### Testing

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

Specific tests:
- `lifecycle_accepts_schema_key` — proves `$schema` is silently absorbed by `Lifecycle` and does not appear in serialized output
- `vocabulary_accepts_schema_key` — proves `$schema` is silently absorbed by `Vocabulary` and does not appear in serialized output
- `vocabulary_absorbs_unknown_fields` — proves non-`$schema` unknown fields are also absorbed
- `lifecycle_roundtrips_json` — proves existing behaviour is unbroken

Note: `vocabulary_deny_unknown_fields` must be deleted; it asserts the opposite of the intended post-fix behaviour.

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm tests `lifecycle_accepts_schema_key`, `vocabulary_accepts_schema_key`,
   `vocabulary_absorbs_unknown_fields` exist and pass.
3. Confirm `vocabulary_deny_unknown_fields` is gone.
4. Run:

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

5. Update plan checkboxes to `[x]`.
6. Commit: `fix: accept $schema in Lifecycle and Vocabulary loaders (#117)`

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] `cargo test --test payload_contracts` passes (no payload structs changed)
- [x] `lifecycle_accepts_schema_key` test exists and passes
- [x] `vocabulary_accepts_schema_key` test exists and passes
- [x] `vocabulary_absorbs_unknown_fields` test exists and passes
- [x] `vocabulary_deny_unknown_fields` test has been removed
- [x] Existing lifecycle and vocabulary tests pass without modification

## Coordination Rules

- Core Model Worker writes only in `crates/srs-core/src/types/lifecycle.rs` and
  `crates/srs-core/src/types/vocabulary.rs`.
- No business logic changes. No CLI changes. No payload changes.
- Verification Agent confirms tests pass and no duplication is introduced.

## Assumptions

- `HashMap` is already imported in `lifecycle.rs` (it is, via the existing
  `LifecycleState.properties` field). Do not add a duplicate import.
- `HashMap` is NOT imported in `vocabulary.rs`; the import must be added.
- The `flatten + extra` pattern is the established project idiom for
  "tolerate unknown fields on top-level entity types" — confirmed by `Field`
  and `RecordType`. No `skip_serializing_if` attribute is needed on flattened
  HashMap fields; the flatten mechanism handles empty maps automatically.
- `PromotionWindow` and `Term` are intentionally left with `deny_unknown_fields`
  as they are not top-level entity files.
