# Plan: Container membership writes must not brick the repository (#841)

## Summary

`container roots add` / `container members add` accept an instance id that resolves to
nothing — an empty string, a whitespace-only string, or a well-formed but non-existent
UUID — report `ok: true`, persist it, and thereby make the repository unloadable: the
`SRS038-R13-DANGLING-REFERENCE` diagnostic is fatal under [R24], so **every** subsequent
command (including the `remove` that would undo the damage) fails at catalog build. This
plan closes the write side (the id must resolve before it is persisted), closes the same
hole on the whole-list writers `container create` / `container update` (blank ids), and
opens a CLI recovery path so an already-bricked repository can be repaired without
hand-editing JSON.

**Empirical correction to the issue text.** #841 states that "non-existent-but-well-formed
UUIDs already fail correctly via the [R13] dangling-reference check". They do not — verified
on `master` (`50501c5`):

```console
$ srs container roots add --repo /tmp/df841 "$ROOT" "11111111-2222-3333-4444-555555555555"
{"ok":true,...,"rootInstanceIds":["11111111-2222-3333-4444-555555555555"]}
$ srs repo navigation --repo /tmp/df841
{"ok":false,"diagnostics":["catalog load failed: … SRS038-R13-DANGLING-REFERENCE: container rootInstanceIds '1111…' resolves to nothing in the instance set"]}
$ srs container roots remove --repo /tmp/df841 "$ROOT" "11111111-2222-3333-4444-555555555555"
{"ok":false,"diagnostics":["catalog load failed: …"]}          ← no way back out
```

A blank-id guard alone would therefore leave the reported failure class wide open. The guard
on the add path is consequently **"the id must resolve to an instance"**, of which blank is
the degenerate case.

`container create` was likewise confirmed to persist `rootInstanceIds: [""]` with `ok: true`
and brick navigation.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Repository Worker | Claude (this session) — `crates/srs-repository/**` |
| Core Worker | Claude (this session) — `crates/srs-core/src/validation/container.rs` |
| CLI Worker | Claude (this session) — `crates/srs-cli/tests/**` (tests only; no payload change) |
| Architecture Reviewer | subagent, Stage 3 + Stage 7 |
| Plan Reviewer | subagent, Stage 3 |
| Verification | subagent, Stage 7 |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Validation lives in the `srs-repository` service, never in the CLI handler — the guard must sit in `container_service` so CLI, MCP and WASM all inherit it | accepted (governs) |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Rejection surfaces as the ordinary `ok: false` envelope; no payload struct changes | accepted (governs) |
| [ADR-037](../docs/adr/037-mcp-adapter-surface.md) | MCP tool handlers call the same service functions as the CLI — the guard must not be added at either adapter | accepted (governs) |
| [ADR-041](../docs/adr/041-storage-backend-guardrails.md) | G1: path strings never appear in `srs-repository` service functions; G3/G4: typed logical-id currency, with the container as the worked example — so the unchecked repair seam lives in `store.rs`, not in `container_service.rs` | accepted (governs) |
| [ADR-042](../docs/adr/042-logical-id-instance-persistence.md) | Instance/container persistence goes through the typed logical-id store methods, never path + `Value` in a service — the repair helpers use `load_container_unchecked` / `save_container_unchecked`, not `load_instance_json` | accepted (governs) |
| [ADR-045](../docs/adr/045-membership-removal-is-a-repair-operation.md) | Container membership **removal** reads and writes through the unchecked catalog builder, so it can repair a repository whose catalog build is fatal under [R24] | **proposed (this plan)** |

ADR-045 is new because it establishes a second sanctioned [R24] exemption (the first being
`repo validate` / `validate_container_invariants`) and rejects the plausible alternative of
demoting the [R13] dangling-reference diagnostic to a warning. It reconciles itself explicitly
against ADR-041 G1/G3/G4 and ADR-042 by putting the unchecked seam beside the checked one in
`store.rs` — so no third path-string carve-out is needed.

No interop/positioning research consult applies: this plan changes no export/import format,
no CLI contract shape, and no agent-facing surface beyond making an already-documented
failure mode return `ok: false` instead of `ok: true`.

---

## Contracts

### CLI output contract (ADR-011)

**No new or changed commands, and no payload struct changes.** `container roots add`,
`container members add`, `container roots remove` and `container members remove` keep their
existing `ContainerRootsMutatePayload` / `ContainerMembersMutatePayload` shapes. The only
behavioural change is that an id that resolves to nothing now produces the standard error
envelope (`ok: false`, message in `diagnostics[]`) instead of a success envelope.
`schemas/payload/` is unchanged; `cargo run --bin generate-schemas` produces no diff.

### Entity schema sync (check-schema-sync.sh)

