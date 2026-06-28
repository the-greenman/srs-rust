---
description: Story-driven triage — derive issue priority from user-story MoSCoW, set readiness + iteration, report.
argument-hint: "<scope: a story #N, a repo, or 'all'> (default: all)"
allowed-tools: Bash, Read
---

# /triage — story-driven priority pass

Scope: **$ARGUMENTS** (a story `#N`, a repo name, or `all`; default `all`).

Priority is **derived from user stories**, never hand-set. See "Project & priority management" in
this repo's `CLAUDE.md`. Run the released tool; do not re-implement its logic.

## Stage 0 — Fetch the tool

```bash
gh release download --repo the-greenman/srs-rust --pattern gh-project.mjs \
  --output /tmp/gh-project.mjs --clobber
node /tmp/gh-project.mjs help >/dev/null && echo "tool ready"
```

## Stage 1 — Ensure stories are on the board

```bash
node /tmp/gh-project.mjs stories sync
```

Then check for stories missing a MoSCoW value:

```bash
node /tmp/gh-project.mjs coverage
```

If any story lacks MoSCoW, **stop and report** which ones — a human must set MoSCoW in the board
UI before priorities can be derived. (You may propose a MoSCoW per story for them to confirm.)

## Stage 2 — Derive priorities

```bash
node /tmp/gh-project.mjs rollup            # dry-run: review the derivation
node /tmp/gh-project.mjs rollup --fix      # apply priority labels + board Priority mirror
```

## Stage 3 — Readiness + iteration

For each implementation issue in scope, set Status and Iteration using the tool:

- **Ready** iff unblocked (dependencies resolved / gate passed); else leave **Backlog**.
- Assign an **Iteration** by the program's gate/phase, bounded by the served story's release.

```bash
node /tmp/gh-project.mjs set <repo> <issue#> --status Ready --iteration "Iteration 4"
```

Never set Status=Ready on an unlinked-non-bug issue — surface it instead (Stage 4).

## Stage 4 — Reconcile + report

```bash
node /tmp/gh-project.mjs reconcile --fix
```

Report, grouped:
1. **Per-iteration** table of in-scope issues (key, priority, status).
2. **Bugs — fix ASAP** lane (bugs with no story, P1 floor / P0 if release-blocking).
3. **Unlinked — could get lost** (non-bug, no story) — propose a story to link each to.
4. **Uncovered stories** (no implementation children yet).

Do not invent priority by hand — if the rollup can't derive it, it belongs in lane 2 or 3.
