# Plan: repo upgrade — in-place path normalisation (#162)

> **Spec gate:** No spec change required. `srs repo upgrade` uses the existing
> `canonical_instance_path` logic from `repository_portability.rs`; no new SRS data-model
> types, extensions, or JSON schemas are needed. `srs-usage.md` is updated as Stage 7.5
> documentation, not an RFC gate.

## Summary

`srs repo copy` already normalises instance file paths to the `{slug}-{id8}.json` convention
(ADR-008, issue #140) as a side-effect of copying. Users with file-backed repos that pre-date
this convention have no way to normalise paths without creating a full copy. This plan adds
`srs repo upgrade` — a CLI command that applies path normalisation in-place to a FileStore
repository, updating the manifest's `instanceIndex` and renaming the physical files atomically
(manifest written before old files deleted, so the repo is always consistent).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| CLI Worker | — |
| Verification Agent | — |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-007](../docs/adr/007-file-index-io-ordering.md) | I/O order: write canonical files → update and persist manifest → delete old files. Deletes happen only after the index is persisted; orphaned files on crash are harmless; dangling index entries are prevented. | accepted |
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | `upgrade_repository_paths` is the in-place sibling of `copy_repository` — normalises paths without a target-store round-trip | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `upgrade_repository_paths` takes no input struct (store is the only parameter; no filter/config axes — a zero-field input struct would be cargo cult); returns `UpgradeRepositoryPathsResult`; all logic in `srs-repository`, zero business logic in the CLI handler | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | New `RepoUpgradePayload` struct in `payload.rs`; golden schema regenerated | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | **No WASM binding.** `upgrade_repository_paths` is a FileStore-specific operation (physical file renames). WASM uses `JsonStore`, which has no persistent file paths to normalise. Consistent with `copy_repository`, which also has no WASM binding. No new ADR needed — ADR-013 already notes that FileStore uses `std::fs` and is not callable from WASM. | accepted |

No new ADRs required — the decisions above are governed by existing accepted ADRs.

---

## Contracts

### CLI output contract (ADR-011)

**New command added:** `srs repo upgrade`

New payload struct in `crates/srs-cli/src/payload.rs`:

```rust
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoUpgradePayload {
    /// Files that were renamed to their canonical slug-id8 path.
    pub renames: Vec<InstancePathRename>,
    /// Instances whose path was already canonical — no rename needed.
    pub already_canonical_count: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InstancePathRename {
    pub instance_id: String,
    pub from_path: String,
    pub to_path: String,
}
```

Run `cargo run --bin generate-schemas` after adding; commit the new
`schemas/payload/repo-upgrade.json`.

Verification: `cargo test --test payload_contracts` must pass.

### Entity schema sync

No changes to `srs/docs/schema/2.0/` — no entity schemas change.

---

## Scope

- Add `upgrade_repository_paths` service function in
  `crates/srs-repository/src/repository_portability.rs`.
- Make `canonical_instance_path` `pub(crate)` so the new service can call it directly
  (it is currently private within that file — still private to the crate).
- Migrate revision sidecar files (`*.revisions.json`) alongside renamed record files,
  using `sidecar_path_for` from `revision_service.rs` (same crate, `pub(crate)`).
- Add `RepoUpgradePayload` and `InstancePathRename` payload structs to
  `crates/srs-cli/src/payload.rs`.
- Add `RepoCommand::Upgrade` variant to `commands/mod.rs` and `cmd_repo_upgrade` handler in
  `commands/repo.rs`.
- Update `srs-usage.md` and `docs/dogfooding.md` in Stage 7.5/7.6.

**Out of scope:**

- WASM binding — no WASM consumer for FileStore-specific path operations (see ADR-013 note above).
- Schema migrations (e.g. `relationType → key` field renames) — path normalisation only.
- `JsonStore` / in-memory upgrade — meaningless without persistent file paths; the command errors
  clearly if the resolved store is not FileStore (enforced in the CLI handler by using
  `FileStore::new` directly, same pattern as `cmd_repo_copy`).
- Dry-run mode — deferred; the idempotency guarantee makes a dry-run less critical.

---

## Phases

### Phase 1: Service — `upgrade_repository_paths`

