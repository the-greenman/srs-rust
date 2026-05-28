# ADR-001: Library-First Architecture

- **Status:** accepted
- **Date:** 2026-05-28
- **Supersedes:** —
- **Superseded by:** —

## Context

The SRS Rust workspace was initially scaffolded with a CLI binary (`srs-cli`) as the primary artefact. All note operations — list, get, create, tag, slug generation, tag auditing, foundation note selection — were implemented directly in CLI command handlers. This made the logic unreachable from any other consumer (Python bindings, WASM, future applications) without going through the process boundary.

The SRS spec itself is language-agnostic. Other planned consumers include TypeScript/WASM bindings and Python bindings for AI tooling. These consumers need the same SRS operations without duplicating logic or shelling out to the CLI.

## Decision

The library crates (`srs-core`, `srs-repository`) are the primary deliverable. The CLI (`srs-cli`) is one consumer of them.

Crate responsibilities:

- **`srs-core`** — canonical Rust types for all SRS entities; validation logic. No file I/O. Suitable for WASM and FFI.
- **`srs-repository`** — repository detection, file loading, manifest management, service functions (note CRUD, analysis, record operations). Depends on `srs-core`. No CLI-specific code.
- **`srs-cli`** — argument parsing, stdin handling, JSON envelope output, exit code handling. Delegates all logic to `srs-repository`. No business logic.

All business logic that any consumer might need lives in `srs-repository` or `srs-core`. The CLI adds only the process interface.

## Consequences

**Positive:**
- Python bindings, WASM modules, and future applications can call library functions directly without shelling out.
- Business logic can be unit-tested without spawning a subprocess.
- The CLI remains thin and easy to audit — if a command handler contains more than argument parsing + a service call + output formatting, it is a smell.

**Negative / trade-offs:**
- Service function signatures must be designed with multiple callers in mind, not just the CLI's convenience.
- Result types returned from services need to be serialisation-friendly for callers that aren't Rust (motivates the JSON-first binding design).

**Neutral:**
- The CLI remains the stable machine-facing contract. Library APIs may evolve independently.
- `anyhow` is acceptable in `srs-cli` for ergonomic error handling. `thiserror` with explicit error types is required in `srs-core` and `srs-repository`.
