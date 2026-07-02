# Plan: init_new_repository — WASM binding, CLI parity, service layer (#258)

## Summary

WS-1 (muDemocracy.org#38) published the canonical `com.mudemocracy.governance @1.0.0` package
and an empty governance-document seed carrying `meta.upstreamPackage` provenance. WS-2 (this
plan) adds the service function, CLI command, and WASM binding that let srs-web initialize a
fresh repository from that seed — re-stamping the identity (new UUID, caller-supplied namespace /
title) while preserving the installed package provenance. This unblocks WS-3 (srs-web onboarding
flow, muDemocracy.org#40).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | primary session agent |
| Repository Service Worker | primary session agent (Phase 1) |
| CLI Worker | primary session agent (Phase 2) |
| Bindings Worker | primary session agent (Phase 3) |
| Verification | primary session agent (milestone gates) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs required. Existing ADRs govern every decision:

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Typed `InitNewRepositoryInput` / `InitNewRepositoryResult` structs; all validation in service | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | `RepoInitNewPayload` struct in `payload.rs`; golden schema regenerated | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | WASM binding calls service via `&self`; no business logic in `srs-bindings` | accepted |
| [ADR-015](../docs/adr/015-wasm-write-and-export.md) | Ships the "future issue" deferred in ADR-015 for new-repo WASM creation; flip status to accepted | proposed → accepted in Stage 7.5 |

---

## Contracts

### CLI output contract (ADR-011)

New command `srs repo init-new` → new payload struct `RepoInitNewPayload` in
`crates/srs-cli/src/payload.rs`. After adding the struct, run:
```bash
cargo run --bin generate-schemas
```
Commit the new `crates/srs-cli/schemas/payload/RepoInitNewPayload.json`. Verification:
`cargo test --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No schema files change — `meta.upstreamPackage` / `UpstreamPackage` is already in the manifest
schema (landed by WS-1). `check-schema-sync.sh` requires no action.

---

## Scope

**In scope:**
- `InitNewRepositoryInput`, `InitNewRepositoryResult`, `init_new_repository()` in `crates/srs-repository/src/repository_lifecycle.rs`
- `RepoCommand::InitNew` variant in `crates/srs-cli/src/commands/mod.rs`
- `cmd_repo_init_new` handler in `crates/srs-cli/src/commands/repo.rs`
- `RepoInitNewPayload` struct in `crates/srs-cli/src/payload.rs` + golden schema regeneration
- `SrsRepository::init_new_repository` WASM binding in `crates/srs-bindings/src/lib.rs`
- Unit tests (MemoryStore + JsonStore roundtrip) in `repository_lifecycle.rs`
- ADR-015 status flip from `proposed` to `accepted`

**Out of scope:**
- WASM binary rebuild / artifact publish to `srs-web` (CI release pipeline post-merge)
- srs-web onboarding UI (WS-3, muDemocracy.org#40)
- Package upgrade / drift detection (future RFC-003 / srs#107)
- Migrating `srs-gov repo-create` to call the new service (separate follow-up)

---

## Phases

### Phase 1: Service — `init_new_repository` in `srs-repository`

**Goal:** A tested service function can re-stamp an initialized store's identity and update
`meta.upstreamPackage.installedAt`, ready for the CLI and WASM layers to call.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add to `crates/srs-repository/src/repository_lifecycle.rs` — input struct:

  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct InitNewRepositoryInput {
      pub repository_id: Option<String>,  // mint UUID4 if None
      pub namespace: String,
      pub title: String,
      pub description: Option<String>,
  }
  ```

- [ ] Add result struct to same file:

  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct InitNewRepositoryResult {
      pub repository_id: String,
      pub namespace: String,
      pub package_id: String,
      pub package_version: String,
  }
  ```

- [ ] Implement `pub fn init_new_repository(store: &dyn RepositoryStore, input: InitNewRepositoryInput) -> Result<InitNewRepositoryResult, RepositoryError>`:

  ```
  1. Validate: namespace.trim().is_empty() → Err(InvalidRepositoryInitialization { "namespace must not be empty" })
               title.trim().is_empty()     → Err(InvalidRepositoryInitialization { "title must not be empty" })
  2. repository_id = input.repository_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
     Note: UUID auto-mint lives here (not in the caller) so WASM and other non-CLI callers benefit.
  3. manifest = store.load_manifest()?
  4. manifest.extra.insert("repositoryId", Value::String(repository_id.clone()))
     manifest.extra.insert("namespace",    Value::String(input.namespace.clone()))
     manifest.extra.insert("title",        Value::String(input.title.clone()))
     if let Some(d) = input.description { manifest.extra.insert("description", Value::String(d)) }
  5. Update meta.upstreamPackage.installedAt using safe object mutation (NOT serde_json IndexMut —
     that panics on non-Object values):
       let meta_val = manifest.extra.get_mut("meta")
           .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
               reason: "meta.upstreamPackage is absent — store must be a seed with upstream provenance".into()
           })?;
       let upstream = meta_val.get_mut("upstreamPackage")
           .and_then(|v| v.as_object_mut())
           .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
               reason: "meta.upstreamPackage is absent or not an object".into()
           })?;
       upstream.insert("installedAt".to_string(), Value::String(chrono::Utc::now().to_rfc3339()));
  6. store.save_manifest(&manifest)?
  7. pkg = store.load_package()?
  8. return Ok(InitNewRepositoryResult { repository_id, namespace: input.namespace, package_id: pkg.id, package_version: pkg.version })
  ```

  Use `use chrono::Utc;` (already in `Cargo.toml`).

- [ ] Add five unit tests at the bottom of `repository_lifecycle.rs` (inside existing `#[cfg(test)]` block):

  - `init_new_repository_updates_identity_on_memory_store` — builds a `MemoryStore` with `meta.upstreamPackage` in `extra`, calls service, asserts `repository_id` ≠ seed value, `namespace` matches, `installedAt` is non-empty ISO-8601 string, other `upstreamPackage` fields preserved. Also asserts `description` is persisted when supplied.

  - `init_new_repository_roundtrips_via_json_store` — builds a seed `.srsj` JSON string (in-memory, with `meta.upstreamPackage`), loads via `JsonStore::from_srsj`, calls service, re-exports via `to_srsj_string`, parses JSON and asserts manifest fields + provenance preservation.

  - `init_new_repository_rejects_missing_upstream_package` — store without `meta.upstreamPackage` → `Err(InvalidRepositoryInitialization { .. })`.

  - `init_new_repository_rejects_empty_namespace` — `namespace: " ".to_string()` → `Err(InvalidRepositoryInitialization { .. })`.

  - `init_new_repository_rejects_empty_title` — `title: " ".to_string()` → `Err(InvalidRepositoryInitialization { .. })`.

