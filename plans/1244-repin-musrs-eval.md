# Plan: Re-pin the discovery eval's muSrs archive after titles (#263)

## Summary

The eval (#1231) pins muSrs at muDemocracy.org@6e6cd5b, before #263 gave all 177 claims and 138 problems real titles and made `title` their identity field (merged as 4c45f35f). Re-pack the archive from main at 4c45f35f, keep the 27 questions and expected ids unchanged, record the before/after table, and raise a floor only where a floored metric improved (never lower one).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | main agent |
| Repository Worker | main agent (fixture + one doc-comment line) |
| Verification | Verification Agent |

## Architecture Decisions

Spec gate: no spec change (fixture data only).

| ADR | Decision | Status |
|---|---|---|
| [ADR-048](../docs/adr/048-implementation-decision-rules.md) | One way per goal: same harness, same method list; only the pinned data changes | accepted |
| [ADR-039](../docs/adr/039-srs-archive-pure-tree-zip.md) | Archive packed with `srs archive pack` (deterministic; packed twice, byte-identical) from a `git archive 4c45f35 muSrs` export | accepted |

No new ADRs: same pinning decision as #1231.

## Contracts

No CLI output, payload struct or entity schema changes (fixture data only), so no golden schemas or schema sync apply.

## Scope

- Replace `crates/srs-repository/tests/fixtures/discovery-eval/musrs-pinned.srs`; update `corpus.sourceCommit` and `corpus.archiveBytes` (2921288 -> 2936741) in `questions.json`. Questions and expected ids untouched.
- One doc-comment line in `tests/discovery_eval.rs` (commit pin).
- Stop-and-report condition: any expected id no longer resolving (it all resolve; instances still 886).
- Floors: `SUBSTRING_RECALL_ALL_FLOOR` (0.34) and similar-recovery (>= 3) are the only asserted floors; the substring recall@all is 0.346 and similar recovery is 3, both unchanged, so no floor is raised.

**Out of scope:** new questions, harness logic, any ranking change (BM25 recall@10 moved down; follow-up noted on the issue, not fixed here).

## Phases

### Phase 1: Re-pin and measure

#### Tasks

- [x] Record the before table (below) (master harness, old archive)
- [x] Pack the archive from 4c45f35, update archiveBytes/sourceCommit
- [x] Run the eval, record the after table and re-classified misses
- [x] Post the table on #1231, srs#726, muDemocracy.org#261

#### Testing

`cargo test -p srs-repository --test discovery_eval -- --nocapture`: asserts archive size, corpus size (886), every expected id resolves, BM25 hit set equals all-words, the two floors; prints all four method rows.

#### Acceptance Criteria

- [x] 27 questions and expected ids unchanged (diff of questions.json is only sourceCommit and archiveBytes); floors unchanged because the floored metrics did not move
- [x] `cargo test -p srs-repository --test discovery_eval -- --nocapture` passes; every expected id resolves

## Final Acceptance

- [x] `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` pass
- [x] No payload structs or schemas changed (no docs surface change)

## Coordination Rules

Fixture, `questions.json` corpus fields, one comment line and this plan only; no `src/` change.

## Results (recall@10 / recall@all / MRR)

| method | before | after |
|---|---|---|
| substring | 0.247 / 0.346 / 0.209 | unchanged |
| all-words | 0.525 / 0.698 / 0.391 | unchanged |
| BM25 | 0.676 / 0.698 / 0.532 | 0.639 / 0.698 / 0.616 |
| BM25 top-5 + similar | 0.688 / 0.735 / 0.532 | 0.688 / 0.796 / 0.620 |

## Assumptions

- muSrs@4c45f35f is whole (886 instances, all expected ids resolve); the archive packs byte-identically twice.
- The table is deterministic on the pinned corpus; docs need no update (test fixture only).
