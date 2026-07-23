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

## Stage 2 — Derive priorities and releases

```bash
node /tmp/gh-project.mjs rollup            # dry-run: review the derivation
node /tmp/gh-project.mjs rollup --fix      # apply priority labels + board Priority mirror
```

`coverage` also reports `orphan_stories_no_epic` — stories under no epic, whose Release can't be
derived. Link each with `epic add-story <epic#> <story#>` (epics are releases; see `epics`).

## Stage 2b — Size unsized work

`coverage` reports `unsized_issues` — open leaf issues with no `size:` label. Size is an **effort
estimate**, not derivable: judge each from its scope and assign one. The implementation-order bands
(`bands`) weight on it, so leaving work unsized degrades the plan.

```bash
node /tmp/gh-project.mjs size <repo> <issue#> <small|medium|large|xl>   # label + board Size field
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
node /tmp/gh-project.mjs summary            # priority estimates + the calculation stages
```

Report, grouped:
1. **Priority estimate summary** — the `summary` output (TOTALS, BY RELEASE, stage table). If a
   single issue's priority looks surprising, run `explain <repo> <#>` to show its six stages.
2. **Per-iteration** table of in-scope issues (key, priority, status).
3. **Bugs — fix ASAP** lane (bugs with no story, P1 floor / P0 if release-blocking).
4. **Unlinked — could get lost** (non-bug, no story) — propose a story to link each to.
5. **Uncovered stories** (no implementation children yet).

Do not invent priority by hand — if the rollup can't derive it, it belongs in lane 3 or 4.