#### Acceptance Criteria

- [ ] `init_new_repository` overwrites `repositoryId`, `namespace`, `title` (and `description` when provided)
- [ ] `meta.upstreamPackage` fields other than `installedAt` are unchanged
- [ ] `installedAt` is a non-empty ISO-8601 string after the call
- [ ] Error on missing `meta.upstreamPackage`
- [ ] Error on empty `namespace` or empty `title`
- [ ] All five unit tests pass

#### Testing

```bash
cargo test -p srs-repository init_new_repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `init_new_repository_updates_identity_on_memory_store`
- `init_new_repository_roundtrips_via_json_store`
- `init_new_repository_rejects_missing_upstream_package`
- `init_new_repository_rejects_empty_namespace`
- `init_new_repository_rejects_empty_title`

#### Milestone gate

1. All four tests pass: `cargo test -p srs-repository init_new_repository`
2. Clippy clean: `cargo clippy -p srs-repository -- -D warnings`
3. Mark checkboxes `[x]` above.
4. Commit: `feat(repository): add init_new_repository service (#258)`

---

### Phase 2: CLI command — `srs repo init-new`

**Goal:** `srs repo init-new --repo <path.srsj> --namespace <ns> --title <title>` re-stamps the
`.srsj` file in place and emits a JSON result envelope.

**Agent:** CLI Worker

#### Tasks

- [ ] Add `InitNew` variant to `RepoCommand` enum in `crates/srs-cli/src/commands/mod.rs` (after `Extensions`):

  ```rust
  /// Re-stamp a seed .srsj with a new repository identity.
  /// Modifies the file at --repo in place. Requires the seed to carry meta.upstreamPackage.
  #[command(name = "init-new")]
  InitNew {
      /// Repository ID (UUID); auto-generated if omitted
      #[arg(long = "repository-id")]
      repository_id: Option<String>,
      /// Repository namespace (reverse-DNS, e.g. com.myorg.governance)
      #[arg(long)]
      namespace: String,
      /// Repository title (display name)
      #[arg(long)]
      title: String,
      /// Repository description
      #[arg(long)]
      description: Option<String>,
  },
  ```

- [ ] Add `RepoInitNewPayload` to `crates/srs-cli/src/payload.rs`:

  ```rust
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct RepoInitNewPayload {
      pub repository_id: String,
      pub namespace: String,
      pub package_id: String,
      pub package_version: String,
  }
  ```

- [ ] Add `RepoCommand::InitNew` branch to `dispatch()` in `crates/srs-cli/src/commands/repo.rs`:

  ```rust
  RepoCommand::InitNew { repository_id, namespace, title, description } =>
      cmd_repo_init_new(ctx, repository_id, namespace, title, description),
  ```

- [ ] Add imports in `repo.rs`: `use srs_repository::repository_lifecycle::InitNewRepositoryInput;` and `use srs_repository::repository_lifecycle::init_new_repository;`.

- [ ] Implement `cmd_repo_init_new` in `commands/repo.rs` (must stay ≤ 15 lines per ADR-010).
  Use `with_store` (not `JsonStore::open` directly — that would violate CLAUDE.md's "no direct filesystem access in handlers" rule and produce misleading errors on FileStore paths):

  ```rust
  fn cmd_repo_init_new(
      ctx: CliContext,
      repository_id: Option<String>,
      namespace: String,
      title: String,
      description: Option<String>,
  ) -> Result<String> {
      let input = InitNewRepositoryInput { repository_id, namespace, title, description };
      let result = with_store(&ctx, |store| Ok(init_new_repository(store, input)?))?;
      output::serialize(
          "repo init-new",
          RepoInitNewPayload {
              repository_id: result.repository_id,
              namespace: result.namespace,
              package_id: result.package_id,
              package_version: result.package_version,
          },
      )
  }
  ```

- [ ] Add `RepoInitNewPayload` to the `use crate::payload::{ ... }` import block in `repo.rs`.

- [ ] Run `cargo run --bin generate-schemas` and stage `crates/srs-cli/schemas/payload/RepoInitNewPayload.json`.

#### Acceptance Criteria

- [ ] `srs repo init-new --repo /tmp/seed.srsj --namespace com.test --title "Test"` prints JSON envelope with `ok: true` and a `RepoInitNewPayload`
- [ ] Re-reading the `.srsj` file via `srs repo validate --repo /tmp/seed.srsj` returns 0 errors
- [ ] The file has a new `repositoryId` (UUID), the caller-supplied `namespace` and `title`
- [ ] `cargo test --test payload_contracts` passes (golden schema in sync)

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

Tests:
- `payload_contracts` (golden schema test covering `RepoInitNewPayload`)

Note: a dedicated CLI integration test `cmd_repo_init_new_modifies_seed_file_in_place` may be added if the srs-cli test suite has a pattern for file-mutation command tests. If no such pattern exists, the acceptance criteria are verified in dogfooding (Stage 7.6) instead.

#### Milestone gate

1. `cargo test --test payload_contracts` passes
2. `cargo clippy -p srs-cli -- -D warnings` clean
3. Mark checkboxes `[x]` above.
4. Commit: `feat(cli): add repo init-new command (#258)`

---

### Phase 3: WASM binding — `SrsRepository::init_new_repository`

**Goal:** srs-web can call `repo.init_new_repository(inputJson)` on a loaded `SrsRepository` to
re-stamp the seed identity entirely in-browser.

**Agent:** Bindings Worker

#### Tasks

- [ ] Add import at top of `crates/srs-bindings/src/lib.rs`:

  ```rust
  use srs_repository::repository_lifecycle::{self, InitNewRepositoryInput};
  ```

  Do NOT import `InitNewRepositoryResult` — `to_js<T: Serialize>` is generic and needs no explicit type annotation; importing it would produce an unused-import warning.

- [ ] Append `init_new_repository` inside the single existing `#[wasm_bindgen] impl SrsRepository` block in `crates/srs-bindings/src/lib.rs` — do NOT create a second `impl SrsRepository` block (it would compile silently but export nothing to WASM):

  ```rust
  /// Re-stamp the loaded seed repository with a new identity.
  ///
  /// `input_json`: `{ "repositoryId"?: "uuid", "namespace": "...", "title": "...", "description"?: "..." }`
  ///
  /// Returns `{ "repositoryId": "...", "namespace": "...", "packageId": "...", "packageVersion": "..." }`
  /// as a JS value. Errors if `meta.upstreamPackage` is absent from the manifest.
  pub fn init_new_repository(&self, input_json: &str) -> Result<JsValue, JsValue> {
      let input: InitNewRepositoryInput = serde_json::from_str(input_json)
          .map_err(|e| js_err(format!("invalid input: {e}")))?;
      let result = repository_lifecycle::init_new_repository(&self.store, input)
          .map_err(js_err)?;
      to_js(&result)
  }
  ```

- [ ] Verify `InitNewRepositoryResult` derives `Serialize` in `repository_lifecycle.rs` (already derived via `#[derive(..., Serialize, ...)]` — no change expected, just confirm).

- [ ] Flip ADR-015 status from `proposed` to `accepted` in `docs/adr/015-wasm-write-and-export.md` — update the Status line and add a note that srs-rust#258 ships the deferred "future issue" for new-repo WASM creation.

#### Acceptance Criteria

- [ ] `crates/srs-bindings` compiles without error: `cargo build -p srs-bindings`
- [ ] `cargo clippy -p srs-bindings -- -D warnings` clean
- [ ] The method is visible to `wasm-bindgen` and correctly calls the service with no logic of its own

#### Testing

```bash
cargo build -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
```

Note: full WASM binary rebuild (`wasm-pack build`) runs via the CI release pipeline post-merge
and is out of scope for this plan. The binding compiles and the service is tested in Phase 1.

#### Milestone gate

1. `cargo build -p srs-bindings` succeeds
2. `cargo clippy -p srs-bindings -- -D warnings` clean
3. Mark checkboxes `[x]` above.
4. Commit: `feat(bindings): add init_new_repository WASM binding (#258)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (`RepoInitNewPayload` golden schema in sync)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schema changes in this plan)
- [ ] `srs repo init-new --repo <seed.srsj> --namespace com.mudemocracy --title "Test"` → 0-error validate
- [ ] Manifest of re-stamped file has a **new** `repositoryId`, **preserved** `meta.upstreamPackage` (namespace = `com.mudemocracy.governance`), and a non-empty `installedAt`

## Coordination Rules

- Phases are sequential: start Phase 2 only when Phase 1 milestone gate passes.
- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.

## Assumptions

- `chrono` 0.4 is already in `srs-repository/Cargo.toml` (confirmed).
- `flush()` on `JsonStore` is a no-op for `<memory>` path (confirmed — WASM write path is safe).
- The governance seed provided to the CLI/WASM always carries `meta.upstreamPackage` (WS-1 guarantee).
- WASM binary artifact rebuild is a CI step, not part of this PR.
