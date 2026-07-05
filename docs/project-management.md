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
EPIC  (muDemocracy.org, label `epic`)                    ← an epic IS a release (1:1)
   Release field: its own identity (Decision Logger v1 | …)   set once by a human in the UI
   Priority:      P0 / P1 / P2                            ← the epic's roadmap rank (hand-set)
        │  native GitHub sub-issues
        ▼
USER STORY  (muDemocracy.org, label `user-story`)        ← the human value layer, on board #5
   MoSCoW field:  Must / Should / Could / Won't           ← value input, set by a human in the UI
   Release        ← DERIVED: inherited from the parent epic (never hand-set)
        │  native GitHub sub-issues (cross-repo)
        ▼
IMPLEMENTATION ISSUE  (srs / srs-rust / srs-vscode / srs-web)
   priority: Pn   ← DERIVED label (computed, never hand-set) + board Priority mirror
   Release        ← DERIVED: inherited from the ancestor epic
   Status         ← Ready iff unblocked; else Backlog
   Iteration      ← gate/phase, bounded by the release
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

## Epics are releases

An **epic** (label `epic`, in muDemocracy.org) **is a release** — there is no separate release
entity. Each epic carries two hand-set inputs and nothing else:

- its **Release** field value — the epic's identity (`Decision Logger v1`, `Governance app`, …);
- its **Priority** (P0/P1/P2) — the epic's rank on the roadmap.

Everything below the epic **inherits its Release**: `release-sync` walks each epic's sub-issue graph
and stamps the epic's Release onto every descendant story and implementation issue. So **Release is
derived, never hand-set on a child** — the only manual release input is on the epic. A node reachable
from two epics is claimed by the **higher-Priority** epic (ties: lower issue number). There is **no
release label** — Release lives only as the board field (add a `release:<slug>` mirror later only if
some REST-only consumer ever needs it).

This makes the board's release views fall out of one groupable field:

- **Roadmap** — filter `label:epic`, sort by Priority: epics as prioritised releases.
- **Swimlanes / drill-down** — group any items by **Release** to see stories/issues per epic.

Every **user-story should sit under an epic**. `coverage` reports `orphan_stories_no_epic` (a story
with no epic ancestor — its release can't be derived); link it with `epic add-story <epic#> <story#>`.

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
node /tmp/gh-project.mjs coverage                                      # bugs / unlinked / uncovered / orphan-stories
node /tmp/gh-project.mjs epics                                         # roadmap: epics (=releases) by Priority + coverage
node /tmp/gh-project.mjs epic set 30 --priority P0 --release "Decision Logger v1"  # an epic's rank + identity
node /tmp/gh-project.mjs epic add-story 30 21                          # link a story under an epic (sub-issue)
node /tmp/gh-project.mjs release-sync --dry-run                        # preview each descendant's derived Release
node /tmp/gh-project.mjs release-sync                                  # propagate epic Release to descendants
node /tmp/gh-project.mjs tree                                          # whole-board tree (epics → stories → issues)
node /tmp/gh-project.mjs tree 30                                       # one story/epic's sub-issue tree
node /tmp/gh-project.mjs size srs-rust 263 medium                      # effort estimate (label + board Size field)
node /tmp/gh-project.mjs bands                                         # implementation order in 10 effort-bands
node /tmp/gh-project.mjs bands --assign --dry-run                      # preview band → iteration assignment
node /tmp/gh-project.mjs set srs-rust 263 --status "In progress"       # board write
node /tmp/gh-project.mjs reconcile --fix                               # repair drift
node /tmp/gh-project.mjs ensure-labels [--repo R]                      # create the mirror label set
```

## Sizing & implementation bands

**Size is a first-class, maintained input** — an effort estimate that is *not* derivable, set by a
human/agent at triage via `gh-project size <repo> <#> <small|medium|large|xl>` (writes the `size:` label
**and** the board Size field). `coverage`/`reconcile` flag `unsized` leaf issues so nothing is left
unsized. Weighting decays if unmaintained, so the **SRS issue assessment** routine re-runs sizing on a
schedule and `/triage` sizes anything new.

`bands [--count N]` (default 10) prints the whole task list as an **implementation order** sliced into N
**equal-effort bands**: ordered by **epic Priority → MoSCoW-derived priority → sub-issue position**, and
weighted by the `size:` label (unsized ⇒ medium). Leaves under no epic are listed as a trailing *unlinked*
group (link them via `/triage`). `bands --assign` writes each band onto the **Iteration** field (band k →
the k-th upcoming iteration) — but GitHub can't create iterations via API, so you first create N iterations
in the UI; the tool assigns whatever exists and reports how many more to add.

The tool **self-discovers** the project field/option/iteration IDs — never hardcode them in a
prompt or doc. `node /tmp/gh-project.mjs fields` dumps them if you need to inspect.

