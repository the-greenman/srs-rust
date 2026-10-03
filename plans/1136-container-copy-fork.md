# Plan: Container copy + record fork services (CLI / MCP / WASM)

> Owner rulings applied 2026-10-03 (D1-D6 + anchor). Issue the-greenman/srs-rust#1136. Serves muDemocracy.org#228 (S4), epic #224 ruling 6. Branch `feat/1136-container-copy-fork`. Decisions marked **[D#]** are open for owner input; the recommended option is written into the tasks and flips only if the owner rules otherwise.

## Summary

The essay editor needs many-document operations without duplicating content. (1) **Container copy** creates a new container that *shares* the original's member records, with the arrangement (ordered depth outline) copied: zero records created. (2) **Record fork** ("make local copy") clones one arrangement subtree of ONE container: for the chosen entry and its descendants it creates new records (same type, same field values, fresh ids, session-actor `createdBy`), asserts `derived-from` (fork -> original) and swaps the forks into that one container in place; every other container keeps the originals. Both are single services in `srs-repository` with thin CLI / MCP / WASM adapters (ADR-001/010/011/013/037, capability-layering). Fork composes the existing `create_record_successor` (it already creates a same-type record plus a relation to the predecessor with any installed relation type) and a new in-place entry swap in `container_service`; no second clone mechanism.

## Spec gate (Stage 1.5)

