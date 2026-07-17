# ADR-034: sourceRefs on Record stored in record.extra rather than a typed field

- **Status:** accepted
- **Date:** 2026-07-17
- **Supersedes:** —
- **Superseded by:** —

## Context

RFC-017 requires that attachment links be represented as `SourceReference` entries on a record's
`sourceRefs[]` array. The `Note`, `Relation`, and `Revision` types already carry `source_refs:
Option<Vec<SourceReference>>` as a typed field. For consistency one might expect `Record` to gain
the same typed field.

However, `Record` uses `#[serde(flatten)] extra: HashMap<String, serde_json::Value>` to absorb
unknown top-level keys. Adding a new typed field to `Record` requires updating every struct
literal and pattern-match construction site across the codebase — approximately 30 sites in
`record_store.rs`, `render_service.rs`, `container_view_service.rs`, and integration tests —
for a field that will start as `None` at every construction site.

## Decision

`sourceRefs` on a `Record` is stored and accessed via `record.extra["sourceRefs"]` rather than
as a new typed field on the `Record` struct. The `record_store::append_source_ref` function
encapsulates all read-parse-mutate-write logic for this key, keeping the raw `serde_json::Value`
manipulation out of service code.

## Consequences

**Positive:** Zero churn across the 30+ `Record` construction sites. The `Record` struct contract
remains stable. The storage boundary rule is honoured — path-aware logic stays in `record_store`.

**Negative / trade-offs:** `sourceRefs` on `Record` is not type-checked at construction time
the way it is on `Note`/`Relation`/`Revision`. A future typed migration would require the same
30-site update that was avoided here plus a schema migration.

**Neutral:** `append_source_ref` validates the parsed `Vec<SourceReference>` via serde at both
read and write time, so corrupt data surfaces as a `RepositoryError::Serialize` rather than
silently propagating. The `attachment_service` tests cover the roundtrip through both `MemoryStore`
and `FileStore`.
