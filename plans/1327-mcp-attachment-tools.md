# Plan: MCP `attachment_add` and `attachment_link` tools (#1327)

## Summary

Agents cannot add or link a source-document attachment: neither `srs mcp serve` nor the browser `McpSession` has an attachment tool, although the core services (`attachment_service::add_attachment` / `link_attachment`) and the CLI (`srs attachment add|link`) exist. This plan adds two tools to `srs-mcp-core` (the shared dispatcher both transports use, so both gain them): `attachment_add` (file name, UTF-8 text *or* base64 content, optional title/subdir/contentType) and `attachment_link` (instanceId, documentId). `attachment_add` rejects content that violates the RFC-017 `attachment_policy`; `attachment_link` is subject to the session write guard (#1165). No spec change: I-107 says an implementation "MAY enforce `attachment_policy` limits as hard rejections".

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | main session |
| Repository Worker | main session |
| MCP Adapter Worker | main session |
| Verification | Verification Agent (haiku) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

Autonomous run (as #1229): each decision is resolved from the charter, ADR-037/048 and existing patterns, and flagged in the final report.

| # | Decision | Rationale / governing rule |
|---|---|---|
| D1 | Two new tools `attachment_add`, `attachment_link` in `srs-mcp-core/src/tools.rs` (constants, `DESC_*`, shadow input structs with mandatory `From`/`TryFrom` + a field-exercising unit test, `all_tools`, `call_tool` arms). Handlers call exactly `attachment_service::add_attachment` / `link_attachment` — the same functions as the CLI. | ADR-037 thin adapter; capability-layering.md. |
| D2 | Policy enforcement lives in the **core service**, not the adapter: `AddAttachmentInput` gains `enforce_policy: bool`; when true, `add_attachment` calls a new `attachment_policy_service::check_attachment(store, content_type, size)` before any write and returns `RepositoryError::InvalidInput` on violation. The CLI and WASM paths set `false` (unchanged behaviour: I-107 warnings at validate time); the MCP tool sets `true`. One mechanism, one function; no parallel MCP-only check. CLI/web enforcement is a follow-up (a behaviour change for human flows, owner's call). | ADR-037 (no logic in adapters), one-way-per-goal, I-107 (MAY reject). |
| D3 | Checks mirror validation.rs's I-107 semantics exactly: `size > max_per_file_bytes`, `size > max_doc_bytes`, exact case-sensitive `content_type ∈ allowed_mime_types`, and aggregate `existing + size > max_total_bytes` (existing from `list_attachments` `size_bytes`). Absent policy / absent limit = no check. | Keeps validate and write-time consistent. |
| D4 | `attachment_add` input: `fileName` (required), exactly one of `content` (UTF-8 text) or `contentBase64`, optional `title`, `subdir`, `contentType`. Neither/both -> invalid params. Base64 decode failure -> invalid params. Decoding is input deserialization, so it lives in the shadow struct's `TryFrom`. `base64` (workspace dep, pure Rust, wasm-safe) added to `srs-mcp-core`. | Issue brief. |
| D5 | Write guard: `attachment_link` mutates a record, so `WriteGuard::check` treats it like `record_transition` (a guarded record rejects it; `fillOnlyFields` do not apply). `attachment_add` writes only a source document, never a guarded container/record, and is allowed. | #1165 semantics: protected records are read-only to the session. |
| D6 | Tool profiles: both tools are `full`-only. `context` stays knowledge-capture; no write tool joins `read`. A profile test already pins subsets of the catalogue. | #1287. |
| D7 | The optional source-document read resource is DEFERRED to #1333 (not needed for the add/link act; `srs://…/record/{id}` already shows `sourceRefs`). | Scope discipline. |
| D8 | ADR-048 checklist. Spec-first: no spec change (I-107 MAY reject). Layer test: policy comparison is a core service concern. One way per goal: the comparison is one pure function `attachment_policy_service::policy_violation(&AttachmentPolicy, content_type, size, existing_total) -> Option<String>`; `check_attachment` calls it. Making `validation.rs` call the same function (so validate warns and write rejects from one comparison) and retiring `enforce_policy` once CLI/web enforce are tracked in #1332 (declared twin, end state: flag removed or policy-mode enum). Decision mode: complicated. Door: Door 1 (executes I-107's permission; non-normative implementation). | ADR-048; review finding 1-3. |
| D9 | The check runs after `content_type` is resolved (inference happens after the duplicate check) and before `save_binary_file`; a `None` `size_bytes` in the aggregate counts as 0. | review finding 5. |
| D10 | Trade-off recorded in the ADR-037 amendment: ADR-049 `WriteSummary` does not record source documents, so `attachment_add` yields no summary entry (web will not show it as agent activity); `attachment_link` shows as a record update. No per-tool summary builder. Guard: add `TOOL_ATTACHMENT_LINK` to the `record` match in `guard.rs` (falls to the generic deny); `attachment_add` is unguarded, so a guarded session can add unlimited bytes when no policy exists. | review findings 4, 6. |

No new ADR: implements ADR-037 and I-107. ADR-037 gets a dated amendment line for the two tools.

---

## Contracts

### CLI output contract (ADR-011)

No CLI command or payload changes (`AddAttachmentInput` is a service type; the CLI sets `enforce_policy: false`). `cargo test --test payload_contracts` must still pass.

### Entity schema sync

No `srs/docs/schema/2.0/` change.

---

## Scope

- `AddAttachmentInput.enforce_policy`, `attachment_policy_service::check_attachment`.
- MCP tools `attachment_add`, `attachment_link`; guard arm for `attachment_link`.
- Docs: `srs-usage.md` MCP tool list, `srs-rust/CLAUDE.md`/ADR-037 amendment, `docs/dogfooding.md` scenario.

**Out of scope:**

- Policy enforcement for `srs attachment add` / web, and unifying `validation.rs` onto `policy_violation` (#1332).
- Source-document read resource (#1333).
- Attachment tools in the `context`/`read` profiles.

---

## Phases

### Phase 1: Core enforcement

**Goal:** `add_attachment` can reject policy-violating content when asked.

**Agent:** Repository Worker

#### Tasks

- [x] `attachment_policy_service::policy_violation(policy, content_type, size, existing_total) -> Option<String>` (pure) and `check_attachment(store, content_type: &str, size: u64) -> Result<(), RepositoryError>` using `read_attachment_policy` and `attachment_service::list_attachments` for the aggregate.
- [x] Add `enforce_policy: bool` to `AddAttachmentInput`; call the check after file-name/duplicate validation and before any write.
- [x] Update every `AddAttachmentInput { .. }` construction (`crates/srs-cli/src/commands/attachment.rs`, `crates/srs-bindings/src/lib.rs`, `crates/srs-bindings/tests/attachment_bytes.rs`, `crates/srs-bindings/tests/list_attachments.rs`, and the ~9 test constructions in `crates/srs-repository/src/attachment_service.rs`) with `enforce_policy: false`.

#### Acceptance Criteria

- [ ] With a policy record, oversize / disallowed-MIME / over-total content is rejected and nothing is written (no content file, no sidecar).
- [ ] With `enforce_policy: false` or no policy, behaviour is unchanged.

#### Testing

```bash
cargo test -p srs-repository attachment
```

- `add_attachment_enforced_rejects_per_file_mime_and_total` — each limit rejects; store has no new files afterwards.
- `add_attachment_unenforced_ignores_policy` — same input succeeds.

#### Milestone gate

`cargo test -p srs-repository`, `cargo clippy -p srs-repository --all-targets -- -D warnings`, mark boxes, commit.

### Phase 2: MCP tools + guard

**Goal:** both transports expose `attachment_add` / `attachment_link`.

**Agent:** MCP Adapter Worker

#### Tasks

- [ ] `tools.rs`: `TOOL_ATTACHMENT_ADD`, `TOOL_ATTACHMENT_LINK`, `DESC_*`, `AttachmentAddToolInput` (`TryFrom` -> `AddAttachmentInput` with `enforce_policy: true`), `AttachmentLinkToolInput` (`From`), `all_tools` entries, `call_tool` arms (`tool_err` on service error).
- [ ] `guard.rs`: `attachment_link` handled like `record_transition` (record guarded -> deny).
- [ ] `Cargo.toml`: `base64 = { workspace = true }`.
- [ ] Extend `tool_input_conversion_second_wave_exercises_every_field` and `list_tools_advertises_every_tool_with_schemas` in `crates/srs-mcp-core/src/tools.rs`, and the profile test `tool_profile_filters_the_catalogue_and_refuses_the_rest` in `crates/srs-mcp-core/tests/surface.rs`.

#### Acceptance Criteria

- [ ] `tools/list` advertises both tools with input schemas; `read`/`context` profiles do not.
- [ ] Text and base64 add produce identical stored bytes; neither/both/bad base64 -> invalid params.
- [ ] A policy violation returns a tool error (`isError: true`), nothing written.
- [ ] `attachment_link` links, and a duplicate link / unknown document / missing record returns a tool error.
- [ ] A guarded record rejects `attachment_link`; `attachment_add` is allowed under a guard.

#### Testing

```bash
cargo test -p srs-mcp-core
cargo test -p srs-mcp
cargo test -p srs-bindings
```

- `attachment_add_and_link_round_trip` (MemoryStore/tempdir via existing mcp-core test helpers).
- `attachment_add_rejected_by_policy`, `attachment_add_content_exclusive`, `guard_rejects_attachment_link_on_protected_record`.

#### Milestone gate

`cargo test -p srs-mcp-core -p srs-mcp -p srs-bindings`, clippy, mark boxes, commit.

### Phase 3: Docs

**Goal:** docs match the code.

**Agent:** Lead Integrator

#### Tasks

- [ ] ADR-037 dated amendment; `srs-usage.md` MCP tool list (in the `srs` repo, separate branch); the MCP server instructions text in `crates/srs-mcp-core/src/lib.rs` if it enumerates tools; `docs/dogfooding.md` scenario.

#### Acceptance Criteria

- [ ] Every doc command block touched still runs.

#### Milestone gate

Docs grep for stale tool lists; commit.

---

## Final Acceptance

- [ ] `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` exit 0
- [ ] `cargo test --test payload_contracts` passes (no payload change)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no schema change)
- [ ] Dogfood: `srs mcp serve` session adds + links an attachment, policy rejection observed

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Lead Integrator owns final API naming and dependency boundaries.
- At the end of each phase: verify acceptance criteria, update checkboxes, commit.

## Assumptions

- `read_attachment_policy` never errors on an absent policy (documented), so enforcement is a no-op without one.
- The browser `McpSession` dispatches through `srs_mcp_core::tools::call_tool` (lib.rs:683), so it gains the tools with no bindings change.
