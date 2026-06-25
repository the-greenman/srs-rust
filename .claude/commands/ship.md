---
description: Plan → review → implement → PR pipeline for a feature. Autonomous between human checkpoints.
argument-hint: <feature description, or issue #N>
allowed-tools: Bash, Read, Write, Edit, Glob, Grep, Agent, TodoWrite, WebFetch
---

# /ship — feature pipeline

You are running the full delivery pipeline for this feature:

> $ARGUMENTS

Run autonomously between stages — do not pause for minor decisions you can resolve from context. Use TodoWrite to track the stages below and work through them in order.

The pipeline runs to a **single terminal state inside this session**: PR open with CI green, everything (including dogfooding) verified pre-merge. It does **not** wait for a human to merge and then resume — never schedule a follow-up routine to "come back after merge". The issue auto-closes via `Closes #N` on merge.

There are two **deliberate human checkpoints** where you stop and wait for input:

| Checkpoint | Stage | When |
|---|---|---|
| RFC gate | 1.5 | Feature requires a spec change → file RFC, stop |
| Design decisions | 2 | Long-term architectural choices → present trade-offs, wait for input |

The pipeline then ends at **Stage 9** (PR open + CI green) — a terminal handoff, not a resume point.

Outside the two checkpoints, keep going. If a stage is genuinely blocked (auth failure, unresolvable rebase conflict, ambiguous requirement that changes the deliverable), stop and report.

All Rust work happens in `srs-rust/`. Run `git` from the relevant sub-repo, never from the `semanticops/` parent (it is not a git repo).

---

## Stage 0 — Preflight

1. Confirm a commit-signing method is available (commits will fail otherwise). The required check depends on the environment:
   ```bash
   if ssh-add -l 2>/dev/null | grep -q "SHA256:vHuO6si5w3RLL4IJZofWbyvEi42WA2fYX7bM"; then
     echo "OK: local SSH signing key loaded in agent"
   elif [ ! -f "$HOME/.ssh/id_ed25519_git_signing.pub" ]; then
     echo "OK: cloud/remote environment — platform provides its own commit signing, ssh-agent not used"
   else
     echo "SIGNING KEY NOT LOADED — local key file present but not in agent"
   fi
   ```
   - **Local** (the signing key file exists under `~/.ssh`): the key must be loaded in the ssh-agent. If you see `SIGNING KEY NOT LOADED`, **stop** and tell the user — do not bypass signing.
   - **Cloud / remote agent** (no local signing key file, e.g. a scheduled CCR run): the ssh-agent is not used — the platform signs commits with its own method. Proceed; do not stop on the ssh-agent check.

   In both environments use plain `git commit` — never `--no-gpg-sign`.
2. Confirm `gh auth status` succeeds. If not, stop.
3. Identify the repo this work belongs to (srs / srs-rust / srs-vscode) from the feature description. Most work is srs-rust.

## Stage 1 — Issue

- If `$ARGUMENTS` references an existing issue (`#N` or a URL), fetch it with `gh issue view N` and use it as the brief.
- Otherwise create one: `gh issue create --title "<concise title>" --body "<one-paragraph problem statement>"`. Capture the issue number — every later stage refers to it.

- Mark the issue as in progress

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
   - **Architecture Reviewer** (`agents.md#architecture-reviewer`) `model: "sonnet"` — checks the plan against **every** ADR for system boundaries, DRYness, consistent coding style, and ADR coverage.
   - **Plan Reviewer** (`agents.md#plan-reviewer`) `model: "haiku"` — completeness, contracts, scope discipline, testability.
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

