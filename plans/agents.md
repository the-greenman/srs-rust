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