**No.** No file under `srs/docs/schema/2.0/` is added or modified. `container.json` already
types `rootInstanceIds`/`memberInstanceIds` as `string[]`; the "must resolve" rule is [R13]
in the spec, enforced here at write time rather than only at read time. No spec RFC required —
this is the reference implementation applying an existing spec rule earlier.

---

## Scope

- `crates/srs-core/src/validation/container.rs`: `validate_container` additionally rejects a
  blank (empty or whitespace-only) entry in `rootInstanceIds`, `memberInstanceIds`, or
  `identityInstanceId`. This is the single place `create_container` and `update_container`
  already route through, so both are covered for every consumer.
- `crates/srs-repository/src/container_service.rs`:
  - `add_member` and `add_root` reject a blank incoming `instance_id`
    (`RepositoryError::InvalidInput`) and an `instance_id` that resolves to no instance in
    the catalog (`RepositoryError::InstanceNotFound`), **before** any write.
  - `remove_member` and `remove_root` load and save the container through the **unchecked**
    catalog builder, via the new `store.rs` seam (`catalog_unchecked`,
    `load_container_unchecked`, `save_container_unchecked`), so they still work on a
    repository whose catalog build is fatal — the recovery path out of an already-bricked
    repository.
- `docs/adr/045-membership-removal-is-a-repair-operation.md`: new ADR (proposed).
- Tests at every layer: `srs-core` unit, `srs-repository` service unit, `srs-cli` integration
  (including a bricked-repo recovery round trip).
- `docs/dogfooding.md`: scenario covering the guard and the repair round trip.

**Out of scope:**

- Demoting or otherwise changing the severity of `SRS038-R13-DANGLING-REFERENCE` — that is a
  spec disposition ([R24]), not an implementation choice.
- Making `container roots list` / `container members list` (or any other read command) work
  on a bricked repository. `repo validate` already works there and names the offending
  container file and id, which is what a repair needs. Deferred as a follow-up.
- Requiring `create_container` / `update_container` ids to *resolve* (not just be non-blank).
  Whole-list container writes are used by scaffolding paths that legitimately write a
  container alongside the instances it names; ordering there is not guaranteed. Deferred as a
  follow-up.
- Any change to `.srsj` carrier behaviour beyond what it inherits from the shared service.

---

## Phases

### Phase 1: Core blank-id rule

**Goal:** `validate_container` rejects blank membership/identity ids, so `container create`
and `container update` can no longer persist one.

**Agent:** Core Worker

#### Tasks

- [x] In `crates/srs-core/src/validation/container.rs`, after the existing `title` check, add
      a check over `identity_instance_id` (when `Some`), `root_instance_ids` and
      `member_instance_ids`: any entry whose `.trim()` is empty returns
      `CoreError::InvalidFieldValue { key, reason: "instance id must not be blank" }` with
      `key` set to `identityInstanceId` / `rootInstanceIds` / `memberInstanceIds`.
- [x] Keep the check allocation-free and order-stable: identity first, then roots, then
      members (deterministic error for a container with several offenders).

#### Acceptance Criteria

- [x] A `Container` with `rootInstanceIds: [""]` fails `validate_container`.
- [x] A `Container` with `memberInstanceIds: ["   "]` fails `validate_container`.
- [x] A `Container` with `identityInstanceId: Some("")` fails `validate_container`.
- [x] The existing minimal container (all three fields `None`) still passes.
- [x] No other `srs-core` test regresses.

#### Testing

```bash
cargo test -p srs-core
```

- `validate_container_blank_root_instance_id_fails` — a blank root id is rejected
- `validate_container_whitespace_member_instance_id_fails` — whitespace-only is rejected
- `validate_container_blank_identity_instance_id_fails` — a blank identity id is rejected
- `validate_container_passes_minimal` (existing) — still passes

#### Milestone gate

```bash
cargo test -p srs-core
cargo clippy -p srs-core -- -D warnings
```

---

### Phase 2: Resolution guard on the add path

**Goal:** `add_member` / `add_root` cannot persist an id that resolves to nothing, for any
consumer (CLI, MCP, WASM bindings).

**Agent:** Repository Worker

#### Tasks

- [x] Add a private helper in `crates/srs-repository/src/container_service.rs`:
      `fn require_resolvable_instance(store: &dyn RepositoryStore, instance_id: &str) -> Result<(), RepositoryError>`.
      It returns `RepositoryError::InvalidInput { message: "instance_id must not be empty" }`
      when `instance_id.trim().is_empty()`, and `RepositoryError::InstanceNotFound { id }`
      when no entry in `store.catalog()?.instances` has that id.
- [x] Call it as the first statement of `add_member` and of `add_root`, before
      `load_container_with_embed_fallback` — no write may precede the check.
