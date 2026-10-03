# Plan: McpSession per-request write summary (srs-rust#1202)

> Owner rulings 2026-10-03 recorded below (D1=A, D2 reduced shape, D3=A, D4 drained, D5 none).

## Summary

srs-web (#372, merged) learns "which agent changed which instance" by parsing `tools/call` requests and results (`src/lib/agent-activity.ts` `writeFrom`), gated on `session.write_epoch()` moving. That is client-side semantics (capability-layering violation) and lossy (relations, container side-writes, multi-instance tools such as `record_fork`, `container_copy`, `note_graduate`, `record_successor`). This plan records what each request actually wrote at the ONE point every write already funnels through, the `FileStore` mutation seam that bumps `write_epoch` (`write_json` / `delete_file` -> `vfs_write` / `vfs_remove`, store.rs), and exposes it as a typed, drained, per-request summary on `McpSession`. Serves muDemocracy.org#226, epic #224.

## Spec gate (Stage 1.5)

No spec change required. The summary is session-scoped transient telemetry: it is produced in memory, drained after each request, never written to a record, note, relation or manifest, and never read by validation. It is not RFC-046 provenance (`createdBy` stays creation-only testimony; no `updatedBy`). It adds no field, type, relation type or extension, changes no entity semantics, touches no `srs/docs/schema/2.0/` schema, and no spec-defined CLI contract. It does not extend ADR-037 beyond an adapter detail (see D3). Tripwire: if the owner wants it PERSISTED (e.g. `updatedBy`, an edit history), that is a spec change (RFC) and this plan stops.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Store/Core Worker (srs-repository) | — |
| MCP/Bindings Worker (srs-mcp-core, srs-bindings) | — |
| Verification | — |

See [agents.md](agents.md).

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-048](../docs/adr/048-implementation-decision-rules.md) | Rule 2 layer test: recording lives once in the store (core); MCP/WASM only expose; srs-web only presents. Rule 3 one way per goal: ONE recorder, no per-tool summary builders. | applies |
| [capability-layering](../docs/architecture/capability-layering.md) | Core owns the capability, adapters expose it, clients present. Replaces srs-web's request/result parsing. | applies |
| [ADR-037](../docs/adr/037-mcp-adapter-surface.md) | MCP adapter stays thin: no `json!()` construction of semantics, one typed struct serialized. | applies |
| ADR-011 | Only if D5 = CLI mirror (payload struct + golden schema). Recommended: not applicable. | conditional |
| ADR-013 | WASM binding is a thin adapter returning the typed struct as JSON. | applies |
| [ADR-049](../docs/adr/049-write-recording-is-a-store-concern.md) | Write recording is a store concern: one recorder at the FileStore write seam; per-service summaries and snapshot diffs rejected. Also ADR-038 (Vfs seam) is the seam this hooks. | proposed (accepted on ship) |

---

## Contracts

### CLI output contract (ADR-011)
Recommended (D5): no CLI change, no payload struct, no golden schema. CLI commands already return their own typed payloads; a CLI-wide change feed would be a second mechanism. `cargo test --test payload_contracts` unchanged.

### Entity schema sync
No schema under `srs/docs/schema/2.0/` changes. `check-schema-sync.sh` unaffected.

---

## Scope