**No spec change required.** `derived-from` is a canonical core relation type ("source is the derived work; target is the source material"). Container `memberInstanceIds` as a shared flat outline (RFC-043) already permits one record in many containers; `childContainerIds` is already a DAG (multiple parents allowed). `createdBy` is RFC-046 and stamped by `actor_service::creation_actor`. A search of `srs` (spec records, `docs/spec`, schemas) finds only *definition-level* lineage (`lineage` / `forkedFromVersion`, RFC-014 package layer); there is no instance-level clone/fork/copy operation, so this is an implementation-side operation, not a ruling the impl would make (ADR-048 rule 1). RFC-014 record-layer clone/diff/reintegrate (muDemocracy #62/#63) stays a distinct layer: this plan adds neither upstream tracking nor reintegration. If D3 is answered "copy other relations" or a new `forked-from` relation is wanted, re-open this gate (the latter needs an RFC).

Charter / ADR-048 check: Layer test: one core service, adapters expose only. One-way-per-goal: fork reuses `create_record_successor` + `relation_service`; copy reuses `get_container`/`create_container`. Parity named: payload schemas (ADR-011), srs-vscode payload contract sync (new commands; file tracking issue), srs-web consumes via WASM (S4 client work, outside this plan). Decision mode: complicated (rules apply).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Opus session (owns naming, D1-D6) |
| Repository Worker | Sonnet: `container_service` copy + swap, new `fork_service` |
| CLI Worker | Sonnet: `container copy`, `record fork`, payloads, golden schemas |
| MCP Adapter Worker | Sonnet: tools in `srs-mcp-core` (+ guard rule) |
| Bindings Worker | Sonnet: WASM methods in `srs-bindings` |
| Verification | Haiku: parity test across CLI / MCP / WASM, gates |

See [agents.md](agents.md). No new role needed (all crates covered).

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| ADR-001 / ADR-010 | Logic in a `srs-repository` service, typed in/out | accepted |
| ADR-011 | New payload structs + golden schemas for the two CLI commands | accepted |
| ADR-013 / ADR-015 | WASM binding is a thin adapter over the same service | accepted |
| ADR-037 | MCP tools are an adapter surface over services (`srs-mcp-core`) | accepted |
| ADR-024 | Multi-write services are best-effort rollback; fork wraps writes in `begin_batch`/`commit_batch` like `delete_container`, and rolls back created forks on failure | accepted |
| ADR-042 | Logical-id instance persistence: forks are new ids; no path-keyed assumptions | accepted |
| ADR-045 | Membership removal is a repair op: fork swaps entries in place, never remove+add | accepted |
| ADR-048 | Spec-first / layer / one-way / parity / decision-mode rules | accepted |
| (none) | No new ADR: D1-D6 and the anchor ruling are recorded in this plan; owner picked the defaults | n/a |

Positioning register (`alignment-opportunities.md`): not consulted in depth; the plan touches agent-facing surfaces (MCP) but adds no interop/export format. Re-check at Stage 2.3 of execution.

## Design decisions (owner-ruled 2026-10-03)

- **D1** names: CLI `container copy`, `record fork <id> --container <cid>`; MCP `container_copy`, `record_fork`; WASM `copy_container`, `fork_record`.
- **D2** copy shares `childContainerIds` by reference, no recursion. Optional deep copy filed as a follow-up.
- **D3** a fork carries only its one `derived-from` edge.
- **D4** core creates no state/Composition records (srs-web#331 decides essay document-state).
- **ANCHOR** copy FORKS the anchor (title) record through the same fork core (derived-from edge, fresh id, session `createdBy`); the new container's `anchorInstanceId`, its outline entry, and `identityInstanceId` (if it named the anchor) point at the fork. Copy therefore creates exactly one record + one relation; "no record duplicated" means no MEMBER record duplicated. This removes the `sectionContainerId` ambiguity (two containers never share an anchor).
- **D5** fresh ids; results return old->new pairs. **D6** `container` required; atomic in-place swap.
- v1: Notes refused; anchor/identity not forkable via `record fork`; fieldValues only.

---

## Contracts

### CLI output contract (ADR-011)

Two new commands: `container copy` -> `ContainerCopyPayload { container, sharedMemberCount }`; `record fork` -> `RecordForkPayload { containerId, forks: [{ originalId, forkId }], relations: [Relation] }`. Add structs to `crates/srs-cli/src/payload.rs`, use `output::serialize()`, run `cargo run --bin generate-schemas`, commit `schemas/payload/container-copy.json` and `record-fork.json`. `cargo test --test payload_contracts` must pass. Register in `generate-schemas.rs`.

### Entity schema sync

No change to `srs/docs/schema/2.0/`. No mirror work.

---

## Scope

- `container_service::copy_container(store, source_id, ContainerCopyInput { title?, container_id? }) -> Container`: load source, new id (`new_instance_id()` unless given; existing id refused per #1167 via `create_container`), clone fields, `memberInstanceIds` cloned verbatim, `childContainerIds` cloned (shared), then `create_container`. Refuses the repository root container (`is_root_container`, same stance as #742). Creates exactly one record (the forked anchor, via `fork_service::fork_records`) and one `derived-from` relation; no member record is duplicated. If the source has no anchor it creates none.
- `container_service::replace_members(store, container_id, &[(old, new)])`: rewrite `instanceId` in place, keep order and depth, refuse an old id that is not a member or is the anchor/identity pointer (`require_not_pointer`), validate via the existing arrangement validation, one `save`.
- `fork_service::fork_records(store, ids) -> Vec<ForkPair>` (THE one fork core: for each id, `create_record_successor` with `derived-from`, in order, rolling back all created forks on failure) and `fork_service::fork_subtree(store, container_id, root_instance_id) -> ForkResult`, which calls it: take the entry's run from the container arrangement (`arrangement` module: entry plus following entries of greater depth); for each: `record_store::create_record_successor(store, original, { relation_type: "derived-from", field_values: original.field_values.clone(), lifecycle_state: None, type_version: None, extra: {} })` (reuses actor stamping, relation validation and rollback); then `replace_members`; `copy_container` calls the same `fork_records` for the anchor; batch-wrapped; on failure delete forks created so far (ADR-024). Forks are Tier 2 records only; a Note (Tier 0) in the run is refused with a clear error in v1.
- Adapters: CLI commands; MCP tools `container_copy`, `record_fork` in `srs-mcp-core/src/tools.rs`; WASM methods in `srs-bindings/src/lib.rs`. All hand the request through `actor_service::reject_supplied_created_by` first.
- Session write guard (`srs-mcp-core/src/guard.rs`): `record_fork` is checked as a write to the target container (rejected if `container_id` guarded) and `container_copy` is allowed on a guarded source (it only reads it) but rejected if the requested new id is guarded. Fork never modifies a guarded record (it creates new ones), consistent with "adding a guarded record to an unguarded container is allowed".
- Docs: add both commands to `srs/srs-usage.md` via a tracking issue (spec repo, not edited from here).

**Out of scope:**

- Recursive deep copy of child containers (D2 alternative), copying relations/comments/evidence (D3 alternative), Composition copy.
- Fork of Tier 0 notes; fork across containers; "reintegrate / diff" (RFC-014 record layer, muDemocracy #62/#63).
- Shared-element badge (`containers_for_instance` already exists; srs-web presentation, S4 client issue).
- Dispatching a fork of the anchor/identity record (refused in v1).

---

## Phases

### Phase 1: Core services

**Goal:** `copy_container`, `replace_members`, `fork_subtree` exist with tests; no adapter yet.

**Agent:** Repository Worker

#### Tasks

- [ ] `copy_container` + `ContainerCopyInput` (serde `deny_unknown_fields`, `mcp-schema` derive like `ContainerCreateInput`) in `crates/srs-repository/src/container_service.rs`
- [ ] `replace_members` in the same file (in place, depth-preserving, pointer guard)
- [ ] `crates/srs-repository/src/fork_service.rs` with `fork_subtree`, `ForkResult { container_id, forks: Vec<ForkPair>, relations }`; export in `lib.rs`
- [ ] Batch + best-effort rollback per ADR-024

#### Acceptance Criteria

- [ ] Copy: record count grows by exactly 1 (the anchor fork); both containers list the same non-anchor ids in the same order and depths; new container's anchor/outline entry is the fork; both containers list the same ids in the same order and depths; `containers_for_instance(member)` returns both
- [ ] Copy of the root container refused; copy to an existing id refused
- [ ] Fork: entry plus descendants replaced in place in the one container; other containers still hold originals; each fork has exactly one outgoing `derived-from` to its original and same `typeId@typeVersion` and `fieldValues`; lifecycle state is the type's initial state, not the original's
- [ ] Fork stamps `createdBy` from the session actor; original's `createdBy` is not copied; a request carrying `createdBy` is `actor-supplied`
- [ ] Forking the anchor/identity entry is refused; failure mid-run leaves no new records or relations

#### Testing

```bash
cargo test -p srs-repository container_copy fork_service
```

- `copy_creates_only_anchor_fork` - the "no member record duplicated" acceptance
- `fork_swaps_only_in_target_container`
- `fork_subtree_preserves_order_and_depth`
- `fork_stamps_session_actor_not_original`
- `fork_failure_rolls_back`

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Mark checkboxes, commit `feat(repository): container copy + record fork services (#1136)`.

### Phase 2: Adapters (CLI, MCP, WASM) and parity

**Goal:** Same semantics through all three surfaces, guard-aware, contracts committed.

**Agents:** CLI Worker, MCP Adapter Worker, Bindings Worker (parallel; disjoint scopes), then Verification

#### Tasks

- [ ] CLI: `ContainerCommand::Copy`, `RecordCommand::Fork`; handlers delegate only; payload structs; `generate-schemas`
- [ ] MCP: `container_copy`, `record_fork` tool constants, input structs (schemars), dispatch in `srs-mcp-core`; guard rule in `guard.rs`; update `tests/surface.rs` expected tool list
- [ ] WASM: `copy_container`, `fork_record` in `srs-bindings`, epoch advance like `create_container`
- [ ] Parity test: one fixture, same request through service / CLI / MCP / WASM yields identical container and fork pairs (id-normalised)
- [ ] Guard test: guarded container rejects `record_fork`; unguarded passes

#### Acceptance Criteria

- [ ] `cargo test --test payload_contracts` passes with the two new schemas
- [ ] MCP surface test lists the new tools; `srs mcp serve` and `open_mcp_session` both expose them
- [ ] Parity test passes

#### Testing

```bash
cargo test -p srs-cli -p srs-mcp-core -p srs-bindings
cargo test --test payload_contracts
```

#### Milestone gate

```bash
cargo test
cargo clippy -- -D warnings
```

Commit `feat(cli,mcp,wasm): container copy and record fork adapters (#1136)`. File tracking issues (parented under epic #224 / story #228): srs-vscode payload sync; `srs-usage.md` update; srs-web S4 client wiring (badge + "make local copy").

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format of existing commands unchanged
- [ ] `cargo test --test payload_contracts` passes
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] Copy duplicates no member record (test-proven); fork swaps into exactly one container
- [ ] Same semantics via CLI, MCP and WASM (parity test)
- [ ] `repo validate` on the fixture after copy and after fork: 0 errors

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary.
- Lead Integrator owns naming (D1) and dependency direction (repository <- cli/mcp-core/bindings).
- End of each phase: verify criteria, confirm tests exist, tick the plan, commit.

## Assumptions

- `create_record_successor` accepts `relation_type: "derived-from"` (relation-type validation is by installed definition, not a successor-only whitelist). Verify first in Phase 1; if it hard-codes supersedes/refines, factor the shared "create record + relation to X" core out of it rather than duplicating (one way per goal).
- `arrangement` module exposes run extraction (used by `move_member`); reuse it.
- Rev-8 corpora only (container entries); a rev-7 corpus is refused by existing loading rules.
- Tags and `meta` on the original record are not copied by fork in v1 (fieldValues only); revisit with D3.
