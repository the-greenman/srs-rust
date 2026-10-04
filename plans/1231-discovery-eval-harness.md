# Plan: Discovery eval harness (recall@10 / MRR on muSrs)

## Summary

Discovery ranking and semantic-search decisions (srs#726 open question 6) are currently guesswork. This plan adds a corpus-gated test, `crates/srs-repository/tests/discovery_eval.rs`, that runs ~25 realistic muSrs questions against a pinned muSrs archive and prints recall@10, recall@all and MRR per method, plus a classification of every miss. Landing scope: the methods that exist now (Layer-1 substring; all-words once srs-rust#1218 merges). BM25 (#1228) and `similar` (#1230) add one table entry each in their own issues. The srs#726 comment waits for the full table; baseline numbers go on #1231.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | main agent |
| Repository Worker | main agent (test + fixture only) |
| Verification | Verification Agent |

## Architecture Decisions

Spec gate: no spec change (test + fixture data only; questions are fixture data, not spec content).

| ADR | Decision | Status |
|---|---|---|
| [ADR-048](../docs/adr/048-implementation-decision-rules.md) | Implementation charter: one way per goal, so methods are a single list of `fn(store, query) -> ranked ids`; the harness calls the real `discovery_service::find`, never a re-implementation | accepted |
| [ADR-033/039](../docs/adr/039-srs-archive-pure-tree-zip.md) | Pin the corpus as a `.srs` archive packed by `srs archive pack`, unpacked into a tempdir by `archive_unpack` | accepted |
| Decision (no new ADR) | The pinned archive (2.9 MB, deterministic zip) is committed under `tests/fixtures/discovery-eval/`. Alternative rejected: fetching live muSrs (not pinned, network, drifts). Alternative rejected: the srs-web 92-instance slice (too small to show noise). | decided |
| Decision (no new ADR) | The test always runs (fixture is in-repo); it guards against a vacuous pass (all expected ids must exist, corpus size asserted) and asserts a recall@all floor for the substring baseline so the baseline cannot silently regress. It never fails on low recall of a method with no floor. | decided |
| Decision (no new ADR) | A question carries an agent-style keyword `query` plus the natural `question`; Layer-1 is unranked (id order), so recall@10 is on the first 10 hits as returned, and recall@all separates "found but buried" (ranking) from "absent". | decided |
| Decision (no new ADR) | Miss classes (automated, via `find` only): `ranking` (found, rank > 10), `label/tag` (a query word absent from the text but present in the record's label or tags), `vocabulary-mismatch` (a query word occurs nowhere in the record), `phrase` (every word is present, only the contiguous phrase fails; the all-words column removes these). | decided |
| Interop register | Not touched (no export/import/agent surface change). | n/a |

## Scope

- `tests/fixtures/discovery-eval/musrs-pinned.srs` (886 instances, from muDemocracy.org commit 6e6cd5b) and `questions.json` (27 questions, expected ids resolved with `srs find`).
- `tests/discovery_eval.rs`: loads, evaluates, prints the table and miss breakdown; one command: `cargo test -p srs-repository --test discovery_eval -- --nocapture`.
- `docs/dogfooding.md` row only if relevant (no CLI surface change: skipped).

**Out of scope:** BM25 column (#1228), `similar` column (#1230), the srs#726 comment (waits for the full table), any embedding layer.

## Phases

### Phase 1: Fixture + harness

**Goal:** the one-command table prints for the Layer-1 baseline.

**Agent:** Repository Worker

#### Tasks

- [x] Pack the pinned archive and write `questions.json`
- [x] Write `discovery_eval.rs` (methods list, metrics, miss classification)
- [x] Run it; record baseline

#### Acceptance Criteria

- [x] `cargo test -p srs-repository --test discovery_eval -- --nocapture` prints the table and breakdown
- [x] Every expected id resolves in the corpus

#### Milestone gate

`cargo test -p srs-repository --test discovery_eval`, `cargo clippy --workspace --all-targets -- -D warnings`, commit.

---

## Final Acceptance

- [x] `cargo test --workspace` passes
- [x] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] No payload structs or schemas changed

## Coordination Rules

- Test and fixture files only; no `src/` change.

## Assumptions

- #1218 has not merged at plan time; if it merges before the PR, add the all-words method entry.
