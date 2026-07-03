# Plan: Validate manifest.json Against Manifest Schema

## Summary

`srs repo validate` silently skips schema validation of `manifest.json`. The code in `validate_repository()` loads the manifest, parses it, and then discards it with `let _ = &manifest_value;` behind a TODO comment that said to re-enable once the manifest format was migrated. PR #312 completed that migration, so the TODO is now unblocked. This plan removes the stub and wires the existing `validate_value_against_schema()` helper to check `manifest.json` against `srs_schema::MANIFEST_SCHEMA_ID`, collecting violations into `diagnostics[]` as ERROR-severity entries.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator (sole worker) | Repository Service Worker |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions. This plan implements deferred work already planned under the validation architecture.

| ADR | Decision | Status |
|---|---|---|
| [ADR-008](../docs/adr/008-repository-lifecycle.md) | Repository lifecycle and validation contract | accepted |
| [ADR-010](../docs/adr/010-service-boundary.md) | Service boundary — validation logic belongs in `srs-repository` | accepted |

No new ADR is needed: removing a deferred TODO that reinstates already-designed behaviour is not a new architectural constraint.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. The `repo validate` command already emits `diagnostics[]` in its payload; adding manifest diagnostics is additive data, not a struct change. No `payload.rs` edits, no `generate-schemas` run required.

Verification: `cargo test --test payload_contracts` must still pass after the change.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are modified. No action required.

---

## Scope

- Remove the `let _ = &manifest_value;` stub and the TODO comment (lines 78–80 of `validation.rs`).
- Call `validate_value_against_schema(&manifest_value, "manifest.json", srs_schema::MANIFEST_SCHEMA_ID, reg)` and extend `diagnostics` with the result.
- Add three tests in `validation.rs`:
  1. Manifest missing a required field (`title`) → at least one ERROR diagnostic pointing at `manifest.json`.
  2. Manifest with an undeclared additional property → at least one ERROR diagnostic pointing at `manifest.json`.
  3. Valid manifest (as produced by `minimal_manifest()`) → no diagnostics from manifest validation.

**Out of scope:**

- Changes to any crate other than `srs-repository`.
- Changes to `payload.rs`, CLI handlers, or WASM bindings.
- Any schema file edits.

---

## Phases

### Phase 1: Enable Manifest Schema Validation

**Goal:** `validate_repository()` validates `manifest.json` against the manifest schema and collects any violations into `diagnostics[]`.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/validation.rs`, remove the TODO comment and `let _ = &manifest_value;` lines (approx. lines 78–80).
- [ ] Replace with:
  ```rust
  if let Some(report) = validate_value_against_schema(
      &manifest_value,
      "manifest.json",
      srs_schema::MANIFEST_SCHEMA_ID,
      reg,
  ) {
      diagnostics.extend(report);
  }
  ```
- [ ] Add test `test_validate_manifest_missing_title`: build a manifest via `minimal_manifest()` but remove the `title` field; assert the result contains at least one `ERROR` diagnostic with `relative_path == "manifest.json"`.
- [ ] Add test `test_validate_manifest_extra_property`: build a valid manifest and add an undeclared key (e.g. `"name": "foo"`); assert the result contains at least one `ERROR` diagnostic with `relative_path == "manifest.json"`.
- [ ] Add test `test_validate_manifest_valid`: use `minimal_manifest()` as-is; assert the result contains zero diagnostics whose `relative_path == "manifest.json"`.

#### Acceptance Criteria

- [x] `srs repo validate` on a repo with a manifest missing `title` → ERROR diagnostic with `relative_path: "manifest.json"`.
- [x] `srs repo validate` on a repo with an extra undeclared property in the manifest → ERROR diagnostic with `relative_path: "manifest.json"`.
- [x] `srs repo validate` on a repo with a valid manifest → zero diagnostics for `manifest.json`.
- [x] `srs repo validate --repo ../srs/srs` → still exits 0 with 0 errors (live spec repo remains clean).
- [x] `cargo test -p srs-repository` passes.
- [x] `cargo clippy -p srs-repository -- -D warnings` passes.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `test_validate_manifest_missing_title` — proves missing required field → ERROR diagnostic on `manifest.json`
- `test_validate_manifest_extra_property` — proves undeclared additional property → ERROR diagnostic on `manifest.json`
- `test_validate_manifest_valid` — proves a schema-compliant manifest → zero manifest diagnostics

#### Milestone gate

1. All acceptance criteria above are checked.
2. All three named tests exist and pass.
3. Run:
   ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Update plan checkboxes to `[x]`.
5. Commit:
   ```bash
   git commit -m "fix(srs-repository): validate manifest.json against manifest schema (#38)"
   ```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `srs repo validate --repo ../srs/srs` exits 0 with 0 errors
- [ ] Missing required manifest field → ERROR diagnostic in output
- [ ] Undeclared additional manifest property → ERROR diagnostic in output
- [ ] Valid manifest → no manifest diagnostics in output

## Coordination Rules

- Single worker: Repository Service Worker owns `crates/srs-repository/**` exclusively.
- No other crates are touched.
- Verification Agent runs final acceptance after Phase 1 before PR.

## Assumptions

- PR #312 has merged, providing fully schema-compliant manifests in all live repos. `minimal_manifest()` is already compliant.
- `srs_schema::MANIFEST_SCHEMA_ID` is registered in the schema registry (confirmed in `srs-schema/src/lib.rs`).
- The manifest schema enforces `title` as a required field and does not allow additional properties (to be confirmed by the test suite).
