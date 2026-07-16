# ADR-029: Protocol Run Storage Model

- **Status:** proposed
- **Date:** 2026-07-16
- **Supersedes:** the "execution out of scope" note in [ADR-016](016-protocols-are-package-definitions.md)
- **Superseded by:** —

## Context

ADR-016 established that Protocol *definitions* are package definitions and explicitly deferred execution state (runs, sessions, stage advancement) as out of scope. Now that `AttentionState` exists (ADR-029 precursors: #250, #251), protocol execution can be implemented.

Protocol runs are distinct from protocol definitions:
- **Definitions** are package-level, shipped by the package author, loaded at package-load time (ADR-016).
- **Runs** are instance-level, created and mutated per-session in a specific repository. They are not records (no type binding), not package definitions (no `package.json` registration), and not relations (no source/target instance pair).

Three storage options were considered:

1. **Single file `runs/runs.json`** — all runs for the repository in one JSON array, loaded/mutated/saved whole. Simple, consistent with `relations/relations.json`.
2. **Individual per-run files** (`runs/<uuid>.json`) — one file per run, indexed in `manifest.instanceIndex`. More files, but no whole-file rewrite on each mutation.
3. **Manifest-embedded** — runs field in `manifest.json`. Changes the manifest schema, coupling run state to the repository bootstrap path.

Protocol runs are expected to be short-lived and few in number (one active run per facilitation session, usually one to a handful per container over the repository's lifetime). Whole-file rewrite on each mutation is not a performance concern at this scale.

## Decision

Protocol runs are stored in `runs/runs.json` at the repository root, as a JSON object:
```json
{ "runs": [ { ...ProtocolRun... }, ... ] }
```

This file is read and written by `protocol_run_service` using the existing `store.load_instance_json("runs/runs.json")` and `store.save_instance_json("runs/runs.json", ...)` methods. No new trait methods are added to `RepositoryStore`.

The `ProtocolRunsCollection` struct (`{ runs: Vec<ProtocolRun> }`) is the serde target. On load failure (file absent, I/O not-found), the service returns an empty collection. This makes a fresh repository work without initializing the file.

`ProtocolRun` carries:
- `runId` — stable UUID generated at creation time.
- `protocolId` / `protocolVersion` — reference to the definition (not a strong foreign key).
- `containerId` — the container this run operates within.
- `targetRecordId` (optional) — the record being produced by this run.
- `status` — `Active | Completed | Abandoned`.
- `attentionState` — current `AttentionState` cursor; updated on each stage advancement, satisfying the spec mandate (subsection 08-5-2: stage advancement must update `AttentionState`).
- `stageStates` — append-on-advance list of per-stage status/timestamps.
- `startedAt` / `completedAt`.

The service functions are: `create_run`, `advance_stage`, `get_run`, `list_runs`, `list_runs_for_record`, `complete_run`, `abandon_run`.

`advance_stage` atomically: updates the cursor, optionally marks the previous stage completed, appends a new `StageState`, and saves. There is no partial update — the entire collection is rewritten per ADR-021 (JsonStore batch-write mode) semantics.

## Consequences

**Positive:**
- No new `RepositoryStore` trait methods — uses the existing generic JSON I/O surface.
- `MemoryStore` works out of the box (via its generic `save_instance_json` / `load_instance_json`).
- Simple: the service is self-contained; no manifest migration needed; fresh repos work immediately.
- `AttentionState` is updated on every stage advance, satisfying the spec mandate in subsection 08-5-2.
- The `protocol_run_history` placeholder in `context_query_service::RecordContextResult` is filled by `list_runs_for_record`.

**Negative / trade-offs:**
- Whole-file rewrite on every mutation. Acceptable for the expected run count (< 50 per repository over a lifetime). If run counts grow, per-run files with a separate index (analogous to `manifest.instanceIndex`) would be more efficient — file this as a follow-up if it becomes a concern.
- `stageStates` is an append-only list; there is no delete surface for individual stage states.
- No referential integrity check between `protocolId` and the loaded package — runs that reference a deleted protocol definition are not invalidated.
- No schema file for `runs/runs.json` under `srs/docs/schema/2.0/` — validation is handled by serde deserialization only (same as revision sidecars).

**Neutral:**
- `protocol run complete` / `abandon` on an already-terminal run returns an error (not a no-op idempotent call). Callers that need idempotent behaviour can check status first via `get_run`.
- Tagged chunks (conversation material associated with a stage) are out of scope for this ADR; `tagged_chunks` remains an empty placeholder.
- Stage context query (`{runId}/{stageId}`) is deferred to a follow-up; runs data enables it once conversation chunk storage exists.
