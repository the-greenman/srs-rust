# Plan: ContainerListFilter refactor (ADR-010)

## Summary

`container_service::list_containers` currently takes three positional `Option<&str>` parameters. ADR-010 requires list functions to accept a typed filter struct. This plan introduces `ContainerListFilter` — mirroring the `DocumentViewListFilter` precedent from #125 — and threads it through all callers: the service, `containers_for_instance`, the CLI handler, and the WASM binding.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| CLI Worker | — |
| Bindings Worker | — |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | List functions take a filter struct — this plan implements the rule | accepted |

No new ADRs needed — this plan directly implements an existing ADR-010 requirement.

---

## Contracts

### CLI output contract (ADR-011)

No new/changed commands. The `container list` payload struct and JSON output shape are unchanged. No schema regeneration required.

### Entity schema sync

No entity schema files are modified.

---

## Scope

- Add `pub struct ContainerListFilter` to `crates/srs-repository/src/container_service.rs`
- Change `list_containers` signature to accept `&ContainerListFilter` instead of three positional params
- Update `containers_for_instance` to build a `ContainerListFilter` and pass it
- Update CLI handler `cmd_list` in `crates/srs-cli/src/commands/container.rs`
- Update WASM binding `list_containers` in `crates/srs-bindings/src/lib.rs` to map `ContainerListBindingFilter` → `ContainerListFilter` (same pattern as `list_document_views`)
- Update all test call sites inside `container_service.rs`

**Out of scope:**

- Any change to CLI flags or output format
- Any change to WASM binding JSON interface shape
- Renaming `ContainerListBindingFilter` (it can stay; it is the serde adapter, `ContainerListFilter` is the service contract)

---

## Phases

### Phase 1: Introduce `ContainerListFilter` and update all callers

**Goal:** Every call site uses the filter struct; all tests pass.

**Agent:** Repository Service Worker + CLI Worker + Bindings Worker

#### Tasks

- [ ] Add `ContainerListFilter { container_type: Option<String>, member_instance_id: Option<String>, root_instance_id: Option<String> }` to `container_service.rs` — plain struct, no serde, `pub`, `#[derive(Debug, Clone, Default)]`
- [ ] Change `list_containers` to `pub fn list_containers(store: &dyn RepositoryStore, filter: &ContainerListFilter) -> Result<...>`; update the filter logic inside to use `filter.container_type`, `filter.member_instance_id`, `filter.root_instance_id`
- [ ] Update `containers_for_instance` to build `ContainerListFilter { member_instance_id: Some(instance_id.to_string()), ..Default::default() }` and call `list_containers(store, &filter)`
- [ ] Update all test call sites in `container_service.rs` (replace positional args with struct literal)
- [ ] Update CLI `cmd_list` in `container.rs` to build `ContainerListFilter` and pass it
- [ ] Update WASM binding: add `ContainerListFilter` to import, map `ContainerListBindingFilter` fields into it, pass `&filter` to `container_service::list_containers`

#### Acceptance Criteria

- [ ] `list_containers` signature takes `&ContainerListFilter`
- [ ] No positional `Option<&str>` args remain in `list_containers`
- [ ] CLI `container list --container-type X --member-instance-id Y` still filters correctly
- [ ] WASM binding JSON interface unchanged (same `containerType`/`memberInstanceId`/`rootInstanceId` keys)
- [ ] All existing tests pass unmodified (they only needed call-site updates)

#### Testing

```bash
cargo test -p srs-repository
cargo test -p srs-cli
cargo clippy -- -D warnings
```

Specific tests to verify:

- `list_containers_returns_all` — proves default filter returns all
- `list_containers_root_filter_matches_root_only` — proves root filter works

#### Milestone gate

1. Verify all acceptance criteria above.
2. Run:

```bash
cargo test -p srs-repository
cargo test -p srs-cli
cargo clippy -- -D warnings
```

3. Mark checkboxes `[x]` and commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] CLI output format unchanged
- [ ] WASM binding JSON interface unchanged

## Assumptions

- `ContainerListFilter` fields hold owned `String` (not `&str`) to match `DocumentViewListFilter` — callers convert from `Option<String>` via `Option<String>` directly, no `.as_deref()` needed at call sites.
