---
description: Plan → review → implement → PR pipeline for a feature. Runs end-to-end autonomously.
argument-hint: <feature description, or issue #N>
allowed-tools: Bash, Read, Write, Edit, Glob, Grep, Agent, TodoWrite, WebFetch
---

# /ship — autonomous feature pipeline

You are running the full delivery pipeline for this feature:

> $ARGUMENTS

This command runs **fully autonomously** — do not pause for approval between stages. Use TodoWrite to track the stages below and work through them in order. If a stage is genuinely blocked (auth failure, unresolvable conflict, ambiguous requirement that changes the deliverable), stop and report; otherwise keep going.

All Rust work happens in `srs-rust/`. Run `git` from the relevant sub-repo, never from the `semanticops/` parent (it is not a git repo).

---

## Stage 0 — Preflight

1. Confirm the signing key is loaded (commits will fail otherwise):
   ```bash
   ssh-add -l | grep -q "SHA256:vHuO6si5w3RLL4IJZofWbyvEi42WA2fYX7bM" || echo "SIGNING KEY NOT LOADED"
   ```
   If missing, **stop** and tell the user — do not bypass signing.
2. Confirm `gh auth status` succeeds. If not, stop.
3. Identify the repo this work belongs to (srs / srs-rust / srs-vscode) from the feature description. Most work is srs-rust.

## Stage 1 — Issue

- If `$ARGUMENTS` references an existing issue (`#N` or a URL), fetch it with `gh issue view N` and use it as the brief.
- Otherwise create one: `gh issue create --title "<concise title>" --body "<one-paragraph problem statement>"`. Capture the issue number — every later stage refers to it.

## Stage 2 — Plan

1. Read the template at `srs-rust/plans/TEMPLATE.md` and the role definitions at `srs-rust/plans/agents.md`. **Review the agent list** — if this feature needs a role that isn't defined (e.g. a new worker for a crate not covered), add it to `agents.md` before writing the plan.
2. Write the plan to `srs-rust/plans/<slug>.md`, filling **every** section of the template: Summary, Agent Assignments, Architecture Decisions, Contracts, Scope, Phases (with tasks / acceptance criteria / testing / milestone gate), Final Acceptance, Coordination Rules, Assumptions. A plan that needs human interpretation at execution time is incomplete.
3. **ADR check:** read every file in `srs-rust/docs/adr/`. For each architectural choice the plan makes, either cite the governing ADR in the Architecture Decisions table, or identify that a **new ADR is needed**. If new ADRs are needed, draft them in `srs-rust/docs/adr/NNN-title.md` using `ADR-TEMPLATE.md` (status: `proposed`) and reference them in the plan.
4. Set the issue body to the plan: `gh issue edit N --body-file srs-rust/plans/<slug>.md`.

## Stage 3 — Plan review loop

1. Spawn review agents **in parallel** (one Agent call, multiple invokes):
   - **Architecture Reviewer** (`agents.md#architecture-reviewer`) — must check the plan against **every** ADR for system boundaries, DRYness, consistent coding style, and ADR coverage.
   - **Plan Reviewer** (`agents.md#plan-reviewer`) — completeness, contracts, scope discipline, testability.
   Give each agent the plan file path and the relevant CLAUDE.md / ADR paths. They are read-only and return numbered findings with severity (`blocking` / `should-fix` / `nit`).
2. Post **all** findings as comments on the issue: `gh issue comment N --body "<findings>"` (one comment per reviewer, clearly attributed).
3. Respond to the review: update the plan to resolve every `blocking` and `should-fix` finding; for any finding you decline, record why in an issue comment. Re-sync the issue body: `gh issue edit N --body-file <plan>`.
4. **Loop:** if the plan is large (≥ 3 phases or touches ≥ 2 crates) **and** the last review produced any `blocking` finding, re-run the review on the updated plan. Repeat until a review pass yields **zero** blocking findings.

## Stage 4 — Branch & worktree

```bash
cd srs-rust
git worktree add ../.worktrees/<slug> -b feat/<slug>
```
Do all implementation inside that worktree. (Worktrees living under `semanticops/.claude/worktrees/` or `../.worktrees/` are fine — pick one and be consistent.)

## Stage 5 — Implement

Execute the plan phase by phase. For each phase:
- Implement the tasks, respecting the agent write-scopes and crate boundaries.
- Run the phase's **milestone gate**: verify acceptance criteria, confirm the named tests exist and pass, then:
  ```bash
  cargo test -p <crate>
  cargo clippy -- -D warnings
  ```
- After changing any struct in `crates/srs-cli/src/payload.rs`: `cargo run --bin generate-schemas` and stage the updated `schemas/payload/` files.
- Mark plan checkboxes `[x]` and **commit at the milestone** with a message referencing the issue (`... (#N)`). Use plain `git commit` — never `--no-gpg-sign`.

Do not start a phase until the previous milestone gate passes.

## Stage 6 — Final acceptance

Run the full Final Acceptance list from the plan:
```bash
cargo test
cargo clippy -- -D warnings
cargo test --test payload_contracts        # if payload structs changed
bash scripts/check-schema-sync.sh           # if entity schemas changed
```
All must pass before proceeding.

## Stage 7 — Code review loop

1. Spawn the **Architecture Reviewer** (`agents.md#architecture-reviewer`) and the **Verification Agent** (`agents.md#verification-agent`) against the **diff** (`git diff main...HEAD`). Architecture Reviewer audits the code against every ADR + crate-boundary rules; Verification Agent runs tests and produces the boundary/duplication report.
2. Post findings as issue comments.
3. Respond: fix every `blocking` and `should-fix` finding, committing the fixes. Decline-with-reason for anything not fixed.
4. **Loop:** on a large change, repeat the code review until a pass yields zero blocking findings.

## Stage 8 — PR

```bash
cd srs-rust
git push -u origin feat/<slug>
gh pr create --fill --base main --body "<summary>

Closes #N

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
```
End the body with the Claude Code attribution line. Link the PR back on the issue if `--fill` didn't.

## Stage 9 — Sweep open issues

Run `gh issue list --state open` and check whether any open issue is now addressable by this change or is a quick adjacent fix. Address what you reasonably can within this branch/PR; for the rest, leave a comment noting status. Do not scope-creep the PR with unrelated large work — note those as follow-ups instead.

---

## Output contract

When done, report: issue #, plan path, ADRs created (if any), worktree path, branch, number of review rounds, and the PR URL. If you stopped early, say exactly which stage and why.
