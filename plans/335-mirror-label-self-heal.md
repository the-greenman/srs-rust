# Plan: Self-healing mirror labels in `gh-project.mjs` (#335)

> Lightweight plan. This change is to the Node project-tooling script `scripts/gh-project.mjs`, not a Rust crate — the crate/ADR/payload/schema/dogfood stages of the standard pipeline do not apply and are marked N/A below.

## Summary

The scheduled cloud routines ("Do the SRS jobs", "SRS issue progress review") read and write a **plain-label mirror set** — `ready`, `priority: P0/P1/P2`, `status: in progress` — that `gh-project` maintains. A repo audit found the set is incomplete across ecosystem repos: `srs` and `srs-web` lack `status: in progress`, and `muDemocracy.org` has **none** of the mirror labels. When a routine runs `gh issue edit --add-label "status: in progress"` in a repo that lacks the label, `gh` hard-fails and the claim step dies. Today `gh-project`'s `ensureLabels(repo)` only guarantees the three `priority: P*` labels exist, so the rest can drift. This plan makes the tool the single source of truth for the **full** mirror set and creates any missing labels on demand, so the set can't drift again. It also performs the one-off cross-repo label ops now (as authorised) and files follow-ups for the parts that live outside this repo.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | this session (direct) |
| Verification | this session + `node --test` |

Single-file script change; no fan-out needed.

## Architecture Decisions

No new ADRs. ADRs govern Rust crate boundaries and the CLI payload contract; `scripts/gh-project.mjs` is a standalone zero-dependency Node tool outside that surface. The relevant governing doc is `docs/project-management.md` (the priority model + mirror-label convention), which this plan updates rather than an ADR.

## Contracts

- **CLI output contract (ADR-011):** N/A — no Rust CLI command or `payload.rs` struct touched.
- **Entity schema sync:** N/A — no files under `srs/docs/schema/2.0/` touched.

## Scope

- Add a canonical `MIRROR_LABELS` definition (name + color + description) and a `MIRROR_REPOS` list (env-overridable) to `scripts/gh-project.mjs`.
- Extend `ensureLabels(repo)` to create the **full** mirror set (was: `priority: P*` only), idempotently via `gh label create --force`.
- Have `rollup --fix` and `reconcile --fix` ensure the mirror set across all `MIRROR_REPOS` (write-mode only; dry-run stays side-effect-free) — the root-cause "create missing mirror labels on demand" fix.
- Add an explicit `ensure-labels [--repo R] [--dry-run]` command as a direct ops entry point.
- Extract a pure `labelCreateArgs(repo, spec)` helper and add a `node:test` unit test (`scripts/gh-project.test.mjs`); guard the top-level dispatch behind a main-module check so the module is importable by the test.
- Update `docs/project-management.md` to document the mirror-label set and the self-heal behaviour.
- **Ops (outside the PR, run now against live repos):** create the missing mirror labels in `srs`, `srs-web`, `muDemocracy.org`; retire the legacy `priority: medium/low/high` labels in `srs-rust`/`srs-web`.

**Out of scope (filed as follow-up issues):**
- Adding `board-sync.yml` to `srs-web` (a file in the `srs-web` repo — separate PR there).
- Verifying/setting the `BOARD_TOKEN` secret in `muDemocracy.org` (a GitHub secret; requires the owner).

## Phases

### Phase 1: Self-healing mirror labels + test

**Goal:** `gh-project` guarantees the full mirror label set exists in every ecosystem repo it writes to, and the behaviour is covered by a hermetic unit test.

#### Tasks

- [x] Add `MIRROR_LABELS` (ready, priority: P0/P1/P2, status: in progress — each with color + description) and env-overridable `MIRROR_REPOS` near the config block.
- [x] Add pure `labelCreateArgs(repo, spec)` returning the `gh label create ... --force` arg array.
- [x] Rewrite `ensureLabels(repo)` to loop `MIRROR_LABELS` via `labelCreateArgs` (keep per-repo memoisation, keep non-fatal try/catch).
- [x] Add `cmdEnsureLabels(argv)` + `ensure-labels` dispatch case + `help` line.
- [x] In `cmdRollup`/`cmdReconcile`, when `!dryRun`, call `ensureLabels` for each `MIRROR_REPOS` entry before applying changes.
- [x] Guard the top-level dispatch behind `import.meta.url === pathToFileURL(process.argv[1]).href`; `export { MIRROR_LABELS, MIRROR_REPOS, labelCreateArgs }`.
- [x] Add `scripts/gh-project.test.mjs` with `node:test` cases (set completeness, color/description shape, `labelCreateArgs` idempotency + repo scoping, `MIRROR_REPOS` coverage).

#### Acceptance Criteria

- [x] `node --test scripts/` passes.
- [x] `node scripts/gh-project.mjs ensure-labels --repo <x> --dry-run` lists all five mirror labels and makes no changes.
- [x] `node scripts/gh-project.mjs help` lists `ensure-labels`.
- [x] Importing the module (as the test does) does not execute the CLI dispatch or shell out to `gh`.

#### Testing

```bash
node --test scripts/gh-project.test.mjs
node scripts/gh-project.mjs ensure-labels --repo srs-web --dry-run
node scripts/gh-project.mjs help
```

#### Milestone gate

1. Acceptance criteria met.
2. `node --test scripts/` green.
3. Commit referencing #335.

## Final Acceptance

- [x] `node --test scripts/` passes.
- [x] Dry-run + help behave as specified.
- [x] `docs/project-management.md` documents the mirror set + self-heal.
- [x] Live ops applied (labels created/retired) and verified with the issue's reproduction loop.
- [x] Follow-up issues filed for `srs-web` board-sync.yml and `muDemocracy.org` BOARD_TOKEN.
