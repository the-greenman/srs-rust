# Plan: <Title>

<!-- WORKFLOW
This file is a design-brief template. Once the plan is ready:
1. Copy the entire body (below this comment block) into a new GitHub issue on the-greenman/srs-rust.
2. Apply labels: `plan` + one of `enhancement` / `bug` / `refactor`.
3. Suggested issue title format: "plan: <verb> <noun>" — e.g. "plan: add srs note delete CLI command".
4. The issue is now the live tracking document. Check boxes directly in the issue body as work progresses.
5. Close the issue when Final Acceptance passes and the PR is merged.

This file stays in plans/ as the original design brief. Do not commit progress updates to it —
those belong as comments or checkbox updates on the GitHub issue.

Agents: at the start of any work session, fetch the current issue state with:
    gh issue view <number> --repo the-greenman/srs-rust
This is the source of truth for which phase is active and which checkboxes are complete.
-->

---

**Issue metadata (apply when creating the issue)**

- Labels: `plan`, `enhancement` | `bug` | `refactor`
- Title format: `plan: <verb> <noun phrase>`
- Branch: `feat/<slug>` (create before assigning agents)

---

## Summary

One paragraph. What problem does this plan solve, and why now?

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| <Worker Role> | — |
| Verification | — |

See [agents.md](agents.md) for role definitions and GitHub issue update responsibilities.

## Architecture Decisions

List any ADRs that govern this plan, or that this plan produces. Create new ADRs in `docs/adr/` for any decision that:
- establishes a new architectural constraint,
- rejects a plausible alternative that others might revisit, or
- changes a previously accepted decision.

| ADR | Decision | Status |
|---|---|---|
| [ADR-NNN](../docs/adr/NNN-title.md) | One-line summary | proposed / accepted |

If no new ADRs are needed, state why: _"No new architectural decisions — this plan implements ADR-NNN."_

---

## Contracts

Answer each question. Delete the section only if the plan touches no command handlers, service outputs, or entity schemas.

### CLI output contract (ADR-011)

Does this plan add or change any CLI command output shapes?

- **No new/changed commands** → no action required; golden schemas stay as-is.
- **New command added** → add a payload struct to `crates/srs-cli/src/payload.rs`, wire the handler to use `output::serialize()`, run `cargo run --bin generate-schemas`, commit the new `schemas/payload/<name>.json` on the feature branch.
- **Existing command payload changed** (field renamed, added, or removed) → update the struct in `payload.rs`, run `cargo run --bin generate-schemas`, commit the updated schema file on the feature branch. The diff in the schema file is the explicit contract change record.
- **Service type used in a payload changes** → if embedded via `#[schemars(with = "serde_json::Value")]` in `payload.rs`, no schema regeneration needed; if the type has a local mirror struct in `payload.rs`, update the mirror and regenerate.

Verification: `cargo test --test payload_contracts` must pass after any payload change.

### Entity schema sync (check-schema-sync.sh)

Does this plan add or modify JSON Schema files under `srs/docs/schema/2.0/`?

- **Yes** → copy the updated files to `crates/srs-schema/schemas/2.0/` and `srs-vscode/schemas/2.0/` and verify `bash scripts/check-schema-sync.sh` exits 0.
- **No** → no action required.

---

## Scope

What is explicitly in scope. Keep it tight — list inclusions not exclusions.

- ...

**Out of scope:** What this plan deliberately defers or excludes.

- ...

---

## Phases

### Phase N: <Name>

**Goal:** One sentence — what state are we in after this phase completes?

**Agent:** <Role from agents.md>

#### Tasks

- [ ] Task description
- [ ] Task description
- [ ] Task description

#### Acceptance Criteria

- [ ] Behaviour X works as described
- [ ] No regression in Y
- [ ] Test Z passes

#### Testing

```bash
# Commands to verify this phase
cargo test -p <crate>
```

Specific tests to write or verify:

- `<test name>` — what it proves

#### Milestone gate

Every phase ends with a full check before the next phase starts:

1. Verify all acceptance criteria above are met — check each checkbox in the GitHub issue.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p <crate>
cargo clippy -p <crate> -- -D warnings
```

4. Update the GitHub issue: check off completed task and acceptance-criteria boxes. Post a milestone comment:

```bash
gh issue comment <number> --repo the-greenman/srs-rust \
  --body "Phase N complete. All acceptance criteria met. Moving to Phase N+1."
```

5. Commit work to the feature branch and push:

```bash
git commit
git push origin feat/<slug>
```

Do not start the next phase until the milestone gate passes and the issue is updated.

---

## Final Acceptance

All of the following must be true before the issue is closed and the PR merged:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (or no payload structs were changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (or no entity schemas were changed)
- [ ] <Plan-specific criterion>
- [ ] <Plan-specific criterion>

When all boxes are checked, close the issue and open the PR:

```bash
gh issue close <number> --repo the-greenman/srs-rust
```

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others. If you find unexpected edits in your write scope, surface them to the Lead Integrator via an issue comment before proceeding.
- Workers post a progress comment on the governing issue at each milestone gate.
- Workers return changed file paths and a short behaviour summary to the Lead Integrator when a phase is complete.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the issue checkboxes, commit to the feature branch, push. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- ...
