# Capability Layering — the default path for building SRS features

**Status:** active guidance · **Read before implementing any new capability.**

This is the front door for *where functionality belongs* in the SRS ecosystem. The
crate boundaries and contracts it depends on are decided in
[ADR-001 (library-first)](../adr/001-library-first-architecture.md),
[ADR-010 (service boundary)](../adr/010-service-boundary-contract.md),
[ADR-011 (CLI output contract)](../adr/011-cli-output-contract.md), and
[ADR-013 (WASM binding strategy)](../adr/013-wasm-binding-strategy.md). This guide
ties them into a single rule and shows how to apply it. It does not restate them —
follow the links for the decisions and trade-offs.

## Why this document exists

A search feature was once built the wrong way: a bespoke case-insensitive substring
filter over two hardcoded fields, written in TypeScript inside srs-web
(`DecisionLogView.svelte`). Nothing was reusable. The CLI couldn't search. A future
graph or vector engine would have searched *different* content and *disagreed* with
the web app about what matched. The semantics lived in a leaf client where no other
consumer could reach them.

The fix — [EPIC #212](https://github.com/the-greenman/srs-rust/issues/212) — re-routes
search onto the path every capability is supposed to take. This document makes that
path explicit so the next feature doesn't drift the same way.

## The load-bearing rule

> **A capability is implemented once, in the core, and consumed identically by every
> client. Clients add presentation, never semantics.**

Concretely, the flow for any new capability is:

```
srs-core          canonical types + in-memory validation (no I/O)
   │
srs-repository    ONE service function: typed input struct → typed output struct.
   │              All business logic — filtering, traversal, projection, validation.
   ├──────────────┬───────────────────────────────────────────────
   ▼              ▼
srs-cli         srs-bindings        ← two thin adapters over the SAME service
(payload         (WASM, reuses
 struct,          the service,
 ADR-011)         no logic)
   │              │
   ▼              ▼
 humans/agents   srs-web, srs-vscode  ← thin clients: call the binding/CLI,
                                          render the result, add ZERO semantics
```

If two clients could ever disagree about the answer, the logic is in the wrong place:
move it down into the `srs-repository` service so there is one answer for everyone.

## The three-layer model (generalised from #212)

The same capability can exist at three layers. Higher layers may accelerate lower
ones but must never contradict them.

- **Layer 0 — Persisted ground truth.** Records, relations, containers, the manifest
  `instanceIndex`. The bytes on disk. Unchanged by any feature.
- **Layer 1 — Portable contract.** Deterministic, pure functions over Layer 0,
  computable with **zero auxiliary index** and identical across implementations. This
  is what lives in `srs-repository` services. Always correct, always available. For
  discovery/search this is spec-defined — see RFC for Discovery & Text Projection
  ([srs-rust #213](https://github.com/the-greenman/srs-rust/issues/213)).
- **Layer 2 — Optional acceleration.** In-memory index, SQLite FTS, graph DB,
  embeddings/vector. Adds speed, ranking, and fuzzy/semantic recall. Pluggable, and
  always optional.

**Consistency rule (the constraint that makes Layer 2 safe):** structured results
(type/container/tag/lifecycle filters) from an accelerator MUST equal the Layer-1
contract results **exactly**. For content matching, the deterministic Layer-1 match is
a **guaranteed-recall floor**: an accelerator MAY add semantically-related results and
MAY reorder by score, but MUST NOT drop anything the contract matched.

Most features only ever need Layer 1. Reach for Layer 2 only when there is a measured
need, and only behind the consistency rule.

## "Where does this logic go?" decision table

| Logic | Owner | Why |
|---|---|---|
| Canonical types, serde shapes, field/value validation rules | `srs-core` | Pure, no I/O; shared by everything including WASM/FFI. |
| Filtering, sorting, graph traversal, container membership, lifecycle transitions | `srs-repository` service | Business logic — every consumer must get the same answer. |
| **Typed/structured projection** (shaping core types into a result struct) | `srs-repository` service | It's a shared data contract, not a format. See #188. |
| **Format-specific rendering** (markdown / html / ASCII / text for humans) | client (`srs-cli`, future `srs-projection`, srs-web) | An output-format opinion; no other consumer wants it. See #131. |
| Arg parsing, stdin, JSON envelope, exit codes | `srs-cli` | Process-interface concern only. |
| WASM (de)serialisation glue | `srs-bindings` | Calls the same service the CLI calls; no logic. |
| File paths (`records/`, `.srs/`, `manifest.json`) | `FileStore` | Storage-adapter detail; must not leak into services or bindings. See #208. |
| UI state, layout, presentation | srs-web / srs-vscode | The only thing clients are allowed to own. |

### The render-vs-project distinction (resolves an apparent contradiction)

Two open issues look like they pull in opposite directions:

- **#188** moves a `ProtocolStage` → summary **projection** *into* the service.
- **#131** moves `render_brief_markdown` *out* of the service into the CLI.

They obey one rule. **Typed data shaping is a contract → it lives in the service so all
clients share it. Format-specific rendering is an opinion → it lives in the client.**
If the output is a struct other consumers would want, push it down. If the output is a
string formatted for one audience, keep it up.

## Anti-pattern case study: the srs-web search filter

What went wrong, concretely:

1. **Semantics in a leaf client.** The match logic lived in `DecisionLogView.svelte`,
   reachable only from the web UI. The CLI and any future engine had no way to call it.
2. **Hardcoded, partial scope.** It searched exactly `title` and `decision_statement`
   — not driven by the type's searchable fields, so it silently missed content.
3. **Guaranteed divergence.** A later FTS or vector engine would index different text
   and return different matches. There was no contract for them to agree on.

The corrected shape: a deterministic text-projection primitive and `find` service in
`srs-repository` (Layer 1), exposed via a CLI payload **and** a WASM binding, with
srs-web calling the binding and rendering the hits. One definition of "what matches,"
shared by all.

## Checklist — adding a new capability

Before opening a PR for a feature that does anything semantic (query, filter, traverse,
validate, project):

- [ ] Logic lives in a `srs-repository` service function: **typed input struct → typed
      output struct**, all validation inside (ADR-010). No `serde_json::Value`
      parameters; no `json!()`-built results in the service.
- [ ] Any new type lives in `srs-core` if a non-Rust consumer could need it; no file I/O
      in `srs-core`.
- [ ] Exposed through **both** adapters where applicable: a named payload struct in
      `srs-cli/src/payload.rs` (ADR-011) **and** a `srs-bindings` method that calls the
      same service (ADR-013). The binding returns a typed struct, not an ad-hoc
      `json!({})` (see #205).
- [ ] CLI handler is ≤ ~15 lines: parse → one service call → `output::ok/err`.
- [ ] No file-path strings outside `FileStore`. No hardcoded vocabularies (lifecycle
      states, tag sets) in client or binding code.
- [ ] Clients (srs-web, srs-vscode) call the binding/CLI and render — they implement no
      matching, sorting, traversal, or validation in TypeScript.
- [ ] Format-specific rendering (markdown/html) lives in the client, not the service.

If you can't satisfy these, the capability is being built in the wrong layer — stop and
move it down.

## Active-issue alignment (audit appendix)

A 2026-06 audit found **no active issue is misimplemented against this architecture**;
the architecture-relevant issues are all corrective refactors converging on the rule
above. Tracked in the architecture-alignment audit issue.

| Issue | What it corrects | Rule it enforces |
|---|---|---|
| #205 | bindings returning ad-hoc `json!({})` | binding returns a typed struct (ADR-011 shape) |
| #204 | `ProtocolStage.outputType` typed as `serde_json::Value` | typed contract in `srs-core` (ADR-010) |
| #208 | `"records/tier-2"` hardcoded in bindings | paths belong in `FileStore` |
| #188 | typed stage projection done in the CLI handler | typed projection → service |
| #131 | `render_brief_markdown` in the service | format rendering → client |
| #183, #174, #175, #176, #130, #206 | protocol/blueprint definition modelling | definitions as package data, typed validation |

These are not re-routings — they are the architecture finishing the job it already
started.
