# ADR-037: MCP Adapter Surface

- **Status:** accepted
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

5. **Async stays contained.** `serve_stdio` builds a tokio current_thread runtime internally and blocks. No `srs-repository`/`srs-core` signature becomes async. Handlers call sync services directly and construct a fresh `FileStore` per request (mirroring the CLI's per-invocation semantics: no shared mutable state, on-disk changes visible between calls). Blocking file I/O inside async handlers is accepted for a single-client stdio server; revisit if a multi-client transport lands. **Explicit scope limit:** the MCP server serves file-backed repositories only — `.srsj` (JsonStore) repos are not servable in this cut (unlike the CLI's `--store` switch). A follow-up may wire the store selection through; until then this is a named limitation, not an oversight.

6. **The `srs://` URI scheme is implementation tooling, not spec.** Resources use `srs://<repositoryId>/{map|navigation|record/<instanceId>|container/<containerId>|view/<documentViewId>}`. This maps onto existing spec ID components (ext:addressability defines component-based Addresses, Invariant 34 — not a URI syntax), so no spec change is required (spec-gate ruling on #676). If a canonical URI syntax is later wanted, that is a tooling-only spec note under ext:addressability — a deferred follow-up, not constrained by this ADR beyond "addresses are built from existing ID components".

## Consequences

**Positive:** Every MCP client becomes an SRS client with the validating write contract intact; one more consumer proves the ADR-001/010 service boundary; protocol conformance is delegated upstream; future transports are incremental.

**Negative / trade-offs:** ~75 new locked packages and an async runtime enter the workspace (confined to one adapter crate); binary grows ~21%; a second schemars major (1.x) rides along; the envelope carve-out means one CLI command is intentionally outside the payload-contract test net — its behaviour is covered by MCP integration tests instead. The ADR-011 "no schemars in library crates" rule forces `srs-mcp`'s tool-input structs to be *shadow copies* of the canonical service inputs — a drift risk of the same class ADR-011 closed for outputs; mitigated by mandatory `From<ToolInput>` conversions (handlers may only reach services through them) plus a unit test exercising every field. Tool description text is likewise single-sourced as constants in `srs-mcp`, with the `srs-usage.md` MCP section written from them; cross-repo CI drift enforcement is deliberately not added (srs-rust CI does not check out the sibling `srs` repo — spec independence).

**Neutral:** The server is single-repo per process (`--repo` at startup), matching the CLI's invocation model. Multi-repo serving, subscriptions/notifications, and HTTP transport are follow-up issues, not part of this decision. **Prompts are now implemented** (srs-rust#682, Amendment 2026-07-24 below).

## Amendment (2026-07-22, #692) — type discovery

The URI enumeration in §6 gains `srs://<repositoryId>/type/<typeId>`. Type schemas are
exposed **both** as resources (concrete enumeration of every loaded-package type in
`resources/list`, plus a `type/{typeId}` template) **and** as a `type_schema` tool — the
dual exposure is deliberate: resources serve browsing/enumeration, while MCP clients
surface tools to the model more reliably, and the schema is the authoring contract an
agent must read before `record_create`. Both forms serve the existing
`type_schema_service::type_schema` projection verbatim (`{schema, diagnostics}` — the
`field_to_property` schema whose properties carry `x-srs-field-id`, `x-srs-ai-guidance`,
`x-srs-widget`, and ADR-026's `x-srs-description`/`x-srs-instructions`). No new
dependency, no new semantics; the shadow-input/`From`-conversion drift guard extends to
`TypeSchemaToolInput`.

## Amendment (2026-07-24, #682) — prompts surface

`srs-mcp` now implements the MCP `prompts` capability: `prompts/list` returns one `Prompt` per installed blueprint (name = blueprint UUID; description = `{namespace}/{name} v{version}: {desc}`); `prompts/get` with a blueprint UUID calls `blueprint_brief_service::blueprint_brief` and returns the rendered markdown as a `Role::User` `PromptMessage`. Both handlers follow the thin-wrapper pattern (one service call each). The `prompts` capability is advertised via `ServerCapabilities::builder().enable_prompts()`. `list_changed` notifications and cursor-based pagination remain follow-up items (srs-rust#749, srs-rust#752).

## Amendment (2026-09-06, #949) — tree, agent-index, descent hook

The URI enumeration in §6 gains `srs://<repositoryId>/tree` (recursive `contains` tree from auto-detected roots), `srs://<repositoryId>/tree/<instanceId>` (subtree rooted at one instance; also a resource template), and `srs://<repositoryId>/agent-index` (the AI orientation index). Each serves its existing service result verbatim — `tree_service::build_tree` (`TreeResult`, now `Serialize`; the CLI's ASCII `text` rendering is CLI presentation and is not carried) and `agent_index_service::build_agent_index` (`AgentIndex`). `ResolvedMember` (the container resource's member entry) gains `sectionContainerId` — the same key `NavigationNode` already carries — so an agent over MCP can descend container → member → sub-container without listing every container. No new traversal, no new semantics.

## Amendment (2026-10-02, #1157) — shared create input, schema derived once

The shadow-copy rule is relaxed where a service input is shared by all three adapters. `container_service::ContainerCreateInput` (`deny_unknown_fields`, every authorable `container.json` property) is the one `container_create` input for the CLI, the WASM binding and MCP; MCP's `inputSchema` is derived from it, not from a mirror. `srs-repository` gains an **optional** `mcp-schema` feature (`schemars` 1.x) that only `srs-mcp-core` enables. It adds no crate to any build: every build that turns it on (`srs` binary, `srs-bindings` wasm) already carries schemars 1.x through `srs-mcp-core`, and builds without `srs-mcp-core` stay schemars-free in the library crates. ADR-011's 0.8 payload-golden generator in `srs-cli` is unaffected. Other tool inputs keep the shadow-copy pattern until they are shared the same way.

## Amendment (2026-10-03, #1134) — record context resource

The URI enumeration in §6 gains `srs://<repositoryId>/context/<instanceId>` and `srs://<repositoryId>/context/<containerId>/<instanceId>` (one resource template, `context/{containerId}/{instanceId}`). The resource is one `context_query_service::get_record_context` call served verbatim — the same read as `srs context record` (the global `--container` supplies the container) and the WASM `context_record` binding. Per owner ruling, `get_record_context` was extended, not paralleled: `relations` now holds both directions (`direction: out|in`, the other endpoint inline as `neighbour`), and a container id adds the record's outline `entry` and `subtree` from `container_service::get_outline`. Native and browser MCP share `srs_mcp_core::srs_resources::read_resource`, so their output is identical by construction; `crates/srs-bindings/tests/context_query.rs` pins it. The server `instructions` text gains one sentence. No spec change.

## Amendment (2026-10-04, #1188) — optional relation-category filter on the context resource

The context template becomes the RFC 6570 form `context/{containerId}/{instanceId}{?excludeRelationCategories}`; `srs_mcp_core::uri` parses `?excludeRelationCategories=<category>[,<category>...]` (comma list, `&`-separable, unknown keys are a URI error, unknown category values are `invalid_params`) and `format` round-trips it. The value maps to `RecordContextQuery.exclude_relation_categories`: the service drops any edge whose relation type's installed `RelationTypeDefinition.category` is listed, never by relation name; edges whose type has no installed definition are kept; empty (the default) leaves the read unchanged and does not load the package. The same field is the CLI `srs context record --exclude-category <cat>` (repeatable) and the WASM `context_record` key `excludeRelationCategories`. Output shape is unchanged (no payload or golden-schema change). Category spellings parse once, in `FromStr for RelationTypeCategory` (srs-core). No spec change.

## Amendment (2026-10-06, #1285) — compact context: `projection` and `format=markdown`

The context template becomes `context/{containerId}/{instanceId}{?excludeRelationCategories,projection,format}`. Both parameters are optional; omitting them leaves the read byte-identical.

**`?projection=full|card|label`** maps to `RecordContextQuery.projection`, the same field as the CLI `srs context record --projection` and the WASM `context_record` key `projection`.
- `card` replaces each inlined neighbour with a `{kind: "card"}` `NeighbourSummary`: instanceId, uri, label, type, lifecycleState, and `summary`. `summary` is the first non-label string field in Type order, at most 160 chars.
- `label` is the same card without `summary`.
- Both drop each edge's endpoint labels and provenance. The context record itself stays whole.
- `NeighbourSummary` is the existing `neighbours` shape, gaining two optional fields that `relation neighbours` never sets. One shape, not a second.

**`?format=markdown`** returns `text/markdown` from `context_query_service::render_record_context_markdown`:
- The record's fields come first, as `render_service`'s baseline rows (`render_record_rows`: the same order, labels, value rendering and row primitive as a composition's baseline path, so there is no second field renderer).
- Then one line per edge, grouped by relation type and direction: label, type, lifecycle state, instance id, and the card summary beneath.
- The projection is forced to `card` unless it is `label`.
- The same renderer backs the CLI `srs context record --markdown` (payload `ContextRecordMarkdownPayload {recordId, rendered}`, golden `context-record-markdown.json`) and the WASM `context_record_markdown`.
- `format` accepts only `json|markdown`; `json` is the default form and formats back without the key. Any other value, an unknown projection, or a duplicated key is a URI error, as on tree URIs.

Measured on srs-context's `srs-repository` component (25 edges): full read ~9.1k tokens via the CLI (~12.5k via MCP), `card` JSON ~4.6k, `label` JSON ~3.6k, card markdown ~1.8k (~0.35k of it the record's own fields), label markdown ~0.9k. No spec change: these are read-time projections of an implementation resource, not a spec construct.

## Amendment (2026-10-04, #1229) — bounded `neighbours` tool, tree query parameters

The tool catalogue gains `neighbours {instanceId, relationType?, direction?, limit?, offset?}`: one `context_query_service::list_neighbours` call, the same read as `srs relation neighbours` and the WASM `neighbours` binding. It returns `total` (every matching edge) and a page of edges (`direction`, `relationId`, `relationType`, and the neighbour's `instanceId`, `label`, `typeNamespace`/`typeName`; never the record), sorted `(relationType, relation createdAt none-last, relationId)`. Only the returned page loads its neighbours, so a 782-edge hub costs one page. The service pages with `limit: None` = all (like `FindPage`); the MCP adapter defaults `limit` to 25 and clamps it to 100. `NeighboursToolInput` is a shadow input with an `into_parts` conversion and a unit test exercising every field. The neighbour `uri` field is added by #1227 (see its amendment). The `tree` and `tree/{instanceId}` resources gain `?maxDepth=&relationType=&typeFilter=` (template `tree/{instanceId}{?maxDepth,relationType,typeFilter}`), parsed in `srs_mcp_core::uri` into `TreeQuery` and passed to `TreeOptions`; a non-integer `maxDepth`, a duplicated key, an unknown key, or any query on a non-tree, non-context URI is a URI error (`invalid_params`). Values are taken verbatim (no percent-decoding; `namespace/name` and relation keys need none). Tree controls are URI parameters, not a second tree tool (one way per goal). No spec change.

## Amendment (2026-10-04, #1220) — `read` tool for resource-blind clients

claude.ai (browser MCP relay) calls tools but not resources, so every `srs://` resource was unreachable. The catalogue gains `read {uri}`, which calls the same `srs_resources::read_resource` dispatch as `resources/read` (one path; no per-URI logic) and returns the text as a tool result — the dual-exposure pattern of the #692 amendment. Decisions:

- **Size cap, `read` only.** The tool's text is capped at `MAX_READ_BYTES` = 96,000 (policy for the ~128 KB browser relay, not protocol). A longer result is cut on a UTF-8 boundary, ends with a notice naming bounded alternatives (`find` limit/offset, `tree/{instanceId}`, `container_outline`, `record/{id}`), and `structuredContent` carries `{uri, mimeType, truncated, totalBytes, shownBytes}` (metadata only; the payload is not doubled). Truncated JSON is not valid JSON: `truncated` is the contract. `resources/read` is unchanged and unbounded; `read` is therefore equal to it for results at or under the cap. Other tools are paged (`find`) or schema-sized and carry no cap.
- **Errors as today.** `read_resource`'s `McpApplicationError` (invalid uri / wrong repository -32602, not found -32002) propagates unchanged: one error contract per address, per the issue. Tool-level `isError` is not used for resource errors.
- **Routing.** Catalogue entry, input shadow struct and `read_tool` live in `srs-mcp-core::tools`; `SrsMcpApplication::call` routes the name because only it knows the repository id. `read` is read-only and bypasses the write guard and the change drain (ADR-049).
- ADR-037 §1 "one service call": the truncation is adapter presentation of that one call's text, not semantics.

Implementation charter (ADR-048): spec-first — none needed (§6); layer — adapter exposure; one way per goal — a single resource dispatch; decision mode — complicated (owner-approved plan, #1220).

## Amendment (2026-10-04, #1227) — navigable hits, one URI builder, `record/{id}` for notes

`find` hits gain `uri` (`srs://<repo>/record/<id>`), `typeId` and `containerIds`; `NeighbourSummary` gains `uri`; `agent-index` `types[]` gain `typeId` and `entryPoints` becomes `{path, instanceId?, uri?}[]`. Decisions:

- **One builder, in core.** Hits and neighbours are built in `srs-repository`, so the instance/type/container URI builders live in `srs_repository::resource_uri`; `srs_mcp_core::uri::format` delegates to it for those kinds (parse and the singleton kinds stay in mcp-core). The same field therefore reaches the CLI, WASM and MCP through the one service type (capability layering).
- **`containerIds` is declared membership only** (RFC-034 / RFC-043): the containers whose own outline lists the instance, never `contains` traversal or nested-child effective membership. Built from one `container_service::membership_index` pass over the returned page.
- **`entryPoints` shape break accepted.** `string[]` to `{path, instanceId?, uri?}[]` is a payload change; consumers were checked (srs-vscode, srs-web, srs-bindings, muDemocracy.org: no reader of `agentIndex.entryPoints`; the only hits are an unrelated esbuild option). Paths that do not name an instance keep `path` only. Pre-1.0 contract; the alternative (parallel `entryPointUris` array) was rejected as a second way to carry one fact.
- **`record/{id}` is any tier**, as its template says: it resolves through `get_instance_by_id` and serves a Tier-0 note as a Note (it failed with "missing field typeId").

Golden schemas: `find`, `repo agent-index` and `relation neighbours` embed the service types as opaque values, so no golden diff (`generate-schemas` re-run, clean). No spec change.

## Amendment (2026-10-04, #1219) — `find` facets and `limit: 0`

The MCP `find` reply carries `facets` (see ADR-019's #1219 amendment): the adapter serialises the service result unchanged and adds no counting. The default `limit` stays 25; an explicit `limit: 0` passes through and returns no hits with full facets, which is the cheap repository map for a resource-blind client (about 14 KB on the 886-instance muSrs; a default-limit call about 15 KB). The facets shape is not pinned by the `find` golden schema, which embeds `DiscoveryResult` opaquely; the service, MCP and bindings tests pin it. No spec change.

## Amendment (2026-10-07, #1286) — `find` facets opt-in, `projection`

Supersedes the first sentence of the #1219 amendment: the MCP `find` reply carries `facets` only for `limit: 0` or `facets: true` (ADR-019's #1286 amendment), and `find`/`similar` accept `projection` (`full|card|label`). The adapter still adds no counting and no trimming: both inputs map one-to-one onto `FindPage`. The MCP default projection stays `full` (as on the CLI and WASM), so existing clients see the same hits; agents ask for `card`. Since #1286 the `find` golden schema pins the facets shape (optional, never `null`) rather than embedding it opaquely. No spec change.

## Amendment (2026-10-07, #1287) — tool profiles

`tools/list` is paid for in context on every session: 33 tools with full input schemas are about 44 KB (about 11k tokens), while an agent keeping project memory uses about ten of them. `srs-mcp-core::tools::ToolProfile` defines three fixed tool sets, once, for every transport:

- `full` (default): the whole catalogue, unchanged.
- `context`: `repo_validate`, `find`, `read`, `type_schema`, `record_create`, `record_update`, `record_allowed_transitions`, `record_transition`, `record_successor`, `note_create`, `relation_create`, `container_member_add` (about 20 KB, about 5.1k tokens). `repo_validate` and `record_allowed_transitions` are in because the server's own guidance tells the agent to call them after writes and before transitions. `neighbours` and `similar` are left out: the `context/{id}` resource (via `read`, `?projection=card`) carries a record's edges, and `find` with `match: "any"` covers lookup before create.
- `read`: discovery, reads, outlines, validation and the read-only protocol/package/lifecycle queries; no tool that writes (about 15 KB).

Decisions:

- **The instructions name the session's tools.** `srs_metadata::initialize_result_for(profile)` appends one sentence listing the profile's tools and saying any other tool named in the guidance is unavailable, so a model does not follow the general instructions into an unknown-tool error. `full` keeps the instructions unchanged.
- **Host-chosen, enforced, not advisory.** The profile is set by the host (`srs mcp serve --profile`, `SrsMcpServer::with_tool_profile`, WASM `McpSession.set_tool_profile`), never by a client request. `SrsMcpApplication` filters `tools/list` and refuses a call to a tool outside the profile with the unknown-tool error (`-32602`), before argument parsing, the write guard and any store access, so a `read` session cannot write by naming a hidden tool. `read` is in every profile.
- **Sets live in core, as tool-name constants.** One list per profile in `tools.rs`, next to the catalogue; a unit test proves every entry names a real tool and that `read` carries no write verb. Adapters only parse the profile name (one `FromStr`).
- **Descriptions unchanged.** The issue's alternative (shorter descriptions pointing at a docs resource) was not needed to reach the `context` target and would have moved guidance out of the one place a model reliably reads it.

Implementation charter (ADR-048): spec-first — none needed (MCP surface, §6); layer — `srs-mcp-core` owns the sets, adapters map a flag; one way per goal — one filter for list and call; decision mode — complicated.

## Amendment (2026-10-07, #1319) — compact JSON text

The `text` of every JSON tool result (`tool_ok`) and JSON resource read (`json_contents`) is serialized compactly through one helper, `srs_mcp_core::json_text`, instead of `to_string_pretty`. The reader is a model paying per token for indentation (about 30% of a `find` page); the value, keys and `structuredContent` are unchanged, and a client can re-format. Markdown resources and the CLI's `--pretty` are untouched. No spec change.

## Amendment (2026-10-07, #1327) — attachment tools

`attachment_add` (`fileName`, `content` UTF-8 text or `contentBase64`, optional `title`/`subdir`/`contentType`) and `attachment_link` (`instanceId`, `documentId`) join the `full` profile (not `context`/`read`). Each calls the CLI's service (`attachment_service::add_attachment` / `link_attachment`); base64 decoding is input deserialization in the shadow struct's `TryFrom`. The MCP tool sets `AddAttachmentInput.enforce_policy`, so `add_attachment` rejects, before any write, content that breaks the RFC-017 `attachment_policy` (I-107 permits hard rejection); the CLI and WASM paths keep validate-time warnings until #1332 decides whether human flows reject too (the flag is a declared twin; end state: removed or a policy mode). The check fails open when the policy record is unreadable, like validate. Trade-offs: ADR-049's `WriteSummary` does not record source documents, so `attachment_add` produces no summary entry (`attachment_link` appears as a record update); the session write guard rejects `attachment_link` on a protected record (like `record_transition`) but does not guard `attachment_add`, so without a policy a guarded session can still add arbitrary bytes. No spec change.

