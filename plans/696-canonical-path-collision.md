# Plan: Fix canonical-path collision + migration-detection regression (#696)

## Summary

PR #694 (issue #684, ADR-038/039 — the VFS tree became the primary operational model) introduced
two P1 regressions between release `build.224` and `build.226`, both rooted in one change: the
`.srsj` browser load path now runs `materialize_tree`, which **re-canonicalizes every instance
path** through the snapshot round-trip. This (1) crashes on repositories whose deterministic
instance UUIDs share their first 8 hex characters — the slug-`id8` canonical filename collapses
two distinct records onto one path (`records/tier-2/decision-00000000.json`) — and (2) silently
normalizes instance paths at load time so the `repo-upgrade` ("Normalise instance file paths")
migration can no longer be detected as `needed`. Both contradict ADR-038's own stated goal
("the operational tree keeps real paths"; "the snapshot pipeline re-canonicalizes … so
re-serialization rewrites untouched files" is named as the problem to eliminate). This plan
realigns `materialize_tree` with that goal and makes the canonical-filename convention
collision-safe so the snapshot import path (`repo copy`, `export_srsj`, `repo upgrade`) never
crashes on deterministic-UUID repositories.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Repository Worker | Claude (this session) |
| Architecture Reviewer | subagent (Stage 3 / Stage 7), `model: sonnet` |
| Plan Reviewer | subagent (Stage 3), `model: haiku` |
| Verification | subagent (Stage 7), `model: haiku` |

See [agents.md](agents.md) for role definitions. All code changes are confined to the
Repository Worker write scope (`crates/srs-repository/src/**`, `crates/srs-repository/tests/**`).

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-038](../docs/adr/038-vfs-tree-primary-model.md) | The in-memory VFS tree is the primary operational model; the operational tree keeps **real paths**. This plan corrects `materialize_tree` (ADR-038 §4) to honor that intent instead of re-canonicalizing. | accepted (amended by ADR-040) |
| [ADR-039](../docs/adr/039-srs-archive-pure-tree-zip.md) | `.srs` archives are a faithful path→bytes tree; `tree_entries` (archive.rs) is the authoritative faithful store→tree enumeration this plan reuses. | accepted |
| [ADR-008](../docs/adr/008-repository-lifecycle-and-portability.md) | Repository portability / snapshot import contract. The collision-safe filename rule refines the slug-`id8` convention (introduced in #392) used by `import_repository_snapshot`. | accepted |
| [ADR-040](../docs/adr/040-materialize-preserves-paths-collision-safe-filenames.md) | **(new)** `materialize_tree` reproduces the source's real file tree faithfully; canonical instance filenames fall back from slug-`id8` to slug-`fullid` when — and only when — the short form collides within the repository. | proposed → accepted on ship |

**No spec RFC required.** The slug-`id8` filename convention and `materialize_tree` are
implementation details of the Rust reference implementation (ADR-level, not spec-level). No SRS
field, type, relation, extension, validation rule, or `srs/docs/schema/2.0/` entity schema
changes. The `srs repo copy` CLI contract is unchanged — its crash-on-collision is a defect, and
the fix restores the documented behavior without altering the payload.

---

## Contracts

### CLI output contract (ADR-011)

**No new/changed commands.** No `crates/srs-cli/src/payload.rs` struct changes. Golden payload
schemas stay as-is. (`repo copy`, `repo upgrade`, and the migration-registry payloads keep their
shapes; only their internal behavior on collision-prone repositories is fixed.)

### Entity schema sync (check-schema-sync.sh)

**No.** No files under `srs/docs/schema/2.0/` or the mirrors are touched.

---

## Scope

In scope (all in `crates/srs-repository`):

- **`tree_session.rs::materialize_tree`** — reproduce the source's real file tree faithfully
  (reuse `archive::tree_entries` → `open_tree`) instead of the canonicalizing snapshot
  round-trip. Preserves real instance paths on `.srsj` load.
- **`archive.rs::tree_entries`** — widen visibility to `pub(crate)` so `tree_session` can reuse
  the single authoritative faithful-enumeration implementation (DRY; no second copy).
- **`repository_portability.rs`** — make the canonical instance-path derivation collision-safe:
  when the slug-`id8` short form collides for ≥2 instances within one repository, those
  instances use the slug-`fullid` form (always unique). Applied at both snapshot-consuming loop
  sites: `import_repository_snapshot` (`do_import`) and `collect_planned_renames`.
- Regression + unit tests for both behaviors.

**Out of scope (deferred, file as follow-ups under Epic 10 / muDemocracy.org#101):**

- Changing `copy_repository` / `import_repository_snapshot` to *preserve* source paths (rather
  than canonicalize collision-safely). Copy deliberately canonicalizes to the target convention;
  only the crash is a bug. A future "path-faithful copy" mode, if wanted, is separate.
- Emitting pretty-printed (vs compact) JSON for instance files materialized from a `.srsj`
  codec. `tree_entries` uses `load_text_file` (compact) — matches `archive_pack`; a formatting
  pass is a separate cosmetic improvement.
- srs-web consuming `build.227+` and re-enabling the disabled e2e specs (srs-web repo work).

---

## Phases

### Phase 1: Collision-safe canonical instance filenames

**Goal:** `import_repository_snapshot` and `collect_planned_renames` never crash on a repository
whose instances share an 8-hex-char id prefix + tier + slug; colliding instances deterministically
get the full-UUID filename form.

**Agent:** Repository Worker

#### Tasks

- [ ] In `repository_portability.rs`, factor the filename construction out of
  `canonical_instance_path` into a helper that yields the `(dir, slug, id)` components, plus a
  `canonical_filename(slug, id) -> String` that builds `"{slug}-{id}.json"` (or `"{id}.json"`
  when slug is empty). Keep `canonical_instance_path` returning the existing short (`id[..8]`)
  form for single-instance callers.
- [ ] Add `canonical_instance_paths(instances: &[&SnapshotInstance], store) -> Result<Vec<String>>`:
  compute the short form for every instance; any short path used by ≥2 instances → those
  instances use the full-`id` form (`{dir}/{slug}-{fullid}.json`). Order-independent (a function
  of the whole set, not iteration order).
- [ ] Rewrite `do_import`'s instance loop to derive all paths up front via
  `canonical_instance_paths`, then write each. Keep the `used_paths` guard — after
  disambiguation it can only fire on a genuine **duplicate instance id**, which stays an error.
- [ ] Rewrite `collect_planned_renames` to derive canonical paths via `canonical_instance_paths`;
  keep its `canonical_paths` guard as the same duplicate-id backstop.

#### Acceptance Criteria

- [ ] Importing a snapshot with two tier-2 instances whose ids differ only past the 8th hex char
  and share `typeName` writes **two** files (both full-`id` form) with no error, both in
  `instanceIndex`.
- [ ] A repository with no id8 collisions is byte-for-byte unchanged (all short form).
- [ ] `collect_planned_renames` / `check_path_upgrade_needed` on a colliding repo returns without
  error and reports the collided instances as needing rename to their full-`id` form.
- [ ] A genuine duplicate instance id still errors.

#### Testing

```bash
cargo test -p srs-repository canonical
cargo test -p srs-repository import
```

Tests to write:

- `canonical_instance_paths_disambiguates_id8_collision` — two instances sharing dir+slug+id8 →
  both full-id, distinct; a non-colliding third stays short.
- `import_snapshot_with_id8_collision_writes_both` — `import_repository_snapshot` of a 2-instance
  colliding snapshot succeeds; both files exist; index has both.
- `import_snapshot_rejects_true_duplicate_instance_id` — two instances with the *same* id → error.

#### Milestone gate

`cargo test -p srs-repository` green; `cargo clippy -p srs-repository -- -D warnings` clean;
mark checkboxes; commit `fix(repository): collision-safe canonical instance filenames (#696)`.

### Phase 2: `materialize_tree` preserves real paths

**Goal:** `.srsj` browser load reproduces the source's real file tree — colliding-UUID repos load,
and load does not pre-normalize paths (so `repo-upgrade` stays detectable).

**Agent:** Repository Worker

#### Tasks

- [ ] Change `archive.rs::tree_entries` from `fn` to `pub(crate) fn` (single authoritative
  faithful store→tree enumeration).
- [ ] Rewrite `tree_session.rs::materialize_tree` to `open_tree(crate::archive::tree_entries(source)?)`.
  Update the doc comment: it now faithfully reproduces the source's real paths (ADR-038/039), not
  a canonicalizing snapshot round-trip. Cross-reference #696 and ADR-040.
- [ ] Remove the now-unused `export_repository_snapshot_with_options` / `import_repository_snapshot`
  imports from `tree_session.rs` if they become dead.

#### Acceptance Criteria

- [ ] `materialize_tree` on a `JsonStore` whose instance paths are the full-UUID form preserves
  those exact paths (asserted against `manifest.instance_index[].path`).
- [ ] `materialize_tree` on a `JsonStore` with two colliding-id8 tier-2 instances succeeds (no
  collision — real paths are unique).
- [ ] `check_path_upgrade_needed` on the materialized store returns `true` when the source paths
  are non-canonical (regression guard for the `repo-upgrade`-not-detected bug).
- [ ] Existing `materialize_tree` tests (`tree_session.rs` inventory/validation parity;
  `tree_roundtrip.rs` srsj round-trip parity) still pass.

#### Testing

```bash
cargo test -p srs-repository materialize
cargo test -p srs-repository tree
cargo test -p srs-bindings tree_roundtrip
```

Tests to write:

- `materialize_tree_preserves_source_instance_paths` — non-canonical source paths survive.
- `materialize_tree_loads_id8_colliding_repository` — the gallery-shaped collision loads.
- `materialize_tree_keeps_repo_upgrade_detectable` — `check_path_upgrade_needed == true` after
  materializing a non-canonical source.

#### Milestone gate

`cargo test -p srs-repository` + `cargo test -p srs-bindings` green;
`cargo clippy -p srs-repository -- -D warnings` clean; mark checkboxes; commit
`fix(repository): materialize_tree preserves real paths (#696)`.

### Phase 3: End-to-end regression coverage + fixture

**Goal:** A fixture mirroring the reported gallery/sample collision proves both regressions are
closed at the level the issue reproduces them (`.srsj` load + `repo copy`).

**Agent:** Repository Worker

#### Tasks

- [ ] Add a compact in-crate fixture (or programmatic builder) with two tier-2 records whose ids
  are `...5801` / `...5802` (same `typeName`), stored at full-UUID paths — the gallery shape.
- [ ] Test: `load_from_srsj` → `materialize_tree` succeeds and lists both records.
- [ ] Test: `copy_repository` (JsonStore source → FileStore/MemVfs target) of the colliding repo
  succeeds and writes both instance files (this is the issue's `srs repo copy` reproduction).

#### Acceptance Criteria

- [ ] Both regression tests pass.
- [ ] `cargo test` (whole workspace) green.

#### Testing

```bash
cargo test -p srs-repository
cargo test
```

#### Milestone gate

Whole-workspace `cargo test` + `cargo clippy -- -D warnings` green; mark checkboxes; commit
`test(repository): gallery-shape collision regression coverage (#696)`.

---

## Final Acceptance

```bash
cargo test
cargo clippy -- -D warnings
cargo test --test payload_contracts   # unchanged — sanity only
```

Dogfooding (Stage 7.6): build `srs` from the branch and run
`srs repo copy --from <colliding>.srsj --from-store json --to /tmp/x --to-store file` — the exact
issue reproduction — plus a migration-status check confirming `repo-upgrade` reports `needed` on a
non-canonical source.
