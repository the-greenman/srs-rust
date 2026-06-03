# ADR-012: Vocabulary Substrate — Term, Vocabulary, and Lifecycle as Package Definitions

- **Status:** accepted
- **Date:** 2026-06-03
- **Supersedes:** ADR-003 (TagDefinition Is a Core SRS Type)
- **Superseded by:** —

## Context

RFC-006 (Vocabulary Substrate, Rev 8) identified that SRS had four different implementations of the same concept — a set of defined strings with stable identity, key field, status, and enrichment metadata:

- `TagDefinition` — lived in the instance index (tier 3), used `tagKey`, had a `flatten extra` bag
- `LifecycleState` — inline on `RecordType.lifecycle`, used `name`, no stable identity
- `RelationTypeDefinition` — in the package, used `relationType`, had `deny_unknown_fields`
- `selectOptions` — anonymous inline array, no identity at all

ADR-003 established `TagDefinition` as a core native type (tier 3 instance) with its own instance index entries. This gave tags stable discovery but conflated definition with instance, left tags in no container, and diverged from the pattern RFC-005 established for relation types.

RFC-006 unifies all four as specializations of the **vocabulary-entry substrate**: a shared contract (id, version, namespace, `key`, status, properties) with domain-specific payloads.

## Decision

`Term`, `Vocabulary`, and `Lifecycle` are **package-level definitions**, not instance-index entries.

- **`Term`** replaces `TagDefinition`. Terms live inside a `Vocabulary` (typically a local `mode: open` vocabulary in the package). `tagKey` is renamed to `key` (the unified substrate field). The `extra` bag is replaced by `properties`.
- **`Vocabulary`** is a named, versioned set of `Term` entries with a `mode` (open/closed). Tags resolve against all vocabularies in the effective package set; unmatched tags in an open vocabulary are valid.
- **`Lifecycle`** is an installable, referenceable container (a closed vocabulary of states + transitions). `RecordType` may reference a shared `Lifecycle` via `lifecycleRef` instead of declaring an inline lifecycle.
- **`LifecycleState.name`** is renamed to `key` (serde alias `name` preserved for backward compat).
- **`RelationTypeDefinition.relationType`** is renamed to `key` in Rust (serializes as `relationType` for JSON compat; also accepts `key` as alias).
- **`TagDefinition` write operations** are deprecated. `is_tag_definition()` on manifest index entries is deprecated. `tier: 3` is retired as a concept.

## Consequences

**Positive:**
- One pattern for all vocabulary types: same substrate, same status semantics, same `properties` extensibility.
- Tags (Terms) no longer pollute the instance index; the manifest is cleaner.
- Lifecycles are shareable across Types — one definition, many references.
- `srs tag list` now returns `terms` with a `key` field; `srs vocabulary list`, `srs lifecycle list` are new commands.
- V3, V5, V7, V9 validation invariants are enforced.

**Negative / trade-offs:**
- `tag create/update/delete` CLI commands now return descriptive errors. Existing workflows that created TagDefinitions via the CLI must switch to editing package vocabulary files.
- `TagDefinition` (the Rust struct) and its service functions are retained but marked `#[deprecated]`. Final removal is deferred.
- Container-scoped tag listing is no longer meaningful since terms are not instance-index members; the command returns an empty list for repos without vocabulary files.

**Neutral:**
- `TagDefinition` has no applied uses in the srs/ spec repo, so the migration carries no data cost.
- Existing inline lifecycles continue to work; extraction to standalone `Lifecycle` definitions is optional.
