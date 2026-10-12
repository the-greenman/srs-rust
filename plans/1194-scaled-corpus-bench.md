# Plan: Scaled-corpus benchmark (srs-rust#1194)

## Summary
Add a repeatable ×N corpus generator and in-process benchmark so perf work under #1193 has a yardstick. Tooling only; no spec change, no payload change.

## Agent Assignments
| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Worker | — |
| Verification | — |

## Architecture Decisions
No new architectural decisions — a dev-only `srs-repository` example using existing store APIs (ADR-042 typed methods); no new dependencies, no CLI/payload surface.

## Scope
- `crates/srs-repository/examples/scaled_bench.rs` (generator + bench), `docs/scaled-corpus-benchmark.md`.
- Out of scope: CI gating, muSrs corpus vendoring, `srs` CLI process-level timings.

## Acceptance
- [x] One command reproduces the ×1/×10 table.
- [x] Generated corpus validates with 0 errors (asserted per scale; checked with the CLI at ×10).
- [x] Docs note on running/reading.
- [x] `cargo clippy -p srs-repository --all-targets -- -D warnings` clean.
