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
  `Must→P0 · Should→P1 · Could→P2 · Won't→excluded`.
- **Blank means Could.** A story with **no MoSCoW set** counts as **Could (P2)** — a value nobody
  has assigned yet is *not* an exclusion. Same for an epic with no Priority: it counts as **P2**.
  A blank must never make work invisible to the feed; issues used to get lost exactly this way
  (story missed the `user-story` label → never on the MoSCoW view → never valued → children
  unprioritised forever).
- An issue with **no story ancestry** but reachable from an **epic** → **epic fallback**: it
  inherits the epic's roadmap Priority **one tier down** (`P0→P1 · P1→P2 · P2→P2`). This is how
  **engineering work** (refactors, extension implementations, tooling, spec chores filed under an
  engineering epic) gets a derived priority without inventing artificial user stories: it rides
  its release's rank but sits below the release's story work by default. A bump label raises a
  specific issue back up.
- A **`bug`** with no story → **P1 floor** (bugs are fixed ASAP and are *never* lost). A
  release-blocking bug bumps to P0.
- An **orphaned non-bug** issue (under **no story and no epic**) gets the **default P2 floor** —
  it enters the feed at the bottom instead of vanishing — and is *still* **flagged** in the
  orphan report: the floor removes the loss, not the linking-hygiene signal. Link it to the
  story/epic it serves. This applies to plain work items in muDemocracy.org too (they are
  ordinary work items; only `user-story`/`epic`/`plan` labels exempt an issue from the rollup).
- **Bump one tier** (cap P0) when an issue carries a bump signal label: `critical-path`,
  `blocks-gate`, or (bug) `regression`.
- The **only** way an issue leaves the feed is an **explicit Won't** on *every* story it serves —
  the one deliberate opt-out. (A bug keeps its P1 floor even then.)

**Linkage = native GitHub sub-issues.** Make an implementation issue a sub-issue of the story
(or epic) it serves. Epics may sit in between; the rollup traverses transitively to the leaves.

**File issues linked — at creation time.** Any agent (or human) filing an implementation issue
must immediately parent it under the story or engineering epic it serves, in the same session that
files it. The sub-issues API is **plain REST**, so even proxy-bound cloud routines can do this:

```bash
# with the tool (CI/local or any REST-capable session):
gh-project link muDemocracy.org#48 srs-web#116        # link <parent-repo>#<n> <child-repo>#<n>

# raw REST equivalent (works everywhere gh works):
CHILD_ID=$(gh api repos/the-greenman/<child-repo>/issues/<child#> --jq .id)
gh api -X POST repos/the-greenman/<parent-repo>/issues/<parent#>/sub_issues -F sub_issue_id=$CHILD_ID
```

Deferred-from work links under the same parent as the issue it was deferred from. If no story or
epic clearly applies, say so explicitly in your summary so it surfaces in the next `coverage`
audit — do not guess a parent.

## Epics are releases

An **epic** (label `epic`, in muDemocracy.org) **is a release** — there is no separate release
entity. Each epic carries two hand-set inputs and nothing else:

- its **Release** field value — the epic's identity (`02 Governance app`, `04 Generic Semantic
  Editor`, …). The **leading `NN` prefix is load-bearing**: it is the epic's **roadmap sequence**,
  read by the feed and every epic listing (falls back to an `Epic NN:` title prefix; an epic with
  neither is flagged `missing-roadmap-number` and sorts after numbered epics in its tier).
  Renumbering the roadmap = renaming the Release option / retitling the epic.
- its **Priority** (P0/P1/P2) — the epic's urgency tier, which overrides roadmap sequence.

Epic ordering everywhere (the feed, `epics`, `tree`) is: **Priority tier → started epics first
(continuity) → roadmap `NN` prefix → issue number**. Issue numbers are filing chronology and only
break final ties.

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
node /tmp/gh-project.mjs sync                                          # the whole hourly pipeline, one process
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
node /tmp/gh-project.mjs topup                                         # dry-run: what would be promoted
node /tmp/gh-project.mjs topup --fix                                   # write promote:ready to fill Ready to target depth
node /tmp/gh-project.mjs promote --fix                                 # convert promote:ready intents to board Status=Ready
```

## Sizing & implementation bands

**Size is a first-class, maintained input** — an effort estimate that is *not* derivable, set by a
human/agent at triage via `gh-project size <repo> <#> <small|medium|large|xl>` (writes the `size:` label
**and** the board Size field). `coverage`/`reconcile` flag `unsized` leaf issues so nothing is left
unsized. Weighting decays if unmaintained, so the **SRS issue assessment** routine re-runs sizing on a
schedule and `/triage` sizes anything new.

