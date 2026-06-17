# Plan: repo copy — record load diagnostics and path collision detection (#139)

## Summary

`srs repo copy` can silently lose records when the copy operation encounters errors or when two instances map to the same canonical path during import. There are also two code paths that suppress errors without producing diagnostics: `load_instance_json` failures in `export_repository_snapshot` provide no context about which record failed (instance_id or source path), and malformed `packageRefs` JSON in a manifest is silently ignored, causing sub-packages to be dropped with no error. Additionally, when two instances generate the same canonical path during import, the second silently overwrites the first in the data store. This plan adds three hardening measures: (1) an `InstanceLoad` error variant that wraps any record-read failure with the instance's id and source path; (2) canonical path collision detection during import that errors loudly instead of overwriting; and (3) converting the `packageRefs` silent suppression into a propagated error. It also documents the path-rewriting evaluation requested in issue #139 comment #3: `repo copy` path normalisation is intentional per ADR-008, already implemented via issue #140; a separate `repo upgrade` command is deferred.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | Path normalisation during `repo copy` is intentional; `repo upgrade` as a separate command is deferred. | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All validation and error context belongs in the service layer, not the CLI handler. | accepted |

No new ADRs required — these are bug fixes within the established ADR-008 and ADR-010 contracts. The path-rewriting evaluation concludes that current behaviour (normalise on copy) is correct per ADR-008 and issue #140. A future `repo upgrade` command (normalise in-place) is a separate enhancement if consumers ever need it; it would require its own plan and potentially a spec addition.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. `RepoCopyPayload` is unchanged. `cargo test --test payload_contracts` passes without modification.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files changed. `bash scripts/check-schema-sync.sh` passes unchanged.

---

## Scope

- Add `RepositoryError::InstanceLoad` variant to `crates/srs-repository/src/error.rs`.
- Wrap `load_instance_json` in `export_repository_snapshot` with the new variant so failures identify the instance_id and source path.
- Add canonical path collision detection to `import_repository_snapshot` using a `HashSet<String>` and return `InvalidSnapshotData` error on collision.
- Convert the `packageRefs` silent `.ok()` suppression in `export_repository_snapshot` to an explicit `map_err`-guarded parse that propagates `ManifestParse` on malformed input.
- Add tests for each new error path.

**Out of scope:**
- `repo upgrade` (in-place path normalisation) — deferred; needs separate plan.
- Transactional JsonStore writes (the intermediate-flush behaviour during import is a separate concern not directly causing the user-reported loss; deferred to a future JsonStore hardening plan).
- Any change to `RepoCopyPayload` or CLI output format.
- Issue #140 (slug+id8 filename convention) — already implemented.

---

## Phases

### Phase 1: `InstanceLoad` error variant and export diagnostics

**Goal:** `export_repository_snapshot` produces an error that names the failing instance by id and source path whenever a record cannot be read, rather than a generic IO error with no context.

**Agent:** Repository Service Worker

#### Tasks

- [ ] **`crates/srs-repository/src/error.rs` — add `InstanceLoad` variant**
  - Add after the `Serialize` variant (line 85), following the `PathBuf` + boxed-source pattern used by `Io` and `NoteLoad`:
    ```rust
    #[error("failed to load instance '{instance_id}' from path {path:?}: {source}")]
    InstanceLoad {
        instance_id: String,
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    ```
  - Add the `PartialEq` arm immediately after the `Io` arm (around line 344), ignoring `source` (consistent with how `PackageRefConflict` and other variants handle non-`PartialEq` inner types):
    ```rust
    (
        RepositoryError::InstanceLoad { instance_id: a, path: pa, .. },
        RepositoryError::InstanceLoad { instance_id: b, path: pb, .. },
    ) => a == b && pa == pb,
    ```

- [ ] **`crates/srs-repository/src/repository_portability.rs` — wrap load in export loop**
  - In `export_repository_snapshot` at line 104, change:
    ```rust
    let value = source.load_instance_json(entry.path())?;
    ```
    to:
    ```rust
    let value = source.load_instance_json(entry.path()).map_err(|e| {
        RepositoryError::InstanceLoad {
            instance_id: entry.instance_id.clone(),
            path: std::path::PathBuf::from(entry.path()),
            source: Box::new(e) as Box<dyn std::error::Error + Send + Sync>,
        }
    })?;
    ```

#### Acceptance Criteria

- [ ] `export_repository_snapshot` on a source where one record file is missing returns `RepositoryError::InstanceLoad` containing the missing record's instance_id and path.
- [ ] Error message string includes both the instance_id and path (confirmed by asserting on the `Display` output).
- [ ] Existing export tests pass unchanged.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write in `repository_portability.rs` test module:

- `export_fails_with_instance_load_error_when_record_missing` — creates a MemoryStore, initialises repository, manually pushes an `InstanceIndexEntry` with a path that has no corresponding data entry, calls `export_repository_snapshot`, asserts the error is `RepositoryError::InstanceLoad { instance_id, path, .. }` with the expected id and path values.

#### Milestone gate

