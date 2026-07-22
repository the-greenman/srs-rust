# Plan: MCP type & field discovery — type resources + `type_schema` tool (#692)

> Issue: [srs-rust#692](https://github.com/the-greenman/srs-rust/issues/692) · Closes the biggest teaching gap in the #676 MCP surface · Parent story muDemocracy.org#128

## Summary

An MCP-only agent authoring against a **fresh type with no existing records** cannot discover its fieldAssignments/fieldIds, and field/type `aiGuidance` — the spec's LLM guidance channel — is not surfaced over MCP. This plan closes the gap with the smallest ADR-037-shaped extension: enumerate every package type as a resource, add a `srs://<repositoryId>/type/{typeId}` resource template whose read returns the existing `type_schema` projection (JSON Schema whose properties carry `x-srs-field-id`, `x-srs-ai-guidance`, `x-srs-widget` — from `plans/type-schema-command.md` / `type-schema-field-help.md` — plus ADR-026's `x-srs-description`/`x-srs-instructions`), and add a model-invocable `type_schema` tool over the same service. The server's teaching text (`instructions`, `record_create` description) is updated to point at the new discovery path. No new semantics, no new dependencies, no new crate.

**Design-pause note (Stage 2.4):** no pause taken — every choice here is additive within accepted ADR-037, uses only existing services, and implements the scope the owner approved in the issue body ("as resources … and/or a `type_schema` tool"). We ship **both** resource and tool forms: resources for enumeration/browse, the tool because MCP clients surface tools to the model more reliably than resources (the same reason the teaching gap exists). Recorded as an ADR-037 amendment (ADR-031 sets the in-repo precedent for dated amendment sections), not a new ADR.