## The plain-label mirror set

The scheduled **cloud routines** can't reach Projects v2 GraphQL through the web-session proxy, so
they read and write a **plain-label mirror** of the board state instead. That set must exist in
**every** repo a routine touches, or a `gh issue edit --add-label` hard-fails and the routine dies.
The canonical set:

| Label | Meaning | Written by |
|---|---|---|
| `ready` | board Status=Ready mirror — the work queue | `promote --fix` / `reconcile --fix` (mirror; **not** by hand) |
| `promote:ready` | promotion **intent** — judged unblocked, awaiting the privileged board write | the judge: progress-review routine / human / rule (REST) |
| `priority: P0` / `P1` / `P2` | derived priority (§ the priority model) | `rollup --fix` |
| `status: in progress` | claimed / in flight | "Do the SRS jobs" routine |

`gh-project` is the single source of truth for this set (`MIRROR_LABELS` in the script) and creates
any missing labels on demand — so it **can't drift**:

- `ensure-labels [--repo R] [--dry-run]` creates the full set in every ecosystem repo (`MIRROR_REPOS`,
  env-overridable via `GHP_MIRROR_REPOS`), or just one `--repo`. Idempotent (`gh label create --force`).
- `rollup --fix` and `reconcile --fix` ensure the set across all `MIRROR_REPOS` before applying
  changes, so any routine run self-heals missing labels. Dry-run makes no changes.

**The values are mirrored from the board, two fields:**

| Board field | Mirror label | Written by |
|---|---|---|
| Priority `P0/P1/P2` | `priority: Pn` | `rollup --fix` (derived from stories), `set --priority` |
| Status `Ready` | `ready` | `reconcile --fix`, `set --status` |
| Status `In progress` | `status: in progress` | `reconcile --fix`, `set --status` |

The routines **cannot read Projects v2 Status/Priority through the proxy** — the label *is* the
signal. `reconcile --fix` mirrors board Status → label for every item (`status-mirror-stale` drift):
open items in `Ready`/`In progress` get the matching label; every other status (and every closed
issue) has both status labels cleared. Without this pass a board-`Ready` issue carries no `ready`
label and is invisible to the work-queue routine.

## Promotion pipeline (Backlog → Ready)

Promotion is **split into judgment and a privileged write** so the two halves never fight. Only the
board is the source of truth for readiness; the `ready` label is a *mirror* of it (above). But the
cloud routines can't set board Status (Projects v2 is proxy-blocked for them) — so if a routine wrote
the `ready` label directly, the next `reconcile` would wipe it (board Status still says Backlog). The
`promote:ready` **intent** label resolves this:

1. **Judge (REST-only).** The progress-review routine — or a human, or a future deterministic rule —
   marks each issue it deems **unblocked** (dependencies resolved / gate passed) with `promote:ready`.
   That's a plain-label write, allowed through the proxy. The judge **never** writes `ready` or sets
   Status.
2. **Promote (privileged).** `gh-project promote --fix`, run where Projects v2 is reachable (the
   `board-sync` GitHub Action, or locally), converts every open Backlog issue carrying `promote:ready`
   to board **Status=Ready**, mirrors the `ready` label, and clears the intent. It is idempotent and
   never demotes: already-advanced (In progress / In review / Done), already-Ready, and closed issues
   just have the stale intent cleared. `reconcile --fix` keeps the `ready` mirror in sync thereafter.

The `board-sync` Action runs `promote --fix` on three triggers: **push** to master (post-merge),
an **hourly schedule** (so the queue refills even during quiet periods, matching the hourly consumer),
and **workflow_dispatch** — a cloud session can label issues then kick promotion immediately with
`gh workflow run board-sync.yml --repo the-greenman/srs-rust` (REST, allowed) instead of waiting
for the schedule.

**There is exactly one board-sync workflow** — `srs-rust/.github/workflows/board-sync.yml`. It
reconciles the entire cross-repo board on every run, so no other repo needs a copy (a copy without
its own secret silently no-ops, which is worse than no copy). It authenticates with a PAT with
`project` scope stored as the **`BOARD_SECRET`** repo secret in srs-rust (`BOARD_TOKEN` also
accepted); if the secret is missing the job **fails loudly** rather than skipping.

```bash
gh-project promote            # dry-run: what the intents would do
gh-project promote --fix      # set Status=Ready for promote:ready issues, clear the intent
```

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

- **Sizing:** a human/agent sets each leaf issue's **size** (`gh-project size`) at triage — an effort
  estimate, not derivable; the assessment routine keeps it fresh. Bands weight on it.
- **Human (board UI):** sets each **epic's Release identity + Priority** and story **MoSCoW**; links
  stories under epics and impl issues under stories (sub-issues); adds iterations. Child **Release is
  derived** (`release-sync`), never hand-set.
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
