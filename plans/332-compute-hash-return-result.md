# Plan: compute_package_content_hash should return Result<String>

## Summary

`srsj_migration_service::compute_package_content_hash` currently returns `String` and silently hashes the string `"null"` when `data["package/package.json"]` is absent (because `serde_json::to_string(&Value::Null)` returns `"null"`). A malformed `.srsj` bundle therefore produces a hash rather than an error, hiding bad input. This plan changes the return type to `Result<String, RepositoryError>`, returns an explicit error when the key is missing or null, propagates it in the one call site (`migrate_rfc014`), and adds a test for the new error path.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification Agent | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan implements ADR-010 (validation belongs in the service, not the caller). The change makes the service correctly reject invalid input rather than silently producing a wrong hash.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service validates input; callers must not receive misleading success on bad data | accepted |

---

## Contracts

### CLI output contract (ADR-011)

**No new/changed commands.** `compute_package_content_hash` is an internal service helper not exposed through any CLI handler or payload struct. No golden schema changes required.

### Entity schema sync (check-schema-sync.sh)

**No** — no JSON Schema files are added or modified.

---

## Scope

- Change `compute_package_content_hash` signature to `pub fn compute_package_content_hash(data: &serde_json::Value) -> Result<String, RepositoryError>`.
- Return `Err(RepositoryError::InvalidRepositoryInitialization { message: "package/package.json absent in srsj data".to_string() })` when `data["package/package.json"]` is null or missing.
- Propagate the error in the single call site: `migrate_rfc014` (use `?`).
- Update the existing test `compute_package_content_hash_returns_sha256_prefix` to `.expect(...)` the `Result`.
- Add a new test `compute_package_content_hash_errors_on_missing_package_key`.

**Out of scope:**
- Any other `unwrap_or_default()` calls deeper in the hash loop (those hash absent definition files as empty strings, which is consistent with the RFC-014 spec — only the top-level `package/package.json` absence is the invariant violation).
- Exposing the function through CLI or WASM (it is a private helper; the public entry points are `load_from_srsj` and `migrate_rfc014`).

---

## Phases

### Phase 1: Fix signature and propagate error

**Goal:** `compute_package_content_hash` returns `Result<String, RepositoryError>`, all call sites compile and propagate the error, and all tests pass.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/srsj_migration_service.rs`, change the return type of `compute_package_content_hash` from `String` to `Result<String, RepositoryError>`.
- [ ] At the top of the function body, check `data["package/package.json"].is_null()` and return `Err(RepositoryError::InvalidRepositoryInitialization { message: "package/package.json absent in srsj data".to_string() })` if true. (Indexing a `Value` with a missing key yields `Value::Null`, so this covers both the absent key and an explicit null.)
- [ ] Change `hasher.update(serde_json::to_string(pkg).unwrap_or_default().as_bytes())` to `hasher.update(serde_json::to_string(pkg).map_err(|e| RepositoryError::Serialize { path: std::path::PathBuf::from("<package/package.json>"), source: e })?.as_bytes())` — or more simply, since `to_string` on a valid `Value` is infallible, just use `serde_json::to_string(pkg).unwrap()` (it cannot fail on a non-null Value we already validated). Either is acceptable; prefer unwrap with a comment explaining it cannot fail after the null guard.
- [ ] Change the return at the bottom from `format!(...)` to `Ok(format!(...))`.
- [ ] In `migrate_rfc014`, update the call from `compute_package_content_hash(&seed["data"])` to `compute_package_content_hash(&seed["data"])?`.
- [ ] Update test `compute_package_content_hash_returns_sha256_prefix`: add `.expect("hash succeeds on valid data")` after the call.
- [ ] Add test `compute_package_content_hash_errors_on_missing_package_key`: pass a `serde_json::Value::Object(Default::default())` (empty object, no `package/package.json` key) and assert `matches!(result, Err(RepositoryError::InvalidRepositoryInitialization { .. }))`.

#### Acceptance Criteria

- [ ] `compute_package_content_hash` compiles with return type `Result<String, RepositoryError>`.
- [ ] Calling it with a valid `data` map (containing `package/package.json`) returns `Ok(hash)` where `hash.starts_with("sha256:")`.
- [ ] Calling it with an empty `serde_json::Value::Object` returns `Err(RepositoryError::InvalidRepositoryInitialization { .. })`.
- [ ] `migrate_rfc014` compiles and all existing migration tests pass unchanged.
- [ ] No `unwrap()` on the `compute_package_content_hash` result remains in production code.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `compute_package_content_hash_returns_sha256_prefix` — proves the happy path still works after the signature change.
- `compute_package_content_hash_errors_on_missing_package_key` — proves the error is returned (not a bogus hash) when the key is absent.
- `migrate_rfc014_moves_upstream_package_and_adds_content_hash` — regression: migration still succeeds on a valid bundle.
- `migrate_rfc014_rejects_invalid_json` — regression: existing error path unaffected.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm all four tests listed in Testing exist and pass.
3. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Mark completed checkboxes `[x]` and commit.

```bash
git commit
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `compute_package_content_hash` returns `Result<String, RepositoryError>`
- [ ] Missing `package/package.json` key returns an explicit error, not a bogus hash

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Verification Agent runs after Phase 1 completes.

## Assumptions

- `RepositoryError::InvalidRepositoryInitialization { message }` is the correct variant for rejecting a structurally invalid `.srsj` bundle (it is already used for similar invariant violations in the codebase).
- `compute_package_content_hash` is not `pub(crate)` exposed beyond `srsj_migration_service`; it is tested directly in the same file's `#[cfg(test)]` block.