`bands [--count N]` (default 10) prints the whole task list as an **implementation order** sliced into N
**equal-effort bands**: ordered by **epic (Priority → started → roadmap prefix) → MoSCoW-derived
priority → sub-issue position**, and
weighted by the `size:` label (unsized ⇒ medium). Leaves under no epic are listed as a trailing *unlinked*
group (link them via `/triage`). `bands --assign` writes each band onto the **Iteration** field (band k →
the k-th upcoming iteration) — but GitHub can't create iterations via API, so you first create N iterations
in the UI; the tool assigns whatever exists and reports how many more to add.

The tool **self-discovers** the project field/option/iteration IDs — never hardcode them in a
prompt or doc. `node /tmp/gh-project.mjs fields` dumps them if you need to inspect.

## API budget

The tool runs hourly and against a rate-limited API, so call volume is a design constraint:

- The whole board is read in **one paginated GraphQL query** (cached per process).
- Sub-issue edges are prefetched in **batched GraphQL queries** (~40 issues per request,
  transitive closure) instead of one REST call per issue — a full-board walk costs a handful of
  requests, not hundreds. If GraphQL is unavailable (proxy-bound session), it falls back to
  per-issue REST automatically.
- The hourly Action runs **`sync`** — the whole pipeline in one process on one fetch. Never split
  it back into per-step invocations: six separate processes re-fetching everything is how the
  quota used to burn out.
- Every subprocess call has a **timeout** — a hung network call fails the run loudly and the next
  hourly run retries; nothing wedges.

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
| `status: in progress` | claimed / in flight | "Do the SRS jobs" routine (reclaimed by `stale-claims --fix` if the claim goes stale — see below) |
| `blocked` | not feedable — **derived** from native blocked-by dependencies when edges exist (auto-cleared when the last blocker closes); hand-set **only** for non-issue blocks | `reconcile --fix` (derived) / human (external blocks only) |
| `needs-input` | stopped at a **human gate** — the question is in the issue comments; excluded from topup/consumable-Ready, never auto-reclaimed; **remove the label after answering** to re-feed | the stopping agent/routine (with the question as a comment) / human clears it |

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

### Auto-topup (Backlog → promote:ready)

`gh-project topup [--fix] [--target N]` keeps the Ready queue at a target depth (default **6** —
deep enough to survive GitHub's cron landing 25–40 min late, a repo-mixed queue head, and a burst
of completions between refills; 3 starved by evening in practice, #715 —
overridable via `--target N` or the `GHP_TOPUP_TARGET` environment variable). It runs **before**
`promote --fix` in `board-sync`, so any intents it writes are immediately realized on the same run:

- Counts `readyCount`: board issues that are OPEN and either Status=Ready or already carry
  `promote:ready`.
- Builds candidates: OPEN work-items (not epic/plan/user-story) with Status=Backlog or no Status,
  **excluding** blocked issues (open blocked-by edges are checked directly, else the `blocked`
  label — see "Dependencies" below), issues with an existing `promote:ready` intent, and
  Won't-exclusions (every served story explicitly Won't).
- Orders candidates in **implementation order** — epic-major: **epic Priority → started epics
  first within a tier → roadmap `NN` prefix → issue priority → sub-issue position**. This is the
  **epic-continuity rule**: once an epic has begun (any descendant claimed, in review, done, or
  closed), the feed drains it to completion before opening the next epic in the same tier — and
  new epics open in **roadmap sequence** (the `NN` prefix), not filing chronology. The old flat
  priority sort interleaved epics — a new epic's P1 work preempted the current epic's P2 tail,
  and the board filled with half-finished epics.
- Nominates the top `deficit = max(0, target − readyCount)` candidates and writes `promote:ready`
  to each. If fewer candidates exist than the deficit, all are nominated — no error.

### Dependencies (blocked by)

Blocking is recorded as **native GitHub issue dependencies** — the same class of structure as
sub-issues, and like them writable over **plain REST**, so proxy-bound judge routines can create
edges. Cross-repo within the owner works. Write them with the tool or raw REST:

```bash
gh-project dep add srs-web#116 srs-rust#330      # srs-web#116 is blocked by srs-rust#330
gh-project dep rm  srs-web#116 srs-rust#330      # remove the edge

# raw REST equivalent (works in proxy-bound routines):
BLOCKER_ID=$(gh api repos/the-greenman/<blocker-repo>/issues/<blocker#> --jq .id)
gh api -X POST repos/the-greenman/<blocked-repo>/issues/<blocked#>/dependencies/blocked_by -F issue_id=$BLOCKER_ID
```