**Repo/PR structure (explicit, resolving plan-review blocking 1–2):** everything in Phases 1–2 lives in **srs-rust** and lands in **this single PR** — including `docs/dogfooding.md` (which is an srs-rust file) and `docs/adr/037-*.md`. The **only** cross-repo artifact is `srs-usage.md` §5i, which lives in the `srs` repo and ships as a **companion docs PR on an `srs` branch**, exactly the #676 pattern (`docs/676-mcp-usage` → here `docs/692-mcp-type-usage`). Cross-repo and pipeline-stage docs tasks (Stages 7.5/7.6) are executed by the **Lead Integrator**, not the MCP Adapter Worker — the worker's write scope stays `crates/srs-mcp/**`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | session lead — owns Phase 2 docs tasks, the Stage 7.5/7.6 doc/dogfood updates (srs-rust `docs/**` + the `srs` companion branch), and cross-repo coordination |
| MCP Adapter Worker | agents.md#mcp-adapter-worker — Phase 1 only (`crates/srs-mcp/**`) |
| Verification | agents.md#verification-agent |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-037](../docs/adr/037-mcp-adapter-surface.md) | Governs everything here. **Amendment added by this plan**: §6 URI enumeration gains `type/<typeId>`; the amendment records the resource+tool dual exposure and that `type_schema`'s result (schema + diagnostics) is served verbatim | accepted (amended) |
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | Each handler = one existing service call: `package_service::list_types`, `type_schema_service::type_schema` | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | No schemars on library crates → `TypeSchemaToolInput` is a shadow struct in `srs-mcp` with a `From<TypeSchemaToolInput> for TypeSchemaInput` conversion + every-field drift test (same guard as #676) | accepted |
| [ADR-026](../docs/adr/026-type-schema-field-help-keys.md) | Governs 2 of the 5 vendor keys served (`x-srs-description`, `x-srs-instructions`); `x-srs-field-id`/`x-srs-ai-guidance`/`x-srs-widget` predate it (established in `plans/type-schema-command.md` / `plans/type-schema-field-help.md`). The adapter serves the `field_to_property` projection verbatim — no re-projection | accepted |
| ADR-004 | Schema content comes from the in-repo package via the service — no network, no sibling paths | accepted |

**Interop register consult:** implements register item 1's composition claim ("an … agent-index projection is just another resource this server exposes") — type schemas are the authoring-contract half of that. No register entry contradicted.

*No new ADR: no new constraint, dependency direction, or reversal — ADR-037 already governs the adapter; the amendment documents the additive surface.*

---

## Contracts

### CLI output contract (ADR-011)

**No new/changed CLI commands.** The CLI surface is untouched; `payload.rs` and golden schemas unchanged; `payload_contracts` must pass untouched. (The MCP server is outside the envelope per ADR-037 §4.)

### Entity schema sync (check-schema-sync.sh)

**No.** No files under `srs/docs/schema/2.0/` touched.

---

## Scope

- `srs-mcp` only (code): new URI kind `Type(String)`; type resources in `resources/list`; `type/{typeId}` resource template; `resources/read` arm → `type_schema_service::type_schema` (latest version); new `type_schema` tool (`typeId`, optional `typeVersion`) returning `TypeSchemaResult` verbatim.
- Teaching-text updates in `srs-mcp` (exact snippets in Phase 1 task 4).
- ADR-037 amendment section; crate README rows (srs-rust, this PR).
- Companion `srs` docs PR: `srs-usage.md` §5i rows (Lead Integrator, Stage 7.5).
- Tests: URI roundtrip extension; the two existing assertions that the new surface breaks (named below); resource enumeration/read; tool happy + negative; drift-guard extension; e2e discover-then-author step.

**Out of scope** (already filed; nothing new to file unless review/dogfood surfaces more):

- Namespace/name → typeId resolution inside the tool (agents get typeIds from the enumerated type resources; a resolve-by-name convenience can ride with the #680 write-tool wave).
- Blueprint brief as a prompt (#682); standalone field enumeration beyond what type schemas carry (fold into #680 if needed).

---

## Phases

### Phase 1: Adapter surface — type resources + `type_schema` tool

**Goal:** MCP clients can enumerate types, read any type's authoring schema (with aiGuidance vendor keys), and call `type_schema` as a tool.

**Agent:** MCP Adapter Worker (write scope `crates/srs-mcp/**`)

#### Tasks

- [x] `crates/srs-mcp/src/uri.rs`: add `SrsUri::Type(String)` — parse/format `srs://<repositoryId>/type/<typeId>`; add the `type/<typeId>` line to the module-doc URI enumeration (lines 1–9); extend `uri_roundtrip_all_kinds` + reject tests; add `type_template(repository_id)` (RFC 6570 `type/{typeId}`).
- [x] `crates/srs-mcp/src/resources.rs`:
  - `list_resources`: one resource per `package_service::list_types(store)` entry — this enumerates the full loaded package set including sub-packages (`TypeSummary.source_package` distinguishes them; no extra filtering). Name = `namespace/name`, description = `TypeSummary.description` (fallback: "Type schema: fieldAssignments + aiGuidance for authoring"), mime `application/json`. If a filter-struct variant exists (`list_types_filtered`/`TypeListFilter`), use it with `::default()` to match the containers/views idiom; else call `list_types` with a one-line comment noting the service has no filter struct.
  - `list_resource_templates`: add the `type/{typeId}` template ("Authoring schema for a type: fieldIds, required flags, and aiGuidance — read before record_create").
  - `read_resource` arm: `SrsUri::Type(id)` → `type_schema(store, TypeSchemaInput { type_id: id, type_version: None }).map_err(service_err)?` — **same pattern as the Container/View arms**; `type_schema` always returns `Err(RepositoryError::TypeNotFound)` for unknown ids (verified — no `Ok(None)` branch exists), so no adapter-authored not-found string is needed. Serialize the `TypeSchemaResult` verbatim (`{schema, diagnostics}`) with `serde_json::to_string_pretty` (the established `json_text` helper).
- [x] `crates/srs-mcp/src/tools.rs`:
  - `TOOL_TYPE_SCHEMA` + `DESC_TYPE_SCHEMA` consts. Description text: "Get the authoring schema for a type by its UUID (typeVersion optional; latest when omitted). The result is a JSON Schema whose properties carry x-srs-field-id (the UUID to use in record_create fieldValues), x-srs-ai-guidance, x-srs-description, and x-srs-instructions; required fields are listed in required. Read this before creating records of an unfamiliar type — discover typeIds from the type resources in resources/list."
  - `TypeSchemaToolInput { type_id: String, type_version: Option<u32> }` (camelCase, `deny_unknown_fields`) + `From<TypeSchemaToolInput> for TypeSchemaInput`; the handler reaches the service only through the conversion.
  - `list_tools` gains the sixth tool; `call_tool` arm → one `type_schema` call; success → `tool_ok(&result)`; `Err` → `tool_err` (tool-level).
  - **Update the existing exact-vec test**: rename `list_tools_advertises_all_five_with_schemas` → `list_tools_advertises_all_six_with_schemas` and add `TOOL_TYPE_SCHEMA` to the expected vec (it asserts equality and would otherwise fail).
  - Extend `tool_input_conversion_exercises_every_field`: populate `type_id` + `type_version` with distinct values and assert both carry through — the test must fail if a field is added to either side without updating the conversion.
- [x] Teaching text (`crates/srs-mcp/src/server.rs` + `tools.rs`), concrete edits:
  - `INSTRUCTIONS`: after the record-template sentence, insert: *"Type schemas live at srs://<repositoryId>/type/{typeId} (also via the type_schema tool): read one before authoring records of an unfamiliar type — it carries each field's UUID (x-srs-field-id) and aiGuidance."*
  - `DESC_RECORD_CREATE`: replace *"resolve the type's fieldAssignments first (read the container or map resources, or find existing records of the type)"* with *"resolve the type's fieldAssignments first via the type_schema tool or the srs://<repositoryId>/type/{typeId} resource (each property's x-srs-field-id is the UUID to use here)"*.
- [x] **Test-fixture updates** (both in `crates/srs-mcp/tests/`):
  - `resources.rs::make_fixture`: author one type (`com.example.mcptest/decision`-style: one required string field carrying `aiGuidance`) so the new resource tests have a type to enumerate/read; store `type_id` + `field_id` on `Fixture`.
  - `resources.rs::list_resources_enumerates_containers_and_views`: templates assertion changes from `len() == 1` to `len() == 2`, asserting both `record/{instanceId}` and `type/{typeId}` `uri_template` values.
  - `tools.rs::Fixture`: re-add `type_id` (dropped as dead in #676) so the tool tests can call `type_schema`.

#### Acceptance Criteria

- [x] Against the fixture: `resources/list` includes the type (name `namespace/name`); `resources/templates/list` has exactly the two templates; reading the type URI returns text equal to `serde_json::to_string_pretty` of the direct service call's `TypeSchemaResult` (pretty form — not compact), including `x-srs-field-id` on every property and `x-srs-ai-guidance` on the guidance-carrying field.
- [x] `tools/list` advertises six tools; `type_schema` tool happy path equals the direct service call; unknown typeId → `is_error: true` (tool) / MCP error carrying the `TypeNotFound` message (resource); server keeps answering afterwards.
- [x] Drift-guard test exercises every `TypeSchemaToolInput` field with distinct values.

#### Testing

```bash
cargo test -p srs-mcp
cargo clippy -p srs-mcp --all-targets -- -D warnings
```

Specific tests:

- `uri_roundtrip_all_kinds` (extended) — includes `Type`.
- `list_resources_enumerates_types` — fixture type appears, namespace-qualified name.
- `read_type_schema_matches_service_output` — asserts pretty-printed byte-equality with the service result + presence of `x-srs-field-id` and `x-srs-ai-guidance`.
- `tool_type_schema_happy_and_unknown_id` — happy equals service; random-UUID id → `is_error: true` with `TypeNotFound` text.
- `list_tools_advertises_all_six_with_schemas` (renamed/extended).
- `tool_input_conversion_exercises_every_field` (extended).
- `tests/e2e.rs` extended: the session calls `type_schema` on the fixture type, extracts each property's `x-srs-field-id` from the returned schema, builds `record_create` fieldValues **from those discovered ids** (not the fixture's variables), then `repo_validate` → zero diagnostics — proving the teaching loop end-to-end.

#### Milestone gate

Standard steps 1–5 (criteria checked, tests exist and pass, `cargo test -p srs-mcp` + clippy, tick boxes, commit `feat(srs-mcp): type discovery — type resources + type_schema tool (#692)`).

---

### Phase 2: ADR amendment + in-repo docs (srs-rust, this PR)

**Goal:** ADR-037 and the crate README describe the extended surface.

**Agent:** Lead Integrator (docs files are outside the worker's crate scope)

#### Tasks

- [x] `docs/adr/037-mcp-adapter-surface.md`: add **Amendment (2026-07-22, #692)** section — URI enumeration gains `type/<typeId>`; type schemas exposed as both resources (browse/enumeration) and a tool (model-invocable; clients surface tools more reliably); result is the `type_schema` projection served verbatim.
- [x] `crates/srs-mcp/README.md`: resource-table row for `type/{typeId}` + sixth tool in the tools list.
- [x] Stale-wording sweep in srs-rust: `rg -n "five tools|all_five" --glob '!target' .` → fix every hit this change made stale.

*(Stage 7.5, Lead Integrator, separate `srs` repo branch `docs/692-mcp-type-usage` → companion PR: `srs-usage.md` §5i resource-table row + `type_schema` tool bullet + record_create bullet update. Stage 7.6, Lead Integrator, srs-rust this PR: `docs/dogfooding.md` S42 extension + matrix row — after actually running the scenario.)*

#### Acceptance Criteria

- [x] ADR-037 amendment present; README matches the shipped surface; the sweep grep returns zero stale hits in srs-rust.

#### Testing

```bash
cargo test -p srs-mcp && cargo clippy --all-targets -- -D warnings
rg -n "five tools|all_five" --glob '!target' . || echo clean
```

#### Milestone gate

Standard; commit `docs: ADR-037 amendment + README for type discovery (#692)`.

---

## Final Acceptance

- [ ] `cargo test` passes (workspace); `cargo clippy --all-targets -- -D warnings` passes
- [ ] `payload_contracts` untouched and green; no entity schemas changed
- [ ] No new dependencies; rmcp/tokio still confined to `srs-mcp`
- [ ] e2e proves the teaching loop: schema discovered over MCP → fieldIds extracted from `x-srs-field-id` → record authored → validate clean
- [ ] Dogfood S42 extension run on the branch binary against the S42 scratch repo: `resources/list` shows the decision type; `type_schema` (tool) on its UUID returns `x-srs-field-id` for both title and decision_statement; a record is created using those discovered ids; `repo_validate` → zero diagnostics; `docs/dogfooding.md` updated accordingly
- [ ] Companion `srs` PR (`docs/692-mcp-type-usage`) opened alongside this PR

## Coordination Rules

Standard (see TEMPLATE.md) plus the explicit split above: MCP Adapter Worker writes only `crates/srs-mcp/**`; Lead Integrator owns `docs/**`, README, and the `srs` companion branch; Verification Agent before sign-off.

## Assumptions

- `type_schema` with `type_version: None` resolves the latest version and returns `Err(RepositoryError::TypeNotFound)` for unknown ids (both verified in service source).
- Type counts per repo are tens, not thousands — concrete enumeration in `resources/list` needs no pagination beyond rmcp defaults.
- The S42 scratch repo (decision type with title/decision_statement) suffices for the dogfood; `aiGuidance` presence is proven by the unit fixture, not the dogfood repo.
