# Plan: Schema Hygiene - Canonical Schemas, Pinned Rust Artifact, Drift Enforcement

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

The canonical JSON Schemas live in `srs/docs/schema/2.0/` and are published at `srs.semanticops.com/schema/2.0/`. The Rust tooling must validate against the same schemas, but Rust builds should not depend on a mutable sibling checkout or on a network fetch. Best practice here is a pinned schema artifact: keep the spec repo as the authority, generate or sync a schema snapshot into the Rust workspace, embed that snapshot in Rust, and make CI fail if the snapshot drifts from the canonical spec schemas.

This plan replaces manual memory with automated gates:

- `srs/docs/schema/2.0/` remains the source of truth.
- `srs-rust/crates/srs-schema/` becomes a generated/pinned consumer artifact, not an independent source.
- Rust validates instances, package files, and repository manifests through the embedded artifact.
- CI/pre-commit compares the Rust artifact against the spec repo and fails on drift.
- The docs publishing flow serves from the canonical spec directory, not from a hand-maintained copy.

## Best-Practice Review Findings

The original plan had the right instinct, but a few choices would create long-term fragility:

1. **Builds depending on `../../../srs/docs/schema/2.0/` are not reproducible.** A Rust crate should build from its checked-out source. Reading schemas from a sibling repo in `build.rs` makes the binary depend on whatever happens to be on disk.
2. **A generated copy is acceptable if it is treated as an artifact.** "Single source of truth" does not mean "only one byte-for-byte copy exists anywhere." It means there is one authoritative input and every generated copy has provenance plus automated drift detection.
3. **Schema validation must be a library service, not only a CLI feature.** `srs-cli` and future bindings should call the same repository/core validation APIs.
4. **Dispatch by `instanceIndex[].tier` alone is unsafe.** Validation should also inspect each file's declared `$schema` and report mismatches between manifest tier and declared schema.
5. **The current Rust models do not yet match the canonical schemas.** For example, `Record.typeNamespace` and `Record.typeName` are required by `record.json`, while the current Rust `Record` makes them optional. Phase 3 must align Rust models with schemas before writer enforcement is enabled.
6. **Coverage must include schema files and repository manifests, not just notes/records.** `manifest.json`, `package/package.json`, field definitions, type definitions, relation types, and future schema kinds need explicit registry entries.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-004](../docs/adr/004-schemas-embedded-at-compile-time.md) | Rust embeds a pinned schema artifact generated from canonical spec schemas | accepted |
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Library is the primary deliverable; CLI is one consumer | accepted |

## Scope

- Add `crates/srs-schema/` as the Rust schema artifact crate.
- Add a schema registry API with validators keyed by canonical `$id`.
- Add a sync/check script that copies `srs/docs/schema/2.0/*.json` into `crates/srs-schema/schemas/2.0/` and writes a provenance manifest with source paths and SHA-256 digests.
- Add CI/pre-commit drift checks so schema changes cannot be published without updating the Rust artifact.
- Align Rust model serialization with canonical schemas before enforcing writer validation.
- Validate schemas themselves against JSON Schema Draft 2020-12 during tests.
- Wire schema validation into repository writer/service functions.
- Add `srs repo validate --repo <path> --json` as a thin CLI wrapper over `srs-repository` validation.
- Remove `srs/srs/schemas/` offline copies after the new validation path exists, and update temporary Node.js scripts to read `docs/schema/2.0/` directly.

**Out of scope:**

- Runtime schema fetch from `srs.semanticops.com`.
- Retiring Node.js validation scripts entirely.
- Full semantic validation of relation graphs, cross-field rules, or rendering constraints.
- WASM/Python binding surface changes beyond keeping validation APIs callable later.

## Target Architecture

```text
srs/docs/schema/2.0/*.json
        |
        | scripts/sync-schemas-from-spec.sh
        v
srs-rust/crates/srs-schema/schemas/2.0/*.json
srs-rust/crates/srs-schema/schemas/2.0/SHA256SUMS
        |
        | include_str!
        v
srs_schema::SchemaRegistry
        |
        +--> srs_core model/schema contract tests
        +--> srs_repository writer validation
        +--> srs_repository repo validation service
        +--> srs_cli repo validate
```

## Phases

### Phase 1: Schema Artifact Crate + Drift Check

**Status:** `complete`

**Goal:** Rust has a reproducible, embedded schema snapshot with an automated check against the canonical spec repo.

**Agent:** Lead Integrator

**Write scope:** `crates/srs-schema/`, `scripts/`, workspace `Cargo.toml`, `.github/` if present

#### Files to create/modify

