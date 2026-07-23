# ADR-042: Logical-id + typed-entity instance persistence (ADR-041 G3–G5, records + notes)

- **Status:** proposed
- **Date:** 2026-07-23
- **Supersedes:** —
- **Superseded by:** —
- **Related:** ADR-041 (storage-backend guardrails G3/G4/G5), ADR-007 (write-before-index
  ordering), ADR-010 (service boundary contract — typed in/out), ADR-021 (batch write mode — the
  JsonStore methods inherit G6 via the shared `flush()`), ADR-024 (best-effort rollback for
  two-write operations), ADR-040 (collision-safe canonical filenames — cited for filename-scheme
  reuse); the container precedent (`save_container`/`load_container`, `store.rs`).

## Context

ADR-041 G3–G5 direct the `RepositoryStore` trait to migrate entity persistence off
`(relative_path, serde_json::Value)` methods and onto **typed entities keyed by logical id**,
following the container precedent (`save_container(&Container)` / `load_container(id)`), and to
make instance **enumeration/query store-answerable** rather than a service-layer walk of
`manifest.instance_index` by `InstanceIndexEntry.path`. This ADR records how that shape is
applied to the **first and largest** entity family — **instances (records + notes)** — and
establishes the template the remaining per-entity stories (relations, fields, types, views)
under [srs-rust#704](https://github.com/the-greenman/srs-rust/issues/704) follow.

Four facts about the current code shape the decision:

1. **No unified `Instance` type exists.** `Note` and `Record` are separate `srs-core` structs;
   there is no `TypedRecord` type (Tier 1 is a `Record` discriminated only by the manifest
   `tier`). The only union is the service-layer `LoadedInstance { Record, Note }` in
   `record_store.rs`. The owner steer (2026-07-23) is to **reuse existing types**, not introduce
   a new `srs-core` enum in this increment.

2. **Tier is not carried in the entity body.** `InstanceIndexEntry.tier: u8` (in the manifest
   index) is the only tier signal; `write_record` hardcodes tier 2, `write_note` tier 0. A typed
   write API therefore derives tier from the runtime type: **`Note` → Tier 0, `Record` → Tier 2**
   (the only tiers any live write path produces).

3. **`load_instance_json` / `save_instance_json` are abused as a generic JSON shim.** ~140+ call
   sites (across ~28 files) use them to read/write non-instance JSON (`package.json`, view/theme/
   blueprint definitions, portability copies). The genuine instance funnel is only a handful of
   functions. This is why the Value methods **cannot be `#[deprecated]` in this increment** — the
   warnings would break `clippy -D warnings` and would mislabel valid non-instance I/O as
   instance-persistence debt. Deprecation is gated on first giving those callers a dedicated
   generic seam (filed follow-up).

4. **Path-based instance readers are more numerous than the CRUD funnel.** Beyond the
   record_store/note-service funnel + enumerators, five more files load instances by
   `entry.path()`: `validation.rs` (the `repo validate` identity check), `migrate_identity_service.rs`,
   `analysis.rs`, `attachment_policy_service.rs`, `attachment_service.rs` (the last two call the
   crate-visible `record_store::load_record` directly). This increment migrates the funnel +
   enumerators and **defers these five** (filed follow-up) — so the path-based helpers
   `record_store::load_record(store, path)` and `loader::load_note(store, path)` survive
   transitionally to keep them compiling.

## Decision

Add a logical-id + typed-entity **instance** surface to `RepositoryStore`, implemented on all
three stores (FileStore, JsonStore, MemoryStore), and route the instance CRUD funnel + enumerators
through it. Concretely:

**New trait methods** (names final, subject to Lead Integrator/review polish):

```rust
// Typed, logical-id-keyed instance persistence (ADR-042). Tier derived from the type:
// Note → Tier 0, Record → Tier 2.
fn save_record(&self, record: &Record) -> Result<(), RepositoryError>;
fn save_note(&self, note: &Note) -> Result<(), RepositoryError>;
fn load_record_by_id(&self, instance_id: &str) -> Result<Record, RepositoryError>;
fn load_note_by_id(&self, instance_id: &str) -> Result<Note, RepositoryError>;
fn delete_instance(&self, instance_id: &str) -> Result<(), RepositoryError>;

// Store-answerable enumeration/query (G5). InstanceRef mirrors the index-satisfiable columns
// (no path). A SQL backend answers these with a native query; the file/json/memory stores
// answer them from manifest.instance_index without the service walking entry.path().
fn find_instance(&self, instance_id: &str) -> Result<Option<InstanceRef>, RepositoryError>;
fn list_instances(&self, query: &InstanceQuery) -> Result<Vec<InstanceRef>, RepositoryError>;
```

**Write ordering & path stability — `save_record`/`save_note` mirror `save_container`'s two
branches** (`store.rs:1409-1440`):
- **Existing id** (already in `manifest.instance_index`): overwrite the entity **at its existing
  indexed path** — the path and `tier` are preserved (no rename). This keeps on-disk layout stable
  across a type-version-migrating `update_record` (which changes `type_name`, hence the slug) and
  prevents a stray Tier-1 record from being relocated into `records/tier-2`. The index entry's
  **denormalized fields (`tags`, and for notes `title`) ARE refreshed** from the entity — a
  refinement over `save_container`'s "index unchanged", required so the tag/update writers
  (`add_record_tag`, `update_record`, …) keep the index's discovery columns in sync exactly as
  their current `upsert_*_index_entry` calls do.
- **New id**: derive a fresh collision-safe filename (`{tier_dir}/{slug}-{id8}.json`, the existing
  scheme, ADR-040 cited for reuse — not for its full-UUID disambiguation fallback) and write the
  **entity before the index entry** (ADR-007: an orphaned file is safe; a dangling index entry
  breaks every subsequent load).
- `delete_instance` removes the **index entry before the entity file** (ADR-007 index-first on
  delete) and returns a not-found error for an unknown id. All three impls follow this ordering —
  including MemoryStore, which must not copy `delete_container`'s data-first inversion.

**New value types** (in `srs-repository/src/index.rs`, next to `InstanceIndexEntry`):

```rust
pub struct InstanceRef { pub instance_id: String, pub tier: u8,
                         pub title: Option<String>, pub tags: Vec<String> }
// Index-answerable axes only. Default = match all. `tag` is a single contains-predicate,
// matching the existing singular RecordListFilter.tag / ListNotesFilter.tag.
pub struct InstanceQuery { pub tier: Option<u8>, pub tag: Option<String> }
```

The FileStore instance-index helpers (analogues of `file_store_{load,find,upsert,remove}_container_index`)
return the **typed** `InstanceIndexEntry` / `InstanceRef`, not the untyped `(String,String,String)`
tuples the older container helpers use.

**Polymorphic dispatch stays in the service.** `LoadedInstance` and `get_instance_by_id` remain
in `record_store.rs`; `get_instance_by_id` uses `find_instance` (for the tier) then the typed
`load_record_by_id` / `load_note_by_id`. The store trait does **not** depend on `LoadedInstance`,
keeping the seam narrow.

**`InstanceIndexEntry.path` becomes a contract-opaque adapter key** — the same status
`ContainerIndexEntry.path` already has. The **migrated** funnel + enumerators address instances by
id; only the FileStore/JsonStore adapters and the explicitly-deferred readers (fact 4) read `path`.

**Migrated funnel + enumerators** — the **complete** set of instance-loading functions in
`record_store.rs` and the `services.rs` `note_service`, minus the revision-coupled carve-out
below. Enumerated exhaustively so the claim is verifiable:
- `record_store.rs` readers: `list_all_records`, `list_records_by_type`, `get_record_by_id`,
  `get_instance_by_id`, `list_records_filtered`, `list_record_summaries`, `list_record_tags`.
- `record_store.rs` writers: `create_record_at_dir`/`write_record`, `update_record`,
  `delete_record`, `add_record_tag`, `remove_record_tag`, `append_source_ref`,
  `create_record_successor`, `write_new_record`.
- `services.rs` note funnel: `list_notes`, `get_note_by_id`, note create, `update_note`,
  `delete_note`, `add_note_tag`, `remove_note_tag` (and `writer.rs::write_note` → `save_note`).

**Revision-coupled carve-out (deferred, allow-listed):** `transition_record_lifecycle`,
`list_record_revisions`, `get_record_revision` retain a raw `manifest.instance_index` → `path`
lookup **solely** because they pass the instance's relative path to the path-keyed
`revision_service` (revision storage is a separate, out-of-scope concern). Where they also write
the record, that write routes through `save_record`; the residual path lookup is deferred with the
revision-storage migration and named in the allow-list.

**The `dir_override` escape hatch is retired.** Every live caller of `create_record_in_context`
passes `dir_override: None`; records always land in `records/tier-2`. `save_record` owns that
directory, so the parameter is removed rather than preserved as path-thinking.

**Explicitly deferred** (each filed as a child issue under #704 by this plan):
- The generic definition/blob JSON seam that replaces the ~140 `load/save_instance_json` shim
  callers, **then** `#[deprecated]` on the Value/path instance methods (their real deprecation path).
- **All remaining path-based instance readers/callers** outside the migrated `record_store.rs` +
  `note_service` funnel (#725, expanded): the revision-coupled trio above, plus every other file
  that loads an instance by `entry.path()` — `validation.rs`, `migrate_identity_service.rs`,
  `analysis.rs`, `attachment_policy_service.rs`, `attachment_service.rs`, `vocabulary_service.rs`,
  `container_view_service.rs`, `view_service.rs`, `context_query_service.rs`,
  `migration_registry_service.rs`, `container_service.rs`, and
  `writer.rs::build_instance_semantic_types` — and, once they are
  migrated, removal of the transitional `load_record(path)` / `load_note(path)` helpers. (The
  archive/export packers legitimately need paths to copy files and stay on the adapter path.)
- Extension-tier records (`RecordTier::Extension` → `package/records`) — package-scoped, not
  Tier-0/1/2 instances.
- The other entity families: relations, fields, types, views (L1/L2), and the smaller definition
  entities (themes, blueprints, vocabularies, lifecycles, relation-type definitions).

## Consequences

**Positive:**
- The highest-value entity family gains the container-shaped logical-id contract; a SQL adapter
  implements `save_record`/`list_instances` as table rows + a native query, not a path-keyed blob
  (the concrete G3/G5 win that de-risks the SQLite spike most).
- The **entire** `record_store.rs` + `note_service` instance CRUD/enumerate funnel (every
  instance-loading function in those two modules, minus the revision-coupled trio) no longer walks
  `manifest.instance_index` by `path`. `InstanceIndexEntry.path` is contract-opaque for those
  paths, matching containers.
- Establishes the exact template (typed per-shape methods + an index-answerable `*Ref`/`*Query`
  pair, dispatch left in the service, typed index helpers) for the remaining five entity stories.

**Negative / trade-offs:**
- The claim is **module-scoped, not crate-wide**: the revision-coupled trio, ~11 other files that
  load instances by path (fact 4), and the ~140 generic-shim callers still address instances/JSON
  by path. The plan narrows its wording to the `record_store.rs`+`note_service` funnel and its
  acceptance gate is a **crate-wide grep with an exhaustive, explicit allow-list** of every
  remaining path-based reader, so the narrowed claim is provable and drift is detectable — rather
  than a two-file grep that can't fail. G4's "no `serde_json::Value` currency" is only partially
  realized until the generic-seam follow-up lands; the deprecation path is a filed issue + this
  ADR, not yet a `#[deprecated]` attribute.
- Two write paths and two read helpers coexist transitionally: `save_record`/`save_note` +
  `load_record_by_id`/`load_note_by_id` (canonical) alongside the path-based `write`/`load`
  helpers still used by the deferred readers and extension records. Both maintain the index
  correctly and load-by-id is layout-independent, so this is functionally safe but not yet uniform.

**Neutral:**
- No on-disk format change: `instance_index`/`InstanceIndexEntry` serialization is unchanged;
  existing repositories load unmodified. Retiring `dir_override` changes only *where new* records
  are written (always `records/tier-2`), which every live caller already did.
- Tier derivation (`Note`→0, `Record`→2) is a code convention, not a spec change; it matches the
  existing hardcoded tiers in `write_record`/`write_note`.
