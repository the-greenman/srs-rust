# Agent roles

Role definitions referenced by plan files (`plans/<slug>.md`) and the `/ship` pipeline.
Reviewer roles are **read-only**; worker roles carry an explicit write scope that the plan
may narrow but not widen without the Lead Integrator's sign-off.

## Lead Integrator

Owns the plan end-to-end: sequencing, final API naming, dependency boundaries, milestone
gates, commits. Resolves conflicts between workers, expands or narrows write scopes, and is
the only role that edits the plan file's checkboxes. Runs the milestone gate at the end of
every phase.

## Repository Worker

- **Write scope:** `crates/srs-repository/src/**`, `crates/srs-repository/tests/**`,
  fixture directories named by the plan.
- Implements storage/service changes in `srs-repository`. Must not change public service
  signatures beyond what the plan names, and must not touch `srs-cli` payloads or
  `srs-bindings`.

## Bindings Worker

- **Write scope:** `crates/srs-bindings/src/**`, `crates/srs-bindings/tests/**`.
- Implements WASM surface changes. Bindings stay thin per ADR-013: deserialize input → one
  service call → serialize output; no business logic.

## CLI Worker

- **Write scope:** `crates/srs-cli/src/**`, `schemas/payload/**` (via
  `cargo run --bin generate-schemas` only).
- Implements command handlers and payload structs per ADR-011.

## MCP Adapter Worker

- **Owns:** The MCP server adapter in `srs-mcp` — protocol wiring, resource/tool handlers, URI scheme.
- **Write scope:** `crates/srs-mcp/**`
- **Constraints:**
  - Every resource/tool handler is a thin wrapper: parse typed input → exactly one `srs-repository` service call → serialize the service's typed result. No business logic, no validation beyond input deserialization (ADR-010, ADR-037).
  - `srs-mcp` is the sole crate depending on `rmcp`/`tokio`; the async runtime never leaks into library-crate signatures.
  - No `json!({...})` result construction — serialize service structs directly.
  - Tool handlers call the exact same service functions as the equivalent CLI handlers — divergence is a bug.

## Architecture Reviewer

- **Read-only.** Reviews a plan (Stage 3) or a diff (Stage 7) against **every** ADR in
  `docs/adr/`, the crate-boundary rules in `CLAUDE.md`/`ARCHITECTURE.md`, DRYness, and
  coding-style consistency. Returns numbered findings with severity
  (`blocking` / `should-fix` / `nit`), each citing the ADR or rule it enforces.

## Plan Reviewer

- **Read-only.** Reviews a plan for completeness against `plans/TEMPLATE.md`: unambiguous
  tasks, explicit file paths, named functions, checkable acceptance criteria, contract
  sections answered, scope discipline, testability. Returns numbered findings with severity.

## Verification Agent

- **Owns:** Test runs, architecture audits, and duplication checks.
- **Write scope:** None (read-only unless explicitly asked to patch tests).
- **Deliverables:**
  - Command/test transcript summary
  - Crate-boundary audit
  - Duplicated-logic report
