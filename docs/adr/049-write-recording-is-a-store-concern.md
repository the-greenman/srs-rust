# ADR-049: Write recording is a store concern

- **Status:** proposed (accepted on ship of #1202)
- **Date:** 2026-10-03
- **Supersedes:** —
- **Superseded by:** —

## Context

Clients presenting agent activity (srs-web#372) need to know which instances an MCP request changed. Today they parse `tools/call` requests and results, gated on `write_epoch()` moving: client-side semantics that misses relations and multi-write tools. The ask is srs-rust#1202. Constraints: one way per goal (ADR-048 rule 3), capability once in core (capability-layering), thin MCP/WASM adapters (ADR-037, ADR-013), Vfs is the single I/O seam (ADR-038).

## Decision

1. Changes are recorded once, inside `FileStore`'s three Vfs wrappers (`vfs_write`, `vfs_remove`, `vfs_create_dir_all`; only the first two record), the same place that already advances `write_epoch`. A guard test fails if any other site in `store.rs` calls the Vfs mutators.
2. Classification is by path prefix: `records/` -> `instance`, `relations/` -> `relation`, `containers/` -> `container`; other paths (package, source documents, manifest) are not recorded in v1. The id is read from the written JSON (`instanceId` / `relationId` / `containerId`). `created` when the file did not exist, `updated` when it did, `deleted` on removal; ids come from the JSON, never the path (ADR-042). Entries coalesce per `(target, id)` within a drain window; created+deleted is `deleted` and deleted+created is `updated`, so a path move never loses an id. Only `containers/*.json` are containers; the root container in `manifest.json` is unreported in v1.
3. Recording is opt-in (`set_change_recording`, default off, enabled by `SrsMcpApplication::new`) so CLI and bulk paths pay nothing. On a mid-way failure with best-effort rollback (ADR-024) the summary reports what reached the Vfs. `RepositoryStore::drain_changes()` (default: empty) hands the buffer to the adapter. The buffer is shared by store clones like the epoch. `McpSession` drains around each `tools/call` and exposes `take_write_summary()` (WASM only; MCP wire unchanged). The summary is session-scoped transient telemetry, never persisted, never provenance (RFC-046 `createdBy` is unaffected).
4. Rejected: per-service summaries (N hand-built implementations, drift); before/after catalog diffs (O(repo) per request); an MCP-result side channel and a CLI mirror (a second mechanism with no current consumer).

## Consequences

**Positive:** one mechanism covers every present and future tool; clients stop parsing requests.

**Negative / trade-offs:** no tool-level semantics (`moved`, field names) in v1; a container change is `updated`; manifest writes are unreported.

**Neutral:** UI writes through `SrsRepository` also land in the buffer; `McpSession` discards entries made before a call starts, so they are never attributed to the agent.

## Implementation charter (ADR-048)

- [x] **Spec-first** — no spec ruling is made; transient telemetry, no spec change (#1202 spec gate).
- [x] **Layer test** — core (srs-repository store) records; WASM adapter exposes; srs-web presents.
- [x] **One way per goal** — replaces client request parsing; no per-tool builders.
- [x] **Parity and mirror obligations** — none (no payload struct, no schema; ADR-011 not triggered).
- [x] **Decision mode** — complicated (owner-ruled 2026-10-03).