| File | Action |
|---|---|
| `Cargo.toml` | Add `crates/srs-schema` workspace member and dependency alias |
| `crates/srs-schema/Cargo.toml` | Create |
| `crates/srs-schema/src/lib.rs` | Create schema registry API |
| `crates/srs-schema/schemas/2.0/*.json` | Generated copy from `srs/docs/schema/2.0/*.json` |
| `crates/srs-schema/schemas/2.0/SHA256SUMS` | Generated digest/provenance file |
| `scripts/sync-schemas-from-spec.sh` | Create |
| `scripts/check-schema-drift.sh` | Create |

#### Tasks

- [ ] Create `crates/srs-schema`.
- [ ] Copy every canonical schema from `../srs/docs/schema/2.0/` into `crates/srs-schema/schemas/2.0/`.
- [ ] Generate `SHA256SUMS` containing filename and digest for every schema file.
- [ ] Implement `scripts/sync-schemas-from-spec.sh` with optional `SRS_SPEC_DIR`, defaulting to `../srs`.
- [ ] Implement `scripts/check-schema-drift.sh`; it must fail if generated schema files or `SHA256SUMS` differ from the canonical source.
- [ ] Ensure `cargo build` works from `srs-rust` without the sibling `srs` checkout.

#### Acceptance

```bash
scripts/check-schema-drift.sh
cargo test -p srs-schema
```

### Phase 2: Schema Registry and Validator Selection

**Status:** `complete`

**Goal:** Consumers validate by canonical `$id` through one registry. Error messages include schema id, instance path, and human-readable detail.

**Agent:** Core Model Worker

**Write scope:** `crates/srs-schema/src/`, `crates/srs-schema/Cargo.toml`

#### Required API shape

```rust
pub const NOTE_SCHEMA_ID: &str = "https://srs.semanticops.com/schema/2.0/note.json";
pub const RECORD_SCHEMA_ID: &str = "https://srs.semanticops.com/schema/2.0/record.json";

pub struct SchemaRegistry;

impl SchemaRegistry {
    pub fn default() -> &'static Self;
    pub fn schema_ids(&self) -> &'static [&'static str];
    pub fn validate_by_id(
        &self,
        schema_id: &str,
        value: &serde_json::Value,
    ) -> Result<(), SchemaValidationErrors>;
    pub fn validate_declared_schema(
        &self,
        value: &serde_json::Value,
    ) -> Result<&'static str, SchemaValidationErrors>;
}
```

#### Tasks

- [ ] Choose one Draft 2020-12 validator crate and pin it in `crates/srs-schema/Cargo.toml`.
- [ ] Validate all embedded schemas against the Draft 2020-12 meta-schema in tests.
- [ ] Register all current schema IDs, not only note/record/field/type:
  - `field.json`
  - `federation-events.json`
  - `federation-registry.json`
  - `manifest.json`
  - `note.json`
  - `package-bundle.json`
  - `package-manifest.json`
  - `record.json`
  - `relation-type.json`
  - `relations-collection.json`
  - `source-document-meta.json`
  - `typed-record.json`
  - `type.json`
- [ ] Add tests for valid and invalid fixtures for note, record, field, type, package manifest, and repository manifest.
- [ ] Ensure unknown/missing `$schema` reports a validation diagnostic instead of panicking.

#### Acceptance

```bash
cargo test -p srs-schema
cargo clippy -p srs-schema -- -D warnings
```

### Phase 3: Align Rust Models With Canonical Schemas

**Status:** `complete`

**Goal:** Rust serialization produces JSON that passes the canonical schemas by construction wherever Rust owns the type.

**Agent:** Core Model Worker

**Write scope:** `crates/srs-core/src/types/`, `crates/srs-core/src/validation/`

#### Known alignment checks

- `Record` must serialize required `typeNamespace` and `typeName` fields for `record.json`.
- `Record` must not serialize fields that `record.json` does not allow, such as `tags`, unless the schema is intentionally changed first.
- `Field` must serialize required `description`, `aiGuidance`, and `createdAt` fields for `field.json`, or those schema requirements must be revisited in the spec.
- `RecordType` must serialize required `description`, `createdAt`, and `FieldAssignment.required` fields for `type.json`, or those schema requirements must be revisited in the spec.
- Optional fields should use `skip_serializing_if` only when the schema permits absence.

#### Tasks

- [ ] Add model contract tests that serialize minimal Rust `Note`, `Record`, `Field`, and `RecordType` values and validate them through `srs_schema`.
- [ ] Fix Rust models or open explicit schema-change follow-ups for every mismatch.
- [ ] Remove tests that rely on schema-invalid placeholder IDs where schema validation is now expected.

#### Acceptance

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

### Phase 4: Repository Validation Service

**Status:** `complete`

**Goal:** `srs-repository` exposes one validation service that validates an entire file-backed repository and returns diagnostics. CLI and bindings can reuse it.

