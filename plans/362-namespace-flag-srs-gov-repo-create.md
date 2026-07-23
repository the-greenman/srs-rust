# Plan: Add --namespace flag to srs-gov repo-create

## Summary

`srs-gov repo-create` currently hardcodes `namespace: None`, causing the service to derive
a `com.example.<slug>` namespace from the title. The service (`CreateGovernanceRepositoryInput`)
already accepts an explicit `namespace: Option<String>`, and the WASM binding exposes it, but
there is no CLI path to override it. This plan adds `--namespace <STRING>` as an optional
argument so shell scripts, CI pipelines, and operators can specify a custom prefix.
Deferred from #331 (namespace-derivation refactor).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | CLI Worker |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | CLI handler maps flag → typed service input; no business logic in handler | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | No payload struct change; output shape unchanged | accepted |

No new ADRs needed: this implements an already-accepted capability (namespace derivation service
already handles `Some(ns)` / `None`). No public API shape change, no cross-crate boundary
change, no new extension model.

---

## Contracts

### CLI output contract (ADR-011)

No new/changed commands and no payload struct changes — `cmd_repo_create` returns `()` via
`render::repo_created`, not a payload DTO. No `cargo run --bin generate-schemas` needed.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files changed under `srs/docs/schema/2.0/`.

---

## Scope

- Add `--namespace <STRING>` optional arg to the `RepoCreate` subcommand in
  `crates/srs-gov/src/main.rs`.
- Thread `namespace` through `cmd_repo_create(output, title, namespace, purpose)`.
- Pass it as `namespace: namespace.map(str::to_string)` to `CreateGovernanceRepositoryInput`.
- Add an integration test in `crates/srs-gov/tests/flow.rs` verifying the explicit namespace
  is applied to the scaffolded repo's manifest.

**Out of scope:**
- No WASM binding changes (already accepts `namespace`).
- No `srs-cli` payload struct changes.
- No service logic changes — the service already handles `Some(ns)` vs `None`.

---

## Phases

### Phase 1: CLI flag + test

**Goal:** `srs-gov repo-create --namespace com.acme.myorg` works end-to-end and the manifest
`namespace` field in the output `.srsj` matches the supplied value.

**Agent:** CLI Worker

#### Tasks

- [x] Add `namespace: Option<String>` field with doc-comment to the `RepoCreate` variant
      in `Commands` enum (`crates/srs-gov/src/main.rs`).
- [x] Add `#[arg(long)]` attribute; doc string: "Namespace prefix for the repository
      (defaults to com.example.<slug> derived from title)".
- [x] Update the `Commands::RepoCreate` arm in `run()` (`crates/srs-gov/src/main.rs`) to
      destructure `namespace` and pass it to `cmd_repo_create`.
- [x] Update `cmd_repo_create` signature in `crates/srs-gov/src/main.rs`:
      `fn cmd_repo_create(output: &str, title: &str, namespace: Option<&str>, purpose: Option<&str>) -> Result<()>`
- [x] Pass `namespace: namespace.map(str::to_string)` in the `CreateGovernanceRepositoryInput`
      in `crates/srs-gov/src/main.rs` (replacing the hardcoded `namespace: None`).
- [x] Add test `repo_create_explicit_namespace_applied` in `crates/srs-gov/tests/flow.rs`:
      call `srs-gov repo-create --output <tmp> --title "Acme" --namespace "com.acme.myorg"`,
      assert `manifest.namespace == "com.acme.myorg"`.

#### Acceptance Criteria

- [x] `srs-gov repo-create --namespace com.acme.myorg` produces a `.srsj` with
      `manifest.namespace == "com.acme.myorg"`.
- [x] `srs-gov repo-create` without `--namespace` still derives namespace from title
      (existing `repo_create_produces_valid_srsj` test passes unchanged).
- [x] `srs repo validate` on the explicit-namespace output reports 0 errors.
- [x] `cargo clippy -- -D warnings` clean.

#### Testing

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests:
- `repo_create_explicit_namespace_applied` — proves explicit namespace is applied.
- `repo_create_produces_valid_srsj` (existing) — proves default derivation still works.

#### Milestone gate

1. Verify all acceptance criteria above.
2. Confirm both tests exist and pass.
3. Run lint and tests.
4. Update plan checkboxes.
5. Commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `srs-gov repo-create --namespace com.acme.myorg --output <tmp>` produces manifest with correct namespace
- [ ] `srs-gov repo-create --output <tmp>` still derives namespace correctly (no regression)

## Coordination Rules

- CLI Worker keeps to write scope: `crates/srs-gov/src/**`, `crates/srs-gov/tests/**`.
- At end of phase: verify acceptance criteria, confirm tests pass, update checkboxes, commit.
- Verification Agent runs final acceptance after Phase 1.

## Assumptions

- The service already handles namespace validation (rejects empty string) — no guard needed in the handler.
- The `render::repo_created` output does not display the namespace field, so no render changes needed.
