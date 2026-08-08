# ADR-043: RFC-039 revision-2 carrier representation, serialization order, and migration I/O

**Status:** accepted
**Date:** 2026-08-08
**Issues:** srs-rust#806 (srs#242 Phase B unit 1)
**Amends:** ADR-017, ADR-036, ADR-037 (mechanism only — the determinism guarantee is
unchanged and strengthened; each of the three states "`preserve_order` disabled" as a
load-bearing invariant, and this ADR replaces that mechanism)

## Context

RFC-039 (Accepted Rev 6) replaces `Record.fieldValues` — `FieldValue[]` keyed by `fieldId` —
with an object keyed by `Field.name` verbatim, recursive per RFC-032 `projectField`'s instance
space, with a parallel `fieldMeta` provenance sibling. It removes `FieldGroup`, `groupValues`,
`ext:repeatable-fields` `entries`, and the `FieldAssignment.{repeatable,minItems,maxItems}`
trio. The owner decision on srs#256 sets a zero-length compatibility window: no public upgrade
command, no old-format runtime reader, no intermediate supported format. Rollback is
`git revert` of the cutover train.

Three questions had plausible alternatives someone might revisit, and one hidden conflict had
to be resolved rather than asserted away:

1. How the object carrier is held in Rust, given [R18]: `fieldValues` keys MUST serialise in
   `FieldAssignment.order` at every depth, byte-idempotently. **The conflict:** ADR-017's
   determinism mechanism is the BTreeMap-backed `serde_json::Value` — every
   `serde_json::to_value(entity)` re-sorts keys alphabetically (`preserve_order` deliberately
   off, audited at workspace `Cargo.toml:44`). Any order-preserving struct field loses its
   order the moment it passes through that funnel, so [R18] and ADR-017-as-mechanism cannot
   both stand.
2. Whether the runtime keeps any ability to read revision ≤ 1 instances.
3. What data the migration transform operates on and which I/O seam it uses — bounded by
   ADR-041 G1 (no `Vfs`/path literals in services), G4 (no `Value` as the store's entity
   currency), G6/ADR-021 (multi-write operations use the batch seam), and ADR-042 (typed
   logical-id persistence, with `load_instance_json`/`save_instance_json` retained as the
   sanctioned transitional generic-JSON surface).

## Decision

1. **Carrier type + serialization order.** `Record.field_values` is a newtype
   `FieldValues(IndexMap<String, serde_json::Value>)` (workspace `indexmap`, `serde`
   feature); `field_meta: Option<IndexMap<String, FieldMeta>>` mirrors its keys ([R6]).
   Write paths normalise insertion order to `FieldAssignment.order` ([R18]).
   **`serde_json`'s `preserve_order` feature is enabled workspace-wide**, and the
   determinism mechanism is amended across ADR-017/036/037: determinism now comes from
   **canonical types at the source** — entity `extra`/auxiliary maps are `BTreeMap`
   (sorted, deterministic), the carrier maps are `IndexMap` (order is data, per [R18]) —
   plus an **explicit canonicalize-on-write step in the `.srsj` writer** that recursively
   sorts every object key set *except* `fieldValues`/`fieldMeta` subtrees. This makes
   ADR-036's `.srsj` ordering claim construction-path-independent (stronger than the old
   implicit BTreeMap-`Value` re-sort) while letting the carrier keep [R18] order. Cargo
   feature-unification propagates the flag to `srs-mcp`/rmcp (the tree ADR-037 audited for
   the flag's *absence*): MCP JSON-RPC output is key-order-insensitive by protocol, verified
   by the MCP snapshot tests under the flag, and ADR-037's constraint line is amended rather
   than silently violated. The stale `srs-projection/src/json_schema.rs:14-17` comment is
   updated in the same commit. Every `serde_json::to_value` call site is audited in the
   implementing PR; the guarantee (byte-deterministic serialization) is unchanged, the
   mechanism is explicit instead of implicit. Golden/fixture byte churn from the flip is
   absorbed in this train, which rewrites the fixture corpus anyway.
2. **No legacy runtime reader.** Deserialization applies [R9]'s structural test (array
   `fieldValues` ⇒ revision ≤ 1; Tier-1 `TypedField` without `fieldType` ⇒ revision ≤ 1) and
   fails with a typed error naming the document and the expected `dataModelRevision`. No
   coercion, no partial read, no silent skip. Legacy-shape fixtures exist only as named
   `legacy-rev1-*` rejection-test inputs.
