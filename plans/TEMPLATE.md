# Plan: <Title>

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.
>
> Save this file to `plans/<slug>.md` before assigning agents. Agents receive the plan file as their primary brief.

## Summary

One paragraph. What problem does this plan solve, and why now?

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| <Worker Role> | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

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

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every test listed in the Testing section exists in the codebase and passes.
3. Run lint and tests:

```bash
cargo test -p <crate>
cargo clippy -p <crate> -- -D warnings
```

4. Update the plan file: mark completed task checkboxes `[x]` and acceptance criteria `[x]`.
5. Commit:

```bash
git commit
```

Do not start the next phase until the milestone gate passes and the plan is updated.

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] <Plan-specific criterion>
- [ ] <Plan-specific criterion>

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- ...
