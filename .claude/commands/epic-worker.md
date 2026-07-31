---
description: Claim the one epic-256 issue dispatched in this repo, work it to an open PR with green CI, and stop. Terminal at the PR.
argument-hint: "<epic number> (default: 256)"
allowed-tools: Bash, Read, Write, Edit, Glob, Grep, Task, TodoWrite, WebFetch
---

# /epic-worker — do exactly one dispatched task, then stop

> $ARGUMENTS

You are an **Epic 256 Worker** for this repository. You do one thing: take the single issue the coordinator has dispatched here, drive it to a PR that is open with green CI, and stop.

Three things you never do, because the coordinator owns them:

- **You never choose what to work on.** If nothing is dispatched, you stop. Picking up "something useful" instead breaks the sequential gate this epic exists to hold.
- **You never merge.** Not your own PR, not anyone's.
- **You never write the ledger or the epic body.**

Run **autonomously**. Track stages with TodoWrite. A run that finds nothing dispatched should print one line and exit — that is a correct run, not a wasted one.

---

## Environment (read first)

Proxy-restricted cloud session; all five ecosystem clones are on disk.

- **Works:** `git`, `gh issue *`, `gh pr *`, `gh api repos/...`, `gh workflow run`.
- **BLOCKED, never call:** `gh api graphql`, `gh-project.mjs`, any Projects v2 query.
- **Default branch is `master`.** Not `main`. PR against `master`.
- Run `git` from the repo directory. **Never** from the `semanticops/` parent — it is not a git repo.

**Commit signing.** No `~/.ssh/id_ed25519_git_signing.pub` → cloud environment, the platform signs, proceed. File present but absent from `ssh-add -l` → **stop and tell the owner**. Never `--no-gpg-sign`.

---

## Stage 0 — Interlock, then claim

```bash
gh issue list --repo the-greenman/<this-repo> --state open --label "epic-256:blocked-owner" --json number
gh issue list --repo the-greenman/srs --state open --label "epic-256:blocked-owner" --json number
```

**Any result in either repo → stop immediately.** The flow is paused on an owner decision; that is global, not per-repo, and not per-issue. Print one line and exit.

Otherwise find your work:

```bash
gh issue list --repo the-greenman/<this-repo> --state open --label "epic-256:dispatched" \
  --json number,title,labels,url
```

- **Nothing dispatched** → print "no dispatched task" and stop. Do not look for other work.
- **More than one** → the coordinator's one-per-repo invariant is broken. Claim nothing, comment the conflict on the epic, and stop.
- **An issue also carrying `epic-256:external-work`** → it is held for a session outside this flow. Do not claim it. Report and stop.

**Claim it before doing anything else**, so a concurrent run of this same routine sees it is taken:

```bash
gh issue edit <n> --repo the-greenman/<this-repo> \
  --add-label "epic-256:working" --remove-label "epic-256:dispatched"
gh issue comment <n> --repo the-greenman/<this-repo> --body "<!-- epic-256:claim -->
**Claimed** by the epic-256 worker at $(date -u +%FT%TZ). Terminal at PR — this session does not merge."
```

Then **re-read the issue** and confirm you are the only claimant. If another claim comment appeared with an earlier timestamp, release yours and stop.

Read the coordinator's `<!-- epic-256:dispatch -->` brief. It carries the acceptance criteria, the gates, the merge class, and any decision already settled — **do not re-litigate a resolved decision**, and do not widen the scope beyond the brief.

## Stage 1 — Route to the right pipeline

Pick by what the task actually is, not by which command is most familiar:

| Task shape | Route |
|---|---|
| An RFC is needed, or an existing RFC must be revised | `/rfc` |
| Authoring or migrating SRS records, relations, containers, views | `/author` |
| Rust crates, CLI, payload contract, engine behaviour | `/ship` |
| Scripts (`scripts/*.mjs`), `packages/`, CI, mirrors, generated artefacts | implement directly (Stage 2) |

The pipeline commands already encode the review loops, doc passes, and PR conventions. **Use them rather than reimplementing their stages.** Two overrides that apply whichever route you take:

- **PR base is `master`.** Where a pipeline says `--base main`, it is stale — use `master`.
- **Terminal at the PR.** Never schedule a follow-up to resume after merge. The coordinator handles everything post-PR.

## Stage 2 — Direct implementation (when no pipeline fits)

Branch off `master`, naming it for the issue: `fix/<n>-<slug>`, `chore/<n>-<slug>`, or `data/<n>-<slug>`. Work in a worktree (`../.worktrees/<n>-<slug>`) so you never disturb a dirty main checkout — another session may be mid-task in it.

**Push the branch early**, before the work is finished. The coordinator can only see pushed branches; an unpushed branch is invisible to the flow and is how work gets duplicated or lost. Pushing early is cheap and makes your claim real.

Implement against the acceptance criteria in the brief. Commit at each meaningful milestone with `(#<n>)` in the message.

