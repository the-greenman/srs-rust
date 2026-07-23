# Plan: Logical-id + typed-entity instance persistence (ADR-041 G3–G5, records + notes)

> First increment of the ADR-041 G3–G5 store-shape migration ([srs-rust#704](https://github.com/the-greenman/srs-rust/issues/704),
> the epic). Migrates the **instances** entity family (records + notes) to typed, logical-id-keyed
> `RepositoryStore` methods following the container precedent. The remaining entity families
> become child stories under #704. Tracking story: **srs-rust#724**.

## Summary

The `RepositoryStore` trait persists instances through `(relative_path, serde_json::Value)`
methods, and every service enumerator walks `manifest.instance_index` by `InstanceIndexEntry.path`.
ADR-041 G3–G5 require the container-shaped alternative: typed entities keyed by logical id, plus a
store-answerable enumeration/query so a future SQL backend is *table rows*, not a path-keyed blob
store. This plan adds `save_record`/`save_note`/`load_record_by_id`/`load_note_by_id`/`delete_instance`
plus `find_instance`/`list_instances(InstanceQuery)` to the trait (implemented on FileStore,
JsonStore, MemoryStore), routes the instance CRUD funnel and enumerators through them (records to the default
`records/tier-2` tier; the `--dir` override and Extension records keep the legacy write), and
makes `InstanceIndexEntry.path` contract-opaque for the
migrated paths — all behind a store-matrix parity suite including an ADR-007 fault-injection test.
It is the highest-value slice (the only family with the index-walk problem G5 names) and sets the
template for the other five entity families.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (pipeline) |
| Repository Worker | Claude (pipeline) — `crates/srs-repository/src/**`, `crates/srs-repository/tests/**` |
| Architecture Reviewer | Stage 3 / Stage 7 spawn (`model: sonnet`) |
| Plan Reviewer | Stage 3 spawn (`model: haiku`) |
| Verification | Stage 7 spawn (`model: haiku`) |

See [agents.md](agents.md). All work is inside `srs-repository`; no new role is required (the
Repository Worker scope covers the trait, its three impls, services, and tests). No `srs-cli`,
`srs-bindings`, or `srs-mcp` changes — those crates call the same service functions, whose
signatures are unchanged, so they recompile untouched.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-042](../docs/adr/042-logical-id-instance-persistence.md) | Typed, logical-id instance persistence surface + index-answerable query; reuse `Record`/`Note`/`LoadedInstance`; dispatch stays in service; `save_record`/`save_note` mirror `save_container`'s existing-path/new-path branches; `InstanceIndexEntry.path` contract-opaque for migrated paths; Value methods + path helpers retained transitionally, deprecation deferred to filed follow-ups | proposed → accepted on merge |
| [ADR-041](../docs/adr/041-storage-backend-guardrails.md) | G3 (logical-id typed methods), G4 (typed currency), G5 (store-answerable enumeration/query) — this plan implements them for instances | accepted |
| [ADR-007](../docs/adr/007-file-index-io-ordering.md) | File-before-index on create, index-before-entity on delete — `save_record`/`save_note`/`delete_instance` obey it (verified by fault injection), all three impls incl. MemoryStore | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Typed in / typed out — new methods take `&Record`/`&Note`, return typed values, no `serde_json::Value` in the new surface | accepted |
| [ADR-021](../docs/adr/021-jsonstore-batch-write-mode.md) | Batch write mode (G6) — the new JsonStore `save_record`/`save_note`/`delete_instance` satisfy the batch seam by reusing the shared `flush()` (honours the `batching` flag); no new machinery | accepted |
| [ADR-024](../docs/adr/024-best-effort-rollback-multi-write-services.md) | Two-write operations (entity + index) — `save_record` mirrors `save_container`'s ordering/rollback posture | accepted |
| [ADR-040](../docs/adr/040-materialize-preserves-paths-collision-safe-filenames.md) | Collision-safe canonical filenames — store-owned filename derivation reuses the `{slug}-{id8}` scheme (cited for scheme reuse, not for implementing ADR-040's full-UUID disambiguation fallback) | accepted |

**Interop/positioning consult (ADR-041 requires it for storage-seam work):** the register
`../srs/docs/research/alignment-opportunities.md` has **no entry** governing an internal
`RepositoryStore` trait-shape refactor. The interop-relevant storage decisions — export/import as
the networked boundary, and a Layer-2 SQL accelerator as the first SQL build — are already settled
inside **ADR-041 G7/G8** (which references the portability engine). This plan neither adds an
export/import format nor a binding/agent-facing surface, so no register entry applies and none is
contradicted.

---

## Contracts

### CLI output contract (ADR-011)

**No new/changed commands.** This plan changes only `srs-repository` internals (the store trait and
service internals). No `crates/srs-cli/src/payload.rs` struct changes → no schema regeneration.
`cargo test --test payload_contracts` must still pass (unchanged).

### Entity schema sync (check-schema-sync.sh)

**No changes** under `srs/docs/schema/2.0/`. The on-disk `instance_index`/`InstanceIndexEntry`
serialization is unchanged. No mirror sync needed.

---

## Scope

In scope (all in `crates/srs-repository/`):

- **Trait surface** (`src/store.rs`): add `save_record`, `save_note`, `load_record_by_id`,
  `load_note_by_id`, `delete_instance`, `find_instance`, `list_instances`; add `InstanceRef` and
  `InstanceQuery` value types in `src/index.rs`.
- **Three impls**: FileStore (`src/store.rs`), JsonStore (`src/json_store.rs`), MemoryStore
  (`src/store.rs` `mod memory`) — mirroring the container implementations, including FileStore
  instance-index helpers analogous to `file_store_{load,find,upsert,remove}_container_index` (these
  return the **typed** `InstanceIndexEntry`/`InstanceRef`, not `(String,String,String)` tuples).
- **ADR-007 fault injection**: add `FailPoint::SaveInstanceIndex` and a test mirroring
  `save_container_file_first_failed_index_leaves_orphaned_data_safe`.
- **CRUD funnel migration**: `create_record_at_dir` (Tier-2 branch → `save_record`; custom/Extension
  dir keeps legacy write), `update_record`, `add_record_tag`, `remove_record_tag`,
  `append_source_ref`, `create_record_successor` (`record_store.rs`); `write_note` (`writer.rs`);
  `note_service::{create, update_note, delete_note, add_note_tag, remove_note_tag}` (`services.rs`).
- **Enumerator migration** (stop touching `entry.path()`): `list_all_records`,
  `list_records_by_type`, `get_record_by_id`, `get_instance_by_id`, `list_records_filtered`,
  `list_record_summaries` (via `list_records_filtered`), `list_record_tags`; note counterparts
  `list_notes`, `get_note_by_id`.
- **`dir_override` retained**: it backs the `srs record create --dir` CLI flag + MCP tool — NOT
  dead. `create_record_at_dir` branches (Tier-2 → typed `save_record`; other dirs → legacy write).
- **Store-matrix parity tests** for the new methods (memory → file, plus JsonStore leg).
- ADR-042 and doc updates (Stage 7.5).

**Transitional (kept, not deprecated, this increment):**
- `record_store::load_record(store, path)` and `loader::load_note(store, path)` survive as
  path-based helpers so the deferred readers below keep compiling. New canonical readers are
  `load_record_by_id` / `load_note_by_id`.
- `load_instance_json` / `save_instance_json` / `list_instance_files` remain (they double as the
  generic JSON shim for ~140 non-instance callers). **Not** `#[deprecated]` this increment.

**Out of scope** (each filed as a child issue under #704 in Stage 3.4):

- **All path-based instance readers/callers outside the migrated `record_store.rs` + `note_service`
  funnel** (#725, expanded), namely:
  - the **revision-coupled functions** in `record_store.rs` — `delete_record`,
    `transition_record_lifecycle`, `list_record_revisions`, `get_record_revision` — which retain a
    raw `instance_index` → `path` lookup solely to feed the path-keyed `revision_service` (their
    record *write*, where present, still routes through `save_record`);
  - every other file that loads an instance by `entry.path()`: `validation.rs`,
    `migrate_identity_service.rs`, `analysis.rs`, `attachment_policy_service.rs`,
    `attachment_service.rs`, `vocabulary_service.rs`, `container_view_service.rs`,
    `view_service.rs` (`document_views_for_container`), `context_query_service.rs`,
    `migration_registry_service.rs`, `container_service.rs`,
    `writer.rs::build_instance_semantic_types`;
  - and, once those are migrated, removal of the transitional `load_record(path)`/`load_note(path)`
    helpers. (Archive/export packers legitimately need paths to copy files — they stay on the
    adapter path, not part of #725.)
- The generic definition/blob JSON seam that replaces the ~140 `load/save_instance_json` shim
  callers, and the subsequent `#[deprecated]` on the Value/path instance methods.
- Extension-tier records (`RecordTier::Extension` → `package/records`) — stay on the transitional
  method this increment.
- Relations entity migration (`load/save_relations_json` → typed).
- Fields entity migration.
- Types entity migration.
- Views (L1 `View` + L2 `DocumentView`) entity migration.
- Smaller definition entities: themes, blueprints, vocabularies, lifecycles, relation-type
  definitions.

---

## Phases

### Phase 1: Trait surface + value types + all three impls (with fault injection)

**Goal:** `RepositoryStore` exposes the typed instance methods; FileStore, JsonStore, and
MemoryStore implement them following the container precedent; parity + ADR-007 fault-injection
tests pass. No service caller uses them yet.

**Agent:** Repository Worker

#### Tasks

- [x] In `src/index.rs`, add `InstanceRef { instance_id, tier, title: Option<String>, tags: Vec<String> }`
      and `InstanceQuery { tier: Option<u8>, tag: Option<String> }` (derive `Debug, Clone, Default`;
      internal to the adapter layer — **no `Serialize`/`Deserialize`** needed). Add
      `InstanceQuery::matches(&InstanceIndexEntry) -> bool` with a doc comment stating the
      combinator: `tier` is exact-match; `tag` is a single **contains** predicate (present in the
      entry's tags); both `None` ⇒ match all.
- [x] In `src/store.rs` trait, add the seven methods under a new `// --- Instances (logical-id +
      typed; ADR-042) ---` block, doc comments citing ADR-007 ordering, the existing-path/new-path
      branch rule (incl. denormalized-field refresh), and G3/G5. Doc-comment `delete_instance(id)`
      to distinguish it from the pre-existing generic-shim `delete_instance_file(path)`. Annotate
      the existing Value/path instance block as the transitional generic-JSON shim (doc note
      pointing at the generic-seam follow-up issue #726).
- [x] FileStore impl (`src/store.rs`): `file_store_find_instance_path(store, id) -> Option<String>`
      and typed `file_store_{load,upsert,remove}_instance_index` helpers.
      - `save_record`/`save_note`: **two branches, mirroring `save_container` (`store.rs:1409-1440`)** —
        if the id is already indexed, overwrite the entity **at its existing path** (path + `tier`
        preserved, no rename) and **refresh the index entry's denormalized `tags`/`title`** from the
        entity (so tag/update writers stay correct — see Phase 2); else derive
        `{tier_dir}/{slug}-{id8}.json`, write **entity before index** (ADR-007), upsert the index
        entry with the type-derived tier (`Note`=0, `Record`=2). Preserve `$schema` injection
        currently in `write_record`.
      - `load_record_by_id`/`load_note_by_id`: resolve id→path via `manifest.instance_index`,
        read, deserialize (`RecordLoad`/note validation errors as today).
      - `delete_instance`: remove the index entry **then** the file (ignore missing file);
        `…NotFound` for an unknown id.
      - `find_instance`/`list_instances`: answer from `manifest.instance_index` (no body loads).
- [x] JsonStore impl (`src/json_store.rs`): key by `records/tier-{n}/…` in the data map, maintain
      the index in the manifest exactly as the container methods do, `flush()` after writes
      (inherits ADR-021 batch safety). Same two-branch save shape.
- [x] MemoryStore impl (`src/store.rs` `mod memory`): mirror **FileStore** against the in-memory map
      (do **not** copy `delete_container`'s data-first delete inversion — `delete_instance` is
      index-first). Add `FailPoint::SaveInstanceIndex` and honour it between data write and index
      update.

#### Acceptance Criteria

- [x] All three impls compile; `save_record`/`save_note` round-trip an entity by id through each store.
- [x] `save_record` on an **existing** id overwrites at the existing path with the index unchanged;
      on a **new** id writes file-before-index. With `FailPoint::SaveInstanceIndex` armed, the data
      file exists and the index entry is absent (orphan-safe, ADR-007).
- [x] `delete_instance` removes the index entry before the file and returns `…NotFound` for an
      unknown id.
- [x] `list_instances(InstanceQuery::default())` returns every instance; a `tier`/`tag` filter
      narrows correctly, answered without loading entity bodies.
- [x] `find_instance(id)` returns the tier for a known id and `None` for an unknown id.

#### Testing

```bash
cargo test -p srs-repository store::
cargo test -p srs-repository json_store::
```

Specific tests to write:

- `save_record_roundtrip_by_id_across_stores` — memory → file (via `repository_portability::copy_repository`) → reload by id equal.
- `save_note_roundtrip_by_id_across_stores` — same for notes (tier 0).
- `save_record_existing_id_preserves_path` — save a record, then `save_record` the same id with a changed `type_name`/slug; assert the on-disk path (from the index) is unchanged.
- `save_record_file_first_failed_index_leaves_orphaned_data_safe` — mirrors the container test using `FailPoint::SaveInstanceIndex`.
- `delete_instance_index_first_and_not_found` — deletion ordering + unknown-id error.
- `list_instances_filters_by_tier_and_tag_from_index` — index-answerable enumeration.
- `find_instance_returns_tier_or_none`.
- `json_store_instance_operations_are_keyed_by_id` + `json_store_instance_persists_across_reopen`.

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Then mark checkboxes and commit: `feat(store): typed logical-id instance persistence methods (#724)`.

---

### Phase 2: Migrate the full `record_store.rs` + `note_service` instance funnel

**Goal:** Every instance-loading function in `record_store.rs` and the `services.rs` `note_service`
— readers, enumerators, and writers — reads/writes/deletes/enumerates instances through the Phase 1
methods (no `entry.path()`), **except** the revision-coupled trio (carved out, allow-listed). Full
suite green.

**Agent:** Repository Worker

#### Tasks (migrate — the complete enumerated set)

- [x] `record_store.rs` readers → `list_instances`/`find_instance` + `load_record_by_id`:
      `list_all_records`, `list_records_by_type`, `get_record_by_id`, `get_instance_by_id`
      (uses `find_instance` for the tier then the typed loader; `LoadedInstance` dispatch unchanged),
      `list_records_filtered`, `list_record_summaries`, `list_record_tags`.
- [x] `record_store.rs` writers → `save_record`: `create_record_at_dir`/`write_record` (Tier-2
      new-id branch), `update_record` (existing-id branch — path preserved, index tags refreshed),
      `add_record_tag`, `remove_record_tag`, `append_source_ref`, `create_record_successor`,
      `write_new_record` (the second Tier-2 write path used by `repository_lifecycle.rs:120`).
      These lose their manual `write_record`+`upsert_record_index_entry` pairs (subsumed by
      `save_record`) — they get **shorter**. (`delete_record` is carved out — see below.)
- [x] `writer.rs::write_note` → `save_note`; `services.rs` `note_service`: `list_notes`,
      `get_note_by_id`, note create, `update_note`, `delete_note`, `add_note_tag`, `remove_note_tag`
      → typed methods.
- [x] Keep `record_store::load_record(store, path)` and `loader::load_note(store, path)` as
      transitional path helpers (**signatures unchanged**) — after this phase their only callers are
      the deferred readers + revision-coupled trio.
- [x] **Keep `dir_override`** on `create_record_in_context` — it backs the `srs record create --dir`
      CLI flag (`srs-cli/.../record.rs`) and the MCP `record_create` tool; it is not dead. The
      routing lives in `create_record_at_dir` instead (next task).
- [x] `create_record_at_dir` is still used by `extension_service.rs` (Extension dir), so **keep it**;
      its Tier-2 internal path delegates to `save_record`, the Extension-dir path stays transitional.

#### Tasks (carve out — do NOT migrate; allow-list in Phase 3)

- [x] `delete_record`, `transition_record_lifecycle`, `list_record_revisions`, `get_record_revision`:
      retain their `instance_index` → `path` lookup **only** to pass the path to `revision_service`
      (`delete_sidecar`/`append`/`list`/`get`). `delete_record` already does an ADR-007 index-first
      delete and does not load by path; where a function writes the record
      (`transition_record_lifecycle`), route that write through `save_record`. The residual path
      lookups are deferred with the revision-storage migration (#725).

#### Acceptance Criteria

- [x] All readers/enumerators/writers listed above in `record_store.rs` and `note_service` no longer
      call `entry.path()` / `load_record(path)` / `load_note(path)`.
- [x] `save_record`'s existing-id branch refreshes the index entry's `tags` (and note `title`) so
      `add_record_tag`/`update_note`/etc. keep the discovery columns correct — verified by the
      existing tag tests (e.g. `services.rs` note-tag tests, record-tag tests).
- [x] `create_record` / `create_record_in_container` / `create_record_in_context` / `update_record`
      / `transition_record_lifecycle` / `create_record_successor` still pass their existing tests.
- [x] Notes create/read/update/delete/tag unchanged in behaviour.
- [x] No behavioural change to CLI output (integration tests green).

#### Testing

```bash
cargo test -p srs-repository
cargo test -p srs-cli
```

Regression guard (must still pass): `list_record_summaries_roundtrip_stores`,
`get_record_summary_by_id_roundtrip_stores`, `create_record_in_container_roundtrip_stores`,
`create_record_in_context_container_branch_roundtrip_stores`, `lifecycle_transition_roundtrip_stores`,
`rfc022_fulfillment_roundtrip_stores`, plus note-service tests and `discovery_service` parity tests.

#### Milestone gate

```bash
cargo test
cargo clippy -- -D warnings
```

Then mark checkboxes and commit: `refactor(store): route instance funnel + enumerators through logical-id methods (#724)`.

---

### Phase 3: `InstanceIndexEntry.path` contract-opaque (scoped) + docs

**Goal:** `InstanceIndexEntry.path` is documented as an adapter-private key; the migrated paths are
proven id-based by a **crate-wide** gate with an explicit allow-list of the deferred readers; ADR-042
+ affected docs land in this PR's diff.

**Agent:** Repository Worker / Lead Integrator

#### Tasks

- [x] Doc-comment `InstanceIndexEntry.path` / `path()` in `src/index.rs`: "adapter-private key;
      migrated service code addresses instances by id via the store's typed methods (ADR-041 G5,
      ADR-042). Remaining path-based readers are tracked in the deferred follow-up."
- [x] Stage 7.5 docs (see that stage): `srs-rust/CLAUDE.md` Storage Boundary note; `ARCHITECTURE.md`
      Store Matrix / Storage Direction pointer to ADR-042; flip ADR-042 status to `accepted` at merge.

#### Acceptance Criteria

- [x] **Crate-wide gate (exhaustive allow-list):** a grep over `crates/srs-repository/src` for the
      pattern "iterate `manifest.instance_index` then load by `entry.path()`" (i.e. `entry.path()`/
      `e.path()` feeding `load_record`/`load_note`/`load_instance_json`) returns hits **only** in the
      allow-list below. `record_store.rs` and `services.rs` appear **only** for their transitional
      `load_record`/`load_note` helper definitions and the revision-coupled trio — no migrated
      function walks the index by path. Allow-list (checked in as a shell gate in the plan/PR):
      - Adapters: `store.rs`, `json_store.rs`
      - Transitional helper definitions: `record_store.rs` (`load_record`), `loader.rs` (`load_note`)
      - Revision-coupled functions (in `record_store.rs`): `delete_record`,
        `transition_record_lifecycle`, `list_record_revisions`, `get_record_revision`
      - Deferred other-file readers: `validation.rs`, `migrate_identity_service.rs`, `analysis.rs`,
        `attachment_policy_service.rs`, `attachment_service.rs`, `vocabulary_service.rs`,
        `container_view_service.rs`, `view_service.rs` (`document_views_for_container`),
        `context_query_service.rs`, `migration_registry_service.rs`,
        `container_service.rs`, `writer.rs` (`build_instance_semantic_types`)
      - Extension records: `extension_service.rs`
      - Archive/export packers (need paths to copy files): `archive.rs`, `export_service.rs`,
        `repository_portability.rs`
      No **new** or **unlisted** file walks the index by path for instance loads.
- [x] Docs updated; ADR cross-referenced.

#### Milestone gate

```bash
cargo test
cargo clippy -- -D warnings
```

Commit: `docs: InstanceIndexEntry.path opaque (scoped) + ADR-042 pointers (#724)`.

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] CLI output format unchanged (integration tests pass)
- [x] `cargo test --test payload_contracts` passes (no payload structs changed)
- [x] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [x] New typed instance methods implemented on all three stores with store-matrix parity tests
- [x] ADR-007 fault-injection test for `save_record` passes (`FailPoint::SaveInstanceIndex`)
- [x] `save_record` existing-id path-stability test passes (no rename on type-version migration)
- [x] The **entire** `record_store.rs` + `note_service` instance funnel (every enumerated reader/
      writer, minus the revision-coupled trio) no longer walks `manifest.instance_index` by
      `entry.path()`; every remaining path-based instance load is named in the exhaustive Phase-3
      allow-list (crate-wide gate)
- [x] `dir_override` retained (`--dir` CLI flag preserved); default Tier-2 writes route through
      `save_record`, custom/Extension dirs through the legacy write in `create_record_at_dir`
- [x] Child stories filed and linked under #704 for the deferred readers, the generic seam, and the
      remaining entity families

## Coordination Rules

- Repository Worker keeps to `crates/srs-repository/**`. No `srs-cli`/`srs-bindings`/`srs-mcp`
  edits — service signatures are unchanged.
- Do not change `record_store::load_record` / `loader::load_note` **signatures** (the deferred
  readers call them with a raw path) — keep them as transitional path helpers.
- Do not `#[deprecated]` the Value/path instance methods this increment (would break
  `clippy -D warnings` across ~140 generic-shim callers) — deprecation is the generic-seam follow-up.
- Each phase ends with the milestone gate (tests + clippy) before the next begins.
- Verification Agent (Stage 7) runs the crate-boundary + duplication audit before PR.

## Assumptions

- `create_record_in_context`'s `dir_override` is a live feature (the `srs record create --dir` CLI
  flag + MCP tool), so it is **retained**. `create_record_at_dir` branches on the target directory:
  the default `records/tier-2` uses `save_record`; any override (including Extension records) keeps
  the legacy path-based write. Only the default tier is migrated this increment.
- `Note` → Tier 0 and `Record` → Tier 2 fully covers live write paths; Tier-1 records are read-only
  legacy (loaded/filtered, never written) and extension records are out of scope. The existing-path
  save branch means a stray Tier-1 update overwrites in place without relocation.
- `LoadedInstance` (service-layer union) is the sanctioned typed currency for polymorphic reads;
  no new `srs-core` `Instance` enum is introduced this increment (owner steer 2026-07-23).