3. **Migration I/O: store-routed raw JSON inside the batch seam.** The transform operates on
   raw `serde_json::Value` documents — it cannot use typed entities on either side of the
   boundary: pre-migration documents are rejected by (2), and round-tripping a legacy
   definition through the *new* typed layer would silently drop the removed trio serde-side,
   destroying Change E.2's `G.repeatable → cardinality: "list"` input. It reads and writes
   through `RepositoryStore`'s generic-JSON methods (`load_instance_json` /
   `save_instance_json`, used for definition documents too — there is no separate raw
   definition method; `package_service.rs` tests are the precedent) — **never through
   `Vfs` directly** (ADR-041 G1) — and the whole run is wrapped
   `begin_batch` → transform → `commit_batch`, with `abort_batch` on every abort
   disposition (ADR-021, ADR-041 G6), so an abort leaves no store half-migrated ([R13]).
   **This is a named exception to the ADR-042/CLAUDE.md doctrine** ("instance persistence
   goes through the typed logical-id methods … never `load_instance_json`/
   `save_instance_json` with a path") **and to the shrink-the-caller-pile goals of
   srs-rust#725/#726**: a format migration is the one consumer that *must* see documents
   the typed layer refuses, it is migration-only and bounded (no general-purpose
   generic-JSON callers are added), and it is added to #725's exemption list when this
   lands. When #726 ships its dedicated generic-JSON seam, the migration service moves
   onto it mechanically.
4. **One value grammar.** The Change-B recursive value rule is implemented once, in
   `srs-core` (`validate_value`), and consumed by instance validation, by the migration's
   post-transform verification, and by a conformance test against `type_schema_service`'s
   emitted schemas — never reimplemented (RFC-039: "a second grammar would be a second
   source of truth").
5. **Registration.** The transform registers as **data-model migration #2** in
   `migration_registry_service` (ADR-032 fn-pointer pattern), exposed through the existing
   `srs migrate` handoff surface. No new public upgrade command (RFC-038 constraint). Every
   first-party tree — including muSrs and the `srs` spec repo in the unit-2 cutover PR —
   migrates through the released binary; `srs/scripts` verify, never transform (retires the
   srs-rust#351 dual-implementation failure class for the carrier migration; #351 itself
   remains open for the RFC-032 transform pair it names).

## Consequences

- The CLI payload contract, WASM bindings, and MCP tool schemas change shape in one release
  (object `fieldValues` + optional `fieldMeta`; `groupValues` removed). Regenerated payload
  golden schemas are the contract record (ADR-011).
- `Record::find_field_value(field_id)` and friends are replaced by name-keyed access plus
  Type-mediated `fieldId` recovery through the effective field set — which is what makes
  [R19] (referenced Type versions are not deletable) load-bearing.
- A binary at this revision cannot load a pre-cutover repository (including `../srs/srs`
  until the unit-2 data PR lands) — expected under the zero-window decision; the [R9]
  diagnostic makes the failure explicit.
- Migration idempotence is testable as byte-equality of a re-run; an aborted migration is
  indistinguishable from one never started (batch rollback).
- The `preserve_order` flip re-orders keys in re-serialized artifacts (struct-declaration /
  authored order instead of alphabetical). Deterministic, but different bytes — golden
  updates land with this train. The `Cargo.toml` audit comment is updated to state the new
  policy.

## Alternatives rejected

- **`HashMap` + sort-on-serialise for the carrier:** loses authored order in memory and
  cannot express [R18] at all.
- **Keep `preserve_order` off + custom `Serialize` for `Record`:** direct-to-writer paths
  would preserve order but every `to_value` intermediate (the ADR-017 funnel, `JsonStore`'s
  `Value`-tree data map, `.srsj` snapshots) still re-sorts — [R18] would silently fail
  exactly where ADR-017 lives. Rejected because it makes the ordering guarantee
  path-dependent.
- **A legacy read path behind a flag:** an intermediate supported format by another name;
  explicitly excluded by the owner decision RFC-039 Change H records.
- **Vfs-direct migration I/O:** violates ADR-041 G1/G6 and bypasses the batch seam; a
  mid-run failure could strand a half-migrated tree — the precise dual-shape state [R13]
  forbids.
- **JS transform in `srs/scripts` (RFC-032 precedent):** recreates the srs-rust#351
  dual-implementation drift for a strictly larger transform; rejected at the srs#242
  Stage-2 checkpoint (decision D2, 2026-08-08).
- **Typed-entity migration:** requires the typed layer to model both generations,
  reintroducing the dual-shape surface [R7]/[R9] exist to forbid — and silently strips the
  trio during legacy deserialization, corrupting the transform's own input.
