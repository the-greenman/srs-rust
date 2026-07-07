# Plan: Ready queue auto-topup (#386)

> Lightweight plan. This change is entirely within the Node project-tooling script
> `scripts/gh-project.mjs` and `.github/workflows/board-sync.yml` — not a Rust crate.
> The crate/ADR/payload/schema/dogfood stages of the standard pipeline are N/A below.

## Summary

The Ready queue starves because `board-sync.yml` runs `promote --fix` (which only *converts*
existing `promote:ready` intents into board Status=Ready) but nothing ever *writes* new intents
when the queue drains. The daily "SRS issue progress review" cloud routine is the sole intent
producer; the hourly consumer ("Do the SRS jobs") drains Ready faster than once a day. This plan
fixes the starvation by adding a `topup` command that keeps the Ready queue at a target depth
(default 3) by writing `promote:ready` intents to the highest-priority unblocked Backlog leaves —
and a `blocked` label signal so topup never auto-promotes work whose prerequisites are unmet.

The fix is two coordinated pieces:
1. **`blocked` label** — a machine-readable "do not auto-promote" signal added to `MIRROR_LABELS`.
2. **`topup` command** — writes `promote:ready` intents to fill Ready to the target depth; preceded
   by `promote --fix` in `board-sync.yml` so the intents are immediately realized on the same run.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | this session (direct) |
| Verification | this session + `node --test` |

Single-file script change plus one workflow line; no fan-out needed.

## Architecture Decisions

No new ADRs. ADRs govern Rust crate boundaries and the CLI payload contract;
`scripts/gh-project.mjs` is a standalone zero-dependency Node tool outside that surface. The
relevant governing doc is `docs/project-management.md`, updated in Stage 7.5.

_Existing ADR coverage_: N/A (no Rust crate logic touched). The `blocked` label design mirrors the
existing `promote:ready` intent-label pattern (REST-only, not a Projects v2 board field) — no new
principle, same label-first architecture.

## Contracts

- **CLI output contract (ADR-011):** N/A — no Rust CLI command or `payload.rs` struct touched.
- **Entity schema sync:** N/A — no files under `srs/docs/schema/2.0/` touched.

---

## Scope

- Add `blocked` to `MIRROR_LABELS` so `ensureLabels` creates it in all repos on every run.
- Add `TOPUP_TARGET_DEFAULT = 3` constant (overridable via `--target N` flag or `GHP_TOPUP_TARGET` env).
- Add pure `planTopup(candidates, readyCount, target)` function — unit-testable, no side effects.
- Add `cmdTopup(argv)` command that reads the board, filters/sorts candidates, calls `planTopup`,
  and writes `promote:ready` to each nominee.
- Export `planTopup` and `TOPUP_TARGET_DEFAULT` for tests.
- Add `topup [--fix] [--target N]` to dispatch and `help`.
- Insert `node scripts/gh-project.mjs topup --fix` in `board-sync.yml` **before** `promote --fix`,
  so intents written by topup are realized on the same run.
- Add unit tests for `planTopup` in `scripts/gh-project.test.mjs` mirroring `planPromotions` style.
- Update `docs/project-management.md` to document the topup step and `blocked` label.

**Out of scope:**
- GitHub native `blockedBy` relations (Projects v2 GraphQL, proxy-blocked for routines; deferred).
- Per-epic target depth (global default sufficient for now).
- Automatic inference of blocked status from sub-issue ordering (manual `blocked` label is the
  correct first step; auto-inference can layer on top later).

---

## Phases

### Phase 1: `blocked` label + `planTopup` pure function + unit tests

**Goal:** The `blocked` label is part of `MIRROR_LABELS`, `planTopup` exists as a pure exported
function, and all new unit tests pass with `node --test scripts/gh-project.test.mjs`.

**Agent:** Lead Integrator

#### Tasks

- [ ] In `scripts/gh-project.mjs`, add `{ name: "blocked", color: "E4E669", description:
      "Unmet prerequisites — auto-topup skips this issue; remove when unblocked" }` to `MIRROR_LABELS`
      (after the last existing entry).
- [ ] Add `const TOPUP_TARGET_DEFAULT = 3;` near the other defaults (after `STALE_CLAIM_HOURS_DEFAULT`
      is a good spot).
