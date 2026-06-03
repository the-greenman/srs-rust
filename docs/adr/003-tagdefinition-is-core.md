# ADR-003: TagDefinition Is a Core SRS Type

- **Status:** superseded
- **Date:** 2026-05-28
- **Supersedes:** —
- **Superseded by:** ADR-012

## Context

Tags are a first-class feature of the SRS data model. Every instance tier supports them: Notes (`Note.tags`, `NoteSection.tags`), and all other tiers. Tags are used universally — for navigation, lifecycle signalling, AI context selection (the `foundation` profile), and semantic classification.

Two approaches were considered for `TagDefinition` (the concept of attaching meaning to a tag):

**Option A — Pluggable Tier 2 Record:** Define `TagDefinition` as a Type in the `srs/` spec package (`com.semanticops.srs/tag-definition@1`). Load and list definitions through the generic `list_records_by_type` / `create_record` service functions. Tag semantics live in a package JSON file, not in Rust code.

**Option B — Core native type:** Define `TagDefinition` as a native `srs-core` struct alongside `Note` — a peer to `Field` and `RecordType`. Give it dedicated service functions in `srs-repository` (`list_tag_definitions`, `get_foundation_signal_tags`, etc.). The Rust struct is authoritative for loading; the spec package carries the schema definition for documentation and validation tooling.

## Decision

`TagDefinition` is a **core native type** (Option B).

Tags are too fundamental to be treated as a user-defined package type. Specifically:

- `get_foundation_signal_tags` — selecting notes for AI context handoff — is a core library operation. It must be callable by any SRS consumer (CLI, Python bindings, WASM) without configuring a package or knowing type UUIDs.
- `TagDefinition` is defined by the SRS spec itself, not by individual repositories. Every conforming SRS implementation handles it the same way.
- The generic Tier 2 Record path requires a package to be loaded and a type UUID to be known before any record operation. For a concept as universal as tags, this is the wrong dependency.

`TagDefinition` instances use `tier: 3` in the manifest index, distinguishing them from Notes (`tier: 0`), TypedRecords (`tier: 1`), and generic Records (`tier: 2`).

The generic Tier 2 Record infrastructure (`list_records_by_type`, `create_record`) remains correct and useful for **user-defined types** in a repository's package. `TagDefinition` is simply not that.

## Consequences

**Positive:**
- `get_foundation_signal_tags(repo_root)` is a single library call with no package dependency. Any consumer can call it.
- Tag definition semantics (field names, validation rules) are expressed in typed Rust, not inferred from JSON schema at runtime.
- `TagDefinition::has_role("foundation")` is ergonomic and type-safe.
- No UUID constants (`TAG_DEF_TYPE_ID`, `TAG_KEY_FIELD_ID`) leak into CLI code.

**Negative / trade-offs:**
- The `srs/` spec package still needs a `tag-definition` type definition (for schema documentation and spec validation tooling) — this must stay in sync with the Rust struct. Two representations of the same thing.
- Adding a new field to `TagDefinition` requires a Rust code change, not just a package JSON update.
- `tier: 3` is a new tier value not previously in the spec. The spec should be updated to document it.

**Neutral:**
- `TagDefinition` storage path convention: `records/tag-definitions/<slug>.json`, where slug is derived from `tag_key`.
- The `srs-cli` `tag` command surface (`srs tag list/get/create`) is unchanged from the original plan; only the backing implementation changes (tag service instead of generic record store).
