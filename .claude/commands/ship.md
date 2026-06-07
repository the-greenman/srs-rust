---
description: Plan → review → implement → PR pipeline for a feature. Autonomous between human checkpoints.
argument-hint: <feature description, or issue #N>
allowed-tools: Bash, Read, Write, Edit, Glob, Grep, Agent, TodoWrite, WebFetch
---

# /ship — feature pipeline

You are running the full delivery pipeline for this feature:

> $ARGUMENTS

Run autonomously between stages — do not pause for minor decisions you can resolve from context. Use TodoWrite to track the stages below and work through them in order.

There are four **deliberate human checkpoints** where you must stop and wait:

| Checkpoint | Stage | When |
|---|---|---|
| RFC gate | 1.5 | Feature requires a spec change → file RFC, stop |
| Design decisions | 2 | Long-term architectural choices → present trade-offs, wait for input |
| PR review & merge | 9 | PR is open → hand off to human, stop |
| Post-merge continuation | 10 | User resumes after merge → cleanup + dogfood |

Outside these checkpoints, keep going. If a stage is genuinely blocked (auth failure, unresolvable conflict, ambiguous requirement that changes the deliverable), stop and report.

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

## Stage 7.5 — Documentation pass

The pipeline is not done until the docs match the code. This stage runs after the code is final (Stage 7 passed) and before the PR, so doc updates land **in the same PR** as the change.

1. **Determine the user-facing surface this change touched.** Ask: did this change add or modify any of —
   - a CLI command, flag, stdin shape, or payload struct (`crates/srs-cli/src/payload.rs`),
   - a service function signature or a crate boundary/responsibility,
   - an ADR (a new one drafted in Stage 2, or an existing one now superseded),
   - build/test/run commands or developer workflow?

   If the change is purely internal (refactor with no observable surface change), state that in one sentence and skip to Stage 8 — but say so explicitly; do not skip silently.

2. **Update each affected doc.** Map surface → doc:
   | Changed surface | Doc(s) to update |
   |---|---|
   | New/changed CLI command, flag, stdin, or payload | `srs/srs-usage.md` (authoritative CLI command reference), and the CLI reference in `semanticops/CLAUDE.md` if the contract-level shape changed |
   | New/changed crate responsibility or boundary | `srs-rust/CLAUDE.md` (Crate Authority table) and `semanticops/CLAUDE.md` (Architecture → Rust crate boundaries) |
   | New ADR | confirm it is listed/cross-referenced where ADRs are indexed; flip its status from `proposed` to `accepted` if the change shipped under it |
   | New build/test/run command or workflow | the **Commands** section of the relevant `CLAUDE.md` |
   | New top-level capability in a crate with a `README.md` | that crate's `README.md`, plus `srs-rust/README.md` if one exists |

   `srs-usage.md` lives in the `srs/` repo. If you update it, commit that change on a branch in `srs/` (coordinate it with this PR the way schema changes are coordinated) — do not edit it inside the `srs-rust` worktree.

3. **Hunt for stale references.** Grep the docs for anything this change made wrong — renamed commands, removed flags, changed payload field names, old crate names:
   ```bash
   rg -n "<old-name-or-flag>" --glob '*.md' .
   ```
   Fix every stale hit you find, not just the ones in the table above.

4. **Verify doc commands still run.** Any command block you added or touched in a `CLAUDE.md` or `README.md` must actually work — run it. A doc command that errors is a regression.

5. Commit the doc updates with a message referencing the issue: `docs: update docs for <slug> (#N)`. Stage them so they are part of this PR's diff.

## Stage 8 — PR

```bash
cd srs-rust
git push -u origin feat/<slug>
gh pr create --fill --base main --body "<summary>

Closes #N

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
```
End the body with the Claude Code attribution line. Link the PR back on the issue if `--fill` didn't.

## Stage 9 — Sweep open issues and close

1. Run `gh issue list --state open` and check whether any open issue is now addressable by this change or is a quick adjacent fix. Address what you reasonably can within this branch/PR; for the rest, leave a comment noting status. Do not scope-creep the PR with unrelated large work — note those as follow-ups instead.
2. **Close the primary issue.** The `Closes #N` in the PR body triggers automatic closure on merge, but only if the repo has that setting enabled. To be safe, also close it explicitly once the PR is open:
   ```bash
   gh issue close N --comment "Implemented in PR #<PR number>."
   ```

**Stop here.** Stages 10 and 11 require the PR to be merged by a human. Report the PR URL and instruct the user to run `/ship` again (or continue this session) once the PR is merged.

## Stage 10 — Post-merge worktree cleanup

**Prerequisite:** confirm the PR is merged before proceeding.
```bash
gh pr view <PR-number> --json state --jq '.state'   # must return MERGED
```
If it is not yet merged, stop and wait — do not clean up a worktree for an open or closed-without-merge PR.

Once confirmed:
```bash
cd srs-rust
git fetch origin --prune
git worktree remove ../.worktrees/<slug> --force
git branch -d feat/<slug> 2>/dev/null || true
```

Verify with `git worktree list` that the worktree is gone. Report the result.

## Stage 11 — Dogfooding

**Skip this stage** if the change is purely internal (refactor, test-only, doc-only, build tooling) with no new or modified CLI commands, flags, stdin shapes, or observable behaviours. State the skip reason explicitly; do not skip silently.

**Otherwise:** exercise every new or modified CLI surface end-to-end using a real SRS repository.

1. **Build the CLI** from the merged state (pull main first):
   ```bash
   cd srs-rust
   git checkout main && git pull origin main
   cargo build --bin srs
   ```

2. **Prepare a test repository.** Prefer creating a fresh one so you are not working against a repo that was set up before this feature existed:
   ```bash
   cargo run --bin srs -- repo init /tmp/dogfood-<slug>
   ```
   If the feature targets an existing repo structure (e.g. requires records already present), use `srs/srs` as the target — it is always valid and representative.

3. **Drive the new surface.** For each new or changed command, flag, or stdin shape added in this PR:
   - Run the happy path and confirm output matches the payload contract.
   - Run at least one negative case (bad input, missing field, wrong type) and confirm the error envelope is correct.
   - Run any edge cases called out in the plan's acceptance criteria.

4. **Log findings as GitHub issues:**
   - **Bug** (something doesn't work as designed): file immediately with label `bug`. Patch it in a follow-up commit on main (or a new branch if non-trivial). Do not leave a `bug`-labelled issue open without at least a comment saying what the fix is.
   - **Feature gap** (an essential step in a real workflow has no built-in way to accomplish it — you had to manually edit JSON, chain commands awkwardly, or give up): file with label `enhancement`. Describe the missing primitive and the workflow it blocks.
   - Do not file issues for cosmetic nits or hypothetical future improvements — only gaps that would block a real use of the feature.

5. **Summarise.** Report: commands exercised, happy-path results, issues filed (URLs), and whether you patched any bugs inline.

---

## Output contract

When done, report: issue #, plan path, ADRs created (if any), worktree path cleaned up (Stage 10), branch deleted, number of review rounds, **the docs updated in Stage 7.5 (or "none — internal change")**, the PR URL, and dogfooding summary (Stage 11 — commands exercised, bugs filed, feature gaps filed, or "skipped — internal change"). If you stopped early, say exactly which stage and why.
