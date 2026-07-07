# Plan: Governance scaffold — fold migrate_rfc014 into WASM load + ship migrated seed in release artifact

> Issue: srs-rust#381

## Summary

Two related gaps let the WASM scaffold path ship a corrupt governance repository. First, `SrsRepository.load` in srs-bindings does a bare `JsonStore::from_srsj` with no RFC-014 migration, so any WASM client that loads the raw governance seed and calls `scaffold_new_repository` produces a manifest with no `upstreamPackage.contentHash` — a bundle that fails `validate_repository`. Second, `srs-bindings-web.tar.gz` does not include the governance seed file, forcing srs-web to vendor its own locally-migrated copy as a stopgap. This plan closes both gaps: fold `migrate_rfc014` into `SrsRepository.load` (idempotent, safe for all loads), and add a `generate-governance-seed` binary that CI uses to include the migrated seed in the release artifact.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Bindings Worker | — |
| CLI Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | `SrsRepository.load` is the WASM entry point for `.srsj` bundles; no business logic in `srs-bindings` | accepted |
| [ADR-015](../docs/adr/015-wasm-write-and-export.md) | `SrsRepository` wraps a `JsonStore`; write bindings call one service each | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | One service call per binding method; no logic in the binding layer | accepted |

No new ADRs are needed. Calling `migrate_rfc014` inside `SrsRepository.load` is consistent with ADR-013's principle that the binding layer should present a correctly-shaped store to callers — it is a load-path invariant, not a new architectural decision.

The issue explicitly rejected the alternative of exposing a separate `migrate` binding (keeps the foot-gun); that decision is recorded on the issue and does not require a new ADR.

---

## Contracts

### CLI output contract (ADR-011)

No CLI command output shapes change. No `payload.rs` structs are added or modified.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are added or modified.

---

## Scope

- Add `srsj_migration_service::load_from_srsj(srsj: &str) -> Result<JsonStore, RepositoryError>` to `crates/srs-repository` — combines `migrate_rfc014` + `JsonStore::from_srsj` in one call so callers always get a migrated store
- Update `SrsRepository::load` in srs-bindings to call `srsj_migration_service::load_from_srsj` (one service call, per ADR-013)
- Fix the `scaffold_new_repository` doc comment (`to_srsj()` → `export_srsj()`)
- Add a `generate-governance-seed` binary to `crates/srs-cli/src/bin/` accepting `<input-path> <output-path>` — runtime I/O, no `include_str!`
- Add fixture copy `crates/srs-bindings/tests/fixtures/governance-seed.srsj` (raw pre-migration seed)
- Add `crates/srs-bindings/tests/scaffold.rs` — service-layer test proving migrate+scaffold+validate succeeds from a raw seed
- Update `release.yml` to run `generate-governance-seed --release` and bundle output in `srs-bindings-web.tar.gz`