- Core recorder in `FileStore` (srs-repository): inside `vfs_write` / `vfs_remove` (so `write_json`, `delete_file` and the raw-content writes all pass through), push a `ChangeEntry` to a store-shared drain buffer (same `Rc` sharing as `epoch`, so McpSession's store clone sees it).
  - Classification by path prefix: `records/` -> instance (id = `instanceId` of the written JSON), `relations/` -> relation, `containers/` -> container (manifest root container writes via `save_manifest` -> container). Package/definition files are not instance changes and are ignored by default.
  - `created` = target path did not exist before the write; `updated` = it did; `deleted` = `delete_file` on a classified path (id read from the file before removal, or parsed from the path for relations).
  - Coalesce per (kind-class, id) within one request: created+updated -> created; any+deleted -> deleted (created then deleted -> dropped).
- Trait surface: `RepositoryStore::drain_changes(&self) -> Vec<ChangeEntry>` with a default empty impl (stores that cannot record return none, mirroring `session_actor`).
- Dispatcher/application (srs-mcp-core): `SrsMcpApplication` wraps `tools/call`: drain before, call, drain after; if non-empty, attach `{ tool, changed }` to the call result (D3). Failed/rejected writes leave nothing (rejection already does not bump the epoch).
- Bindings: `McpSession.take_write_summary()` (D3) returning JSON or `undefined`.
- Replace nothing in srs-web here; a follow-up srs-web issue swaps `writeFrom` for the summary (client work, separate PR).

**Out of scope:** `moved` kind and per-field detail (owner D2); manifest.json writes (root container identity changes are not reported in v1); persisted provenance / `updatedBy` (spec change); CLI change feed; per-field value diffs (names/ids only); stdio `srs mcp serve` differences beyond sharing the same application code; UI-originated writes through `SrsRepository` (could use the same drain; deferred); definition (package) changes.

---

## Phases

### Phase 1: Store recorder

**Goal:** every FileStore mutation of a record, relation or container is recorded exactly once and drainable.
**Agent:** Store/Core Worker

#### Tasks
- [ ] `ChangeKind` (`created|updated|deleted`) and `ChangeEntry { target: ChangeTarget(instance|relation|container), id, kind }` in srs-repository (serde camelCase; NO schemars, ADR-011).
- [ ] `FileStore`: shared `Rc<RefCell<Vec<..>>>` buffer cloned with the store (like `epoch`); record inside `write_json` / `delete_file` (the only sanctioned write sites, store.rs comment at "Catalog cache invalidation").
- [ ] `RepositoryStore::drain_changes` default `Vec::new()`; FileStore overrides; `MemoryStore` test double records too if it is the unit-test vehicle.
- [ ] Coalescing as above.
- [ ] Guard test: a source scan fails if `self.vfs.write(`, `.remove(` or `.create_dir_all(` appear in store.rs outside the three wrappers.
- [ ] Draft ADR-049 (status proposed).

#### Acceptance Criteria
- [ ] record create -> one `created` instance entry (+ `updated` container entry when filed into a container; relations as `created` relation entries).
- [ ] reads and rejected writes produce no entries; entries exist iff `write_epoch` moved.
- [ ] clones share the buffer.

#### Testing
- `store_records_create_update_delete` - kinds and coalescing.
- `drain_empty_when_epoch_unmoved` - invariant tying recorder to epoch.

#### Milestone gate
`cargo test -p srs-repository && cargo clippy -p srs-repository -- -D warnings`; tick boxes; commit `(#1202)`.

### Phase 2: MCP + WASM exposure

**Goal:** `McpSession` reports the summary for the last request.
**Agent:** MCP/Bindings Worker

#### Tasks
- [ ] `SrsMcpApplication::call("tools/call")`: attach summary per D3 using the already-parsed tool `name`; guard rejection path unchanged (no entries).
- [ ] `McpSession::take_write_summary(&mut self) -> Option<String>` (D3, drained = lifetime per request).
- [ ] Document in `srs-usage.md` MCP section if the result shape is agent-visible (D3 option B only).

#### Acceptance Criteria
- [ ] `record_update` summary: `{tool:"record_update",changed:[{instanceId,kind:"updated"}]}`.
- [ ] `container_copy` / `record_fork` list every instance they created.
- [ ] guard-rejected call: no summary.

#### Testing
- `crates/srs-bindings/tests/mcp_session.rs`: `write_summary_reports_each_tool`, `write_summary_absent_on_read_and_rejection`, `write_summary_drained_once`.
- `crates/srs-mcp-core/tests/surface.rs`: summary matches epoch movement.

#### Milestone gate
`cargo test -p srs-mcp-core -p srs-bindings && cargo clippy -- -D warnings`; commit `(#1202)`.

---

## Final Acceptance

- [ ] `cargo test` and `cargo clippy -- -D warnings` pass
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed under D5 recommendation)
- [ ] `check-schema-sync.sh` exits 0 (no schema change)
- [ ] No per-tool summary code exists (grep: recorder referenced only from FileStore)
- [ ] Follow-up srs-web issue filed and parented under epic #224 (replace `writeFrom` parsing)
- [ ] CLAUDE.md MCP/bindings rows note the new method

## Coordination Rules
- Keep write scopes; Lead Integrator owns type names. Concurrent srs-rust#1199 (WASM `updateContainer`) touches `srs-bindings/src/lib.rs`; rebase, trivial conflict only. A container write by `updateContainer` through `SrsRepository` also lands in the recorder buffer; it is drained only by McpSession, so drain on the repository handle is out of scope (see Out of scope).
- Milestone gate per phase as in the template.

## Assumptions
- Every record/relation/container mutation reaches `FileStore::write_json`/`delete_file` (store.rs documents these as the only sanctioned VFS write sites; verify in Phase 1 with a test that fails if `vfs.write` is called elsewhere).
- Instance id is readable from the written JSON (`instanceId`/`relationId`/`containerId`); relation file name is `<relationId>.json`.
- Single-threaded session (store uses `Rc`/`Cell`), so a plain `RefCell` buffer is sufficient.
- Interleaving: the drain-before/after per `tools/call` isolates requests; UI writes between requests are not attributed to the agent (they would be drained as stale before the call).

---

## Owner rulings (2026-10-03)

- D1 = A: record at the FileStore write seam (`vfs_write` / `vfs_remove`, the wrappers every mutation already goes through and that bump `write_epoch`). New ADR-049 "write recording is a store concern".
- D2: shape `{ tool, changed: [{ target: instance|relation|container, id, kind: created|updated|deleted }] }`. No `moved` (a container change is `updated`). Relations included as `target:"relation"`. No `fields` in v1.
- D3 = A: `McpSession.take_write_summary()`, WASM only; MCP wire unchanged.
- D4: per request, drained.
- D5: no CLI mirror.