**Agent:** Repository Service Worker

**Write scope:** `crates/srs-repository/src/`

#### Files to create/modify

| File | Action |
|---|---|
| `crates/srs-repository/src/validation.rs` | Create repository validation service |
| `crates/srs-repository/src/lib.rs` | Export validation module |
| `crates/srs-repository/src/error.rs` | Add schema validation and validation-run error variants as needed |
| `crates/srs-repository/src/writer.rs` | Validate serialized values before write |
| `crates/srs-repository/src/package.rs` | Preserve raw JSON paths needed by validation service |

#### Tasks

- [ ] Define `ValidationDiagnostic { severity, path, schema_id, message }`.
- [ ] Define `ValidationSummary { checked, errors, warnings }`.
- [ ] Implement `validate_repository(repo_root: &Path) -> Result<RepositoryValidationReport, RepositoryError>`.
- [ ] Validate root `manifest.json` against `manifest.json`.
- [ ] Validate every `instanceIndex` file against its declared `$schema`.
- [ ] Report a diagnostic when `instanceIndex[].tier` conflicts with the declared `$schema`.
- [ ] Validate `package/package.json` against `package-manifest.json`.
- [ ] Validate package fields, types, relation types, and future registered package paths against their schema IDs.
- [ ] Treat schema violations as diagnostics; treat I/O and malformed JSON as invocation errors unless the report model explicitly supports fatal diagnostics.
- [ ] Validate writer outputs before writing to disk.

#### Acceptance

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

### Phase 5: CLI Wrapper

**Status:** `complete`

**Goal:** `srs repo validate --repo <path> --json` exposes repository validation without duplicating validation logic.

**Agent:** CLI Worker

**Write scope:** `crates/srs-cli/src/`, `crates/srs-cli/tests/`

#### Command surface

```bash
srs repo validate --repo <path> --json
```

The existing CLI style uses `--repo`; keep that style unless a broader CLI redesign changes it.

Output:

```json
{
  "ok": true,
  "command": "repo validate",
  "version": "...",
  "diagnostics": [
    {
      "severity": "error",
      "path": "records/notes/foo.json",
      "schemaId": "https://srs.semanticops.com/schema/2.0/note.json",
      "message": "required property instanceId is missing"
    }
  ],
  "summary": { "checked": 42, "errors": 1, "warnings": 0 }
}
```

#### Tasks

- [ ] Add `Validate { repo: Option<PathBuf>, json: bool }` to `RepoCommand`.
- [ ] Dispatch to `srs_repository::validation::validate_repository`.
- [ ] Preserve standard output envelope conventions from `crates/srs-cli/src/output.rs`.
- [ ] Add integration test for the live `srs/srs/` repository.
- [ ] Add integration test with an injected invalid note.
- [ ] Add integration test where manifest tier and `$schema` disagree.

#### Acceptance

```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
```

### Phase 6: Remove Redundant SRS Repo Schema Copy

**Status:** `complete`

**Goal:** `srs/srs/schemas/` is removed, and temporary Node.js scripts read the canonical schema directory directly.

**Agent:** Lead Integrator

**Write scope:** `../srs/`

#### Tasks

- [ ] Delete `srs/srs/schemas/`.
- [ ] Update `srs/scripts/validate-package.mjs` to read `docs/schema/2.0/`.
- [ ] Update `srs/scripts/validate-records.mjs` to read `docs/schema/2.0/`.
- [ ] Confirm docs publishing serves from `docs/schema/2.0/`.
- [ ] Run the Node validation suite.

#### Acceptance

```bash
# from ../srs
node scripts/validate-all.mjs
```

## Final Acceptance

- [x] `cargo test` passes.
- [x] `cargo clippy -- -D warnings` passes.
- [x] `scripts/check-schema-drift.sh` passes.
- [x] `srs repo validate --repo ../srs/srs --json` reports no schema errors.
- [x] A Rust-written note validates through `srs repo validate`.
- [x] A temp repo with a missing required field produces an error diagnostic.
- [x] A temp repo with a manifest tier / `$schema` mismatch produces an error diagnostic.
- [x] `../srs/srs/schemas/` does not exist.
- [x] `node scripts/validate-all.mjs` passes from `../srs`.

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behavior summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- Workers run the phase acceptance commands before marking a phase complete.

## Assumptions

- The authoritative spec repo is available as `../srs` during drift checks and schema sync, or `SRS_SPEC_DIR` is set.
- The Rust workspace must still build and test without `../srs`; only drift/sync commands require the spec checkout.
- Published docs are generated from, or directly serve, `srs/docs/schema/2.0/`.
- Runtime validation never fetches schemas from the network.
