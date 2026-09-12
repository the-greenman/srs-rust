# MCP Streamable HTTP Profile — 2025-06-18

- **Status:** compatibility-spike contract; no transport implementation is implied
- **Owner:** [srs-web#309](https://github.com/the-greenman/srs-web/issues/309)
- **Consumers:** `browser-executor-relay`, the browser MCP HTTP adapter, and the future
  transport-neutral `srs-mcp` core ([srs-rust#1057](https://github.com/the-greenman/srs-rust/issues/1057))
- **Normative base:** [MCP Streamable HTTP, 2025-06-18](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports)

This profile is the first delivery gate for browser-hosted SRS MCP.  It deliberately
does **not** implement a transport or extract the MCP core.  It fixes the wire contract
which the compatibility spike must prove before either change begins.

## Profile boundary

The public caller endpoint implements Streamable HTTP revision `2025-06-18` with a
single-response JSON mode.  It does not implement server-initiated messages, SSE,
stream resumption, or HTTP MCP sessions.

The permanent caller URL is a bearer capability, not a session URL.  It selects a
currently connected browser executor; it does not identify a repository to the relay.
The relay returns a non-MCP error while there is no executor, rather than synthesising
an MCP `initialize` or tool list.

This profile does not support the deprecated 2024-11-05 HTTP+SSE transport.  A client
which falls back to that transport after receiving `405` is unsupported by this profile.

## Request and response rules

| Input / condition | Owner | Result |
| --- | --- | --- |
| `POST` containing one JSON-RPC request | browser HTTP adapter + MCP core | `200`, `Content-Type: application/json`, exactly one JSON-RPC response object |
| `POST` containing one JSON-RPC notification or response | browser HTTP adapter + MCP core | `202`, empty body, no content type |
| `POST` with malformed JSON | browser HTTP adapter + MCP core | `400`, `application/json`, JSON-RPC parse error (`id: null`) |
| `POST` with a syntactically valid but invalid JSON-RPC message | browser HTTP adapter + MCP core | `400`, `application/json`, JSON-RPC invalid-request error (`id: null`) |
| `POST` whose top-level body is a batch | browser HTTP adapter | `400`, `application/json`, JSON-RPC invalid-request error (`id: null`); do not dispatch any batch member |
| Missing/unsupported `Content-Type` | browser HTTP adapter | `415`, empty body |
| `Accept` does not advertise both `application/json` and `text/event-stream` | browser HTTP adapter | `406`, empty body |
| `GET` | caller entrypoint | `405`, `Allow: POST`, empty body (no standalone SSE stream) |
| `DELETE` | caller entrypoint | `405`, `Allow: POST`, empty body (no HTTP MCP session exists to delete) |
| Any other method | caller entrypoint | `405`, `Allow: POST`, empty body |
| Browser executor unavailable | generic relay | `503`, `application/json`, `{ "error": "executor_offline" }`; never an MCP response |
| Executor replaced after call admission | generic relay | `503`, `application/json`, `{ "error": "executor_replaced" }`; retryable |
| Executor misses the relay deadline | generic relay | `504`, `application/json`, `{ "error": "executor_timeout" }`; an earlier write may have executed |
| Relay queue/limit exceeded | generic relay | `429`, `application/json`, `{ "error": "executor_overloaded" }`; retryable |

All caller responses carry `Cache-Control: no-store` and `Referrer-Policy: no-referrer`.
They must not echo a capability, request body, or executor response in generic error
text.  The relay applies these capability-protection headers independently of MCP;
the browser adapter must preserve them when it supplies the application response.

### POST requirements

The caller sends UTF-8 JSON.  `Content-Type` must be `application/json` (parameters
such as `charset=utf-8` are permitted).  `Accept` must contain both
`application/json` and `text/event-stream`; the response is always the JSON option.
The adapter accepts one JSON-RPC message only — never an array — which is the
Streamable HTTP 2025-06-18 body shape.

An `initialize` request is negotiated by the MCP core from its JSON-RPC parameters.
The `MCP-Protocol-Version` HTTP header is optional on that request.  Every other POST
must carry exactly `MCP-Protocol-Version: 2025-06-18`; a missing, malformed, or other
value returns `400` with an empty body and does not reach MCP dispatch.  This stricter
rule intentionally declines the specification's legacy no-header fallback to
`2025-03-26`.

The server issues no `Mcp-Session-Id` header and does not accept one: a request carrying
`Mcp-Session-Id` returns `400` with an empty body.  Session state would contradict the
stateless relay design.  `Last-Event-ID` is ignored because this profile has no SSE.

An explicit `notifications/cancelled` message is an ordinary accepted JSON-RPC
notification and therefore receives `202`; it is forwarded to the browser/core.  An
HTTP disconnect is **not** a cancellation and must not cause the relay to send one.

### Origin policy

Native MCP clients commonly omit `Origin`, which this profile permits.  If an `Origin`
header is supplied, the caller entrypoint validates it against the configured application
origin and returns `403` without dispatch on a mismatch.  There is no CORS support for
the bearer caller endpoint.  The executor WebSocket has a separate, mandatory configured
application-origin check.

## Layer ownership

| Layer | Owns | Must not own |
| --- | --- | --- |
| Generic relay | capability authentication, origin/configuration enforcement, bounded opaque HTTP envelope forwarding, executor-generation/request correlation, lifecycle failures, queue/deadline limits, capability-safe headers | MCP methods, JSON-RPC parsing, protocol versions, sessions, SRS data, tool schemas, or repository identity |
| Browser MCP HTTP adapter | this profile's method/content/accept/session/version gates; conversion of an MCP dispatch outcome into the profile's HTTP status/body; binding a request to the browser's current repository epoch | SRS service logic, tool schemas, relay credential validation, or relay request correlation |
| Transport-neutral Rust MCP core | JSON-RPC parsing/error result, initialization negotiation, MCP capability metadata, resources, prompts, tools, and all generic SRS semantics | HTTP headers/statuses, Cloudflare, WebSocket, caller capability, browser view state, or provider persistence |
| Browser host | serialisation with UI/Save mutations, repository epoch transitions, dirty/recovery revision updates, WASM-store invalidation after successful mutation | relay protocol semantics or duplicate SRS validation |

The generic relay transports a method, a bounded allowlist of request metadata required by
the browser adapter (`Content-Type`, `Accept`, `MCP-Protocol-Version`,
`Mcp-Session-Id`, and `Origin`), and an opaque UTF-8 body.  Its response envelope has
only status, allowlisted headers, and opaque body.  It must neither parse nor classify
JSON-RPC/MCP data.

## Repository epochs and write ambiguity

The caller URL addresses an executor channel, not a fixed repository.  The browser host
maintains a monotonically increasing repository/executor epoch.  On unload, switch,
restore/migration, or executor takeover it stops admission, fails queued/in-flight old
epoch calls retryably, replaces the dispatcher, then resumes.  Relay request IDs are
generated independently of JSON-RPC IDs and replies are bound to executor generation;
late or prior-generation replies are discarded.

No automatic retry/replay is permitted for a timed-out mutating POST.  A timeout,
browser disconnect, takeover, or redeploy may be ambiguous: the mutation may already
have occurred in the old browser working copy even though the caller received no result.
The caller must rediscover/read state before choosing a deliberate retry.

## Fixture contract

`crates/srs-mcp/tests/fixtures/streamable-http-2025-06-18.json` is a transport-neutral
set of request/response vectors.  It deliberately uses generic JSON-RPC probe methods,
not SRS tools, so a future native HTTP adapter, browser/WASM adapter, and the reusable
relay's echo probe can execute the same cases.  The fixture excludes executor-offline,
replacement, timeout, and overload cases because those are relay lifecycle cases rather
than MCP dispatch cases; the generic relay package owns their executable tests.

The fixture manifest test checks that every mandatory profile case exists and that every
JSON body declared valid/error is valid JSON.  Once an adapter exists, its integration
test must execute every vector and compare status, required/absent headers, dispatch
expectation, and response kind.  The compatibility spike records whether real clients
actually meet the `Accept`, protocol-version, and no-session constraints before core
extraction begins.
