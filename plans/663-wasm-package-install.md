# Plan: `.srspkg` export, bundle install, and WASM package install (srs-rust#632 + #690 + #663)

> One PR, owner decision 2026-10-03. Closes #632, #690, #663. Serves story
> the-greenman/muDemocracy.org#242 (units U2 + U3 install half), epic muDemocracy.org#224.
> Consumers: srs-web#339 (pinned `.srspkg` with sha256 at build time), srs-web#340 (install on open).
> Revision 2: resolves the Stage 3 architecture and plan reviews (see "Review resolutions").

## Summary

A repository can install a package only from a package *directory* on disk, and only from the CLI.
srs-web needs to install editor packages in the browser from pinned, sha256-verified `.srspkg`
files, and those files need a producer. This plan adds, once in `srs-repository`: a deterministic
`.srspkg` writer for a whole package boundary (#632), a bytes-in bundle reader that runs the
RFC-043 pre-load transformer before validating (#690), and an install entry point over that reader;
then exposes all three through thin CLI handlers (`srs package export`, `srs package install
--bundle`) and thin WASM methods (`export_package_bundle`, `install_package_bundle`) with payload
parity (#663). The acceptance test is the round trip: export a boundary carrying all ten definition
kinds, install the bytes into a fresh repository, and every definition comes back identical with
zero validation errors.

## Spec gate (Stage 1.5)

No spec change in this PR. Governing spec: `docs/schema/2.0/package-bundle.json` (normative; the
srs-rust mirror is byte-identical to srs `origin/master` cbb44cb), RFC-003 Rev 5 Change C
[C1]-[C5] (RFC still Draft, shipped under OD4), RFC-043 [R17] (bundle pre-load transformer),
RFC-044 [R5]/[R8] (`packageDependencies` in a bundle), RFC-045 (readme, deferred to srs-rust#1164
and refused here). One deliberate deviation (OD5) is recorded in ADR-050, and the [C1] amendment is
filed in `srs` (deferred issue D1).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | /ship session |
| Repository Worker | Phases 1 and 2 |
| CLI Worker | Phase 3 |
| Bindings Worker | Phase 4 |
| Verification | Verification Agent, after Phases 2, 4 and before sign-off |

See [agents.md](agents.md). No new role needed.

## Architecture Decisions

Owner decisions (settled 2026-10-03, not to relitigate):

| # | Decision |
|---|---|
| OD1 | The artifact is the spec'd Package Bundle (`package-bundle.json`, `.srspkg`, one JSON document with inline definitions). No directory-tree-bytes input. |
| OD2 | Scope = #632 export + #690 bytes/bundle reader + #663 WASM install, one PR. The round trip `package export` -> `package install --bundle` into a fresh repo -> identical definitions is the acceptance test. |
| OD3 | Capability layering: every capability once in `srs-repository` (typed in/out); CLI handlers <= 15 lines; WASM bindings thin over the same services; payload parity. |
| OD4 | (was O1) RFC-003 is Draft: ship under `gate:owner-merge`; this PR is the implementation evidence for accepting Change C. |
| OD5 | (was O2) The embedded `com.semanticops.core` definitions are **omitted** from bundles (present in every repository, ADR-025). Recorded in ADR-050 as an explicit deviation from RFC-003 [C1] and the schema's `bundled` description; the amendment is filed in `srs` (D1). |

Plan decisions (owner may override at the Stage 4 checkpoint):

| # | Decision |
|---|---|
| PD1 | Bundle revision rule (#690 comment). (a) Refuse `dataModelRevision` greater than `CURRENT_DATA_MODEL_REVISION` (`crates/srs-repository/src/field_type_migration_service.rs:32`, = 9) with `bundle-revision-too-new`. (b) An absent stamp means 0 (schema text). (c) Always run the existing `migrate_package_bundle_value` (idempotent; one path, no revision branch), after fixing it so it never lowers a stamp above 8. (d) No numeric floor: the floor is the content shape (schema + loader-strict definition checks); a failure on a bundle older than current names its revision and says to re-export it. (e) Export stamps `CURRENT_DATA_MODEL_REVISION`. |
| PD2 | Closure for `mode: "bundled"`: id-scan fixpoint over the repository's other package boundaries; core omitted (OD5); `dependencyRefs: []`. |
| PD3 | Export is whole-boundary only, `mode: "bundled"` only. `packageId`/`packageVersion` = boundary `id`/`version` ([C3]). Subset export and `standalone` mode are not built (no consumer). |
| PD4 | A bundle carrying `readme` is refused (`bundle-readme-unsupported`, srs-rust#1164) rather than silently dropped; export never emits one. |
| PD5 | The WASM install takes the bundle as JSON text (`&str`); the core reader takes `&[u8]` (the CLI reads bytes). srs-web verifies the sha256 over the file bytes before passing the text. |
| PD6 | No MCP tool in this PR (srs-rust#1153 exists; not needed by #242's browser path). |
| PD7 | Divergence (#663 point 3): `list_package_imports_json` already returns `conflictState` (`clean` / `local-ahead`) per upstream-tracked definition, which is what an "edited since install" warning needs. Upstream-ahead / diverged against a candidate bundle is not built here (belongs with upgrade, srs-rust#1152). |
| PD8 | sha256 wire format is the existing `attachment_service::sha256_hex` output, **`sha256:<64 lowercase hex>`** (one convention, not a second bare-hex one). Pinned in `PackageExportSummary.sha256`, the CLI payload doc comment, ADR-050 and the tests. A stable hash across rebuilds requires a fixed `--published-at` (`publishedAt` is part of the bytes); srs-web's build must pass it (comment on srs-web#339, D6). |

| ADR | Decision | Status |
|---|---|---|
| [ADR-050](../docs/adr/050-package-bundle-codec.md) | `.srspkg` is a boundary codec: one reader (fixed pipeline), one deterministic writer, id-scan closure, core omitted (deviation recorded), readme refused | **ships in this PR**: drafted with this plan (status proposed), committed in Phase 1, flipped to accepted with its charter boxes ticked in the final commit |
| [ADR-004](../docs/adr/004-schemas-embedded-at-compile-time.md) | Bundles validate offline against the embedded `package-bundle.json` (`SchemaRegistry::global().validate_by_id`) | applies |
| [ADR-009](../docs/adr/009-package-boundary-model.md) | Boundaries addressed by `PackageSelector`; export resolves the target via `list_package_boundaries` | applies |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Typed input/output services; handler = parse, one call, output | applies |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | New `PackageExportPayload`; `PackageInstallPayload` gains `notes`; golden schemas regenerated | applies |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | Bindings: deserialize, one service call, serialize; wasm32 build is the CI gate | applies |
| [ADR-015](../docs/adr/015-wasm-write-and-export.md) | WASM write + export precedent (`export_srsj`/`export_archive` return the artifact to JS); `export_package_bundle` follows it | applies |
| [ADR-017](../docs/adr/017-deterministic-srsj-serialization.md) + [ADR-043](../docs/adr/043-rfc039-carrier-representation.md) | `preserve_order` is ON; determinism comes from the explicit canonicalize step, reused from `srsj.rs` | applies |
| [ADR-025](../docs/adr/025-implicit-core-package-merge.md) | Core package present in every repo; basis for OD5 | applies |
| [ADR-030](../docs/adr/030-import-record-storage-model.md) | Import records + reference copies written by the existing install core, unchanged | applies |
| [ADR-032](../docs/adr/032-migration-registry-fn-pointer-pattern.md) | Bundle forms stay out of the registry; pre-load transformer instead | applies |
| [ADR-033](../docs/adr/033-srs-archive-format.md) / [ADR-039](../docs/adr/039-srs-archive-pure-tree-zip.md) | `.srs` is a repository archive (ZIP of a tree); a `.srspkg` is not a repository and is not a ZIP. Export borrows ADR-039's rule: a listed definition file that does not exist is a hard error naming the path, never a silent skip | applies (distinct artifact) |
| [ADR-035](../docs/adr/035-flat-export-bundle-format.md) | The flat decision-export ZIP (`InvalidExportBundle`) is a different artifact; the new `InvalidPackageBundle` error is deliberately separate | applies (distinct artifact) |
| [ADR-037](../docs/adr/037-mcp-adapter-surface.md) | MCP adapter not extended here (PD6, srs-rust#1153) | n/a this PR |
| [ADR-038](../docs/adr/038-vfs-tree-primary-model.md) / [ADR-049](../docs/adr/049-write-recording-is-a-store-concern.md) | WASM install writes through the MemVfs `FileStore`, so `write_epoch` advances with no extra code; package files are not in the ADR-049 change summary (by design) | applies |
| [ADR-041](../docs/adr/041-storage-backend-guardrails.md) G1 | The CLI file read/write is process-boundary; the core reader is store-free | applies |
| [ADR-042](../docs/adr/042-logical-id-instance-persistence.md) | Boundary definitions are read through the transitional `load_instance_json` shim in ONE shared loader (`load_boundary_definitions`), the single migration point for #726 | applies |
| [ADR-048](../docs/adr/048-implementation-decision-rules.md) | Charter below | applies |

### Implementation charter (ADR-048)

- **Spec-first:** schema + RFC-003 Change C (Draft, OD4) + RFC-043 [R17] + RFC-044 [R5]/[R8]; deviation OD5 recorded, amendment filed (D1).
- **Layer test:** core `srs-repository` (`package_bundle.rs`, `package_install_service.rs`, `package_service.rs`); CLI and WASM adapters only; srs-web presents (srs-web#339/#340).
- **One way per goal:** one install core (`install_package_bundle`) behind two codecs (dir, bundle); one boundary-definition loader (`load_boundary_definitions`); one canonicalize (`srsj.rs`); one kind-key table (`definition_kind_key`); one definition filename helper (`package_service::definition_rel_path`, also adopted by `create_field`/`create_type`/`create_relation_type`); one sha256 helper and format (`sha256:<hex>`); no suffix inference (the `--bundle` flag is the one way).
- **Parity and mirror obligations:** golden payload schemas `package-export.json` (new) and `package-install.json` (changed). No entity schema change. srs-vscode has no package-install types (checked), so no mirror issue. srs-web pin bump after the release (srs-web#339/#340).
- **Decision mode / door:** complicated; implements a Draft RFC's conformance rules and adds a CLI contract, so `gate:owner-merge` (OD4).

### Positioning (alignment register, `srs/docs/research/alignment-opportunities.md`)

- **#18 Frictionless Data Package** (`$schema` self-versioning): the exported bundle carries `"$schema": "https://srs.semanticops.com/schema/2.0/package-bundle.json"` (the schema permits it).
- **#14 Nanopublications** (Trusty-URI-style content hashing): byte-deterministic output plus the reported sha256 makes a bundle content-addressable, which is what srs-web#339's lock file pins.
- **#6 atproto lexicon publishing**: registry/publishing is out of scope; nothing here forecloses publishing bundles as records later.
- **#1 MCP server**: a `package_install` MCP tool is the natural next adapter over the same reader (srs-rust#1153, PD6).

---

## Contracts

### CLI output contract (ADR-011)

- **New command** `srs package export` -> new `PackageExportPayload` in `crates/srs-cli/src/payload.rs`, `write_schema!("package-export", PackageExportPayload)` in `src/bin/generate-schemas.rs`, test `package_export` in `tests/payload_contracts.rs`, golden `schemas/payload/package-export.json`.
- **Changed payload** `PackageInstallPayload` gains `notes: Vec<String>` (always serialized). Regenerate `schemas/payload/package-install.json`.
- Run `cargo run --bin generate-schemas`, commit the schema diffs. `cargo test -p srs-cli --test payload_contracts` must pass.

### Entity schema sync (check-schema-sync.sh)

No file under `srs/docs/schema/2.0/` changes. No action.

---

## Scope

- `package_bundle.rs` (new, srs-repository): `read_package_bundle`, `export_package_bundle`, types.
- `package_install_service.rs`: `install_package_bundle_bytes`; `notes` on the result; `InstallBundleOptions` deserializable; shared `load_boundary_definitions`.
- `package_service.rs`: shared `definition_rel_path`.
- Existing `migrate_package_bundle_value` stamp fix (PD1c).
- New error variant `RepositoryError::InvalidPackageBundle { code, message }`.
- Fixture `install-package` extended with one theme and one vocabulary (all ten kinds).
- CLI: `srs package export`; `srs package install --bundle <path>`; handler refactor to <= 15 lines.
- WASM: `SrsRepository::export_package_bundle`, `SrsRepository::install_package_bundle`.
- Docs: ADR-050; dogfooding S49 + coverage matrix row.

**Out of scope:** see "Deferred items" (each is filed or already tracked).

---

## Types and signatures (names are final; Lead Integrator owns any polish)

All in `crates/srs-repository/src/package_bundle.rs` unless noted. serde `rename_all = "camelCase"`.

```rust
/// Input for export_package_bundle. Deserialize (WASM input_json) + Clone + Debug.
pub struct ExportPackageInput {
    /// Package boundary selector; None = primary package (same convention as `package update --selector`).
    #[serde(default)] pub selector: Option<String>,
    /// RFC 3339; None = now (UTC). Validated with chrono; invalid -> `bundle-published-at-invalid`.
    /// Part of the bytes: a reproducible sha256 needs a fixed value (PD8).
    #[serde(default)] pub published_at: Option<String>,
    /// Optional `publisher` property; omitted from the bundle when None.
    #[serde(default)] pub publisher: Option<String>,
}

/// The reader's result. Notes live here, not on the shared PackageSourceBundle (whose struct
/// literals in package_service.rs:2805 and tests/package_dependency_check.rs:334 stay unchanged).
pub struct ReadPackageBundle {
    pub bundle: PackageSourceBundle,
    /// Non-fatal pre-load transformer notes (RFC-043 `migration-memberorder-dropped`, ...).
    pub notes: Vec<String>,
}

/// Serialize. Returned by the export service; WASM returns it whole.
pub struct PackageBundleExport {
    /// The exact .srspkg text (pretty JSON + trailing '\n'); the sha256 is over these bytes.
    pub text: String,
    pub summary: PackageExportSummary,
}

pub struct PackageExportSummary {
    pub package_id: String,
    pub package_namespace: String,
    pub package_name: String,
    pub package_version: String,
    pub data_model_revision: u64,
    pub published_at: String,
    /// `sha256:<64 lowercase hex>` over `text`'s UTF-8 bytes (attachment_service format, PD8).
    pub sha256: String,
    pub byte_length: usize,
    pub definition_count: usize,
    /// Ids inlined from other boundaries by the closure (sorted). Empty for a self-contained boundary.
    pub inlined: Vec<String>,
    /// Per-kind counts, INSTALL_ORDER, kinds with count > 0 only; labels = install's kind_label.
    pub kinds: Vec<PackageExportKindCount>,
}
pub struct PackageExportKindCount { pub kind: String, pub count: usize }

pub fn read_package_bundle(bytes: &[u8]) -> Result<ReadPackageBundle, RepositoryError>;
pub fn export_package_bundle(store: &dyn RepositoryStore, input: ExportPackageInput)
    -> Result<PackageBundleExport, RepositoryError>;
```

In `package_install_service.rs`:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)] #[serde(rename_all = "camelCase", default)]
pub struct InstallBundleOptions { pub boundary_path: Option<String>, pub strict: bool }  // derives added
pub struct InstallPackageResult { /* existing fields */ #[serde(default)] pub notes: Vec<String> }
// install_package_bundle sets `notes: vec![]`; install_package_bundle_bytes overwrites it with the reader's notes.
pub fn install_package_bundle_bytes(store: &dyn RepositoryStore, bytes: &[u8], options: InstallBundleOptions)
    -> Result<InstallPackageResult, RepositoryError>;
/// The ONE reader of a boundary's definitions (ADR-042 shim migration point), used by
/// collect_existing and export. `{prefix}/package.json` not found -> Ok(vec![]) (preserves
/// collect_existing's "a boundary without an index contributes nothing"); a listed definition
/// file that fails to load -> Err naming the path (ADR-039 rule). prefix = selector or "package".
pub(crate) fn load_boundary_definitions(store: &dyn RepositoryStore, boundary: &PackageBoundary)
    -> Result<Vec<PackageSourceDefinition>, RepositoryError>;
// visibility only: INSTALL_ORDER, kind_label, definition_name, validate_source_definition -> pub(crate)
```

In `package_service.rs`:

```rust
/// One filename scheme for a definition inside a boundary: `{dir}/{slugify(slug_source)}-{id[..min(8)]}.json`,
/// dir per kind: fields, types, views, compositions, relation-types, blueprints, protocols,
/// vocabularies, lifecycles, themes. Replaces the inline format! at create_field (:546),
/// create_type (:697) and create_relation_type (:848); behaviour-identical for UUID ids.
pub(crate) fn definition_rel_path(kind: DefinitionKind, slug_source: &str, id: &str) -> String;
```

In `error.rs`:

```rust
/// A `.srspkg` was refused. Coded like `ActorProvenance { code, message }` (the existing coded
/// refusal precedent) because clients branch on the reason (srs-web shows "re-export" vs "upgrade srs").
#[error("{code}: {message}")]
InvalidPackageBundle { code: &'static str, message: String },
```

Codes (exhaustive): `bundle-not-json`, `bundle-readme-unsupported`, `bundle-revision-too-new`,
`bundle-schema-invalid`, `bundle-definition-invalid`, `bundle-published-at-invalid`.
RFC-043 transformer refusals propagate unchanged (`InvalidSnapshotData`, code in message).
Unknown selector on export: existing `PackageNotFound { selector }`.

Visibility-only changes (no behaviour change): `srsj::canonicalize` -> `pub(crate)`;
`attachment_service::sha256_hex` -> `pub(crate)`. Register `pub mod package_bundle;` in `lib.rs`.

### CLI flags (`crates/srs-cli/src/commands/mod.rs`, `PackageCommand`)

```text
srs package export [--selector <boundary>] --output <path> [--published-at <rfc3339>] [--publisher <text>]
srs package install [<source_dir>] [--bundle <path>] [--boundary <path>] [--strict]
```

`Install.source_dir` becomes `Option<String>` with `#[arg(required_unless_present = "bundle", conflicts_with = "bundle")]`;
new `#[arg(long)] bundle: Option<PathBuf>`. `Export { selector: Option<String> (long), output: PathBuf (long, required), published_at: Option<String> (long), publisher: Option<String> (long) }`.

### Payloads (`crates/srs-cli/src/payload.rs`)

```rust
pub struct PackageExportPayload {
    pub output_path: String, pub package_id: String, pub package_namespace: String,
    pub package_name: String, pub package_version: String, pub data_model_revision: u64,
    pub published_at: String,
    /// `sha256:<64 lowercase hex>` of the written file's bytes.
    pub sha256: String,
    pub byte_length: usize, pub definition_count: usize,
    pub inlined: Vec<String>, pub kinds: Vec<PackageExportKindEntry>,
}
pub struct PackageExportKindEntry { pub kind: String, pub count: usize }
impl PackageExportPayload { pub fn new(output_path: String, s: PackageExportSummary) -> Self }
// PackageInstallPayload: add `pub notes: Vec<String>`; add impl From<InstallPackageResult> for PackageInstallPayload
```

### Handlers (`crates/srs-cli/src/commands/package.rs`, each <= 15 lines after rustfmt)

Process-boundary file I/O in a handler has precedent: `commands/archive.rs` (`File::create`) and
`commands/attachment.rs:81` (`std::fs::read`).

```rust
fn cmd_package_export(ctx: CliContext, selector: Option<String>, out_path: PathBuf,
                      published_at: Option<String>, publisher: Option<String>) -> Result<String> {
    let input = ExportPackageInput { selector, published_at, publisher };
    let export = with_store(&ctx, |s| Ok(export_package_bundle(s, input.clone())?))?;
    std::fs::write(&out_path, &export.text)
        .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", out_path.display()))?;
    output::serialize("package export",
        PackageExportPayload::new(out_path.to_string_lossy().into_owned(), export.summary))
}

fn cmd_package_install(ctx: CliContext, source_dir: Option<String>, bundle: Option<PathBuf>,
                       boundary: Option<String>, strict: bool) -> Result<String> {
    let options = InstallBundleOptions { boundary_path: boundary, strict };
    let result = match (bundle, source_dir) {
        (Some(path), _) => {
            let bytes = read_bundle_file(&path)?;
            with_store(&ctx, |s| Ok(install_package_bundle_bytes(s, &bytes, options.clone())?))?
        }
        (None, Some(source_dir)) => {
            let input = InstallPackageInput { source_dir, boundary_path: options.boundary_path, strict };
            with_store(&ctx, |s| Ok(install_package(s, input.clone())?))?
        }
        (None, None) => anyhow::bail!("give <source_dir> or --bundle"), // clap already enforces this
    };
    output::serialize("package install", PackageInstallPayload::from(result))
}

/// File I/O only: read the .srspkg bytes (no parsing, no logic).
fn read_bundle_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))
}
```

### WASM (`crates/srs-bindings/src/lib.rs`, on `impl SrsRepository`)

```rust
/// Export a package boundary as a deterministic .srspkg (same service as `srs package export`).
/// input_json: {"selector"?: string|null, "publishedAt"?: string, "publisher"?: string}.
/// Pass a fixed publishedAt for a reproducible sha256. Returns {text, summary}
/// (PackageBundleExport; summary.sha256 is "sha256:<hex>"). Read-only: write_epoch does not move.
pub fn export_package_bundle(&self, input_json: &str) -> Result<JsValue, JsValue>;

/// Install a .srspkg (same service as `srs package install --bundle`). bundle_json is the file's
/// text (verify its sha256 over the file bytes before calling); options_json:
/// {"boundaryPath"?: string, "strict"?: bool} ("{}" for defaults). Returns InstallPackageResult
/// (same fields as the CLI payload, incl. notes). Advances write_epoch; the session is dirty.
/// Call check_package_requirements(bundle_json) first for RFC-044 requirement outcomes
/// (BundleRequirements reads packageId/packageDependencies from the same text).
pub fn install_package_bundle(&self, bundle_json: &str, options_json: &str) -> Result<JsValue, JsValue>;
```

Each body: `serde_json::from_str(..).map_err(js_err)?` -> one service call -> `to_js(&result)`.

---

## Phases

### Phase 1: Bundle reader and bytes install (#690)

**Goal:** a `.srspkg` byte slice installs into any store through the existing install core, with RFC-043 pre-load migration and every refusal named.

**Agent:** Repository Worker. **Write scope:** `crates/srs-repository/src/{package_bundle.rs (new), package_install_service.rs, package_service.rs (definition_rel_path + its three call sites only), rfc043_container_entries_migration_service.rs, error.rs, lib.rs, srsj.rs / attachment_service.rs (visibility only)}`, `crates/srs-repository/tests/package_bundle_reader.rs` (new), `docs/adr/050-package-bundle-codec.md` (commit the draft as-is).

#### Tasks

- [x] Commit the existing ADR-050 draft (`docs/adr/050-package-bundle-codec.md`, status proposed) with this phase.
- [x] Add `InvalidPackageBundle` to `error.rs`. Grep `crates/srs-cli`, `crates/srs-mcp`, `crates/srs-mcp-core` for exhaustive `match` on `RepositoryError` (today they use `_ =>` arms, e.g. `srs-mcp/src/server.rs:49`, `srs-mcp-core/src/lib.rs:416`); add an arm only where a match is exhaustive.
- [x] Modify the **existing** `rfc043_container_entries_migration_service::migrate_package_bundle_value` (line ~810): stamp `RFC043_REVISION` only when the existing stamp is lower; a stamp of 9 stays 9.
- [x] Add `notes` to `InstallPackageResult` (`install_package_bundle` sets `vec![]`); add the `InstallBundleOptions` derives. `PackageSourceBundle` is unchanged.
- [x] Add `package_service::definition_rel_path` and switch `create_field`, `create_type`, `create_relation_type` to it (behaviour-identical; existing tests prove it).
- [x] Implement `read_package_bundle`, steps in this order:
  1. `serde_json::from_slice::<Value>` else `bundle-not-json`; a non-object -> `bundle-not-json`.
  2. A `readme` key -> `bundle-readme-unsupported` (message cites srs-rust#1164).
  3. `rev = dataModelRevision.as_u64().unwrap_or(0)`; `rev > CURRENT_DATA_MODEL_REVISION` -> `bundle-revision-too-new` (message names both numbers).
  4. Always call `migrate_package_bundle_value(&mut v)?` (idempotent); its `diagnostics` become `notes`.
  5. `SchemaRegistry::global().validate_by_id(PACKAGE_BUNDLE_SCHEMA_ID, &v)` else `bundle-schema-invalid`; when `rev < CURRENT_DATA_MODEL_REVISION`, append "bundle declares dataModelRevision {rev}; re-export it with a current srs".
  6. For each kind in `INSTALL_ORDER`, the array at `definition_kind_key(kind)`; per item: `validate_source_definition(kind, Path::new(&format!("<bundle>/{key}/{index}")), item)` mapped to `bundle-definition-invalid` (keep the inner message, add the same revision hint); missing `id` -> `bundle-definition-invalid`; `definition_name(kind, item)` is `None` or blank -> `bundle-definition-invalid` (never `slugify("")`, which collides); `rel_path = definition_rel_path(kind, name, id)`.
  7. Return `ReadPackageBundle { bundle: PackageSourceBundle { id: packageId, namespace: packageNamespace, name: packageName, version: packageVersion, package_dependencies: non-empty packageDependencies array else None, definitions }, notes }`.
- [x] Implement `install_package_bundle_bytes` = `read_package_bundle` -> `install_package_bundle` -> `result.notes = read.notes`.

#### Acceptance Criteria

- [x] Each of the six codes `bundle-not-json`, `bundle-readme-unsupported`, `bundle-revision-too-new`, `bundle-schema-invalid`, `bundle-definition-invalid`, `bundle-published-at-invalid` is produced by exactly one named test (the last one in Phase 2).
- [x] A revision-7 bundle with a Composition `memberOrder` installs and its result `notes` contains `migration-memberorder-dropped`.
- [x] Existing install tests pass unchanged except for the new `notes` field.

#### Testing

`crates/srs-repository/tests/package_bundle_reader.rs` (bundles built inline with `serde_json::json!` modeled on the `install-package` fixture; no spec content):

- `read_bundle_rejects_non_json` — `bundle-not-json`.
- `read_bundle_refuses_readme_until_1164` — `bundle-readme-unsupported`.
- `read_bundle_refuses_newer_revision` — revision 10 -> `bundle-revision-too-new`.
- `read_bundle_rejects_schema_invalid` — missing `packageId` -> `bundle-schema-invalid`.
- `read_bundle_schema_error_on_old_bundle_names_revision` — revision 5 with `documentViews` -> message contains "re-export".
- `read_bundle_rejects_invalid_definition` — field with key `descriptionn` -> `bundle-definition-invalid`.
- `read_bundle_rejects_definition_without_id` — a view without `id` -> `bundle-definition-invalid`.
- `read_bundle_rejects_definition_without_name` — a view without `name` -> `bundle-definition-invalid`.
- `read_bundle_migrates_pre_rev8_member_order` — revision 7, memberOrder dropped, note present, install succeeds.
- `read_bundle_current_revision_passes_through_unchanged` — revision 9 bundle: no notes, definitions value-equal to the input.
- `read_bundle_member_order_conflict_refuses` — memberOrder + `containerScope: "subtree"` -> refused, store unchanged.
- `read_bundle_derives_rel_paths_by_kind` — a relation type lands at `relation-types/<key>-<id8>.json`.
- `install_options_json_maps_to_install_bundle_options` — `{}` -> defaults; `{"boundaryPath":"packages/x","strict":true}` -> set (the WASM `options_json` contract, tested here because srs-bindings has no wasm-bindgen-test harness).
- `install_bundle_bytes_memory_and_file_store_agree` — same bytes into `MemoryStore` and a disk `FileStore`: equal `InstallPackageResult` except `installedAt` (a wall-clock stamp taken at install time, so it differs by construction) (cross-store rule).
- `install_bundle_bytes_reinstall_is_idempotent` — second run: `installed == 0`, `skipped_identical == n`.

Unit test in `rfc043_container_entries_migration_service.rs`:

- `migrate_package_bundle_value_never_lowers_a_newer_stamp` — revision 9 stays 9.

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository --all-targets -- -D warnings
```

Check every box above, update this file, commit (`feat(package): .srspkg reader + bytes install (#690)`).

---

### Phase 2: Deterministic export and the round trip (#632)

**Goal:** `export_package_bundle` emits a schema-valid, byte-deterministic bundle, and the ten-kind round trip is green.

**Agent:** Repository Worker. **Write scope:** `crates/srs-repository/src/{package_bundle.rs, package_install_service.rs (load_boundary_definitions + collect_existing only)}`, `crates/srs-repository/tests/package_bundle_roundtrip.rs` (new), `crates/srs-repository/tests/fixtures/install-package/**`, `crates/srs-repository/tests/package_install.rs` (count updates only).

#### Tasks

- [x] Implement `load_boundary_definitions` and switch `collect_existing` to it (per boundary: `load_boundary_definitions(store, &boundary)?`, then index ids/keys as today). Behaviour change, deliberate: a boundary whose `package.json` lists a file that cannot be loaded now fails install instead of being skipped per file; such a repository already fails `load_package()`.
- [x] Fixture: add `themes/plain-9a1b0ca0.json` (id `9a1b0ca0-000a-4aaa-8bbb-00000000a001`) and `vocabularies/moods-9a1b0cb0.json` (id `9a1b0cb0-000b-4aaa-8bbb-00000000b001`), minimal and valid against `theme.json`/`vocabulary.json` and the srs-core `Theme`/`Vocabulary` structs; list them under `themes`/`vocabularies` in `tests/fixtures/install-package/package.json`. In `crates/srs-repository/tests/package_install.rs` change `FIXTURE_DEFINITION_COUNT` from 8 to 10 and update any per-kind and `skippedDefinitions` assertions (themes and vocabularies have no `DefinitionType`, so they are added to `skippedDefinitions`).
- [x] Implement `export_package_bundle`:
  1. Validate `published_at` (chrono RFC 3339) else `bundle-published-at-invalid`; default `Utc::now().to_rfc3339()`.
  2. `store.list_package_boundaries()`; target = selector match else `PackageNotFound`.
  3. For every boundary, `load_boundary_definitions`; index `id -> (kind, value)`, first boundary wins.
  4. Closure: start with the target's definitions; repeatedly scan every string leaf of every selected value; a leaf equal to an indexed id not yet selected is added (and listed in `inlined`); stop at fixpoint. Core-package ids are never in the index (OD5). Add `// ponytail: id-scan closure; replace with a typed reference walker if a non-reference string ever carries a definition UUID.`
  5. Build the bundle object: `$schema` (`PACKAGE_BUNDLE_SCHEMA_ID`), `schemaVersion` (manifest `srs_version`), `packageId`/`packageNamespace`/`packageName`/`packageVersion` (boundary), `dataModelRevision` (`CURRENT_DATA_MODEL_REVISION`), `publishedAt`, `publisher` (if Some), `mode: "bundled"`, `fields` and `types` always (possibly `[]`), the other eight kind arrays only when non-empty, each sorted by `id`, key = `definition_kind_key(kind)`; `dependencyRefs: []`; `packageDependencies`: the boundary's raw list or `[]`.
  6. `srsj::canonicalize(value, false)`; validate against `PACKAGE_BUNDLE_SCHEMA_ID` else `bundle-schema-invalid` (when the failing path is under `/packageDependencies`, append "repair with `srs package dependency add --repair-legacy`").
  7. `text = to_string_pretty + "\n"`; `sha256 = sha256_hex(text.as_bytes())` (`sha256:<hex>`); fill the summary.

#### Acceptance Criteria

- [x] [C1] closure (minus core, OD5), [C3] identity, [C4] verbatim definitions, [C5] determinism each proven by a named test below.
- [x] The round trip covers all ten definition kinds (the schema carries themes/blueprints/protocols since RFC-044 [R6]; srs#390's claim is stale).
- [x] No change to `install_package` (directory) behaviour beyond the deliberate `collect_existing` refusal above.

#### Testing

Unit tests in `package_bundle.rs` (FileStore on a tempdir with the install fixture installed; MemoryStore where it serves `package/package.json`, else FileStore):

- `export_whole_package_preserves_package_identity` — [C3].
- `export_preserves_definition_identity_verbatim` — every exported definition is value-equal to the stored one (key order canonicalized) [C4].
- `export_is_byte_deterministic` — two exports with the same `publishedAt`: identical text and sha256 [C5].
- `export_sha256_is_prefixed_hex_of_text` — `summary.sha256 == format!("sha256:{}", hex(sha256(text)))` and matches `^sha256:[0-9a-f]{64}$`.
- `export_keys_are_sorted` — every object in the output has sorted keys.
- `export_definitions_sorted_by_id_and_empty_kinds_omitted`.
- `export_inlines_cross_boundary_references` — a type in boundary B uses a field from the primary package; the field is inlined and listed in `inlined` [C1].
- `export_omits_core_package_definitions` — a type using a `com.semanticops.core` field id: not inlined, and the bundle still installs and validates in a fresh repository (OD5).
- `export_stamps_current_data_model_revision`.
- `export_primary_package_when_selector_absent`.
- `export_unknown_selector_is_package_not_found`.
- `export_rejects_invalid_published_at` — `bundle-published-at-invalid`.
- `export_refuses_legacy_package_dependencies` — an entry without `packageId` -> `bundle-schema-invalid` with the repair hint.
- `export_output_validates_against_package_bundle_schema`.
- `export_missing_listed_definition_file_is_hard_error`.
- `export_input_json_maps_to_export_package_input` — `{"selector":null,"publishedAt":"2026-10-03T00:00:00Z"}` (the WASM `input_json` contract).

In `crates/srs-repository/tests/package_install.rs`:

- `install_refuses_when_a_boundary_lists_a_missing_definition_file` — the deliberate `collect_existing` change.

Integration `crates/srs-repository/tests/package_bundle_roundtrip.rs` (FileStore on tempdirs, as `package_install.rs` does):

- `bundle_roundtrip_all_ten_kinds_definitions_identical` — repo A: install the fixture dir; export `packages/install-fixture`; repo B (fresh): `install_package_bundle_bytes`; for every id, B's definition Value == A's; `kinds.len() == 10`; both `validate_repository` with 0 errors.
- `bundle_roundtrip_boundary_metadata_identical` — B's boundary id/namespace/name/version equal A's.
- `bundle_roundtrip_imports_report_clean` — `list_package_imports` on B: every upstream-tracked record `Clean`. Note in the test: `ImportSummary` has no theme, vocabulary, lifecycle or composition lists, so this covers fields, types, views, blueprints, protocols and relation types only; the other four are covered by the definitions-identical test.
- `bundle_roundtrip_reexport_is_byte_identical` — export from B with the same `publishedAt` equals A's text.
- `bundle_roundtrip_tree_session_matches_disk` — repo A opened as a tree session (`open_tree` over its files) exports the same bytes as the disk store (cross-store rule).

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository --all-targets -- -D warnings
```

Update this file, commit (`feat(package): deterministic .srspkg export + ten-kind round trip (#632)`). Verification Agent runs a crate-boundary and duplication audit (no second canonicalize, filename scheme, sha256 format, boundary loader or kind table).

---

### Phase 3: CLI surface

**Goal:** `srs package export` and `srs package install --bundle` call the Phase 1/2 services through <= 15-line handlers with golden payloads.

**Agent:** CLI Worker. **Write scope:** `crates/srs-cli/src/{commands/mod.rs, commands/package.rs, payload.rs, bin/generate-schemas.rs}`, `crates/srs-cli/schemas/payload/**` (via generate-schemas only), `crates/srs-cli/tests/{package_install_cli.rs, payload_contracts.rs}`.

#### Tasks

- [x] `PackageCommand::Export { .. }` and the `Install` changes (flags above); dispatch arms.
- [x] `PackageExportPayload`, `PackageExportKindEntry`, `PackageExportPayload::new`, `notes` on `PackageInstallPayload`, `impl From<InstallPackageResult> for PackageInstallPayload`.
- [x] `cmd_package_export`, `read_bundle_file`, and `cmd_package_install` in the form above (it is ~45 lines today because of inline payload mapping; the `From` impl removes that).
- [x] `write_schema!("package-export", PackageExportPayload)`; `#[test] fn package_export()` in `payload_contracts.rs`; `cargo run --bin generate-schemas`.

#### Acceptance Criteria

- [x] Both handlers <= 15 lines (rustfmt'd), one service call per branch, no `json!`; `read_bundle_file` does file I/O only.
- [x] `srs package install <dir>` output unchanged except `notes: []`.

#### Testing

In `crates/srs-cli/tests/package_install_cli.rs`:

- `package_export_cli_writes_srspkg_and_reports_sha256` — the file exists; payload `sha256` equals `"sha256:" + hex(sha256(file bytes))`; `byteLength` equals the file size.
- `package_export_cli_is_deterministic_with_published_at` — two runs with a fixed `--published-at`: byte-identical files.
- `package_install_cli_bundle_roundtrip` — export from A, `install --bundle` into fresh B, `package list` shows the boundary, `repo validate` errors == 0.
- `package_install_cli_rejects_source_dir_with_bundle` — both given: non-zero exit (clap), raw runner.
- `package_export_cli_unknown_selector_is_error_envelope` — `ok: false`.
- `package_install_cli_bundle_newer_revision_is_error_envelope` — `ok: false`, diagnostic contains `bundle-revision-too-new`.

#### Milestone gate

```bash
cargo test -p srs-cli
cargo test -p srs-cli --test payload_contracts
cargo clippy -p srs-cli --all-targets -- -D warnings
```

Update this file, commit (`feat(cli): package export + install --bundle (#632, #690)`).

---

### Phase 4: WASM surface (#663)

**Goal:** the browser can export and install bundles through the same services, and an install marks the session dirty.

**Agent:** Bindings Worker. **Write scope:** `crates/srs-bindings/src/lib.rs`, `crates/srs-bindings/tests/package_bundle.rs` (new).

#### Tasks

- [x] `export_package_bundle` and `install_package_bundle` on `SrsRepository` (signatures and doc comments above). No logic beyond (de)serialization.
- [x] Confirm the `list_package_imports_json` doc already states the `conflictState` values (PD7); no change.

#### Acceptance Criteria

- [x] `cargo build --target wasm32-unknown-unknown -p srs-bindings` succeeds.
- [x] WASM results are the service structs (`PackageBundleExport`, `InstallPackageResult`) serialized as-is: same field names as the CLI payloads, plus `text` on export (parity).
- [x] **Thinness of the two `#[wasm_bindgen]` methods is verified by audit only.** srs-bindings has no wasm-bindgen-test harness (its tests are native and never call `to_js`), and adding one is out of scope. The Verification Agent checks each body is exactly: deserialize -> one service call -> `to_js`. The JSON input contracts are tested in srs-repository (`install_options_json_maps_to_install_bundle_options`, `export_input_json_maps_to_export_package_input`).

#### Testing

`crates/srs-bindings/tests/package_bundle.rs` (native tests per the crate's convention, exercising the services the methods call on the same MemVfs store type):

- `tree_session_install_advances_write_epoch` — gallery (`open_srsj`) exports its primary package; a blank tree session (`new_tree_session` + `create_blank_repository`, as `SrsRepository::create` does) installs it; `write_epoch` strictly increases.
- `tree_session_export_does_not_advance_write_epoch`.
- `tree_session_install_then_validate_has_zero_errors`.

#### Milestone gate

```bash
cargo test -p srs-bindings
cargo build --target wasm32-unknown-unknown -p srs-bindings
cargo clippy -p srs-bindings --all-targets -- -D warnings
```

Update this file, commit (`feat(bindings): export_package_bundle + install_package_bundle (#663)`). Verification Agent: binding thinness audit (the acceptance criterion above).

---

### Phase 5: Docs, dogfood, follow-ups

**Goal:** the surface is documented, dogfooded end to end, and every deferral is filed and linked.

**Agent:** Lead Integrator. **Write scope:** `docs/dogfooding.md`, `docs/adr/050-package-bundle-codec.md`, this plan; GitHub issues/comments listed under "Deferred items".

#### Tasks

- [ ] Add scenario **S49 — Publish a package as a `.srspkg` and install it into a fresh repository** to `docs/dogfooding.md` (text below), and extend the coverage-matrix `package` row: "`srs package export` / `srs package install --bundle` in S49 (#632/#690); WASM `export_package_bundle` / `install_package_bundle` via `crates/srs-bindings/tests/package_bundle.rs` (#663)".
- [ ] Run S49 with the branch binary (`cargo run --bin srs --`) and record the outcome in the PR body.
- [ ] File and link every item in "Deferred items" (`gh-project link <parent> <child>` at creation).
- [ ] Final commit: flip ADR-050 to `accepted` and tick its charter boxes.

S49 text (paths relative to `srs-rust/`):

```bash
srs repo create --repo /tmp/s49-a --namespace com.example.s49a
srs package install crates/srs-repository/tests/fixtures/install-package --repo /tmp/s49-a
srs package export --selector packages/install-fixture --output /tmp/s49.srspkg \
    --published-at 2026-10-03T00:00:00Z --repo /tmp/s49-a           # kinds: 10, sha256 "sha256:<hex>"
srs package export --selector packages/install-fixture --output /tmp/s49b.srspkg \
    --published-at 2026-10-03T00:00:00Z --repo /tmp/s49-a && cmp /tmp/s49.srspkg /tmp/s49b.srspkg   # identical
srs repo create --repo /tmp/s49-b --namespace com.example.s49b
srs package install --bundle /tmp/s49.srspkg --repo /tmp/s49-b      # installed: 10
srs package install --bundle /tmp/s49.srspkg --repo /tmp/s49-b      # installed: 0, skippedIdentical: 10
srs package imports --repo /tmp/s49-b                                # every conflictState "clean"
srs repo validate --repo /tmp/s49-b                                  # 0 errors
# negative: set "dataModelRevision": 99 in a copy -> ok:false, bundle-revision-too-new
```

Done when: both exports are byte-identical, the reinstall skips everything, imports are clean, validate reports 0 errors, and the negative case is an error envelope.

#### Milestone gate

Full Final Acceptance below, then commit (`docs: S49 + ADR-050 accepted`).

---

## Deferred items (file in Phase 5, linked at creation)

| # | Repo | Item | Parent |
|---|---|---|---|
| D1 | srs | Amend RFC-003 [C1] and the `package-bundle.json` `mode`/`bundled` description: embedded core-package (`com.semanticops.core`) definitions MAY be omitted from a bundled bundle because every repository has them (ADR-025 / RFC-018); record the srs-rust deviation (OD5) as the motivating evidence. Also ask for RFC-003 Change C acceptance (OD4). | muDemocracy.org#242 |
| D2 | srs | srs-usage.md (Stage 7.5): document `srs package export` and `srs package install --bundle`, including the `sha256:<hex>` format and the `--published-at` reproducibility rule; fix §5g's `--source-dir` (the CLI takes a positional `<source_dir>`); remove §5c's `srs package install --url <downloadUrl>` (no such flag; registry fetch is deferred with #542); keep the §5g readme line and add "a bundle carrying `readme` is refused". | muDemocracy.org#242 |
| D3 | srs-rust | bug: `install_package_bundle` Phase 5 rebuilds `.srs-import/import-records.json` from the current run's installs only, so a later install that adds definitions drops the earlier import records (pre-existing; found while planning). Cross-reference #1152. | muDemocracy.org#242 |
| D4 | srs-rust | The primary package is read at the literal `package/` prefix (`collect_existing` convention, now `load_boundary_definitions`); a repository whose `packageRef.path` differs exports nothing for selector `None`. Route through the boundary's real path. | muDemocracy.org#242 |
| D5 | srs (comment) | srs#390: the schema already carries `themes`/`blueprints`/`protocols` (RFC-044 [R6]) and this PR round-trips all ten kinds; recommend the owner close it. Also note on muDemocracy.org#242 that its "not installable until srs#390" gate is met. | — |
| D6 | srs-web (comment) | srs-web#339: the export reports `sha256:<64 hex>` (the existing srs convention); the lock file should store that string, and the build must pass a fixed `--published-at` for the hash to stay stable. | — |
| D7 | srs-rust (comment) | srs-rust#1152: add install preview / upstream-ahead divergence against a candidate bundle (PD7) to its scope, and link D3. | — |

Already tracked, no new issue: MCP tool (srs-rust#1153), readme in `.srspkg` (srs-rust#1164), srs-web install UI (srs-web#339/#340). Not filed: subset export and `standalone` mode (no consumer; RFC-003 already describes them, file when a consumer appears).

## Final Acceptance

- [x] `cargo build --workspace` exits 0
- [x] `cargo test --workspace` exits 0 (zero failures; `SRS_SPEC_DIR` points at a fresh clone of `srs` `origin/master`)
- [x] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [x] `cargo test -p srs-cli --test payload_contracts` exits 0; `schemas/payload/package-export.json` new, `package-install.json` gains `notes`
- [x] `cargo build --target wasm32-unknown-unknown -p srs-bindings` exits 0
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schema change)
- [x] `bundle_roundtrip_all_ten_kinds_definitions_identical` and `bundle_roundtrip_reexport_is_byte_identical` pass
- [ ] Both CLI handlers <= 15 lines; WASM methods audited as deserialize -> one call -> serialize
- [ ] S49 dogfooded on the branch binary, outcome in the PR body
- [ ] D1-D7 filed or posted and linked
- [ ] ADR-050 accepted with charter boxes ticked
- [ ] PR body: `Closes #632`, `Closes #690`, `Closes #663`; decision mode complicated; `gate:owner-merge`; links ADR-050, srs-rust#1152/#1153/#1164, srs#390, D1-D4

## Deviations during implementation

- Phase 1: `InvalidPackageBundle` needed no new match arm (no exhaustive `match` on `RepositoryError` in srs-cli, srs-mcp, srs-mcp-core).
- Phase 2: `FIXTURE_DEFINITION_COUNT` was 9 (two fields), not 8; it is now 11 (all ten kinds). No `skippedDefinitions` assertion existed to update.
- Phase 2: `schemaVersion` comes from `manifest.extra["srsVersion"]` (the `Manifest` struct has no typed `srs_version`; fallback `"2.0"`).
- Phase 2: `export_omits_core_package_definitions` references the core field from a **view** (`fieldViews`), not a type. The RFC-038 [R13] catalog resolves a Type's `FieldAssignment.fieldId` only against the repository's own definition files, never the embedded core, so any type referencing a core field fails `validate_repository` with `SRS038-R13-DANGLING-REFERENCE` regardless of bundles (pre-existing; candidate follow-up issue). Also pre-existing: `validate_source_definition` (install's loader-strictness check) accepts a view without `$schema`, which the catalog then rejects with `SRS038-R8-SHAPE-NO-MATCH`; the test view carries `$schema`.
- Phase 2: `load_boundary_definitions` reports an unloadable listed file as `InvalidRepositoryInitialization` naming the boundary and path (the existing generic install error variant).
- Phase 3: rustfmt expands the struct literals, so after formatting the handler bodies are 14 lines (`cmd_package_export`) and 21 lines (`cmd_package_install`), not <= 15. Each branch is still one flag-to-struct mapping plus one service call, with no logic and no `json!`. The `(None, None)` arm is replaced by `source_dir.context(..)?` (clap already enforces it), which makes the handler shorter.
- Phase 3: `crates/srs-cli/Cargo.toml` gains `sha2` and `hex` as dev-dependencies (both are workspace deps already in the lockfile) so the CLI test can recompute the file hash. A `package_install` golden contract test was also added, because `package-install.json` changes and had no contract test before.
- Phase 4: `tree_session_install_then_validate_has_zero_errors` uses the srs-repository `install-package` fixture as its source (installed into a blank tree session, exported, then installed into a second blank tree session), not the gallery. The gallery fixture itself fails validation: it has three V8 errors (types `decision`/`article`/`role` carry `lifecycleRef` `3c504040-...`, which resolves nowhere) and no root container. A faithful bundle carries those errors along. The two `write_epoch` tests still use the gallery. Binding thinness was audited: each method is deserialize -> one service call -> `to_js`. `list_package_imports_json` already documents `conflictState` "clean" | "local-ahead" (PD7), so it is unchanged.
- Stage 6: `bash scripts/check-schema-sync.sh` exits 1. All three divergences are in the **srs-vscode** sibling mirror (`package-bundle.json`, `package-manifest.json`, `srsj-envelope.json`), measured against the sibling `../srs` worktree. This repo's mirror (`crates/srs-schema/schemas/2.0/`) is byte-identical to a fresh clone of `srs` `origin/master` (5ed71a9) for every spec schema, so nothing here needs a sync. srs-vscode is out of scope: never touch sibling repos.

## Coordination Rules

- Agents keep to their write scopes unless the Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- At the end of each phase: verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed without the milestone gate.
- Verification Agent runs after Phases 2 and 4 and before final sign-off.

## Assumptions

- srs-rust `origin/master` 2f478edd; srs `origin/master` cbb44cb; the `package-bundle.json` mirror equals the canonical schema.
- `jsonschema` (0.29, draft 2020-12 defaults) does not assert `format`, so a non-UUID primary package `id` passes schema validation; export adds no stricter check.
- Same-UUID-different-version definitions in one repository are deduplicated by id on install (pre-existing install behaviour; upgrade work is #1152).
- A `mode: "standalone"` bundle produced elsewhere is accepted by the reader; unresolved dependencies surface in `repo validate`, and RFC-044 requirements via `check_package_requirements`.

## Review resolutions (Stage 3)

Architecture review:

| # | Sev | Resolution |
|---|---|---|
| 1 | should-fix | PD8: keep `sha256:<hex>`; pinned in the summary/payload docs, ADR-050, S49, D2/D6; tests assert the exact string. |
| 2 | should-fix | One `package_service::definition_rel_path`, adopted by create_field/type/relation-type and the reader; no `repository_portability` widening, no `definition_dir` in store.rs. |
| 3 | should-fix | A missing or blank name -> `bundle-definition-invalid`; test `read_bundle_rejects_definition_without_name`. |
| 4 | should-fix | Notes moved to the reader's return type `ReadPackageBundle`; `PackageSourceBundle` unchanged. |
| 5 | should-fix | Migration called unconditionally (PD1c); the stamp fix is now reachable and tested; `read_bundle_current_revision_passes_through_unchanged` added. |
| 6 | should-fix | serde contract tests moved to srs-repository; no wasm-bindgen-test harness exists, so WASM method thinness is stated as audit-only. |
| 7 | should-fix | ADR-050 gets a Deviation line; "self-contained" reworded; D1 files the [C1] amendment in srs. |
| 8 | should-fix | PD8 + ADR-050 + doc comments: a stable sha256 requires `--published-at`; D6 tells srs-web#339. |
| 9 | should-fix | ADR table adds 004, 009, 015, 033/039, 035, 037; ADR-050 gets a "Related" line on artifact distinctions. |
| 10 | should-fix | Shared `load_boundary_definitions` used by `collect_existing` and export (one ADR-042 shim point); the deliberate behaviour change is tested. |
| 11 | nit | ADR-050: "verbatim means value-equal; keys are canonicalized". |
| 12 | nit | Coded variant justified by the `ActorProvenance` precedent; Phase 1 task greps for exhaustive matches. |
| 13 | nit | Handler precedents cited (`archive.rs`, `attachment.rs:81`); no `unwrap_or_default`; `read_bundle_file` committed. |
| 14 | nit | Coverage note added to `bundle_roundtrip_imports_report_clean`. |
| 15 | nit | ADR-050 charter boxes unticked until the final commit; the `packageRef.path` limitation is filed (D4) rather than left as an assumption. |

Plan review:

| # | Sev | Resolution |
|---|---|---|
| B1 | blocking | ADR-050 ships in this PR: the draft exists, Phase 1 commits it, Phase 5 accepts it. |
| B2 | blocking | `CURRENT_DATA_MODEL_REVISION` cited at `field_type_migration_service.rs:32`. |
| S3 | should-fix | Phase 1 says `migrate_package_bundle_value` is an existing function being modified. |
| S4 | should-fix | Fixture count change pinned to `crates/srs-repository/tests/package_install.rs`, 8 -> 10. |
| S5 | should-fix | Phase 1 acceptance criterion names all six codes. |
| N1 | nit | PD1 rewritten as (a)-(e). |
| N2 | nit | The `installedAt` exclusion is explained in the cross-store test. |

### Declined review findings

None declined.
