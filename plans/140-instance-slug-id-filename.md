# Plan: Unify instance file naming to slug + id prefix (#140)

## Summary

`srs repo copy` generates UUID-only filenames for instance files when writing to a FileStore, destroying the slug-based names that `note create` produces. The root cause is that `canonical_instance_path` in `repository_portability.rs` always produces `{uuid}.json`. The agreed fix is to converge all three path-generation sites — note create, record create, and the copy fallback — on a single **slug + id prefix** convention (`{slug}-{id8}.json`), matching the convention already used for package files (fields, types, views). Paths inside `.srsj` bundles are internal HashMap keys and are not affected.

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
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | "Snapshot import currently materializes canonical file-backed paths in FileStore, which is acceptable for parity but leaves room for richer path policy later." This plan implements that richer policy. | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service functions own path generation; path strings must not appear in CLI handlers. | accepted |
| [ADR-012](../docs/adr/012-vocabulary-substrate.md) | Tier-3 (TagDefinition) was fully retired; the `canonical_instance_path` tier-3 arm is dead code and will be replaced by a catch-all. | accepted |

No new ADRs required — this is a FileStore implementation detail within ADR-008's explicitly anticipated extension point.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI command output shapes. The instance file paths are implementation details of the FileStore adapter and are not exposed in any payload struct. `cargo test --test payload_contracts` will pass unchanged.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files are added or modified under `srs/docs/schema/2.0/`. `bash scripts/check-schema-sync.sh` will pass unchanged.

---

## Scope

- Add `pub(crate) fn slugify_instance_name(name: &str) -> String` to `crates/srs-repository/src/writer.rs` — the single canonical slug function for instance file naming. Uses the `slugify_title` algorithm: replace every non-alphanumeric char with `-`, split on `-`, filter empty parts, rejoin. Returns `""` on empty input (callers fall back to id-only form).
- Fix `canonical_instance_path` in `crates/srs-repository/src/repository_portability.rs` to accept `&SnapshotInstance` and produce `{slug}-{id8}.json`. Tier-3 arm becomes a catch-all `_ =>`.
- Fix note create path in `crates/srs-repository/src/services.rs` (line 352) to produce `records/notes/{slug}-{id8}.json`. When slug is empty (no title), produce `records/notes/{id8}.json`.
- Fix record create path in `crates/srs-repository/src/record_store.rs` (line 170) to produce `{dir}/{type_slug}-{id8}.json` using `slugify_instance_name(&record.type_name)`.
- Update the two tests in `services.rs` and the tests in `record_store.rs` that assert exact file paths.
- Add five regression tests proving the new convention.

**Out of scope:**
- Migration of existing repository files (the `instanceIndex` is the source of truth; existing repos are unaffected).
- `.srsj` internal path format (opaque bundle keys).
- The existing `portability::slugify` private function — it serves package-file naming and stays as-is.
- The pre-existing `srs-bindings` issue where `create_record` is called with `relative_dir = "records"` (flat path, wrong tier directory). That is a pre-existing bug unrelated to this plan.
- Any changes to `srs-cli` or `srs-core`.
- Tag-definition write paths (tier-3 is fully retired per ADR-012; no live write path exists).

---

## Phases

### Phase 1: Slug + id prefix at all three write sites

**Goal:** All FileStore instance file writes — note create, record create, and the `repo copy` fallback — produce `{slug}-{id8}.json` using a single slug algorithm.

**Agent:** Repository Service Worker

#### Tasks

- [ ] **`crates/srs-repository/src/writer.rs` — add shared slug function**
  - Add `pub(crate) fn slugify_instance_name(name: &str) -> String`:
    - Replace every char that is not alphanumeric with `-`.
    - Split on `-`, filter empty parts, rejoin with `-`.
    - Return `""` if the result is empty (callers treat empty → id-only filename, no leading dash).
  - This is the same algorithm as `services::slugify_title`. Do not add a new copy in any other file.

