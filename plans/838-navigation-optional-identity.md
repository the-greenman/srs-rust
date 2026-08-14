# Plan: Navigation identity is optional, never inferred (#838)

## Summary

`repository_navigation` currently guarantees a non-optional `identity: NavigationNode` in its
payload. When the root container carries no `identityInstanceId`, the service manufactures one by
promoting the first `rootInstanceIds` entry — and then, because navigation excludes the identity
from `sections` (RFC-013 Change B), that ordinary section record simultaneously disappears from
navigation. No diagnostic is emitted. RFC-029 (line 104) is explicit that a root container with no
`identityInstanceId` is **valid**, so this is a reachable, supported state being silently
misrepresented rather than a corrupt one. srs-rust#834's delete cascade — which clears
`identityInstanceId` when the record it names is deleted — makes the state substantially easier to
reach, but the fallback predates it and fires for any identity-less root container. This plan makes
`identity` optional, drops the fallback entirely, and emits a diagnostic naming the absence.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Lead Integrator |
| Repository Worker | Repository Worker |
| Bindings Worker | Bindings Worker |
| CLI Worker | CLI Worker |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions. No new role is required — the change is confined to
one `srs-repository` service plus its pass-through adapters, all of which existing roles cover. The
**Bindings Worker** is assigned because `crates/srs-bindings/tests/navigation.rs` asserts on the
service struct directly; its `agents.md` write scope (`crates/srs-bindings/tests/**`) already covers
that file, so no scope expansion is needed.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-044](../docs/adr/044-navigation-identity-optional-never-inferred.md) | A derived payload field with no source in the data is absent + diagnosed, never inferred from an unrelated record | proposed |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Service returns a typed result struct; all logic in `srs-repository` | accepted (governs) |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Payload shape is a named struct in `payload.rs`; golden schema is the contract-change record | accepted (governs) |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | Bindings serialize the same service struct as the CLI — no separate shape | accepted (governs) |
| [ADR-037](../docs/adr/037-mcp-adapter-surface.md) | MCP resources serialize the service result; no adapter-side semantics | accepted (governs) |

**Owner decision (2026-08-14, Stage 2 checkpoint):** make `identity` optional and drop the fallback.
Rationale as given: *"The identity node is handy, and it makes the repository more navigable, but a
container should also be able to stand alone."* Options 2 (keep fallback + diagnostic) and 3 (keep
fallback, keep in sections) were presented and rejected — both continue to present a record as the
repository's identity when it never claimed to be one.

**Positioning research consult:** `srs/docs/research/alignment-opportunities.md` was read. No
register entry bears on this change — it is a truthfulness fix to an existing payload field, not an
interop, export-format, or portability decision. No entry is contradicted.

---

## Contracts

### CLI output contract (ADR-011)

**Existing command payload changed.** `RepositoryNavigation.identity` goes from
`NavigationNode` to `Option<NavigationNode>` with `#[serde(skip_serializing_if = "Option::is_none")]`.

