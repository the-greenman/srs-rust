# Plan: Add wasm32-unknown-unknown build to CI for srs-bindings

> **Issue:** srs-rust#344

## Summary

CI currently validates `srs-bindings` only on the native target (`cargo test -p srs-bindings`). ADR-013 requires that the WASM binding surface compile to `wasm32-unknown-unknown`, but no CI job enforces this. A `#[wasm_bindgen]` method that uses a WASM-incompatible type (e.g. a type that does not implement `wasm_bindgen::describe::WasmDescribe`) can pass all native tests yet fail for the WASM target. This plan adds a dedicated CI job that builds `srs-bindings` for `wasm32-unknown-unknown` and fails the build if it does not compile.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | CI type-safety enforcement uses `cargo build --target wasm32-unknown-unknown -p srs-bindings` (not `wasm-pack build`) — recorded in the ADR-013 Neutral section (amended by this plan). | accepted |

**Approach choice — `cargo build --target wasm32-unknown-unknown` vs `wasm-pack build`:**
`cargo build --target wasm32-unknown-unknown -p srs-bindings` is used because: (1) `wasm-pack` is not in the standard toolchain, (2) `cargo build` is sufficient to catch `WasmDescribe`-incompatible types at compile time, (3) JS binding/package generation is a distribution concern, not a CI type-safety concern. This decision is recorded in ADR-013's Neutral section.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. No payload structs added or modified. No action required; `cargo test --test payload_contracts` is unaffected.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/` or any schema mirror. No action required.

---

## Scope

- Add a `wasm-build` job to `.github/workflows/ci.yml` as a peer job alongside `test`, `lint`, and `schema-drift` (no `needs:` dependency; parallel execution).
- Install the `wasm32-unknown-unknown` target via `dtolnay/rust-toolchain@stable` with `targets: wasm32-unknown-unknown`.
- Use `Swatinem/rust-cache@v2` for build caching, consistent with the existing `lint` job (single checkout, no `workspaces:` override).
- Amend ADR-013 Neutral section to record the `cargo build` vs `wasm-pack` CI choice.

**Out of scope:**
- `wasm-pack` integration (not needed to catch type-compatibility failures in CI).
- Generating or publishing the WASM package artifact.
- Adding WASM-specific tests (covered by existing native tests; type-safety is what this job proves).
- Any changes to `srs-bindings` source code.

---

## Phases

### Phase 1: Add `wasm-build` CI job

**Goal:** `.github/workflows/ci.yml` has a `wasm-build` job that builds `srs-bindings` for `wasm32-unknown-unknown` and fails CI if the build fails. ADR-013 is amended to record the approach choice.

**Agent:** Lead Integrator

#### Tasks

- [x] Edit `.github/workflows/ci.yml`: add a new `wasm-build` job as a peer of `test`, `lint`, and `schema-drift` (no `needs:` dependency). The job:
  ```yaml
  wasm-build:
    name: WASM Build (srs-bindings)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-unknown-unknown

      - uses: Swatinem/rust-cache@v2

      - name: Build srs-bindings for wasm32-unknown-unknown
        run: cargo build --target wasm32-unknown-unknown -p srs-bindings
  ```
- [x] Amend `docs/adr/013-wasm-binding-strategy.md` Neutral section to record the CI enforcement approach.

#### Acceptance Criteria

- [x] `.github/workflows/ci.yml` contains a `wasm-build` job.
- [x] The `wasm-build` job uses `dtolnay/rust-toolchain@stable` with `targets: wasm32-unknown-unknown`.
- [x] The `wasm-build` job runs `cargo build --target wasm32-unknown-unknown -p srs-bindings`.
- [x] The `wasm-build` job uses `Swatinem/rust-cache@v2`.
- [x] The `wasm-build` job is positioned after the `lint` job in `.github/workflows/ci.yml`.
- [x] YAML is syntactically valid.
- [x] No other CI jobs are modified.
- [x] `docs/adr/013-wasm-binding-strategy.md` Neutral section contains the CI enforcement rationale.

#### Testing

This phase adds no Rust source code — there are no `cargo test` targets for the CI YAML itself. Acceptance is verified by:
1. Local WASM pre-flight: `rustup target add wasm32-unknown-unknown && cargo build --target wasm32-unknown-unknown -p srs-bindings` — proves the existing codebase compiles for the target before enabling the enforcement gate.
2. YAML syntax validation: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"`.
3. Confirm `wasm-build` job is present and has the correct steps.
4. Run `cargo clippy -- -D warnings` and `cargo test --test payload_contracts` to confirm no regressions.

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Run WASM pre-flight, YAML syntax validation, clippy, and payload_contracts gate (all passed).
3. Commit:
   ```bash
   git commit -m "ci: add wasm32-unknown-unknown build job for srs-bindings (#344)"
   ```

Do not start the next phase until the milestone gate passes and the plan is updated.

---

## Final Acceptance

- [x] `cargo clippy -- -D warnings` passes.
- [x] `cargo test --test payload_contracts` passes (no payload structs changed).
- [x] `cargo build --target wasm32-unknown-unknown -p srs-bindings` passes (WASM pre-flight).
- [x] `.github/workflows/ci.yml` YAML is syntactically valid.
- [x] `wasm-build` CI job is present, correctly configured, and positioned after `lint`.
- [x] `docs/adr/013-wasm-binding-strategy.md` Neutral section documents the CI enforcement approach.
- [x] No changes to any file outside `.github/workflows/ci.yml`, `docs/adr/013-wasm-binding-strategy.md`, and `plans/344-wasm32-ci-build.md`.
- Note: `cargo test --all` has 2 pre-existing failures (`decision_log_get_shows_field_labels`, `tui_smoke_renders_first_frame` in `srs-gov`) that reproduce on unmodified `main` HEAD — not caused by this branch.

## Coordination Rules

- Lead Integrator owns the CI YAML change and ADR-013 amendment.
- Verification Agent reviews the YAML and runs the baseline `cargo test` + `cargo clippy` gate.
- At the end of Phase 1: verify all acceptance criteria, update the plan checkboxes, then commit.

## Assumptions

- The stable Rust toolchain supports `wasm32-unknown-unknown` as a cross-compilation target (Tier 2 supported target).
- `Swatinem/rust-cache@v2` works with cross-compilation targets (it does — it caches by target directory).
- The `srs-bindings` crate already compiles for `wasm32-unknown-unknown` on the current `main` branch.