1. New test exists and passes.
2. Existing tests pass.
3. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
4. Mark completed checkboxes `[x]`.
5. Commit with message referencing issue (#139).

---

### Phase 2: Canonical path collision detection in import

**Goal:** `import_repository_snapshot` detects when two instances map to the same canonical path and returns `InvalidSnapshotData` instead of silently overwriting the first instance's data.

**Agent:** Repository Service Worker

#### Tasks

- [ ] **`crates/srs-repository/src/repository_portability.rs` — add collision map to import loop**
  - In `import_repository_snapshot`, at line 240 the existing code already does `manifest.instance_index = Vec::new();`. Insert a `used_paths` declaration on the line BEFORE that existing line (do not duplicate or move the `Vec::new()` assignment):
    ```rust
    // Insert this line immediately before the existing `manifest.instance_index = Vec::new();`
    let mut used_paths: std::collections::HashMap<String, String> =
        std::collections::HashMap::with_capacity(snapshot.instances.len());
    ```
  - Inside the for loop, immediately after `let rel_path = canonical_instance_path(instance);`, add the collision check:
    ```rust
    if let Some(first_id) = used_paths.get(&rel_path) {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!(
                "canonical path collision at '{}': instance '{}' and '{}' both map to the same path",
                rel_path, first_id, instance.instance_id
            ),
        });
    }
    used_paths.insert(rel_path.clone(), instance.instance_id.clone());
    ```
  - The `manifest.instance_index = Vec::new();` line at line 240 stays exactly where it is — no movement.

#### Acceptance Criteria

- [ ] `import_repository_snapshot` with two instances that generate identical canonical paths returns `RepositoryError::InvalidSnapshotData` with a message identifying the path and the colliding instance_id.
- [ ] Normal imports with unique paths are unaffected.
- [ ] Existing round-trip tests pass.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write in `repository_portability.rs` test module:

- `import_fails_on_canonical_path_collision` — builds a snapshot with two tier-0 instances that have the same title AND the same first 8 UUID characters, calls `import_repository_snapshot` into a MemoryStore target, asserts the error is `RepositoryError::InvalidSnapshotData` with a message containing both instance ids and the collision path.

  Both instances share `id8 = "aaaaaaaa"` and slug `"same-title"` → path `records/notes/same-title-aaaaaaaa.json`. The test verifies the error message contains the path and both instance ids:
  ```rust
  // instance A: id[..8] == "aaaaaaaa", title "same title"
  // instance B: id[..8] == "aaaaaaaa", title "same title"
  // canonical_instance_path produces "records/notes/same-title-aaaaaaaa.json" for both
  let Err(RepositoryError::InvalidSnapshotData { message }) = result else { panic!(...) };
  assert!(message.contains("records/notes/same-title-aaaaaaaa.json"));
  assert!(message.contains("aaaaaaaa-0000-4000-8000-000000000001"));
  assert!(message.contains("aaaaaaaa-0000-4000-8000-000000000002"));
  ```

#### Milestone gate

1. New test exists and passes.
2. All existing tests pass.
3. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
4. Mark completed checkboxes `[x]`.
5. Commit with message referencing issue (#139).

---

### Phase 3: packageRefs silent suppression → explicit error

**Goal:** `export_repository_snapshot` no longer silently drops sub-packages when `packageRefs` in the manifest is present but malformed — it returns a `ManifestParse` error instead.

**Agent:** Repository Service Worker

#### Tasks

- [ ] **`crates/srs-repository/src/repository_portability.rs` — convert `.ok()` to error**
  - At lines 131–136, change:
    ```rust
    let refs: Vec<RawPackageRef> = manifest
        .extra
        .get("packageRefs")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    ```
    to:
    ```rust
    let refs: Vec<RawPackageRef> = match manifest.extra.get("packageRefs") {
        None => Vec::new(),
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| {
            RepositoryError::InvalidSnapshotData {
                message: format!("malformed packageRefs in manifest: {e}"),
            }
        })?,
    };
    ```
  - Using `InvalidSnapshotData` (not `ManifestParse`) avoids embedding a `"manifest.json"` path literal in service logic, which would violate the storage boundary rule (CLAUDE.md: path strings must not appear in service logic).

#### Acceptance Criteria

- [ ] `export_repository_snapshot` returns `InvalidSnapshotData` error when manifest `packageRefs` value is present but not a valid `Vec<{ mode, path }>`.
- [ ] `export_repository_snapshot` succeeds when `packageRefs` is absent.
- [ ] `export_repository_snapshot` succeeds when `packageRefs` is a valid array.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write in `repository_portability.rs` test module:

- `export_fails_on_malformed_package_refs` — builds a MemoryStore, initialises, loads the manifest, sets `manifest.extra.insert("packageRefs", serde_json::json!("not-an-array"))`, saves the manifest, calls `export_repository_snapshot`, asserts the error is `RepositoryError::InvalidSnapshotData { .. }` with a message containing "malformed packageRefs".

#### Milestone gate

1. New test exists and passes.
2. All existing tests pass.
3. Run:
```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```
4. Mark completed checkboxes `[x]`.
5. Commit with message referencing issue (#139).

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] Three new tests pass: `export_fails_with_instance_load_error_when_record_missing`, `import_fails_on_canonical_path_collision`, `export_fails_on_malformed_package_refs`
- [ ] `srs repo copy` on a repo with a missing record file now fails with an error that names the instance_id (not a generic IO error)
- [ ] `srs repo copy` on a repo where two instances share a canonical path fails with `InvalidSnapshotData` instead of silently overwriting

## Coordination Rules

- Agent keeps to `crates/srs-repository/src/error.rs` and `crates/srs-repository/src/repository_portability.rs` write scope only.
- No changes to `srs-cli` or payload structs.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `MemoryStore` can be used to inject a missing-record scenario by calling `save_manifest` with an entry pointing to a path that was never written to `data`. The `load_instance_json` implementation returns `RepositoryError::Io { .. }` for a missing key, which the new `InstanceLoad` variant wraps.
- UUID first-8-characters collisions are possible in tests by using hand-crafted UUIDs that share the first segment (e.g., `aaaaaaaa-...`).
- No changes to the `PartialEq` implementation are needed for `InvalidSnapshotData` — it already compares by `message` string.