- [x] Do **not** add the check to `remove_member` / `remove_root` (removal must stay possible
      for an unresolvable id — that is the repair path) nor to any CLI/MCP/bindings adapter
      (ADR-010, ADR-037).
- [x] Update `okf_export_service.rs`'s `missing_instance_fails_the_export` test: it currently
      manufactures the dangling state with `add_member(&store, …, "does-not-exist")`, which is
      now correctly rejected. Rebuild the same state through `create_container` with
      `memberInstanceIds: ["does-not-exist"]` (still permitted — Phase 1 rejects only *blank*
      ids on that path). The test's assertion is unchanged.
- [x] Grep for any other test or service that relies on adding an unresolvable member and fix
      the same way: `rg -n "add_member\(|add_root\(|add_container_member\(" --glob '*.rs' crates/`.

#### Acceptance Criteria

- [x] `add_root` with `""` returns `InvalidInput` and writes nothing.
- [x] `add_root` with `"   "` returns `InvalidInput` and writes nothing.
- [x] `add_root` with a well-formed but non-existent UUID returns `InstanceNotFound` and
      writes nothing.
- [x] `add_member` behaves identically for all three inputs.
- [x] `add_member` / `add_root` with a real instance id still succeed and stay idempotent.
- [x] `create_note_in_context` / `create_record_in_context` (create-then-add-member
      orchestration) still succeed — the instance is persisted before `add_member` runs.
- [x] The container on disk is byte-identical after a rejected add.

#### Testing

```bash
cargo test -p srs-repository
cargo test -p srs-bindings
```

- `add_root_rejects_blank_instance_id` — `""` and `"   "` both rejected, container unchanged
- `add_root_rejects_unresolvable_instance_id` — non-existent UUID rejected, container unchanged
- `add_member_rejects_blank_instance_id` — same for members
- `add_member_rejects_unresolvable_instance_id` — same for members
- `add_root_succeeds_with_resolvable_instance_id` — a seeded instance id is accepted and the
  add stays idempotent (the guard rejects nothing it should not)
- `missing_instance_fails_the_export` (existing, `okf_export_service.rs`) — rebuilt via
  `create_container`, assertion unchanged
- `add_member_remove_member_round_trip` (existing, `srs-bindings/tests/containers.rs`) — still passes

#### Milestone gate

```bash
cargo test -p srs-repository
cargo test -p srs-bindings
cargo clippy -p srs-repository -- -D warnings
```

---

### Phase 3: Removal is a repair operation

**Goal:** `container roots remove` / `container members remove` work on a repository whose
catalog build is fatal, giving a CLI way out of an already-bricked repository.

**Agent:** Repository Worker

#### Tasks

- [x] Draft `docs/adr/045-membership-removal-is-a-repair-operation.md` from `ADR-TEMPLATE.md`
      (status `proposed`): removal reads/writes through `crate::catalog::build` (unchecked)
      rather than `store.catalog()` (checked, [R24]-fatal), because removal is the only
      operation that can *reduce* incoherence and can never introduce a dangling reference.
      Record the rejected alternative (demote `SRS038-R13-DANGLING-REFERENCE` to a warning —
      rejected: [R24]/[R13] severity is a spec disposition, and a warning would leave the
      brick permitted rather than repaired).
- [x] Add the unchecked seam to `crates/srs-repository/src/store.rs`, beside the checked one
      (ADR-041 G1 — path handling stays in the store, not the service):
      - `RepositoryStore::catalog_unchecked` — trait method mirroring `catalog`, defaulting to
        `CatalogUnsupported` and implemented as `catalog::build` on `FileStore`/`MemoryStore`.
      - `unchecked_file_container_locator` — `catalog_file_container_locator` over that builder.
      - `load_container_unchecked` / `save_container_unchecked` — trait defaults that differ
        from `load_container` / `save_container` *only* in which locator they pass to the
        newly extracted shared bodies `load_container_at` / `save_container_at`, so the checked
        and unchecked paths cannot drift.
- [x] Add two private helpers to `container_service.rs`, using those typed store methods
      (ADR-042 — no path + `Value` in a service function):
      - `fn load_container_for_repair(store, container_id) -> Result<(Container, bool), RepositoryError>`
        — `store.load_container_unchecked`, falling back on `ContainerNotFound` to
        `store.load_manifest()?.container` **read directly**. It must not call
        `resolve_root_container`, which routes through the *checked* `store.load_container`
        and would re-raise the very fatal error being repaired — and since a file-backed root
        is now a fatal [R12] duplicate-id, the embed-only root is the common shape and the one
        the #841 reproduction actually hits.
      - `fn save_container_for_repair(store, container, is_embed_only) -> Result<(), RepositoryError>`
        — `is_embed_only` ⇒ `write_manifest` with `manifest.container` replaced (same id-match
        guard as `save_container_syncing_embed`); else `store.save_container_unchecked`.
        Mirrors its checked counterpart's `sync_file_backed_root = false` behaviour.
