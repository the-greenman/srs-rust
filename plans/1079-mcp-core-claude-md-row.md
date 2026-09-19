# Plan: Add `srs-mcp-core` to CLAUDE.md Crate Authority table

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

PR #1064 (part of epic #1057) added a new workspace crate, `srs-mcp-core` — a transport-agnostic
MCP application core (initialization, capability metadata, URI routing, JSON-RPC dispatch,
resource/prompt/tool semantics shared between native and WASM adapters). `CLAUDE.md`'s "Crate
Authority — what lives where" table is the authoritative "what lives where" reference for this
workspace, but it was never updated with a row for the new crate. This plan adds that one row,
worded from the crate's actual current scope (`crates/srs-mcp-core/src/lib.rs`,
`crates/srs-mcp-core/Cargo.toml`), closing issue #1079.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |

No specialized worker role is needed: this is a single-file, single-row markdown edit to the
repo-root `CLAUDE.md`, which is outside every existing worker's write scope (`Repository
Worker`/`Bindings Worker`/`CLI Worker`/`MCP Adapter Worker` all scope to `crates/**`) and too
small to warrant a new "Docs Worker" role in `agents.md`. The Lead Integrator makes the edit
directly. See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs. This plan documents an existing crate boundary already established by ADR-037 (MCP
adapter surface — `srs-mcp` is the sole crate depending on `rmcp`/`tokio`) and already implemented
by `srs-mcp-core`'s actual dependency graph (`srs-repository`, `serde`, `serde_json` only, plus
`uuid`/`getrandom` under `cfg(target_arch = "wasm32")`). The row being added restates that
boundary in the authority table; it does not change it.

| ADR | Decision | Status |
|---|---|---|
| [ADR-037](../docs/adr/037-mcp-adapter-surface.md) | `srs-mcp` is the sole crate owning `rmcp`/`tokio`; a transport-agnostic core sits underneath it | accepted (pre-existing; this plan documents its consequence for `srs-mcp-core`) |

No new architectural decisions — this plan documents ADR-037's existing boundary.

---

## Contracts

### CLI output contract (ADR-011)

No new/changed commands — no action required; golden schemas stay as-is.

### Entity schema sync (check-schema-sync.sh)

No — this plan does not touch `srs/docs/schema/2.0/` or any schema mirror.

---

## Scope

- Add one row for `srs-mcp-core` to the "Crate Authority — what lives where" table in
  `srs-rust/CLAUDE.md`, describing:
  - **Owns:** transport-agnostic MCP application core — initialization/capability metadata
    (`srs_metadata`), URI routing (`uri`), the generic JSON-native resource contract
    (`srs_resources`), and JSON-RPC dispatch (`McpDispatcher`/`McpApplication`) shared between
    native (`srs-mcp`) and WASM adapters.
  - **Hard constraints:** no `rmcp`, Tokio, stdio, file paths, or `FileStore`; carries
    `wasm32-unknown-unknown`-only dependencies (`uuid` with the `js` feature, `getrandom` with
    `wasm_js`) gated under `cfg(target_arch = "wasm32")`, reflecting the intent that it must
    compile for that target.
- Update the top-of-file crate list sentence ("Rust implementation of the SRS system:
  `srs-core`, `srs-repository`, `srs-cli`, `srs-bindings`, `srs-mcp`, `srs-projection`.") to
  include `srs-mcp-core` if it is still missing there once the table is updated.
- Grep `CLAUDE.md` (and any other doc under `srs-rust/`) for other crate-list enumerations that
  are now stale because of the missing crate, and fix any found.

**Out of scope:**

- Any change to `srs-mcp-core`'s actual code, `Cargo.toml`, or dependency graph.
- Wiring `srs-mcp-core` into the `wasm32-unknown-unknown` CI build job (`ci.yml` currently builds
  only `-p srs-bindings` for that target) — that is implementation work for epic #1057, not a doc
  fix, and is out of scope here.
- Updating `semanticops/CLAUDE.md`'s "Architecture → Rust crate boundaries" section — Stage 7.5's
  surface-to-doc map routes *crate responsibility/boundary* changes there, but this issue is
  scoped to the `srs-rust/CLAUDE.md` table specifically (per the issue body) and no such section
  was found in `semanticops/CLAUDE.md` referencing individual Rust crates by name beyond the data
  model; verified during Stage 7.5 and noted in the PR if anything needs touching there.

---

## Phases

### Phase 1: Add the `srs-mcp-core` row

**Goal:** `CLAUDE.md`'s Crate Authority table lists all seven workspace crates, including
`srs-mcp-core`, with accurate Owns/Hard-constraints wording matching current code.

**Agent:** Lead Integrator

#### Tasks

- [x] Verify `srs-mcp-core`'s current scope against `crates/srs-mcp-core/src/lib.rs` and
      `crates/srs-mcp-core/Cargo.toml`.
- [x] Add the `srs-mcp-core` row to the Crate Authority table in `CLAUDE.md`.
- [x] Update the intro sentence's crate list if it omits `srs-mcp-core`.
- [x] Grep for other stale crate enumerations in `srs-rust/CLAUDE.md` and fix any found. (None
      found within `CLAUDE.md` itself. `README.md`'s separate "Workspace layout" table has a
      pre-existing, unrelated gap — also missing `srs-mcp` — filed as follow-up issue #1081
      rather than expanding this PR's scope.)

#### Acceptance Criteria

- [x] The Crate Authority table has exactly one new row, for `srs-mcp-core`, correctly
      distinguishing it from `srs-mcp` (the native adapter).
- [x] The row's "Hard constraints" cell matches the issue's suggested wording (no `rmcp`, Tokio,
      stdio, file paths, or `FileStore`; must compile for `wasm32-unknown-unknown`), adjusted only
      if verification against the source found it inaccurate. (Reworded slightly: `srs-mcp-core`
      is not yet actually built for `wasm32-unknown-unknown` in CI — see follow-up #1080 — so the
      row states this as a dependency-driven intent rather than an enforced fact.)
- [x] No other table row or doc content changed.

#### Testing

This is a markdown-only change with no build/test surface. Verification is a visual diff review
of `CLAUDE.md` plus:

```bash
git diff -- CLAUDE.md
```

No `cargo test`/`cargo clippy` changes are expected as a result of this phase (the workspace gate
in Stage 6 still runs to confirm the doc-only change caused no regression).

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Confirm the diff touches only `CLAUDE.md` (and this plan file).
3. Commit:

```bash
git commit
```

---

## Final Acceptance

- [x] `cargo test` passes with no failures (unaffected — doc-only change)
- [x] `cargo clippy -- -D warnings` passes (unaffected — doc-only change)
- [x] CLI output format unchanged (no CLI code touched)
- [x] `cargo test --test payload_contracts` passes (no payload structs changed)
- [x] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [x] `CLAUDE.md`'s Crate Authority table lists `srs-mcp-core` with wording verified against
      `crates/srs-mcp-core/src/lib.rs` and `Cargo.toml`

## Coordination Rules

- Single-agent plan (Lead Integrator only) — no multi-worker coordination needed.
- At the end of the phase: verify acceptance criteria, update plan checkboxes, commit.

## Assumptions

- The issue's suggested wording is directionally correct but must be checked against the actual
  crate source before being copied in verbatim (per the issue body's own instruction).
- No RFC, spec, or schema surface is touched — this is Door-N/A (not a spec/conformance change at
  all), confirmed at Stage 1.5.