- [ ] **`crates/srs-repository/src/repository_portability.rs` — fix `canonical_instance_path`**
  - Change function signature from `fn canonical_instance_path(tier: u8, instance_id: &str) -> String` to `fn canonical_instance_path(instance: &SnapshotInstance) -> String`.
  - Derive `id8 = &instance.instance_id[..8]` (UUID4 length is guaranteed; raw slice is acceptable here since `id_prefix()` is private to this module — use `id_prefix(&instance.instance_id)?` if within a `Result`-returning context, otherwise `&instance.instance_id[..8]`).
  - Derive `slug` using the same extraction for all tiers:
    - For tier 0 (notes): `instance.title.as_ref().and_then(|v| v.as_str()).unwrap_or_default()` → `slugify_instance_name(title_str)`.
    - For tier 1/2 (records): `instance.value["typeName"].as_str().unwrap_or_default()` → `slugify_instance_name(type_name_str)`.
    - For any other tier (catch-all, including the retired tier-3): `""` (id-only fallback).
  - Build filename: if `slug.is_empty()` → `"{id8}.json"`, else `"{slug}-{id8}.json"`.
  - Full paths: tier 0 → `records/notes/{filename}`, tier 1 → `records/tier-1/{filename}`, tier 2 → `records/tier-2/{filename}`, `_` → `records/tier-{tier}/{filename}`.
  - Update the one call site in `import_repository_snapshot` (line 231) from `canonical_instance_path(instance.tier, &instance.instance_id)` to `canonical_instance_path(instance)`.
  - Import `crate::writer::slugify_instance_name` at the top of the file.
  - Update the test comment in `repository_snapshot_contains_no_paths`: "The snapshot DTO must not serialize the file-backed `path` field from `InstanceIndexEntry` — paths are an adapter concern (FileStore layout), not part of the logical snapshot."

- [ ] **`crates/srs-repository/src/services.rs` — fix note create path**
  - At line 347–352, change:
    ```rust
    let slug = note.title.as_ref().map(|t| slugify_title(t))
        .unwrap_or_else(|| note.instance_id.clone());
    let relative_path = format!("records/notes/{}.json", slug);
    ```
    to:
    ```rust
    let slug = note.title.as_ref().map(|t| slugify_instance_name(t));
    let relative_path = match &slug {
        Some(s) if !s.is_empty() => format!("records/notes/{}-{}.json", s, &note.instance_id[..8]),
        _ => format!("records/notes/{}.json", &note.instance_id[..8]),
    };
    ```
  - Import `crate::writer::slugify_instance_name`. Remove or deprecate `pub fn slugify_title` only if it has no other callers (check with `grep -rn slugify_title`; if tests call it directly, keep it but have it delegate to `slugify_instance_name`).
  - Update test `create_note_mints_id_and_stores_note` (line 902): change the hardcoded path `"records/notes/my-new-note.json"` to `format!("records/notes/my-new-note-{}.json", &result.note.instance_id[..8])`.

