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

## Stage 1.5 — Spec gate

Before writing any plan, determine whether this feature requires a **change to the SRS specification** (`srs/` repo).

A spec change is required if the feature:
- introduces a new field, type, relation type, or extension to the SRS data model,
- changes the semantics or validation rules of an existing entity,
- adds or modifies a canonical CLI contract that the spec defines (not just a Rust implementation detail), or
- requires updating `srs/docs/schema/2.0/` entity schemas.

**If a spec change is required:** do not proceed past this stage. Instead:
1. File an RFC issue in the `srs` repository: `gh issue create --repo <srs-repo-remote> --title "RFC: <title>" --label "rfc" --body "<problem, proposed change, open questions>"`.
2. Post a comment on the current issue linking the RFC and explaining that implementation is blocked until the RFC is accepted.
3. **Stop** — return to the user with the RFC URL. No planning, no implementation until the RFC is resolved.

**If no spec change is required:** state this explicitly (one sentence) and continue to Stage 2.

## Stage 2 — Plan

1. Read the template at `srs-rust/plans/TEMPLATE.md` and the role definitions at `srs-rust/plans/agents.md`. **Review the agent list** — if this feature needs a role that isn't defined (e.g. a new worker for a crate not covered), add it to `agents.md` before writing the plan.
2. Write a **draft** plan to `srs-rust/plans/<slug>.md`, filling every section of the template. A plan that needs human interpretation at execution time is incomplete.
3. **ADR check:** read every file in `srs-rust/docs/adr/`. Identify:
   - Existing ADRs that govern choices in this plan (cite them in the Architecture Decisions table).
   - Choices that require a **new ADR** — any decision that establishes a new architectural constraint, rejects a plausible alternative others might revisit, or changes a prior decision.
4. **Design decision pause:** before finalising the plan, identify any decision that has **long-term consequences** — a new public API shape, a new payload contract, a cross-crate dependency direction, a new extension model, or anything that would be painful to reverse later. For each such decision, present it clearly to the user with the trade-offs and **wait for their input** before continuing. Record their decision in the plan's Architecture Decisions table (and draft a new ADR if warranted). This is the one deliberate pause in the autonomous pipeline.
5. After input is received and decisions are recorded, finalise the plan and draft any new ADRs in `srs-rust/docs/adr/NNN-title.md` using `ADR-TEMPLATE.md` (status: `proposed`).
6. Set the issue body to the plan: `gh issue edit N --body-file srs-rust/plans/<slug>.md`.

## Stage 3 — Plan review loop

1. Spawn review agents **in parallel** (one Agent call, multiple invokes):
   - **Architecture Reviewer** (`agents.md#architecture-reviewer`) — must check the plan against **every** ADR for system boundaries, DRYness, consistent coding style, and ADR coverage.
   - **Plan Reviewer** (`agents.md#plan-reviewer`) — completeness, contracts, scope discipline, testability.
   Give each agent the plan file path and the relevant CLAUDE.md / ADR paths. They are read-only and return numbered findings with severity (`blocking` / `should-fix` / `nit`).
2. Post **all** findings as comments on the issue: `gh issue comment N --body "<findings>"` (one comment per reviewer, clearly attributed).
3. Respond to the review: update the plan to resolve every `blocking` and `should-fix` finding; for any finding you decline, record why in an issue comment. Re-sync the issue body: `gh issue edit N --body-file <plan>`.
4. **File deferred items as issues:** for every item the plan explicitly defers to a future plan (marked in *Out of scope* or *Assumptions*), create a GitHub issue capturing the deferred work:
   ```
   gh issue create --title "<deferred item title>" \
     --label "enhancement,complexity: <low|medium|high>" \
     --body "<what was deferred, why, and what the future plan needs to address>"
   ```
   If the deferred item would require a spec change, add `--label "requires-spec-rfc"` and note it in the body. Post a comment on the current issue listing all newly filed deferred issues.
5. **Loop:** if the plan is large (≥ 3 phases or touches ≥ 2 crates) **and** the last review produced any `blocking` finding, re-run the review on the updated plan. Repeat until a review pass yields **zero** blocking findings.

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
