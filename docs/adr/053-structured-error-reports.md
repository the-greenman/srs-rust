# ADR-053: Structured error reports — one `ErrorReport`, carried by every adapter

- **Status:** proposed
- **Date:** 2026-10-08
- **Supersedes:** —
- **Superseded by:** —

## Context

`RepositoryError` reached clients as Display text only: `format!("{e:#}")` into the CLI
envelope's `diagnostics: Vec<String>`, a bare string thrown from WASM `js_err`, and text-only MCP
`tool_err`. Clients that must act on an error kind matched prose: srs-web#512 never detects
`LifecycleNotDefined`, and srs-vscode#131 matches text the engine never emits. Some variants put a
code into the prose instead (`SCREAMING_SNAKE:` prefixes, `"{code}: {message}"`). ADR-048 rule 6
("identifier over label in outputs") forbids both. srs-rust#1338 is the remediation.

## Decision

1. **One core shape.** `srs-repository` owns `ErrorReport { code, message, details? }`.
   `RepositoryError::report()` builds it; adapters carry it unchanged and never compute a code.
   srs-cli keeps a schemars mirror (`ErrorReportPayload`) only to emit the payload golden
   (schemars stays out of the library crates, ADR-011); a round-trip test pins the two together.
2. **Codes are an explicit contract.** `RepositoryError::code()` is an exhaustive match with one
   kebab-case code per variant. It is not derived from the Rust variant name, so renaming a variant
   never changes the wire. RFC/ADR-named codes pass through verbatim: RFC-046 `actor-*`, ADR-050
   `bundle-*`, ADR-051 `slice-*` (those variants keep their inner `code`). The former
   SCREAMING_SNAKE forms are renamed once, with no aliases (owner ruling on #1264).
3. **Display is presentation.** No `#[error]` string carries a code prefix.
4. **Details are derived, open.** `RepositoryError` derives `Serialize` (untagged, camelCase
   fields, non-serializable sources skipped). `details` is that object, so a new variant cannot
   forget its fields. The schema types `details` as `object`. A per-code typed union is a
   deferred follow-up.
5. **Adapters:**
   - **CLI:** `ok:false` envelopes gain `errors: [ErrorReport]` aligned 1:1 with `diagnostics`,
     which stays unchanged for string consumers. `main.rs` finds the `RepositoryError` in the
     anyhow chain.
   - **WASM:** throws a JS `Error` whose `.code` and `.details` are set. No blanket `From<String>`:
     each site passes the `RepositoryError` or names `invalid-input` / `unclassified` explicitly, so a
     stringified error cannot silently lose its code.
   - **MCP:** `tool_err` returns `structuredContent: ErrorReport`. Write-guard rejections are
     `write-guard-rejected`.
     Resource reads and prompts, and the `read` tool, which forwards `resources/read` errors
     verbatim (#1220), fail as JSON-RPC `-32603` with the `ErrorReport` in `error.data`. MCP's
     own `-32002` resource-not-found is a protocol identifier and stays as it is.
6. **Non-repository failures** route through an existing variant where one fits (`invalid-input`,
   `*-not-found`). Otherwise they carry `unclassified`, an honest marker to drive to zero rather
   than an invented label.

## Consequences

**Positive:**
- Clients branch on `code` (srs-web#512, srs-vscode#131, the #1247 `candidates` picker).
- Tests assert on `code` (rule 6(d)).
- Message text is free to improve.

**Negative / trade-offs:**
- The CLI envelope's top-level shape grows (`errors`), which is an owner-merge contract change.
- WASM now throws an `Error` object rather than a string, so `String(e)` gains an `Error: `
  prefix for clients that stringified it.
- `details` is untyped in the schema until the follow-up.

**Neutral:**
- `ok:true` envelope diagnostics are out of scope (#1339).
- Validation diagnostics are #1264.
- `details` carries the variant's fields verbatim, including unbounded lists (`CatalogLoad.diagnostics`, `RecordHasInboundRelations.relations`) and file paths already present in the message; this is intentional, with no capping.

## Implementation charter (ADR-048)

- [x] **Spec-first:** no spec change. The envelope is an srs-rust contract (ADR-011). RFC-046
  codes are used verbatim. Governed by srs charter clarification `rfc-decision-b2ff7c91`.
- [x] **Layer test:** core service (`srs-repository` owns codes and the report); CLI, WASM and MCP
  are adapters only.
- [x] **One way per goal:** one `ErrorReport`. `diagnostics` strings stay as the presentation
  twin during transition, aligned 1:1 and not independently computed.
- [x] **Parity and mirror obligations:** new payload golden `error-report.json`. No entity schema
  mirrors are involved. The srs-web and srs-vscode consumers are tracked in their own issues.
- [x] **Decision mode:** complicated. The owner chose among the analysed options at the Stage 2
  pause (2026-10-08).
