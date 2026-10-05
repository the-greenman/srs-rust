# Plan: Package upgrade (#1152)

## Summary

`srs package upgrade` upgrades an installed package boundary in place from a newer `.srspkg`. The design is fixed by the owner rulings of 2026-10-05 (decision record srs#890; RFC-014 R2/R3/R6, RFC-003): no RFC needed. It builds on #1267 (install identity is `(kind, uuid, version)`, PR #1268).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | this session |
| Repository Worker | this session (`package_install_service.rs`, tests) |
| CLI Worker | this session |
| Bindings Worker | this session |
| Verification | gates below |

## Architecture Decisions

No new architectural decisions: this plan implements the rulings in srs-rust#1152 and srs#890, and capability-layering (one core service; CLI and WASM adapters carry no logic).

## Contracts

- CLI: new `package upgrade` payload `PackageUpgradePayload` in `crates/srs-cli/src/payload.rs`; `cargo run --bin generate-schemas`; commit `schemas/payload/package-upgrade.json`; `cargo test --test payload_contracts`.
- Entity schemas: unchanged.

## Scope

- Extract the per-definition classification of `install_package_bundle` into one shared `analyse_definitions`; install keeps its exact behaviour.
- `upgrade_package_bundle(store, bytes, UpgradeOptions { dry_run, boundary_path })` -> `UpgradePackageResult`.
  - Boundary must already be installed with the bundle's `packageId` ("not installed; use install").
  - SemVer (`srs_core::types::package_dependency::SemVer`): lower refused, equal = content sync, higher = bump via `update_package_metadata`.
  - New UUIDs -> `added`; new version of a known UUID -> `newVersions`; installed alongside, never removed.
  - Same uuid+version, different content: installed == reference copy -> overwrite + refresh reference (`updated`); installed != reference -> `conflicts` (`local-edit`), local file kept; installed == bundle -> `unchanged`.
  - In-boundary definitions absent from the bundle -> `removedUpstream`, never deleted. Records never touched.
  - Import records rewritten for touched definitions (replace, not append; `conflictState: clean`, new `sourcePackageVersion`).
  - RFC-044 `check_bundle` unsatisfied results -> `dependencyWarnings`, never blocking.
  - `dry_run`: full result, no writes.
- Adapters: CLI `srs package upgrade --bundle <file> [--dry-run] [--boundary <path>]`; WASM `upgrade_package_bundle(bundle_json, options_json)`; MCP only if install has an MCP tool (it has none: no tool added).
- Tests: `crates/srs-repository/tests/package_upgrade.rs`, `crates/srs-bindings/tests/package_bundle.rs`. Dogfooding scenario S54.

**Out of scope:** manifest-level `packageRefs`/`upstreamPackage` (RFC-014 R2), install preview as a separate command, writing the srs-usage.md reference (srs repo).

**Judgement calls inside the rulings:** a same-version differing definition with no reference copy (kinds that get no import record) is a conflict with `conflictKind: "no-reference-copy"` (cannot prove it clean); a same-logical-key/different-UUID collision is a conflict `key-collision`.

## Phases

### Phase 1: Core service
- [x] `analyse_definitions` shared; helpers `import_record`, `write_ref_copy`, `validate_bundle_identity`
- [x] `upgrade_package_bundle` / `upgrade_package_source`
- [x] repository tests (essay shape, local edit, downgrade, same version, re-run, removed upstream, dry run, not installed, imports clean)

### Phase 2: Adapters
- [x] CLI command, payload, schema, contract test
- [x] WASM binding + test (epoch moves only on a real run)
- [x] dogfooding S54

## Final Acceptance

- [x] `cargo test --workspace` passes
- [x] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] `cargo fmt --check` passes
- [x] `cargo test --test payload_contracts` passes