- [ ] **`crates/srs-repository/src/record_store.rs` — fix record create path**
  - At line 166–170, change:
    ```rust
    record.instance_id = new_instance_id();
    store.ensure_instance_dir(relative_dir)?;
    let relative_path = format!("{}/{}.json", relative_dir, record.instance_id);
    ```
    to:
    ```rust
    record.instance_id = new_instance_id();
    store.ensure_instance_dir(relative_dir)?;
    let type_slug = slugify_instance_name(&record.type_name);
    let relative_path = if type_slug.is_empty() {
        format!("{}/{}.json", relative_dir, &record.instance_id[..8])
    } else {
        format!("{}/{}-{}.json", relative_dir, type_slug, &record.instance_id[..8])
    };
    ```
  - Import `crate::writer::slugify_instance_name` at top of file.
  - Update tests that assert the old path pattern. Specific tests to fix:
    - `create_record_in_temp_repo` (around line 1302): change `format!("records/test-items/{}.json", record.instance_id)` → `format!("records/test-items/test-type-{}.json", &record.instance_id[..8])` (adjust type name slug to match the test's type name).
    - Any other tests constructing `format!("{}/{}.json", dir, record.instance_id)` — search with `grep -n "instance_id).json\|instance_id\\.json"` and update each one to the new formula. Use the test's actual `type_name` slug for the prefix.

#### Acceptance Criteria

- [ ] `note create` on a new repo produces `records/notes/{slug}-{id8}.json` (slug from title) or `records/notes/{id8}.json` (no title) and manifests it in `instanceIndex`.
- [ ] `record create` produces `{dir}/{type_slug}-{id8}.json` where `{dir}` is the `--dir` argument (default: `package/records`).
- [ ] `repo copy --from-store file --to-store file` produces `{slug}-{id8}.json` at the target.
- [ ] `repo copy` file→json→file round-trip produces `{slug}-{id8}.json` at the final FileStore target.
- [ ] All three write sites use `slugify_instance_name` from `writer.rs` — no duplicate slug function in `record_store.rs` or `repository_portability.rs` for instance naming.
- [ ] `canonical_instance_path` has no tier-3-specific arm; a `_` catch-all covers unknown/retired tiers.
- [ ] Repos written before this change continue to load and validate correctly (manifest index paths are the source of truth — no migration needed).

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or update:

New tests in `repository_portability.rs`:
- `copy_file_to_file_produces_slug_id_filename` — build MemoryStore with one tier-0 instance titled "My Note" (instance_id `"11111111-1111-4111-8111-111111111111"`), copy to FileStore, assert file exists at `records/notes/my-note-11111111.json`.
- `copy_file_to_file_no_title_produces_id_only_filename` — tier-0 instance with `title: None`, assert `records/notes/11111111.json`.
- `file_json_file_roundtrip_produces_slug_id_filename` — copy MemoryStore→JsonStore→FileStore, assert the final FileStore has `records/notes/my-note-{id8}.json`.
- `copy_tier2_record_uses_type_slug_id_filename` — tier-2 instance with `value["typeName"] = "section"`, assert `records/tier-2/section-{id8}.json`.

New test in `services.rs`:
- `note_create_no_title_produces_id_only_filename` — create note with no title, assert path is `records/notes/{id8}.json`.

Updated tests:
- `create_note_mints_id_and_stores_note` in `services.rs` — use dynamic path formula (see task above).
- `create_record_in_temp_repo` and related tests in `record_store.rs` — use `{type_slug}-{id8}` formula.

#### Milestone gate

1. All five new tests exist and pass.
2. All pre-existing tests in `srs-repository` pass.
3. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Confirm `grep -rn "fn slugify_instance_name" crates/srs-repository/src/` shows exactly one definition (in `writer.rs`).
5. Mark all task and acceptance checkboxes `[x]`.
6. Commit:

```bash
git commit
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `slugify_instance_name` defined exactly once in `writer.rs`; no other copy added
- [ ] `note create` produces `{slug}-{id8}.json` (confirmed by new test)
- [ ] `record create` produces `{type_slug}-{id8}.json` (confirmed by new test)
- [ ] `repo copy` file→file and file→json→file produce `{slug}-{id8}.json` (confirmed by new tests)

## Coordination Rules

- Agent keeps to `crates/srs-repository/` write scope only.
- No business logic changes — only path-generation strings and test fixtures.
- `slugify_instance_name` lives in `writer.rs` and is `pub(crate)`. Do not add a private copy in any other file.
- Return changed file paths and a short behaviour summary when done.

## Assumptions

- `record.instance_id` is always at least 8 characters (UUID4 — guaranteed by `new_instance_id()`).
- `relative_dir` passed to `create_record` has no trailing slash (existing callers confirm this).
- Records always have a non-empty `type_name` (required by the type system); if somehow empty, `slugify_instance_name` returns `""` and the id-only fallback applies.
- MemoryStore test double does not enforce filename conventions and does not need updating for this fix.
- Tier-3 (tag-definition) is fully retired (ADR-012); the catch-all arm in `canonical_instance_path` covers it without active logic.
- `srs-bindings` pre-existing flat `relative_dir = "records"` issue is not addressed here.
