# Plan: Scaled-corpus benchmark (#1194)

## Summary
No repeatable way exists to measure engine scaling with corpus size. Add a corpus generator and benchmark, runnable with one command, with a short doc.

## Agent Assignments
| Role | Agent |
|---|---|
| Lead Integrator | srs-repository worker |
| Verification | Verification Agent |

## Architecture Decisions
No new architectural decisions. Dev tooling only: an `examples/` binary in `srs-repository` (like `migrate_fixture`), calling existing services; no CLI payload, no spec or schema change (Stage 1.5: no spec change required). Uses the vendored fixture (CLAUDE.md "Working with the Spec Repo"), not a live sibling checkout.

## Scope
- `crates/srs-repository/examples/scale_bench.rs`: `gen` (xN copy, deterministic id remap, canonical filenames) and `bench` (catalog build, find/get, write+read, list, validate).
- `scripts/bench-scale.sh`: unpack the pinned archive, generate x10, print the x1/x10 table.
- `docs/benchmarking.md`, one line in CLAUDE.md Commands.

**Out of scope:** CI gating; criterion; perf fixes (#1195 and siblings).

## Phases
### Phase 1: generator + bench + docs
- [x] generator; output loads with zero catalog/validation errors
- [x] bench table; script; docs

## Final Acceptance
- [x] `scripts/bench-scale.sh` prints the x1 / x10 table
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean
