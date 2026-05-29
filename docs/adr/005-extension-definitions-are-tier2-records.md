# ADR-005: Extension Definition Records Are Generic Tier 2 Records

- **Status:** accepted
- **Date:** 2026-05-29
- **Supersedes:** —
- **Superseded by:** —

## Context

The CLI command structure plan introduces `srs extension list/get/create/update/delete` commands and `srs repo extensions enable/disable` commands. Two concerns need separating:

1. **`repo extensions` commands** manage `declaredExtensions[]` in `manifest.json` — a string array of active extension IDs (e.g. `"ext:lifecycle"`). These require no record loading.
2. **`extension` commands** query extension *definition records* — the full spec text, dependency declarations, and normative content for each extension. These are Tier 2 Records in the spec package.

The spec already defines `meta.extension` as a Tier 2 type (`srs/srs/package/types/meta.extension.json`, id `0f71335f-1bef-5fb7-8944-ae2f3ecaf3ec`). All 13 SRS extensions are stored as Tier 2 Records of this type in `srs/srs/records/extensions/`.

The question is whether the Rust library needs a native `Extension` struct in `srs-core`, or whether the generic record infrastructure suffices.

Two options were considered:

**Option A — Native core type:** Define an `Extension` struct in `srs-core` alongside `Note` and `TagDefinition`. Give it dedicated service functions. The Rust struct is authoritative for loading.

**Option B — Generic Tier 2 Record:** Extension definitions are loaded and queried through the generic `list_records_by_type` / `get_record_by_id` / `create_record` services, using the `meta.extension` type resolved by name.

## Decision

`Extension` definitions are **generic Tier 2 Records** bound to `com.semanticops.srs/meta.extension@1` (Option B). No native Rust struct is required in `srs-core`. The `srs-core/src/extensions/mod.rs` stub remains empty and may be removed.

The `srs extension` CLI commands use `list_records_by_type`, `get_record_by_id`, and `create_record` with the `meta.extension` type resolved by name — no hardcoded type UUID constants in CLI code.

The `srs repo extensions enable/disable` commands operate only on the `declaredExtensions[]` string array in `manifest.json`. They do not load or validate extension definition records.

**Contrast with TagDefinition (ADR-003):** `TagDefinition` is a native core type because `get_foundation_signal_tags` must be callable by any SRS consumer (CLI, bindings, WASM) without configuring a package or knowing type UUIDs. Extension lookup has no equivalent universal primitive — nothing in the system needs to call `get_extension_by_role` or similar as a library operation. Extensions are referenced by string ID in the manifest, not by loading and filtering records.

## Consequences

**Positive:**
- No maintenance burden of keeping a Rust struct in sync with the `meta.extension` type JSON.
- `srs extension list` immediately works against any repo that has the `com.semanticops.srs` package.
- Consistent with ADR-002: generic record operations for spec-defined types that are not universally queried primitives.
- `srs-core/src/extensions/mod.rs` can be removed, eliminating a misleading empty stub.

**Negative / trade-offs:**
- Extension field access is via `fieldValues` lookup, not typed struct fields. Code that needs extension data must know field IDs or resolve by field name from the package.
- No `extension.has_dependency("ext:lifecycle")` ergonomic helper — callers must inspect `fieldValues` directly.

**Neutral:**
- `repo extensions enable/disable` and `extension list/get/create` are distinct operations with distinct semantics. The CLI naming distinguishes them: `repo extensions` = manifest string management; `extension` = definition record management.
- The `meta.extension` type namespace is `com.semanticops.srs`, not `com.semanticops.spec`. CLI code resolves it by name, not by hardcoded UUID.
