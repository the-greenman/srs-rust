# Project & priority management

How issues across the SRS ecosystem are prioritised and tracked. This is the **canonical**
copy (lives in `srs-rust`); the other code repos carry a short pointer in their `CLAUDE.md`.
The process is **story-driven**, **agent-runnable**, and works inside an **isolated single-repo**
checkout — every operation hits the GitHub API, so nothing depends on a sibling repo on disk.

## The one board

Everything lives on **Project #5 "SRS"** (`https://github.com/users/the-greenman/projects/5`).
User stories and implementation issues coexist on it.

## The priority model (top-down)

```
USER STORY  (muDemocracy.org, label `user-story`)        ← the human layer, on board #5
   MoSCoW field:  Must / Should / Could / Won't           ← value input, set by a human in the UI
   Milestone:     safe-to-try | decision-logger-v1 | …    ← release window
        │  native GitHub sub-issues (cross-repo)
        ▼
IMPLEMENTATION ISSUE  (srs / srs-rust / srs-vscode / srs-web)
   priority: Pn   ← DERIVED label (computed, never hand-set) + board Priority mirror
   Status         ← Ready iff unblocked; else Backlog
   Iteration      ← gate/phase, bounded by the story's release window
```

**Priority is derived, not hand-set.** A human expresses value once, as **MoSCoW on the story**.
The tool rolls that down to implementation issues:

- An impl issue that **serves ≥1 story** → priority = **highest** served MoSCoW:
  `Must→P0 · Should→P1 · Could→P2 · Won't→none`.
- A **`bug`** with no story → **P1 floor** (bugs are fixed ASAP and are *never* lost). A
  release-blocking bug bumps to P0.
- **Bump one tier** (cap P0) when an issue carries a bump signal label: `critical-path`,
  `blocks-gate`, or (bug) `regression`.
- An **unlinked non-bug** issue (no parent story) gets **no** derived priority and is **flagged**
  in the "could get lost" report — link it to a story or justify it. Nothing is silently dropped.

**Linkage = native GitHub sub-issues.** Make an implementation issue a sub-issue of the story
(or epic) it serves. Epics may sit in between; the rollup traverses transitively to the leaves.

## Status lifecycle

`Backlog → Ready → In progress → In review → Done`. **Ready = unblocked** (dependencies resolved
/ gate passed). Closed issues should be `Done` (the tool reconciles this).

## Iterations

Iterations are the delivery windows. **GitHub has no API to create iterations** — add new ones in
the project UI; the tool only *assigns* existing ones.

## The tool

`gh-project` is a single-file, zero-dependency Node CLI wrapping `gh`. Fetch the released artifact
(works in any isolated checkout) and run it:

```bash
gh release download --repo the-greenman/srs-rust --pattern gh-project.mjs \
  --output /tmp/gh-project.mjs --clobber
node /tmp/gh-project.mjs help
```

Common commands:

```bash
node /tmp/gh-project.mjs board --repo srs-rust --status Ready --open   # the work queue
node /tmp/gh-project.mjs rollup                                        # dry-run: derived priorities
node /tmp/gh-project.mjs rollup --fix                                  # apply labels + board mirror
node /tmp/gh-project.mjs summary                                       # priority estimates + the calc stages
node /tmp/gh-project.mjs explain srs-rust 263                          # stage-by-stage for one issue
node /tmp/gh-project.mjs coverage                                      # bugs / unlinked / uncovered audit
node /tmp/gh-project.mjs release-sync                                  # set Release from labels + parentage
node /tmp/gh-project.mjs tree 30                                       # story → sub-issue tree
node /tmp/gh-project.mjs set srs-rust 263 --status "In progress"       # board write
node /tmp/gh-project.mjs reconcile --fix                               # repair drift
```

The tool **self-discovers** the project field/option/iteration IDs — never hardcode them in a
prompt or doc. `node /tmp/gh-project.mjs fields` dumps them if you need to inspect.

## Understanding a priority estimate (the stages)

Every implementation issue's `priority: Pn` is *derived*, and the derivation is fully explainable
— `summary` shows all estimates with the stages; `explain <repo> <#>` walks one issue through them:

1. **served stories** — walk the sub-issue graph up to the user stories the issue serves.
2. **MoSCoW → P** — each served story's value maps: `Must→P0 · Should→P1 · Could→P2 · Won't→(none)`.
3. **base** — the **highest** (most urgent) P across the served stories.
4. **bug floor** — a `bug` is never weaker than **P1**, even with no story.
5. **bump** — **+1 tier** (capped at P0) if the issue carries a label in
   `{critical-path, blocks-gate, regression}`.
6. **final** — the derived priority, written as the `priority: Pn` label + the board Priority mirror.

`summary [--repo R] [--release X] [--brief]` prints the stage legend, TOTALS (P0/P1/P2 + bugs /
unlinked / uncovered counts), a BY RELEASE breakdown, and a per-issue stage table. Use it to sense-
check the model — e.g. "everything is P0" usually means every story is set **Must**.

## Agents vs humans

- **Human (board UI):** sets story **MoSCoW + release**; links impl issues as sub-issues; adds iterations.
- **Interactive/local agents:** use the **GitHub MCP** for issues/labels/comments/sub-issues/search,
  and `gh-project` for board fields.
- **Cloud routines:** use `gh` + `gh-project` only (no interactively-authenticated MCP — it may be
  absent headless).
- **`gh-project` is the only writer of Projects v2 fields** (Status/Priority/Iteration/MoSCoW).

## Skills

- `/triage <scope>` — sync stories, `rollup --fix`, set readiness + iteration, reconcile, report.
- `/stories` — maintain the story layer; surface missing MoSCoW, bugs, unlinked work, coverage.
- `/roadmap <program>` — sequence a program's issues into iterations by gate/phase.

## Relationship to `problem-index/`

`problem-index/priorities.md` is **strategic/research** priority over *problems* (P0–P3). This
board priority is **delivery** priority over *issues*, derived from user-story value. They are
intentionally separate.
