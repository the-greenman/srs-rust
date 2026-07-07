# Plan: gh-project.mjs: board() null-guard for proxy-blocked GraphQL

## Summary

`board()` in `scripts/gh-project.mjs` reads `data.user.projectV2.items` without null-checking
the response. In the Claude Code cloud/web environment the network proxy intercepts
`api.github.com/graphql` and returns HTTP 200 with a non-GraphQL error body when Projects v2 is
not enabled. Because the body has no `data` key, `graphql()` returns the raw error object; then
`data.user` is `undefined` and the property access throws `TypeError: Cannot read properties of
undefined (reading 'projectV2')` — a confusing failure that reads as a code bug rather than an
environment limitation. The fix adds the same defensive optional chaining that `meta()` already
uses, plus proxy-error detection that emits a clear actionable message instead of crashing.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Script Worker | Claude (this session) |
| Verification | Claude (this session) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan implements a defensive guard using existing patterns
already present in `meta()`. No new ADR required; the change is a bug fix with no design
consequences.

| ADR | Decision | Status |
|---|---|---|
| n/a | No Rust crate boundaries crossed; `scripts/gh-project.mjs` is a standalone utility script | — |

---

## Contracts

### CLI output contract (ADR-011)

**No new/changed commands.** `scripts/gh-project.mjs` is a standalone Node.js utility, not a Rust
CLI handler. No payload structs changed; no schema regeneration needed.

### Entity schema sync (check-schema-sync.sh)

**No schema changes.** This plan modifies only `scripts/gh-project.mjs`.

---

## Scope

- Add optional chaining to `data.user?.projectV2?.items` in the `board()` `do...while` loop.
- Detect when `items` is `undefined` (proxy-blocked or unexpected response shape) and call
  `die()` with the message: `"Projects v2 GraphQL is unavailable in this session — <proxy msg or
  'unexpected response shape'>; run from a local machine or CI"`.
- Surface the raw `data.message` or `data.errors?.[0]?.message` if present for additional context.

**Out of scope:**

- Adding organization fallback to `board()` (it currently only queries `user(login:$owner)`; that
  is a pre-existing limitation, not related to this crash). File as a follow-up.
- Changing error handling in `meta()` — it already works correctly.
- Any Rust crate changes.

---

## Phases

### Phase 1: Guard board() against null/undefined data.user

**Goal:** `board()` no longer throws TypeError on a proxy-blocked response; it calls `die()` with
a clear, actionable message.

**Agent:** Script Worker

#### Tasks

- [x] Open `scripts/gh-project.mjs`.
- [ ] In the `do...while` loop in `board()` (line ~408-409), replace:
  ```js
  const items = data.user.projectV2.items;
  ```
  with:
  ```js
  const items = data.user?.projectV2?.items;
  if (!items) {
    const why = data.message || data.errors?.[0]?.message
      || "unexpected response shape";
    die(`Projects v2 GraphQL is unavailable in this session — ${why}; run from a local machine or CI`);
  }
  ```

#### Acceptance Criteria

- [ ] `data.user?.projectV2?.items` is used (optional chaining, no TypeError on undefined).
- [ ] When `items` is falsy, `die()` is called with a message containing the substring
  `"Projects v2 GraphQL is unavailable in this session"`.
- [ ] When `items` has a `message` field in the raw data, that message is included in the
  `die()` output.
- [ ] No regression: when `data.user.projectV2.items` is a valid items object, `board()` continues
  to paginate and return results as before.

#### Testing

No Rust tests are involved. Manual verification:

```bash
# Confirm the guard is syntactically valid and the module loads
node -e "import('/home/user/srs-rust/scripts/gh-project.mjs').catch(e => { console.error(e); process.exit(1); })"
```

Specific tests to write or verify:

- Manual code inspection of the changed lines to confirm optional chaining is present.
- Dogfooding in Stage 7.6 will exercise the script end-to-end (within proxy constraints).

#### Milestone gate

1. Verify all acceptance criteria are met.
2. Confirm the change is in place with correct syntax.
3. Module loads without error:
   ```bash
   node --input-type=module < /dev/null   # not applicable — use the import check above
   ```
4. Mark completed checkboxes `[x]`.
5. Commit.

---

## Final Acceptance

- [ ] `scripts/gh-project.mjs` contains `data.user?.projectV2?.items` (optional chaining)
- [ ] `board()` calls `die(...)` with `"Projects v2 GraphQL is unavailable in this session"` when
  items is absent
- [ ] No other function in the file is changed
- [ ] `cargo test` passes (no Rust code changed, but verifies the repo is healthy)
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)

## Coordination Rules

- Single-agent execution; standard pipeline rules apply.
- Do not modify any Rust crates.

## Assumptions

- The proxy returns a response body parseable as JSON (object with optional `message` or `errors`
  key). If the proxy returned an empty body or non-JSON, `ghJson()` would throw before reaching
  the guard — that edge is pre-existing and out of scope.
- `board()` is only ever called for the configured `OWNER`/`PROJECT_NUMBER` (user account, not
  org); the org-fallback gap is pre-existing and filed as a follow-up deferred issue.
