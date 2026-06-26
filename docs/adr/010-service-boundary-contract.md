# ADR-010: Service Boundary Contract

- **Status:** accepted
- **Date:** 2026-05-30 (proposed), 2026-06-26 (accepted)
- **Supersedes:** —
- **Superseded by:** —

## Context

ADR-001 established that `srs-cli` is one consumer of `srs-repository`, and that all business logic belongs in the library crates. In practice, the CLI has accumulated 26 identified instances of business logic:

- Container membership filtering duplicated identically across 4+ list handlers
- Multi-step create/delete orchestration wired in CLI handlers (validate container → create → add member; check membership → remove member → delete)
- Input parsing and normalization (`normalize_field_input`, `parse_field_values_payload`, the 153-line `protocol import` field-mapping block)
- Validation rules (ID mismatch checks, container existence checks) evaluated in handlers
- Service selection logic in the CLI (branching across 3 service functions based on filter combinations)

This makes it impossible to build an HTTP API, Python bindings, or any other consumer that shares the same semantics without duplicating or rewriting this logic.

A second structural problem: there is no enforced pattern for service function signatures. Services currently mix `serde_json::Value` inputs, typed struct inputs, and unparsed strings with no consistent contract. The CLI handler must know the internal shape of each service to call it correctly, making service boundaries implicit.

## Decision

Every `srs-repository` service function must conform to the following contract:

**Input:** A typed input struct (e.g., `CreateNoteInput`, `RecordListFilter`) defined alongside the service function in the service module. The struct is the public contract. The CLI deserializes stdin into the struct; the service receives the struct. No `serde_json::Value` parameters on public service functions.

**Validation:** The service is responsible for all validation: JSON schema validation, semantic validation, and cross-entity validation (e.g., container existence, membership checks). The CLI must not perform any validation that would also be required by an API consumer.

**Orchestration:** Multi-step operations (create + add to container; check membership + remove + delete) are atomic service operations. The CLI calls one function; it does not coordinate multiple service calls to complete one logical operation.

**Output:** A typed result struct (e.g., `NoteResult`, `NoteSummary`) defined alongside the service function. The CLI serializes this to the JSON envelope. The struct is the contract for all consumers.

**Filtering:** List functions accept a filter struct rather than exposing multiple service functions for different filter combinations. The CLI maps CLI flags to the filter struct; it does not select which service function to call.

The CLI handler pattern is therefore:

```rust
fn cmd_note_create(ctx: CliContext) -> Result<OutputDTO> {
    let input: CreateNoteInput = serde_json::from_reader(io::stdin())?;
    let result = with_store(&ctx, |store| Ok(note_service::create(store, input)?))?;
    Ok(output::ok("note create", result))
}
```

If a CLI handler contains anything beyond: arg parsing, one `serde_json::from_reader` or flag-to-struct mapping, one service call, and `output::ok/err` — it is a violation of ADR-001 and this ADR.

## Consequences

**Positive:**
- Any future consumer (HTTP handler, Python binding, WASM export) calls the same service functions with the same typed inputs and receives the same typed outputs.
- Service functions are fully testable in isolation without a CLI subprocess or flag parsing.
- The pattern is mechanically checkable: a handler with more than ~15 lines is a candidate for review.
- Schema validation at the service boundary means all consumers get consistent error handling, not just the CLI.

**Negative / trade-offs:**
- Requires defining explicit input and output structs for every service operation — more upfront type definitions.
- The `container_id` scoping (currently a global CLI flag) must be passed explicitly through input structs, making the optional nature of container scoping explicit in service signatures.
- Some existing service functions accept `serde_json::Value` directly; these must be refactored, which is a breaking change to their internal signatures.

**Neutral:**
- The CLI continues to own the `--repo` path resolution, store construction, and JSON envelope formatting — these are correctly CLI concerns.
- `anyhow` remains acceptable in `srs-cli`. `thiserror` with explicit error types required in service functions.
- The JSON CLI output contract does not change. The typed result structs serialize to the same JSON shapes as the current `json!({...})` literals.

## Enforcement and the render-vs-project boundary

This ADR is the load-bearing rule for the default capability path described in
[`docs/architecture/capability-layering.md`](../architecture/capability-layering.md).
Apply it together with one clarification that resolves a recurring ambiguity:

- **Typed/structured projection** (shaping core types into a result struct that other
  consumers would also want) is a shared data contract — it belongs **in the service**
  (e.g. #188).
- **Format-specific rendering** (markdown/html/text produced for one audience) is an
  output opinion — it belongs **in the client** (`srs-cli` / future `srs-projection`),
  not in the service (e.g. #131).

A service returns data; a client decides how to render it.