- [ ] Add pure function `planTopup(candidates, readyCount, target)`:
  - `candidates`: pre-filtered, pre-sorted array of board rows (open workItems, Backlog/null status,
    not `blocked`, not `promote:ready` — caller's responsibility to filter/sort).
  - `readyCount`: number of issues currently filling the Ready slot (Status=Ready or has
    `promote:ready` label — supplied by caller from board state).
  - `target`: integer target queue depth.
  - Returns `{ toNominate: candidates.slice(0, deficit), deficit, currentReady: readyCount, target }`
    where `deficit = Math.max(0, target - readyCount)`.
  - Place just below `planStaleClaims` in the file.
- [ ] Export `planTopup` and `TOPUP_TARGET_DEFAULT` in the existing `export { ... }` statement.
- [ ] In `scripts/gh-project.test.mjs`, import `planTopup, TOPUP_TARGET_DEFAULT` from the module.
- [ ] Add these unit tests:
  - `TOPUP_TARGET_DEFAULT is a positive integer` — `assert.ok(Number.isInteger(TOPUP_TARGET_DEFAULT) && TOPUP_TARGET_DEFAULT > 0)`.
  - `planTopup returns empty toNominate when queue is already at target` — readyCount=3, target=3, one candidate row → `toNominate` is `[]`, `deficit` is 0.
  - `planTopup nominates up to deficit rows in order` — readyCount=1, target=3 (deficit=2), 5 candidates → `toNominate` has exactly 2 rows (the first two), order preserved.
  - `planTopup clamps when fewer candidates than deficit` — readyCount=0, target=5, 2 candidates → `toNominate` has 2 rows (all of them), no error.
  - `planTopup returns correct metadata fields` — check `result.deficit`, `result.currentReady`, `result.target` match inputs.

#### Acceptance Criteria

- [ ] `MIRROR_LABELS` contains an entry with `name: "blocked"`.
- [ ] `planTopup` and `TOPUP_TARGET_DEFAULT` appear in the `export { ... }` line.
- [ ] `node --test scripts/gh-project.test.mjs` passes with the new tests present.

#### Testing

```bash
node --test scripts/gh-project.test.mjs
```

Specific tests to write or verify:

- `TOPUP_TARGET_DEFAULT is a positive integer` — guards against mistyped constant.
- `planTopup returns empty toNominate when queue is already at target` — no over-promotion when full.
- `planTopup nominates up to deficit rows in order` — correct count and order.
- `planTopup clamps when fewer candidates than deficit` — never throws when backlog is thin.
- `planTopup returns correct metadata fields` — caller can log meaningful output.

#### Milestone gate

1. All acceptance criteria above are checked.
2. `node --test scripts/gh-project.test.mjs` exits 0 with the new tests present.
3. Commit:

```bash
git add scripts/gh-project.mjs scripts/gh-project.test.mjs
git commit -m "feat(gh-project): planTopup pure function + blocked label (#386)"
```

---

### Phase 2: `cmdTopup` command + dispatch + `board-sync.yml` update

**Goal:** `node scripts/gh-project.mjs topup --fix` is a working command that writes `promote:ready`
intents to fill the Ready queue to target depth; `board-sync.yml` runs it before `promote --fix`.

**Agent:** Lead Integrator

#### Tasks

- [ ] Add `function cmdTopup(argv)` in `scripts/gh-project.mjs`, placed after `cmdStaleClaims`:
  - Parse `--fix` (dryRun when absent), `--target N` (default `Number(process.env.GHP_TOPUP_TARGET) || TOPUP_TARGET_DEFAULT`).
  - When `!dryRun`, call `ensureLabels(repo)` for each `MIRROR_REPOS` entry (self-heal).
  - Count `readyCount`: iterate `board().values()`, count rows where `row.state === "OPEN"` AND
    (`row.status === "Ready"` OR `row.labels.includes(PROMOTE_INTENT_LABEL)`).
  - Build candidates: `[...board().values()].filter(row => row.state === "OPEN" && isWorkItem(row) && (row.status === "Backlog" || row.status == null) && !row.labels.includes("blocked") && !row.labels.includes(PROMOTE_INTENT_LABEL))`.
  - Sort candidates by priority label: extract `priority: Pn` label, use `pRank` to get a sort key
    (rows with no priority label sort last, using `pRank(null)` which returns 99).
  - Call `planTopup(candidates, readyCount, target)`.
  - For each row in `result.toNominate`:
    - Log: `"${dryRun ? "[dry-run] " : ""}topup: ${row.key} (${row.labels.find(l => l.startsWith("priority: ")) ?? "no priority"}) → promote:ready"`.
    - When `!dryRun`: `gh(["issue", "edit", String(row.num), "--repo", \`${OWNER}/${row.repo}\`, "--add-label", PROMOTE_INTENT_LABEL])`.
  - Log summary: `"Ready: ${readyCount} · target: ${target} · deficit: ${result.deficit} · nominated: ${result.toNominate.length}"`.
  - When `dryRun && result.toNominate.length > 0`: log `"(dry-run; pass --fix to nominate)"`.
- [ ] Add `case "topup": cmdTopup(rest); break;` to the dispatch switch (after `case "promote"`).
- [ ] Add `topup [--fix] [--target N]   keep Ready queue at target depth (default ${TOPUP_TARGET_DEFAULT}) by nominating` to `help()` output.
- [ ] In `.github/workflows/board-sync.yml`, insert `node scripts/gh-project.mjs topup --fix`
      as the **first** step inside the `run:` block, before `node scripts/gh-project.mjs promote --fix`.
- [ ] Update `docs/project-management.md`:
  - In the **"Promotion pipeline (Backlog → Ready)"** section, after the paragraph describing the judge/promote split,
    add a new subsection "**Auto-topup (Backlog → promote:ready)**" explaining: the `topup` command runs
    in `board-sync.yml` before `promote --fix`; it keeps Ready at target depth (default 3,
    `GHP_TOPUP_TARGET` env or `--target N`); it writes `promote:ready` to the highest-priority
    unblocked Backlog leaves; it skips issues with the `blocked` label.
  - In the **"The plain-label mirror set"** table, add a row for `blocked` with meaning "Unmet
    prerequisites — auto-topup skips this issue; remove when resolved" and "Written by" = "judge/human".
  - Add `topup [--fix] [--target N]` to the Common commands block (after `promote`).

#### Acceptance Criteria

- [ ] `node scripts/gh-project.mjs topup --dry-run` (no `--fix`) runs without shell calls and exits 0.
- [ ] `node scripts/gh-project.mjs topup --target 0` logs `deficit: 0` and makes no changes even with `--fix`.
- [ ] `node scripts/gh-project.mjs help` output includes the word `topup`.
- [ ] `board-sync.yml` contains `topup --fix` before `promote --fix`.
- [ ] `docs/project-management.md` has the `blocked` label row in the mirror-set table and the
      auto-topup subsection.

#### Testing

```bash
node scripts/gh-project.mjs topup --dry-run
node scripts/gh-project.mjs topup --target 0
node scripts/gh-project.mjs help | grep topup
grep -n "topup" .github/workflows/board-sync.yml
grep -n "blocked" docs/project-management.md
grep -n "topup" docs/project-management.md
```

#### Milestone gate

1. All acceptance criteria above are met.
2. Phase 1 tests still pass: `node --test scripts/gh-project.test.mjs`.
3. Commit:

```bash
git add scripts/gh-project.mjs .github/workflows/board-sync.yml
git commit -m "feat(gh-project): topup command + board-sync integration (#386)"
```

---

## Final Acceptance

- [ ] `node --test scripts/gh-project.test.mjs` passes with no failures.
- [ ] `node scripts/gh-project.mjs topup --dry-run` exits 0.
- [ ] `node scripts/gh-project.mjs topup --target 0` exits 0 with `deficit: 0`.
- [ ] `node scripts/gh-project.mjs help` lists `topup`.
- [ ] `board-sync.yml` has `topup --fix` before `promote --fix`.
- [ ] `blocked` label is in `MIRROR_LABELS`.
- [ ] `planTopup` and `TOPUP_TARGET_DEFAULT` are exported from the module.
- [ ] `docs/project-management.md` has a `blocked` row in the mirror-set table, an auto-topup subsection in the Promotion pipeline section, and `topup` in the Common commands block.
- [ ] `cargo test`, `cargo clippy` N/A — no Rust changes.

## Coordination Rules

- Lead Integrator is this session (single actor). No multi-agent fan-out.
- The export line must be kept in a single statement — do not split.
- The test file imports from `./gh-project.mjs` — never from a built/copied version.

## Assumptions

- The `board()` function already returns per-row `labels` as string arrays, `status`, `state`, and
  `repo`/`num` — no new board fields required.
- `pRank(null)` returns 99 (current implementation; test confirms this is `P_ORDER.indexOf(null)`
  which is -1 mapped to 99 in `pRank`).
- `isWorkItem(row)` already filters out epic/plan/story labels — no duplication needed in candidates filter.
- The `GHP_TOPUP_TARGET` env var is only used as a coerced Number; if unset/NaN, the default
  applies (`Number(undefined) || TOPUP_TARGET_DEFAULT` → `0 || 3` → `3`).
- `pRank` is a module-internal function (defined at line ~39 of `gh-project.mjs`). `cmdTopup`
  uses it directly since both live in the same file — no export needed. Only `planTopup` and
  `TOPUP_TARGET_DEFAULT` need to be added to the `export { ... }` statement.