**Out of scope:**
- Any change to `create_governance_repository` service signature (the service continues to accept a pre-loaded store)
- Changes to srs-web (srs-web#141 will remove the vendored seed once the release artifact includes it — that is tracked on that issue)
- Any refactoring of the governance scaffold service itself

---

## Phases

### Phase 1: Fix the WASM load path

**Goal:** After this phase, `SrsRepository.load` calls a single `srs-repository` combinator that migrates and loads in one step, and a new service-layer test proves the full scaffold path succeeds from a raw seed.

**Agent:** Repository Service Worker + Bindings Worker

#### Tasks

- [ ] In `crates/srs-repository/src/srsj_migration_service.rs`, add a public combinator:
  ```rust
  pub fn load_from_srsj(srsj: &str) -> Result<crate::JsonStore, crate::error::RepositoryError> {
      let migrated = migrate_rfc014(srsj)?;
      crate::JsonStore::from_srsj(&migrated)
  }
  ```
- [ ] In `crates/srs-bindings/src/lib.rs`, update `SrsRepository::load` (lines 57–59) to call the new combinator:
  ```rust
  pub fn load(srsj: &str) -> Result<SrsRepository, JsValue> {
      let store = srs_repository::srsj_migration_service::load_from_srsj(srsj).map_err(js_err)?;
      Ok(SrsRepository { store })
  }
  ```
  No additional import needed — `srs_repository` is already in scope.
- [ ] Fix the doc comment on `scaffold_new_repository` (around line 616): replace `call \`to_srsj()\`` with `call \`export_srsj()\`` in the "After this returns" sentence.
- [ ] Copy `crates/srs-repository/tests/fixtures/governance-seed.srsj` to `crates/srs-bindings/tests/fixtures/governance-seed.srsj` (raw pre-migration seed; `mkdir -p` the directory first).
- [ ] Create `crates/srs-bindings/tests/scaffold.rs` with test `scaffold_from_raw_seed_produces_valid_repository`:
  ```rust
  use srs_repository::{
      governance_scaffold_service::{create_governance_repository, CreateGovernanceRepositoryInput},
      srsj_migration_service,
      validation,
  };

  #[test]
  fn scaffold_from_raw_seed_produces_valid_repository() {
      let raw = std::fs::read_to_string(concat!(
          env!("CARGO_MANIFEST_DIR"),
          "/tests/fixtures/governance-seed.srsj"
      ))
      .expect("fixture must exist");

      let store = srsj_migration_service::load_from_srsj(&raw)
          .expect("load_from_srsj must succeed on raw seed");

      create_governance_repository(
          &store,
          CreateGovernanceRepositoryInput {
              namespace: Some("com.test.381".to_string()),
              title: "Test Org".to_string(),
              purpose: None,
              repository_id: None,
          },
      )
      .expect("scaffold must succeed");

      let report = validation::validate_repository(&store)
          .expect("validate must not error");

      assert!(
          report.diagnostics.is_empty(),
          "expected no diagnostics, got: {:?}",
          report.diagnostics
      );
  }
  ```

#### Acceptance Criteria

- [ ] `load_from_srsj` is `pub` in `srs_repository::srsj_migration_service`
- [ ] `SrsRepository::load` calls `srsj_migration_service::load_from_srsj` (one call, no inline logic)
- [ ] The `scaffold_new_repository` doc comment no longer references `to_srsj()`
- [ ] `crates/srs-bindings/tests/fixtures/governance-seed.srsj` exists (raw seed copy)
- [ ] `scaffold_from_raw_seed_produces_valid_repository` test exists and passes
- [ ] Existing WASM binding tests pass (no regression)

#### Testing

```bash
cargo test -p srs-repository   # load_from_srsj + existing migration tests
cargo test -p srs-bindings     # scaffold test + existing binding tests
```

#### Milestone gate

1. All acceptance criteria checked.
2. `scaffold_from_raw_seed_produces_valid_repository` exists and passes.
3. No existing test regressed.

```bash
cargo test -p srs-repository
cargo test -p srs-bindings
cargo clippy -p srs-bindings -- -D warnings
cargo clippy -p srs-repository -- -D warnings
```

4. Commit with message: `fix(bindings): add load_from_srsj combinator; wire into SrsRepository::load (#381)`

---

### Phase 2: generate-governance-seed binary and release artifact

**Goal:** After this phase, the release CI generates the migrated governance seed and bundles it in `srs-bindings-web.tar.gz` under the filename `governance-seed.srsj`.

**Agent:** CLI Worker

#### Tasks

- [ ] Create `crates/srs-cli/src/bin/generate-governance-seed.rs`. The binary:
  - Accepts exactly two positional CLI arguments: `<input-seed-path>` and `<output-path>`. If wrong count, print usage to stderr and exit 1.
  - Reads the raw governance seed from `<input-seed-path>` at runtime (no `include_str!`).
  - Calls `srs_repository::srsj_migration_service::migrate_rfc014` on the read content.
  - Writes the migrated string to `<output-path>`.
  - Exits 0 on success; prints the error message to stderr and exits 1 on any failure.
  - No `[[bin]]` entry is needed in `crates/srs-cli/Cargo.toml` — Cargo auto-discovers `src/bin/*.rs` targets even when other explicit `[[bin]]` entries exist (confirmed by existing `generate-schemas.rs` in the same directory).
- [ ] In `.github/workflows/release.yml`, add a step **after** "Build WASM bindings" and **before** "Package WASM bindings":
  ```yaml
  - name: Generate migrated governance seed
    run: |
      cargo run --release --bin generate-governance-seed -- \
        crates/srs-gov/assets/governance-seed.srsj \
        dist/srs-bindings/governance-seed.srsj
  ```
  This writes `governance-seed.srsj` into `dist/srs-bindings/` so the existing `tar -czf "$archive" -C dist/srs-bindings .` step picks it up. `--release` avoids a redundant debug rebuild after the earlier release build steps.

#### Acceptance Criteria

- [ ] `cargo run --bin generate-governance-seed -- /tmp/test-seed.srsj` succeeds and writes a valid migrated `.srsj` to `/tmp/test-seed.srsj`
- [ ] The output file has `manifest.upstreamPackage.contentHash` (top-level, not under `meta`) — confirming RFC-014 migration was applied
- [ ] The release workflow step is present in `release.yml`, placed after `wasm-pack build` and before the package step
- [ ] The `tar` archive list in release.yml is unchanged (the seed is added by being present in `dist/srs-bindings/`, not by explicit archive inclusion)
- [ ] `cargo test -p srs-cli` passes (no regression from the new binary)

#### Testing

```bash
cargo run --bin generate-governance-seed -- /tmp/test-seed.srsj
# Then verify the output
python3 -c "import json,sys; d=json.load(open('/tmp/test-seed.srsj')); assert 'upstreamPackage' in d['manifest'], 'missing upstreamPackage'; assert 'contentHash' in d['manifest']['upstreamPackage'], 'missing contentHash'; print('OK')"
```

Also run:
```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
```

#### Milestone gate

1. All acceptance criteria checked.
2. `generate-governance-seed` binary runs and produces correct output.
3. release.yml diff reviewed.

```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
```

4. Commit with message: `feat(release): add generate-governance-seed binary; bundle seed in srs-bindings-web.tar.gz (#381)`

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `scaffold_from_raw_seed_produces_valid_repository` exists in srs-bindings tests and passes
- [ ] `cargo run --bin generate-governance-seed -- /tmp/test-seed.srsj` succeeds and the output contains `manifest.upstreamPackage.contentHash`
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `release.yml` includes a "Generate migrated governance seed" step

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `crates/srs-repository/tests/fixtures/governance-seed.srsj` and `crates/srs-gov/assets/governance-seed.srsj` are the same pre-migration raw seed (either identical files or one is a copy of the other).
- `migrate_rfc014` applied to an already-migrated store returns the same JSON content (idempotent) — confirmed by the `migrate_rfc014_is_idempotent_on_already_migrated_bundle` test.
- The srs-bindings wasm32 target compiles cleanly after the change; `migrate_rfc014` uses only `serde_json` and `sha2` which are already wasm-compatible.
- The WASM binding tests in srs-bindings are not compiled for wasm32 (they run as native tests); adding a test that reads a fixture file is consistent with the existing test pattern.
