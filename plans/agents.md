# Agent Definitions

Standard agent roles for SRS Rust implementation work. Reference this file in plans using `agents.md#<role>`.

---

## Lead Integrator

- **Owns:** Architecture decisions, sequencing, final integration, public API consistency, and review.
- **Write scope:** Workspace manifests, cross-crate wiring, final cleanup.
- **Coordination:** Merges worker outputs, resolves API disagreements, enforces crate-boundary model.
- **Does not:** Implement features directly — delegates to workers and reviews their output.

---

## Repository Service Worker

- **Owns:** Service logic and repository operations in `srs-repository`.
- **Write scope:** `crates/srs-repository/**`
- **Constraints:**
  - No business logic in CLI — services must work without the CLI layer.
  - Avoid double-loading the manifest in multi-step operations.
  - Return structured result types that callers can serialize without reconstructing logic.

---

## CLI Worker

- **Owns:** Command handlers in `srs-cli` — argument parsing, stdin handling, JSON output.
- **Write scope:** `crates/srs-cli/**`
- **Constraints:**
  - Handlers must only parse args/stdin, call library services, and wrap output.
  - No duplicated business logic. No direct filesystem access in handlers.
  - JSON envelope format must remain compatible across changes.

---

## Core Model Worker

- **Owns:** In-memory SRS types and validation in `srs-core`.
- **Write scope:** `crates/srs-core/**`
- **Constraints:**
  - No filesystem dependencies — `srs-core` must remain I/O-free.
  - Serde names must align with existing JSON schemas.
  - Validation that depends only on in-memory data belongs here.

---

## Bindings Worker

- **Owns:** JSON-first binding surface over library services.
- **Write scope:** `crates/srs-bindings/**`
- **Constraints:**
  - Accept repo paths; return JSON strings or JSON-compatible data.
  - No duplicated CLI logic — call the same services the CLI calls.
  - Smoke tests must prove outputs are parseable JSON.

---

## Verification Agent

- **Owns:** Test runs, architecture audits, and duplication checks.
- **Write scope:** None (read-only unless explicitly asked to patch tests).
- **Deliverables:**
  - Command/test transcript summary
  - Crate-boundary audit
  - Duplicated-logic report

---

## Architecture Reviewer

- **Owns:** Reviewing a plan or a code diff against the project's architectural rules. Read-only.
- **Write scope:** None — produces findings only.
- **Reviews for, in priority order:**
  1. **System boundaries** — crate authority is respected (see `srs-rust/CLAUDE.md` "Crate Authority"): no file I/O in `srs-core`, no business logic in `srs-cli`, all services in `srs-repository`, no path strings outside `FileStore`. The handler pattern (ADR-010) and payload contract (ADR-011) hold.
  2. **DRYness** — no duplicated business logic across crates or handlers; list operations use filter structs not overloaded functions; bindings call the same services as the CLI.
  3. **Consistent coding style** — naming, error handling, serde shapes, and module organization match the surrounding code and existing ADRs.
  4. **ADR coverage** — every architectural choice is governed by an existing ADR or flags the need for a new one.
- **Constraints:** Must check the plan/diff against **every** ADR in `srs-rust/docs/adr/`, not a sampled subset. Each finding cites the specific rule, ADR, or file:line it violates, and is labelled `blocking` / `should-fix` / `nit`.
- **Deliverables:** Numbered findings list; each with severity, the rule it violates, and a concrete suggested change.

---

## Plan Reviewer

- **Owns:** Reviewing a plan file for executability before any code is written. Read-only.
- **Write scope:** None — produces findings only.
- **Reviews for:**
  - **Completeness** — every task has an explicit file path / named function and a checkable acceptance criterion; no step requires human interpretation at execution time (TEMPLATE.md usage note).
  - **Contracts** — the CLI output contract and entity schema-sync sections are answered correctly for what the plan touches.
  - **Scope discipline** — in-scope list is tight; out-of-scope is explicit; phases have milestone gates.
  - **Testability** — each phase names the specific tests that prove it and the verification commands.
- **Deliverables:** Numbered findings list with severity (`blocking` / `should-fix` / `nit`) and a concrete fix for each.
