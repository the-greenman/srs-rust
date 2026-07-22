# ADR-037: MCP Adapter Surface

- **Status:** proposed
- **Date:** 2026-07-22
- **Supersedes:** —
- **Superseded by:** —

## Context

[srs-rust#676](https://github.com/the-greenman/srs-rust/issues/676) (alignment register item 1, weight 100) adds an MCP (Model Context Protocol) server so any MCP client — Claude Code, Cursor, Copilot, Goose — can read SRS repositories and use the validated write workflows. MCP is the agent ecosystem's settled access layer (revisions stable through 2025-11-25; a 2026-07-28 RC is in flight), and the validating write contract is exactly what existing MCP knowledge tools lack.

The architecture already has two adapter surfaces over `srs-repository` services: the CLI (ADR-001/010/011) and the WASM bindings (ADR-013). No ADR covers additional binding surfaces generically; ADR-013 is WASM-specific. Constraints in play: services are sync-only (ADR-008/019 + CLAUDE.md storage rules); `schemars` must stay out of library crates (ADR-011); nothing in the dependency graph may enable serde_json's `preserve_order` feature (ADR-017); schemas are embedded, never fetched (ADR-004).

Owner decisions (2026-07-22, issue #676): prefer a single binary if the size cost is modest; use the official SDK; include the validated write tools in the first cut.

## Decision

1. **New adapter crate `srs-mcp`, mirroring the ADR-013 pattern.** `srs-mcp` is the sole crate depending on the MCP SDK (`rmcp` v2, features `server` + `transport-io`) and on `tokio`. Every resource/tool handler is a thin wrapper: typed input → exactly one `srs-repository` service call → the service's typed result serialized. No business logic, no `json!()` construction, no validation beyond input deserialization. Tool handlers call the exact same service functions as the equivalent CLI handlers. This extends the ADR-001 crate roster; nothing is superseded.

2. **Official SDK over hand-rolled JSON-RPC.** `rmcp` 2.2 implements every protocol revision (2024-11-05 … 2026-07-28 RC) with version negotiation; conformance with an actively evolving protocol is maintained upstream, and an HTTP/streamable transport later is a feature flag rather than a rewrite. Measured cost: +~2.9 MB release binary, ~75 locked packages. The dependency tree was audited for the ADR-017 landmine: rmcp 2.2.0 does **not** enable serde_json `preserve_order` (serde_json's resolved deps are itoa/memchr/ryu/serde only). rmcp brings `schemars` 1.x, which coexists with `srs-cli`'s 0.8 (different majors do not unify features); library crates gain no schemars dependency.

3. **Single binary: `srs mcp serve`.** The server ships inside the existing `srs` binary (13.6 → ~16.5 MB), not as a separate executable — one artifact to install, one `--repo` convention, one release pipeline. The CLI handler is a single delegation into `srs_mcp::serve_stdio(repo_path)`.

4. **Envelope carve-out.** `srs mcp serve` is the one CLI command that does not emit the ADR-011 `{ok, command, payload}` envelope: it is a long-running server speaking MCP JSON-RPC on stdout, not a "one service call + output envelope" handler. It has no payload struct and no golden schema. Pre-serve failures (no repository found) go to stderr with a non-zero exit; stdout stays protocol-clean. The global CLI flags `--pretty` and `--container` (clap `global = true`) parse on this command but are accepted-and-ignored: there is no envelope to prettify, and container-scoping the served surface is a deliberate non-feature of this cut (a follow-up may add it as explicit behaviour, never as a silent half-implementation).

5. **Async stays contained.** `serve_stdio` builds a tokio current_thread runtime internally and blocks. No `srs-repository`/`srs-core` signature becomes async. Handlers call sync services directly and construct a fresh `FileStore` per request (mirroring the CLI's per-invocation semantics: no shared mutable state, on-disk changes visible between calls). Blocking file I/O inside async handlers is accepted for a single-client stdio server; revisit if a multi-client transport lands.

6. **The `srs://` URI scheme is implementation tooling, not spec.** Resources use `srs://<repositoryId>/{map|navigation|record/<instanceId>|container/<containerId>|view/<documentViewId>}`. This maps onto existing spec ID components (ext:addressability defines component-based Addresses, Invariant 34 — not a URI syntax), so no spec change is required (spec-gate ruling on #676). If a canonical URI syntax is later wanted, that is a tooling-only spec note under ext:addressability — a deferred follow-up, not constrained by this ADR beyond "addresses are built from existing ID components".

## Consequences

**Positive:** Every MCP client becomes an SRS client with the validating write contract intact; one more consumer proves the ADR-001/010 service boundary; protocol conformance is delegated upstream; future transports are incremental.

**Negative / trade-offs:** ~75 new locked packages and an async runtime enter the workspace (confined to one adapter crate); binary grows ~21%; a second schemars major (1.x) rides along; the envelope carve-out means one CLI command is intentionally outside the payload-contract test net — its behaviour is covered by MCP integration tests instead. The ADR-011 "no schemars in library crates" rule forces `srs-mcp`'s tool-input structs to be *shadow copies* of the canonical service inputs — a drift risk of the same class ADR-011 closed for outputs; mitigated by mandatory `From<ToolInput>` conversions (handlers may only reach services through them) plus a unit test exercising every field. Tool description text is likewise single-sourced as constants in `srs-mcp`, with the `srs-usage.md` MCP section written from them; cross-repo CI drift enforcement is deliberately not added (srs-rust CI does not check out the sibling `srs` repo — spec independence).

**Neutral:** The server is single-repo per process (`--repo` at startup), matching the CLI's invocation model. Multi-repo serving, subscriptions/notifications, prompts, and HTTP transport are follow-up issues, not part of this decision.