**Goal:** The service function exists in `srs-repository`, passes tests on both MemoryStore and
FileStore, and is idempotent.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/repository_portability.rs`:
  - Change `fn canonical_instance_path` to `pub(crate) fn canonical_instance_path`.
  - Add public result structs:
    ```rust
    #[derive(Debug, Clone)]
    pub struct InstancePathRename {
        pub instance_id: String,
        pub from_path: String,
        pub to_path: String,
    }

    #[derive(Debug)]
    pub struct UpgradeRepositoryPathsResult {
        pub renames: Vec<InstancePathRename>,
        pub total_instances: usize,
    }
    ```
  - Add `pub fn upgrade_repository_paths(store: &dyn RepositoryStore) -> Result<UpgradeRepositoryPathsResult, RepositoryError>` implementing the ADR-007-ordered strategy:
    1. **Plan phase** — load manifest; for each `InstanceIndexEntry`, call `store.load_instance_json(entry.path())` to get the file value (`typeName` for tier-1/2 records lives in the file, not the manifest entry — do not construct `SnapshotInstance` from the manifest entry alone). Construct a `SnapshotInstance` and call `canonical_instance_path`. If the current path differs from canonical, record a `PlannedRename { manifest_index, instance_id, from_path, to_path, value }`. Detect path collisions and return `RepositoryError::InvalidSnapshotData` immediately.
    2. **Apply phase** — for each planned rename: `ensure_instance_parent(store, &to_path)`, then `store.save_instance_json(&to_path, &value)`. Also copy any revision sidecar: `sidecar_path_for(&from_path)` → `store.load_instance_json(...)` → on success, `store.save_instance_json(&sidecar_path_for(&to_path), &sidecar_value)`; on `RepositoryError::NotFound` or `Io { source }` where `source.kind() == NotFound`, skip silently; propagate other errors. `sidecar_path_for` is `pub(crate)` in `revision_service.rs`.
    3. **Manifest update** — patch each renamed entry's `path` field in the manifest and call `store.save_manifest(&manifest)`. (ADR-007: index must be persisted before any deletes.)
    4. **Cleanup** — for each rename: `let _ = store.delete_instance_file(&from_path)`; also `let _ = store.delete_instance_file(&sidecar_path_for(&from_path))` if the old sidecar existed. Both deletes are best-effort; orphan files are harmless per ADR-007.
    5. Return `UpgradeRepositoryPathsResult { renames, total_instances }`.
  - If no renames are needed, return early without touching the manifest.

- [ ] Export the result types from `crates/srs-repository/src/lib.rs` — add to the existing
  `pub use repository_portability::{...}` block alongside `SnapshotRepository`/`SnapshotInstance`.

#### Acceptance Criteria

- [ ] `upgrade_repository_paths` on a repo with all-canonical paths returns `renames: []`.
- [ ] `upgrade_repository_paths` on a repo with one non-canonical tier-2 path renames it to
  `records/tier-2/{typeName-slug}-{id8}.json`, updates the manifest index, and deletes the old file.
- [ ] `upgrade_repository_paths` on a repo with a non-canonical note path renames it to
  `records/notes/{title-slug}-{id8}.json`.
- [ ] Running upgrade twice produces 0 renames on the second call (idempotent).
- [ ] `srs repo validate` on the upgraded repo exits with 0 diagnostics.

#### Testing

Specific tests to add in `crates/srs-repository/src/repository_portability.rs` (or a new
`upgrade_service.rs` test module):

- `upgrade_no_op_when_paths_canonical` — create a FileStore fixture (via `copy_repository` from
  a MemoryStore so all paths are canonical), call `upgrade_repository_paths`, assert
  `result.renames` is empty.
- `upgrade_renames_non_canonical_tier2_path` — using a MemoryStore: init repo, manually insert a
  tier-2 instance at `"records/tier-2/old.json"` via `store.save_instance_json` + manifest patch,
  call `upgrade_repository_paths`, assert one rename with correct `from_path`/`to_path` and that
  the manifest entry reflects the new path.
- `upgrade_renames_non_canonical_note_path` — same pattern for a tier-0 note instance.
- `upgrade_is_idempotent` — call `upgrade_repository_paths` twice on the same store, assert
  second call returns empty `renames`.
- `upgrade_does_not_rename_already_canonical_paths` — round-trip through `copy_repository`
  (which canonicalises), then `upgrade_repository_paths` — assert 0 renames.
- `upgrade_renames_non_canonical_path_on_filestore` — using a `TempDir`-backed `FileStore`:
  write a non-canonical instance file to disk via `store.save_instance_json("records/tier-2/old.json", &value)` and patch the manifest entry, call `upgrade_repository_paths`, assert the canonical file exists on disk, the old file is absent, and the manifest entry reflects the new path. This is the required cross-store roundtrip test (CLAUDE.md: "New service features need at least one cross-store roundtrip test").
- `upgrade_moves_revision_sidecar` — inject a non-canonical record file plus its `.revisions.json` sidecar (via `sidecar_path_for`), call upgrade, assert the sidecar exists at the new canonical sidecar path and the old sidecar is absent.

```bash
cargo test -p srs-repository upgrade
cargo clippy -p srs-repository -- -D warnings
```

#### Milestone gate

1. All acceptance criteria above checked.
2. All named tests exist and pass.
3. `cargo test -p srs-repository` passes.
4. `cargo clippy -p srs-repository -- -D warnings` clean.
5. Commit: `feat(repository): upgrade_repository_paths service (#162)`.

---

### Phase 2: CLI — `srs repo upgrade` command

**Goal:** `srs repo upgrade --repo <path>` is a working CLI command with a typed payload.

**Agent:** CLI Worker

#### Tasks

- [ ] In `crates/srs-cli/src/payload.rs`, add:
  ```rust
  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct InstancePathRename {
      pub instance_id: String,
      pub from_path: String,
      pub to_path: String,
  }

  #[derive(Debug, Serialize, JsonSchema)]
  #[serde(rename_all = "camelCase")]
  pub struct RepoUpgradePayload {
      pub renames: Vec<InstancePathRename>,
      pub already_canonical_count: usize,
  }
  ```
  (Note: `InstancePathRename` in `payload.rs` is a separate CLI-facing mirror from the service's
  `InstancePathRename` in `srs-repository` — follow the mirror pattern used by other payloads.)

- [ ] Run `cargo run --bin generate-schemas` and commit the generated
  `crates/srs-cli/schemas/payload/repo-upgrade.json`.

- [ ] In `crates/srs-cli/src/commands/mod.rs`, add to `RepoCommand`:
  ```rust
  /// Normalise instance file paths in-place to the canonical slug-id8 convention.
  /// Only valid for file-backed repositories.
  Upgrade,
  ```

- [ ] In `crates/srs-cli/src/commands/repo.rs`, add to `dispatch`:
  ```rust
  RepoCommand::Upgrade => cmd_repo_upgrade(ctx),
  ```
  and implement:
  ```rust
  fn cmd_repo_upgrade(ctx: CliContext) -> Result<String> {
      let store = match ctx.store {
          StoreBackend::File => FileStore::new(&ctx.repo),
          _ => return Err(anyhow::anyhow!(
              "repo upgrade only supports file-backed repositories (--store file)"
          )),
      };
      let result = upgrade_repository_paths(&store)?;
      let already_canonical_count = /* computed from manifest index count - renames.len() — */
          // NOTE: the service returns only renames; total count requires loading manifest again
          // OR the service can return total_count in UpgradeRepositoryPathsResult
          result.renames.len(); // placeholder — see note below
      output::serialize("repo upgrade", RepoUpgradePayload {
          already_canonical_count: /* see note */,
          renames: result.renames.into_iter().map(|r| payload::InstancePathRename {
              instance_id: r.instance_id,
              from_path: r.from_path,
              to_path: r.to_path,
          }).collect(),
      })
  }
  ```
  **Note on `already_canonical_count`:** The service result `UpgradeRepositoryPathsResult` must
  also carry `total_instances: usize` (the count of all entries processed) so the CLI can
  compute `already_canonical_count = total_instances - renames.len()` without reloading the
  manifest. Update `UpgradeRepositoryPathsResult` to include `total_instances` in Phase 1.

- [ ] Update Phase 1's `UpgradeRepositoryPathsResult` to include `total_instances: usize`.

#### Acceptance Criteria

- [ ] `srs repo upgrade --repo <path>` returns a valid JSON envelope with
  `ok: true, command: "repo upgrade"`.
- [ ] `payload.renames` is an array of `{instanceId, fromPath, toPath}` objects.
- [ ] `payload.alreadyCanonicalCount` is a non-negative integer.
- [ ] On a repo with no non-canonical paths, `renames: []` and `alreadyCanonicalCount` equals
  the total number of instances.
- [ ] Passing a `.srsj` path or `--store json` returns a clear error.
- [ ] `cargo test --test payload_contracts` passes (golden schema matches struct).

#### Testing

```bash
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -p srs-cli -- -D warnings
```

Smoke test (requires a file-backed repo):
```bash
cargo run --bin srs -- repo upgrade --repo /tmp/test-repo --pretty
# → ok: true, payload.renames: [...], payload.alreadyCanonicalCount: N
```

#### Milestone gate

1. All acceptance criteria checked.
2. `cargo test --test payload_contracts` passes.
3. `cargo test -p srs-cli` passes.
4. `cargo clippy -p srs-cli -- -D warnings` clean.
5. Commit: `feat(cli): srs repo upgrade command (#162)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `srs repo upgrade --repo <canonical-repo>` returns `renames: []`
- [ ] `srs repo upgrade --repo <non-canonical-repo>` returns correct renames and a subsequent
  `srs repo validate` returns 0 diagnostics
- [ ] Running `srs repo upgrade` twice on the same repo returns 0 renames on the second call

## Coordination Rules

- Agents keep to their write scopes.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming.
- **At the end of each phase:** verify acceptance criteria, confirm tests exist and pass, update
  plan checkboxes, then commit. Do not proceed without completing the milestone gate.

## Assumptions

- `InstanceIndexEntry` in `srs-repository` carries `tier`, `title`, and `tags` — all needed to
  construct a `SnapshotInstance` for path computation. (Confirmed in existing code.)
- The `ensure_instance_parent` helper already handles creating intermediate directories for new
  canonical paths. (Confirmed in existing code.)
- No existing repos in production have two instances whose `(typeName, id8)` pairs would collide.
  The collision check is a safety net, not a likely scenario.