`RepoNavigationPayload` in `crates/srs-cli/src/payload.rs` embeds `RepositoryNavigation` opaquely
(the committed golden `schemas/payload/repo-navigation.json` renders it as `"navigation": true`,
per ADR-011's "external service types are serialized as `serde_json::Value`" rule), so the golden
schema file is expected to be **unchanged**. `cargo run --bin generate-schemas` is still run to
confirm this, and `cargo test --test payload_contracts` must pass.

The JSON-level change is: when absent, the `identity` key is omitted from the `navigation` object
entirely (rather than emitted as an empty-string node). Known consumers:

- `srs-web/src/lib/srs-client.ts:1388` — `normalizeNavigationNode(raw.identity ?? {})`; already
  tolerant of an absent key. No change required in this PR.
- `srs-vscode` — no consumer of `navigation.identity` (grepped; only unrelated `.identity-link` CSS).
- `srs-mcp` `srs://<id>/navigation` resource — serializes the service struct verbatim; inherits the
  change with no adapter edit.
- `srs-bindings::repository_navigation` — same; serializes the service struct verbatim. Its **tests**
  do read the struct directly and must be updated (see Phase 2).
- `crates/srs-repository/src/agent_index_service.rs` — calls `repository_navigation()` but reads only
  `.sections`. Unaffected.
- `crates/srs-cli/tests/integration_tests.rs:276` — reads `navigation.identity.displayLabel` via JSON
  indexing against an identity-present fixture. Unaffected (the key is still present in that case).
- **`crates/srs-gov`** — an in-workspace consumer that reads this exact field from the CLI's JSON
  output, not through the typed struct. Two sites, **both already safe**:
  - `src/tui_data.rs:13-16` — `navigation["navigation"]["identity"]["displayLabel"].as_str().filter(|s| !s.is_empty()).unwrap_or("Governance")`
  - `src/main.rs:287-289` — `nav["identity"]["displayLabel"].as_str().unwrap_or("(untitled)")`

  A missing key JSON-indexes to `Value::Null`, so `.as_str()` yields `None` and both fall through to
  their literal defaults. No change required. Worth noting that `tui_data.rs` already filters the
  empty string — it was *already* working around the fabricated empty node this plan removes, which
  is precisely the presentation-layer fallback ADR-044 argues is the correct place for one.

### Entity schema sync (check-schema-sync.sh)

**No.** No files under `srs/docs/schema/2.0/` are added or modified. `identityInstanceId` is already
optional in `manifest.json` and `container.json` (RFC-029 line 104) — this plan aligns the
implementation with a schema that already permits absence.

---

## Scope

- `crates/srs-repository/src/repository_navigation_service.rs`:
  - `RepositoryNavigation.identity` → `Option<NavigationNode>` with `skip_serializing_if`.
  - Remove the `.or_else(|| root_instance_ids.first())` fallback and the subsequent
    `.ok_or_else(RepositoryError::NotFound)` — an absent `identityInstanceId` is no longer an error
    and no longer inferred.
  - Emit a diagnostic when `identityInstanceId` is absent.
  - Section-exclusion guard becomes identity-aware of `None` (every root stays in `sections`).
  - `manifest.container`-absent early return returns `identity: None` instead of
    `NavigationNode::default()` (the same fabrication in its other form).
- Unit tests in the same file's `mod tests` covering the new behaviour.
- Doc updates for the changed payload shape (Stage 7.5) and the dogfood scenario (Stage 7.6).

**Out of scope:**

- Changing `repo create` / `repo set-root-container` to require an identity. RFC-029 R6 already
  requires `repo create` to set one; this plan does not touch creation paths.
- Any validation-severity change. `srs repo validate` behaviour for an absent `identityInstanceId`
  is unchanged (it is valid — RFC-029 line 104).
- Updating `srs-web`'s client. Verified: `normalizeRepositoryNavigation` already reads
  `raw.identity ?? {}` (`src/lib/srs-client.ts:1388`) and **nothing downstream consumes
  `navigation.identity`** — grep across `srs-web/src` finds no other reader. So there is no visible
  regression. Its `RepositoryNavigation` TypeScript type does still declare `identity` as
  non-optional, which becomes untrue after this change; that type-honesty fix is filed separately
  (see Stage 3.4 follow-ups) rather than reached into from this repo.
- srs-rust#837 (deleting an `identityInstanceId` target silently unlinks the identity) — adjacent,
  separately tracked.

---

## Phases

### Phase 1: Optional identity in the navigation service

**Goal:** `repository_navigation` returns `identity: None` plus a diagnostic for an identity-less
root container, with every root present in `sections`; no code path infers an identity.

**Agent:** Repository Worker

#### Tasks

All line numbers below refer to `crates/srs-repository/src/repository_navigation_service.rs` at the
branch base **`b37f1d5`** (`origin/master`, the srs-rust#834 cascade fix). The service body
(lines 35–158) is unchanged from the earlier review base; the test module shifted by ~9–19 lines,
and the numbers below are re-verified against `b37f1d5`.

- [x] Change `RepositoryNavigation.identity` (line 35) to `Option<NavigationNode>` and add
      `#[serde(skip_serializing_if = "Option::is_none")]`.
- [x] Replace the `identity_id` resolution (**lines 60–71**): remove the
      `.or_else(|| root_instance_ids.first())` fallback and the
      `.ok_or_else(|| RepositoryError::NotFound { path: "manifest.container.identityInstanceId" })`.
      Bind instead `let identity_id: Option<String> = container_ref.identity_instance_id.clone();`
      — no fallback, no error.
- [x] Wrap the identity-node construction (**lines 76–112**) in `if let Some(identity_id)` so it
      produces `Option<NavigationNode>`. **Both** paths must sit inside the guard: the Tier-0 grace
      branch (**lines 78–99**) and the ordinary `get_record_by_id` branch (**lines 100–111**).
- [x] When `identity_id` is `None`, push a diagnostic. Note there is **no local `container_id`
      binding** in this function — the id is `container_ref.container_id` (as used at line 153), so
      bind it first or interpolate the field explicitly. Write it as:

      ```rust
      let container_id = &container_ref.container_id;
      diagnostics.push(format!(
          "repository-navigation: root container {container_id} has no identityInstanceId; \
           no repository identity node (RFC-029 permits this) - set one with `repo set-root-container`"
      ));
      ```
- [x] Change the section-exclusion guard (**line 126**) from `if id == &identity_id` to
      `if identity_id.as_deref() == Some(id.as_str())`.
- [x] Change the `manifest.container`-absent early return (**lines 44–54**) to return
      `identity: None` instead of `identity: NavigationNode::default()`.
- [x] Update the existing test
      `repository_navigation_missing_manifest_container_returns_empty_with_diagnostic`
      (**line 541**) — it asserts `nav.identity.instance_id == ""` at **line 546**; change to
      `assert!(nav.identity.is_none())`.
- [x] Update the **five** remaining tests in this module that read `nav.identity.<field>` and will
      otherwise fail to compile, unwrapping the `Option` via `.as_ref().expect("identity present")`:
  - [x] `repository_navigation_returns_identity_and_precedes_ordered_sections` (line 434; reads at 439, 442)
  - [x] `repository_navigation_resolves_embed_only_root_container` (line 463; reads at 521, 524)
  - [x] `navigation_tier0_note_identity_returns_diagnostic` (line 604; reads at 609, 612)
  - [x] `navigation_tier0_note_identity_no_title_falls_back_to_id` (line 619; reads at 624, 628)
  - [x] `repository_navigation_root_instance_ids_only_yields_same_sections` (line 709; reads at 778)

  These six (the five above plus the container-absent test) are the complete set — verified by
  grepping every `nav.identity` read in the module at `b37f1d5`.
  `repository_navigation_prefers_materialised_container_over_embed` (531),
  `repository_navigation_root_is_member_of_its_own_sub_container` (643), and
  `repository_navigation_union_deduplicates_ids_in_both_arrays` (797) do not read identity and need
  no change.

#### Acceptance Criteria

- [x] A root container with `identityInstanceId: None` and two roots yields `identity: None`,
      `sections.len() == 2` (both roots, precedes-ordered), and exactly one diagnostic naming the
      absent `identityInstanceId`.
- [x] A root container with a valid `identityInstanceId` is unchanged: `identity` is `Some`, the
      identity record is excluded from `sections`, and no new diagnostic appears.
- [x] The Tier-0-note grace diagnostic path still fires unchanged when `identityInstanceId` is
      present and points at a note.
- [x] `manifest.container` absent yields `identity: None` and its existing diagnostic.
- [x] No `RepositoryError::NotFound { path: "manifest.container.identityInstanceId" }` is reachable —
      the constructor is removed.
- [x] Serialized JSON omits the `identity` key when `None` (asserted on `serde_json::to_value`).

#### Testing

```bash
cargo test -p srs-repository repository_navigation
cargo test -p srs-repository
```

Specific tests to write or verify:

- `navigation_absent_identity_keeps_all_roots_as_sections` (new) — an identity-less root container
  with two roots: `identity` is `None`, both roots appear in `sections`, one diagnostic. This is the
  regression test for #838.
- `navigation_absent_identity_omits_identity_key_in_json` (new) — `serde_json::to_value(&nav)`
  has no `identity` key when `None`. Locks the wire shape ADR-044 promises.
- `repository_navigation_returns_identity_and_precedes_ordered_sections` (existing, updated) —
  proves the happy path is untouched.
- `navigation_tier0_note_identity_returns_diagnostic` (existing, updated) — proves the grace path
  is untouched.
- `repository_navigation_missing_manifest_container_returns_empty_with_diagnostic` (existing,
  updated — line 553) — proves the container-absent branch now returns `None` rather than an empty
  node.

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit: `fix(navigation): identity is optional, never inferred from the first root (#838)`

---

### Phase 2: Adapter surfaces and workspace green

**Goal:** CLI, bindings, and MCP compile and behave against the new `Option`, with the golden payload
schema confirmed unchanged and the whole workspace green.

**Agent:** Bindings Worker (`crates/srs-bindings/tests/**`) and CLI Worker, with Lead Integrator for
cross-crate call sites.

#### Tasks

- [x] **Bindings Worker:** update `crates/srs-bindings/tests/navigation.rs`, which calls
      `repository_navigation()` and asserts on the struct directly. Four sites break:
  - [x] lines **127–128** — `nav.identity.instance_id` / `nav.identity.display_label`; unwrap the `Option`.
  - [x] lines **262–263** — same; unwrap.
  - [x] lines **287–288** — same; unwrap.
  - [x] line **423** — `assert_eq!(nav.identity.instance_id, ""); // NavigationNode::default(), not null`.
        This is the `manifest.container`-absent assertion ADR-044 retires. Replace with
        `assert!(nav.identity.is_none())` and update the trailing comment, which now states the
        opposite of the intended behaviour.
- [x] `cargo build --workspace` and fix any remaining call site broken by the `Option`.
- [x] Run `cargo run --bin generate-schemas` and confirm `git diff --stat
      crates/srs-cli/schemas/payload/` is **empty** (the embed is opaque). If it is not empty, stage
      the regenerated file — the diff is the contract-change record per ADR-011.
- [x] Grep the **whole workspace** (`rg -n '\.identity\b|\["identity"\]' crates/`) for any remaining
      reader and confirm each is handled. Expected outcome: `integration_tests.rs:276` and both
      `srs-gov` sites pass unchanged; confirm by running them rather than assuming.

#### Acceptance Criteria

- [x] `cargo build --workspace` succeeds.
- [x] `cargo test` passes across the workspace.
- [x] `cargo test --test payload_contracts` passes.
- [x] `crates/srs-bindings/tests/navigation.rs` compiles and passes, with line 437 now asserting
      `identity.is_none()`.
- [x] No adapter **source** file reconstructs or defaults `identity` back into existence — grep
      confirms `srs-cli/src`, `srs-bindings/src`, and `srs-mcp/src` contain no `.identity` field
      access on a `RepositoryNavigation`. (Test files legitimately assert on it; `srs-gov` reads it
      from JSON with its own presentation fallback, which ADR-044 explicitly permits.)
- [x] `cargo test -p srs-gov` passes unchanged.

#### Testing

```bash
cargo test
cargo test --test payload_contracts
cargo clippy --workspace -- -D warnings
```

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm the named tests pass.
3. Run lint and tests:

```bash
cargo test
cargo clippy --workspace -- -D warnings
```

4. Update the plan file checkboxes.
5. Commit: `fix(navigation): propagate optional identity through adapter surfaces (#838)`

---

## Final Acceptance

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] CLI output format unchanged except the documented `identity` optionality (integration tests pass)
- [x] `cargo test --test payload_contracts` passes
- [x] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [x] An identity-less root container returns `identity` absent, all roots in `sections`, and a
      diagnostic naming the absence — verified end-to-end via `srs repo navigation` in dogfooding
- [x] ADR-044 exists and is cross-referenced from the changed service
- [x] `srs/srs-usage.md` documents that `navigation.identity` may be absent

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass,
  update the plan checkboxes, then commit. Do not proceed to the next phase without completing the
  milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- `RepositoryNavigation` is embedded opaquely in `RepoNavigationPayload`, so the golden payload
  schema does not change. Verified against the committed
  `crates/srs-cli/schemas/payload/repo-navigation.json` (`"navigation": true`). Phase 2 re-checks
  this rather than trusting it.
- The `identity` key becoming absent is a tolerable change for all current consumers. Verified by
  grep across `srs-web` (already `raw.identity ?? {}`) and `srs-vscode` (no consumer). A consumer
  outside this monorepo that assumes the key is always present would break; this is the accepted
  cost of the owner's Option-1 decision and is recorded in ADR-044's Consequences.
- The `precedes` chain ordering over roots is unaffected — `sort_by_precedes_chain` already runs
  over whatever section set it is handed, and a root with no `precedes` edge sorts by its existing
  deterministic tiebreak.
