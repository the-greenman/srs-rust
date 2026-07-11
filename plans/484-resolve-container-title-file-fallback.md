# Plan: Fix resolve_container_title to fall back to container files when containerIndex is absent

## Summary

`render_service.rs::resolve_container_title` resolves the `{{container-title}}` template variable only from `manifest.containerIndex`. When the manifest has no containerIndex (a common state for repos without it), a requested `container_id` is never looked up in the actual container files, and the function falls through to the repo-level title. This produces incorrect output (e.g. "muDemocracy" instead of "Recognising decisions") even when the container file carries the correct title. The fix adds `store: &dyn RepositoryStore` as a parameter to `resolve_container_title` and inserts a `store.load_container(cid)` call as a fallback when the container ID is not found in (or when there is no) containerIndex. Fixes #484.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude |
| Repository Service Worker | Claude |
| Verification | Claude |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Fix stays in `srs-repository`; no business logic in CLI | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `resolve_container_title` is a private helper in `render_service.rs`; adding store param is an internal refactor, not a public service contract change | accepted |

No new ADRs required — the fix follows existing patterns: service logic consults the store directly.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. `render document-view` output shape is unchanged — the template variable `{{container-title}}` resolves to a more correct title in the bug scenario, but the payload struct is identical. No `payload.rs` change required.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files. Not applicable.

---

## Scope

- Fix `resolve_container_title` in `crates/srs-repository/src/render_service.rs` to accept `store: &dyn RepositoryStore` and fall back to `store.load_container(cid)` when the container ID is not found in containerIndex or when containerIndex is absent.
- Update the single call site at line 142 to pass `opts.store`.
- Add a targeted unit test covering: (a) container title resolved from containerIndex when present, (b) container title resolved from container file when containerIndex is absent, (c) falls back to manifest/repo title when both containerIndex and container file are absent.

**Out of scope:**
- Fixing the containerType heuristic fallback path (no bug reported there).
- Issue #466 (JsonStore container lookup ignores containerIndex — opposite direction, separate issue).
- Any changes to `srs-cli`, `srs-bindings`, or `srs-core`.

---

## Phases

### Phase 1: Fix resolve_container_title and add tests

**Goal:** `resolve_container_title` correctly falls back to `store.load_container()` when containerIndex does not contain the requested container ID, and is covered by a unit test.

**Agent:** Repository Service Worker

#### Tasks

- [x] In `crates/srs-repository/src/render_service.rs`:
  - [x] Add `store: &dyn RepositoryStore` parameter to `resolve_container_title` (line 921)
  - [x] After the per-ID containerIndex scan fails (i.e. the requested cid was not found with a non-empty title), insert: `if let Ok(c) = store.load_container(cid) { if !c.title.is_empty() { return c.title; } }`
  - [x] Update the call site at line 142: `resolve_container_title(opts.store, &dv, &manifest, opts.container_id)`
- [x] In the `#[cfg(test)]` block inside `render_service.rs`, add test `resolve_container_title_falls_back_to_container_file`:
  - Uses a `MemoryStore` with a container whose title is set but whose ID is NOT in manifest.containerIndex (containerIndex is empty)
  - Asserts that the returned title equals the container's file title (not the repo-level fallback)
- [x] Optionally add `resolve_container_title_uses_index_when_present`: uses a store with the container ID in containerIndex and asserts the index title wins

#### Acceptance Criteria

- [x] `resolve_container_title` with a `Some(cid)` that matches a container file (but is not in containerIndex) returns the container's title, not the repo title
- [x] `resolve_container_title` with a `Some(cid)` that IS in containerIndex still returns the index title (regression check)
- [x] `resolve_container_title` with no container ID and no index returns the repo title (regression check)
- [x] All existing `render_service` tests continue to pass

#### Testing

```bash
cargo test -p srs-repository resolve_container_title
cargo test -p srs-repository render
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:
- `resolve_container_title_falls_back_to_container_file` — proves the bug is fixed
- `resolve_container_title_uses_index_when_present` — regression check

#### Milestone gate

1. All acceptance criteria above are checked.
2. Both named tests exist and pass.
3. All render_service tests pass.
4. Clippy passes.
5. Commit: `fix(repository): resolve_container_title falls back to container file when index absent (#484)`

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (payload structs unchanged)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `resolve_container_title_falls_back_to_container_file` test exists and passes
- [ ] Full render document-view output with a container that has no containerIndex entry renders the container's title, not the repo title

## Coordination Rules

- Lead Integrator owns this single-phase plan end to end.
- No other crates are modified.

## Assumptions

- `MemoryStore` supports `load_container` / `save_container` (confirmed from store.rs line 1556+).
- `Container.title` is a plain `String` with `#[serde(default)]`; an empty string is treated as "no title" for fallback purposes.
- The `store.load_container()` call returns `ContainerNotFound` if the ID is truly unknown; `.ok()` / graceful handling means the function will continue to subsequent fallbacks rather than returning an error.