Naming convention: **`feat/<issue-N>-<slug>`** where `<issue-N>` is the issue number and `<slug>` is a short kebab-case descriptor from the title (e.g. issue #42 "Add field validation" → `feat/42-add-field-validation`). Worktrees mirror: `../.worktrees/<issue-N>-<slug>`.

Before creating, check whether a branch for this issue already exists:

```bash
cd srs-rust
BRANCH="feat/$ISSUE_N-$SLUG"
WORKTREE="../.worktrees/$ISSUE_N-$SLUG"

if git show-ref --verify --quiet "refs/heads/$BRANCH"; then
  echo "Branch $BRANCH already exists locally — reusing"
  git worktree add "$WORKTREE" "$BRANCH"
elif git show-ref --verify --quiet "refs/remotes/origin/$BRANCH"; then
  echo "Branch $BRANCH exists on remote — tracking"
  git worktree add "$WORKTREE" --track -b "$BRANCH" "origin/$BRANCH"
else
  git worktree add "$WORKTREE" -b "$BRANCH"
fi
```

Do all implementation inside that worktree.

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

## Stage 6 — Sync with main + final acceptance

First bring the branch up to date so acceptance, dogfooding, and CI all reflect current `main`:
```bash
# from inside the worktree
git fetch origin
git rebase origin/main
```
If the rebase conflicts and you cannot resolve it confidently from context, **stop and report** — this is a genuine blocker. Re-run the milestone gates after a non-trivial rebase.

Then run the full Final Acceptance list from the plan:
```bash
cargo test
cargo clippy -- -D warnings
cargo test --test payload_contracts        # if payload structs changed
bash scripts/check-schema-sync.sh          # if entity schemas changed
```

If `check-schema-sync.sh` reports that **this repo's** schema mirror has drifted from the canonical spec in `srs/docs/schema/2.0/`, refresh it from the released canonical schema — **this repo only**:
```bash
# From srs-rust/
scripts/sync-schemas-from-spec.sh          # pulls the canonical schema (local ../srs if present,
                                           # else the srs `schemas-2.0.tar.gz` release artifact)
                                           # and regenerates SHA256SUMS
```
Commit the refreshed mirror in `srs-rust` only. **Do not reach into `../srs-vscode` or any other sibling working tree** — in a cloud session it won't exist, and each mirror repo syncs itself from the `srs` release artifact through its own pipeline. Cross-repo mirror consistency is enforced by the release-drift CI, not by editing siblings in this session.

All checks for this repo must pass before proceeding.

## Stage 7 — Code review loop

1. Spawn in parallel against the diff (`git diff main...HEAD`):
   - **Architecture Reviewer** (`agents.md#architecture-reviewer`) `model: "sonnet"` — audits code against every ADR + crate-boundary rules.
   - **Verification Agent** (`agents.md#verification-agent`) `model: "haiku"` — runs tests, produces the boundary/duplication report.
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
   | New/changed CLI surface that needs end-to-end dogfooding | `srs-rust/docs/dogfooding.md` — but make this update in **Stage 7.6**, not here (it depends on having actually run the scenario) |

   `srs-usage.md` lives in the `srs/` repo. If you update it, commit that change on a branch in `srs/` (coordinate it with this PR the way schema changes are coordinated) — do not edit it inside the `srs-rust` worktree.

3. **Hunt for stale references.** Grep the docs for anything this change made wrong — renamed commands, removed flags, changed payload field names, old crate names:
   ```bash
   rg -n "<old-name-or-flag>" --glob '*.md' .
   ```
   Fix every stale hit you find, not just the ones in the table above.

4. **Verify doc commands still run.** Any command block you added or touched in a `CLAUDE.md` or `README.md` must actually work — run it. A doc command that errors is a regression.

5. Commit the doc updates with a message referencing the issue: `docs: update docs for <slug> (#N)`. Stage them so they are part of this PR's diff.

## Stage 7.6 — Dogfooding (pre-PR, on the branch)

Reference: `docs/dogfooding.md`. Read its principles first — dogfooding proves the change advances a *meaningful intention*, not just that a command returns `ok: true`. This runs **before** the PR, on the rebased feature branch (Stage 6 already synced it with `main`), so the dogfood results and any guide updates land in this same PR.

**Skip** if purely internal (refactor, test-only, doc-only, build tooling). State the reason; do not skip silently.

1. **Build from the branch under review** (the current worktree HEAD — do **not** check out `main`):
   ```bash
   cargo build --bin srs            # from inside the worktree
   ```

2. **Find scenario(s)** using the Coverage matrix in `docs/dogfooding.md`. Note if it's a `gap` row or entirely new surface.

3. **Prepare repo** per scenario — default: `srs repo create --repo /tmp/dogfood-<slug> --namespace com.example.dogfood`.

4. **Run end-to-end:** happy path + named negative case. Confirm output matches the payload contract, negative case returns a correct error envelope, and "Done when" signals hold. Run `srs repo validate` (diagnostics are in the payload, not the exit code). Run extra edge cases from the plan's acceptance criteria.

5. **Update `docs/dogfooding.md`** (part of this PR's diff): update scenario steps/"Done when" for changed surfaces; add a new scenario for new surfaces (must lead with a meaningful intention — if you can't state it, note it on the issue instead); update the Coverage matrix. Verify every touched command block still runs. Commit: `docs: update dogfood scenarios for <slug> (#N)`.

6. **File issues:** `bug` for anything not working as designed (patch immediately if trivial); `enhancement` for workflow gaps that required workarounds. No cosmetic/hypothetical issues.

7. **Summarise:** scenarios run, commands exercised, guide updates made, issues filed, bugs patched inline.

## Stage 8 — PR

Before pushing, run a final lint gate:
```bash
cargo clippy -- -D warnings
```
If clippy reports errors, fix them and commit before proceeding.

Then push and open the PR:
```bash
cd srs-rust
git push -u origin feat/$ISSUE_N-$SLUG
gh pr create --fill --base main --body "<summary>

Closes #N

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
```
End the body with the Claude Code attribution line. Link the PR back on the issue if `--fill` didn't.

**If this PR includes schema mirror changes** (files under `crates/srs-schema/schemas/`): this PR carries only **this** repo's mirror, refreshed from the `srs` release artifact. Do not coordinate sibling branches in this session — each mirror repo (`srs-vscode`, etc.) refreshes itself from the same `srs` release through its own pipeline, and the release-drift CI enforces eventual consistency. If you want to nudge the other mirrors, file a best-effort mirror-sync tracking issue in each rather than editing their trees.

## Stage 9 — Sweep open issues + CI watch

1. Run `gh issue list --state open` and check whether any open issue is now addressable by this change or is a quick adjacent fix. Address what you reasonably can within this branch/PR; for the rest, leave a comment noting status. Do not scope-creep the PR with unrelated large work — note those as follow-ups instead.

2. **Watch CI and fix failures.** Subscribe to PR activity using `subscribe_pr_activity`, then:
   - When a CI check fails, fetch the job logs (`mcp__github__get_job_logs` or `gh run view --log-failed`), diagnose the root cause, push a fix commit, and repeat.
   - Keep fixing until all required checks are green or you hit a blocker you cannot resolve (external flake, infrastructure outage, requires human decision).
   - If a failure is a pre-existing flake unrelated to this change, note it in a PR comment and move on.
   - Do not close the PR or give up silently — always report status.

**This is the terminal state.** Once all CI checks pass (or a blocker is hit), the pipeline is done — report the PR URL and hand off. Do **not** wait for the merge, do **not** schedule a routine to resume after merge, and do **not** close the issue manually: `Closes #N` in the PR body closes it automatically on merge.

**Best-effort cleanup (optional, non-blocking).** If you created a local worktree and are in a long-lived local session, you may tidy it now — but never wait for the merge to do so, and never treat cleanup as a gate. In a cloud session the harness reclaims the workspace, so skip it.
```bash
git worktree remove ../.worktrees/<issue-N>-<slug> 2>/dev/null || true
git branch -d feat/<issue-N>-<slug> 2>/dev/null || true
```

---

## Output contract

When done, report: issue #, plan path, ADRs created (if any), number of review rounds, **the docs updated in Stage 7.5 (or "none — internal change")**, the dogfooding summary (Stage 7.6 — scenario(s) run, commands exercised, dogfood-guide updates made, bugs filed, feature gaps filed, or "skipped — internal change"), and the PR URL with its final CI status. The pipeline ends at PR-open + CI green; the issue closes on merge via `Closes #N` — do not report a manual close or a post-merge step. If you stopped early, say exactly which stage and why.
