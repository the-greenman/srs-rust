# Plan: Tree as the primary in-memory model — Vfs seam, MemVfs FileStore, tree bindings

> Issue: [#684](https://github.com/the-greenman/srs-rust/issues/684) — Phase 1 of Epic 10
> ([muDemocracy.org#101](https://github.com/the-greenman/muDemocracy.org/issues/101)).
> Phase 2 (the srs-web GitHub tree provider) is a separate srs-web issue and is out of scope here.
> Rev 2 — incorporates Stage 3 review findings (see issue comments).

## Summary

srs-web must open a whole exploded SRS repository from GitHub, edit it in the browser, and commit
all edits back as **one git commit with clean per-file diffs**. The current browser store
(`JsonStore`) cannot deliver this: `archive_pack` never emits per-definition files,
`import_repository_snapshot` re-canonicalizes every path, and raw file bytes are not preserved — so
untouched files would be rewritten on every save. Owner direction (2026-07-22): the in-memory VFS
tree becomes the **primary operational model**, especially for web; loading a `.srsj` creates an
in-memory tree; import/export formats are codecs at the boundary and say nothing about how data is
managed in memory. Additionally: **no backwards compatibility** — existing repos/archives are
force-updated to the new form on next save; the only legacy affordance is a *read-side* migration
ramp so old archives can still be opened in order to be updated.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | session lead |
| Repository Worker | session lead (phases 0–3) |
| Bindings Worker | session lead (phase 4) |
| Verification | Verification Agent (`agents.md#verification-agent`) |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-037](../docs/adr/037-vfs-tree-primary-model.md) | Vfs seam; FileStore over Vfs; the in-memory file tree (FileStore + MemVfs) is the primary operational model for all WASM sessions; JsonStore demoted to `.srsj` codec. Partially supersedes ADR-013 (which rejected the "map of `{path: content}`" model) and amends ADR-015 ("WASM `SrsRepository` wraps a `JsonStore`"). | proposed |
| [ADR-038](../docs/adr/038-srs-archive-pure-tree-zip.md) | `.srs` archive = deterministic zip of the exploded tree, per-definition files included (all package boundaries), no `package/package.snapshot.json`. Supersedes ADR-033 items 8–9. Legacy snapshot archives remain **readable** as a migration ramp only. | proposed |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | Governs bindings thinness and the wasm32 CI gate. Bundle-format rationale partially superseded by ADR-037 (reciprocal header update is a Phase 0 task). | accepted |
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | `materialize_tree` consumes the `ensure_target_empty` / `initialize_repository` import contract unchanged. | accepted |
| [ADR-031](../docs/adr/031-source-doc-blob-portability.md) | `materialize_tree` uses `include_content_blobs: true`; source-doc binaries land as MemVfs files. | accepted |
| [ADR-017](../docs/adr/017-deterministic-srsj-serialization.md) | Determinism: `preserve_order` stays disabled; MemVfs uses `BTreeMap` for the same reason. | accepted |
| [ADR-007](../docs/adr/007-file-index-io-ordering.md) | File-before-index write ordering — the Vfs refactor must not reorder writes. | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | No CLI payload changes in this plan (see Contracts). | accepted |
| [ADR-021](../docs/adr/021-jsonstore-batch-write-mode.md) | The `"<memory>"` sentinel convention — reused for MemVfs-backed `repository_root()` display. | accepted |

**Interop register consult** (`srs/docs/research/alignment-opportunities.md`, 2026-07-22): no entry
contradicts this plan. Relevant citations: **item 11** (Automerge sync transport — WATCH): git
remains the transport for Epic 10; CRDT evaluation only if Live Governance demands multi-writer
realtime. **item 18** (Frictionless Data Package — COMPONENT): manifest `$schema` self-versioning
noted; no action. The archive realignment *improves* interop: a `.srs` becomes a plain zip of the
spec-conformant ext:repository layout, matching the spec's own definition ("structurally identical
to a repository snapshot", lossless round-trip).

**Design decisions already made by owner (recorded, no Stage-2 pause needed):**
1. Tree is the primary operational model; formats are boundary codecs (2026-07-22).
2. No backwards compatibility; existing repos are force-updated. Write-side emits only the new
   form; the legacy `.srs` snapshot **read** path is retained solely as the migration ramp
   (without it, an old archive could not be opened in order to be updated) and its removal is a
   filed follow-up.

## Contracts

### CLI output contract (ADR-011)

**No new/changed command payload shapes.** `srs archive pack`/`unpack` keep their payload structs
(`ArchivePackPayload`, `ArchiveUnpackPayload`) unchanged; the *content* of packed archives changes
(per-definition files included, snapshot omitted) but the CLI envelope does not. No
`generate-schemas` run needed. `cargo test --test payload_contracts` must still pass.

### Entity schema sync

No files under `srs/docs/schema/2.0/` change. No mirror sync needed.

## Scope

- `Vfs` trait + `DiskVfs` + `MemVfs` in `crates/srs-repository/src/vfs.rs`.
- `FileStore` refactored over `Vfs` (behavior-neutral for disk).
- `tree_session.rs`: `open_tree`, `export_tree`, `materialize_tree`.
- Archive realignment in `archive.rs` (pure tree zip incl. **all** package boundaries; legacy read
  ramp; missing referenced file = hard error).
- Shared `type_json.rs` module (mirrors `field_json.rs`) preserving `_extra` — fixes the
  `$schema`/`aiGuidance` drop in both loaders.
- WASM bindings: `SrsRepository { store: FileStore }` for all load paths; new
  `load_tree` / `export_tree`; `export_srsj` via codec.
- Reciprocal ADR header updates (013/015/033).
- Tests incl. byte-stability round-trip against committed fixtures generated from pristine
  pre-change code.

**Out of scope** (each filed as a follow-up issue in Stage 3):

- srs-web GitHub tree provider (Epic 10 Phase 2 — srs-web repo issue, linked under the epic).
- CLI adoption of the tree model for `--repo <file>.srsj` sessions (JsonStore remains the CLI's
  operational store for `.srsj` until then).
- Removal of the legacy `.srs` snapshot read ramp (after ecosystem archives are migrated).
- JsonStore manifest key-order nondeterminism (`json_store.rs::load_text_file("manifest.json")`
  serializes the flattened-HashMap `Manifest` without a `to_value` sort — affects srsj-session
  archive exports; the tree path is not affected).
- `package_install_service` bytes-based input variant (currently direct fs; needed only if
  srs-web ever installs packages).
- Spec/impl divergence on the `.srs` marker (spec: file with optional format version; impl:
  directory) — pre-existing, tracked separately with `requires-spec-rfc`.

## Phases

### Phase 0: Fixtures from pristine code + ADR hygiene

**Goal:** both test fixtures exist, generated by **pre-change** code, and superseded ADRs carry
reciprocal headers — before any functional code changes land.

**Agent:** Repository Worker

#### Tasks

- [x] Generate the exploded fixture at `crates/srs-repository/tests/fixtures/exploded-basic/`
  using the **current pristine** CLI (`cargo run --bin srs` at the branch point, before any code
  edits). Exact composition:
  - `srs repo create` (namespace `com.example.treefix`), then via CLI: one `string` field
    (`title`), one `boolean` field (`approved`), one Type `com.example.treefix/decision@1`
    combining both, two Records of that Type with distinct values, one `precedes` Relation
    between them, one Container grouping both records.
  - Hand-add `$schema` and `aiGuidance` keys to the Type's definition file (this is the
    fidelity-fix probe — legal per the schema, currently dropped on load).
  - Add decoys the SRS code must never touch: `README.md` ("decoy — must survive round-trips
    byte-identical") and `.github/workflows/ci.yml` (any valid YAML).
  - The fixture is committed as files; UUIDs/dates are frozen at generation time (no
    regeneration in tests).
- [x] Generate the legacy archive fixture
  `crates/srs-repository/tests/fixtures/legacy-snapshot.srs` by running the **pristine**
  `srs archive pack` against the exploded fixture. Verify it contains
  `package/package.snapshot.json` and no `package/fields|types/*.json` entries before
  committing.
- [x] Reciprocal ADR headers: ADR-013 gains `Superseded by: ADR-037 (bundle-format rationale
  only)`; ADR-015 gains `Amended by: ADR-037`; ADR-033 gains `Superseded by: ADR-038 (items
  8–9)`.

#### Acceptance Criteria

- [x] Both fixtures committed; legacy archive verifiably in old format (snapshot present, no
  per-definition entries).
- [x] ADR-013/-015/-033 headers updated; ADR-037/-038 committed as `proposed`.

#### Testing

```bash
unzip -l crates/srs-repository/tests/fixtures/legacy-snapshot.srs   # manual inspection
```

#### Milestone gate

Standard gate (`cargo test -p srs-repository` still green — no code changed); commit.

---

### Phase 1: Vfs seam (behavior-neutral)

**Goal:** `FileStore` reads/writes exclusively through a `Vfs` trait; disk behavior is
byte-identical; full suite green with zero behavioral test changes.

**Agent:** Repository Worker

#### Tasks

- [x] Create `crates/srs-repository/src/vfs.rs`:
  - `pub trait Vfs: std::fmt::Debug` (Debug supertrait — `FileStore` derives `Debug` over
    `Rc<dyn Vfs>`): `read_bytes(&self, rel: &str) -> Result<Vec<u8>, RepositoryError>`,
    `read_to_string`, `write(&self, rel: &str, bytes: &[u8])`, `remove` (idempotent),
    `exists`, `is_dir`, `byte_len`, `list_dir(&self, rel) -> Vec<VfsEntry>` (direct children,
    name + is_dir), `list_recursive(&self, rel) -> Vec<String>`, `create_dir_all`,
    `check_within_root(&self, rel) -> Result<(), RepositoryError>` (path-escape guard),
    `as_mem_snapshot(&self) -> Option<BTreeMap<String, Vec<u8>>>` (Some for MemVfs — the
    Mem/Disk discriminator used by `export_tree` and the pack branch; avoids `Any`
    downcasting).
  - `pub(crate) const SRS_MARKER_DIR: &str = ".srs";` — single marker literal, referenced by
    the store's `repository_exists` check and `tree_session`.
  - All paths are repo-relative, forward-slash. Errors wrap `std::io::Error` preserving
    `ErrorKind::NotFound` so `RepositoryError::is_not_found()` (error.rs) keeps working —
    the `load_relations_raw_text` filename fallback depends on it.
  - `DiskVfs { root: PathBuf }`: absorbs `FileStore`'s root-join and the `canonicalize()`
    escape check currently inside `validate_package_ref_path` (store.rs:1897-1925).
  - `MemVfs`: `RefCell<BTreeMap<String, Vec<u8>>>` files + `RefCell<BTreeSet<String>>`
    explicit dirs; `is_dir` = explicit dir ∨ any key has `"{rel}/"` prefix; listings via
    BTreeMap prefix range scans; lexical `..`-escape check.
- [x] Refactor `store.rs`: `FileStore { repo_root: PathBuf, vfs: Rc<dyn Vfs> }` — `repo_root`
  becomes **display-only** (feeds `repository_root()`; MemVfs-backed stores use
  `PathBuf::from("<memory>")` per the ADR-021 sentinel convention); all I/O routes through
  `vfs`. `FileStore::new(root)` keeps its signature (wraps `DiskVfs`); add
  `FileStore::from_vfs(vfs: Rc<dyn Vfs>)`. Route the ~39 non-test fs/`Path` call sites
  (`std::fs::*`, `.exists()`, `.is_dir()`, `.canonicalize()`, `.is_file()`, `write_json`,
  `save_text_file`, `save_binary_file`, `list_instance_files`, `collect_paths_recursive`,
  `repository_exists`, `initialize_repository`, `file_byte_len`) through the Vfs.
  `#[cfg(test)]` fixture builders stay on `std::fs` (they construct real on-disk repos).
  `validate_package_ref_path` **remains** a `RepositoryStore` trait method — its
  implementation delegates to `Vfs::check_within_root` + existence checks.
- [x] Delete the dead free fn `manifest.rs::load_manifest` (only caller is its own test);
  keep `FileStore::load_manifest` as the single implementation.

#### Acceptance Criteria

- [x] `cargo test -p srs-repository` passes with no behavioral test changes.
- [x] `FileStore::new` public signature unchanged; all existing callers compile untouched.
- [x] MemVfs not-found errors satisfy `RepositoryError::is_not_found()`.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write:

- `vfs::tests::mem_vfs_roundtrip` — write/read/remove/exists/list on MemVfs.
- `vfs::tests::mem_vfs_not_found_kind` — read of a missing path is `is_not_found()`.
- `vfs::tests::mem_vfs_list_dir_and_recursive` — nested dirs, explicit-dir entries.
- `vfs::tests::disk_vfs_escape_rejected` / `mem_vfs_escape_rejected` — `..` traversal fails.
- `vfs::tests::as_mem_snapshot_discriminates` — Some for MemVfs, None for DiskVfs.

#### Milestone gate

Standard gate (criteria → tests → `cargo test -p srs-repository` + clippy → checkboxes → commit).

---

### Phase 2: Tree sessions + TypeJson fidelity

**Goal:** an in-memory exploded tree opens as a `FileStore`, round-trips byte-identically when
untouched, and single edits produce single-file diffs.

**Agent:** Repository Worker

#### Tasks

- [ ] Create `crates/srs-repository/src/tree_session.rs`:
  - `pub fn open_tree(files: BTreeMap<String, Vec<u8>>) -> Result<FileStore, RepositoryError>` —
    errors if `manifest.json` absent; populates a `MemVfs`; registers the `SRS_MARKER_DIR`
    explicit dir when no `.srs` entry exists (git cannot track empty dirs;
    `repository_exists` requires it).
  - `pub fn export_tree(store: &FileStore) -> Result<BTreeMap<String, Vec<u8>>, RepositoryError>`
    — `vfs.as_mem_snapshot()` dump (unknown files — README, CI config — ride through
    untouched); emits `.srs/.gitkeep` (empty) iff no file under `.srs/` exists, so exported
    trees are clone-detectable. Errors when `as_mem_snapshot()` is `None` (disk stores export
    via the filesystem, not this API).
  - `pub fn materialize_tree(source: &dyn RepositoryStore) -> Result<FileStore, RepositoryError>`
    — `export_repository_snapshot_with_options(include_content_blobs: true)` →
    `import_repository_snapshot` into a fresh MemVfs-backed FileStore. The bridge that turns
    any codec-loaded repo (JsonStore srsj sessions, legacy archives) into the operational tree.
  - Re-export the three functions from `lib.rs`.
- [ ] Fidelity fix via shared module (mirrors `field_json.rs`): extract
  `crates/srs-repository/src/type_json.rs` with the `TypeJson` intermediate and a single
  `into_record_type()` that carries `#[serde(flatten)] _extra` into `RecordType.extra`
  (`RecordType` already has `#[serde(flatten)] pub extra` in
  `crates/srs-core/src/types/record_type.rs` — **no srs-core change**). Replace the two
  duplicated inline conversions: `crates/srs-repository/src/store.rs` (TypeJson ~L520-716)
  and `crates/srs-repository/src/json_store.rs` (TypeJson ~L86-531).

#### Acceptance Criteria

- [ ] `open_tree(fixture)` → no edits → `export_tree` returns a `BTreeMap` equal to the input
  map for every input path (full `BTreeMap<String, Vec<u8>>` equality; the only permitted
  delta is the added `.srs/.gitkeep` entry).
- [ ] `open_tree` → `update_record` on one record → exported map differs from input in
  exactly that record's file (assert the precise changed-key set, not just "something
  changed"; manifest untouched when the index is unchanged).
- [ ] Type definition edit preserves `$schema`/`aiGuidance` in the written file.
- [ ] Decoy files are byte-identical in every scenario.
- [ ] `materialize_tree(JsonStore::from_srsj(...))` yields a store whose `validate` and
  `list_records` outputs match the JsonStore source.

#### Testing

```bash
cargo test -p srs-repository tree_session
```

Specific tests to write (in `crates/srs-repository/tests/tree_session.rs`; a shared helper
asserts "exported map == input map except exactly these keys"):

- `open_export_roundtrip_byte_identical` — the load-bearing clean-diff guarantee.
- `single_record_edit_single_file_diff`.
- `type_edit_preserves_extra` — `$schema`/`aiGuidance` intact (fails before the fidelity fix).
- `decoys_untouched_after_edits`.
- `open_tree_missing_manifest_errors`.
- `materialize_from_srsj_parity` — validate/list parity vs the JsonStore source.
- `export_tree_synthesizes_marker` — `.srs/.gitkeep` appears iff absent.
- `export_tree_disk_store_errors` — DiskVfs-backed store returns an error.

#### Milestone gate

Standard gate; commit.

---

### Phase 3: Archive realignment (pure tree zip)

**Goal:** `.srs` = deterministic zip of the exploded tree; legacy snapshot archives still load
(read-only migration ramp); no snapshot file is ever written again.

**Prerequisite:** Phase 0 fixtures committed (verified present before starting).

**Agent:** Repository Worker

#### Tasks

- [ ] `archive.rs` pack: replace the snapshot-driven entry list.
  - MemVfs-backed store: zip the `export_tree` map verbatim (decoys included — the archive is
    a faithful snapshot of the session tree; divergence from the disk path is documented in
    ADR-038).
  - Disk store: enumerate from the store — manifest raw, **every package boundary's**
    `package.json` raw plus every per-definition file each references (closes the pre-existing
    sub-package pack gap), relations raw, instanceIndex files, containerIndex files,
    source-document sidecars + binaries. A referenced file that does not exist is a **hard
    error** naming the missing path — never a silent skip.
  - Drop the `package/package.snapshot.json` entry. Keep ADR-033 determinism (sorted entries,
    zeroed timestamps, Deflated).
- [ ] `archive.rs` unpack: try native tree load first (`open_tree` over the unzipped map —
  requires `package/package.json` present with resolvable definition files); if the zip
  carries `package/package.snapshot.json` **and** lacks per-definition files, fall back to
  the legacy snapshot import (existing code path) into a MemVfs-backed FileStore target —
  marked `// legacy migration ramp — remove after ecosystem archives are migrated (#<follow-up>)`.
- [ ] `archive_unpack(reader, target)` keeps its signature (CLI unpack-to-disk unchanged);
  add `archive_to_tree(reader) -> Result<FileStore, RepositoryError>` for in-memory callers.
- [ ] Update existing archive tests for the new entry list; keep determinism tests.

#### Acceptance Criteria

- [ ] Packed archives contain per-definition files for all boundaries and **no**
  `package.snapshot.json`.
- [ ] New-format pack → unpack round-trip is byte-faithful (paths preserved, no
  re-canonicalization) for tree-backed sources.
- [ ] `tests/fixtures/legacy-snapshot.srs` still unpacks and validates via the ramp.
- [ ] A pack with a dangling definition reference fails with the missing path in the error.
- [ ] Determinism tests (`test_archive_determinism`, `test_archive_zip_entry_order`) pass.

#### Testing

```bash
cargo test -p srs-repository archive
```

Specific tests to write/adjust:

- `pack_contains_definition_files_no_snapshot`.
- `pack_unpack_tree_roundtrip_byte_faithful`.
- `legacy_snapshot_archive_still_loads` — against the committed Phase 0 fixture.
- `pack_missing_definition_file_errors`.
- Existing round-trip and determinism tests updated in place.

#### Milestone gate

Standard gate; commit.

---

### Phase 4: WASM bindings — one store model

**Goal:** every load path yields a MemVfs-backed `FileStore` session; tree load/export exposed;
formats are codecs at the boundary.

**Agent:** Bindings Worker

#### Tasks

- [ ] `SrsRepository { store: FileStore }` (was `JsonStore`) — service call sites unchanged
  (`&self.store` coerces to `&dyn RepositoryStore`).
- [ ] `load(srsj)` → `srsj_migration_service::load_from_srsj` (codec, keeps RFC-014 + srsj
  open-time migrations) → `materialize_tree`.
- [ ] `load_archive(bytes)` → `archive_to_tree`.
- [ ] `load_tree(files: JsValue)` (new) — JS object `{ "<path>": Uint8Array }` → `open_tree`.
- [ ] `export_tree() -> JsValue` (new) — `tree_session::export_tree` → JS object of
  `Uint8Array`s.
- [ ] `export_srsj()` — snapshot → fresh in-memory `JsonStore` codec → `to_srsj_string`
  (documented: this projection re-canonicalizes paths; `.srsj` is an interchange format,
  not the session state).
- [ ] `export_archive()` — unchanged call (`archive_to_vec(&self.store)`).
- [ ] `get_attachment_bytes` now serves real bytes for tree/archive-loaded repos via
  FileStore `load_binary_file`.

#### Acceptance Criteria

- [ ] All existing srs-bindings native tests pass unmodified (or with mechanical updates only).
- [ ] `load` → edit → `export_srsj` → `load` round-trip: validate/list parity.
- [ ] `load_tree`/`export_tree` round-trip byte-identity (native test via the underlying
  service functions; js-sys types are covered by the wasm32 build gate per ADR-013).
- [ ] `cargo build --target wasm32-unknown-unknown -p srs-bindings` succeeds.

#### Testing

```bash
cargo test -p srs-bindings
cargo build --target wasm32-unknown-unknown -p srs-bindings
```

Specific tests to write:

- `crates/srs-bindings/tests/tree_roundtrip.rs` — fixture tree → open → edit → export;
  byte-diff is exactly the edit.
- Existing `attachment_bytes.rs` extended: archive-loaded repo serves bytes (was
  JsonStore-`binary_files`; now MemVfs).

#### Milestone gate

Standard gate (`-p srs-bindings` + wasm32 build); commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] No entity schema changes (`check-schema-sync.sh` untouched surface)
- [ ] `cargo build --target wasm32-unknown-unknown -p srs-bindings` succeeds
- [ ] Byte-stability: `open_export_roundtrip_byte_identical` and
  `single_record_edit_single_file_diff` pass — the clean-diff guarantee Epic 10 needs
- [ ] Legacy `.srs` fixture still loads (migration ramp verified)

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **Phase 0 must complete before any functional change** — fixtures are only valid if generated
  by pristine pre-change code. Phase 3 verifies both fixtures exist before starting.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and
  pass, update the plan checkboxes, then commit. Do not proceed until the milestone gate passes.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- Owner decisions of 2026-07-22 stand: tree-primary model; no backwards compatibility (legacy
  `.srs` **read** ramp only, filed for removal).
- `import_repository_snapshot` and `export_repository_snapshot_with_options` remain the
  snapshot machinery for codecs and RFC-014 portability — unchanged by this plan.
- The CLI keeps `JsonStore` as the operational store for `--repo <file>.srsj` sessions until the
  filed follow-up lands; nothing in this plan changes CLI behavior except archive content.
- srs-web consumes the new bindings via the release artifact (`srs-bindings-web.tar.gz`,
  auto-published on master merge); no cross-repo coordination needed in this PR.