- [x] Point `remove_member` and `remove_root` at the two repair helpers. Their return values,
      signatures and the empty-list→`None` normalisation are unchanged.
- [x] Document the exemption inline at both helpers, in the style of the existing
      `validate_container_invariants` comment, citing ADR-045.

#### Acceptance Criteria

- [x] Given a repository bricked by a dangling root id, `remove_root` succeeds and the
      repository loads again (`store.catalog()` builds without fatal diagnostics).
- [x] The same holds for a dangling member id via `remove_member`.
- [x] The same holds when the affected container is the `manifest.container` embed **and**
      when it is a file-backed container.
- [x] Removing a non-existent id from a healthy container is still a no-op returning the
      unchanged list.
- [x] No other command gains an [R24] exemption.

#### Testing

```bash
cargo test -p srs-repository
```

- `remove_root_repairs_bricked_embed_root_container` — brick the embed root, remove, assert
  `store.catalog()` succeeds again
- `remove_root_repairs_bricked_file_backed_container` — same for a file-backed container
- `remove_member_repairs_bricked_file_backed_container` — same via the member list
- `container_membership_unchanged` (existing) — ordinary add/remove round trip still passes

#### Milestone gate

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
# The [R24] exemption stays narrow: the unchecked builder is reachable only through the
# store seam and the two repair helpers.
rg -n "catalog_unchecked|catalog::build\(" crates/srs-repository/src --glob '!catalog.rs'
```

---

### Phase 4: CLI integration + end-to-end recovery

**Goal:** The guard and the recovery path are proven through the actual `srs` binary, at the
envelope level.

**Agent:** CLI Worker (tests only)

#### Tasks

- [x] Extend `crates/srs-cli/tests/integration_tests.rs` (near the existing
      `container_roots_add_list_remove` / `container_members_add_list_remove`) with a test that
      runs `container roots add <container> ""` and asserts `ok: false` and an unchanged
      container file.
- [x] Add a test for a non-existent well-formed UUID on both `roots add` and `members add`.
- [x] Add an end-to-end recovery test: brick a repository by writing a dangling root id
      directly into the container, assert `repo navigation` fails, run
      `container roots remove`, assert `repo navigation` then succeeds.
- [x] Confirm no `schemas/payload/` diff: `cargo run --bin generate-schemas && git diff --exit-code schemas/payload/`.

#### Acceptance Criteria

- [x] `container roots add <c> ""` exits with the `ok: false` envelope and a message naming
      the empty id.
- [x] `container members add <c> <random-uuid>` exits with the `ok: false` envelope and a
      message naming the unresolvable id.
- [x] The bricked→repaired round trip passes through the binary only — no hand-edited JSON.
- [x] `cargo test --test payload_contracts` passes; `schemas/payload/` has no diff.

#### Testing

```bash
cargo test -p srs --test integration_tests
cargo test --test payload_contracts
```

- `container_roots_add_rejects_blank_instance_id`
- `container_members_add_rejects_unresolvable_instance_id`
- `container_roots_remove_repairs_bricked_repository`

#### Milestone gate

```bash
cargo test
cargo clippy -- -D warnings
```

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] CLI output format unchanged (integration tests pass)
- [x] `cargo test --test payload_contracts` passes and `schemas/payload/` has no diff
- [x] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [x] The three inputs from the issue (`""`, `"   "`, non-existent UUID) are rejected on both
      `container roots add` and `container members add`
- [x] A repository bricked by a dangling container reference can be repaired entirely through
      the CLI, with no hand-edited JSON
- [x] ADR-045 exists, is cited at both repair helpers, and no other command gains an [R24]
      exemption
- [x] `docs/dogfooding.md` carries a scenario for the guard and the repair round trip

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist
  and pass, update the plan checkboxes, then commit.
- Verification Agent runs after Phase 4 and before final sign-off.

## Assumptions

- Instances are always persisted before `add_member` / `add_root` is called for them. Verified
  for the two in-tree orchestrations (`services::create_note_in_context`,
  `record_store::create_record_in_context`) — both save the instance, then add membership,
  with no enclosing batch that would hide the write from the catalog.
- `crate::catalog::build` (unchecked) is a sanctioned seam for repair-class operations; the
  precedent is `container_service::validate_container_invariants`, which already uses it with
  a documented [R24] rationale.
- Rejecting a blank id inside `validate_container` cannot break an existing healthy
  repository: a container carrying a blank id is already fatally invalid at catalog build,
  so no loadable repository can contain one.
- `repo validate` remains usable on a bricked repository (confirmed on `master`) and is the
  discovery half of the repair workflow.