**Ownership rule (one sentence):** if an issue has *any* blocked-by edges, the `blocked` label is
**derived** — `reconcile` sets it while a blocker is open and auto-clears it when the last blocker
closes; if it has *no* edges, the label is **human-owned** (external, non-issue blocks: a pending
decision, a third-party dependency) and the tool never touches it.

This is why an edge beats a bare label: **it unblocks itself.** `topup` checks edges directly, so
a freshly-unblocked issue is feedable on the very next hourly run — no judge has to notice that a
prerequisite closed. Judges should record an issue-blocker as an edge instead of hand-setting
`blocked`, and reserve the bare label for blocks that aren't issues.

Scope note: blocked-by is deliberately the **only** relation the tooling adopts. Intra-epic build
order is owned by **sub-issue position** (one ordering primitive — no parallel `requires`/`precedes`
graph), and relations without a pipeline consumer are not added.

The `blocked` label remains part of `MIRROR_LABELS`, so `ensureLabels` creates it in all ecosystem
repos.

```bash
node /tmp/gh-project.mjs topup                  # dry-run: shows what would be nominated
node /tmp/gh-project.mjs topup --target 5       # dry-run with a different target depth
node /tmp/gh-project.mjs topup --fix            # write promote:ready intents
node /tmp/gh-project.mjs topup --fix --target 0 # fill to 0 (no-op, safe check)
```

The `board-sync` Action runs `gh-project sync` — the whole pipeline (stories-sync → rollup →
release-sync → topup → promote → stale-claims → reconcile) in **one process on one board fetch**
(see "API budget") — on three triggers: **push** to master (post-merge), an **hourly schedule**
(so the queue refills even during quiet periods, matching the hourly consumer), and
**workflow_dispatch** — a cloud session can label issues then kick promotion immediately with
`gh workflow run board-sync.yml --repo the-greenman/srs-rust` (REST, allowed) instead of waiting
for the schedule.

**There is exactly one board-sync workflow** — `srs-rust/.github/workflows/board-sync.yml`. It
reconciles the entire cross-repo board on every run, so no other repo needs a copy (a copy without
its own secret silently no-ops, which is worse than no copy). It authenticates with a PAT with
`project` scope stored as the **`BOARD_SECRET`** repo secret in srs-rust (`BOARD_TOKEN` also
accepted); if the secret is missing the job **fails loudly** rather than skipping.

## Recovering a stale claim (In progress → Ready)

Nothing in the pipeline above ever revisits an issue once it reaches **In progress**: `promote`
explicitly leaves advanced statuses alone (that's "never demotes," above) and `reconcile` only
mirrors the `status: in progress` label, it never changes Status itself. So if whatever claimed the
issue — the "Do the SRS jobs" routine, another consumer, a human — crashes, times out, or is
interrupted mid-task, the claim is permanent: the issue isn't Backlog (so `promote` can't touch it)
and isn't Ready (so the queue consumer never picks it up again).

