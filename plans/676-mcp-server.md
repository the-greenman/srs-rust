# Plan: MCP server binding — expose SRS repositories to MCP clients (#676)

> Issue: [srs-rust#676](https://github.com/the-greenman/srs-rust/issues/676) · Alignment register item 1 (weight 100, NOW) — `srs/docs/research/alignment-opportunities.md` (srs PR #217)

## Summary

Expose SRS repositories to any MCP client (Claude Code, Cursor, Copilot, Goose, …) through a stdio MCP server embedded in the existing `srs` binary as `srs mcp serve`. The server is a thin adapter over existing `srs-repository` services — the same capability-layering shape as the CLI (ADR-011) and WASM bindings (ADR-013): resources for read access (repo map, navigation, records, containers, rendered document views) and tools for the validated write workflows (`repo_validate`, `find`, `record_create`, `relation_create`, `note_create`). No new semantics anywhere. The validating write contract is the differentiator the MCP knowledge-tool space lacks (register item 1). Owner decisions (2026-07-22): single binary (measured cost +~2.9 MB, 13.6 → ~16.5 MB); official `rmcp` SDK (v2.2, speaks all protocol revisions incl. the 2026-07-28 RC); first cut includes the write tools.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | session lead |
| MCP Adapter Worker | agents.md#mcp-adapter-worker (new role, added with this plan) |
| CLI Worker | agents.md#cli-worker |
| Verification | agents.md#verification-agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-037](../docs/adr/037-mcp-adapter-surface.md) | **New.** MCP adapter surface: `srs-mcp` is the sole crate depending on `rmcp`/`tokio`; thin one-service-call handlers; per-request `FileStore`; `srs://` URI scheme is tooling-only; `srs mcp serve` envelope carve-out; single binary | proposed |
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Governs: `srs-mcp` is another process-interface consumer of `srs-repository`; zero business logic. ADR-037 extends the crate roster | accepted |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Governs: every tool/resource handler = typed input → one service call → typed output; no validation in the adapter | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | Governs: no schemars in library crates — `srs-mcp` defines its own schemars-1-derived tool-input structs rather than deriving on service types. (The schemars-major-coexistence decision itself is ADR-037 point 2) | accepted |
| [ADR-013](../docs/adr/013-wasm-binding-strategy.md) | Template: sole-SDK-dependency crate, thin wrappers — `srs-mcp` mirrors this for MCP | accepted |
| [ADR-017](../docs/adr/017-deterministic-srsj-serialization.md) | Constraint: no dep may enable serde_json `preserve_order`. Audited: rmcp 2.2.0 tree is clean (serde_json deps = itoa/memchr/ryu/serde only) | accepted |
| [ADR-004](../docs/adr/004-schemas-embedded-at-compile-time.md) | Governs: anything schema-shaped served by MCP comes from the embedded registry, never network/sibling paths | accepted |
| ADR-008 / ADR-019 (async rule) | Constraint: services stay sync. tokio (current_thread) is confined to `srs-mcp::serve_stdio`; handlers call sync services directly (single-client stdio server — acceptable blocking, recorded in ADR-037) | accepted |
| [ADR-019](../docs/adr/019-discovery-service.md) / [ADR-020](../docs/adr/020-resolve-view-authored-list-defaults.md) | Governs: the `find` tool calls `discovery_service::find`; container resources go through `resolve_container_view` — never re-derived | accepted |

**Interop register consult (required):** this plan implements register item 1 (MCP server, weight 100, NOW) directly; the URI-scheme spec question was resolved at the Stage 1.5 spec gate — no spec change; deferred follow-up will be filed (see Out of scope).

---

## Contracts

### CLI output contract (ADR-011)

**One new command, no envelope:** `srs mcp serve` is a long-running server that speaks MCP JSON-RPC on stdout — it deliberately does **not** emit the `{ok, command, payload}` envelope and has **no payload struct / golden schema**. This carve-out is recorded in ADR-037 (a serving loop is not a "one service call + output envelope" handler). No existing command payload changes. `cargo test --test payload_contracts` must still pass untouched; `generate-schemas` is not run.

### Entity schema sync (check-schema-sync.sh)

**No.** No files under `srs/docs/schema/2.0/` are added or modified. No action required.

---

## Scope

- New workspace crate `crates/srs-mcp` (lib only): rmcp 2.x stdio MCP server over `srs-repository` services.
- MCP **resources** (read): repo map, navigation, container resolve-view, rendered document views; record read via resource template.
- MCP **tools**: `repo_validate`, `find`, `record_create`, `relation_create`, `note_create` — each exactly one existing service call, diagnostics returned to the client.
- CLI wiring: `srs mcp serve` subcommand (respects global `--repo`), delegating to `srs_mcp::serve_stdio`.
- ADR-037 (proposed → accepted when shipped).
- Integration tests: in-process rmcp client ↔ server over a duplex transport (handshake, list/read, tool calls incl. negative cases).

**Out of scope** (each filed as a linked follow-up issue at Stage 3):

- HTTP / streamable-http transport (`srs mcp serve --http`).
- Additional write tools: `record_update`, `record_successor`, lifecycle `record_transition`, `note_graduate`, container membership writes.
- MCP resource subscriptions / list-changed notifications (would build on `ext:changelog`).
- MCP prompts surface (`blueprint brief` is the natural first prompt).
- Multi-repo serving from one server process (registry of roots).
- Spec-side: whether a canonical `srs://` URI syntax deserves a tooling-only spec note under `ext:addressability` (srs repo; `requires-spec-rfc`).

---

## Phases

### Phase 1: Crate scaffold + ADR

**Goal:** `crates/srs-mcp` exists in the workspace with a constructible server handler, serving stdio behind `serve_stdio()`; ADR-037 committed.

**Agent:** MCP Adapter Worker

#### Tasks

- [x] Workspace `Cargo.toml`: add members entry `crates/srs-mcp`; add `[workspace.dependencies]` entries `rmcp = { version = "2", features = ["server", "transport-io"] }` and `tokio = { version = "1", features = ["rt", "macros", "io-std"] }` (referenced only by `srs-mcp`).
- [x] `crates/srs-mcp/Cargo.toml`: deps `srs-core`, `srs-repository`, `serde`, `serde_json`, `anyhow`, `uuid`, `rmcp` (workspace), `tokio` (workspace), `schemars = "1"` (tool input schemas only). Dev-deps: `tempfile`, `tokio` with `io-util`/duplex support for tests, `rmcp` `client` feature.
- [x] `crates/srs-mcp/src/lib.rs`: `pub fn serve_stdio(repo_path: std::path::PathBuf) -> anyhow::Result<()>` — builds a `tokio` current_thread runtime, constructs `SrsMcpServer { repo_path, repository_id }` (repository id read once from the manifest via a `FileStore`), serves with `rmcp::ServiceExt::serve(stdio())`, blocks on `service.waiting()`.
- [x] `crates/srs-mcp/src/server.rs`: `SrsMcpServer` implementing `rmcp::ServerHandler` — `get_info()` returns serverInfo `{ name: "srs-mcp", version: env!("CARGO_PKG_VERSION") }`, capabilities `{ resources, tools }`, and an `instructions` string summarising the surface and pointing at the discovery ladder (map → find → record read → validated writes).
- [x] Store access helper: `fn open_store(&self) -> Result<FileStore, ...>` — a fresh `FileStore` per request, mirroring the CLI's per-invocation `with_store` semantics (no shared mutable state, picks up on-disk changes between calls).
- [x] `docs/adr/037-mcp-adapter-surface.md` (status: proposed) per the Architecture Decisions table.
- [x] Verify `cargo tree -p srs-mcp -i serde_json -e features` shows no `preserve_order` (ADR-017) and record the check in the ADR.

#### Acceptance Criteria

- [x] `cargo build -p srs-mcp` succeeds; `cargo build --bin srs` unchanged and green.
- [x] `serve_stdio` compiles as a blocking fn; no `async` appears in any `srs-repository`/`srs-core` signature.
- [x] No crate other than `srs-mcp` gained a dependency on `rmcp`/`tokio` (Phase 4 adds `srs-mcp` itself, not its deps, to `srs-cli`).
- [x] serde_json `preserve_order` absent from the full workspace feature graph.

#### Testing

```bash
cargo test -p srs-mcp
cargo clippy -p srs-mcp -- -D warnings
```

Specific tests to write or verify:

- `server_info_reports_name_and_capabilities` — `get_info()` carries name `srs-mcp`, resources + tools capabilities.
- `open_store_on_missing_repo_errors` — constructing against a path with no `.srs/` marker yields a clean error, not a panic.

#### Milestone gate

1. Verify all acceptance criteria above are met — check each checkbox.
2. Confirm every test listed exists and passes.
3. `cargo test -p srs-mcp && cargo clippy -p srs-mcp -- -D warnings`
4. Update plan checkboxes.
5. Commit (`feat(srs-mcp): crate scaffold + ADR-037 (#676)`).

---

### Phase 2: Read surface — resources

**Goal:** An MCP client can list and read repo map, navigation, containers, rendered document views, and individual records.

**Agent:** MCP Adapter Worker

#### Tasks

- [x] `crates/srs-mcp/src/uri.rs`: parse/format the tooling-only scheme (documented in ADR-037):
  - `srs://<repositoryId>/map`
  - `srs://<repositoryId>/navigation`
  - `srs://<repositoryId>/record/<instanceId>`
  - `srs://<repositoryId>/container/<containerId>`
  - `srs://<repositoryId>/view/<documentViewId>`
  Unit-tested round-trip; unknown shapes → typed parse error.
- [x] `crates/srs-mcp/src/resources.rs` — `list_resources` returns concrete resources: map (`application/json`), navigation (`application/json`), one per container from `container_service::list_containers` (name = container title), one per document view from the loaded package (name = `DocumentView.name` qualified with `namespace` — the struct has no `title` field; `text/markdown`). Records are **not** enumerated concretely; `list_resource_templates` exposes `srs://<repositoryId>/record/{instanceId}`.
- [x] `read_resource` dispatch — each arm exactly one service call, serialized with `serde_json::to_string` (values produced by service structs; no `json!()` literals):
  - map → `analysis::build_repo_map`
  - navigation → `repository_navigation_service::repository_navigation`
  - record → `record_store::get_record_by_id`
  - container → `container_view_service::resolve_container_view` (ADR-020: authored columns + ordered members + `isVisibleByDefault`)
  - view → `render_service::render_document_view` with markdown format → `text/markdown` contents
- [x] Error mapping: a genuine `Err(RepositoryError)` from a service → MCP resource error carrying the service's error message verbatim (no invented text). The `Ok(None)` branch of `record_store::get_record_by_id` (missing record is not a service error) → a fixed adapter-authored `resource not found: <uri>` MCP error. Malformed URIs → the `uri.rs` parse error.

#### Acceptance Criteria

- [x] Against a fixture repo, `resources/list` returns map + navigation + every container + every document view, and `resources/templates/list` returns the record template.
- [x] `resources/read` on each URI kind returns contents matching the corresponding service output byte-for-byte (JSON) / the rendered markdown (view).
- [x] Unknown record id and malformed URI produce MCP errors, not panics; messages surface the service error.

#### Testing

```bash
cargo test -p srs-mcp
cargo clippy -p srs-mcp -- -D warnings
```

Specific tests (in-process client over `tokio::io::duplex`). Fixture repos are created inline in each test's setup using `srs-repository` writer services in a `tempfile` dir — one fixture per test for isolation; no shared fixture helper is required, but extracting one inside `srs-mcp/tests/` is fine if it stays test-local:

- `uri_roundtrip_all_kinds` — format→parse identity for all five URI kinds.
- `list_resources_enumerates_containers_and_views`
- `read_record_matches_service_output`
- `read_view_renders_markdown`
- `read_unknown_record_is_mcp_error`

#### Milestone gate

Steps 1–5 as Phase 1. Commit (`feat(srs-mcp): read resources — map, navigation, records, containers, views (#676)`).

---

### Phase 3: Tools — validate, find, and validated writes

**Goal:** MCP clients can run discovery and the validated write workflows and receive the same diagnostics the CLI envelope carries.

**Agent:** MCP Adapter Worker

#### Tasks

- [x] `crates/srs-mcp/src/tools.rs` — schemars-1-derived input structs local to `srs-mcp` (serde names matching the CLI stdin shapes in `srs-usage.md`): `FindToolInput`, `RecordCreateToolInput { type_filter (serialized as "type" via #[serde(rename = "type")] — reserved keyword): "namespace/name", typeVersion?, fieldValues, groupValues?, tags?, containerId? }`, `RelationCreateToolInput`, `NoteCreateToolInput`; `repo_validate` takes no input.
- [x] Drift guard for the shadow input shapes (they must field-for-field track the canonical service inputs): implement `From<RecordCreateToolInput> for record_store::CreateRecordInput`, `From<RelationCreateToolInput> for srs_core Relation`, `From<NoteCreateToolInput> for services::CreateNoteInput`, `From<FindToolInput> for DiscoveryQuery` — handlers may only reach services through these conversions — plus unit test `tool_input_conversion_exercises_every_field` populating every field of each tool input and asserting the converted service input carries all of them.
- [x] Tool description strings defined once as `pub const` items in `tools.rs` (single source; no inline literals at call sites). The Stage-7.5 docs pass writes the `srs-usage.md` MCP section from these constants. (A CI drift check against `srs-usage.md` is deliberately not added: it lives in the sibling `srs` repo, which srs-rust CI does not check out — spec independence.)
- [x] Tool handlers — each exactly one service call, mirroring the CLI handler for the same operation:
  - `repo_validate` → `validation::validate_repository`
  - `find` → `discovery_service::find` (map input onto `DiscoveryQuery` — all axes)
  - `record_create` → `record_store::create_record_in_context(store, type_filter, type_version, input, container_id, None)`
  - `relation_create` → `relation_service::create_relation_auto`
  - `note_create` → `srs_repository::services::create_note_in_context` (verified: the exact import `commands/note.rs` uses — `services` is the real module name, not a placeholder)
- [x] Result mapping: success → `CallToolResult` with the service result serialized as JSON text content (and `structured_content` where rmcp supports it); service validation failure → `CallToolResult { is_error: true }` whose text carries the service error/diagnostics verbatim — a rejected write is a *tool-level* result the model can read, not a protocol error.
- [x] `tools/list` advertises all five tools using the single-source description constants above (agents get the same guidance in both surfaces; `srs-usage.md`'s MCP section is written from these constants at the docs pass).

#### Acceptance Criteria

- [x] Each tool's happy path writes/reads exactly what the equivalent CLI command produces against the same fixture repo (asserted via a follow-up service read, not string comparison with the CLI).
- [x] `record_create` with a missing required field returns `is_error: true` with the service's diagnostic listing the field — nothing is written (verified by `repo_validate` + record list).
- [x] `relation_create` with an uninstalled `relationType` is rejected with the RFC-005 resolution error; nothing is written.
- [x] `find` results equal `discovery_service::find` called directly with the same query.

#### Testing

```bash
cargo test -p srs-mcp
cargo clippy -p srs-mcp -- -D warnings
```

Specific tests:

- `tool_repo_validate_clean_fixture_zero_diagnostics`
- `tool_record_create_happy_then_validate`
- `tool_record_create_missing_required_is_error_no_write`
- `tool_relation_create_unknown_type_rejected`
- `tool_note_create_and_find_roundtrip` — create a note, `find` by content match returns it.

#### Milestone gate

Steps 1–5 as Phase 1. Commit (`feat(srs-mcp): tools — repo_validate, find, record/relation/note create (#676)`).

---

### Phase 4: CLI wiring — `srs mcp serve`

**Goal:** The single `srs` binary serves MCP: `srs mcp serve --repo <path>` runs the stdio server until the client disconnects.

**Agent:** CLI Worker

#### Tasks

- [x] `crates/srs-cli/Cargo.toml`: add `srs-mcp = { path = "../srs-mcp" }` (and workspace dep entry).
- [x] `crates/srs-cli/src/commands/mcp.rs`: `McpCommands::Serve` — handler resolves the repo root exactly as other commands (global `--repo`, cwd auto-detect via existing context), then makes **one** call: `srs_mcp::serve_stdio(repo_path)`. No envelope output (ADR-037 carve-out). ≤ 15 lines.
- [x] Register `Mcp` subcommand in `commands/mod.rs` clap tree with help text: "Serve this repository over the Model Context Protocol (stdio)".
- [x] Global flags: `--pretty` and `--container` (clap `global = true`) parse on `mcp serve` but are **accepted-and-ignored** in this cut (no envelope to prettify; container scoping of the served surface is a possible follow-up, not silently half-implemented). Recorded in ADR-037.
- [x] `crates/srs-mcp/README.md`: what the crate is, the URI scheme, tool list, one client-config example (Claude Code `.mcp.json` snippet using `srs mcp serve --repo <path>`).

#### Acceptance Criteria

- [x] `srs mcp serve --repo <fixture>` starts, answers an `initialize` handshake on stdio, and exits cleanly on stdin close (verified by integration test driving the real binary).
- [x] `srs mcp serve` with no repo in cwd and no `--repo` exits non-zero with a clear error on stderr (stdout stays protocol-clean).
- [x] `cargo test --test payload_contracts` passes with zero schema changes (no envelope, no payload struct).
- [x] Handler is a single delegation call — no logic.

#### Testing

```bash
cargo test -p srs
cargo test --test payload_contracts
cargo clippy -- -D warnings
```

Specific tests:

- `mcp_serve_binary_initialize_handshake` (integration test in `crates/srs-cli/tests/`: spawn the built binary, send `initialize` + `notifications/initialized` + `tools/list` over stdio, assert well-formed responses, close stdin, assert clean exit).
- `mcp_serve_without_repo_errors_on_stderr`

#### Milestone gate

Steps 1–5 as Phase 1. Commit (`feat(cli): srs mcp serve subcommand (#676)`).

---

### Phase 5: End-to-end integration + hardening

**Goal:** A full client session against a real repo works end-to-end; the whole workspace is green.

**Agent:** MCP Adapter Worker + Verification Agent

#### Tasks

- [ ] `crates/srs-mcp/tests/e2e.rs`: one scripted session over an in-process duplex transport — initialize (assert negotiated protocol version + both capabilities) → `resources/list` → `resources/read` (map, one record) → `tools/call record_create` → `tools/call repo_validate` (zero diagnostics) → `tools/call find` returns the created record.
- [ ] Concurrency/robustness pass: malformed tool arguments (schema violation) are answered as rmcp invalid-params, not a crash; oversized/unknown URIs handled.
- [ ] Verification Agent run: crate-boundary audit (no business logic in `srs-mcp`, no rmcp/tokio outside it), duplication report (tool handlers vs CLI handlers call identical service functions), full test transcript.

#### Acceptance Criteria

- [ ] e2e session test green.
- [ ] `cargo test` (workspace) and `cargo clippy -- -D warnings` green.
- [ ] Verification report: zero boundary violations, zero duplicated business logic.

#### Testing

```bash
cargo test
cargo clippy -- -D warnings
```

Specific tests:

- `e2e_full_session_read_write_validate` — the scripted session above.
- `tool_call_malformed_args_invalid_params`

#### Milestone gate

Steps 1–5 as Phase 1. Commit (`test(srs-mcp): e2e session + hardening (#676)`).

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass; `payload_contracts` untouched and green)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] No crate outside `srs-mcp` depends on `rmcp`/`tokio`; no async in library-crate signatures; serde_json `preserve_order` absent from the workspace graph
- [ ] Release binary size delta measured and recorded on the issue (expected ≈ +3 MB; flag if > +5 MB — the owner's single-binary decision was conditioned on "not significantly larger")
- [ ] Dogfood scenario run pre-PR on the feature branch: build `srs` from the branch, configure a real MCP client against a scratch repo (`srs repo create`), run a full session — initialize → resources list/read → `record_create` → `repo_validate` (zero diagnostics) → `find` — plus one rejected write (missing required field → `is_error` with diagnostics); add the scenario to `docs/dogfooding.md` with a meaningful intention and "Done when" signals
- [ ] ADR-037 status flipped to `accepted`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit. Do not proceed to the next phase without completing the milestone gate.
- Verification Agent runs after Phase 5 and before final sign-off.

## Assumptions

- rmcp 2.2.x API is stable for the duration of this plan; version pinned as `"2"` in the workspace.
- Single-client stdio serving; repositories at current scale (≤ a few thousand instances) need no pagination beyond rmcp defaults — multi-repo and HTTP serving are deferred follow-ups.
- Blocking file I/O inside async handlers is acceptable for a single-client stdio server on a current_thread runtime (recorded in ADR-037).
- The `srs://` URI scheme is implementation tooling (Stage 1.5 spec-gate ruling); a spec-side note is a deferred follow-up, and nothing in this plan constrains that future design beyond using existing ID components.