## Stage 3 — Gates before the PR

These are the traps this epic has already fallen into. Every one has cost a real PR.

- **`srs repo validate --repo srs/srs` must report 0 errors.** Diagnostics are in the payload, not the exit code — read them.
- **`node scripts/validate-all.mjs` green.**
- **`check-release-drift` is live again and is NOT part of `validate-all.mjs`.** If you changed records you **must** re-render the committed exports: `SRS_CLI_PATH=<pinned srs> node scripts/publish-spec.mjs`. This is the single easiest gate to miss.
- **Pin the `srs` binary — do not trust `which srs`.** A stale binary fails with `missing field valueType` and rejects `dataModelRevision`, which looks exactly like the ADR-004 condition this epic already closed out. Build from `origin/master` and record the SHA in the PR body.
- **`publish-spec.mjs` writes schema mirrors to the wrong path when run from a worktree**, creating orphan directories instead of updating the real mirrors. Check where the files actually landed.
- **Never edit a sibling repo's tree.** Mirrors refresh from the `srs` `schemas-2.0.tar.gz` release artifact via their own pipelines. Raise a tracking issue instead.
- Rust work additionally: `cargo test`, `cargo clippy -- -D warnings`, and regenerate payload schemas if `payload.rs` structs changed.

Anything you deliberately skip, **say so in one sentence in the PR body**. Never skip silently.

## Stage 4 — The decision protocol

The moment you hit a question the issue, the epic brief, and the spec do not settle — **stop implementing and resolve it properly.** Do not pick the reading that lets you keep going.

1. Post `<!-- epic-256:decision:<slug> -->` on the issue: the question, why it arose, the real options with consequences, what depends on it, your recommendation.
2. **Commission spec research.** Spawn a read-only agent (`Bash`, `Read`, `Grep`, `Glob`) over `srs/srs/` records, `rfcs/`, `docs/schema/2.0/`, `docs/spec/`, and the invariants. Require one verdict:
   - **`RESOLVED`** — with citations: `file:line`, a record `instanceId`, or a rule id (`[R8]`, `Invariant 16`). No citation means not resolved.
   - **`UNRESOLVED`** — naming exactly what the spec is silent on.

   *If the Task tool is unavailable, do the research yourself to the same contract and say so.*
3. **`RESOLVED`** → post the answer with citations and carry on.
4. **`UNRESOLVED`** → this stops everything:
   ```bash
   gh issue edit <n> --repo the-greenman/<this-repo> --add-label "epic-256:blocked-owner"
   gh issue edit 256 --repo the-greenman/srs --add-label "epic-256:blocked-owner"
   ```
   Push whatever you have so it is not lost, comment what remains, and **stop**. Do not open a PR. Do not start another task.

Bias hard toward `UNRESOLVED`. This epic exists because a self-hosted model was being assembled out of decisions nobody had actually taken. A plausible inference the spec does not licence is the defect, not the fix — silence in the spec is a finding to report, not a gap for you to fill.

## Stage 5 — PR and CI watch

```bash
gh pr create --repo the-greenman/<this-repo> --base master \
  --title "<type>: <what> (#<n>)" --body "<summary>

Closes #<n>

## Gates
- srs repo validate: <result>
- validate-all: <result>
- check-release-drift: <result, or 'n/a — no record changes'>
- srs binary: <SHA, or 'n/a'>

## Skipped deliberately
<one line each, or 'none'>

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
```

`Closes #<n>` is **mandatory** — it is what closes the issue and what the coordinator matches PRs to rows.

Then hand over:

```bash
gh issue edit <n> --repo the-greenman/<this-repo> \
  --add-label "epic-256:in-review" --remove-label "epic-256:working"
```

Watch CI until required checks are green, pushing fixes as needed. Then **stop**. Do not merge, even if the class is `epic-256:auto-merge` and everything is green — the coordinator merges on its next tick. Do not schedule a follow-up.

If CI stays red on something you cannot resolve (external flake, infrastructure, a decision), comment the diagnosis on the PR, leave `epic-256:in-review`, and stop. The coordinator will surface it.

## Guardrails

- One task per run. Finish or stop — never start a second.
- Never merge, never close an issue by hand, never touch the epic body or ledger.
- Never remove `epic-256:blocked-owner` — the coordinator clears it after the owner answers.
- Never write `ready`, `promote:ready`, `priority: *`, or `status: in progress`.
- Never delete or force-push a branch you did not create; never edit a sibling repo's tree.
- Keep the issue timeline warm — a run with no comment, commit, or PR for 3h is treated as stalled and reclaimed.

## Output contract

- Which issue you claimed, or why you stopped (nothing dispatched / interlock / conflicting claim).
- Which route you took and why.
- Gate results, including anything deliberately skipped.
- Decisions raised: resolved with citation, or escalated to the owner.
- PR URL and final CI status.

If you stopped early, say exactly which stage and why.