`gh-project stale-claims [--hours N] [--fix]` closes that gap, and staleness is **activity-aware**
(#715):

1. Finds every OPEN board item claimed in-progress (board Status or the label).
2. For each, reads the issue's REST timeline once, extracting **two** timestamps: when
   `status: in progress` was last applied (the claim's start), and the last **real activity** —
   a comment, or a commit/PR referencing the issue (`commented` / `cross-referenced` /
   `referenced` events). Label churn from the hourly sync deliberately does **not** count as
   activity, or nothing would ever look stale.
3. Age runs from the **later** of the two. Anything past the threshold (default **3h** —
   consumers are single-session and terminal-at-PR, so a claim with no comments, commits, or PR
   mentions for 3 hours is dead, not slow; a genuinely long-running task refreshes itself by
   committing/commenting) is reset to **Status=Ready**, the `ready` mirror set immediately, and a
   comment left noting the auto-reclaim.
4. An issue labeled **`needs-input`** is never reclaimed — it is legitimately paused on a human,
   and re-feeding it would just hit the same gate. It is reported separately.
5. A claim whose label event can't be resolved is reported as `unknown` rather than silently
   ignored or wrongly reclaimed — it needs a human look.

Idempotent and safe to re-run: a fresh claim is left alone; a reclaimed issue simply won't match
"In progress" on the next pass. Wired into `board-sync`'s three triggers (push/hourly/dispatch)
right after `promote --fix`, so a dead claim recovers within ~2 hours end to end.

```bash
gh-project stale-claims               # dry-run: which claims would be reclaimed
gh-project stale-claims --hours 12    # looser window if long human claims are expected
gh-project stale-claims --fix         # reset stale claims to Ready + comment
```

### Issues waiting on a human (`needs-input`)

An automatic session cannot ask a question. When it hits a **human gate** — a decision to make, a
missing secret, an ambiguity it must not guess at — the protocol is: **comment the specific
question, add `needs-input`, remove `status: in progress`, stop.** From there:

- topup and the consumable-Ready count **exclude** it (a gated issue must not clog the queue), and
  `stale-claims` never touches it;
- the **morning progress-review routine lists all `needs-input` issues first** — that report is
  the daily "waiting on you" list;
- on the **project board**, filter any view with `label:needs-input` (or save a dedicated view
  with that filter — views are UI-only, the API can't create them);
- a human **answers in the comments and removes the label** — that alone puts the issue back in
  the feed on the next hourly run.

```bash
gh-project promote            # dry-run: what the intents would do
gh-project promote --fix      # set Status=Ready for promote:ready issues, clear the intent
```

## Understanding a priority estimate (the stages)

Every implementation issue's `priority: Pn` is *derived*, and the derivation is fully explainable
— `summary` shows all estimates with the stages; `explain <repo> <#>` walks one issue through them:

1. **served stories** — walk the sub-issue graph up to the user stories the issue serves.
2. **MoSCoW → P** — each served story's value maps:
   `Must→P0 · Should→P1 · Could→P2 · blank→Could (P2) · Won't→excluded`.
3. **base** — the **highest** (most urgent) P across the served stories.
4. **epic fallback** — no story: inherit the claiming epic's Priority **one tier down**
   (`P0→P1 · P1→P2 · P2→P2`; a blank epic Priority counts as P2); a diamond is claimed by the
   higher-Priority epic.
5. **bug floor** — a `bug` is never weaker than **P1**, even with no story.
6. **default floor** — an orphan (no story, no epic, not a bug) gets **P2** — nothing is ever
   left unprioritised; it stays flagged as orphaned until linked.
7. **bump** — **+1 tier** (capped at P0) if the issue carries a label in
   `{critical-path, blocks-gate, regression}`.
8. **final** — the derived priority, written as the `priority: Pn` label + the board Priority
   mirror. An explicit **Won't** on every served story ⇒ excluded (no priority, out of the feed).

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

## Weekly story-planning audit

Stories that are *just stories* — no implementation sub-issues — produce no work items, so they sit
outside the delivery feed until someone breaks them down. The **SRS Story Planning Auditor** cloud
routine (Mondays 08:00 UTC) keeps them visible and easy to plan:

- **Tags** every open `user-story` with zero sub-issues as **`story:unplanned`** in muDemocracy.org,
  and clears the tag once the story gains children. The routine is the label's **only writer** —
  it is *not* part of the gh-project mirror set and gh-project neither reads nor writes it.
- **Suggests** an implementation approach as an issue comment where intent is derivable from
  context (story body, epic, planned sibling stories, repo docs). When an epic has **≥2 unplanned
  stories, they are planned together**: one joint comment on the epic (shared components, repo
  split per capability-layering, build order, rough sizes) so the breakdown is coherent, with
  pointer comments on the covered stories. Comments carry the marker
  `<!-- srs-story-planning-audit -->` and are updated in place, never duplicated.
- **Reports** into the "Story planning radar" issue in muDemocracy.org (labeled `plan`, which
  keeps it out of the delivery feed): unplanned stories grouped by epic, plus a "needs owner
  input" list for stories whose intent can't be derived — those need a body, not a guess.

The suggestions are **proposals** — a human (or `/stories` / `/triage`) ratifies them by filing
and linking the implementation issues. The routine never files or links issues and never touches
priority/status/promotion labels or the board.

## Skills

- `/triage <scope>` — sync stories, `rollup --fix`, set readiness + iteration, reconcile, report.
- `/stories` — maintain the story layer; surface missing MoSCoW, bugs, unlinked work, coverage.
- `/roadmap <program>` — sequence a program's issues into iterations by gate/phase.

## Relationship to `problem-index/`

`problem-index/priorities.md` is **strategic/research** priority over *problems* (P0–P3). This
board priority is **delivery** priority over *issues*, derived from user-story value. They are
intentionally separate.
