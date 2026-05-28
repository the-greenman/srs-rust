# ADR-002: Tier 2 Record Operations Are Generic, Not Type-Specific

- **Status:** accepted
- **Date:** 2026-05-28
- **Supersedes:** —
- **Superseded by:** —

## Context

SRS has three instance tiers:

- **Tier 0 (Note)** — free text sections, no type binding. Universal: every SRS repository has Notes.
- **Tier 1 (TypedRecord)** — named fields, no type binding.
- **Tier 2 (Record)** — bound to a Type definition (`typeId` + `typeVersion`) declared in the repository's package.

Tier 2 types are defined as data in the package (`package/types/*.json`). Examples in the SRS spec repo: `meta.section`, `meta.extension`, `meta.requirement`. The planned `tag-definition` type is another.

A naive approach would implement Tier 2 support by writing type-specific Rust code for each type: `load_tag_definition`, `list_tag_definitions`, `create_tag_definition`. This would require the same treatment for every future type (`load_extension`, `list_extensions`, etc.), violating DRY at the type level and coupling the library to specific type semantics.

## Decision

`srs-repository` provides **generic** Tier 2 Record operations that work against the package's type definitions:

```
list_records_by_type(repo_root, type_namespace, type_name) -> Vec<Record>
get_record_by_id(repo_root, id) -> Option<Record>
create_record(repo_root, type_id, type_version, field_values, relative_dir) -> Record
```

The library has no knowledge of `TagDefinition`, `Extension`, or any other concrete Tier 2 type. All type semantics (field names, required fields, allowed values) live in the package JSON files.

The CLI may name commands after types (`srs tag list`) but its handlers call the generic record operations:

```rust
// In cmd_tag_list:
list_records_by_type(repo, "com.semanticops.srs", "tag-definition")
```

Adding a new Tier 2 type requires:
1. A new type definition file in `package/types/`
2. New field definition files in `package/fields/` (if needed)
3. An update to `package/package.json`
4. Optionally: a CLI command that calls the generic operations

No new Rust library code is required per type.

## Consequences

**Positive:**
- Adding a new Tier 2 type is a data change, not a code change.
- `srs-repository` stays small and does not accumulate type-specific modules.
- The package system becomes the authoritative source of type semantics — consistent with the SRS spec's intent.
- Generic record validation (required fields, value types) is implemented once and applies to all types automatically.

**Negative / trade-offs:**
- Tier 0 (Note) remains hardcoded in the library because Notes are universal and their structure is defined by the SRS spec itself, not a package. This creates an asymmetry: Notes have dedicated service functions; Tier 2 types use generic operations.
- Rich type-specific behaviour (e.g. computed fields, cross-record constraints) cannot be expressed through the generic path alone. These would require extension points not planned in this ADR.
- The package must be loaded before any Tier 2 operation, adding a load step that Note operations don't require.

**Neutral:**
- The `FOUNDATION_SIGNAL_TAGS` CLI constant (a list of tags used to select foundation notes) is transitional. Once `tag-definition` records with a `foundation` role exist in the repo, `cmd_note_foundations` can derive its tag list from the data. The constant is removed at that point.
- Tier 1 (TypedRecord) is deferred — no concrete use case exists yet. If it is implemented, it follows the same generic-operations principle.
