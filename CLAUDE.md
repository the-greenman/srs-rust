# CLAUDE.md — srs-rust

Rust implementation of the SRS system: `srs-core`, `srs-repository`, `srs-cli`, `srs-bindings`, `srs-projection`.

This file covers everything needed to work in this repo. It is part of a monorepo (`srs`, `srs-rust`, `srs-vscode`, `srs-web`) — when using Claude Code on the web, each repo is accessed independently. The authoritative agent usage rules for SRS repositories live in `srs-usage.md` in the `srs` repo (accessible locally at `../srs/srs-usage.md`).

## SRS data model (reference)

**Field** — atomic semantic unit. UUID `id`, `namespace`, `name` (snake_case), `version` (integer), `valueType` (string|text|number|boolean|date|url|select|multiselect), optional `aiGuidance`. Field semantics are immutable.

**Type** — named, versioned composition of Fields. Contains `fields[]` as FieldAssignments: `{ fieldId, order, required, displayLabel? }`. `displayLabel` is rendering-only and never affects semantics.

**Record tiers:**
- **Tier 0 (Note)**: free text, no type binding
- **Tier 1 (TypedRecord)**: named fields with values, no Type binding
- **Tier 2 (Record)**: instantiated Type via `typeId` + `typeVersion`; `fieldValues[]` maps `fieldId → value`

**Relation** — typed edge between two instance UUIDs. Canonical types: `contains`, `depends-on`, `supersedes`, `refines`, `derived-from`, `evidences`, `precedes`.

**Container** — lightweight grouping boundary. `containerId` is distinct from instance IDs.

**Repository** — directory with `.srs/` marker + `manifest.json`. `instanceIndex` in the manifest is the authoritative member list.

## CLI JSON envelope

All `srs` commands return:
```json
{ "ok": true, "command": "...", "version": "0.1.0", "payload": { ... } }
```
On error: `"ok": false` with `"diagnostics": [...]`. Exit code 0 means the command ran — validate errors are in `payload.diagnostics`, not the exit code.

## Git commit signing (local CLI use)

All commits use an SSH signing key. Before committing, verify the key is loaded:

```bash
ssh-add -l | grep -q "SHA256:vHuO6si5w3RLL4IJZofWbyvEi42WA2fYX7bM" || echo "SIGNING KEY NOT LOADED"
```

If missing, stop — do not bypass signing.

## Commands

Run from `srs-rust/`:

```bash
cargo build
cargo test
cargo test -p srs-core
cargo test test_name
cargo clippy -- -D warnings
cargo run --bin srs -- <args>
cargo run --bin generate-schemas          # regenerate payload JSON Schema golden files after changing payload.rs
```

## Crate Authority — what lives where

| Crate | Owns | Hard constraints |
|---|---|---|
| `srs-core` | Canonical Rust types, serde shapes, in-memory validation | No file I/O. No async. No `schemars`. |
| `srs-repository` | Repository loading, writing, package resolution, service functions | Depends on `srs-core`. All business logic lives here, not in the CLI. |
| `srs-cli` | Arg parsing, stdin handling, JSON envelope output | One service call per handler. No business logic. No direct filesystem access in handlers. |
| `srs-bindings` | JSON-first binding surface over repository services | Calls the same services as the CLI. No duplicated logic. |
| `srs-projection` | Rendering and export projections | Placeholder — no work until consumers exist. |

When in doubt about where logic belongs: if it would also be needed by an HTTP API or Python binding, it belongs in `srs-repository`, not `srs-cli`.

## CLI Handler Pattern (ADR-010, ADR-011)

A CLI handler must contain exactly: arg parsing, one `serde_json::from_reader` or flag-to-struct mapping, one service call, `output::ok/err`. Nothing else.

```rust
fn cmd_note_create(ctx: CliContext) -> Result<OutputDTO> {
    let input: CreateNoteInput = serde_json::from_reader(io::stdin())?;
    let result = with_store(&ctx, |store| Ok(note_service::create(store, input)?))?;
    Ok(output::ok("note create", result))
}
```

If a handler exceeds ~15 lines, the excess is almost certainly business logic that belongs in `srs-repository`.

## Payload Contract (ADR-011)

Every CLI command output is a named struct in `crates/srs-cli/src/payload.rs`. No `json!({...})` literals in handlers.

After changing any struct in `payload.rs`:

```bash
cargo run --bin generate-schemas
# commit the updated files in crates/srs-cli/schemas/payload/
```

The pre-commit hook and `cargo test --test payload_contracts` enforce this. A failing golden-file test means the schema files are out of sync with the structs.

## Service Function Contract (ADR-010)

Service functions in `srs-repository` must use:
- **Input:** typed struct (e.g. `CreateNoteInput`) — no `serde_json::Value` parameters
- **Validation:** all validation in the service, not in the CLI handler
- **Output:** typed result struct — no `json!()` construction in the service
- **Filtering:** list functions take a filter struct, not multiple overloaded functions

## Storage Boundary Rules

- `FileStore` owns all file paths. Path strings (`records/`, `.srs/`, `manifest.json`) must not appear in service logic.
- `MemoryStore` is the canonical test double — tests that only work against `FileStore` are testing the adapter, not the service.
- New service features need at least one cross-store roundtrip test (e.g. memory → json → file).
- Do not introduce `async` traits until there is a concrete async consumer.

## Tags in This Codebase

Tags are weak discovery labels. They are not semantic claims, not formal ontology, not hidden command policy. If a command needs a tag set, it belongs in a named profile or explicit input, not hardcoded in command code.

## Working with the Spec Repo

`srs/` is an external SRS repository consumed by the Rust workspace as test data — it is not internal source. Do not embed spec content directly in Rust source or tests. Use fixture copies or vendor the spec repo.

```bash
srs repo validate --repo ../srs/srs        # should be 0 errors
cargo test --test payload_contracts        # golden schema tests
```

## Pre-commit Hook

The hook runs `cargo test --test payload_contracts`. If it fails, regenerate schemas with `cargo run --bin generate-schemas` and stage the updated files before committing.
